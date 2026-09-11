//! Deterministic AVR transmission timing policy.
//!
//! This policy is runtime-free so scenario tests can advance logical time
//! without sleeping. The infrastructure actor may use any monotonic runtime
//! clock to drive the same rules.

use crate::MonotonicMillis;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransmissionSchedule {
    pub minimum_interval_ms: u64,
    pub power_on_quiet_ms: u64,
    last_transmission: Option<MonotonicMillis>,
    quiet_until: Option<MonotonicMillis>,
}

impl Default for TransmissionSchedule {
    fn default() -> Self {
        Self {
            minimum_interval_ms: 50,
            power_on_quiet_ms: 1_000,
            last_transmission: None,
            quiet_until: None,
        }
    }
}

impl TransmissionSchedule {
    pub fn next_allowed_at(&self, now: MonotonicMillis) -> MonotonicMillis {
        let interval_due = self
            .last_transmission
            .map(|last| MonotonicMillis(last.0.saturating_add(self.minimum_interval_ms)))
            .unwrap_or(now);
        let quiet_due = self.quiet_until.unwrap_or(now);
        MonotonicMillis(interval_due.0.max(quiet_due.0))
    }

    pub fn record(&mut self, command: &str, sent_at: MonotonicMillis) {
        self.last_transmission = Some(sent_at);
        if command == "PWON" {
            self.quiet_until = Some(MonotonicMillis(
                sent_at.0.saturating_add(self.power_on_quiet_ms),
            ));
        } else if self.quiet_until.is_some_and(|until| sent_at >= until) {
            self.quiet_until = None;
        }
    }

    pub fn last_transmission(&self) -> Option<MonotonicMillis> {
        self.last_transmission
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logical_time_enforces_global_spacing_and_power_quiet() {
        let mut schedule = TransmissionSchedule::default();
        assert_eq!(
            schedule.next_allowed_at(MonotonicMillis(0)),
            MonotonicMillis(0)
        );
        schedule.record("PWON", MonotonicMillis(0));
        assert_eq!(
            schedule.next_allowed_at(MonotonicMillis(999)),
            MonotonicMillis(1_000)
        );
        assert_eq!(
            schedule.next_allowed_at(MonotonicMillis(1_000)),
            MonotonicMillis(1_000)
        );
        schedule.record("MV?", MonotonicMillis(1_000));
        assert_eq!(
            schedule.next_allowed_at(MonotonicMillis(1_000)),
            MonotonicMillis(1_050)
        );
    }

    #[test]
    fn custom_intervals_are_deterministic_without_wall_clock_sleep() {
        let mut schedule = TransmissionSchedule {
            minimum_interval_ms: 7,
            power_on_quiet_ms: 13,
            ..TransmissionSchedule::default()
        };
        schedule.record("MV?", MonotonicMillis(20));
        assert_eq!(
            schedule.next_allowed_at(MonotonicMillis(26)),
            MonotonicMillis(27)
        );
    }

    #[test]
    fn exact_fifty_millisecond_boundary_is_allowed_but_forty_nine_is_not() {
        let mut schedule = TransmissionSchedule::default();
        schedule.record("MV?", MonotonicMillis(1_000));
        assert_eq!(
            schedule.next_allowed_at(MonotonicMillis(1_049)),
            MonotonicMillis(1_050)
        );
        assert_eq!(
            schedule.next_allowed_at(MonotonicMillis(1_050)),
            MonotonicMillis(1_050)
        );
    }
}
