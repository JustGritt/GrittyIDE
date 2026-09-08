//! What was asked for, and the compiled matcher it becomes.
//!
//! Compilation is the fast validation `search.query` runs *before* an
//! operation exists (T080 §2, mirroring `StartDemoRequest::validate`): an
//! empty query and an uncompilable pattern are both `InvalidQuery` here, and
//! nothing is created, started or emitted.

use std::sync::Arc;

use grep_regex::{RegexMatcher, RegexMatcherBuilder};
use gritty_workspace::{MAX_DOCUMENT_BYTES, WorkspacePath, is_ignored};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::error::SearchError;

/// Longest query accepted, in Unicode scalar values. A bound on untrusted
/// input (rule 13) before the regex engine ever sees it.
pub const MAX_QUERY_CHARS: usize = 1_024;

/// Largest file the walk will read for content matching.
///
/// The same 64 MiB `documents.open` refuses at ([`MAX_DOCUMENT_BYTES`]), and
/// deliberately not a second number: "too large for Gritty to read" must mean
/// one thing. The 12 MiB `weird-files/generated/large-text.txt` is well under
/// it and *is* scanned.
pub const MAX_FILE_BYTES: u64 = MAX_DOCUMENT_BYTES;

/// Compiled-pattern size bounds. A user-typed regex is untrusted input; the
/// regex engine is linear-time, so the risk is compilation blow-up rather than
/// backtracking, and these are what bound it.
const REGEX_SIZE_LIMIT: usize = 10 * (1 << 20);
const REGEX_DFA_SIZE_LIMIT: usize = 10 * (1 << 20);
const REGEX_NEST_LIMIT: u32 = 64;

/// What a query is matched against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
pub enum SearchScope {
    /// File contents only.
    Content,
    /// Workspace-relative paths only; no file is opened.
    Filenames,
    /// Both, which is the default.
    #[default]
    Both,
}

impl SearchScope {
    pub const fn includes_content(self) -> bool {
        matches!(self, Self::Content | Self::Both)
    }

    pub const fn includes_filenames(self) -> bool {
        matches!(self, Self::Filenames | Self::Both)
    }
}

/// Decides whether a path is in a generated or dependency subtree.
///
/// T070's [`is_ignored`] is the production answer; it is injected rather than
/// called directly so a test can prove the walk *consults* it per directory
/// before descending, which is the property that matters.
pub type IgnorePredicate = Arc<dyn Fn(&WorkspacePath) -> bool + Send + Sync>;

/// The default predicate: T070's, so "ignored" means one thing in the
/// explorer, the watcher and search (ADR-022 decision 2).
pub fn default_ignore_predicate() -> IgnorePredicate {
    Arc::new(is_ignored)
}

/// A search as the caller expressed it.
///
/// The defaults are the plain case, and `SearchScope`'s own default is `Both`:
/// a case-insensitive literal matched against contents and paths, skipping
/// generated directories.
#[derive(Debug, Clone, Default)]
pub struct Query {
    pub text: String,
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub use_regex: bool,
    pub scope: SearchScope,
    /// Scan directories the ignore predicate flags, too. Mirrors the
    /// explorer's stance: such directories are shown with `ignoredHint`, never
    /// hidden, so search is never blind to what the tree already shows.
    pub include_ignored: bool,
}

impl Query {
    /// A literal, case-insensitive, both-scope query. Test and caller sugar.
    pub fn literal(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ..Self::default()
        }
    }

    /// Validate and compile. Every failure here is `InvalidQuery`, and the
    /// caller must not have created anything yet.
    pub fn compile(&self) -> Result<CompiledQuery, SearchError> {
        if self.text.trim().is_empty() {
            return Err(SearchError::invalid_query(
                "query",
                "must not be empty or only whitespace",
            ));
        }
        let length = self.text.chars().count();
        if length > MAX_QUERY_CHARS {
            return Err(SearchError::invalid_query(
                "query",
                format!("is {length} characters, above the {MAX_QUERY_CHARS}-character limit"),
            ));
        }
        // A line-oriented search cannot express a pattern that spans lines,
        // and saying so is better than matching nothing forever.
        if self.text.contains('\n') || self.text.contains('\r') {
            return Err(SearchError::invalid_query(
                "query",
                "must not contain a line break (search is line-oriented)",
            ));
        }

        let matcher = RegexMatcherBuilder::new()
            .case_insensitive(!self.case_sensitive)
            .word(self.whole_word)
            .fixed_strings(!self.use_regex)
            // The matcher's `m` flag, so `^` and `$` anchor to lines rather
            // than to the whole haystack. Whether a *search* may span lines is
            // the searcher's separate `multi_line`, which stays off.
            .multi_line(true)
            .dot_matches_new_line(false)
            // Sets the matcher's line terminator to CRLF *and* makes `$`
            // treat `\r\n` as one, so `foo$` matches in the `encoding/crlf.txt`
            // fixture as well as in a LF file. The searcher is configured with
            // the same terminator; a mismatch is a hard error there, which is
            // exactly the kind of disagreement worth failing on.
            .crlf(true)
            .size_limit(REGEX_SIZE_LIMIT)
            .dfa_size_limit(REGEX_DFA_SIZE_LIMIT)
            .nest_limit(REGEX_NEST_LIMIT)
            .build(&self.text)
            .map_err(|error| SearchError::invalid_query("query", error.to_string()))?;

        Ok(CompiledQuery {
            matcher,
            scope: self.scope,
            include_ignored: self.include_ignored,
            ignored: default_ignore_predicate(),
            max_file_bytes: MAX_FILE_BYTES,
        })
    }
}

