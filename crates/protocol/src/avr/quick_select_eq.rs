//! Quick Select/EQ AVR command primitives. These are kept typed so callers cannot
//! accidentally treat a preset as several independently replayable controls.

use super::command::{AvrCommand, AvrProtocolError};
use denon_avr_domain::{EqFeature, EqState, QuickSelectSlot};

pub fn quick_select_command(slot: QuickSelectSlot) -> AvrCommand {
    // Denon Main Zone Quick Select recall uses the MSQUICK family.  There is
    // no validated per-slot query, so recall is intentionally execute-only.
    AvrCommand::new(format!("MSQUICK{}", slot.number())).expect("validated Quick Select slot")
}
pub fn eq_status_query(feature: EqFeature) -> AvrCommand {
    let command = match feature {
        EqFeature::MultEqXt32 => "PSMULTEQ: ?",
        EqFeature::DynamicEq => "PSDYNEQ ?",
        EqFeature::DynamicEqReferenceLevel => "PSREFLEV ?",
        EqFeature::DynamicVolume => "PSDYNVOL ?",
        EqFeature::AudysseyLfc => "PSLFC ?",
        EqFeature::DiracLive => "PSDIRAC ?",
    };
    AvrCommand::new(command).expect("static EQ command")
}

pub fn parse_eq_status(feature: EqFeature, response: &str) -> Result<EqState, AvrProtocolError> {
    let family = match feature {
        EqFeature::MultEqXt32 => "PSMULTEQ",
        EqFeature::DynamicEq => "PSDYNEQ",
        EqFeature::DynamicEqReferenceLevel => "PSREFLEV",
        EqFeature::DynamicVolume => "PSDYNVOL",
        EqFeature::AudysseyLfc => "PSLFC",
        EqFeature::DiracLive => "PSDIRAC",
    };
    let suffix = response
        .strip_prefix(family)
        .ok_or(AvrProtocolError::MalformedResponse(
            "EQ response has wrong family",
        ))?;
    let value = suffix
        .strip_prefix(':')
        .or_else(|| suffix.strip_prefix(' '))
        .ok_or(AvrProtocolError::MalformedResponse(
            "EQ response has no parameter separator",
        ))?
        .trim();
    if value.is_empty() {
        return Err(AvrProtocolError::MalformedResponse(
            "EQ response has no value",
        ));
    }
    Ok(match value.to_ascii_uppercase().as_str() {
        "ON" | "1" => EqState::On,
        "OFF" | "0" => EqState::Off,
        "N/A" | "NA" => EqState::NotApplicable("not applicable for the current setup".into()),
        "UNAVAILABLE" => EqState::Unavailable("not available for the current setup".into()),
        "UNSUPPORTED" => EqState::Unsupported,
        "NOT APPLICABLE" | "NOT_APPLICABLE" => {
            EqState::NotApplicable("not applicable for the current setup".into())
        }
        "UNKNOWN" => EqState::Unknown,
        value => EqState::Configured(value.to_owned()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quick_select_is_one_command() {
        assert_eq!(
            quick_select_command(QuickSelectSlot::new(2).unwrap()).as_str(),
            "MSQUICK2"
        );
    }
    #[test]
    fn eq_states_do_not_collapse_configured_values() {
        assert_eq!(
            parse_eq_status(EqFeature::DynamicEqReferenceLevel, "PSREFLEV 10").unwrap(),
            EqState::Configured("10".into())
        );
        assert_eq!(
            parse_eq_status(EqFeature::MultEqXt32, "PSMULTEQ:AUDYSSEY").unwrap(),
            EqState::Configured("AUDYSSEY".into())
        );
    }

    #[test]
    fn eq_queries_preserve_documented_parameter_spacing() {
        assert_eq!(
            eq_status_query(EqFeature::MultEqXt32).as_str(),
            "PSMULTEQ: ?"
        );
        assert_eq!(eq_status_query(EqFeature::DynamicEq).as_str(), "PSDYNEQ ?");
        assert_eq!(
            eq_status_query(EqFeature::DynamicVolume).as_bytes(),
            b"PSDYNVOL ?\r"
        );
    }

    #[test]
    fn malformed_and_unknown_eq_responses_remain_honest() {
        assert_eq!(
            parse_eq_status(EqFeature::DiracLive, "PSDIRAC UNKNOWN").unwrap(),
            EqState::Unknown
        );
        assert!(parse_eq_status(EqFeature::DynamicEq, "MSSTEREO").is_err());
        assert_eq!(
            parse_eq_status(EqFeature::DynamicVolume, "PSDYNVOL UNSUPPORTED").unwrap(),
            EqState::Unsupported
        );
        assert!(parse_eq_status(EqFeature::DynamicEq, "PSDYNEQON").is_err());
        assert!(matches!(
            parse_eq_status(EqFeature::DynamicVolume, "PSDYNVOL N/A").unwrap(),
            EqState::NotApplicable(_)
        ));
    }
}
