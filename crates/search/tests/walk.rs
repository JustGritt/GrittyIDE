//! Integration tests for the walk: the committed `fixtures/small-repo`, a
//! staged copy of `fixtures/weird-files`, the generated `large-repo`, and
//! scratch trees for the cases no fixture should carry.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use gritty_search::model::MatchKind;
use gritty_search::{
    MAX_SNIPPET_CHARS, Query, SCAN_CHUNK_BYTES, SearchError, SearchMatch, SearchScope,
    SearchSummary, search,
};
use gritty_workspace::fixtures::{
    WEIRD_DENIED, WEIRD_FILES_DIR, WEIRD_LARGE_TEXT, generate_large_repo, large_repo_is_current,
    stage_weird_files,
};
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

/// A copy of `weird-files` staged under a name of this crate's own, so two
/// test binaries never stage over each other.
fn weird() -> WorkspaceRoot {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    let dir = DIR.get_or_init(|| {
        let committed = repo_relative(&["fixtures", WEIRD_FILES_DIR]);
        let dest = repo_relative(&["target", "fixtures", "weird-files-search"]);
        let summary = stage_weird_files(&committed, &dest).expect("stage weird-files");
        assert!(summary.copied_files >= 20, "{summary:?}");
        dest
    });
    WorkspaceRoot::open(dir).expect("open staged weird-files")
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

/// A scratch directory removed on drop.
struct TempTree {
    path: PathBuf,
}

impl TempTree {
    fn new(label: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let path =
            std::env::temp_dir().join(format!("gritty-search-{label}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    fn write(&self, relative: &str, contents: impl AsRef<[u8]>) {
        let path = self.path.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
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

/// Collects every match; never asks the walk to stop.
#[derive(Default)]
struct Collected {
    matches: Vec<SearchMatch>,
    progress: Vec<u64>,
}

impl gritty_search::SearchSink for Collected {
    fn matched(&mut self, found: SearchMatch) -> bool {
        self.matches.push(found);
        true
    }

    fn scanned(&mut self, files: u64) {
        self.progress.push(files);
    }
}

fn run(root: &WorkspaceRoot, query: Query) -> (Collected, SearchSummary) {
    let compiled = query.compile().expect("compile");
    let mut sink = Collected::default();
    let summary = search(root, &compiled, &mut sink, &|| false).expect("search");
    (sink, summary)
}

fn paths(collected: &Collected) -> Vec<String> {
    let mut paths: Vec<String> = collected
        .matches
        .iter()
        .map(|found| found.path.to_string())
        .collect();
    paths.sort();
    paths.dedup();
    paths
}

fn of(collected: &Collected, path: &str) -> Vec<SearchMatch> {
    collected
        .matches
        .iter()
        .filter(|found| found.path.as_str() == path)
        .cloned()
        .collect()
}

// --- content, filenames, positions -----------------------------------------

#[test]
fn a_content_query_reports_the_path_line_and_column() {
    let (collected, summary) = run(
        &small_repo(),
        Query {
            scope: SearchScope::Content,
            ..Query::literal("greet")
        },
    );
    assert!(
        paths(&collected).contains(&"src/lib/util.ts".to_owned()),
        "{:?}",
        paths(&collected)
    );
    let util = of(&collected, "src/lib/util.ts");
    let first = util.first().expect("a match in util.ts");
    assert_eq!(first.kind, MatchKind::Content);
    assert_eq!(first.line, Some(1));
    // `export function greet(...)` — "greet" starts at the 17th character.
    assert_eq!(first.column, Some(17));
    assert_eq!(
        first.snippet,
        "export function greet(name: string): string {"
    );
    assert!(!first.truncated);
    assert_eq!(first.match_start, 16);
    assert_eq!(first.match_length, 5);
    assert!(summary.files_scanned > 5, "{summary:?}");
    assert!(summary.matches_found >= 3, "{summary:?}");
}

#[test]
fn a_filename_query_reports_paths_with_no_line_or_column() {
    let (collected, _) = run(
        &small_repo(),
        Query {
            scope: SearchScope::Filenames,
            ..Query::literal("util")
        },
    );
    assert_eq!(paths(&collected), vec!["src/lib/util.ts".to_owned()]);
    let found = &collected.matches[0];
    assert_eq!(found.kind, MatchKind::Filename);
    assert_eq!(found.line, None);
    assert_eq!(found.column, None);
    assert_eq!(found.snippet, "src/lib/util.ts");
}

#[test]
fn filename_scope_opens_no_file_at_all() {
    let (_, summary) = run(
        &small_repo(),
        Query {
            scope: SearchScope::Filenames,
            ..Query::literal("util")
        },
    );
    assert_eq!(summary.bytes_scanned, 0, "{summary:?}");
    assert_eq!(summary.skipped.total(), 0, "{summary:?}");
}

#[test]
fn both_is_the_default_and_finds_a_name_and_a_line_at_once() {
    let (collected, _) = run(&small_repo(), Query::literal("util"));
    let kinds: Vec<MatchKind> = collected.matches.iter().map(|found| found.kind).collect();
    assert!(kinds.contains(&MatchKind::Filename), "{kinds:?}");
    assert!(kinds.contains(&MatchKind::Content), "{kinds:?}");
}

#[test]
fn case_sensitivity_whole_word_and_regex_are_honoured() {
    let tree = TempTree::new("options");
    tree.write("a.txt", "Needle\nneedle\nneedles\n");
    let root = tree.root();

    let (insensitive, _) = run(&root, Query::literal("needle"));
    assert_eq!(insensitive.matches.len(), 3, "case-insensitive by default");

    let (sensitive, _) = run(
        &root,
        Query {
            case_sensitive: true,
            ..Query::literal("needle")
        },
    );
    assert_eq!(sensitive.matches.len(), 2);

    let (whole, _) = run(
        &root,
        Query {
            whole_word: true,
            ..Query::literal("needle")
        },
    );
    assert_eq!(whole.matches.len(), 2, "`needles` is not the whole word");

    let (regex, _) = run(
        &root,
        Query {
            use_regex: true,
            scope: SearchScope::Content,
            ..Query::literal("^needles$")
        },
    );
    assert_eq!(regex.matches.len(), 1);
    assert_eq!(regex.matches[0].line, Some(3));
}

// --- binary, encoding, size, permissions -----------------------------------

#[test]
fn binary_and_invalid_utf8_files_are_skipped_counted_and_never_carried() {
    let root = weird();
    // Every one of these bytes appears in the binary fixtures; a match would
    // mean their content was decoded.
    let (collected, summary) = run(
        &root,
        Query {
            scope: SearchScope::Content,
            ..Query::literal("e")
        },
    );
    for path in paths(&collected) {
        assert!(
            !path.starts_with("binary/") && !path.starts_with("encoding/invalid-utf8"),
            "{path} must never appear in a content match"
        );
    }
    assert!(summary.skipped.binary >= 2, "{summary:?}");
    assert!(summary.skipped.invalid_utf8 >= 1, "{summary:?}");
}

#[test]
fn an_oversized_line_yields_one_truncated_match_not_a_dropped_file() {
    let (collected, summary) = run(
        &weird(),
        Query {
            scope: SearchScope::Content,
            ..Query::literal("0123456789")
        },
    );
    let long = of(&collected, "long-lines/one-line-100k.txt");
    assert_eq!(long.len(), 1, "one line, one match");
    let found = &long[0];
    assert!(found.truncated, "a 100k line must report truncation");
    assert!(
        found.snippet.chars().count() <= MAX_SNIPPET_CHARS + 2,
        "snippet is {} characters",
        found.snippet.chars().count()
    );
    assert_eq!(found.line, Some(1));
    assert!(summary.snippets_truncated >= 1, "{summary:?}");
}

#[test]
fn a_file_above_the_byte_bound_is_counted_rather_than_read() {
    let tree = TempTree::new("too-large");
    tree.write("small.txt", "needle\n");
    tree.write("big.txt", "needle\n".repeat(1_000));
    let compiled = Query {
        scope: SearchScope::Content,
        ..Query::literal("needle")
    }
    .compile()
    .unwrap()
    .with_max_file_bytes(100);
    let mut sink = Collected::default();
    let summary = search(&tree.root(), &compiled, &mut sink, &|| false).expect("search");
    assert_eq!(paths(&sink), vec!["small.txt".to_owned()]);
    assert_eq!(summary.skipped.too_large, 1, "{summary:?}");
}

#[cfg(unix)]
#[test]
fn an_unreadable_file_is_skipped_and_counted_not_an_error_of_the_walk() {
    let root = weird();
    if fs::read(root.path().join(WEIRD_DENIED)).is_ok() {
        eprintln!("this process bypasses permission bits (root); nothing to assert");
        return;
    }
    let (_, summary) = run(
        &root,
        Query {
            scope: SearchScope::Content,
            ..Query::literal("you should not be able")
        },
    );
    assert!(
        fs::metadata(root.path().join(WEIRD_DENIED)).is_ok(),
        "the denied fixture must exist"
    );
    assert!(summary.skipped.unreadable >= 1, "{summary:?}");
    assert!(summary.files_scanned > 0, "the rest of the tree still ran");
}

// --- ignore consultation ---------------------------------------------------

#[test]
fn ignored_directories_are_pruned_by_default_and_scanned_on_request() {
    let tree = TempTree::new("ignored");
    tree.write("src/a.txt", "needle\n");
    tree.write("node_modules/pkg/b.txt", "needle\n");
    tree.write("target/debug/c.txt", "needle\n");
    let root = tree.root();

    let (default, summary) = run(&root, Query::literal("needle"));
    assert_eq!(paths(&default), vec!["src/a.txt".to_owned()]);
    assert_eq!(summary.skipped.ignored_directories, 2, "{summary:?}");

    let (included, summary) = run(
        &root,
        Query {
            include_ignored: true,
            ..Query::literal("needle")
        },
    );
    assert_eq!(
        paths(&included),
        vec![
            "node_modules/pkg/b.txt".to_owned(),
            "src/a.txt".to_owned(),
            "target/debug/c.txt".to_owned(),
        ]
    );
    assert_eq!(summary.skipped.ignored_directories, 0, "{summary:?}");
}

#[test]
fn the_ignore_predicate_is_consulted_per_entry_before_descending() {
    let tree = TempTree::new("predicate");
    tree.write("keep/a.txt", "needle\n");
    tree.write("drop/deep/deeper/b.txt", "needle\n");
    let asked: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let recorder = Arc::clone(&asked);
    let compiled = Query::literal("needle")
        .compile()
        .unwrap()
        .with_ignore_predicate(Arc::new(move |path: &WorkspacePath| {
            recorder.lock().unwrap().push(path.to_string());
            path.segments().any(|segment| segment == "drop")
        }));
    let mut sink = Collected::default();
    let summary = search(&tree.root(), &compiled, &mut sink, &|| false).expect("search");

    assert_eq!(paths(&sink), vec!["keep/a.txt".to_owned()]);
    let asked = asked.lock().unwrap().clone();
    assert!(asked.contains(&"drop".to_owned()), "{asked:?}");
    assert!(asked.contains(&"keep".to_owned()), "{asked:?}");
    assert!(
        !asked.iter().any(|path| path.starts_with("drop/deep")),
        "the walk descended into a directory it was told to skip: {asked:?}"
    );
    assert_eq!(summary.skipped.ignored_directories, 1, "{summary:?}");
}

// --- symlinks --------------------------------------------------------------

#[test]
fn a_symlinked_directory_is_never_walked_into() {
    // `small-repo/link-to-src` points at `src`. Walking through it would
    // report every match under `src/` a second time.
    let (collected, _) = run(
        &small_repo(),
        Query {
            scope: SearchScope::Content,
            ..Query::literal("greet")
        },
    );
    assert!(
        !paths(&collected)
            .iter()
            .any(|path| path.starts_with("link-to-src")),
        "{:?}",
        paths(&collected)
    );
}

#[cfg(unix)]
#[test]
fn a_symlinked_file_is_read_inside_the_root_and_refused_outside_it() {
    let outside = TempTree::new("outside");
    outside.write("secret.txt", "needle outside\n");
    let tree = TempTree::new("links");
    tree.write("real.txt", "needle inside\n");
    std::os::unix::fs::symlink("real.txt", tree.path.join("to-real")).unwrap();
    std::os::unix::fs::symlink(outside.path.join("secret.txt"), tree.path.join("escape")).unwrap();

    let (collected, summary) = run(
        &tree.root(),
        Query {
            scope: SearchScope::Content,
            ..Query::literal("needle")
        },
    );
    let found = paths(&collected);
    assert!(found.contains(&"real.txt".to_owned()), "{found:?}");
    assert!(
        found.contains(&"to-real".to_owned()),
        "a link inside the root is followed: {found:?}"
    );
    assert!(
        !found.contains(&"escape".to_owned()),
        "a link out of the root is never read: {found:?}"
    );
    assert_eq!(summary.skipped.outside_root, 1, "{summary:?}");
}

// --- chunking and line numbers ---------------------------------------------

#[test]
fn line_numbers_stay_exact_across_chunk_boundaries() {
    let tree = TempTree::new("chunks");
    // Three chunks' worth of identical-length lines, with one distinctive line
    // well past the first boundary.
    let filler = "the quick brown fox jumps over the lazy dog padding padding\n";
    let per_chunk = SCAN_CHUNK_BYTES / filler.len();
    let target_line = per_chunk * 2 + 7;
    let mut body = String::with_capacity(SCAN_CHUNK_BYTES * 3);
    for index in 1..=(per_chunk * 3) {
        if index == target_line {
            body.push_str("here is the needle\n");
        } else {
            body.push_str(filler);
        }
    }
    tree.write("big.txt", &body);

    let (collected, summary) = run(
        &tree.root(),
        Query {
            scope: SearchScope::Content,
            ..Query::literal("needle")
        },
    );
    assert_eq!(collected.matches.len(), 1);
    assert_eq!(
        collected.matches[0].line,
        Some(target_line as u64),
        "a match in the third chunk keeps its absolute line number"
    );
    assert_eq!(collected.matches[0].column, Some(13));
    assert!(summary.bytes_scanned as usize >= SCAN_CHUNK_BYTES * 2);
}

#[test]
fn a_twelve_mib_fixture_is_scanned_with_exact_line_numbers() {
    let root = weird();
    let (collected, summary) = run(
        &root,
        Query {
            scope: SearchScope::Content,
            case_sensitive: true,
            ..Query::literal("line 00090000:")
        },
    );
    let found = of(&collected, WEIRD_LARGE_TEXT);
    assert_eq!(found.len(), 1, "{:?}", paths(&collected));
    assert_eq!(
        found[0].line,
        Some(90_001),
        "line 00090000 is the 90001st line"
    );
    assert!(
        summary.bytes_scanned > 12 * 1024 * 1024,
        "the 12 MiB fixture was read: {summary:?}"
    );
}

// --- cancellation ----------------------------------------------------------

#[test]
fn cancellation_stops_the_walk_between_files() {
    let root = large_repo();
    let polls = AtomicUsize::new(0);
    let compiled = Query::literal("fn ").compile().unwrap();
    let mut sink = Collected::default();
    let error = search(&root, &compiled, &mut sink, &|| {
        polls.fetch_add(1, Ordering::SeqCst) >= 32
    })
    .expect_err("cancelled");
    assert!(matches!(error, SearchError::Cancelled), "{error:?}");
    assert!(
        polls.load(Ordering::SeqCst) <= 64,
        "the walk kept going long after it was cancelled: {} polls",
        polls.load(Ordering::SeqCst)
    );
}

#[test]
fn cancellation_stops_inside_one_large_file_without_finishing_it() {
    let tree = TempTree::new("cancel-inside");
    let line = "needle in a haystack of padding padding padding padding pad\n";
    let lines = (SCAN_CHUNK_BYTES * 4) / line.len();
    tree.write("one.txt", line.repeat(lines));
    let root = tree.root();

    /// Cancels as soon as the walk has delivered anything at all, which is
    /// partway through the first chunk of the only file there is.
    struct CancelOnFirst {
        seen: Arc<AtomicUsize>,
    }
    impl gritty_search::SearchSink for CancelOnFirst {
        fn matched(&mut self, _found: SearchMatch) -> bool {
            self.seen.fetch_add(1, Ordering::SeqCst);
            true
        }
    }
    let seen = Arc::new(AtomicUsize::new(0));
    let compiled = Query {
        scope: SearchScope::Content,
        ..Query::literal("needle")
    }
    .compile()
    .unwrap();
    let mut sink = CancelOnFirst {
        seen: Arc::clone(&seen),
    };
    let watcher = Arc::clone(&seen);
    let error = search(&root, &compiled, &mut sink, &|| {
        watcher.load(Ordering::SeqCst) > 0
    })
    .expect_err("cancelled");

    assert!(matches!(error, SearchError::Cancelled), "{error:?}");
    let delivered = seen.load(Ordering::SeqCst);
    assert!(
        delivered > 0,
        "matches found before the cancellation are still delivered"
    );
    assert!(
        delivered < lines,
        "the 4 MiB file was scanned to the end anyway: {delivered} of {lines}"
    );
}

#[test]
fn a_root_that_disappeared_fails_rather_than_reporting_no_matches() {
    let tree = TempTree::new("vanished");
    tree.write("a.txt", "needle\n");
    let root = tree.root();
    fs::remove_dir_all(&tree.path).unwrap();

    let compiled = Query::literal("needle").compile().unwrap();
    let mut sink = Collected::default();
    let error = search(&root, &compiled, &mut sink, &|| false).expect_err("io");
    assert!(matches!(error, SearchError::Io { .. }), "{error:?}");
    assert!(error.to_string().contains("workspace root"), "{error}");
}

#[test]
fn cancelling_before_the_first_entry_reads_nothing() {
    let compiled = Query::literal("greet").compile().unwrap();
    let mut sink = Collected::default();
    let error = search(&small_repo(), &compiled, &mut sink, &|| true).expect_err("cancelled");
    assert!(matches!(error, SearchError::Cancelled), "{error:?}");
    assert!(sink.matches.is_empty());
}

// --- the sink's own bound --------------------------------------------------

#[test]
fn a_sink_that_stops_the_walk_is_obeyed_and_recorded() {
    struct Two {
        seen: usize,
    }
    impl gritty_search::SearchSink for Two {
        fn matched(&mut self, _found: SearchMatch) -> bool {
            self.seen += 1;
            self.seen < 2
        }
    }
    let compiled = Query::literal("greet").compile().unwrap();
    let mut sink = Two { seen: 0 };
    let summary = search(&small_repo(), &compiled, &mut sink, &|| false).expect("search");
    assert_eq!(sink.seen, 2);
    assert!(summary.limit_reached, "{summary:?}");
}

// --- volume ----------------------------------------------------------------

#[test]
fn the_large_repo_is_walked_whole_with_its_generated_directories_pruned() {
    let (collected, summary) = run(
        &large_repo(),
        Query {
            scope: SearchScope::Filenames,
            ..Query::literal("module")
        },
    );
    assert!(summary.files_scanned >= 10_000, "{summary:?}");
    assert!(summary.skipped.ignored_directories > 0, "{summary:?}");
    assert!(
        !paths(&collected)
            .iter()
            .any(|path| path.starts_with("node_modules/") || path.starts_with("target/")),
        "an ignored directory was scanned"
    );
    assert!(
        collected.progress.len() > 100,
        "progress was reported as the walk ran: {:?}",
        collected.progress.len()
    );
    // The number ADR-022's "revisit when" clause is about, printed rather than
    // asserted: a threshold here would be a timing test on shared CI hardware.
    eprintln!(
        "large-repo filenames: {} files in {} ms",
        summary.files_scanned, summary.elapsed_ms
    );
}

/// The same tree, read for real. This is the measurement the "no index"
/// non-goal rests on (T080: "a live walk, proven against `fixtures/large-repo`'s
/// 10,500 files, is the requirement this task has").
#[test]
fn the_large_repo_is_content_searched_in_one_pass() {
    let (_, summary) = run(
        &large_repo(),
        Query {
            scope: SearchScope::Content,
            ..Query::literal("needle-that-is-not-there")
        },
    );
    assert!(summary.files_scanned >= 10_000, "{summary:?}");
    assert_eq!(summary.matches_found, 0);
    eprintln!(
        "large-repo content: {} files, {} bytes in {} ms",
        summary.files_scanned, summary.bytes_scanned, summary.elapsed_ms
    );
}
