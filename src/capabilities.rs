//! Model capability declarations.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Model {
    AvrX3800h,
    Unknown,
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
        match model {
            Model::AvrX3800h => Self {
                model,
                tcp_avr_port: 23,
                heos_cli_port: 1255,
                supports_heos_secure_cli: None,
                needs_live_validation: true,
            },
            Model::Unknown => Self {
                model,
                tcp_avr_port: 23,
                heos_cli_port: 1255,
                supports_heos_secure_cli: None,
                needs_live_validation: true,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn x3800h_starts_with_documented_ports_and_unverified_features() {
        let capabilities = ModelCapabilities::for_model(Model::AvrX3800h);
        assert_eq!(capabilities.tcp_avr_port, 23);
        assert_eq!(capabilities.heos_cli_port, 1255);
        assert_eq!(capabilities.supports_heos_secure_cli, None);
        assert!(capabilities.needs_live_validation);
    }
}
