//! Watching the open workspace (T070, ADR-025).
//!
//! A watch turns two on-demand checks proactive — the external-modification
//! check ADR-011 runs on `documents.status`, and the explorer's listing — and
//! it does so with the coarsest signal every backend can give honestly: a
//! coalesced, deduplicated, sorted set of **directories** whose entries
//! changed. Never a per-entry delta: FSEvents coalesces by directory and says
//! nothing useful about which entry moved across a rename, so a per-entry
//! protocol would be a promise one platform cannot keep.
//!
//! Everything here exists to keep a filesystem storm from reaching the
//! renderer (hard rule 8):
//!
//! - Gritty's own atomic-save temporaries are dropped before coalescing, so a
//!   debounced autosave never reports its own directory (`is_temp_file_name`).
//! - Generated subtrees are pruned (`is_ignored`); an ignored directory's own
//!   creation or removal is still reported, because that changes its parent.
//! - Changes coalesce on a quiet period ([`WatchConfig::debounce`]) with a
//!   ceiling ([`WatchConfig::max_wait`]) so a continuous flood still flushes.
//! - A batch is capped ([`WatchConfig::max_directories`]); above the cap it
//!   says [`ChangeBatch::overflowed`] with an empty list, rather than
//!   presenting a partial list as complete.
//!
//! When the watch cannot cover the whole tree it says so, once per reason, as
//! [`WatchEvent::Degraded`] (WATCH-DEGRADED, amending ADR-025) — a reason, not
//! a count, because a bounded walk cannot know how much it did not reach.
//!
//! The watch is a background observer, not an operation: it has no start, no
//! completion and no result to cancel. Dropping its [`Watch`] handle stops it
//! without joining the pump thread, so a workspace can close while a batch is
//! pending without the close path ever waiting on the thread that may be
//! calling back into it.

use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use notify::event::{EventKind, ModifyKind};
use notify::{RecursiveMode, Watcher};

use crate::error::WorkspaceError;
use crate::file::is_temp_file_name;
use crate::listing::is_ignored;
use crate::path::WorkspacePath;
use crate::root::WorkspaceRoot;

/// Quiet time after the last observed change before a batch is delivered.
pub const DEBOUNCE: Duration = Duration::from_millis(200);
/// Longest a batch is withheld while changes keep arriving.
pub const MAX_WAIT: Duration = Duration::from_secs(1);
/// Directories a single batch reports before it degrades to `overflowed`.
pub const MAX_DIRECTORIES: usize = 64;
/// How often the pump confirms the root is still there, independently of what
/// the backend reports: a backend that loses its root does not reliably say so.
pub const ROOT_CHECK_INTERVAL: Duration = Duration::from_secs(1);
/// Ceiling on a [`WatchStrategy::PerDirectory`] registration walk. A tree
/// larger than this is watched in part; the rest degrades to pre-T070
/// behaviour rather than consuming the whole per-user descriptor budget.
pub const MAX_WATCHED_DIRECTORIES: usize = 8_192;
/// Backend events absorbed between two deadline checks, so a storm cannot
/// postpone a due batch indefinitely.
const DRAIN_LIMIT: usize = 4_096;

/// Timings and bounds of a watch; tests shrink them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WatchConfig {
    /// Quiet period: no batch until nothing changed for this long.
    pub debounce: Duration,
    /// Ceiling: a batch older than this is delivered even under a flood.
    pub max_wait: Duration,
    /// Largest number of directories a batch names before overflowing.
    pub max_directories: usize,
    /// How often the root's presence is confirmed.
    pub root_check_interval: Duration,
    /// Most directories a [`WatchStrategy::PerDirectory`] watch registers;
    /// beyond it the watch reports [`Degradation::DirectoryLimit`].
    pub max_watched_directories: usize,
}

impl Default for WatchConfig {
    fn default() -> Self {
        Self {
            debounce: DEBOUNCE,
            max_wait: MAX_WAIT,
            max_directories: MAX_DIRECTORIES,
            root_check_interval: ROOT_CHECK_INTERVAL,
            max_watched_directories: MAX_WATCHED_DIRECTORIES,
        }
    }
}

