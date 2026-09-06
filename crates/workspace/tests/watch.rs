//! Integration tests for T070's watch: a real `notify` backend against a
//! scratch directory under `std::env::temp_dir()`, never a developer's project.
//!
//! Everything here is inherently timing-dependent — a backend delivers when it
//! delivers — so the assertions are about *what* arrives and how much of it,
//! never about how fast. Timeouts are generous; the coalescing windows are
//! shrunk through [`WatchConfig::for_tests`] so a test asserts the rule rather
//! than waiting out the production one.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::{Duration, Instant};

use gritty_workspace::watch::{ChangeBatch, WatchConfig, WatchEvent, WatchStrategy, watch};
use gritty_workspace::{WorkspacePath, WorkspaceRoot};

/// Long enough that a loaded machine is not the thing under test.
const DELIVERY_TIMEOUT: Duration = Duration::from_secs(10);
/// How long "nothing was reported" waits before believing itself.
const SILENCE: Duration = Duration::from_secs(2);

fn config() -> WatchConfig {
    WatchConfig::for_tests(Duration::from_millis(100), Duration::from_millis(500), 8)
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
            std::env::temp_dir().join(format!("gritty-watch-{label}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("scratch directory");
        Self { path }
    }

    fn root(&self) -> WorkspaceRoot {
        WorkspaceRoot::open(&self.path).expect("open scratch root")
    }

    fn dir(&self, relative: &str) {
        fs::create_dir_all(self.path.join(relative)).expect("create directory");
    }

    fn write(&self, relative: &str, contents: &str) {
        let path = self.path.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent");
        }
        fs::write(path, contents).expect("write file");
    }

    fn remove(&self, relative: &str) {
        let path = self.path.join(relative);
        if path.is_dir() {
            fs::remove_dir_all(path).expect("remove directory");
        } else {
            fs::remove_file(path).expect("remove file");
        }
    }

    fn rename(&self, from: &str, to: &str) {
        fs::rename(self.path.join(from), self.path.join(to)).expect("rename");
    }
}

impl Drop for TempTree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

struct Observed {
    events: Receiver<WatchEvent>,
}

impl Observed {
    fn next(&self) -> WatchEvent {
        self.events
            .recv_timeout(DELIVERY_TIMEOUT)
            .expect("the watch delivered nothing before the timeout")
    }

    fn next_batch(&self) -> ChangeBatch {
        match self.next() {
            WatchEvent::Changed(batch) => batch,
            other => panic!("expected a batch, got {other:?}"),
        }
    }

    /// Every batch delivered until the stream goes quiet for [`SILENCE`].
    fn drain(&self) -> Vec<ChangeBatch> {
        let mut batches = Vec::new();
        loop {
            match self.events.recv_timeout(SILENCE) {
                Ok(WatchEvent::Changed(batch)) => batches.push(batch),
                Ok(WatchEvent::RootMissing) => panic!("unexpected RootMissing"),
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => return batches,
            }
        }
    }

    fn expect_silence(&self) {
        if let Ok(event) = self.events.recv_timeout(SILENCE) {
            panic!("expected nothing to be reported, got {event:?}");
        }
    }
}

fn start(tree: &TempTree, strategy: WatchStrategy) -> (gritty_workspace::Watch, Observed) {
    let (tx, rx) = channel();
    let handle = watch(&tree.root(), config(), strategy, move |event| {
        let _ = tx.send(event);
    })
    .expect("start the watch");
    (handle, Observed { events: rx })
}

/// A backend needs a moment before its first event arrives; prove the watch is
/// live with a sentinel instead of sleeping a guessed interval.
fn settle(tree: &TempTree, observed: &Observed) {
    tree.write(".watch-sentinel", "1");
    let deadline = Instant::now() + DELIVERY_TIMEOUT;
    while Instant::now() < deadline {
        if let Ok(WatchEvent::Changed(batch)) = observed.events.recv_timeout(Duration::from_secs(1))
            && batch.directories.contains(&WorkspacePath::root())
        {
            tree.remove(".watch-sentinel");
            // Absorb the removal's own batch so the test starts from quiet.
            let _ = observed.drain();
            return;
        }
        tree.write(".watch-sentinel", "1");
    }
    panic!("the watch never reported the sentinel");
}

