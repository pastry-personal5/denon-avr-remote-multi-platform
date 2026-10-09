//! The per-receiver connection pool: one slot per receiver, the leases that keep
//! its session open, connecting on demand, and releasing it when idle or when its
//! saved entry changes.

use super::{locked, unavailable, Inner, SHUT_DOWN};
use crate::control::{ConnectionStatus, ControlError};
use crate::session_v3::SharedReceiverSession;
use denon_avr_domain::{ConfiguredReceivers, ReceiverId, ReceiverIdentity};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::watch;
use tracing::{debug, warn};

/// One receiver's connection. The async lock serializes connect, use, and
/// release, which is what makes each of those safe against the others.
pub(super) struct Slot {
    pub(super) session: tokio::sync::Mutex<Option<Held>>,
    /// Leases alive: reads, held subscriptions, and operations in flight.
    leases: AtomicUsize,
    /// How many of those leases belong to an operation. Retiring a session
    /// waits for this to reach zero, because closing under an operation in
    /// flight would lose its outcome.
    operations: watch::Sender<usize>,
    /// Bumped whenever a lease is taken or dropped, so an idle timer can tell
    /// whether anything happened after it was armed.
    activity: AtomicU64,
    pub(super) status: Mutex<ConnectionStatus>,
}

impl Slot {
    fn new() -> Self {
        Self {
            session: tokio::sync::Mutex::new(None),
            leases: AtomicUsize::new(0),
            operations: watch::channel(0).0,
            activity: AtomicU64::new(0),
            status: Mutex::new(ConnectionStatus::Released),
        }
    }
}

/// An open session, with what it was opened for and the signal that ends its
/// subscribers when the service closes it.
pub(super) struct Held {
    session: SharedReceiverSession,
    /// The identity the session connected with. A saved entry whose host later
    /// differs retires the session.
    identity: ReceiverIdentity,
    ended: watch::Sender<bool>,
}

/// Tell the session's subscribers it is over, then close it. The caller has
/// taken it out of the slot, under the slot lock.
pub(super) async fn close_held(slot: &Slot, held: Held, why: &str) {
    held.ended.send_replace(true);
    if let Err(error) = held.session.close().await {
        warn!(%error, why, "closing receiver session");
    }
    *locked(&slot.status) = ConnectionStatus::Released;
}

/// Marks a slot as connecting for as long as an attempt runs. A caller can drop
/// the attempt at any await, and the slot must not stay `Connecting` for a
/// connection nobody is making, so an attempt that did not finish puts it back.
struct ConnectAttempt<'a> {
    status: &'a Mutex<ConnectionStatus>,
    finished: bool,
}

impl<'a> ConnectAttempt<'a> {
    fn start(status: &'a Mutex<ConnectionStatus>) -> Self {
        *locked(status) = ConnectionStatus::Connecting;
        Self {
            status,
            finished: false,
        }
    }

    fn finish(mut self, status: ConnectionStatus) {
        *locked(self.status) = status;
        self.finished = true;
    }
}

impl Drop for ConnectAttempt<'_> {
    fn drop(&mut self) {
        if !self.finished {
            *locked(self.status) = ConnectionStatus::Released;
        }
    }
}

/// Keeps a receiver connected while it lives.
pub(super) struct Lease {
    inner: Arc<Inner>,
    slot: Arc<Slot>,
    pub(super) session: SharedReceiverSession,
    pub(super) ended: watch::Receiver<bool>,
    operation: bool,
}

impl Drop for Lease {
    fn drop(&mut self) {
        if self.operation {
            self.slot.operations.send_modify(|count| *count -= 1);
        }
        self.slot.activity.fetch_add(1, Ordering::SeqCst);
        if self.slot.leases.fetch_sub(1, Ordering::SeqCst) == 1 {
            self.inner.arm_idle_timer(&self.slot);
        }
    }
}

impl Inner {
    pub(super) fn existing_slot(&self, receiver: &ReceiverId) -> Option<Arc<Slot>> {
        locked(&self.slots).get(receiver).cloned()
    }

    /// The receiver's current address, from the saved configuration or, for an
    /// ad hoc receiver, from what the Operator registered.
    pub(super) async fn identity(
        &self,
        receiver: &ReceiverId,
    ) -> Result<ReceiverIdentity, ControlError> {
        if receiver.is_ad_hoc() {
            return locked(&self.ad_hoc)
                .get(receiver)
                .cloned()
                .ok_or(ControlError::NotFound("receiver"));
        }
        let configuration = self.config.load().await.map_err(ControlError::Receiver)?;
        configuration
            .receivers
            .get(receiver.as_str())
            .cloned()
            .ok_or(ControlError::NotFound("receiver"))
    }

    /// Take a lease on the receiver's session, connecting and synchronizing
    /// first when there is none. Concurrent callers wait on the slot lock, so
    /// exactly one connection is attempted.
    pub(super) async fn lease(
        self: &Arc<Self>,
        receiver: &ReceiverId,
    ) -> Result<Lease, ControlError> {
        self.lease_as(receiver, false).await
    }