impl WatchConfig {
    /// Milliseconds instead of seconds, so a test asserts behaviour rather
    /// than waiting for it.
    pub fn for_tests(debounce: Duration, max_wait: Duration, max_directories: usize) -> Self {
        Self {
            debounce,
            max_wait,
            max_directories,
            root_check_interval: Duration::from_millis(50),
            max_watched_directories: MAX_WATCHED_DIRECTORIES,
        }
    }

    /// The same, with a registration limit small enough for a test tree to
    /// exceed.
    pub fn with_max_watched_directories(self, max_watched_directories: usize) -> Self {
        Self {
            max_watched_directories,
            ..self
        }
    }
}

/// How the tree is handed to the backend.
///
/// A runtime value rather than a `cfg` so the platform that is not this one
/// can still be exercised by a test (hard rule 11: compiling is not evidence).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchStrategy {
    /// One recursive registration on the root. Ignored subtrees are filtered
    /// out of the event stream rather than left unwatched.
    Recursive,
    /// One non-recursive registration per directory, skipping ignored
    /// subtrees entirely and adding a registration when a directory appears.
    PerDirectory,
}

impl WatchStrategy {
    /// macOS: FSEvents is a single stream for a whole subtree, and `notify`
    /// restarts that stream for every added path, so per-directory
    /// registration would be quadratic for no benefit.
    ///
    /// Linux: inotify costs one descriptor per directory against
    /// `fs.inotify.max_user_watches`, which a `node_modules` tree exhausts on
    /// its own — pruning is the whole point there.
    pub fn for_platform() -> Self {
        if cfg!(target_os = "linux") {
            Self::PerDirectory
        } else {
            Self::Recursive
        }
    }
}

/// One coalesced report: directories whose entry set may have changed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ChangeBatch {
    /// Deduplicated and sorted workspace-relative directory paths. Empty when
    /// `overflowed`.
    pub directories: Vec<WorkspacePath>,
    /// More directories changed than the batch may name. The list is empty,
    /// never partial: a consumer converges by refreshing everything it holds.
    pub overflowed: bool,
}

/// Why part of the tree is not being watched (WATCH-DEGRADED).
///
/// A reason and never a count: the registration walk stops at its limit
/// without knowing how many directories it did not reach, so any number would
/// be a lower bound presented as a total.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Degradation {
    /// The backend refused to register some directory (inotify's per-user
    /// budget, most often). The directories that did register keep reporting.
    RegistrationRefused,
    /// [`WatchConfig::max_watched_directories`] was reached; the rest of the
    /// tree is not registered.
    DirectoryLimit,
    /// The backend reported an error while running.
    BackendError,
}

/// What a watch tells its owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchEvent {
    Changed(ChangeBatch),
    /// The root is gone. The watch has stopped; nothing follows this.
    RootMissing,
    /// Part of the tree is not being watched, for this reason. Delivered at
    /// most once per reason per watch, so a failing backend cannot storm.
    Degraded(Degradation),
}

/// A live watch. Dropping it stops the watch.
pub struct Watch {
    stopped: Arc<AtomicBool>,
    control: Sender<Pulse>,
}

impl std::fmt::Debug for Watch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Watch")
            .field("stopped", &self.stopped.load(Ordering::Acquire))
            .finish()
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        // Set the flag *before* the wake-up so a pump already mid-loop sees
        // it, and never join: the pump may be calling back into the owner
        // that is dropping this handle.
        self.stopped.store(true, Ordering::Release);
        let _ = self.control.send(Pulse::Stop);
    }
}

/// Everything that wakes the pump.
enum Pulse {
    Event(notify::Result<notify::Event>),
    Stop,
}

