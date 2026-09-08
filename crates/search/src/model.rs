//! What a search finds, and what the walk cost.
//!
//! These are the wire payloads of `search.query`: `gritty-core` owns the
//! request shape and the operation, the way it owns `ListDirectoryRequest`
//! while [`DirectoryPage`] lives beside the code that builds it.
//!
//! [`DirectoryPage`]: gritty_workspace::DirectoryPage

use gritty_workspace::WorkspacePath;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Longest snippet a match carries, in Unicode scalar values.
///
/// A bound, not a formatting preference: `weird-files/long-lines/one-line-100k.txt`
/// is a single 100,000-character line, and a match inside it must arrive as one
/// truncated match rather than as a dropped file or a 100 KB event.
pub const MAX_SNIPPET_CHARS: usize = 400;

/// How much of a truncated snippet is spent on the text *before* the match, so
/// the match is never flush against the left edge with no context at all.
pub const SNIPPET_LEAD_CHARS: usize = 80;

const ELLIPSIS: char = '\u{2026}';

/// Whether the match was found in a file's content or in its path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
pub enum MatchKind {
    Content,
    Filename,
}

/// One match, in the one envelope both kinds share.
///
/// A filename match carries no `line` and no `column` — there is no line to
/// point at — and its `snippet` is the workspace-relative path that matched.
/// Everything else is identical, so a consumer folds both into one list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SearchMatch {
    pub path: WorkspacePath,
    pub kind: MatchKind,
    /// One-based line number, content matches only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub line: Option<u64>,
    /// One-based column in UTF-16 code units — what Monaco's `IPosition`
    /// means and what `TextPosition` (T060) already records, so a match can be
    /// navigated to without a second conversion. Content matches only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub column: Option<u64>,
    /// The matching line (content) or path (filename), bounded to
    /// [`MAX_SNIPPET_CHARS`] and with the line terminator removed.
    pub snippet: String,
    /// The source line was longer than the snippet bound and a window around
    /// the match was taken. Never a silent elision.
    pub truncated: bool,
    /// Where the match starts inside `snippet`, in UTF-16 code units from its
    /// start (zero-based). Offsets are snippet-relative precisely because the
    /// snippet may be a window into a much longer line.
    pub match_start: u32,
    /// Length of the match inside `snippet`, in UTF-16 code units. Clipped
    /// when the match itself is longer than the window.
    pub match_length: u32,
}

/// Files the walk deliberately did not search, counted by reason.
///
/// Counted rather than dropped: "3 files were binary" is inspectable, and a
/// silently missing file is the failure this exists to prevent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SkipCounts {
    /// Contains a NUL byte (T030's test, `security.md:71-79`).
    pub binary: u64,
    /// Not valid UTF-8 as a whole (T030's other half).
    pub invalid_utf8: u64,
    /// Above the byte bound a single file may be read at
    /// ([`crate::MAX_FILE_BYTES`]).
    pub too_large: u64,
    /// Vanished, or the filesystem refused it, between the walk and the read.
    pub unreadable: u64,
    /// A symlink whose target resolves outside the workspace root. Never read,
    /// following `WorkspaceRoot`'s discipline.
    pub outside_root: u64,
    /// Directories not descended into because the ignore predicate flagged
    /// them (`options.includeIgnored` was false).
    pub ignored_directories: u64,
}

impl SkipCounts {
    pub fn total(&self) -> u64 {
        self.binary + self.invalid_utf8 + self.too_large + self.unreadable + self.outside_root
    }
}

/// What one walk cost and found. Carried by `OperationCompleted.result`
/// (inside `SearchCompletion`), never the match list — that already streamed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SearchSummary {
    /// Files actually opened and matched against (filename-only scope opens
    /// nothing, so this counts the files considered).
    pub files_scanned: u64,
    /// Files that produced at least one match.
    pub files_matched: u64,
    pub directories_scanned: u64,
    /// Bytes read for content matching.
    pub bytes_scanned: u64,
    pub matches_found: u64,
    /// Matches whose source line exceeded [`MAX_SNIPPET_CHARS`].
    pub snippets_truncated: u64,
    pub skipped: SkipCounts,
    /// The sink asked the walk to stop before the tree was exhausted (its own
    /// match cap). Results are partial, and saying so is the point.
    pub limit_reached: bool,
    pub elapsed_ms: u64,
}

