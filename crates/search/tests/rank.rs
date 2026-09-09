//! Integration tests for the ranked walk (T130-R): the committed
//! `fixtures/small-repo`, the generated `large-repo`, and scratch trees for the
//! cases no fixture should carry.
//!
//! The scorer's own rules are unit-tested in `src/rank.rs`, where they need no
//! filesystem. What is proved here is everything the walk adds: order, bounds,
//! cancellation, and that the ignore predicate is consulted before descending.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

use gritty_search::query::IgnorePredicate;
use gritty_search::{MAX_FIND_LIMIT, RankedFiles, SearchError, rank_files, rank_workspace_files};
use gritty_workspace::fixtures::{generate_large_repo, large_repo_is_current};
use gritty_workspace::{WorkspacePath, WorkspaceRoot};

fn repo_relative(parts: &[&str]) -> PathBuf {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..");
    let mut path = gritty_workspace::canonicalize(&repo).expect("repository root exists");
    path.extend(parts);
    path
}

fn small_repo() -> WorkspaceRoot {
    WorkspaceRoot::open(&repo_relative(&["fixtures", "small-repo"])).expect("open small-repo")
}

fn large_repo() -> WorkspaceRoot {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    let dir = DIR.get_or_init(|| {
        let dir = repo_relative(&["target", "fixtures", "large-repo"]);
        if !large_repo_is_current(&dir) {
            let summary = generate_large_repo(&dir).expect("generate large-repo");
            assert!(summary.files >= 10_000, "{summary:?}");
        }
        dir
    });
    WorkspaceRoot::open(dir).expect("open large-repo")
}

fn never_cancelled() -> impl Fn() -> bool {
    || false
}

fn rank(root: &WorkspaceRoot, query: &str, limit: usize) -> RankedFiles {
    rank_workspace_files(root, query, limit, &never_cancelled()).expect("ranked walk succeeds")
}

fn paths(found: &RankedFiles) -> Vec<String> {
    found
        .matches
        .iter()
        .map(|item| item.path.as_str().to_string())
        .collect()
}

/// A scratch directory removed on drop.
struct TempTree {
    path: PathBuf,
}

impl TempTree {
    fn new(label: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = repo_relative(&["target", "fixtures"]).join(format!("rank-{label}-{n}"));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("create scratch tree");
        Self { path }
    }

    fn write(&self, relative: &str, body: &str) {
        let target = self.path.join(relative);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).expect("create parent");
        }
        fs::write(target, body).expect("write scratch file");
    }

    fn root(&self) -> WorkspaceRoot {
        WorkspaceRoot::open(&self.path).expect("open scratch tree")
    }
}

impl Drop for TempTree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn the_best_match_for_a_basename_query_is_that_file() {
    let found = rank(&small_repo(), "leaf", 10);
    assert_eq!(
        paths(&found).first().map(String::as_str),
        Some("src/lib/nested/deep/leaf.ts"),
        "{:?}",
        paths(&found)
    );
}

#[test]
fn an_initialism_finds_the_nested_file_it_abbreviates() {
    // `l`, `n`, `d`: three word starts down one path, none of them adjacent.
    let found = rank(&small_repo(), "lnd", 10);
    assert_eq!(
        paths(&found).first().map(String::as_str),
        Some("src/lib/nested/deep/leaf.ts"),
        "{:?}",
        paths(&found)
    );
}

#[test]
fn results_are_ordered_best_first_and_never_by_walk_order() {
    let found = rank(&small_repo(), "ts", 20);
    let scores: Vec<i32> = found.matches.iter().map(|item| item.score).collect();
    let mut sorted = scores.clone();
    sorted.sort_unstable_by(|a, b| b.cmp(a));
    assert_eq!(scores, sorted, "scores must descend: {:?}", paths(&found));
}

#[test]
fn two_runs_over_one_tree_agree_exactly() {
    // Nothing is cached, so the guarantee has to come from the ordering being
    // total: score descending, then path ascending. A heap alone would not.
    let root = small_repo();
    let first = rank(&root, "ts", 20);
    let second = rank(&root, "ts", 20);
    assert_eq!(first.matches, second.matches);
}

