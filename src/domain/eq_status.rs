//! Independent room-correction and audio-processing observations.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EqFeature {
    MultEqXt32,
    DynamicEq,
    DynamicEqReferenceLevel,
    DynamicVolume,
    AudysseyLfc,
    DiracLive,
}
impl EqFeature {
    pub const ALL: [Self; 6] = [
        Self::MultEqXt32,
        Self::DynamicEq,
        Self::DynamicEqReferenceLevel,
        Self::DynamicVolume,
        Self::AudysseyLfc,
        Self::DiracLive,
    ];
    pub const fn label(self) -> &'static str {
        match self {
            Self::MultEqXt32 => "Audyssey MultEQ XT32",
            Self::DynamicEq => "Dynamic EQ",
            Self::DynamicEqReferenceLevel => "Dynamic EQ reference offset",
            Self::DynamicVolume => "Dynamic Volume",
            Self::AudysseyLfc => "Audyssey LFC",
            Self::DiracLive => "Dirac Live",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EqState {
    On,
    Off,
    Configured(String),
    Unavailable(String),
    Unsupported,
    NotApplicable(String),
    Unknown,
}
impl EqState {
    pub fn explanation(&self) -> &str {
        match self {
            Self::On => "On",
            Self::Off => "Off",
            Self::Configured(v) => v,
            Self::Unavailable(v) => v,
            Self::Unsupported => "Unsupported",
            Self::NotApplicable(v) => v,
            Self::Unknown => "EQ status not reported",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EqEvidence {
    pub feature: EqFeature,
    pub response: Option<String>,
    pub error: Option<String>,
    pub elapsed_millis: u128,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EqStatus {
    pub multeq_xt32: EqState,
    pub dynamic_eq: EqState,
    pub dynamic_eq_reference_level: EqState,
    pub dynamic_volume: EqState,
    pub audyssey_lfc: EqState,
    pub dirac_live: EqState,
    pub generation: u64,
    pub freshness: super::Freshness,
    pub evidence: Vec<EqEvidence>,
}
impl Default for EqStatus {
    fn default() -> Self {
        Self {
            multeq_xt32: EqState::Unknown,
            dynamic_eq: EqState::Unknown,
            dynamic_eq_reference_level: EqState::Unknown,
            dynamic_volume: EqState::Unknown,
            audyssey_lfc: EqState::Unknown,
            dirac_live: EqState::Unknown,
            generation: 0,
            freshness: super::Freshness::Unknown,
            evidence: Vec::new(),
        }
    }
}
impl EqStatus {
    pub fn state(&self, feature: EqFeature) -> &EqState {
        match feature {
            EqFeature::MultEqXt32 => &self.multeq_xt32,
            EqFeature::DynamicEq => &self.dynamic_eq,
            EqFeature::DynamicEqReferenceLevel => &self.dynamic_eq_reference_level,
            EqFeature::DynamicVolume => &self.dynamic_volume,
            EqFeature::AudysseyLfc => &self.audyssey_lfc,
            EqFeature::DiracLive => &self.dirac_live,
        }
    }
    pub fn invalidate(&mut self) {
        *self = Self {
            freshness: super::Freshness::Invalidated,
            ..Self::default()
        };
    }

    pub fn record_evidence(&mut self, evidence: EqEvidence) {
        self.evidence.retain(|old| old.feature != evidence.feature);
        self.evidence.push(evidence);
    }

    /// Merge a partial refresh without erasing an authoritative observation
    /// merely because its follow-up query failed.
    pub fn preserve_failed_observations(&mut self, previous: &Self) {
        for evidence in &self.evidence {
            if evidence.error.is_none() {
                continue;
            }
            let previous_state = previous.state(evidence.feature).clone();
            if matches!(previous_state, EqState::Unknown) {
                continue;
            }
            match evidence.feature {
                EqFeature::MultEqXt32 => self.multeq_xt32 = previous_state,
                EqFeature::DynamicEq => self.dynamic_eq = previous_state,
                EqFeature::DynamicEqReferenceLevel => {
                    self.dynamic_eq_reference_level = previous_state
                }
                EqFeature::DynamicVolume => self.dynamic_volume = previous_state,
                EqFeature::AudysseyLfc => self.audyssey_lfc = previous_state,
                EqFeature::DiracLive => self.dirac_live = previous_state,
            }
        }
    }

    /// Direct modes bypass the processing features represented here. This is
    /// a receiver-mode restriction, not evidence that those features are off.
    pub fn apply_mode_restrictions(&mut self, mode: &str) {
        if matches!(mode.to_ascii_uppercase().as_str(), "DIRECT" | "PURE DIRECT") {
            let state = || EqState::NotApplicable(format!("not applicable in {mode} mode"));
            self.multeq_xt32 = state();
            self.dynamic_eq = state();
            self.dynamic_eq_reference_level = state();
            self.dynamic_volume = state();
            self.audyssey_lfc = state();
            self.dirac_live = state();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn features_remain_independent() {
        let s = EqStatus {
            dynamic_eq: EqState::Off,
            ..EqStatus::default()
        };
        assert_eq!(s.state(EqFeature::DynamicEq), &EqState::Off);
        assert_eq!(s.state(EqFeature::DiracLive), &EqState::Unknown);
    }

    #[test]
    fn direct_modes_are_not_reported_as_eq_off() {
        let mut status = EqStatus {
            dynamic_eq: EqState::On,
            ..EqStatus::default()
        };
        status.apply_mode_restrictions("PURE DIRECT");
        assert!(matches!(status.dynamic_eq, EqState::NotApplicable(_)));
    }

    #[test]
    fn failed_refresh_preserves_the_previous_feature_observation() {
        let previous = EqStatus {
            dynamic_eq: EqState::On,
            ..EqStatus::default()
        };
        let mut refresh = EqStatus {
            dynamic_eq: EqState::Unavailable("timeout".into()),
            ..EqStatus::default()
        };
        refresh.record_evidence(EqEvidence {
            feature: EqFeature::DynamicEq,
            response: None,
            error: Some("timeout".into()),
            elapsed_millis: 10,
        });
        refresh.preserve_failed_observations(&previous);
        assert_eq!(refresh.dynamic_eq, EqState::On);
    }
}
