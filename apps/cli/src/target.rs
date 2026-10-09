//! The receiver a command acts on: resolving it from a selector or the saved
//! configuration, and remembering it as the current receiver afterwards.

use crate::args::Selection;
use crate::render::Report;
use denon_avr_application::{ControlError, OperatorControl};
use denon_avr_domain::{ConfiguredReceivers, DiscoveredReceiver, ReceiverId, ReceiverIdentity};
use denon_avr_infrastructure::discovery_ssdp::DEFAULT_DISCOVERY_TIMEOUT;
use std::collections::BTreeMap;

/// The receiver a command acts on.
pub(super) struct Target {
    pub(super) id: ReceiverId,
    pub(super) identity: ReceiverIdentity,
    /// The configuration entry name, when the receiver is a saved one.
    pub(super) saved_name: Option<String>,
}

pub(super) fn describe(error: ControlError) -> String {
    error.to_string()
}

pub(super) async fn resolve_target(
    operator: &dyn OperatorControl,
    selection: &Selection,
) -> Result<Target, String> {
    let identity = if let Some(host) = &selection.host {
        ReceiverIdentity::ad_hoc(host)
    } else if let Some(index) = selection.receiver {
        operator
            .discover(DEFAULT_DISCOVERY_TIMEOUT)
            .await
            .map_err(describe)?
            .get(index)
            .map(DiscoveredReceiver::identity)
            .ok_or("receiver selection is out of range")?
    } else {
        let configuration = operator.configuration().await.map_err(describe)?;
        let (name, identity) = configuration
            .current()
            .ok_or("a saved receiver or explicit selector is required")?;
        return Ok(Target {
            id: ReceiverId::new(name).map_err(str::to_owned)?,
            identity: identity.clone(),
            saved_name: Some(name.to_owned()),
        });
    };
    let id = operator
        .register_ad_hoc(identity.clone())
        .await
        .map_err(describe)?;
    Ok(Target {
        id,
        identity,
        saved_name: None,
    })
}

/// Remember the receiver a command used as the current one, as every selected
/// read or write has done since version 1.
///
/// A configuration with several receivers was written by hand, and nothing here
/// adds a second one, so it is left alone. A saved receiver keeps its entry
/// name, which is its id.
async fn remember(operator: &dyn OperatorControl, target: &Target) -> Result<(), String> {
    let configuration = operator.configuration().await.map_err(describe)?;
    if configuration.receivers.len() > 1 {
        return Ok(());
    }
    let name = target.saved_name.clone().unwrap_or_else(|| {
        target
            .identity
            .friendly_name
            .clone()
            .unwrap_or_else(|| "default".into())
    });
    let mut sound_mode_favorites = configuration.sound_mode_favorites;
    sound_mode_favorites.retain(|receiver, _| *receiver == name);
    operator
        .save_configuration(&ConfiguredReceivers {
            current: Some(name.clone()),
            receivers: BTreeMap::from([(name, target.identity.clone())]),
            sound_mode_favorites,
        })
        .await
        .map_err(describe)
}

pub(super) async fn remember_or_warn(
    operator: &dyn OperatorControl,
    target: &Target,
    report: &mut Report,
) {
    if let Err(error) = remember(operator, target).await {
        report
            .warnings
            .push(format!("could not remember the receiver: {error}"));
    }
}
