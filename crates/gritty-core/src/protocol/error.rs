//! The error envelope every command returns on failure.
//!
//! `Result<T, CoreError>` is the only error shape that crosses the IPC
//! boundary. The code set is closed so the UI can switch over it; free-form
//! context goes into `details`.

use std::fmt;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::ids::CorrelationId;

/// Closed set of error classes. Serialized as the variant name (`"NotFound"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
pub enum CoreErrorCode {
    /// Arguments failed validation; `details` names the field. No side effects happened.
    InvalidInput,
    NotFound,
    PermissionDenied,
    /// The requested change conflicts with current state (dirty buffer, busy resource, ...).
    Conflict,
    /// The work was cancelled cooperatively.
    Cancelled,
    Timeout,
    /// Filesystem, process or network failure.
    Io,
    /// A bug, a panic or a serialization failure. Always carries a correlation id when
    /// produced by a command adapter.
    Internal,
    /// The client and the core disagree on the protocol MAJOR version.
    ProtocolMismatch,
}

impl CoreErrorCode {
    /// Whether retrying the same request unchanged may reasonably succeed.
    pub const fn default_retryable(self) -> bool {
        matches!(self, Self::Timeout | Self::Io)
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidInput => "InvalidInput",
            Self::NotFound => "NotFound",
            Self::PermissionDenied => "PermissionDenied",
            Self::Conflict => "Conflict",
            Self::Cancelled => "Cancelled",
            Self::Timeout => "Timeout",
            Self::Io => "Io",
            Self::Internal => "Internal",
            Self::ProtocolMismatch => "ProtocolMismatch",
        }
    }
}

impl fmt::Display for CoreErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The error envelope. See `docs/architecture/overview.md` for the contract.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, thiserror::Error)]
#[serde(rename_all = "camelCase")]
#[error("{code}: {message}")]
pub struct CoreError {
    pub code: CoreErrorCode,
    /// Human-readable, safe to display. Never contains secrets.
    pub message: String,
    /// Structured context (for `InvalidInput`: `{ "field": ..., "reason": ... }`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub details: Option<serde_json::Value>,
    pub retryable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub correlation_id: Option<CorrelationId>,
}

impl CoreError {
    pub fn new(code: CoreErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: None,
            retryable: code.default_retryable(),
            correlation_id: None,
        }
    }

    /// Validation failure for one field. Produces `details: { field, reason }`.
    pub fn invalid_input(field: &str, reason: impl Into<String>) -> Self {
        let reason = reason.into();
        Self::new(CoreErrorCode::InvalidInput, format!("{field}: {reason}"))
            .with_details(serde_json::json!({ "field": field, "reason": reason }))
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(CoreErrorCode::NotFound, message)
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(CoreErrorCode::Conflict, message)
    }

    pub fn cancelled(message: impl Into<String>) -> Self {
        Self::new(CoreErrorCode::Cancelled, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(CoreErrorCode::Internal, message)
    }

    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(details);
        self
    }

    pub fn with_retryable(mut self, retryable: bool) -> Self {
        self.retryable = retryable;
        self
    }

    /// Attach a correlation id, replacing any existing one.
    pub fn with_correlation(mut self, id: CorrelationId) -> Self {
        self.correlation_id = Some(id);
        self
    }

    /// Attach a correlation id only when the error does not carry one yet.
    pub fn or_correlation(mut self, id: &CorrelationId) -> Self {
        self.correlation_id.get_or_insert_with(|| id.clone());
        self
    }
}

impl From<std::io::Error> for CoreError {
    fn from(error: std::io::Error) -> Self {
        Self::new(CoreErrorCode::Io, error.to_string())
            .with_details(serde_json::json!({ "kind": format!("{:?}", error.kind()) }))
    }
}

impl From<serde_json::Error> for CoreError {
    fn from(error: serde_json::Error) -> Self {
        Self::internal(format!("serialization failed: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_serializes_camel_case_and_omits_absent_fields() {
        let error = CoreError::not_found("operation missing");
        let json = serde_json::to_value(&error).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "code": "NotFound",
                "message": "operation missing",
                "retryable": false,
            })
        );
    }

    #[test]
    fn envelope_round_trips_with_details_and_correlation() {
        let id = CorrelationId::new();
        let error = CoreError::invalid_input("steps", "must be between 1 and 10000")
            .with_correlation(id.clone());
        let json = serde_json::to_value(&error).unwrap();
        assert_eq!(json["code"], "InvalidInput");
        assert_eq!(json["details"]["field"], "steps");
        assert_eq!(json["correlationId"], id.as_str());
        let back: CoreError = serde_json::from_value(json).unwrap();
        assert_eq!(back, error);
    }

    #[test]
    fn retryable_defaults_follow_the_code() {
        assert!(CoreError::new(CoreErrorCode::Timeout, "slow").retryable);
        assert!(CoreError::from(std::io::Error::other("disk")).retryable);
        assert!(!CoreError::internal("bug").retryable);
        assert!(
            !CoreError::internal("bug")
                .with_retryable(true)
                .with_retryable(false)
                .retryable
        );
    }

    #[test]
    fn or_correlation_keeps_an_existing_id() {
        let first = CorrelationId::new();
        let second = CorrelationId::new();
        let error = CoreError::internal("x")
            .with_correlation(first.clone())
            .or_correlation(&second);
        assert_eq!(error.correlation_id, Some(first));
        let error = CoreError::internal("x").or_correlation(&second);
        assert_eq!(error.correlation_id, Some(second));
    }

    #[test]
    fn display_is_code_and_message() {
        assert_eq!(CoreError::internal("boom").to_string(), "Internal: boom");
    }
}
