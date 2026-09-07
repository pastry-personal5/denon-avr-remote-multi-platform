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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelCapabilities {
    pub model: Model,
    pub tcp_avr_port: u16,
    pub heos_cli_port: u16,
    pub supports_heos_secure_cli: Option<bool>,
    pub needs_live_validation: bool,
}

impl ModelCapabilities {
    pub const fn for_model(model: Model) -> Self {
        Self {
            model,
            tcp_avr_port: 23,
            heos_cli_port: 1255,
            supports_heos_secure_cli: None,
            needs_live_validation: true,
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