fn wire(path: &str) -> WorkspacePath {
    WorkspacePath::parse(path).expect(path)
}

fn all_directories(batches: &[ChangeBatch]) -> Vec<WorkspacePath> {
    let mut all: Vec<WorkspacePath> = batches
        .iter()
        .flat_map(|batch| batch.directories.iter().cloned())
        .collect();
    all.sort();
    all.dedup();
    all
}

#[test]
fn a_real_create_modify_delete_and_rename_are_coalesced_into_one_batch() {
    let tree = TempTree::new("real");
    tree.dir("src");
    let (_watch, observed) = start(&tree, WatchStrategy::Recursive);
    settle(&tree, &observed);

    tree.write("src/a.txt", "one");
    tree.write("src/a.txt", "two");
    tree.write("src/b.txt", "b");
    tree.rename("src/b.txt", "src/c.txt");
    tree.remove("src/a.txt");

    let batches = observed.drain();
    assert!(!batches.is_empty(), "five changes reported nothing");
    assert_eq!(all_directories(&batches), vec![wire("src")]);
    // Five filesystem operations inside one quiet period, not five events.
    assert!(
        batches.len() <= 2,
        "changes inside one window were not coalesced: {batches:?}"
    );
}

#[test]
fn changes_inside_an_ignored_directory_are_not_reported() {
    let tree = TempTree::new("ignored");
    tree.dir("node_modules/react/dist");
    tree.dir("src");
    let (_watch, observed) = start(&tree, WatchStrategy::Recursive);
    settle(&tree, &observed);

    for n in 0..20 {
        tree.write(&format!("node_modules/react/dist/chunk-{n}.js"), "x");
    }
    tree.write("node_modules/react/package.json", "{}");
    observed.expect_silence();

    // The same change one directory over is still reported, so the silence
    // above is the filter and not a broken watch.
    tree.write("src/main.rs", "fn main() {}");
    assert_eq!(observed.next_batch().directories, vec![wire("src")]);
}

#[test]
fn removing_an_ignored_directory_reports_its_parent() {
    let tree = TempTree::new("ignored-entry");
    tree.dir("crates/x/target/debug");
    let (_watch, observed) = start(&tree, WatchStrategy::Recursive);
    settle(&tree, &observed);

    tree.remove("crates/x/target");
    let batches = observed.drain();
    assert!(
        all_directories(&batches).contains(&wire("crates/x")),
        "the ignored directory's own removal changes its parent: {batches:?}"
    );
}

#[test]
fn gritty_s_own_atomic_save_reports_only_the_destination() {
    let tree = TempTree::new("atomic");
    tree.dir("docs");
    let (_watch, observed) = start(&tree, WatchStrategy::Recursive);
    settle(&tree, &observed);

    // What `write_file_atomic` does: temporary beside the destination, then a
    // rename over it. The temporary must not be a change of its own.
    let temp = format!(".notes.md.gritty-{}-0.tmp", std::process::id());
    tree.write(&format!("docs/{temp}"), "new contents");
    tree.rename(&format!("docs/{temp}"), "docs/notes.md");

    let batches = observed.drain();
    assert_eq!(all_directories(&batches), vec![wire("docs")]);
}

#[test]
fn a_storm_is_bounded_and_the_last_batch_converges() {
    let tree = TempTree::new("storm");
    for n in 0..40 {
        tree.dir(&format!("d{n:02}"));
    }
    let (_watch, observed) = start(&tree, WatchStrategy::Recursive);
    settle(&tree, &observed);

    // 40 directories against a cap of 8: the batch must overflow rather than
    // name a partial list, and the renderer must not receive one event per file.
    for round in 0..10 {
        for n in 0..40 {
            tree.write(&format!("d{n:02}/f{round}.txt"), "x");
        }
    }
    let batches = observed.drain();
    assert!(!batches.is_empty(), "400 files reported nothing");
    assert!(
        batches.len() < 40,
        "400 file writes produced {} batches; the storm reached the renderer",
        batches.len()
    );
    assert!(
        batches.iter().any(|batch| batch.overflowed),
        "40 changed directories against a cap of 8 must overflow: {batches:?}"
    );
    for batch in &batches {
        assert!(
            batch.directories.len() <= 8,
            "a batch named more directories than the cap allows: {batch:?}"
        );
        assert!(
            !batch.overflowed || batch.directories.is_empty(),
            "an overflowed batch must name nothing: {batch:?}"
        );
    }

    // Once the storm stops, one more change is reported normally: the watch
    // converged instead of latching.
    tree.write("d00/after.txt", "x");
    let batch = observed.next_batch();
    assert!(!batch.overflowed);
    assert_eq!(batch.directories, vec![wire("d00")]);
}

