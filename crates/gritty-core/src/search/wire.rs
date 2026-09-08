//! Wire types of the `search.query` command (T080).
//!
//! The domain payloads — [`SearchMatch`], [`SearchSummary`] — live in
//! `gritty-search` beside the code that builds them, the way [`DirectoryPage`]
//! lives in `gritty-workspace`. What this module adds is the request shape,
//! the streaming event's payload and the summary the operation completes with.
//!
//! **There is no response union here, and that is the difference from
//! `workspace.list`.** A listing is bounded — one level, at most 5000 entries —
//! so it can answer inline within a budget and only *sometimes* becomes an
//! operation. A search is open-ended and its output is a stream, so there is no
//! inline case to fall back to: `search.query` always returns the
//! [`Operation`], and matches arrive as `SearchMatchesFound` until it ends.
//!
//! [`DirectoryPage`]: gritty_workspace::DirectoryPage

use gritty_search::{SearchMatch, SearchScope, SearchSummary};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::protocol::ids::{OperationId, WorkspaceId};

/// `search.query`: scan the open workspace for `query`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SearchQueryRequest {
    pub workspace_id: WorkspaceId,
    /// What to look for. Empty or all-whitespace is `InvalidInput` before
    /// anything is created.
    pub query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub options: Option<SearchOptions>,
}

/// How to interpret the query, and what to look in. Every field is optional
/// and the defaults are the plain case: a case-insensitive literal, matched
/// against contents and paths, skipping generated directories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SearchOptions {
    /// Default `false`: an editor's find-in-files is case-insensitive until
    /// asked otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub case_sensitive: Option<bool>,
    /// Require word boundaries around the match. Default `false`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub whole_word: Option<bool>,
    /// Treat `query` as a regular expression instead of a literal. Default
    /// `false`; a pattern that does not compile is `InvalidInput`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub use_regex: Option<bool>,
    /// Contents, paths, or both. Default `both`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub scope: Option<SearchScope>,
    /// Also scan directories T070 flags ignored. Default `false`.
    ///
    /// Offered rather than assumed, mirroring the explorer's stance: such
    /// directories are listed with `ignoredHint: true` and never hidden, so
    /// search is never blind to what the tree already shows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub include_ignored: Option<bool>,
}

/// `SearchMatchesFound`: one bounded batch of matches for a running search.
///
/// Not `OperationProgress`, whose `message` is a human string rather than
/// match data, and not one event per match: batches are throttled and the
/// queue behind them is bounded (rule 8).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SearchMatchesFoundPayload {
    pub operation_id: OperationId,
    pub matches: Vec<SearchMatch>,
    /// Matches found but dropped before this batch because they arrived faster
    /// than they could be delivered. Non-zero means matches are genuinely
    /// missing and the UI must say so rather than presenting a gap as a
    /// complete result — exactly `TerminalOutputPayload.truncated_bytes`.
    /// Always present: an absent field would read as "no truncation" whether
    /// or not the core meant to say that.
    #[serde(default)]
    pub truncated_matches: u64,
}

/// What `OperationCompleted.result` carries for a `search.query` operation:
/// a summary, never the match list, which already streamed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SearchCompletion {
    pub summary: SearchSummary,
    /// Matches dropped across the whole search (the sum of every batch's
    /// `truncatedMatches`, plus any that never reached a batch).
    #[serde(default)]
    pub truncated_matches: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_needs_only_a_workspace_and_a_query() {
        let request: SearchQueryRequest = serde_json::from_value(serde_json::json!({
            "workspaceId": "w-1",
            "query": "needle",
        }))
        .expect("minimal request");
        assert_eq!(request.query, "needle");
        assert_eq!(request.options, None);
    }

    #[test]
    fn options_are_camel_case_and_individually_optional() {
        let request: SearchQueryRequest = serde_json::from_value(serde_json::json!({
            "workspaceId": "w-1",
            "query": "needle",
            "options": { "useRegex": true, "scope": "filenames" },
        }))
        .expect("partial options");
        let options = request.options.expect("options");
        assert_eq!(options.use_regex, Some(true));
        assert_eq!(options.scope, Some(SearchScope::Filenames));
        assert_eq!(options.case_sensitive, None);
        assert_eq!(options.include_ignored, None);
    }

    #[test]
    fn a_batch_always_carries_its_truncation_count() {
        let payload = SearchMatchesFoundPayload {
            operation_id: OperationId::from_raw("op-1"),
            matches: Vec::new(),
            truncated_matches: 0,
        };
        assert_eq!(
            serde_json::to_value(&payload).unwrap(),
            serde_json::json!({
                "operationId": "op-1",
                "matches": [],
                "truncatedMatches": 0,
            })
        );
    }
}
