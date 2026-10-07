//! Connector that opens the canonical X3800H session.

use crate::{AvrSessionConfig, X3800hSession};
use denon_avr_application::ports::{BoxFuture, OperationError, ReceiverConnector};
use denon_avr_application::SharedReceiverSession;
use denon_avr_domain::{ReceiverId, ReceiverIdentity};

/// Opens an [`X3800hSession`] for each receiver the control service connects.
#[derive(Debug, Clone, Default)]
pub struct X3800hConnector {
    pub config: AvrSessionConfig,
}

impl X3800hConnector {
    pub fn new(config: AvrSessionConfig) -> Self {
        Self { config }
    }
}

impl ReceiverConnector for X3800hConnector {
    fn connect<'a>(
        &'a self,
        receiver: &'a ReceiverId,
        identity: &'a ReceiverIdentity,
    ) -> BoxFuture<'a, Result<SharedReceiverSession, OperationError>> {
        Box::pin(async move {
            let session =
                X3800hSession::connect(receiver.clone(), &identity.host, self.config.clone())
                    .await?;
            Ok(session as SharedReceiverSession)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_connector_is_object_safe_and_shareable() {
        let shared: std::sync::Arc<dyn ReceiverConnector> =
            std::sync::Arc::new(X3800hConnector::default());
        let _clone = std::sync::Arc::clone(&shared);
    }
}
