//! Infrastructure-side bridge from parsed X3800H wire frames to domain state.

use denon_avr_domain::CoreFrame;
use denon_avr_protocol::avr::X3800hFrame;

/// Unknown frames remain diagnostics only: they never become canonical state.
pub fn core_frame(frame: X3800hFrame) -> Option<CoreFrame> {
    match frame {
        X3800hFrame::SystemPower(value) => Some(CoreFrame::SystemPower(value)),
        X3800hFrame::MainZonePower(value) => Some(CoreFrame::MainZonePower(value)),
        X3800hFrame::Zone2Power(value) => Some(CoreFrame::Zone2Power(value)),
        X3800hFrame::Source(value) => Some(CoreFrame::Source(value)),
        X3800hFrame::Volume(value) => Some(CoreFrame::Volume(value)),
        X3800hFrame::Mute(value) => Some(CoreFrame::Mute(value)),
        X3800hFrame::SoundMode(value) => Some(CoreFrame::SoundMode(value)),
        X3800hFrame::VolumeUnavailable => Some(CoreFrame::VolumeUnavailable {
            raw: "MV---".into(),
        }),
        X3800hFrame::Unknown(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use denon_avr_domain::ZonePower;

    #[test]
    fn unknown_wire_frame_cannot_mutate_typed_state() {
        assert!(core_frame(X3800hFrame::Unknown("ZZanything".into())).is_none());
        assert_eq!(
            core_frame(X3800hFrame::MainZonePower(ZonePower::On)),
            Some(CoreFrame::MainZonePower(ZonePower::On))
        );
    }
}
