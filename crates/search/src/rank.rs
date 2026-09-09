//! Ranked path matching (T130-R): the quick-open half of this crate.
//!
//! [`search`] answers "which files contain this text". This module answers a
//! different question — "which files *look like* this" — and the difference is
//! ranking. `SearchScope::Filenames` is a literal or regex hit returned in walk
//! order; `epn` cannot find `editor-pane.tsx` there, and could not say *where*
//! it matched if it did, because a subsequence hit is a set of positions and
//! `SearchMatch` carries one contiguous span.
//!
//! ## Two stages, and why
//!
//! Scoring is `O(query × path)` and the walk sees thousands of paths, so almost
//! every path is rejected by an `O(path)` subsequence test before the scorer
//! ever runs. That prefilter is the reason a walk *per query* is affordable and
//! this crate still needs no index — the property T080's non-goal argued for and
//! this module keeps.
//!
//! ## Still not an index
//!
//! Nothing is stored and nothing is remembered between calls, exactly as the
//! crate header promises. A ranked query is a fresh walk, cancellable between
//! entries, bounded by a heap of `limit`.

use std::cmp::Ordering as CmpOrdering;
use std::collections::BinaryHeap;
use std::io;
use std::sync::atomic::Ordering;
use std::time::Instant;

use gritty_workspace::{WorkspacePath, WorkspaceRoot};

use crate::error::SearchError;
use crate::model::SkipCounts;
use crate::query::{IgnorePredicate, MAX_QUERY_CHARS, default_ignore_predicate};
use crate::walk::{build_walker, entry_path};

/// How many results a caller gets when it does not say.
pub const DEFAULT_FIND_LIMIT: usize = 50;

/// The most a caller may ask for. A quick-open list nobody scrolls past fifty
/// rows of does not need a thousand, and the cap is what bounds the heap.
pub const MAX_FIND_LIMIT: usize = 200;

/// Entries walked between two cancellation checks.
///
/// Matches `list_directory`'s interval rather than [`crate::walk::search`]'s
/// every-entry check: ranking a path is arithmetic over a short string, not a
/// file read, so the check would cost more than the work it interrupts.
const CANCEL_CHECK_INTERVAL: u64 = 256;

// The scoring constants. They are relative to each other and nothing else, so a
// test asserts an *ordering* between two paths rather than a magic number —
// which is what lets these be tuned without rewriting the suite.
const MATCH_BASE: i32 = 16;
const CONSECUTIVE_BONUS: i32 = 8;
const BOUNDARY_BONUS: i32 = 12;
const CAMEL_BONUS: i32 = 10;
const BASENAME_BONUS: i32 = 8;
const BASENAME_START_BONUS: i32 = 10;
/// Every matched character landed in the file name, not in a directory above
/// it. Applied once, to the whole match, rather than per character — see
/// `Scorer::best`, where the failing real-world case is recorded.
const WHOLLY_IN_BASENAME_BONUS: i32 = 24;
/// The query began exactly where the file name begins. This is the strongest
/// single signal a quick-open has — someone typing `epn` for `editor-pane.tsx`
/// almost always starts from the first letter of the name — and it is why fzf
/// gives the first character its own multiplier.
const STARTS_THE_BASENAME_BONUS: i32 = 20;
const EXACT_CASE_BONUS: i32 = 2;
/// Opening a gap. Larger in magnitude than [`BOUNDARY_BONUS`] on purpose: if it
/// were not, jumping to the next separator would always pay for itself and a
/// solid run would never win.
const GAP_START_PENALTY: i32 = -14;
/// Widening a gap that is already open.
const GAP_PENALTY: i32 = -1;
const LENGTH_PENALTY_DIVISOR: i32 = 8;

/// Characters after which the next character starts a new "word".
const BOUNDARY_CHARS: [char; 5] = ['/', '_', '-', '.', ' '];

/// What a query scored against one path, and where it matched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathScore {
    pub score: i32,
    /// Matched spans as `(start, length)` in **char** indices over the
    /// workspace-relative path, ascending and non-overlapping, with adjacent
    /// matches merged into one span.
    ///
    /// Char indices, not bytes: the UI slices this string to highlight it, and
    /// a byte index is wrong the moment a path contains a non-ASCII character.
    pub ranges: Vec<(u32, u32)>,
}

