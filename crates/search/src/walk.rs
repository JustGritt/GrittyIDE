//! The walk: one pass over the live tree, matched as it goes, cancellable
//! between files and inside one.
//!
//! This is `list_directory`'s pattern (`crates/workspace/src/listing.rs`)
//! extended from one level to a tree, and cursor-free: results stream through
//! a [`SearchSink`] rather than paginating. Nothing is indexed and nothing is
//! remembered between calls.
//!
//! ## Cancellation
//!
//! `cancelled` is polled **between every file** — strictly more often than
//! `list_directory`'s every-256-entries, because reading and matching a file
//! costs far more than reading a directory entry — and between
//! [`SCAN_CHUNK_BYTES`] chunks *within* one file, so cancelling mid-scan of a
//! 12 MiB file does not wait for it to finish.
//!
//! ## What is never followed
//!
//! `follow_links` is off, so a symlinked directory is never descended into and
//! no cycle detection is needed. A symlinked *file* is read only when it
//! resolves inside the root, which is `WorkspaceRoot::resolve`'s job and not a
//! second implementation of it.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use grep_matcher::{LineTerminator, Matcher};
use grep_searcher::{MmapChoice, Searcher, SearcherBuilder, Sink, SinkMatch};
use gritty_workspace::detect::{BinaryReason, UTF8_BOM};
use gritty_workspace::{ContentKind, WorkspacePath, WorkspaceRoot, detect_content};
use ignore::WalkBuilder;

use crate::error::SearchError;
use crate::model::{MatchKind, SearchMatch, SearchSummary, column_of, snippet_of};
use crate::query::{CompiledQuery, IgnorePredicate};

/// Bytes searched between two cancellation checks inside one file.
///
/// 1 MiB of line-oriented matching is single-digit milliseconds, so a
/// cancellation is observed about as promptly inside a large file as between
/// two small ones.
pub const SCAN_CHUNK_BYTES: usize = 1 << 20;

/// Files scanned between two `scanned` progress callbacks. The sink throttles
/// its own emission; this only bounds how often it is asked to.
const PROGRESS_INTERVAL: u64 = 64;

/// Where matches go as they are found.
///
/// The walk owns no event bus and no throttle: `gritty-core` implements this
/// and decides how matches are batched and bounded (rule 8). Returning `false`
/// stops the walk, which is how a caller enforces its own match cap.
pub trait SearchSink {
    /// One match. `false` stops the walk; the summary then says `limit_reached`.
    fn matched(&mut self, found: SearchMatch) -> bool;

    /// Files considered so far, roughly every [`PROGRESS_INTERVAL`] files.
    /// There is no total: a live walk does not know one until it is over.
    fn scanned(&mut self, _files: u64) {}
}

impl<F: FnMut(SearchMatch) -> bool> SearchSink for F {
    fn matched(&mut self, found: SearchMatch) -> bool {
        self(found)
    }
}

