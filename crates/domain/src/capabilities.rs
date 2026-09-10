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

use super::{
    Input, MainZoneControl, MuteState, PowerState, SoundModeCategory, SurroundMode, VolumeLevel,
};

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
    pub quick_select_recall: bool,
    pub quick_select_names: bool,
    pub eq_status: bool,
    pub source_catalog_read: bool,
    pub http_information_read: bool,
    pub zone2_power: bool,
}

/// Capabilities may be enabled only after model/firmware-specific Quick
/// Recall/EQ validation has been recorded. Quick Select names are a separate,
/// read-only AppCommand capability validated for the X3800H profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuickSelectEqCapabilities {
    pub quick_select_recall: bool,
    pub quick_select_names: bool,
    pub eq_status: bool,
}

impl Default for QuickSelectEqCapabilities {
    fn default() -> Self {
        Self {
            quick_select_recall: false,
            quick_select_names: true,
            eq_status: false,
        }
    }
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
const X3800H_SURROUND_MODES: &[&str] = &[
    "DIRECT",
    "PURE DIRECT",
    "STEREO",
    "MULTI CH IN",
    "DOLBY SURROUND",
    "MCH STEREO",
    "MONO MOVIE",
    "ROCK ARENA",
    "JAZZ CLUB",
    "MATRIX",
    "VIDEO GAME",
    "DTS NEURAL:X",
    "DTS VIRTUAL:X",
    "AURO-3D",
    "VIRTUAL",
    "AUTO",
];
const X3800H_MOVIE_MODES: &[&str] = &[
    "DOLBY SURROUND",
    "DTS NEURAL:X",
    "DTS VIRTUAL:X",
    "AURO-3D",
    "MULTI CH IN",
    "MONO MOVIE",
    "VIRTUAL",
];
const X3800H_MUSIC_MODES: &[&str] = &[
    "STEREO",
    "DOLBY SURROUND",
    "DTS NEURAL:X",
    "DTS VIRTUAL:X",
    "AURO-3D",
    "MULTI CH IN",
    "MCH STEREO",
    "ROCK ARENA",
    "JAZZ CLUB",
    "MATRIX",
];
const X3800H_GAME_MODES: &[&str] = &[
    "DOLBY SURROUND",
    "DTS NEURAL:X",
    "DTS VIRTUAL:X",
    "AURO-3D",
    "MULTI CH IN",
    "MCH STEREO",
    "VIDEO GAME",
];
const X3800H_PURE_MODES: &[&str] = &["AUTO", "DIRECT", "PURE DIRECT", "STEREO"];

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
            // Quick Select recall and EQ remain execute/query candidates until
            // separately validated. Quick Select names are a validated,
            // read-only AppCommand observation for the X3800H profile.
            quick_select_recall: false,
            quick_select_names: matches!(model, Model::AvrX3800h),
            eq_status: false,
            // Source labels and visibility are receiver-owned presentation
            // facts. The X3800H profile reads them without enabling any
            // source rename or hide/write operation.
            source_catalog_read: matches!(model, Model::AvrX3800h),
            // Validated Phase 1 observations apply only to X3800H-family models.
            http_information_read: matches!(model, Model::AvrX3800h),
            zone2_power: matches!(model, Model::AvrX3800h),
        }
    }

    pub const fn with_validated_quick_select_eq(
        mut self,
        capabilities: QuickSelectEqCapabilities,
    ) -> Self {
        if matches!(self.model, Model::AvrX3800h) {
            self.quick_select_recall = capabilities.quick_select_recall;
            self.quick_select_names = capabilities.quick_select_names;
            self.eq_status = capabilities.eq_status;
        }
        self
    }

    pub const fn with_validated_source_catalog(
        mut self,
        capabilities: SourceCatalogCapabilities,
    ) -> Self {
        if matches!(self.model, Model::AvrX3800h) {
            self.source_catalog_read |= capabilities.source_catalog_read;
        }
        self
    }

    pub fn supports_control(&self, control: &MainZoneControl) -> bool {
        if !self.writable && !matches!(control, MainZoneControl::Volume(_)) {
            return false;
        }
        match control {
            MainZoneControl::Power(PowerState::On | PowerState::Standby)
            | MainZoneControl::Mute(MuteState::On | MuteState::Off) => true,
            // Main Zone volume uses the stable MV protocol across Denon AVR
            // models. A confirmed volume snapshot is enough evidence to
            // expose this bounded control even when the broader model write
            // profile has not been validated yet.
            MainZoneControl::Volume(level) => {
                let code = level.to_native_code();
                code >= self.native_volume_min && code <= self.native_volume_max
            }
            MainZoneControl::Input(value) => self.inputs.contains(&value.as_str()),
            MainZoneControl::SurroundMode(value) => self.surround_modes.contains(&value.as_str()),
            MainZoneControl::SelectSoundMode { mode, .. } => {
                self.surround_modes.contains(&mode.as_str())
            }
            MainZoneControl::RecallSoundModeCategory(_) => {
                matches!(self.model, Model::AvrX3800h)
            }
        }
    }

    pub fn sound_modes(&self, category: SoundModeCategory) -> &'static [&'static str] {
        if !self.writable {
            return &[];
        }
        match category {
            SoundModeCategory::Movie => X3800H_MOVIE_MODES,
            SoundModeCategory::Music => X3800H_MUSIC_MODES,
            SoundModeCategory::Game => X3800H_GAME_MODES,
            SoundModeCategory::Pure => X3800H_PURE_MODES,
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

    #[test]
    fn individual_modes_are_available_as_a_complete_catalog() {
        let capabilities = ModelCapabilities::for_model(Model::AvrX3800h);
        assert!(capabilities
            .sound_modes(SoundModeCategory::Pure)
            .contains(&"PURE DIRECT"));
    }

    #[test]
    fn uses_the_x3800h_telnet_spelling_for_multi_channel_stereo() {
        let capabilities = ModelCapabilities::for_model(Model::AvrX3800h);
        assert!(capabilities.surround_mode("MCH STEREO").is_ok());
        assert!(capabilities.surround_mode("MULTI CH STEREO").is_err());
    }

    #[test]
    fn quick_select_eq_candidates_do_not_inherit_main_zone_validation() {
        let capabilities = ModelCapabilities::for_model(Model::AvrX3800h);
        assert!(capabilities.writable);
        assert!(!capabilities.quick_select_recall);
        assert!(!capabilities.eq_status);
    }

    #[test]
    fn unknown_models_can_use_bounded_main_zone_volume() {
        let capabilities = ModelCapabilities::for_model(Model::Unknown);
        let volume = VolumeLevel::from_native_code(500).unwrap();
        assert!(capabilities.supports_control(&MainZoneControl::Volume(volume)));
    }

    #[test]
    fn validated_quick_select_eq_profile_is_explicit_opt_in() {
        let capabilities = ModelCapabilities::for_model(Model::AvrX3800h)
            .with_validated_quick_select_eq(QuickSelectEqCapabilities {
                quick_select_recall: true,
                quick_select_names: true,
                eq_status: true,
            });
        assert!(capabilities.quick_select_recall);
        assert!(capabilities.eq_status);
    }

    #[test]
    fn unknown_models_never_inherit_quick_select_eq_validation() {
        let capabilities = ModelCapabilities::for_model(Model::Unknown)
            .with_validated_quick_select_eq(QuickSelectEqCapabilities {
                quick_select_recall: true,
                quick_select_names: true,
                eq_status: true,
            });
        assert!(!capabilities.quick_select_recall);
        assert!(!capabilities.eq_status);
    }

    #[test]
    fn http_information_is_gated_to_validated_x3800h_models() {
        assert!(ModelCapabilities::for_model(Model::AvrX3800h).http_information_read);
        assert!(!ModelCapabilities::for_model(Model::Unknown).http_information_read);
    }

    #[test]
    fn source_catalog_read_is_available_only_for_x3800h_models() {
        assert!(ModelCapabilities::for_model(Model::AvrX3800h).source_catalog_read);
        assert!(!ModelCapabilities::for_model(Model::Unknown).source_catalog_read);
    }
}

/// Model/firmware-specific evidence required before candidate source catalog
/// reads may leave the diagnostic tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SourceCatalogCapabilities {
    pub source_catalog_read: bool,
}
