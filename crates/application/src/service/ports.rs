//! The control-service port as a [`ServiceHandle`] serves it: receiver reads,
//! operation control and its event stream, and the Operator's administration.

use super::operations::Admission;
use super::{agent, locked, no_agent_path, run_operation, AgentState, ServiceHandle, SHUT_DOWN};
use crate::audit::{AuditEvent, AuditPage, AuditQuery};
use crate::control::{
    AgentLabel, ApprovalHealth, AuditHealth, ConnectionStatus, ControlError, DryRun,
    DryRunDecision, OperationControl, OperationEvent, OperationEventSource, OperationEvents,
    OperationSnapshot, OperationSubmission, OperatorAdmin, PolicyHealth, PolicyView, Principal,
    ReceiverCapabilities, ReceiverReads, ReceiverSummary, ServiceHealth,
};
use crate::ports::BoxFuture;
use crate::receiver_selection::receiver_id;
use crate::session_v3::{Readiness, StateSubscription};
use crate::tokens::{IssuedToken, TokenError, TokenId, TokenRecord};
use denon_avr_domain::{
    ConfiguredReceivers, DiscoveredReceiver, HttpInformationSnapshot, Model, ModelCapabilities,
    OperationId, QuickSelectNameObservation, ReceiverId, ReceiverIdentity, ReceiverIntent,
    SourceCatalogObservation,
};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;
use tracing::warn;

impl ReceiverReads for ServiceHandle {
    fn receivers(&self) -> BoxFuture<'_, Result<Vec<ReceiverSummary>, ControlError>> {
        Box::pin(async move {
            let configuration = self
                .inner
                .config
                .load()
                .await
                .map_err(|error| self.sanitized(ControlError::Receiver(error)))?;
            let mut summaries = Vec::with_capacity(configuration.receivers.len());
            for (name, identity) in &configuration.receivers {
                let Ok(id) = ReceiverId::new(name.as_str()) else {
                    continue;
                };
                let model = Model::from_reported(identity.model.as_deref().unwrap_or_default());
                let connection = self
                    .inner
                    .existing_slot(&id)
                    .map_or(ConnectionStatus::Released, |slot| *locked(&slot.status));
                summaries.push(ReceiverSummary {
                    id,
                    model: identity.model.clone(),
                    capabilities: ReceiverCapabilities::from(&ModelCapabilities::for_model(model)),
                    connection,
                });
            }
            Ok(summaries)
        })
    }

    fn state<'a>(
        &'a self,
        receiver: &'a ReceiverId,
    ) -> BoxFuture<'a, Result<StateSubscription, ControlError>> {
        Box::pin(async move {
            self.visible_receiver(receiver)?;
            let lease = self
                .inner
                .lease(receiver)
                .await
                .map_err(|error| self.sanitized(error))?;
            Ok(lease
                .session
                .state()
                .ending_with(lease.ended.clone())
                .holding(lease))
        })
    }

    fn source_catalog<'a>(
        &'a self,
        receiver: &'a ReceiverId,
    ) -> BoxFuture<'a, Result<SourceCatalogObservation, ControlError>> {
        Box::pin(async move {
            self.visible_receiver(receiver)?;
            let lease = self
                .inner
                .lease(receiver)
                .await
                .map_err(|error| self.sanitized(error))?;
            lease
                .session
                .source_catalog()
                .await
                .map_err(|error| self.sanitized(ControlError::Receiver(error)))
        })
    }

    fn health(&self) -> BoxFuture<'_, Result<ServiceHealth, ControlError>> {
        Box::pin(async move {
            Ok(match &self.inner.agent {
                Some(agent) => agent.health(),
                // No Agent path: nothing to judge, record, or count with.
                None => ServiceHealth {
                    policy: PolicyHealth::NotConfigured,
                    audit: AuditHealth::NotConfigured,
                    ledger_ready: false,
                    approval: ApprovalHealth::Unavailable,
                },
            })
        })
    }
}

