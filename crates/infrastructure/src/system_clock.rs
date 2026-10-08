//! The operating system's clock as the application's [`Clock`].

use denon_avr_application::Clock;
use denon_avr_domain::WallTime;
use std::time::{SystemTime, UNIX_EPOCH};

/// Reads [`SystemTime`]. A clock set before the epoch reads as the epoch.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> WallTime {
        let since_epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        WallTime::from_millis(u64::try_from(since_epoch.as_millis()).unwrap_or(u64::MAX))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use denon_avr_application::Clock;

    #[test]
    fn the_system_clock_reads_milliseconds_since_the_unix_epoch() {
        let before = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let now = SystemClock.now();
        let after = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        assert!(before <= now.as_millis() && now.as_millis() <= after);
        // Not seconds, and not a clock that was never set.
        assert!(now.as_millis() > 1_700_000_000_000);
    }

    #[test]
    fn it_is_usable_as_the_shared_clock() {
        let clock: denon_avr_application::SharedClock = std::sync::Arc::new(SystemClock);
        assert!(clock.now().as_millis() > 0);
    }
}
