//! Protocol timestamps: milliseconds since the Unix epoch.
//!
//! A plain integer keeps the core free of date/time crates and maps directly
//! to `new Date(ms)` in the WebView. Exported to TypeScript as `number`.

use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
pub struct Timestamp(u64);

impl Timestamp {
    /// The current wall-clock time. A clock before 1970 yields `0`.
    pub fn now() -> Self {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis())
            .unwrap_or(0);
        Self(u64::try_from(millis).unwrap_or(u64::MAX))
    }

    pub const fn from_millis(millis: u64) -> Self {
        Self(millis)
    }

    pub const fn as_millis(self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn now_is_after_2026() {
        let jan_2026 = 1_767_225_600_000;
        assert!(Timestamp::now().as_millis() > jan_2026);
    }

    #[test]
    fn serializes_as_integer() {
        let ts = Timestamp::from_millis(42);
        assert_eq!(serde_json::to_string(&ts).unwrap(), "42");
        let back: Timestamp = serde_json::from_str("42").unwrap();
        assert_eq!(back, ts);
    }
}
