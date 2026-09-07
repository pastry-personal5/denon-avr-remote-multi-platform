//! Model capability declarations.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Model {
    AvrX3800h,
    Unknown,
}

impl Model {
    pub fn from_reported(value: &str) -> Self {
        let normalized = value.to_ascii_uppercase().replace([' ', '-'], "");
        if normalized.contains("AVRX3800H") || normalized.contains("AVCX3800H") {
            Self::AvrX3800h
        } else {
            Self::Unknown
        }
    }
}

use super::{Input, MainZoneControl, MuteState, PowerState, SurroundMode, VolumeLevel};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelCapabilities {
    pub model: Model,
    pub tcp_avr_port: u16,
    pub heos_cli_port: u16,
    pub supports_heos_secure_cli: Option<bool>,
    pub needs_live_validation: bool,
    pub writable: bool,
    pub native_volume_min: u16,
    pub native_volume_max: u16,
    pub inputs: &'static [&'static str],
    pub surround_modes: &'static [&'static str],
}

const X3800H_INPUTS: &[&str] = &[
    "PHONO",
    "CD",
    "TUNER",
    "DVD",
    "BD",
    "GAME",
    "MEDIA PLAYER",
    "TV AUDIO",
    "AUX1",
    "AUX2",
    "AUX3",
    "AUX4",
    "AUX5",
    "AUX6",
    "AUX7",
];
const X3800H_SURROUND_MODES: &[&str] = &["DIRECT", "PURE DIRECT", "STEREO", "MULTI CH IN"];

impl ModelCapabilities {
    pub const fn for_model(model: Model) -> Self {
        let writable = matches!(model, Model::AvrX3800h);
        Self {
            model,
            tcp_avr_port: 23,
            heos_cli_port: 1255,
            supports_heos_secure_cli: None,
            needs_live_validation: writable,
            writable,
            native_volume_min: 0,
            native_volume_max: 985,
            inputs: if writable { X3800H_INPUTS } else { &[] },
            surround_modes: if writable { X3800H_SURROUND_MODES } else { &[] },
        }
    }

    pub fn supports_control(&self, control: &MainZoneControl) -> bool {
        if !self.writable {
            return false;
        }
        match control {
            MainZoneControl::Power(PowerState::On | PowerState::Standby)
            | MainZoneControl::Mute(MuteState::On | MuteState::Off) => true,
            MainZoneControl::Volume(level) => {
                let code = level.to_native_code();
                code >= self.native_volume_min && code <= self.native_volume_max
            }
            MainZoneControl::Input(value) => self.inputs.contains(&value.as_str()),
            MainZoneControl::SurroundMode(value) => self.surround_modes.contains(&value.as_str()),
        }
    }

    pub fn volume_level_from_native(&self, code: u16) -> Result<VolumeLevel, &'static str> {
        if code < self.native_volume_min || code > self.native_volume_max {
            return Err("native volume code is outside model capability range");
        }
        VolumeLevel::from_native_code(code)
    }

    pub fn input(&self, value: &str) -> Result<Input, &'static str> {
        let input = Input::new(value.to_owned())?;
        if self.inputs.contains(&input.as_str()) {
            Ok(input)
        } else {
            Err("input is not supported by this receiver")
        }
    }

    pub fn surround_mode(&self, value: &str) -> Result<SurroundMode, &'static str> {
        let mode = SurroundMode::new(value.to_owned())?;
        if self.surround_modes.contains(&mode.as_str()) {
            Ok(mode)
        } else {
            Err("surround mode is not supported by this receiver")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_avr_and_avc_x3800h_names() {
        assert_eq!(Model::from_reported("Denon AVR-X3800H"), Model::AvrX3800h);
        assert_eq!(Model::from_reported("AVC X3800H"), Model::AvrX3800h);
        assert_eq!(Model::from_reported("unknown"), Model::Unknown);
    }
}
