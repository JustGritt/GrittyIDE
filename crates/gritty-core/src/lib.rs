//! Gritty core: the UI-agnostic application domain.
//!
//! This crate must never depend on Tauri or any UI toolkit. The desktop app
//! (`apps/desktop/src-tauri`) is a thin composition root that wires this crate
//! into Tauri commands and events. See `AGENTS.md` and
//! `docs/architecture/overview.md`.

use serde::{Deserialize, Serialize};

/// Version of the core crate, taken from `Cargo.toml` at compile time.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Basic runtime facts about the core, used by the T000 health round-trip and
/// as Phase 0 evidence. Extended or replaced by the T010 protocol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthReport {
    /// `CARGO_PKG_VERSION` of `gritty-core`.
    pub core_version: String,
    /// `std::env::consts::OS` of the running binary.
    pub os: String,
    /// `std::env::consts::ARCH` of the running binary.
    pub arch: String,
    /// `true` when compiled with debug assertions (development builds).
    pub debug: bool,
}

/// Report the core's identity and build facts.
pub fn health() -> HealthReport {
    HealthReport {
        core_version: VERSION.to_owned(),
        os: std::env::consts::OS.to_owned(),
        arch: std::env::consts::ARCH.to_owned(),
        debug: cfg!(debug_assertions),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_reports_crate_version_and_platform() {
        let report = health();
        assert_eq!(report.core_version, env!("CARGO_PKG_VERSION"));
        assert!(!report.os.is_empty());
        assert!(!report.arch.is_empty());
    }

    #[test]
    fn health_report_serde_round_trip_uses_camel_case() {
        let report = health();
        let json = serde_json::to_string(&report).expect("serialize");
        assert!(json.contains("\"coreVersion\""));
        let back: HealthReport = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, report);
    }
}