    /// As [`Inner::lease`], counting the lease as an operation's when
    /// `operation` is set.
    pub(super) async fn lease_as(
        self: &Arc<Self>,
        receiver: &ReceiverId,
        operation: bool,
    ) -> Result<Lease, ControlError> {
        let closed = || ControlError::Unavailable(SHUT_DOWN.into());
        if self.closed.load(Ordering::SeqCst) {
            return Err(closed());
        }
        let slot = match self.existing_slot(receiver) {
            Some(slot) => slot,
            None => {
                // Only receivers that exist get a slot.
                self.identity(receiver).await?;
                let mut slots = locked(&self.slots);
                let slot = slots
                    .entry(receiver.clone())
                    .or_insert_with(|| Arc::new(Slot::new()));
                Arc::clone(slot)
            }
        };
        let mut held = slot.session.lock().await;
        if self.closed.load(Ordering::SeqCst) {
            return Err(closed());
        }
        let (session, ended) = match held.as_ref() {
            Some(open) => (Arc::clone(&open.session), open.ended.subscribe()),
            None => {
                let (session, identity) = self.connect(receiver, &slot).await?;
                let ended = watch::channel(false).0;
                let subscriber = ended.subscribe();
                *held = Some(Held {
                    session: Arc::clone(&session),
                    identity,
                    ended,
                });
                (session, subscriber)
            }
        };
        // Counted while the lock is held, so a release cannot slip in between.
        slot.leases.fetch_add(1, Ordering::SeqCst);
        if operation {
            slot.operations.send_modify(|count| *count += 1);
        }
        slot.activity.fetch_add(1, Ordering::SeqCst);
        Ok(Lease {
            inner: Arc::clone(self),
            slot: Arc::clone(&slot),
            session,
            ended,
            operation,
        })
    }

    async fn connect(
        &self,
        receiver: &ReceiverId,
        slot: &Slot,
    ) -> Result<(SharedReceiverSession, ReceiverIdentity), ControlError> {
        let attempt = ConnectAttempt::start(&slot.status);
        let result = async {
            let identity = self.identity(receiver).await?;
            let session = self
                .connector
                .connect(receiver, &identity)
                .await
                .map_err(unavailable)?;
            // The first state a caller sees is complete.
            if let Err(error) = session.synchronize().await {
                let _ = session.close().await;
                return Err(unavailable(error));
            }
            Ok((session, identity))
        }
        .await;
        attempt.finish(match result {
            Ok(_) => ConnectionStatus::Connected,
            Err(_) => ConnectionStatus::Released,
        });
        result
    }

    /// Close the session of every receiver whose saved entry no longer reaches
    /// the address it connected to, or was removed, so the next lease connects
    /// to the address now in the file.
    ///
    /// Each close takes the slot lock, which keeps a new connection from opening
    /// beside the old one (the receiver accepts a single control connection),
    /// and waits for operations in flight, because a close that outlasts its
    /// grace period aborts the session and would lose an outcome. It does not
    /// wait for subscribers, who can hold a session indefinitely: they are told
    /// it ended and subscribe again.
    pub(super) async fn retire_changed(&self, saved: &ConfiguredReceivers) {
        let slots: Vec<(ReceiverId, Arc<Slot>)> = locked(&self.slots)
            .iter()
            .map(|(id, slot)| (id.clone(), Arc::clone(slot)))
            .collect();
        for (id, slot) in slots {
            if id.is_ad_hoc() {
                continue;
            }
            let mut held = slot.session.lock().await;
            let unchanged = held.as_ref().is_none_or(|open| {
                saved
                    .receivers
                    .get(id.as_str())
                    .is_some_and(|entry| entry.host == open.identity.host)
            });
            if unchanged {
                continue;
            }
            let mut operations = slot.operations.subscribe();
            let _ = operations.wait_for(|count| *count == 0).await;
            if let Some(open) = held.take() {
                debug!(
                    receiver = id.as_str(),
                    "retiring session for a changed entry"
                );
                close_held(&slot, open, "its saved entry changed").await;
            }
        }
    }

    /// Start the idle clock. One timer runs per time the last lease drops; a
    /// timer that finds activity since it was armed does nothing, because the
    /// lease that caused the activity arms its own when it drops.
    fn arm_idle_timer(self: &Arc<Self>, slot: &Arc<Slot>) {
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let armed_at = slot.activity.load(Ordering::SeqCst);
        let inner = Arc::clone(self);
        let slot = Arc::clone(slot);
        runtime.spawn(async move {
            tokio::time::sleep(inner.settings.idle_release).await;
            inner.release_if_idle(&slot, armed_at).await;
        });
    }

    /// Close the session if nothing used it since `armed_at`. The check and the
    /// close happen under the slot lock, so a request that arrives during the
    /// close waits for it and then reconnects.
    async fn release_if_idle(&self, slot: &Slot, armed_at: u64) {
        let mut session = slot.session.lock().await;
        if slot.leases.load(Ordering::SeqCst) != 0
            || slot.activity.load(Ordering::SeqCst) != armed_at
        {
            return;
        }
        if let Some(open) = session.take() {
            debug!("releasing idle receiver session");
            close_held(slot, open, "idle").await;
        }
    }
}
