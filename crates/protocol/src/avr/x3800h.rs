//! Strict AVC/AVR-X3800H core protocol profile.

use super::{AvrCommand, AvrProtocolError};
use denon_avr_domain::{
    MasterVolume, MuteState, ReceiverIntent, SoundModeIntent, SoundModeStatus, SourceId,
    SystemPower, ZonePower,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum X3800hFrame {
    SystemPower(SystemPower),
    MainZonePower(ZonePower),
    Zone2Power(ZonePower),
    Source(SourceId),
    Volume(MasterVolume),
    Mute(MuteState),
    SoundMode(SoundModeStatus),
    VolumeUnavailable,
    /// Receiver-reported volume ceiling; not the current Main Zone volume.
    VolumeLimit(String),
    Unknown(String),
}

pub fn query(intent: &ReceiverIntent) -> AvrCommand {
    AvrCommand::new(match intent {
        ReceiverIntent::SystemPower(_) => "PW?",
        ReceiverIntent::MainZonePower(_) => "ZM?",
        ReceiverIntent::Zone2Power(_) => "Z2?",
        ReceiverIntent::Source(_) => "SI?",
        ReceiverIntent::Volume(_) => "MV?",
        ReceiverIntent::Mute(_) => "MU?",
        ReceiverIntent::SoundMode(_) => "MS?",
    })
    .expect("fixed X3800H query is valid")
}

pub fn encode(intent: &ReceiverIntent) -> Result<AvrCommand, AvrProtocolError> {
    let value = match intent {
        ReceiverIntent::SystemPower(SystemPower::On) => "PWON".to_owned(),
        ReceiverIntent::SystemPower(SystemPower::Standby) => "PWSTANDBY".to_owned(),
        ReceiverIntent::MainZonePower(ZonePower::On) => "ZMON".to_owned(),
        ReceiverIntent::MainZonePower(ZonePower::Off) => "ZMOFF".to_owned(),
        ReceiverIntent::Zone2Power(ZonePower::On) => "Z2ON".to_owned(),
        ReceiverIntent::Zone2Power(ZonePower::Off) => "Z2OFF".to_owned(),
        ReceiverIntent::Source(id) => format!(
            "SI{}",
            denon_avr_domain::canonical_x3800h_source(id.as_str()).ok_or(
                AvrProtocolError::InvalidCommand("source is not supported by X3800H profile"),
            )?
        ),
        ReceiverIntent::Volume(value) => volume_command(*value)?,
        ReceiverIntent::Mute(MuteState::On) => "MUON".to_owned(),
        ReceiverIntent::Mute(MuteState::Off) => "MUOFF".to_owned(),
        ReceiverIntent::SoundMode(intent) => sound_mode_command(intent)?,
    };
    AvrCommand::new(value)
}

pub fn parse(line: &str) -> Result<X3800hFrame, AvrProtocolError> {
    match line {
        "PWON" => Ok(X3800hFrame::SystemPower(SystemPower::On)),
        "PWSTANDBY" => Ok(X3800hFrame::SystemPower(SystemPower::Standby)),
        "ZMON" => Ok(X3800hFrame::MainZonePower(ZonePower::On)),
        "ZMOFF" => Ok(X3800hFrame::MainZonePower(ZonePower::Off)),
        "Z2ON" => Ok(X3800hFrame::Zone2Power(ZonePower::On)),
        "Z2OFF" => Ok(X3800hFrame::Zone2Power(ZonePower::Off)),
        "MUON" => Ok(X3800hFrame::Mute(MuteState::On)),
        "MUOFF" => Ok(X3800hFrame::Mute(MuteState::Off)),
        "MV---" => Ok(X3800hFrame::VolumeUnavailable),
        // The AVR periodically reports its configured volume ceiling in this
        // auxiliary form. It is not the current `MV` level.
        _ if line.starts_with("MVMAX ") => Ok(X3800hFrame::VolumeLimit(line.to_owned())),
        _ if line.starts_with("MV") => parse_volume(line).map(X3800hFrame::Volume),
        _ if line.starts_with("SI") => SourceId::new(&line[2..])
            .map(X3800hFrame::Source)
            .map_err(AvrProtocolError::MalformedResponse),
        _ if line.starts_with("MS") && line.len() > 2 => {
            let value = line[2..].to_owned();
            Ok(X3800hFrame::SoundMode(SoundModeStatus {
                id: value.clone(),
                raw: value,
            }))
        }
        _ => Ok(X3800hFrame::Unknown(line.to_owned())),
    }
}

fn volume_command(value: MasterVolume) -> Result<String, AvrProtocolError> {
    match value {
        MasterVolume::Minimum => Ok("MVMIN".to_owned()),
        MasterVolume::DbHalfSteps(step) => {
            if !(MasterVolume::LOWEST_HALF_STEP..=MasterVolume::HIGHEST_HALF_STEP).contains(&step) {
                return Err(AvrProtocolError::InvalidVolume(
                    "volume is outside X3800H range",
                ));
            }
            let native = step + 160;
            let whole = native / 2;
            Ok(if native % 2 == 0 {
                format!("MV{whole:02}")
            } else {
                format!("MV{whole:02}5")
            })
        }
    }
}

fn parse_volume(line: &str) -> Result<MasterVolume, AvrProtocolError> {
    if line == "MVMIN" {
        return Ok(MasterVolume::Minimum);
    }
    let code = &line[2..];
    let native = match code.len() {
        2 => code.parse::<i16>().ok().map(|whole| whole * 2),
        3 if code.ends_with('5') => code[..2].parse::<i16>().ok().map(|whole| whole * 2 + 1),
        _ => None,
    }
    .ok_or(AvrProtocolError::InvalidVolume(
        "invalid X3800H volume frame",
    ))?;
    MasterVolume::db_half_steps(native - 160).map_err(AvrProtocolError::InvalidVolume)
}

fn sound_mode_command(intent: &SoundModeIntent) -> Result<String, AvrProtocolError> {
    let command = match intent {
        SoundModeIntent::Auto => "MSAUTO".to_owned(),
        SoundModeIntent::Direct => "MSDIRECT".to_owned(),
        SoundModeIntent::PureDirect => "MSPURE DIRECT".to_owned(),
        SoundModeIntent::Stereo => "MSSTEREO".to_owned(),
        SoundModeIntent::RecallMovie => "MSMOVIE".to_owned(),
        SoundModeIntent::RecallMusic => "MSMUSIC".to_owned(),
        SoundModeIntent::RecallGame => "MSGAME".to_owned(),
        SoundModeIntent::Select(value)
            if !value.trim().is_empty() && !value.chars().any(char::is_control) =>
        {
            format!("MS{value}")
        }
        SoundModeIntent::Select(_) => {
            return Err(AvrProtocolError::InvalidCommand(
                "sound mode is empty or contains controls",
            ))
        }
    };
    Ok(command)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn main_zone_never_uses_system_power_family() {
        assert_eq!(
            encode(&ReceiverIntent::MainZonePower(ZonePower::On))
                .unwrap()
                .as_str(),
            "ZMON"
        );
        assert_eq!(
            query(&ReceiverIntent::MainZonePower(ZonePower::Off)).as_str(),
            "ZM?"
        );
    }
    #[test]
    fn exact_volume_boundaries_are_enforced() {
        assert_eq!(
            encode(&ReceiverIntent::Volume(MasterVolume::Minimum))
                .unwrap()
                .as_str(),
            "MVMIN"
        );
        assert_eq!(
            encode(&ReceiverIntent::Volume(
                MasterVolume::db_half_steps(36).unwrap()
            ))
            .unwrap()
            .as_str(),
            "MV98"
        );
        assert!(parse("MV985").is_err());
        assert_eq!(
            parse("MV005").unwrap(),
            X3800hFrame::Volume(MasterVolume::db_half_steps(-159).unwrap())
        );
        assert_eq!(
            parse("MVMAX 80").unwrap(),
            X3800hFrame::VolumeLimit("MVMAX 80".into())
        );
    }

    #[test]
    fn source_encoding_uses_canonical_profile_identifier() {
        assert_eq!(
            encode(&ReceiverIntent::Source(SourceId::new("cd").unwrap()))
                .unwrap()
                .as_str(),
            "SICD"
        );
        assert!(encode(&ReceiverIntent::Source(SourceId::new("CUSTOM").unwrap())).is_err());
    }
}