/// One ranked path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RankedPath {
    pub path: WorkspacePath,
    pub score: i32,
    pub ranges: Vec<(u32, u32)>,
}

/// What a ranked walk found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RankedFiles {
    /// Best first. Score descending, then path ascending.
    pub matches: Vec<RankedPath>,
    /// A scoring candidate was discarded because the heap was full. The caller
    /// must say so: a truncated list that reads as a complete one is the
    /// failure T080's contract already names.
    pub truncated: bool,
    pub files_scanned: u64,
    pub directories_scanned: u64,
    pub skipped: SkipCounts,
    pub elapsed_ms: u64,
}

/// A reusable scorer for one query.
///
/// Holds its own scratch buffers so scoring ten thousand paths allocates once,
/// not ten thousand times. [`score_path`] is the one-shot convenience over it.
pub struct Scorer {
    /// The query, lowercased. Empty means "match everything with score 0".
    query: Vec<char>,
    /// True where the query character was typed in upper case, so an exact-case
    /// hit can be rewarded without a second pass over the original string.
    query_upper: Vec<bool>,
    path: Vec<char>,
    path_lower: Vec<char>,
    bonuses: Vec<i32>,
    previous: Vec<i32>,
    current: Vec<i32>,
    /// `parent[i * path_len + j]` — which path index matched query character
    /// `i - 1`, so the ranges can be walked back out of the table.
    parent: Vec<u32>,
}

const NO_PARENT: u32 = u32::MAX;

impl Scorer {
    /// Compile a query for repeated scoring.
    ///
    /// The only failure is a query longer than [`MAX_QUERY_CHARS`], which is
    /// the same limit and the same error `Query::compile` raises — one rule for
    /// how long a query may be, not two.
    pub fn new(query: &str) -> Result<Self, SearchError> {
        let chars: Vec<char> = query.chars().collect();
        if chars.len() > MAX_QUERY_CHARS {
            return Err(SearchError::invalid_query(
                "query",
                format!(
                    "is {} characters, above the {MAX_QUERY_CHARS}-character limit",
                    chars.len()
                ),
            ));
        }
        Ok(Self {
            query: chars.iter().map(|c| lower(*c)).collect(),
            query_upper: chars.iter().map(|c| c.is_uppercase()).collect(),
            path: Vec::new(),
            path_lower: Vec::new(),
            bonuses: Vec::new(),
            previous: Vec::new(),
            current: Vec::new(),
            parent: Vec::new(),
        })
    }

    /// Whether this query matches nothing in particular, and so ranks every
    /// path equally. This is what a quick-open shows before a key is pressed.
    pub fn is_empty(&self) -> bool {
        self.query.is_empty()
    }

    /// Score `path`, or `None` when the query is not a subsequence of it.
    pub fn score(&mut self, path: &str) -> Option<PathScore> {
        if self.query.is_empty() {
            return Some(PathScore {
                score: 0,
                ranges: Vec::new(),
            });
        }

        self.path.clear();
        self.path.extend(path.chars());
        let path_len = self.path.len();
        if self.query.len() > path_len {
            return None;
        }

        self.path_lower.clear();
        self.path_lower.extend(self.path.iter().map(|c| lower(*c)));

        // Stage one: the cheap rejection. Almost every path leaves here.
        if !is_subsequence(&self.query, &self.path_lower) {
            return None;
        }

        // Stage two: the table. Only survivors pay for this.
        self.fill_bonuses();
        self.fill_table(path_len);
        self.best(path_len)
    }

    /// The positional bonus of every path index, computed once per path rather
    /// than once per (query, path) cell.
    fn fill_bonuses(&mut self) {
        let basename_start = basename_start(&self.path);
        self.bonuses.clear();
        self.bonuses.reserve(self.path.len());
        for index in 0..self.path.len() {
            let mut bonus = 0;
            let current = self.path[index];
            match index.checked_sub(1).map(|previous| self.path[previous]) {
                // The first character of the path is a boundary by definition.
                None => bonus += BOUNDARY_BONUS,
                Some(previous) if BOUNDARY_CHARS.contains(&previous) => bonus += BOUNDARY_BONUS,
                // camelCase: the capital starts a word even with no separator.
                Some(previous) if previous.is_lowercase() && current.is_uppercase() => {
                    bonus += CAMEL_BONUS;
                }
                _ => {}
            }
            if index >= basename_start {
                bonus += BASENAME_BONUS;
            }
            if index == basename_start {
                bonus += BASENAME_START_BONUS;
            }
            self.bonuses.push(bonus);
        }
    }

