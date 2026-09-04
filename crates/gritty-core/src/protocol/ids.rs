//! Newtype identifiers shared across the protocol.
//!
//! One generation scheme for every id: UUID v7 (RFC 9562), hyphenated
//! lowercase. v7 ids sort by creation time, which keeps later persistence
//! (T060) and log correlation simple, and `uuid` is already part of the
//! dependency tree. On the TypeScript side each id is a branded `string`, so
//! a `WorkspaceId` cannot be passed where an `OperationId` is expected.

use std::fmt;

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

fn new_uuid_v7() -> String {
    Uuid::now_v7().hyphenated().to_string()
}

macro_rules! string_id {
    ($(#[$doc:meta])* $name:ident => $brand:literal) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
        pub struct $name(#[ts(type = $brand)] String);

        impl $name {
            /// Generate a fresh, time-ordered id.
            pub fn new() -> Self {
                Self(new_uuid_v7())
            }

            /// Wrap an id received from outside. No validation happens here:
            /// lookups reject unknown ids with `NotFound`.
            pub fn from_raw(raw: impl Into<String>) -> Self {
                Self(raw.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }

            /// `true` when the id has the shape this scheme generates.
            pub fn is_well_formed(&self) -> bool {
                Uuid::try_parse(&self.0).is_ok()
            }
        }

        impl Default for $name {
            /// A fresh id; equivalent to [`Self::new`].
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl From<$name> for String {
            fn from(id: $name) -> Self {
                id.0
            }
        }
    };
}

string_id! {
    /// Identifies an open workspace (T020).
    WorkspaceId => "string & { readonly __brand: 'WorkspaceId' }"
}

string_id! {
    /// Identifies an asynchronous operation in the registry.
    OperationId => "string & { readonly __brand: 'OperationId' }"
}

string_id! {
    /// Identifies an open document (T030).
    DocumentId => "string & { readonly __brand: 'DocumentId' }"
}

string_id! {
    /// Identifies a terminal session (T090).
    TerminalId => "string & { readonly __brand: 'TerminalId' }"
}

string_id! {
    /// Identifies an agent run (T300); used by [`super::actor::Actor::Agent`].
    AgentRunId => "string & { readonly __brand: 'AgentRunId' }"
}

string_id! {
    /// Ties a command invocation, its operation, its events and its errors together.
    CorrelationId => "string & { readonly __brand: 'CorrelationId' }"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_uuid_v7_hyphenated_lowercase() {
        let id = OperationId::new();
        let parsed = Uuid::try_parse(id.as_str()).expect("valid uuid");
        assert_eq!(parsed.get_version_num(), 7);
        assert_eq!(id.as_str().len(), 36);
        assert_eq!(id.as_str(), id.as_str().to_lowercase());
        assert!(id.is_well_formed());
    }

    #[test]
    fn ids_are_unique_and_time_ordered() {
        let first = OperationId::new();
        let second = OperationId::new();
        assert_ne!(first, second);
        assert!(first <= second, "v7 ids sort by creation time");
    }

    #[test]
    fn ids_serialize_as_plain_strings() {
        let id = WorkspaceId::from_raw("ws-1");
        assert_eq!(serde_json::to_string(&id).unwrap(), "\"ws-1\"");
        let back: WorkspaceId = serde_json::from_str("\"ws-1\"").unwrap();
        assert_eq!(back, id);
        assert!(!back.is_well_formed());
        assert_eq!(back.to_string(), "ws-1");
    }
}