#[test]
fn a_score_tie_breaks_on_the_path_not_on_the_filesystem() {
    let tree = TempTree::new("ties");
    // Same basename at the same depth: identical scores by construction.
    for name in ["b", "a", "c"] {
        tree.write(&format!("{name}/same.ts"), "x");
    }
    let found = rank(&tree.root(), "same", 10);
    assert_eq!(paths(&found), vec!["a/same.ts", "b/same.ts", "c/same.ts"]);
    let scores: Vec<i32> = found.matches.iter().map(|item| item.score).collect();
    assert_eq!(
        scores
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        1,
        "the fixture is only meaningful while the three tie: {scores:?}"
    );
}

#[test]
fn a_query_that_matches_nothing_is_an_empty_list_not_an_error() {
    let found = rank(&small_repo(), "zzzqqq", 10);
    assert!(found.matches.is_empty());
    assert!(
        !found.truncated,
        "nothing was discarded, so nothing is truncated"
    );
    assert!(found.files_scanned > 0, "the walk still happened");
}

#[test]
fn an_empty_query_returns_the_first_limit_paths_and_says_it_truncated() {
    let root = small_repo();
    let found = rank(&root, "", 3);
    assert_eq!(found.matches.len(), 3);
    assert!(
        found
            .matches
            .iter()
            .all(|item| item.score == 0 && item.ranges.is_empty()),
        "an empty query ranks nothing"
    );
    assert!(found.truncated, "small-repo has more than three files");
}

#[test]
fn limit_is_honoured_and_truncation_is_reported_only_when_it_bites() {
    let root = small_repo();
    let one = rank(&root, "ts", 1);
    assert_eq!(one.matches.len(), 1);
    assert!(one.truncated, "small-repo has more than one .ts path");

    // A limit above the number of matches discards nothing.
    let all = rank(&root, "leaf", 50);
    assert!(!all.matches.is_empty());
    assert!(
        !all.truncated,
        "one match and room for fifty: {:?}",
        paths(&all)
    );
}

#[test]
fn a_limit_above_the_cap_is_clamped_rather_than_trusted() {
    let found = rank(&large_repo(), "ts", MAX_FIND_LIMIT + 500);
    assert!(
        found.matches.len() <= MAX_FIND_LIMIT,
        "returned {} rows, above the {MAX_FIND_LIMIT} cap",
        found.matches.len()
    );
}

#[test]
fn keeping_the_best_limit_is_not_keeping_the_first_limit() {
    // The heap must evict, not stop. A walk that simply stopped at `limit`
    // would return whatever it saw first, which is the bug this asserts is not
    // present: the single best match lives deepest in the tree.
    let tree = TempTree::new("evicts");
    for index in 0..40 {
        tree.write(&format!("a{index}/other-target-file.ts"), "x");
    }
    tree.write("z/target.ts", "x");
    let found = rank(&tree.root(), "target", 3);
    assert_eq!(
        paths(&found).first().map(String::as_str),
        Some("z/target.ts"),
        "the best match must survive eviction: {:?}",
        paths(&found)
    );
    assert!(found.truncated);
}

#[test]
fn ranges_point_at_the_characters_that_matched() {
    let found = rank(&small_repo(), "leaf", 5);
    let best = found.matches.first().expect("a match");
    let chars: Vec<char> = best.path.as_str().chars().collect();
    let matched: String = best
        .ranges
        .iter()
        .flat_map(|(start, length)| {
            chars[*start as usize..(*start + *length) as usize]
                .iter()
                .copied()
        })
        .collect();
    assert_eq!(matched.to_lowercase(), "leaf");
}