    /// The dynamic-programming table.
    ///
    /// `current[j]` is the best score for matching the query up to character
    /// `i` with that character landing exactly on path index `j`. A greedy
    /// left-to-right match would be cheaper and would rank `epn` against
    /// `editor-pane.tsx` wrongly — it would take the first `e` of `editor` and
    /// then the `p` of `pane`, missing that a later alignment scores higher.
    fn fill_table(&mut self, path_len: usize) {
        let query_len = self.query.len();
        self.parent.clear();
        self.parent.resize(query_len * path_len, NO_PARENT);
        self.previous.clear();
        self.previous.resize(path_len, i32::MIN);
        self.current.clear();
        self.current.resize(path_len, i32::MIN);

        for i in 0..query_len {
            std::mem::swap(&mut self.previous, &mut self.current);
            self.current.iter_mut().for_each(|cell| *cell = i32::MIN);
            // The best cell of the previous row strictly before `j - 1`,
            // carried forward so the inner loop stays linear instead of
            // scanning back over every earlier index. `j - 1` itself is the
            // consecutive case and is handled separately, so it is folded in
            // only at the *end* of each iteration.
            let mut best_before = i32::MIN;
            let mut best_before_at = NO_PARENT;

            for j in 0..path_len {
                if self.query[i] == self.path_lower[j] {
                    let mut here = MATCH_BASE + self.bonuses[j];
                    if self.query_upper[i] == self.path[j].is_uppercase() {
                        here += EXACT_CASE_BONUS;
                    }

                    if i == 0 {
                        // No positional penalty for where the first character
                        // landed. Depth is not irrelevance: `editor-pane.tsx`
                        // is nested six directories deep and is still the best
                        // answer for `epn`. Path length is already paid for
                        // once, in `best`.
                        self.current[j] = here;
                    } else if j > 0 {
                        // A later query character can never land on index 0,
                        // so `j - 1` is always a real predecessor here.
                        //
                        // Two ways to arrive: immediately after the previous
                        // match (consecutive, worth a bonus), or after a gap.
                        let consecutive = self.previous[j - 1];
                        let consecutive_score = if consecutive == i32::MIN {
                            i32::MIN
                        } else {
                            consecutive + here + CONSECUTIVE_BONUS
                        };
                        let gapped_score = if best_before == i32::MIN {
                            i32::MIN
                        } else {
                            // Opening a gap costs more than widening one, so a
                            // solid run wins against the same characters spread
                            // across separators — otherwise every boundary
                            // bonus would pay for the gap that reached it.
                            let distance = j as i32 - best_before_at as i32 - 1;
                            best_before + here + GAP_START_PENALTY + GAP_PENALTY * (distance - 1)
                        };

                        if consecutive_score != i32::MIN || gapped_score != i32::MIN {
                            if consecutive_score >= gapped_score {
                                self.current[j] = consecutive_score;
                                self.parent[i * path_len + j] = (j - 1) as u32;
                            } else {
                                self.current[j] = gapped_score;
                                self.parent[i * path_len + j] = best_before_at;
                            }
                        }
                    }
                }

                // Fold `j - 1` in now, so the next iteration's `best_before`
                // covers exactly the cells that are a *gap* away from it.
                if i > 0 && j >= 1 {
                    let candidate = self.previous[j - 1];
                    if candidate > best_before {
                        best_before = candidate;
                        best_before_at = (j - 1) as u32;
                    }
                }
            }
        }
    }