#[test]
fn deleting_the_root_reports_root_missing() {
    let tree = TempTree::new("root-gone");
    let (_watch, observed) = start(&tree, WatchStrategy::Recursive);
    settle(&tree, &observed);

    fs::remove_dir_all(&tree.path).expect("remove the root");
    let deadline = Instant::now() + DELIVERY_TIMEOUT;
    loop {
        assert!(
            Instant::now() < deadline,
            "the root's removal was never reported"
        );
        match observed.events.recv_timeout(Duration::from_secs(1)) {
            Ok(WatchEvent::RootMissing) => break,
            // The removal of the root's own contents may be reported first.
            Ok(WatchEvent::Changed(_)) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => panic!("the watch stopped without reporting"),
        }
    }
}

#[test]
fn dropping_the_handle_stops_delivery() {
    let tree = TempTree::new("stop");
    tree.dir("src");
    let (handle, observed) = start(&tree, WatchStrategy::Recursive);
    settle(&tree, &observed);

    drop(handle);
    for n in 0..50 {
        tree.write(&format!("src/after-{n}.txt"), "x");
    }
    observed.expect_silence();
}

/// The strategy Linux runs, exercised here: `WatchStrategy` is a runtime value
/// precisely so the platform that is not this one is tested rather than merely
/// compiled (hard rule 11). What is *not* proven on macOS is inotify's
/// descriptor accounting — only that the registration walk, the pruning and
/// the follow-the-tree logic are right.
#[test]
fn the_per_directory_strategy_reports_the_same_changes() {
    let tree = TempTree::new("per-dir");
    tree.dir("src/nested/deeper");
    tree.dir("node_modules/react");
    let (_watch, observed) = start(&tree, WatchStrategy::PerDirectory);
    settle(&tree, &observed);

    tree.write("src/nested/deeper/x.rs", "x");
    let batches = observed.drain();
    assert_eq!(all_directories(&batches), vec![wire("src/nested/deeper")]);

    tree.write("node_modules/react/index.js", "x");
    observed.expect_silence();
}

#[test]
fn a_directory_created_after_the_watch_started_is_followed() {
    let tree = TempTree::new("per-dir-new");
    let (_watch, observed) = start(&tree, WatchStrategy::PerDirectory);
    settle(&tree, &observed);

    tree.dir("late/inner");
    let _ = observed.drain();
    tree.write("late/inner/file.txt", "x");

    let batches = observed.drain();
    assert!(
        all_directories(&batches).contains(&wire("late/inner")),
        "a directory created after the walk was never watched: {batches:?}"
    );
}

#[test]
fn a_failing_backend_start_is_an_error_not_a_panic() {
    // A root that exists when it is opened and is gone by the time the watch
    // starts: the workspace must still be usable, just not proactively.
    let tree = TempTree::new("gone-before-watch");
    let root = tree.root();
    fs::remove_dir_all(&tree.path).expect("remove the root");
    let started = watch(&root, config(), WatchStrategy::Recursive, |_| {});
    if let Ok(handle) = started {
        // Some backends accept a missing path and report it later; either way
        // nothing may panic and the handle must stop cleanly.
        drop(handle);
    }
    assert!(!Path::new(&tree.path).exists());
}

