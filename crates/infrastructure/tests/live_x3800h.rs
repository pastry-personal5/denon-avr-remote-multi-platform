//! Opt-in, read-only validation against a real AVC-X3800H.
//!
//! Ordinary CI never runs this target. Set `DENON_X3800H_HOST` and explicitly
//! pass `--ignored` when a receiver, firmware, region, and Network Control
//! setting have been recorded in the validation log.

use denon_avr_application::CanonicalReceiverSession;
use denon_avr_domain::ReceiverId;
use denon_avr_infrastructure::{AvrSessionConfig, X3800hSession};

#[tokio::test]
#[ignore = "requires an explicitly selected physical AVC-X3800H"]
async fn read_only_core_sync_against_live_x3800h() {
    let host = std::env::var("DENON_X3800H_HOST")
        .expect("set DENON_X3800H_HOST to arm live read-only validation");
    let session = X3800hSession::connect(
        ReceiverId::new(host.clone()).expect("host is a valid receiver identity"),
        &host,
        AvrSessionConfig::default(),
    )
    .await
    .expect("connect to live receiver");
    let readiness = session
        .synchronize()
        .await
        .expect("read-only synchronization");
    assert!(
        readiness.ready,
        "live receiver synchronization degraded: {readiness:?}"
    );
    session.close().await.expect("close live read-only session");
}