impl OperationControl for ServiceHandle {
    fn submit<'a>(
        &'a self,
        receiver: &'a ReceiverId,
        submission: OperationSubmission,
    ) -> BoxFuture<'a, Result<OperationSnapshot, ControlError>> {
        Box::pin(async move {
            self.visible_receiver(receiver)?;
            // An unknown receiver is refused before an operation exists for it.
            self.inner
                .identity(receiver)
                .await
                .map_err(|error| self.sanitized(error))?;
            let admission = self.inner.admit(&self.principal, receiver, submission)?;
            let snapshot = match admission {
                Admission::Existing(snapshot) => return Ok(snapshot),
                Admission::New(snapshot) => snapshot,
            };
            // The first snapshot is `allowed` for the Operator and `submitted`
            // for an agent; the task decides the rest.
            self.inner.publish(&self.principal, &snapshot);
            tokio::spawn(run_operation(
                Arc::clone(&self.inner),
                snapshot.id,
                snapshot.receiver.clone(),
                snapshot.intent.clone(),
                self.principal.clone(),
            ));
            Ok(snapshot)
        })
    }

    fn operation(
        &self,
        id: OperationId,
        wait: Option<Duration>,
    ) -> BoxFuture<'_, Result<OperationSnapshot, ControlError>> {
        Box::pin(async move {
            let mut watched = self.inner.watch_operation(&self.principal, id)?;
            let current = watched.borrow().clone();
            let Some(wait) = wait.filter(|_| !current.status.is_terminal()) else {
                return Ok(current);
            };
            let wait = wait.min(self.inner.settings.max_operation_wait);
            let _ = tokio::time::timeout(wait, watched.wait_for(|s| s.status.is_terminal())).await;
            let latest = watched.borrow().clone();
            Ok(latest)
        })
    }

    fn cancel(&self, id: OperationId) -> BoxFuture<'_, Result<OperationSnapshot, ControlError>> {
        Box::pin(async move { self.inner.cancel(&self.principal, id) })
    }

    fn operation_events(&self) -> BoxFuture<'_, Result<OperationEvents, ControlError>> {
        Box::pin(async move {
            let events = locked(&self.inner.events)
                .as_ref()
                .map(broadcast::Sender::subscribe)
                .ok_or_else(|| ControlError::Unavailable(SHUT_DOWN.into()))?;
            Ok(Box::new(EventStream {
                events,
                viewer: self.principal.clone(),
            }) as OperationEvents)
        })
    }

    fn dry_run<'a>(
        &'a self,
        receiver: &'a ReceiverId,
        intent: ReceiverIntent,
    ) -> BoxFuture<'a, Result<DryRun, ControlError>> {
        Box::pin(async move {
            self.visible_receiver(receiver)?;
            // An unknown receiver is refused here, as it is for a submission.
            self.inner
                .identity(receiver)
                .await
                .map_err(|error| self.sanitized(error))?;
            match &self.principal {
                // The Operator skips policy, so there is nothing to evaluate.
                Principal::Operator => Ok(DryRun {
                    decision: DryRunDecision::Allow,
                    policy: self.inner.agent.as_ref().and_then(AgentState::digest),
                }),
                Principal::Agent(label) => {
                    let agent = self.inner.agent.as_ref().ok_or(ControlError::Forbidden)?;
                    // A dry run opens the receiver, so it is a write against the cap.
                    agent.charge_write(label)?;
                    // An agent is told the limits, not the ids of the rules.
                    agent::dry_run(&self.inner, agent, label.as_str(), receiver, &intent, false)
                        .await
                        .map_err(|error| self.sanitized(error))
                }
            }
        })
    }
}

/// Operation events for one viewer: all of them for the Operator, the viewer's
/// own otherwise.
struct EventStream {
    events: broadcast::Receiver<(Principal, OperationEvent)>,
    viewer: Principal,
}

impl OperationEventSource for EventStream {
    fn next(&mut self) -> BoxFuture<'_, Option<OperationEvent>> {
        Box::pin(async move {
            loop {
                match self.events.recv().await {
                    Ok((owner, event)) => {
                        if self.viewer == Principal::Operator || owner == self.viewer {
                            return Some(event);
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        return Some(OperationEvent::Missed)
                    }
                    Err(broadcast::error::RecvError::Closed) => return None,
                }
            }
        })
    }
}