/// Walk `root` and match every file against `query`.
///
/// Errors are the walk's own: a root that disappeared, or a cancellation. A
/// single file that vanishes, is binary or is refused is counted in
/// [`SearchSummary::skipped`] and never fails the search — one unreadable file
/// must not lose the other ten thousand results.
pub fn search(
    root: &WorkspaceRoot,
    query: &CompiledQuery,
    sink: &mut dyn SearchSink,
    cancelled: &dyn Fn() -> bool,
) -> Result<SearchSummary, SearchError> {
    let started = Instant::now();
    let mut summary = SearchSummary::default();
    if cancelled() {
        return Err(SearchError::Cancelled);
    }
    // The root can go away between the command that resolved the workspace and
    // the blocking job that walks it. Failing here is the difference between
    // "the search failed" and "there were no matches", and only one of those
    // is true.
    if !root.is_present() {
        return Err(SearchError::io(
            format!("workspace root {}", root.path().display()),
            io::Error::from(io::ErrorKind::NotFound),
        ));
    }

    let (builder, ignored_directories) =
        build_walker(root, (!query.include_ignored()).then(|| query.ignored()));

    let mut searcher = build_searcher();
    let mut considered = 0u64;
    let mut stopped = false;
    for result in builder.build() {
        if cancelled() {
            return Err(SearchError::Cancelled);
        }
        let entry = match result {
            Ok(entry) => entry,
            Err(error) => {
                // A directory that vanished or refused us mid-walk. Counted,
                // logged without its content, and not the end of the search.
                tracing::debug!(%error, "search skipped an unreadable entry");
                summary.skipped.unreadable += 1;
                continue;
            }
        };
        if entry.depth() == 0 {
            continue;
        }
        let file_type = entry.file_type();
        if file_type.is_some_and(|kind| kind.is_dir()) {
            summary.directories_scanned += 1;
            continue;
        }
        let Some(path) = relative(root.path(), entry.path()) else {
            summary.skipped.unreadable += 1;
            continue;
        };
        // A symlink is read only when it resolves to a file inside the root.
        // `WorkspaceRoot::resolve` is the one place that decides that.
        let native: PathBuf = if entry.path_is_symlink() {
            match root.resolve(&path) {
                Ok(resolved) if resolved.canonical().is_file() => {
                    resolved.canonical().to_path_buf()
                }
                Ok(_) => continue,
                Err(_) => {
                    summary.skipped.outside_root += 1;
                    continue;
                }
            }
        } else if file_type.is_some_and(|kind| kind.is_file()) {
            entry.path().to_path_buf()
        } else {
            // Sockets, devices, pipes: nothing to search.
            continue;
        };

        considered += 1;
        if considered.is_multiple_of(PROGRESS_INTERVAL) {
            sink.scanned(considered);
        }

        let mut matched_here = false;
        if query.scope().includes_filenames()
            && let Some(found) = filename_match(query, &path)
        {
            summary.matches_found += 1;
            matched_here = true;
            if !sink.matched(found) {
                stopped = true;
                break;
            }
        }

        if query.scope().includes_content() {
            match scan_file(
                &mut searcher,
                query,
                &path,
                &native,
                sink,
                cancelled,
                &mut summary,
            )? {
                Flow::Continue { matched } => matched_here |= matched,
                Flow::Stop => {
                    stopped = true;
                    break;
                }
            }
        }
        if matched_here {
            summary.files_matched += 1;
        }
    }

    summary.files_scanned = considered;
    summary.limit_reached = stopped;
    summary.skipped.ignored_directories = ignored_directories.load(Ordering::Relaxed);
    summary.elapsed_ms = started.elapsed().as_millis() as u64;
    // Ids, counts and durations only: never a path's content, never the query.
    tracing::debug!(
        files = summary.files_scanned,
        matched = summary.files_matched,
        directories = summary.directories_scanned,
        matches = summary.matches_found,
        skipped = summary.skipped.total(),
        limit_reached = summary.limit_reached,
        elapsed_ms = summary.elapsed_ms,
        "search finished"
    );
    Ok(summary)
}

/// The one `WalkBuilder` configuration every tree walk in this crate shares,
/// plus the counter of directories the ignore predicate pruned.
///
/// Extracted so [`search`] and [`crate::rank::rank_files`] cannot drift: ADR-022
/// decision 2 says "ignored" means the same thing in the explorer, the watcher
/// and search, and a second copy of these switches is exactly how that stops
/// being true. `ignored` is `None` when the caller wants ignored directories
/// walked anyway.
pub(crate) fn build_walker(
    root: &WorkspaceRoot,
    ignored: Option<&IgnorePredicate>,
) -> (WalkBuilder, Arc<AtomicU64>) {
    let ignored_directories = Arc::new(AtomicU64::new(0));
    let mut builder = WalkBuilder::new(root.path());
    builder
        // ADR-022 decision 2: `ignore` is taken for its walker with every
        // gitignore source turned off. What Gritty means by "ignored" is
        // T070's `is_ignored`, and it must stay one thing across the explorer,
        // the watcher and search.
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false)
        .ignore(false)
        .parents(false)
        .hidden(false)
        .require_git(false)
        // Never walk into a symlinked directory (see the module header).
        .follow_links(false)
        // Deterministic order, so a test can assert on results and a user sees
        // a stable list rather than filesystem order.
        .sort_by_file_name(|a, b| a.cmp(b));
    if let Some(predicate) = ignored {
        let root_path = root.path().to_path_buf();
        let predicate = Arc::clone(predicate);
        let counter = Arc::clone(&ignored_directories);
        builder.filter_entry(move |entry| {
            // The root is the search, not a candidate for pruning.
            if entry.depth() == 0 {
                return true;
            }
            let Some(path) = relative(&root_path, entry.path()) else {
                return true;
            };
            if !predicate(&path) {
                return true;
            }
            if entry.file_type().is_some_and(|kind| kind.is_dir()) {
                counter.fetch_add(1, Ordering::Relaxed);
            }
            false
        });
    }
    (builder, ignored_directories)
}

