//! Evidence-backed main-zone event reduction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MainZoneEvent {
    Power(String),
    Input(String),
    Volume(String),
    Mute(String),
    Surround(String),
    Unknown(String),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    Unknown,
    Live,
    Invalidated,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateAuthority {
    Unconfirmed,
    Event,
    Authoritative,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MainZoneState {
    pub power: Option<String>,
    pub input: Option<String>,
    pub volume: Option<String>,
    pub mute: Option<String>,
    pub surround: Option<String>,
    pub freshness: Freshness,
    pub authority: StateAuthority,
}
impl Default for MainZoneState {
    fn default() -> Self {
        Self {
            power: None,
            input: None,
            volume: None,
            mute: None,
            surround: None,
            freshness: Freshness::Unknown,
            authority: StateAuthority::Unconfirmed,
        }
    }
}
impl MainZoneState {
    pub fn invalidate(&mut self) {
        self.power = None;
        self.input = None;
        self.volume = None;
        self.mute = None;
        self.surround = None;
        self.freshness = Freshness::Invalidated;
        self.authority = StateAuthority::Unconfirmed;
    }
    pub fn apply(&mut self, event: MainZoneEvent) {
        match event {
            MainZoneEvent::Power(v) => self.power = Some(v),
            MainZoneEvent::Input(v) => self.input = Some(v),
            MainZoneEvent::Volume(v) => self.volume = Some(v),
            MainZoneEvent::Mute(v) => self.mute = Some(v),
            MainZoneEvent::Surround(v) => self.surround = Some(v),
            MainZoneEvent::Unknown(_) => return,
        };
        self.freshness = Freshness::Live;
        self.authority = StateAuthority::Event;
    }

    pub fn apply_authoritative(&mut self, event: MainZoneEvent) {
        self.apply(event);
        if self.freshness == Freshness::Live {
            self.authority = StateAuthority::Authoritative;
        }
    }
}
pub fn parse_event(line: &str) -> MainZoneEvent {
    if line == "PWON" {
        MainZoneEvent::Power("on".into())
    } else if line == "PWSTANDBY" {
        MainZoneEvent::Power("standby".into())
    } else if let Ok(v) = crate::response::parse_input(line) {
        MainZoneEvent::Input(v)
    } else if let Ok(v) = crate::response::parse_volume(line) {
        MainZoneEvent::Volume(v)
    } else if let Ok(v) = crate::response::parse_mute(line) {
        MainZoneEvent::Mute(v)
    } else if let Ok(v) = crate::response::parse_surround(line) {
        MainZoneEvent::Surround(v)
    } else {
        MainZoneEvent::Unknown(line.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_validated_events_and_preserves_unknown_lines() {
        assert_eq!(parse_event("PWON"), MainZoneEvent::Power("on".into()));
        assert_eq!(parse_event("SICD"), MainZoneEvent::Input("CD".into()));
        assert_eq!(
            parse_event("MV805"),
            MainZoneEvent::Volume("code 805 (0.5 dB)".into())
        );
        assert_eq!(
            parse_event("MVMAX 615"),
            MainZoneEvent::Unknown("MVMAX 615".into())
        );
    }

    #[test]
    fn reducer_supports_partial_ordered_updates_and_invalidation() {
        let mut state = MainZoneState::default();
        state.apply(MainZoneEvent::Input("CD".into()));
        assert_eq!(state.input.as_deref(), Some("CD"));
        assert_eq!(state.power, None);
        assert_eq!(state.authority, StateAuthority::Event);
        state.invalidate();
        assert_eq!(state.freshness, Freshness::Invalidated);
        assert_eq!(state.input, None);
        state.apply_authoritative(MainZoneEvent::Power("on".into()));
        assert_eq!(state.authority, StateAuthority::Authoritative);
    }
}
