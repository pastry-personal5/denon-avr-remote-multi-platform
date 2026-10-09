//! Agent tokens: the issue request, a token's public record, the list, and the one
//! response that holds a secret.

use denon_avr_application::{AgentLabel, IssuedToken, TokenId, TokenRecord, TokenSecret};
use denon_avr_domain::WallTime;
use serde::{Deserialize, Serialize};

/// `POST /v1/tokens`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenIssueRequest {
    pub label: String,
}

/// What a token is, with no secret and no digest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenDto {
    pub id: String,
    pub label: String,
    pub created_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revoked_ms: Option<u64>,
}

impl From<&TokenRecord> for TokenDto {
    fn from(record: &TokenRecord) -> Self {
        Self {
            id: record.id.as_str().to_owned(),
            label: record.label.as_str().to_owned(),
            created_ms: record.created.as_millis(),
            revoked_ms: record.revoked.map(WallTime::as_millis),
        }
    }
}

impl TryFrom<TokenDto> for TokenRecord {
    type Error = String;

    fn try_from(dto: TokenDto) -> Result<Self, String> {
        Ok(Self {
            id: TokenId::new(dto.id).map_err(str::to_owned)?,
            label: AgentLabel::new(dto.label).map_err(str::to_owned)?,
            created: WallTime(dto.created_ms),
            revoked: dto.revoked_ms.map(WallTime),
        })
    }
}

/// `GET /v1/tokens`'s answer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenListDto {
    pub tokens: Vec<TokenDto>,
}

/// The only response that holds a token's secret, once, when it is issued.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssuedTokenDto {
    #[serde(flatten)]
    pub token: TokenDto,
    pub secret: String,
}

impl From<&IssuedToken> for IssuedTokenDto {
    fn from(issued: &IssuedToken) -> Self {
        Self {
            token: (&issued.record).into(),
            secret: issued.secret.expose().to_owned(),
        }
    }
}

impl TryFrom<IssuedTokenDto> for IssuedToken {
    type Error = String;

    fn try_from(dto: IssuedTokenDto) -> Result<Self, String> {
        Ok(Self {
            record: dto.token.try_into()?,
            secret: TokenSecret::new(dto.secret),
        })
    }
}