impl OperatorAdmin for ServiceHandle {
    fn discover(
        &self,
        timeout: Duration,
    ) -> BoxFuture<'_, Result<Vec<DiscoveredReceiver>, ControlError>> {
        Box::pin(async move {
            self.require_operator()?;
            self.inner
                .discovery
                .discover(timeout)
                .await
                .map_err(ControlError::Receiver)
        })
    }

    fn register_ad_hoc(
        &self,
        identity: ReceiverIdentity,
    ) -> BoxFuture<'_, Result<ReceiverId, ControlError>> {
        Box::pin(async move {
            self.require_operator()?;
            let id = receiver_id(None, &identity)
                .map_err(|error| ControlError::InvalidRequest(error.message))?;
            locked(&self.inner.ad_hoc).insert(id.clone(), identity);
            Ok(id)
        })
    }

    fn configuration(&self) -> BoxFuture<'_, Result<ConfiguredReceivers, ControlError>> {
        Box::pin(async move {
            self.require_operator()?;
            self.inner
                .config
                .load()
                .await
                .map_err(ControlError::Receiver)
        })
    }

    fn save_configuration<'a>(
        &'a self,
        configuration: &'a ConfiguredReceivers,
    ) -> BoxFuture<'a, Result<(), ControlError>> {
        Box::pin(async move {
            self.require_operator()?;
            configuration
                .validate()
                .map_err(ControlError::InvalidRequest)?;
            self.inner
                .config
                .save(configuration)
                .await
                .map_err(ControlError::Receiver)?;
            // A session open to an address the file no longer holds would
            // otherwise serve it until released, and a held subscription keeps
            // it from ever being released.
            self.inner.retire_changed(configuration).await;
            Ok(())
        })
    }

    fn quick_select_names<'a>(
        &'a self,
        receiver: &'a ReceiverId,
    ) -> BoxFuture<'a, Result<QuickSelectNameObservation, ControlError>> {
        Box::pin(async move {
            self.require_operator()?;
            let lease = self.inner.lease(receiver).await?;
            lease
                .session
                .quick_select_names()
                .await
                .map_err(ControlError::Receiver)
        })
    }

    fn http_information<'a>(
        &'a self,
        receiver: &'a ReceiverId,
    ) -> BoxFuture<'a, Result<HttpInformationSnapshot, ControlError>> {
        Box::pin(async move {
            self.require_operator()?;
            let lease = self.inner.lease(receiver).await?;
            lease
                .session
                .http_information()
                .await
                .map_err(ControlError::Receiver)
        })
    }

    fn refresh<'a>(
        &'a self,
        receiver: &'a ReceiverId,
    ) -> BoxFuture<'a, Result<Readiness, ControlError>> {
        Box::pin(async move {
            self.require_operator()?;
            let lease = self.inner.lease(receiver).await?;
            lease
                .session
                .synchronize()
                .await
                .map_err(ControlError::Receiver)
        })
    }

    fn dry_run_as<'a>(
        &'a self,
        agent: AgentLabel,
        receiver: &'a ReceiverId,
        intent: ReceiverIntent,
    ) -> BoxFuture<'a, Result<DryRun, ControlError>> {
        Box::pin(async move {
            self.require_operator()?;
            let state = self.inner.agent.as_ref().ok_or_else(no_agent_path)?;
            self.inner.identity(receiver).await?;
            // The Operator testing a tier does not spend the agent's allowance.
            agent::dry_run(&self.inner, state, agent.as_str(), receiver, &intent, true).await
        })
    }

    fn policy(&self) -> BoxFuture<'_, Result<PolicyView, ControlError>> {
        Box::pin(async move {
            self.require_operator()?;
            let agent = self.inner.agent.as_ref().ok_or_else(no_agent_path)?;
            Ok(agent.policy_view())
        })
    }

    fn reload_policy(&self) -> BoxFuture<'_, Result<PolicyView, ControlError>> {
        Box::pin(async move {
            self.require_operator()?;
            let agent = self.inner.agent.as_ref().ok_or_else(no_agent_path)?;
            Ok(agent.load_policy().await)
        })
    }

    fn audit(&self, query: AuditQuery) -> BoxFuture<'_, Result<AuditPage, ControlError>> {
        Box::pin(async move {
            self.require_operator()?;
            let agent = self.inner.agent.as_ref().ok_or_else(no_agent_path)?;
            agent.audit_page(query).await
        })
    }

    fn issue_token(&self, label: AgentLabel) -> BoxFuture<'_, Result<IssuedToken, ControlError>> {
        Box::pin(async move {
            self.require_operator()?;
            let agent = self.inner.agent.as_ref().ok_or_else(no_agent_path)?;
            let tokens = agent.tokens().ok_or_else(no_token_store)?;
            // The label rule is the policy crate's, so a token and a rule that name
            // the same label spell it the same way.
            if !denon_avr_policy::label_is_well_formed(label.as_str()) {
                return Err(ControlError::InvalidRequest(
                    "a token label is lowercase letters, digits, '.', '_' and '-', \
                     starting with a letter or digit"
                        .into(),
                ));
            }
            let issued = tokens
                .issue(label, agent.now())
                .await
                .map_err(token_error)?;
            agent
                .record_token_event(AuditEvent::TokenIssued {
                    id: issued.record.id.clone(),
                    label: issued.record.label.clone(),
                })
                .await;
            Ok(issued)
        })
    }

    fn tokens(&self) -> BoxFuture<'_, Result<Vec<TokenRecord>, ControlError>> {
        Box::pin(async move {
            self.require_operator()?;
            let agent = self.inner.agent.as_ref().ok_or_else(no_agent_path)?;
            Ok(agent.tokens().ok_or_else(no_token_store)?.list())
        })
    }

    fn revoke_token(&self, id: TokenId) -> BoxFuture<'_, Result<TokenRecord, ControlError>> {
        Box::pin(async move {
            self.require_operator()?;
            let agent = self.inner.agent.as_ref().ok_or_else(no_agent_path)?;
            let tokens = agent.tokens().ok_or_else(no_token_store)?;
            let was_active = tokens.is_active(&id);
            let record = tokens.revoke(&id, agent.now()).await.map_err(token_error)?;
            // Revoking twice is not an error, and it is recorded once.
            if was_active {
                agent
                    .record_token_event(AuditEvent::TokenRevoked {
                        id: record.id.clone(),
                        label: record.label.clone(),
                    })
                    .await;
            }
            Ok(record)
        })
    }
}

fn no_token_store() -> ControlError {
    ControlError::Unavailable("this service has no token store".into())
}

/// What a token store's refusal means to a caller of the port.
fn token_error(error: TokenError) -> ControlError {
    match error {
        TokenError::InvalidLabel | TokenError::LabelInUse => {
            ControlError::InvalidRequest(error.to_string())
        }
        TokenError::NotFound => ControlError::NotFound("token"),
        TokenError::Storage(why) => {
            // The store's text can name a path, which stays in the log.
            warn!(%why, "the token store failed");
            ControlError::Unavailable("the token store could not be read or written".into())
        }
    }
}