/// Start watching `root`, delivering coalesced batches to `sink`.
///
/// `sink` runs on the pump thread, is never called after the [`Watch`] is
/// dropped, and must not block: it holds up every later batch.
pub fn watch<S>(
    root: &WorkspaceRoot,
    config: WatchConfig,
    strategy: WatchStrategy,
    sink: S,
) -> Result<Watch, WorkspaceError>
where
    S: Fn(WatchEvent) + Send + 'static,
{
    let root_path = root.path().to_path_buf();
    let (tx, rx) = mpsc::channel::<Pulse>();
    let mut backend = {
        let tx = tx.clone();
        notify::recommended_watcher(move |result| {
            // A closed receiver only means the watch stopped first.
            let _ = tx.send(Pulse::Event(result));
        })
        .map_err(|error| watch_error(&root_path, error))?
    };

    let mut registered = HashSet::new();
    let mut initial = BTreeSet::new();
    match strategy {
        WatchStrategy::Recursive => {
            backend
                .watch(&root_path, RecursiveMode::Recursive)
                .map_err(|error| watch_error(&root_path, error))?;
            registered.insert(root_path.clone());
        }
        WatchStrategy::PerDirectory => {
            // The root itself must register, or there is no watch at all;
            // deeper failures degrade the tree instead of failing the open.
            backend
                .watch(&root_path, RecursiveMode::NonRecursive)
                .map_err(|error| watch_error(&root_path, error))?;
            registered.insert(root_path.clone());
            let limit = config
                .max_watched_directories
                .saturating_sub(registered.len());
            let outcome = register_subtree(
                &mut |directory| {
                    backend
                        .watch(directory, RecursiveMode::NonRecursive)
                        .is_ok()
                },
                &mut registered,
                &root_path,
                limit,
            );
            initial.extend(outcome.degradations());
        }
    }

    let stopped = Arc::new(AtomicBool::new(false));
    let pump = Pump {
        root: root_path,
        strategy,
        coalescer: Coalescer::new(config),
        stopped: Arc::clone(&stopped),
        registered,
        sink,
        reported: BTreeSet::new(),
        initial,
    };
    std::thread::Builder::new()
        .name("gritty-watch".to_owned())
        .spawn(move || pump.run(backend, rx))
        .map_err(|error| WorkspaceError::Io {
            context: "starting the workspace watcher".to_owned(),
            source: error,
        })?;

    Ok(Watch {
        stopped,
        control: tx,
    })
}

fn watch_error(root: &Path, error: notify::Error) -> WorkspaceError {
    WorkspaceError::Io {
        context: format!("watching {}", root.display()),
        source: std::io::Error::other(error),
    }
}

struct Pump<S> {
    root: PathBuf,
    strategy: WatchStrategy,
    coalescer: Coalescer,
    stopped: Arc<AtomicBool>,
    /// Directories handed to the backend ([`WatchStrategy::PerDirectory`]).
    registered: HashSet<PathBuf>,
    sink: S,
    /// Degradations already delivered: each reason goes out once.
    reported: BTreeSet<Degradation>,
    /// Found by the synchronous walk in [`watch`], delivered from the pump
    /// thread so the sink only ever runs there.
    initial: BTreeSet<Degradation>,
}

impl<S: Fn(WatchEvent)> Pump<S> {
    fn run(mut self, mut backend: notify::RecommendedWatcher, rx: mpsc::Receiver<Pulse>) {
        let mut last_root_check = Instant::now();
        let mut backend_errors = 0_u64;
        for degradation in std::mem::take(&mut self.initial) {
            self.degrade(degradation);
        }
        loop {
            if self.stopped.load(Ordering::Acquire) {
                break;
            }
            let now = Instant::now();
            let timeout = self
                .coalescer
                .deadline()
                .map(|deadline| deadline.saturating_duration_since(now))
                .unwrap_or(self.coalescer.config.root_check_interval)
                .min(self.coalescer.config.root_check_interval);
            let mut drained = 0;
            match rx.recv_timeout(timeout) {
                Ok(Pulse::Stop) => break,
                Ok(Pulse::Event(result)) => {
                    self.absorb(&mut backend, result, &mut backend_errors);
                    drained += 1;
                }
                Err(RecvTimeoutError::Timeout) => {}
                // The handle and the backend are both gone.
                Err(RecvTimeoutError::Disconnected) => break,
            }
            // Take whatever else is already queued, so a storm costs one loop
            // iteration per batch of events rather than per event.
            while drained < DRAIN_LIMIT {
                match rx.try_recv() {
                    Ok(Pulse::Event(result)) => {
                        self.absorb(&mut backend, result, &mut backend_errors);
                        drained += 1;
                    }
                    Ok(Pulse::Stop) => return,
                    Err(_) => break,
                }
            }
            if self.stopped.load(Ordering::Acquire) {
                break;
            }
            let now = Instant::now();
            if now.duration_since(last_root_check) >= self.coalescer.config.root_check_interval {
                last_root_check = now;
                if !root_is_present(&self.root) {
                    tracing::info!(root = %self.root.display(), "watched root disappeared");
                    self.deliver(WatchEvent::RootMissing);
                    break;
                }
            }
            if let Some(batch) = self.coalescer.take_if_due(Instant::now()) {
                self.deliver(WatchEvent::Changed(batch));
            }
        }
        tracing::debug!(root = %self.root.display(), "workspace watch stopped");
    }