    /// The best final cell, walked back into merged ranges.
    fn best(&self, path_len: usize) -> Option<PathScore> {
        let query_len = self.query.len();
        // Ties break on the *earliest* end position, so an identical score
        // never depends on which cell was visited last.
        let mut best_score = i32::MIN;
        let mut best_at = None;
        for (j, score) in self.current.iter().enumerate() {
            if *score > best_score {
                best_score = *score;
                best_at = Some(j);
            }
        }
        let mut at = best_at?;

        let mut positions = Vec::with_capacity(query_len);
        for i in (0..query_len).rev() {
            positions.push(at as u32);
            if i == 0 {
                break;
            }
            let parent = self.parent[i * path_len + at];
            debug_assert_ne!(parent, NO_PARENT, "a scored cell always has a parent");
            at = parent as usize;
        }
        positions.reverse();

        // A query almost always names the *file*, not the path it sits under.
        // Without this, a query can spend a character inside a directory and
        // buy the rest back with one lucky boundary run in the basename:
        // `epn` scored `fixtures/weird-files/binary/pixel.png` (the `e` of
        // `fixtures`, then `p`+`n` of `png`) above `editor-pane.tsx`. Rewarding
        // a match that lies wholly inside the basename says the thing the
        // per-character basename bonus cannot: not "more of this matched the
        // file name" but "*all* of it did".
        let basename_start = basename_start(&self.path) as u32;
        let first = positions.first().copied();

        // Longer paths are worse answers for the same score: `src/a.ts` should
        // beat `vendor/generated/deep/a.ts` when both match equally well.
        let mut score = best_score - path_len as i32 / LENGTH_PENALTY_DIVISOR;
        if first.is_some_and(|first| first >= basename_start) {
            score += WHOLLY_IN_BASENAME_BONUS;
        }
        if first == Some(basename_start) {
            score += STARTS_THE_BASENAME_BONUS;
        }
        Some(PathScore {
            score,
            ranges: merge(&positions),
        })
    }
}

/// Score one path against one query. Convenience over [`Scorer`]; prefer the
/// struct when scoring more than one path, which the walk does.
pub fn score_path(query: &str, path: &str) -> Option<PathScore> {
    Scorer::new(query).ok()?.score(path)
}

/// Walk `root` and return the best `limit` paths for `query`.
///
/// `ignored` is the predicate that decides which directories are skipped;
/// [`default_ignore_predicate`] is the production answer, injected so a test can
/// prove the walk consults it per directory before descending.
pub fn rank_files(
    root: &WorkspaceRoot,
    query: &str,
    limit: usize,
    ignored: Option<&IgnorePredicate>,
    cancelled: &dyn Fn() -> bool,
) -> Result<RankedFiles, SearchError> {
    let started = Instant::now();
    let limit = limit.min(MAX_FIND_LIMIT);
    let mut scorer = Scorer::new(query)?;
    if cancelled() {
        return Err(SearchError::Cancelled);
    }
    // The root can go away between the command that resolved the workspace and
    // the blocking job that walks it. Failing here is the difference between
    // "the search failed" and "there were no matches", and only one is true.
    if !root.is_present() {
        return Err(SearchError::io(
            format!("workspace root {}", root.path().display()),
            io::Error::from(io::ErrorKind::NotFound),
        ));
    }

    let (builder, ignored_directories) = build_walker(root, ignored);
    let mut found = RankedFiles::default();
    let mut heap: BinaryHeap<Candidate> = BinaryHeap::with_capacity(limit.saturating_add(1));
    let mut considered = 0u64;

    for result in builder.build() {
        if considered.is_multiple_of(CANCEL_CHECK_INTERVAL) && cancelled() {
            return Err(SearchError::Cancelled);
        }
        let entry = match result {
            Ok(entry) => entry,
            Err(error) => {
                // A directory that vanished or refused us mid-walk. Counted,
                // logged without its content, and not the end of the walk.
                tracing::debug!(%error, "ranked walk skipped an unreadable entry");
                found.skipped.unreadable += 1;
                continue;
            }
        };
        if entry.depth() == 0 {
            continue;
        }
        if entry.file_type().is_some_and(|kind| kind.is_dir()) {
            found.directories_scanned += 1;
            continue;
        }
        let path = match entry_path(root, &entry) {
            Ok(Some(path)) => path,
            Ok(None) => continue,
            Err(_) => {
                found.skipped.outside_root += 1;
                continue;
            }
        };
        considered += 1;

        // An empty query ranks everything equally, so the first `limit` entries
        // in the walk's own deterministic order are the answer and there is
        // nothing to score.
        if scorer.is_empty() {
            if heap.len() < limit {
                heap.push(Candidate {
                    score: 0,
                    path,
                    ranges: Vec::new(),
                });
            } else {
                found.truncated = true;
            }
            continue;
        }

        let Some(scored) = scorer.score(path.as_str()) else {
            continue;
        };
        let candidate = Candidate {
            score: scored.score,
            path,
            ranges: scored.ranges,
        };
        if heap.len() < limit {
            heap.push(candidate);
        } else if let Some(worst) = heap.peek()
            && candidate < *worst
        {
            // `Candidate`'s ordering is reversed, so the heap's "greatest" is
            // the worst result: replacing it keeps the best `limit`.
            heap.pop();
            heap.push(candidate);
            found.truncated = true;
        } else {
            found.truncated = true;
        }
    }

    let mut matches: Vec<RankedPath> = heap
        .into_iter()
        .map(|candidate| RankedPath {
            path: candidate.path,
            score: candidate.score,
            ranges: candidate.ranges,
        })
        .collect();
    // A heap is unordered; the contract is not. Score descending, then path
    // ascending, so two runs over one tree cannot disagree.
    matches.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.path.cmp(&b.path)));

    found.matches = matches;
    found.files_scanned = considered;
    found.skipped.ignored_directories = ignored_directories.load(Ordering::Relaxed);
    found.elapsed_ms = started.elapsed().as_millis() as u64;
    // Counts and durations only: never a path, never the query (rule 13).
    tracing::debug!(
        files = found.files_scanned,
        directories = found.directories_scanned,
        returned = found.matches.len(),
        truncated = found.truncated,
        skipped = found.skipped.total(),
        elapsed_ms = found.elapsed_ms,
        "ranked walk finished"
    );
    Ok(found)
}

