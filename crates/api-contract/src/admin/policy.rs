//! The policy in force, as the Operator reads it.

use denon_avr_application::{PolicyDigest, PolicyView};
use denon_avr_domain::WallTime;
use serde::{Deserialize, Serialize};

/// The policy in force, or why there is none.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loaded_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl From<&PolicyView> for PolicyDto {
    fn from(view: &PolicyView) -> Self {
        Self {
            digest: view.digest.map(|digest| digest.to_string()),
            loaded_at_ms: view.loaded_at.map(WallTime::as_millis),
            text: view.text.clone(),
            error: view.error.clone(),
        }
    }
}

impl TryFrom<PolicyDto> for PolicyView {
    type Error = String;

    fn try_from(dto: PolicyDto) -> Result<Self, String> {
        Ok(Self {
            digest: dto
                .digest
                .map(|hex| {
                    PolicyDigest::from_hex(&hex)
                        .ok_or_else(|| "the policy digest is not 64 hexadecimal digits".to_owned())
                })
                .transpose()?,
            loaded_at: dto.loaded_at_ms.map(WallTime),
            text: dto.text,
            error: dto.error,
        })
    }
}