    fn deliver(&self, event: WatchEvent) {
        if self.stopped.load(Ordering::Acquire) {
            return;
        }
        (self.sink)(event);
    }

    /// Report a degradation the first time its reason occurs, and log it; every
    /// later occurrence is only counted in the log.
    fn degrade(&mut self, degradation: Degradation) {
        if self.reported.insert(degradation) {
            tracing::warn!(
                root = %self.root.display(),
                ?degradation,
                "part of the workspace is not being watched"
            );
            self.deliver(WatchEvent::Degraded(degradation));
        }
    }

    fn absorb(
        &mut self,
        backend: &mut notify::RecommendedWatcher,
        result: notify::Result<notify::Event>,
        errors: &mut u64,
    ) {
        let event = match result {
            Ok(event) => event,
            Err(error) => {
                *errors += 1;
                // Once per watch, not once per failure: a backend that starts
                // failing usually keeps failing.
                if *errors == 1 {
                    tracing::warn!(root = %self.root.display(), %error, "watch backend error");
                    self.degrade(Degradation::BackendError);
                }
                return;
            }
        };
        let now = Instant::now();
        // The backend says events were lost or cannot be trusted (inotify's
        // queue overflowed, FSEvents wants its subdirectories rescanned). The
        // paths it names, if any, are not the whole change, so the batch says
        // `overflowed` rather than presenting them as complete (ADR-025).
        if event.need_rescan() {
            self.coalescer.overflow(now);
            return;
        }
        if !is_interesting(&event.kind) {
            return;
        }
        for path in &event.paths {
            if self.strategy == WatchStrategy::PerDirectory {
                self.follow_tree(backend, &event.kind, path);
            }
            if let Some(directory) = changed_directory(&self.root, path) {
                self.coalescer.record(directory, now);
            }
        }
    }

    /// Keep a per-directory registration in step with the tree: a directory
    /// that appears is watched (with whatever it already contains), one that
    /// goes away is released.
    fn follow_tree(
        &mut self,
        backend: &mut notify::RecommendedWatcher,
        kind: &EventKind,
        path: &Path,
    ) {
        let inside_root = WorkspacePath::from_native(&self.root, path)
            .ok()
            .is_some_and(|relative| !is_ignored(&relative));
        if !inside_root {
            return;
        }
        if matches!(kind, EventKind::Remove(_)) && self.registered.remove(path) {
            // inotify drops the watch itself; this only clears the map.
            let _ = backend.unwatch(path);
            self.registered.retain(|watched| !watched.starts_with(path));
            return;
        }
        if !path.is_dir() || self.registered.contains(path) {
            return;
        }
        let limit = self.coalescer.config.max_watched_directories;
        if self.registered.len() >= limit {
            self.degrade(Degradation::DirectoryLimit);
            return;
        }
        if backend.watch(path, RecursiveMode::NonRecursive).is_err() {
            self.degrade(Degradation::RegistrationRefused);
            return;
        }
        self.registered.insert(path.to_path_buf());
        // A directory can arrive with a subtree already inside it (a move).
        let room = limit.saturating_sub(self.registered.len());
        let outcome = register_subtree(
            &mut |directory| {
                backend
                    .watch(directory, RecursiveMode::NonRecursive)
                    .is_ok()
            },
            &mut self.registered,
            path,
            room,
        );
        for degradation in outcome.degradations() {
            self.degrade(degradation);
        }
    }
}

/// What registering a subtree could not do.
#[derive(Debug, Default, PartialEq, Eq)]
struct SubtreeOutcome {
    /// Directories the backend refused.
    refused: u64,
    /// The walk stopped at its limit with directories left unregistered.
    truncated: bool,
}

impl SubtreeOutcome {
    fn degradations(&self) -> impl Iterator<Item = Degradation> + use<> {
        let refused = (self.refused > 0).then_some(Degradation::RegistrationRefused);
        let truncated = self.truncated.then_some(Degradation::DirectoryLimit);
        refused.into_iter().chain(truncated)
    }
}