/// A validated query with its matcher built once, ready for the walk.
#[derive(Clone)]
pub struct CompiledQuery {
    matcher: RegexMatcher,
    scope: SearchScope,
    include_ignored: bool,
    ignored: IgnorePredicate,
    max_file_bytes: u64,
}

impl CompiledQuery {
    pub fn matcher(&self) -> &RegexMatcher {
        &self.matcher
    }

    pub const fn scope(&self) -> SearchScope {
        self.scope
    }

    pub const fn include_ignored(&self) -> bool {
        self.include_ignored
    }

    pub fn ignored(&self) -> &IgnorePredicate {
        &self.ignored
    }

    pub const fn max_file_bytes(&self) -> u64 {
        self.max_file_bytes
    }

    /// Replace T070's ignore predicate (tests only in practice).
    #[must_use]
    pub fn with_ignore_predicate(mut self, predicate: IgnorePredicate) -> Self {
        self.ignored = predicate;
        self
    }

    /// Lower the per-file byte bound, so a test can exercise `too_large`
    /// without writing a 64 MiB file.
    #[must_use]
    pub const fn with_max_file_bytes(mut self, bytes: u64) -> Self {
        self.max_file_bytes = bytes;
        self
    }
}

impl std::fmt::Debug for CompiledQuery {
    /// Deliberately omits the pattern. A query is user input and can carry a
    /// secret the user pasted; `security.md` keeps content out of logs.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompiledQuery")
            .field("scope", &self.scope)
            .field("include_ignored", &self.include_ignored)
            .field("max_file_bytes", &self.max_file_bytes)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_or_whitespace_query_never_compiles() {
        for text in ["", " ", "\t  ", "   "] {
            let error = Query::literal(text).compile().unwrap_err();
            assert!(
                matches!(error, SearchError::InvalidQuery { field: "query", .. }),
                "{text:?} -> {error:?}"
            );
        }
    }

    #[test]
    fn an_uncompilable_regex_is_invalid_input_not_a_panic() {
        let query = Query {
            use_regex: true,
            ..Query::literal("foo(")
        };
        let error = query.compile().unwrap_err();
        assert!(matches!(
            error,
            SearchError::InvalidQuery { field: "query", .. }
        ));
    }

    #[test]
    fn the_same_pattern_is_a_literal_when_regex_is_off() {
        // `foo(` is a syntax error as a regex and a fine literal.
        assert!(Query::literal("foo(").compile().is_ok());
    }

    #[test]
    fn an_overlong_query_is_refused_before_the_engine_sees_it() {
        let error = Query::literal("x".repeat(MAX_QUERY_CHARS + 1))
            .compile()
            .unwrap_err();
        let SearchError::InvalidQuery { reason, .. } = error else {
            panic!("expected InvalidQuery");
        };
        assert!(reason.contains("above the"), "{reason}");
    }

    #[test]
    fn a_query_spanning_lines_is_refused() {
        for text in ["foo\nbar", "foo\r\nbar"] {
            let error = Query::literal(text).compile().unwrap_err();
            let SearchError::InvalidQuery { reason, .. } = error else {
                panic!("expected InvalidQuery for {text:?}");
            };
            assert!(reason.contains("line break"), "{reason}");
        }
    }

    #[test]
    fn scopes_serialize_lowercase_and_default_to_both() {
        assert_eq!(
            serde_json::to_string(&SearchScope::Filenames).unwrap(),
            "\"filenames\""
        );
        assert_eq!(SearchScope::default(), SearchScope::Both);
        assert!(SearchScope::Both.includes_content());
        assert!(SearchScope::Both.includes_filenames());
        assert!(!SearchScope::Filenames.includes_content());
        assert!(!SearchScope::Content.includes_filenames());
    }

    #[test]
    fn the_debug_form_never_carries_the_pattern() {
        let compiled = Query::literal("hunter2").compile().unwrap();
        let shown = format!("{compiled:?}");
        assert!(!shown.contains("hunter2"), "{shown}");
    }
}
