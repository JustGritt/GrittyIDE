//! `core.health`: identity and build facts, plus the protocol version the
//! client checks before issuing any other command.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::protocol::PROTOCOL_VERSION;

/// Facts only the desktop shell knows; passed in by the composition root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ShellInfo {
    /// Version of the desktop application package.
    pub app_version: String,
    /// Name of the shell runtime, e.g. `"tauri"`.
    pub runtime: String,
    /// Version of the shell runtime crate.
    pub runtime_version: String,
    /// Version string of the system WebView, when detectable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub webview_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct HealthReport {
    /// [`PROTOCOL_VERSION`] of the running core.
    pub protocol_version: String,
    /// `CARGO_PKG_VERSION` of `gritty-core`.
    pub core_version: String,
    /// `std::env::consts::OS` of the running binary.
    pub os: String,
    /// `std::env::consts::ARCH` of the running binary.
    pub arch: String,
    /// `true` when compiled with debug assertions (development builds).
    pub debug: bool,
    pub shell: ShellInfo,
}

/// Report the core's identity and build facts.
pub fn health(shell: ShellInfo) -> HealthReport {
    HealthReport {
        protocol_version: PROTOCOL_VERSION.to_owned(),
        core_version: crate::VERSION.to_owned(),
        os: std::env::consts::OS.to_owned(),
        arch: std::env::consts::ARCH.to_owned(),
        debug: cfg!(debug_assertions),
        shell,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shell() -> ShellInfo {
        ShellInfo {
            app_version: "0.1.0".into(),
            runtime: "test".into(),
            runtime_version: "0".into(),
            webview_version: None,
        }
    }

    #[test]
    fn health_reports_versions_and_platform() {
        let report = health(shell());
        assert_eq!(report.protocol_version, PROTOCOL_VERSION);
        assert_eq!(report.core_version, env!("CARGO_PKG_VERSION"));
        assert!(!report.os.is_empty());
        assert!(!report.arch.is_empty());
        assert_eq!(report.shell.runtime, "test");
    }

    #[test]
    fn health_report_serde_round_trip_uses_camel_case() {
        let report = health(shell());
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("\"protocolVersion\""));
        assert!(json.contains("\"coreVersion\""));
        assert!(
            !json.contains("webviewVersion"),
            "absent optional fields are omitted"
        );
        let back: HealthReport = serde_json::from_str(&json).unwrap();
        assert_eq!(back, report);
    }
}
