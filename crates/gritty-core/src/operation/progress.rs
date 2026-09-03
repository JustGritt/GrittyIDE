//! Bounded progress event frequency (addendum §3.3 backpressure).
//!
//! Every progress report updates the operation snapshot, but at most one
//! `OperationProgress` event per [`MIN_PROGRESS_INTERVAL`] reaches the
//! renderer per operation. The first report is always delivered; the terminal
//! event that follows carries the final state, so dropped intermediate
//! reports never leave the UI stale.

use std::time::Duration;

use tokio::time::Instant;

/// Minimum spacing between two `OperationProgress` events of one operation.
pub const MIN_PROGRESS_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Debug)]
pub struct Throttle {
    min_interval: Duration,
    last_emit: Option<Instant>,
}

impl Throttle {
    pub const fn new(min_interval: Duration) -> Self {
        Self {
            min_interval,
            last_emit: None,
        }
    }

    /// Whether an event may be emitted `now`; records the emission when allowed.
    pub fn allow(&mut self, now: Instant) -> bool {
        let allowed = self
            .last_emit
            .is_none_or(|last| now.duration_since(last) >= self.min_interval);
        if allowed {
            self.last_emit = Some(now);
        }
        allowed
    }
}

impl Default for Throttle {
    fn default() -> Self {
        Self::new(MIN_PROGRESS_INTERVAL)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_report_is_always_allowed_then_spaced() {
        let mut throttle = Throttle::new(Duration::from_millis(50));
        let start = Instant::now();
        assert!(throttle.allow(start));
        assert!(!throttle.allow(start + Duration::from_millis(10)));
        assert!(!throttle.allow(start + Duration::from_millis(49)));
        assert!(throttle.allow(start + Duration::from_millis(50)));
        assert!(!throttle.allow(start + Duration::from_millis(60)));
        assert!(throttle.allow(start + Duration::from_millis(120)));
    }
}