/// The same walk with the production ignore predicate, which is what every
/// caller but a test wants.
pub fn rank_workspace_files(
    root: &WorkspaceRoot,
    query: &str,
    limit: usize,
    cancelled: &dyn Fn() -> bool,
) -> Result<RankedFiles, SearchError> {
    rank_files(
        root,
        query,
        limit,
        Some(&default_ignore_predicate()),
        cancelled,
    )
}

/// A heap entry ordered **worst first**, so `BinaryHeap::peek` is the row to
/// evict. Ordering mirrors the final sort exactly — score, then path — or the
/// heap would evict a row the sort would have kept.
#[derive(Debug, PartialEq, Eq)]
struct Candidate {
    score: i32,
    path: WorkspacePath,
    ranges: Vec<(u32, u32)>,
}

impl Ord for Candidate {
    fn cmp(&self, other: &Self) -> CmpOrdering {
        other
            .score
            .cmp(&self.score)
            .then_with(|| self.path.cmp(&other.path))
    }
}

impl PartialOrd for Candidate {
    fn partial_cmp(&self, other: &Self) -> Option<CmpOrdering> {
        Some(self.cmp(other))
    }
}

/// The index just past the last separator: where the file name starts.
fn basename_start(path: &[char]) -> usize {
    path.iter()
        .rposition(|c| *c == '/')
        .map_or(0, |index| index + 1)
}