/// Register every non-ignored directory under `start`, at most `limit` of them,
/// through `register` (the backend's `watch`, or a test's stand-in). Failures
/// are per-directory: the ones that succeeded keep reporting (the inotify
/// budget runs out mid-tree rather than all at once). What could not be done
/// is returned, for the caller to report.
fn register_subtree(
    register: &mut dyn FnMut(&Path) -> bool,
    registered: &mut HashSet<PathBuf>,
    start: &Path,
    limit: usize,
) -> SubtreeOutcome {
    let (directories, truncated) = watchable_directories(start, limit);
    let mut refused = 0_u64;
    for directory in directories {
        if registered.contains(&directory) {
            continue;
        }
        if register(&directory) {
            registered.insert(directory);
        } else {
            refused += 1;
        }
    }
    if refused > 0 {
        tracing::warn!(
            start = %start.display(),
            refused,
            "some directories could not be watched; those subtrees keep the pre-watch behaviour"
        );
    }
    SubtreeOutcome { refused, truncated }
}

/// Directories under `start` (excluding `start`), skipping ignored names and
/// never following a symlink, at most `limit` of them — and whether any were
/// left out. `true` only when one more directory was actually seen, so a tree
/// of exactly `limit` directories is not reported as cut short.
fn watchable_directories(start: &Path, limit: usize) -> (Vec<PathBuf>, bool) {
    let mut found = Vec::new();
    let mut queue = vec![start.to_path_buf()];
    while let Some(directory) = queue.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            // `DirEntry::file_type` does not follow symlinks, so a link to a
            // directory is not descended into — the lister does not follow one
            // either.
            if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                continue;
            }
            let name = entry.file_name();
            if name
                .to_str()
                .is_some_and(|name| crate::listing::IGNORED_DIRECTORY_NAMES.contains(&name))
            {
                continue;
            }
            if found.len() >= limit {
                return (found, true);
            }
            let child = entry.path();
            found.push(child.clone());
            queue.push(child);
        }
    }
    (found, false)
}

fn root_is_present(root: &Path) -> bool {
    std::fs::metadata(root).is_ok_and(|metadata| metadata.is_dir())
}

/// Whether an event kind can change what a directory listing shows.
///
/// Reads and access-time updates cannot, and inotify reports them freely; a
/// watch that forwarded them would report a directory as changed every time
/// something in it was opened.
fn is_interesting(kind: &EventKind) -> bool {
    match kind {
        EventKind::Access(_) => false,
        EventKind::Modify(ModifyKind::Metadata(
            notify::event::MetadataKind::AccessTime | notify::event::MetadataKind::Extended,
        )) => false,
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) => true,
        // `Any` and `Other` are what a backend says when it does not know;
        // taking them as real is the safe direction — a spurious refresh
        // costs a listing, a missed one costs correctness.
        EventKind::Any | EventKind::Other => true,
    }
}

/// The directory an event path changes, or `None` when the event is not one
/// this watch reports.
///
/// Dropped: Gritty's own atomic-save temporaries (§5 — otherwise every
/// debounced autosave reports its own directory), anything that resolves
/// outside the canonicalized root (a symlink race, a bind mount), and anything
/// *inside* an ignored subtree. An ignored directory's own creation or removal
/// is kept, because that is a change to its parent, which is not ignored.
fn changed_directory(root: &Path, native: &Path) -> Option<WorkspacePath> {
    if native
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(is_temp_file_name)
    {
        return None;
    }
    let path = WorkspacePath::from_native(root, native).ok()?;
    let directory = path.parent().unwrap_or_else(WorkspacePath::root);
    (!is_ignored(&directory)).then_some(directory)
}

/// The quiet-period/ceiling timer and the bounded set of changed directories.
/// Pure and clock-driven: every timing rule here is asserted without sleeping.
#[derive(Debug)]
struct Coalescer {
    config: WatchConfig,
    directories: BTreeSet<WorkspacePath>,
    overflowed: bool,
    /// When the current batch started accumulating (drives `max_wait`).
    opened: Option<Instant>,
    /// The last change absorbed (drives `debounce`).
    touched: Option<Instant>,
}

impl Coalescer {
    fn new(config: WatchConfig) -> Self {
        Self {
            config,
            directories: BTreeSet::new(),
            overflowed: false,
            opened: None,
            touched: None,
        }
    }

