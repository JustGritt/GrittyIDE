//! Wire-level conventions shared by every command and event.
//!
//! Everything in this module (and in [`crate::event`] / [`crate::operation`])
//! that derives [`ts_rs::TS`] is exported to `packages/protocol/src/generated`
//! by the `export_bindings` test (`cargo test -p gritty-core`). Rust owns the
//! schema; the TypeScript side is generated and checked for freshness in CI.
//!
//! Conventions (see `docs/architecture/overview.md`):
//! - JSON field names are `camelCase`; `Option` fields that are `None` are
//!   omitted and exported as optional (`field?: T`).
//! - Identifiers are opaque strings generated as UUID v7 ([`ids`]).
//! - Timestamps are milliseconds since the Unix epoch ([`time::Timestamp`]).
//! - 64-bit integers are exported as `number`; values stay below 2^53.

pub mod actor;
pub mod error;
pub mod ids;
pub mod time;

#[cfg(test)]
mod export;

/// Version of the command/event/error contract, `MAJOR.MINOR.PATCH`.
///
/// Bump MAJOR for breaking shape changes, MINOR for additive ones. The client
/// refuses to talk to a core whose MAJOR differs from its own.
pub const PROTOCOL_VERSION: &str = "1.3.0";

/// The single Tauri event channel every core event envelope is delivered on.
/// The client demultiplexes by the envelope's `name`.
pub const CORE_EVENT_CHANNEL: &str = "gritty://core/event";

/// Major component of a `MAJOR.MINOR.PATCH` version string, if parseable.
pub fn major_version(version: &str) -> Option<u64> {
    version.split('.').next()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn major_version_parses_leading_component() {
        assert_eq!(major_version("1.0.0"), Some(1));
        assert_eq!(major_version("12.3.4-beta"), Some(12));
        assert_eq!(major_version(""), None);
        assert_eq!(major_version("x.1"), None);
    }

    #[test]
    fn protocol_version_is_semver_like() {
        assert!(major_version(PROTOCOL_VERSION).is_some());
        assert_eq!(PROTOCOL_VERSION.split('.').count(), 3);
    }

    #[test]
    fn event_channel_uses_only_characters_tauri_accepts() {
        assert!(
            CORE_EVENT_CHANNEL
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '/' | ':' | '_'))
        );
    }
}