/// The workspace-relative path of a walked entry, or `None` when it is not a
/// readable file inside the root.
///
/// Shared with [`crate::rank`] for the same reason [`build_walker`] is: a
/// symlink is followed only when `WorkspaceRoot::resolve` says it lands inside
/// the root, and that decision must have one implementation.
pub(crate) fn entry_path(
    root: &WorkspaceRoot,
    entry: &ignore::DirEntry,
) -> Result<Option<WorkspacePath>, OutsideRoot> {
    let Some(path) = relative(root.path(), entry.path()) else {
        return Err(OutsideRoot);
    };
    if entry.path_is_symlink() {
        return match root.resolve(&path) {
            Ok(resolved) if resolved.canonical().is_file() => Ok(Some(path)),
            Ok(_) => Ok(None),
            Err(_) => Err(OutsideRoot),
        };
    }
    if entry.file_type().is_some_and(|kind| kind.is_file()) {
        Ok(Some(path))
    } else {
        // Sockets, devices, pipes: nothing to read.
        Ok(None)
    }
}

/// A walked entry that could not be represented under the root.
pub(crate) struct OutsideRoot;

enum Flow {
    Continue { matched: bool },
    Stop,
}

/// One file's content, read whole, classified by T030's test and then matched
/// chunk by chunk.
fn scan_file(
    searcher: &mut Searcher,
    query: &CompiledQuery,
    path: &WorkspacePath,
    native: &Path,
    sink: &mut dyn SearchSink,
    cancelled: &dyn Fn() -> bool,
    summary: &mut SearchSummary,
) -> Result<Flow, SearchError> {
    let metadata = match fs::metadata(native) {
        Ok(metadata) => metadata,
        Err(_) => {
            summary.skipped.unreadable += 1;
            return Ok(Flow::Continue { matched: false });
        }
    };
    if metadata.len() > query.max_file_bytes() {
        summary.skipped.too_large += 1;
        return Ok(Flow::Continue { matched: false });
    }
    let bytes = match fs::read(native) {
        Ok(bytes) => bytes,
        Err(_) => {
            summary.skipped.unreadable += 1;
            return Ok(Flow::Continue { matched: false });
        }
    };
    // Binary detection is Gritty's own (ADR-022, `security.md:71-79`), not
    // `grep-searcher`'s: two definitions of "binary" in one product would be
    // the same mistake as two definitions of "ignored", and T030 got there
    // first. A binary file's content is never decoded, so it never matches.
    let body = match detect_content(&bytes) {
        ContentKind::Binary {
            reason: BinaryReason::NulByte,
        } => {
            summary.skipped.binary += 1;
            return Ok(Flow::Continue { matched: false });
        }
        ContentKind::Binary {
            reason: BinaryReason::InvalidUtf8,
        } => {
            summary.skipped.invalid_utf8 += 1;
            return Ok(Flow::Continue { matched: false });
        }
        ContentKind::Text { bom: true } => &bytes[UTF8_BOM.len()..],
        ContentKind::Text { bom: false } => &bytes[..],
    };
    summary.bytes_scanned += body.len() as u64;

    let mut offset = 0usize;
    let mut lines_before = 0u64;
    let mut matched = false;
    while offset < body.len() {
        if cancelled() {
            return Err(SearchError::Cancelled);
        }
        let end = chunk_end(body, offset);
        let chunk = &body[offset..end];
        let mut collector = Collector {
            path,
            matcher: query.matcher(),
            lines_before,
            sink,
            found: 0,
            truncated: 0,
            stopped: false,
        };
        searcher
            .search_slice(query.matcher(), chunk, &mut collector)
            .map_err(|error| SearchError::io(path.to_string(), error))?;
        summary.matches_found += collector.found;
        summary.snippets_truncated += collector.truncated;
        matched |= collector.found > 0;
        if collector.stopped {
            return Ok(Flow::Stop);
        }
        lines_before += count_lines(chunk);
        offset = end;
    }
    Ok(Flow::Continue { matched })
}

