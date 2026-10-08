//! Where wall-clock time comes from.
//!
//! The budget window and the audit log are dated, and they have to survive a
//! restart, so they use wall-clock time. The domain holds a reading as
//! [`WallTime`] and never takes one; this port supplies it, so a test sets the
//! time and tokio's paused clock, which does not move the system clock, is not
//! mistaken for it.

use denon_avr_domain::WallTime;
use std::sync::Arc;

pub trait Clock: Send + Sync {
    /// The time now, in milliseconds since the Unix epoch.
    fn now(&self) -> WallTime;
}

pub type SharedClock = Arc<dyn Clock>;