    fn record(&mut self, directory: WorkspacePath, now: Instant) {
        self.opened.get_or_insert(now);
        self.touched = Some(now);
        if self.overflowed {
            return;
        }
        self.directories.insert(directory);
        if self.directories.len() > self.config.max_directories {
            // Above the cap the list is dropped rather than truncated: a
            // partial list presented as complete is the lie rule 8 forbids.
            self.overflowed = true;
            self.directories.clear();
        }
    }

    /// The backend lost track: whatever this batch holds is not the whole
    /// change, so it overflows, exactly as a batch above the cap does.
    fn overflow(&mut self, now: Instant) {
        self.opened.get_or_insert(now);
        self.touched = Some(now);
        self.overflowed = true;
        self.directories.clear();
    }

    /// When the next batch is due, or `None` when nothing is pending.
    fn deadline(&self) -> Option<Instant> {
        let touched = self.touched?;
        let opened = self.opened.unwrap_or(touched);
        Some((touched + self.config.debounce).min(opened + self.config.max_wait))
    }

    fn take_if_due(&mut self, now: Instant) -> Option<ChangeBatch> {
        if now < self.deadline()? {
            return None;
        }
        let batch = ChangeBatch {
            directories: std::mem::take(&mut self.directories).into_iter().collect(),
            overflowed: std::mem::take(&mut self.overflowed),
        };
        self.opened = None;
        self.touched = None;
        // An overflowed batch names nothing; an empty, non-overflowed one
        // cannot happen, but it would say nothing either.
        (!batch.directories.is_empty() || batch.overflowed).then_some(batch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wire(path: &str) -> WorkspacePath {
        WorkspacePath::parse(path).expect(path)
    }

    fn config() -> WatchConfig {
        WatchConfig::for_tests(Duration::from_millis(200), Duration::from_millis(1000), 4)
    }

    #[test]
    fn nothing_is_due_before_the_quiet_period_ends() {
        let mut coalescer = Coalescer::new(config());
        let start = Instant::now();
        coalescer.record(wire("src"), start);
        assert!(
            coalescer
                .take_if_due(start + Duration::from_millis(199))
                .is_none()
        );
        let batch = coalescer
            .take_if_due(start + Duration::from_millis(200))
            .expect("due");
        assert_eq!(batch.directories, vec![wire("src")]);
        assert!(!batch.overflowed);
        // Nothing pending afterwards, at any later time.
        assert!(
            coalescer
                .take_if_due(start + Duration::from_secs(10))
                .is_none()
        );
    }

    #[test]
    fn a_change_restarts_the_quiet_period() {
        let mut coalescer = Coalescer::new(config());
        let start = Instant::now();
        coalescer.record(wire("a"), start);
        coalescer.record(wire("b"), start + Duration::from_millis(150));
        // 200 ms after the *first* change is no longer enough.
        assert!(
            coalescer
                .take_if_due(start + Duration::from_millis(200))
                .is_none()
        );
        let batch = coalescer
            .take_if_due(start + Duration::from_millis(350))
            .expect("due");
        assert_eq!(batch.directories, vec![wire("a"), wire("b")]);
    }

    #[test]
    fn a_continuous_flood_still_flushes_at_max_wait() {
        let mut coalescer = Coalescer::new(config());
        let start = Instant::now();
        // A change every 50 ms: the quiet period never elapses on its own.
        let mut now = start;
        for step in 0..40 {
            now = start + Duration::from_millis(50 * step);
            coalescer.record(wire("busy"), now);
            if now >= start + Duration::from_millis(1000) {
                break;
            }
            assert!(coalescer.take_if_due(now).is_none(), "flushed at {step}");
        }
        assert!(now >= start + Duration::from_millis(1000));
        let batch = coalescer.take_if_due(now).expect("the ceiling flushes");
        assert_eq!(batch.directories, vec![wire("busy")]);
    }

    #[test]
    fn directories_are_deduplicated_and_sorted() {
        let mut coalescer = Coalescer::new(config());
        let start = Instant::now();
        for path in ["src/z", "a", "src/z", "a", "m"] {
            coalescer.record(wire(path), start);
        }
        let batch = coalescer
            .take_if_due(start + Duration::from_millis(200))
            .expect("due");
        assert_eq!(batch.directories, vec![wire("a"), wire("m"), wire("src/z")]);
    }

    #[test]
    fn above_the_cap_the_batch_overflows_with_an_empty_list() {
        let mut coalescer = Coalescer::new(config());
        let start = Instant::now();
        for n in 0..10 {
            coalescer.record(wire(&format!("d{n}")), start);
        }
        let batch = coalescer
            .take_if_due(start + Duration::from_millis(200))
            .expect("due");
        assert!(batch.overflowed);
        assert!(
            batch.directories.is_empty(),
            "a partial list must never be presented as complete: {:?}",
            batch.directories
        );
        // The next batch starts clean.
        coalescer.record(wire("after"), start + Duration::from_secs(5));
        let next = coalescer
            .take_if_due(start + Duration::from_secs(6))
            .expect("due");
        assert!(!next.overflowed);
        assert_eq!(next.directories, vec![wire("after")]);
    }

    #[test]
    fn the_directory_reported_is_the_parent_of_the_changed_entry() {
        let root = Path::new("/ws");
        assert_eq!(
            changed_directory(root, Path::new("/ws/src/main.rs")),
            Some(wire("src"))
        );
        assert_eq!(
            changed_directory(root, Path::new("/ws/README.md")),
            Some(wire(""))
        );
        // The root itself changing is a change to the root.
        assert_eq!(changed_directory(root, Path::new("/ws")), Some(wire("")));
    }

    #[test]
    fn gritty_s_own_atomic_write_temporaries_are_dropped() {
        let root = Path::new("/ws");
        let temp = format!(".notes.md.gritty-{}-7.tmp", std::process::id());
        assert!(is_temp_file_name(&temp));
        assert_eq!(
            changed_directory(root, &Path::new("/ws/docs").join(&temp)),
            None
        );
        // The rename's destination is still reported: that is the real change.
        assert_eq!(
            changed_directory(root, Path::new("/ws/docs/notes.md")),
            Some(wire("docs"))
        );
    }

    #[test]
    fn events_inside_an_ignored_subtree_are_dropped_but_its_own_entry_is_not() {
        let root = Path::new("/ws");
        assert_eq!(
            changed_directory(root, Path::new("/ws/node_modules/react/index.js")),
            None
        );
        assert_eq!(
            changed_directory(root, Path::new("/ws/a/b/target/debug/x")),
            None
        );
        // Creating or removing the ignored directory changes its parent.
        assert_eq!(
            changed_directory(root, Path::new("/ws/node_modules")),
            Some(wire(""))
        );
        assert_eq!(
            changed_directory(root, Path::new("/ws/crates/x/target")),
            Some(wire("crates/x"))
        );
    }

    #[test]
    fn an_event_resolving_outside_the_root_is_rejected() {
        let root = Path::new("/ws");
        for outside in ["/etc/passwd", "/ws2/src/main.rs", "/"] {
            assert_eq!(
                changed_directory(root, Path::new(outside)),
                None,
                "{outside}"
            );
        }
    }

    #[test]
    fn reads_are_not_changes() {
        use notify::event::{AccessKind, CreateKind, MetadataKind, RemoveKind};

        assert!(!is_interesting(&EventKind::Access(AccessKind::Read)));
        assert!(!is_interesting(&EventKind::Modify(ModifyKind::Metadata(
            MetadataKind::AccessTime
        ))));
        assert!(is_interesting(&EventKind::Create(CreateKind::File)));
        assert!(is_interesting(&EventKind::Remove(RemoveKind::Folder)));
        assert!(is_interesting(&EventKind::Modify(ModifyKind::Data(
            notify::event::DataChange::Content
        ))));
        assert!(is_interesting(&EventKind::Any));
    }

    /// A scratch tree for the registration walk; the pure tests above need
    /// no disk.
    struct Scratch(PathBuf);

    impl Scratch {
        fn with_directories(label: &str, directories: &[&str]) -> Self {
            use std::sync::atomic::AtomicUsize;
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let n = COUNTER.fetch_add(1, Ordering::SeqCst);
            let path = std::env::temp_dir().join(format!(
                "gritty-watch-unit-{label}-{}-{n}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            for directory in directories {
                std::fs::create_dir_all(path.join(directory)).expect("scratch directory");
            }
            std::fs::create_dir_all(&path).expect("scratch root");
            Self(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_refused_registration_is_counted_not_fatal() {
        let tree = Scratch::with_directories("refused", &["ok", "refused", "ok/deeper"]);
        let mut registered = HashSet::new();
        let outcome = register_subtree(
            &mut |directory| !directory.ends_with("refused"),
            &mut registered,
            &tree.0,
            100,
        );
        assert_eq!(
            outcome,
            SubtreeOutcome {
                refused: 1,
                truncated: false
            }
        );
        // The others still registered: the budget runs out per directory.
        assert_eq!(registered.len(), 2, "{registered:?}");
        assert_eq!(
            outcome.degradations().collect::<Vec<_>>(),
            vec![Degradation::RegistrationRefused]
        );
    }

    #[test]
    fn the_walk_says_when_it_stopped_at_its_limit() {
        let tree = Scratch::with_directories("limit", &["a", "b", "c"]);
        let (found, truncated) = watchable_directories(&tree.0, 2);
        assert_eq!(found.len(), 2);
        assert!(truncated, "a third directory was left out");

        // Exactly at the limit is not cut short.
        let (found, truncated) = watchable_directories(&tree.0, 3);
        assert_eq!(found.len(), 3);
        assert!(!truncated);

        let mut registered = HashSet::new();
        let outcome = register_subtree(&mut |_| true, &mut registered, &tree.0, 2);
        assert_eq!(
            outcome.degradations().collect::<Vec<_>>(),
            vec![Degradation::DirectoryLimit]
        );
    }

    /// Everything a test pump delivered, in order.
    type Delivered = Arc<std::sync::Mutex<Vec<WatchEvent>>>;

    fn recording_pump() -> (Pump<impl Fn(WatchEvent)>, Delivered) {
        let delivered = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = {
            let delivered = Arc::clone(&delivered);
            move |event| delivered.lock().expect("lock").push(event)
        };
        let pump = Pump {
            root: PathBuf::from("/ws"),
            strategy: WatchStrategy::PerDirectory,
            coalescer: Coalescer::new(config()),
            stopped: Arc::new(AtomicBool::new(false)),
            registered: HashSet::new(),
            sink,
            reported: BTreeSet::new(),
            initial: BTreeSet::new(),
        };
        (pump, delivered)
    }

    #[test]
    fn a_degradation_is_delivered_once_per_reason() {
        let (mut pump, delivered) = recording_pump();
        for _ in 0..100 {
            pump.degrade(Degradation::DirectoryLimit);
            pump.degrade(Degradation::RegistrationRefused);
        }
        pump.degrade(Degradation::DirectoryLimit);
        assert_eq!(
            *delivered.lock().expect("lock"),
            vec![
                WatchEvent::Degraded(Degradation::DirectoryLimit),
                WatchEvent::Degraded(Degradation::RegistrationRefused),
            ],
            "a failing backend reports each reason once, not once per failure"
        );
    }

    #[test]
    fn a_rescan_event_overflows_the_batch() {
        use notify::event::{CreateKind, Flag};

        let (mut pump, _delivered) = recording_pump();
        let mut backend = notify::recommended_watcher(|_| {}).expect("a backend");
        let start = Instant::now();
        let mut errors = 0;
        // An ordinary change first, so there is a partial list to distrust.
        pump.absorb(
            &mut backend,
            Ok(notify::Event::new(EventKind::Create(CreateKind::File))
                .add_path(PathBuf::from("/ws/src/a.rs"))),
            &mut errors,
        );
        pump.absorb(
            &mut backend,
            Ok(notify::Event::new(EventKind::Other)
                .add_path(PathBuf::from("/ws/src"))
                .set_flag(Flag::Rescan)),
            &mut errors,
        );
        let batch = pump
            .coalescer
            .take_if_due(start + Duration::from_secs(5))
            .expect("due");
        assert!(batch.overflowed, "a rescan is not a complete list");
        assert!(batch.directories.is_empty(), "{:?}", batch.directories);
    }

    #[test]
    fn the_platform_strategy_is_the_one_the_platform_can_afford() {
        let strategy = WatchStrategy::for_platform();
        if cfg!(target_os = "linux") {
            assert_eq!(strategy, WatchStrategy::PerDirectory);
        } else {
            assert_eq!(strategy, WatchStrategy::Recursive);
        }
    }
}