/// A6 and B5, on the fixture the spike named: `fixtures/large-repo` (10 501
/// files, 103 directories, depth 25), stormed with rapid creates and removes.
///
/// `#[ignore]` because it generates and then churns a ten-thousand-file tree —
/// too slow and too disk-heavy for every `pnpm check` — and because its
/// interesting output is a *measurement*, which the phase-0 lesson says must
/// not become a threshold: one calibrated on this Mac would fail in CI. What
/// it does assert are the properties rather than the timings: the cap holds,
/// no ignored subtree is ever named, the storm does not reach the consumer one
/// event per file, and one change after it is reported on its own.
///
/// Run it, and record what it prints:
/// `cargo test -p gritty-workspace --test watch -- --ignored --nocapture`
#[test]
#[ignore = "generates and storms a 10k-file tree; run explicitly for A6/B5"]
fn the_large_repo_storm_stays_batched_and_bounded() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("target")
        .join("fixtures")
        .join("large-repo");
    fs::create_dir_all(&dir).expect("fixture directory");
    let dir = gritty_workspace::canonicalize(&dir).expect("canonical fixture directory");
    if !gritty_workspace::fixtures::large_repo_is_current(&dir) {
        let summary = gritty_workspace::fixtures::generate_large_repo(&dir).expect("generate");
        assert!(summary.files >= 10_000, "{summary:?}");
    }
    let root = WorkspaceRoot::open(&dir).expect("open large-repo");

    let (tx, rx) = channel();
    let started = Instant::now();
    let handle = watch(
        &root,
        WatchConfig::default(),
        WatchStrategy::for_platform(),
        move |event| {
            let _ = tx.send(event);
        },
    )
    .expect("start the watch");
    let registration = started.elapsed();

    // The storm: creates and removes across the fifty `src` modules and down
    // the twenty-four-level `deep` chain — more than the sixty-four-directory
    // cap on purpose, so the overflow path runs on the real fixture and not
    // only on a synthetic one.
    let storm = Instant::now();
    let mut operations = 0_u64;
    let mut deep = dir.join("deep");
    let deep_chain: Vec<PathBuf> = (1..=24)
        .map(|level| {
            deep = deep.join(format!("level_{level:02}"));
            deep.clone()
        })
        .collect();
    for round in 0..5 {
        for module in 0..50 {
            let file = dir.join(format!("src/module_{module:02}/storm-{round}.rs"));
            if fs::write(&file, "// storm").is_ok() {
                operations += 1;
                if round % 2 == 1 && fs::remove_file(&file).is_ok() {
                    operations += 1;
                }
            }
        }
        for level in &deep_chain {
            let file = level.join(format!("storm-{round}.md"));
            if fs::write(&file, "storm").is_ok() {
                operations += 1;
                if fs::remove_file(&file).is_ok() {
                    operations += 1;
                }
            }
        }
    }
    let storm_elapsed = storm.elapsed();

    let mut batches = Vec::new();
    while let Ok(event) = rx.recv_timeout(Duration::from_secs(3)) {
        match event {
            WatchEvent::Changed(batch) => batches.push(batch),
            WatchEvent::RootMissing => panic!("the fixture root vanished"),
        }
    }

    let named: usize = batches.iter().map(|batch| batch.directories.len()).sum();
    let overflowed = batches.iter().filter(|batch| batch.overflowed).count();
    println!(
        "large-repo storm: registration {registration:?}; {operations} filesystem operations in \
         {storm_elapsed:?} produced {} batches ({named} directory names, {overflowed} overflowed)",
        batches.len()
    );

    assert!(!batches.is_empty(), "the storm reported nothing");
    assert!(
        batches.len() < operations as usize / 10,
        "{operations} operations produced {} batches: the storm reached the consumer",
        batches.len()
    );
    for batch in &batches {
        assert!(batch.directories.len() <= gritty_workspace::MAX_DIRECTORIES);
        assert!(!batch.overflowed || batch.directories.is_empty());
        for directory in &batch.directories {
            assert!(
                !gritty_workspace::is_ignored(directory),
                "an ignored subtree was reported: {directory}"
            );
        }
    }

    // It converges: one change after the storm is reported on its own.
    let after = dir.join("docs/after-the-storm.md");
    fs::write(&after, "done").expect("write");
    let batch = match rx
        .recv_timeout(DELIVERY_TIMEOUT)
        .expect("no batch after the storm")
    {
        WatchEvent::Changed(batch) => batch,
        other => panic!("expected a batch, got {other:?}"),
    };
    assert_eq!(batch.directories, vec![wire("docs")]);
    let _ = fs::remove_file(&after);
    drop(handle);
}