#[test]
fn generated_directories_are_pruned_by_default_and_walked_on_request() {
    let tree = TempTree::new("ignored");
    tree.write("src/keep.ts", "x");
    tree.write("node_modules/react/keep.ts", "x");

    let pruned = rank(&tree.root(), "keep", 10);
    assert_eq!(paths(&pruned), vec!["src/keep.ts"]);
    assert!(pruned.skipped.ignored_directories > 0);

    // `None` means "walk everything", which is how the caller opts in.
    let everything = rank_files(&tree.root(), "keep", 10, None, &never_cancelled())
        .expect("ranked walk succeeds");
    // Both are found; the shallower one still ranks first, because opting into
    // ignored directories changes what is *walked*, not how it is scored.
    assert_eq!(
        paths(&everything),
        vec!["src/keep.ts", "node_modules/react/keep.ts"]
    );
}

#[test]
fn the_ignore_predicate_is_consulted_per_entry_before_descending() {
    // The property that matters is that the walk *asks*, per directory, rather
    // than hard-coding a list of its own.
    let tree = TempTree::new("asks");
    tree.write("src/keep.ts", "x");
    tree.write("skipme/keep.ts", "x");

    let asked: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&asked);
    let predicate: IgnorePredicate = Arc::new(move |path: &WorkspacePath| {
        seen.lock().expect("lock").push(path.as_str().to_string());
        path.as_str().starts_with("skipme")
    });

    let found = rank_files(
        &tree.root(),
        "keep",
        10,
        Some(&predicate),
        &never_cancelled(),
    )
    .expect("ranked walk succeeds");
    assert_eq!(paths(&found), vec!["src/keep.ts"]);
    let asked = asked.lock().expect("lock");
    assert!(
        asked.iter().any(|path| path == "skipme"),
        "the predicate was never asked about the directory it pruned: {asked:?}"
    );
}

#[test]
fn cancellation_stops_the_walk_and_returns_nothing_partial() {
    let cancelled = || true;
    let error = rank_workspace_files(&large_repo(), "ts", 10, &cancelled)
        .expect_err("a cancelled walk fails");
    assert!(matches!(error, SearchError::Cancelled), "{error:?}");
}

#[test]
fn cancellation_part_way_through_is_observed_rather_than_finishing_quietly() {
    let seen = AtomicUsize::new(0);
    // Flips well after the first check, so the walk is genuinely interrupted
    // rather than refused before it starts.
    let cancelled = || seen.fetch_add(1, Ordering::SeqCst) > 2;
    let error = rank_workspace_files(&large_repo(), "ts", 10, &cancelled)
        .expect_err("a cancelled walk fails");
    assert!(matches!(error, SearchError::Cancelled), "{error:?}");
}

#[test]
fn a_query_above_the_character_limit_fails_before_the_walk_starts() {
    let root = WorkspaceRoot::open(&repo_relative(&["fixtures", "small-repo"])).expect("open");
    let error = rank_workspace_files(&root, &"x".repeat(2_000), 10, &never_cancelled())
        .expect_err("an over-long query is refused");
    assert!(
        matches!(error, SearchError::InvalidQuery { field: "query", .. }),
        "{error:?}"
    );
}

#[test]
fn a_root_that_vanished_is_an_error_and_not_an_empty_result() {
    let tree = TempTree::new("vanished");
    tree.write("a.ts", "x");
    let root = tree.root();
    fs::remove_dir_all(&tree.path).expect("remove the tree under the walk");
    let error =
        rank_workspace_files(&root, "a", 10, &never_cancelled()).expect_err("a missing root fails");
    assert!(
        matches!(error, SearchError::Io { .. }),
        "\"the search failed\" and \"there were no matches\" are different answers: {error:?}"
    );
}

#[test]
fn the_large_repo_is_ranked_whole_with_its_generated_directories_pruned() {
    let found = rank(&large_repo(), "ts", MAX_FIND_LIMIT);
    assert!(
        found.files_scanned >= 10_000,
        "scanned {} files",
        found.files_scanned
    );
    assert_eq!(found.matches.len(), MAX_FIND_LIMIT);
    assert!(found.truncated);
    assert!(
        found.matches.iter().all(|item| !item.ranges.is_empty()),
        "every non-empty query's match carries its highlight"
    );
}
