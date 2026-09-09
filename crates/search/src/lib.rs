//! Gritty search (T080): a cancellable live walk of the open workspace,
//! matched in-process, streamed as it goes.
//!
//! This crate knows nothing about Tauri, the core protocol or the operation
//! model. `gritty-core` owns the command, the operation and the batching of
//! results into events; this crate owns the walk, the ignore consultation and
//! the matching, the way `gritty-workspace` owns `list_directory` and
//! `gritty-git` owns `status`.
//!
//! ## The shape of the decision (ADR-022)
//!
//! Search runs **in-process, on the crates ripgrep itself is built from** —
//! `grep-searcher` and `grep-regex` for matching, `ignore` for the walk — and
//! never spawns an `rg` process. Cancellation is then free: the [`Sink`] is
//! ours, so [`crate::search`]'s `cancelled` closure is checked in our own code
//! between files and inside one, exactly as `list_directory` already does.
//!
//! Two constraints ADR-022 adds, both about not saying two different things:
//!
//! - `ignore` is taken **for its walker only, with every gitignore source
//!   turned off**. What Gritty calls "ignored" is T070's `is_ignored` — eleven
//!   directory names — and it stays one thing across the explorer, the watcher
//!   and search. Real gitignore support is a separate, later decision.
//! - **Binary detection stays T030's**: not valid UTF-8, or a NUL byte. Not
//!   `grep-searcher`'s, which has its own idea.
//!
//! ## What this is not
//!
//! Not an index: nothing is stored and nothing is remembered between calls —
//! and that stayed true when ranking arrived. Not a replace: [`search`] reads
//! and returns matches, and there is no mutating variant here.
//!
//! ## Two questions, two entry points
//!
//! [`search`] answers "which files *contain* this text". `SearchScope::Filenames`
//! is part of that answer and is still plain substring or regex matching against
//! the corpus already walked, in walk order, unranked.
//!
//! [`rank_files`] (T130-R) answers the different question a quick-open asks —
//! "which files *look like* this" — by subsequence, scored, best first, with the
//! matched positions. It shares this crate's walker configuration rather than
//! copying it, so ADR-022 decision 2 keeps holding: "ignored" means one thing.
//!
//! [`Sink`]: grep_searcher::Sink

pub mod error;
pub mod model;
pub mod query;
pub mod rank;
pub mod walk;

pub use error::SearchError;
pub use model::{
    MAX_SNIPPET_CHARS, MatchKind, SNIPPET_LEAD_CHARS, SearchMatch, SearchSummary, SkipCounts,
};
pub use query::{
    CompiledQuery, IgnorePredicate, MAX_FILE_BYTES, MAX_QUERY_CHARS, Query, SearchScope,
    default_ignore_predicate,
};
pub use rank::{
    DEFAULT_FIND_LIMIT, MAX_FIND_LIMIT, PathScore, RankedFiles, RankedPath, Scorer, rank_files,
    rank_workspace_files, score_path,
};
pub use walk::{SCAN_CHUNK_BYTES, SearchSink, search};
