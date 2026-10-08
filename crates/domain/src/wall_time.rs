//! Wall-clock time as a plain value.
//!
//! A budget window must survive a restart, so it is measured in wall-clock
//! time, which `MonotonicMillis` is not. This type holds a reading and never
//! takes one: a `Clock` port supplies it, so domain tests need no runtime and a
//! test can set the time.

use std::time::Duration;

/// Milliseconds since the Unix epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct WallTime(pub u64);

impl WallTime {
    pub fn from_millis(millis: u64) -> Self {
        Self(millis)
    }

    pub fn as_millis(self) -> u64 {
        self.0
    }

    /// This time minus `duration`, or the epoch when that would be earlier.
    pub fn saturating_sub(self, duration: Duration) -> Self {
        Self(self.0.saturating_sub(millis(duration)))
    }

    /// This time plus `duration`, or the latest time when that would overflow.
    pub fn saturating_add(self, duration: Duration) -> Self {
        Self(self.0.saturating_add(millis(duration)))
    }

    /// How long after `earlier` this time is, or zero when it is not later.
    pub fn since(self, earlier: Self) -> Duration {
        Duration::from_millis(self.0.saturating_sub(earlier.0))
    }
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_order_by_their_milliseconds() {
        assert!(WallTime(1) < WallTime(2));
        assert_eq!(WallTime::from_millis(5).as_millis(), 5);
    }

    #[test]
    fn subtraction_stops_at_the_epoch() {
        let t = WallTime(1_000);
        assert_eq!(t.saturating_sub(Duration::from_millis(400)), WallTime(600));
        assert_eq!(t.saturating_sub(Duration::from_secs(60)), WallTime(0));
    }

    #[test]
    fn addition_stops_at_the_latest_time() {
        assert_eq!(
            WallTime(1).saturating_add(Duration::from_millis(2)),
            WallTime(3)
        );
        assert_eq!(
            WallTime(u64::MAX).saturating_add(Duration::from_secs(1)),
            WallTime(u64::MAX)
        );
        // A duration longer than u64 milliseconds saturates rather than wraps.
        assert_eq!(
            WallTime(1).saturating_add(Duration::from_secs(u64::MAX)),
            WallTime(u64::MAX)
        );
    }

    #[test]
    fn since_is_zero_for_a_time_that_is_not_later() {
        assert_eq!(
            WallTime(900).since(WallTime(400)),
            Duration::from_millis(500)
        );
        assert_eq!(WallTime(400).since(WallTime(900)), Duration::ZERO);
    }
}