/// End of the chunk starting at `offset`: the first line boundary at or after
/// [`SCAN_CHUNK_BYTES`], so a chunk never splits a line and line numbers stay
/// exact across chunks.
fn chunk_end(body: &[u8], offset: usize) -> usize {
    let target = offset.saturating_add(SCAN_CHUNK_BYTES);
    if target >= body.len() {
        return body.len();
    }
    match body[target..].iter().position(|byte| *byte == b'\n') {
        Some(index) => target + index + 1,
        None => body.len(),
    }
}

fn count_lines(chunk: &[u8]) -> u64 {
    chunk.iter().filter(|byte| **byte == b'\n').count() as u64
}

/// The searcher: line numbers on, one line at a time, no memory mapping and
/// no binary detection of its own.
///
/// Memory mapping is off because the walk searches slices it already read —
/// which also means nothing here can take a `SIGBUS` from a file truncated
/// under it, and the crate needs no `unsafe` to say so.
fn build_searcher() -> Searcher {
    SearcherBuilder::new()
        .line_number(true)
        .multi_line(false)
        .bom_sniffing(false)
        .memory_map(MmapChoice::never())
        .line_terminator(LineTerminator::crlf())
        .build()
}

fn filename_match(query: &CompiledQuery, path: &WorkspacePath) -> Option<SearchMatch> {
    let text = path.as_str();
    let found = query.matcher().find(text.as_bytes()).ok().flatten()?;
    let (snippet, start, length, truncated) = snippet_of(text, found.start()..found.end());
    Some(SearchMatch {
        path: path.clone(),
        kind: MatchKind::Filename,
        line: None,
        column: None,
        snippet,
        truncated,
        match_start: start,
        match_length: length,
    })
}

/// Turns `grep-searcher`'s callbacks into [`SearchMatch`]es. It *is* the
/// adapter ADR-022 says a library needs no separate one for: there is no
/// external output to translate, only a sink we own.
struct Collector<'a> {
    path: &'a WorkspacePath,
    matcher: &'a grep_regex::RegexMatcher,
    lines_before: u64,
    sink: &'a mut dyn SearchSink,
    found: u64,
    truncated: u64,
    stopped: bool,
}

impl Sink for Collector<'_> {
    type Error = io::Error;

    fn matched(&mut self, _searcher: &Searcher, mat: &SinkMatch<'_>) -> Result<bool, io::Error> {
        let line = String::from_utf8_lossy(strip_terminator(mat.bytes()));
        // Where inside the line: `SinkMatch` reports the line, not the span.
        let span = self
            .matcher
            .find(line.as_bytes())
            .map_err(io::Error::from)?
            .map_or(0..0, |found| found.start()..found.end());
        let column = column_of(&line, span.start);
        let (snippet, start, length, truncated) = snippet_of(&line, span);
        if truncated {
            self.truncated += 1;
        }
        self.found += 1;
        let keep = self.sink.matched(SearchMatch {
            path: self.path.clone(),
            kind: MatchKind::Content,
            line: Some(self.lines_before + mat.line_number().unwrap_or(1)),
            column: Some(column),
            snippet,
            truncated,
            match_start: start,
            match_length: length,
        });
        if !keep {
            self.stopped = true;
        }
        Ok(keep)
    }
}

fn strip_terminator(line: &[u8]) -> &[u8] {
    let line = line.strip_suffix(b"\n").unwrap_or(line);
    line.strip_suffix(b"\r").unwrap_or(line)
}

/// The workspace-relative path of `native`, or `None` when it is not under
/// `root` or cannot be represented (a name that is not valid UTF-8).
fn relative(root: &Path, native: &Path) -> Option<WorkspacePath> {
    WorkspacePath::from_native(root, native).ok()
}