/// Build the snippet, and the match position inside it, for one line.
///
/// `line` is the matching line with its terminator already removed;
/// `match_bytes` is the match's byte range inside it. Returns the snippet, the
/// UTF-16 offset and length of the match inside it, and whether a window was
/// taken.
pub(crate) fn snippet_of(
    line: &str,
    match_bytes: std::ops::Range<usize>,
) -> (String, u32, u32, bool) {
    // Repository content is untrusted input (rule 13): a byte range that is
    // not on a character boundary must degrade, never panic.
    let (start, end) = clamp_to_boundaries(line, match_bytes);
    let before = &line[..start];
    let matched = &line[start..end];
    let total_chars = line.chars().count();
    if total_chars <= MAX_SNIPPET_CHARS {
        return (
            line.to_owned(),
            utf16_len(before),
            utf16_len(matched),
            false,
        );
    }

    // A window around the match, with the lead spent on context before it, so
    // a match 90,000 characters into a line is not reported as column 1 of a
    // line that starts somewhere else.
    let start_char = before.chars().count();
    let window_start = start_char.saturating_sub(SNIPPET_LEAD_CHARS);
    let body: String = line
        .chars()
        .skip(window_start)
        .take(MAX_SNIPPET_CHARS)
        .collect();
    let leading = window_start > 0;
    let trailing = window_start + MAX_SNIPPET_CHARS < total_chars;
    let mut snippet = String::with_capacity(body.len() + 2 * ELLIPSIS.len_utf8());
    if leading {
        snippet.push(ELLIPSIS);
    }
    snippet.push_str(&body);
    if trailing {
        snippet.push(ELLIPSIS);
    }
    // Offsets are relative to the snippet, ellipsis included: it is a
    // character the consumer renders like any other.
    let lead_chars = start_char - window_start;
    let body_before: String = body.chars().take(lead_chars).collect();
    let visible: String = body
        .chars()
        .skip(lead_chars)
        .take(matched.chars().count())
        .collect();
    let offset = utf16_len(&body_before) + u32::from(leading);
    (snippet, offset, utf16_len(&visible), true)
}

/// Clamp a byte range into `line` and onto character boundaries.
fn clamp_to_boundaries(line: &str, range: std::ops::Range<usize>) -> (usize, usize) {
    let mut start = range.start.min(line.len());
    while start > 0 && !line.is_char_boundary(start) {
        start -= 1;
    }
    let mut end = range.end.clamp(start, line.len());
    while end > start && !line.is_char_boundary(end) {
        end -= 1;
    }
    (start, end)
}

fn utf16_len(text: &str) -> u32 {
    u32::try_from(text.encode_utf16().count()).unwrap_or(u32::MAX)
}

/// One-based UTF-16 column of a byte offset inside a line.
pub(crate) fn column_of(line: &str, byte_offset: usize) -> u64 {
    let (start, _) = clamp_to_boundaries(line, byte_offset..byte_offset);
    u64::from(utf16_len(&line[..start])) + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_line_is_its_own_snippet() {
        let (snippet, start, length, truncated) = snippet_of("let needle = 1;", 4..10);
        assert_eq!(snippet, "let needle = 1;");
        assert_eq!((start, length), (4, 6));
        assert!(!truncated);
    }

    #[test]
    fn a_long_line_is_windowed_around_the_match_and_says_so() {
        let mut line = "a".repeat(10_000);
        line.push_str("needle");
        line.push_str(&"b".repeat(10_000));
        let start = 10_000;
        let (snippet, offset, length, truncated) = snippet_of(&line, start..start + 6);
        assert!(truncated, "a 20k line must report truncation");
        // Two ellipses plus the window.
        assert_eq!(snippet.chars().count(), MAX_SNIPPET_CHARS + 2);
        assert_eq!(length, 6);
        let matched: String = snippet
            .chars()
            .skip(offset as usize)
            .take(length as usize)
            .collect();
        assert_eq!(matched, "needle");
    }

    #[test]
    fn a_match_at_the_start_of_a_long_line_keeps_its_offset() {
        let line = format!("needle{}", "x".repeat(10_000));
        let (snippet, offset, length, truncated) = snippet_of(&line, 0..6);
        assert!(truncated);
        assert_eq!(offset, 0, "no leading ellipsis when the window starts at 0");
        assert_eq!(length, 6);
        assert!(snippet.starts_with("needle"));
        assert!(snippet.ends_with(ELLIPSIS));
    }

    #[test]
    fn columns_and_offsets_count_utf16_units() {
        // An astral emoji is two UTF-16 code units, one char, four bytes.
        let line = "🎉 needle";
        let byte_start = line.find("needle").unwrap();
        assert_eq!(column_of(line, byte_start), 4);
        let (_, offset, length, _) = snippet_of(line, byte_start..byte_start + 6);
        assert_eq!((offset, length), (3, 6));
    }

    #[test]
    fn a_match_serializes_camel_case_and_omits_absent_positions() {
        let filename = SearchMatch {
            path: WorkspacePath::parse("src/main.rs").unwrap(),
            kind: MatchKind::Filename,
            line: None,
            column: None,
            snippet: "src/main.rs".to_owned(),
            truncated: false,
            match_start: 4,
            match_length: 4,
        };
        assert_eq!(
            serde_json::to_value(&filename).unwrap(),
            serde_json::json!({
                "path": "src/main.rs",
                "kind": "filename",
                "snippet": "src/main.rs",
                "truncated": false,
                "matchStart": 4,
                "matchLength": 4,
            })
        );
    }

    #[test]
    fn skip_counts_add_up_without_the_directory_count() {
        let counts = SkipCounts {
            binary: 1,
            invalid_utf8: 2,
            too_large: 3,
            unreadable: 4,
            outside_root: 5,
            ignored_directories: 99,
        };
        assert_eq!(counts.total(), 15);
    }
}