/// Lowercase one character, taking the first of a multi-character mapping.
///
/// Deterministic and allocation-free, which a `str::to_lowercase` per path
/// would not be. The cases where a lowercase mapping is longer than one char
/// (`İ`) do not occur in a path this comparison needs to be exact about.
fn lower(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

/// Stage one: is `query` a subsequence of `path`? Both already lowercased.
fn is_subsequence(query: &[char], path: &[char]) -> bool {
    let mut wanted = query.iter();
    let mut next = wanted.next();
    for candidate in path {
        match next {
            None => return true,
            Some(c) if c == candidate => next = wanted.next(),
            Some(_) => {}
        }
    }
    next.is_none()
}

/// Ascending positions into `(start, length)` spans, adjacent ones merged.
fn merge(positions: &[u32]) -> Vec<(u32, u32)> {
    let mut ranges: Vec<(u32, u32)> = Vec::new();
    for position in positions {
        match ranges.last_mut() {
            Some((start, length)) if *start + *length == *position => *length += 1,
            _ => ranges.push((*position, 1)),
        }
    }
    ranges
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scored(query: &str, path: &str) -> PathScore {
        score_path(query, path).unwrap_or_else(|| panic!("{query:?} should match {path:?}"))
    }

    fn score(query: &str, path: &str) -> i32 {
        scored(query, path).score
    }

    /// Asserts an *ordering*, never a magic number: the constants above are
    /// relative to each other, so a tuned bonus must not rewrite the suite.
    #[track_caller]
    fn beats(query: &str, better: &str, worse: &str) {
        let (a, b) = (score(query, better), score(query, worse));
        assert!(
            a > b,
            "{query:?}: {better:?} scored {a}, {worse:?} scored {b}"
        );
    }

    #[test]
    fn a_query_that_is_not_a_subsequence_does_not_match() {
        assert!(score_path("zzz", "src/main.rs").is_none());
        // Right characters, wrong order: a subsequence is ordered.
        assert!(score_path("niam", "src/main.rs").is_none());
        // Longer than the path it is matched against.
        assert!(score_path("abcdefghijk", "ab.rs").is_none());
    }

    #[test]
    fn an_empty_query_matches_everything_with_no_ranges() {
        let found = scored("", "src/main.rs");
        assert_eq!(found.score, 0);
        assert!(found.ranges.is_empty());
        // Even a path nothing else would match.
        assert_eq!(scored("", "").score, 0);
    }

    #[test]
    fn matching_is_case_insensitive_but_exact_case_scores_higher() {
        assert!(score_path("EPN", "editor-pane.tsx").is_some());
        assert!(score_path("epn", "EDITOR-PANE.TSX").is_some());
        beats("APP", "APP.tsx", "app.tsx");
        beats("app", "app.tsx", "APP.tsx");
    }

    #[test]
    fn a_consecutive_run_beats_the_same_characters_scattered() {
        beats("abc", "abc.ts", "a-b-c.ts");
    }

    #[test]
    fn a_word_boundary_beats_the_middle_of_a_word() {
        // `p` after a separator, versus `p` inside `apple`.
        beats("p", "a/pear", "apple");
        beats("p", "a_pear", "apple");
        beats("p", "a-pear", "apple");
        beats("p", "a.pear", "apple");
    }

    #[test]
    fn a_camel_case_capital_is_a_boundary_even_without_a_separator() {
        beats("p", "aPear", "apear");
    }

    #[test]
    fn the_basename_beats_the_directory() {
        beats("src", "a/src.ts", "src/a.ts");
    }

    #[test]
    fn the_start_of_the_basename_beats_the_middle_of_it() {
        beats("pane", "x/pane-editor.tsx", "x/editor-pane.tsx");
    }

    #[test]
    fn a_shorter_path_beats_a_longer_one_that_matches_as_well() {
        beats("a.ts", "src/a.ts", "vendor/generated/deep/nested/a.ts");
    }

    #[test]
    fn epn_finds_editor_pane_ahead_of_a_path_that_merely_contains_the_letters() {
        // The acceptance criterion in the task file, and the case that fails
        // first if the basename bonus is dropped.
        // `epn` is **e**ditor-**p**a**n**e: two of the three land on word
        // starts inside the basename. In `expand.rs` the `p` and the `n` are
        // both buried mid-word, which is the difference the bonuses encode.
        beats(
            "epn",
            "apps/desktop/src/features/editor/editor-pane.tsx",
            "crates/persistence/src/expand.rs",
        );
    }

    #[test]
    fn a_match_wholly_inside_the_basename_beats_one_that_borrows_a_directory() {
        // The real failure, found by ranking this repository rather than by a
        // counter-example chosen to be beatable: `epn` scored
        // `fixtures/weird-files/binary/pixel.png` **above** `editor-pane.tsx`,
        // by spending its `e` on the `e` of `fixtures` and buying the rest back
        // with the boundary-plus-consecutive run in `.png`. Every per-character
        // bonus was working as designed; what was missing was a signal for
        // "the whole query named the file".
        beats(
            "epn",
            "apps/desktop/src/features/editor/editor-pane.tsx",
            "fixtures/weird-files/binary/pixel.png",
        );
    }

    #[test]
    fn a_directory_query_still_matches_across_the_separator() {
        // The basename reward must not become a basename *requirement*: a query
        // that genuinely spans directories has nowhere else to match.
        assert!(score_path("srcutil", "src/lib/util.ts").is_some());
        beats(
            "srcutil",
            "src/lib/util.ts",
            "src/lib/nested/deep/utility-other.ts",
        );
    }

    #[test]
    fn ranges_are_merged_and_ascending() {
        // `ab` lands on two adjacent characters: one span, not two.
        assert_eq!(scored("ab", "ab.ts").ranges, vec![(0, 2)]);
        // Separated matches stay separate, in order.
        let ranges = scored("ac", "abc.ts").ranges;
        assert_eq!(ranges, vec![(0, 1), (2, 1)]);
        for pair in ranges.windows(2) {
            assert!(
                pair[0].0 + pair[0].1 < pair[1].0,
                "{ranges:?} must be ascending and disjoint"
            );
        }
    }

    #[test]
    fn ranges_count_characters_not_bytes() {
        // `é` is two bytes and one char: `ab` starts at char 2, byte 3.
        let found = scored("ab", "é/ab.ts");
        assert_eq!(found.ranges, vec![(2, 2)]);
        // The span is usable as a char slice, which is the point.
        let chars: Vec<char> = "é/ab.ts".chars().collect();
        let (start, length) = found.ranges[0];
        let slice: String = chars[start as usize..(start + length) as usize]
            .iter()
            .collect();
        assert_eq!(slice, "ab");
    }

    #[test]
    fn every_query_character_appears_exactly_once_in_the_ranges() {
        let found = scored("epn", "apps/desktop/src/features/editor/editor-pane.tsx");
        let matched: u32 = found.ranges.iter().map(|(_, length)| length).sum();
        assert_eq!(
            matched, 3,
            "three query characters, three matched positions"
        );
    }

    #[test]
    fn a_query_above_the_character_limit_is_refused_not_truncated() {
        let refused = Scorer::new(&"x".repeat(MAX_QUERY_CHARS + 1));
        assert!(
            matches!(
                refused,
                Err(SearchError::InvalidQuery { field: "query", .. })
            ),
            "a query above the limit must be refused, not truncated"
        );
        // The limit itself is accepted.
        assert!(Scorer::new(&"x".repeat(MAX_QUERY_CHARS)).is_ok());
    }

    #[test]
    fn one_scorer_reused_gives_the_same_answers_as_fresh_ones() {
        // The scratch buffers are reused across paths; a stale buffer would
        // show up as a second call disagreeing with a first.
        let paths = [
            "src/main.rs",
            "src/lib.rs",
            "apps/desktop/src/App.tsx",
            "zzz",
        ];
        let mut scorer = Scorer::new("src").expect("compiles");
        for path in paths {
            assert_eq!(scorer.score(path), score_path("src", path), "{path}");
        }
        for path in paths.iter().rev() {
            assert_eq!(scorer.score(path), score_path("src", path), "{path}");
        }
    }

    #[test]
    fn scoring_is_a_pure_function_of_query_and_path() {
        assert_eq!(
            score_path("epn", "editor-pane.tsx"),
            score_path("epn", "editor-pane.tsx")
        );
    }

    #[test]
    fn merge_folds_only_adjacent_positions() {
        assert_eq!(merge(&[]), vec![]);
        assert_eq!(merge(&[3]), vec![(3, 1)]);
        assert_eq!(merge(&[0, 1, 2]), vec![(0, 3)]);
        assert_eq!(merge(&[0, 2, 3, 7]), vec![(0, 1), (2, 2), (7, 1)]);
    }

    #[test]
    fn is_subsequence_is_the_prefilter_the_scorer_relies_on() {
        let chars = |s: &str| -> Vec<char> { s.chars().collect() };
        assert!(is_subsequence(&chars("epn"), &chars("editor-pane.tsx")));
        assert!(is_subsequence(&chars(""), &chars("anything")));
        assert!(!is_subsequence(&chars("epn"), &chars("pane-editor.tsx")));
        assert!(!is_subsequence(&chars("aa"), &chars("a")));
    }

    #[test]
    fn basename_start_is_the_index_after_the_last_separator() {
        let at = |s: &str| basename_start(&s.chars().collect::<Vec<_>>());
        assert_eq!(at("main.rs"), 0);
        assert_eq!(at("src/main.rs"), 4);
        assert_eq!(at("a/b/c"), 4);
        assert_eq!(at("trailing/"), 9);
    }
}
