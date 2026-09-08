//! Agent session discovery (T220): what else on this machine is working.
//!
//! Gritty is an orchestration cockpit long before it orchestrates anything, and
//! the first question in `AGENTS.md` — *what is happening* — has an answer today
//! for the agents the user already runs in their own terminals. This module
//! polls for them, translates the evidence into domain state, and publishes a
//! roster.
//!
//! It reads, and does exactly one other thing. There is no command here that
//! starts a session, stops one, types into one or cancels a turn, because
//! Gritty does not own these sessions: another terminal does. When T300 brings
//! runs Gritty *does* own, they get their own type and their own lifecycle;
//! this one keeps meaning "someone else's work, visible".
//!
//! The exception is [`AgentSessionManager::answer_permission`] (T335, ADR-020),
//! and it is narrower than it looks. Answering a question the agent asked is
//! not driving the agent — the prompt exists whether or not Gritty is running,
//! and if Gritty says nothing the user answers it in the terminal instead.
//! What travels back is not a derivation this module made; it is a decision a
//! person took, in a UI that named the verb, the target and the blast radius.
//! Everything about the wait is built to fail open, and [`permission`] is where
//! that lives.
//!
//! **Where the boundary is.** `gritty-agents` reads `ps`, `lsof` and the
//! agents' own transcripts and reports what it saw. Nothing it returns crosses
//! the protocol: this module decides what an observation *means* in Gritty's
//! terms — the activity values, the ranking against the open workspace, the
//! display path — which is the adapter/domain split ADR-005 asks for.
//!
//! **Why polling, and why it is cheap.** There is nothing to subscribe to: a
//! process table has no change feed. The cost is bounded instead — one `ps` per
//! tick, one `lsof` per agent process, and a fixed tail of each transcript —
//! and the scan runs on a blocking thread so neither a command handler nor the
//! renderer ever waits on it (rules 8 and 10). An unchanged roster emits
//! nothing at all.

pub mod attention;
pub mod permission;
pub mod timeline;
pub mod wire;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use gritty_agents::hook::{HookPhase, HookScanOptions, HookState};
use gritty_agents::integration::{self, HookTarget, IntegrationError};
use gritty_workspace::WorkspacePath;

use gritty_agents::{
    CommandRunner, DiscoveredSession, Discovery, ScanOptions, SystemCommandRunner,
};
use tokio::runtime::Handle;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::event::{CoreEvent, EventBus, EventOrigin};
use crate::protocol::actor::Actor;
use crate::protocol::error::{CoreError, CoreErrorCode};
use crate::protocol::time::Timestamp;
use crate::workspace::WorkspaceManager;
use crate::workspace::recents::{display_name, shorten};
use attention::{AttentionNotifier, AttentionTracker};
use timeline::OpenTimeline;

pub use attention::{
    AgentNotificationPermission, AgentNotificationSettings, AttentionAlert, MAX_NOTIFICATION_BODY,
    NOTIFY_CAP_PER_WINDOW, NOTIFY_COOLDOWN, NOTIFY_WINDOW, SetAgentNotificationsRequest,
};
pub use timeline::MAX_TIMELINE_ENTRIES;
pub use wire::{
    AgentActivity, AgentActivitySource, AgentEvidence, AgentIntegration, AgentIntegrationChange,
    AgentIntegrationChangeRequest, AgentIntegrationEvent, AgentIntegrationRequest,
    AgentIntegrationStatus, AgentNoticeLevel, AgentPermissionAnswer, AgentPermissionGrant,
    AgentPermissionGrants, AgentPermissionRequest, AgentPermissionVerb, AgentPermissionVerdict,
    AgentProvider, AgentRoster, AgentSession, AgentTimeline, AgentTimelineAppendedPayload,
    AgentTimelineEntry, AgentTimelineKind, AgentTimelineRequest, AgentToolCall,
    AnswerPermissionRequest, ListAgentSessionsRequest, RevokeGrantRequest,
};

/// How often the roster is rescanned.
///
/// Fast enough that a tool call starting or a session ending is noticed while
/// the user is still looking, slow enough that the probe's cost stays a small
/// fraction of a core. A scan of three live sessions measures ~250 ms, most of
/// it `lsof`.
pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(6);

/// A live session whose transcript has been quiet for longer than this reads as
/// `IDLE` rather than `WAITING`, so a terminal left open overnight stops
/// claiming the user's attention.
pub const DEFAULT_IDLE_AFTER: Duration = Duration::from_secs(5 * 60);

/// How often an **open** timeline is followed.
///
/// Two loops, deliberately. Discovery stays at [`DEFAULT_POLL_INTERVAL`]
/// because `ps` and `lsof` are its cost; this one is affordable at three times
/// a second precisely because it seeks to a byte offset, reads only what was
/// appended, and runs only for what is open. A tick with nothing open does no
/// I/O at all.
pub const DEFAULT_TIMELINE_INTERVAL: Duration = Duration::from_millis(400);

/// Timelines that may be open at once.
///
/// The rail follows the session the user selected, so one is the normal number
/// and eight is generous. The bound exists for the same reason the terminal
/// bounds sessions: a loop in the renderer must not be able to turn "open a
/// timeline" into unbounded file following.
pub const MAX_OPEN_TIMELINES: usize = 8;

/// What the fast loop has actually done. Exposed so "closing a timeline stops
/// its follower" is something a test asserts rather than something this module
/// claims (rule 12).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FollowMetrics {
    /// Fast-loop ticks completed, whether or not they read anything.
    pub ticks: u64,
    /// Transcripts read. This is the counter that stops moving when the last
    /// timeline is closed.
    pub reads: u64,
    /// Batches emitted. Never more than one per open timeline per tick, and
    /// none at all while nothing changed.
    pub batches: u64,
}

/// Where a session's timeline is read from. Kept beside the roster because
/// neither the transcript path nor the working directory may cross the
/// protocol (ADR-010), yet both are needed to follow and to resolve.
#[derive(Debug, Clone, Default)]
struct TimelineSource {
    transcript: Option<PathBuf>,
    cwd: Option<PathBuf>,
}

/// Reads the set of sessions Gritty already owns, once per scan.
///
/// See [`AgentDiscoveryConfig::owned`] for why this is a function rather than a
/// handle on the run manager.
pub type OwnedSessionsFn = Arc<dyn Fn() -> OwnedSessions + Send + Sync>;

/// Re-exported so the composition root can build one without taking its own
/// dependency on `gritty-agents`. The wiring names as little of an adapter as
/// it can get away with.
pub use gritty_agents::OwnedSessions;

/// What the composition root injects. Tests replace the runner and the root so
/// no test spawns a process or reads a real transcript.
pub struct AgentDiscoveryConfig {
    /// Where the transcripts are, or `None` to run on process evidence alone.
    pub scan: Option<ScanOptions>,
    pub runner: Arc<dyn CommandRunner>,
    pub poll_interval: Duration,
    /// How often open timelines are followed (T230).
    pub timeline_interval: Duration,
    pub idle_after: Duration,
    /// Used to shorten displayed paths; `None` shows them in full. Also where
    /// the hook installer looks for a provider's configuration file.
    pub home: Option<std::path::PathBuf>,
    /// Where the hook writes its state files (T221), or `None` to run on the
    /// two sources T220 shipped. Read on the fast loop, not the discovery tick:
    /// "pending validation" arriving six seconds after the agent stopped
    /// defeats the value.
    pub hooks: Option<HookScanOptions>,
    /// This build's own `gritty-agent-hook`, which is what an install writes
    /// into the provider's configuration. `None` when it could not be found,
    /// and then nothing is offered rather than an entry that would be broken
    /// the moment it was written.
    pub hook_binary: Option<PathBuf>,
    /// How a notification reaches the platform (T223), or `None` for a build
    /// that cannot raise one. `None` makes the setting *absent* rather than
    /// broken: nothing is offered that would do nothing.
    pub notifier: Option<Arc<dyn AttentionNotifier>>,
    /// Where a verdict is written back to a waiting hook, and where the
    /// liveness marker that makes waiting safe is touched (T335, ADR-020).
    /// `None` alongside `hooks: None`: a build that cannot read an ask has
    /// nothing to answer.
    pub permissions: Option<PermissionPaths>,
    /// Sessions another part of Gritty started and already reports itself, or
    /// `None` to exclude nothing (T310-B §8).
    ///
    /// **A closure, not a handle**, and the distinction is the point.
    /// `agent_session` and `agent_run` never import each other
    /// (`docs/architecture/agent-runs.md`, "The line, first"), so the thing
    /// that knows both is the composition root; what it injects here is an
    /// *answer*, not an object to ask. `ownership` below is a handle because a
    /// registry is shared ground both lanes legitimately use — a run manager is
    /// the other lane itself.
    ///
    /// Called on **every** scan and never captured: a run started since the
    /// last tick has to be excluded from this one.
    pub owned: Option<OwnedSessionsFn>,
    /// The registry the files an agent touched are cross-checked against
    /// (T326), or `None` for a build with no session database — and then no
    /// violation is found rather than a half-checked one being reported.
    ///
    /// The rail hands over what a tick already produced; the ownership pass
    /// decides everything else. This direction is deliberate: the registry does
    /// not subscribe to `agents.timeline`, which is a read for a human and not
    /// a feed for a checker (T326 §1).
    pub ownership: Option<Arc<crate::ownership::OwnershipManager>>,
}

/// The two paths ADR-020 adds, beside the state directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionPaths {
    pub verdicts: PathBuf,
    pub marker: PathBuf,
}

impl AgentDiscoveryConfig {
    /// The production configuration.
    ///
    /// Reuses the terminal crate's home resolver — `$HOME`, then the account's
    /// own home from the password database — because it is the one place that
    /// already answers this correctly for an app launched without an
    /// environment (the same call `session` makes for recent paths).
    pub fn for_current_home() -> Self {
        let home = match gritty_terminal::shell::home_directory(|key| std::env::var(key).ok()) {
            Ok(home) => Some(home),
            Err(error) => {
                tracing::debug!(%error, "no home directory; agent transcripts are not scanned");
                None
            }
        };
        Self::for_home(home)
    }

    /// The same, for a home the caller resolved. `None` costs the display
    /// shortening and the transcript root, but not the process evidence.
    pub fn for_home(home: Option<std::path::PathBuf>) -> Self {
        Self {
            scan: home
                .as_ref()
                .map(|dir| ScanOptions::rooted_at(dir.join(".claude/projects"))),
            runner: Arc::new(SystemCommandRunner),
            poll_interval: DEFAULT_POLL_INTERVAL,
            timeline_interval: DEFAULT_TIMELINE_INTERVAL,
            idle_after: DEFAULT_IDLE_AFTER,
            home,
            hooks: None,
            hook_binary: None,
            notifier: None,
            permissions: None,
            ownership: None,
            owned: None,
        }
    }

    /// Point the hook reader at Gritty's application-data directory (T221).
    ///
    /// Separate from [`Self::for_home`] because the data directory is the
    /// composition root's answer, not the home resolver's: `GRITTY_DATA_DIR`
    /// overrides it, and a build with no data directory runs without
    /// persistence *and* without hook evidence rather than inventing a path.
    pub fn with_data_dir(mut self, data_dir: Option<&Path>) -> Self {
        self.hooks = data_dir.map(|dir| {
            HookScanOptions::rooted_at(gritty_agents::hook::state_dir_from_data_dir(dir))
        });
        self.permissions = data_dir.map(|dir| PermissionPaths {
            verdicts: gritty_agents::verdict::verdict_dir_from_data_dir(dir),
            marker: gritty_agents::verdict::marker_from_data_dir(dir),
        });
        self
    }

    /// Where this build's hook binary is, when it could be found.
    pub fn with_hook_binary(mut self, hook_binary: Option<PathBuf>) -> Self {
        self.hook_binary = hook_binary;
        self
    }

    /// How attention notifications reach the platform (T223). Left unset, the
    /// core still tracks the transitions and simply raises nothing.
    pub fn with_notifier(mut self, notifier: Option<Arc<dyn AttentionNotifier>>) -> Self {
        self.notifier = notifier;
        self
    }

    /// Which registry a touched file is checked against (T326). Left unset, the
    /// rail behaves exactly as it did before: the check is the only thing that
    /// does not happen.
    /// Exclude the sessions another part of Gritty already reports (T310-B §8).
    pub fn with_owned(mut self, owned: Option<OwnedSessionsFn>) -> Self {
        self.owned = owned;
        self
    }

    pub fn with_ownership(
        mut self,
        ownership: Option<Arc<crate::ownership::OwnershipManager>>,
    ) -> Self {
        self.ownership = ownership;
        self
    }
}

/// Owns the roster and the loop that refreshes it.
pub struct AgentSessionManager {
    events: Arc<EventBus>,
    workspaces: Arc<WorkspaceManager>,
    config: AgentDiscoveryConfig,
    /// `None` until the first scan completes, which is what lets `list` scan on
    /// demand instead of reporting a confident empty roster it never checked.
    roster: Mutex<Option<AgentRoster>>,
    /// Held for the length of a scan so only one runs at a time.
    ///
    /// `start` fires its first tick immediately, so without this a `list`
    /// arriving in the same instant runs a *second* `ps`/`lsof` pair against the
    /// same moment, and the two publish two rosters for one change. Async rather
    /// than `std`, because a scan is awaited across `spawn_blocking`.
    scan_gate: tokio::sync::Mutex<()>,
    /// The raw evidence the last discovery tick found, kept so the fast loop can
    /// re-derive the roster against fresh hook state without paying for `ps` and
    /// `lsof` again (T221).
    discovery: Mutex<Option<Discovery>>,
    /// The agent's own reports, by *adapter* session id — the exact join key,
    /// unlike the roster's working-directory fallback.
    hooks: Mutex<HashMap<String, HookState>>,
    /// The permission prompt each session is stopped at, by domain session id.
    /// Derived beside the roster, because that is where the hook state and the
    /// transcript's tool preview are both in hand.
    permissions: Mutex<HashMap<String, AgentTimelineEntry>>,
    /// What is being asked, what was granted, and what was just answered
    /// (T335). Separate from `permissions` above, which is the *timeline* row
    /// for the same prompt: that one is a view, this one is the state a verdict
    /// is written against.
    asks: permission::Permissions,
    /// Every open timeline, by session id.
    timelines: Mutex<HashMap<String, OpenTimeline>>,
    /// The transcript and working directory behind each roster row, refreshed
    /// by every scan.
    sources: Mutex<HashMap<String, TimelineSource>>,
    metrics: Mutex<FollowMetrics>,
    /// The edge-triggering, throttle and cap behind attention notifications
    /// (T223). Advanced from [`Self::publish`], which is the one place a roster
    /// change is observed, so the two loops cannot disagree about what a
    /// transition is.
    attention: Mutex<AttentionTracker>,
    /// Off until the user turns it on, every run (T223 §2).
    notifications_enabled: AtomicBool,
    /// Whether the Gritty window has focus. **Starts true**: focus that is
    /// unknown counts as focused, and a missed notification is better than one
    /// over the user's face.
    window_focused: AtomicBool,
    /// Both loops, so shutdown can wait for them rather than assume.
    tasks: Mutex<Vec<JoinHandle<()>>>,
    cancel: CancellationToken,
}

impl AgentSessionManager {
    /// Builds the manager and starts both loops. Neither fires immediately:
    /// the roster is populated by the first `list`, which scans on demand when
    /// nothing is cached, so an immediate tick would only duplicate it.
    pub fn start(
        handle: Handle,
        events: Arc<EventBus>,
        workspaces: Arc<WorkspaceManager>,
        config: AgentDiscoveryConfig,
    ) -> Arc<Self> {
        let manager = Arc::new(Self {
            events,
            workspaces,
            config,
            roster: Mutex::new(None),
            scan_gate: tokio::sync::Mutex::new(()),
            discovery: Mutex::new(None),
            hooks: Mutex::new(HashMap::new()),
            permissions: Mutex::new(HashMap::new()),
            asks: permission::Permissions::default(),
            timelines: Mutex::new(HashMap::new()),
            sources: Mutex::new(HashMap::new()),
            metrics: Mutex::new(FollowMetrics::default()),
            attention: Mutex::new(AttentionTracker::new()),
            notifications_enabled: AtomicBool::new(false),
            window_focused: AtomicBool::new(true),
            tasks: Mutex::new(Vec::new()),
            cancel: CancellationToken::new(),
        });
        let poller = Arc::clone(&manager);
        let follower = Arc::clone(&manager);
        let tasks = vec![
            handle.spawn(async move { poller.poll().await }),
            handle.spawn(async move { follower.follow().await }),
        ];
        *manager.tasks.lock().unwrap_or_else(PoisonError::into_inner) = tasks;
        manager
    }

    /// `agents.list`. Returns the last roster, scanning once if none exists yet.
    pub async fn list(&self) -> AgentRoster {
        if let Some(roster) = self.cached() {
            return roster;
        }
        // Re-check under the gate: the scan this was about to duplicate may
        // have been the startup tick, and by the time we hold the gate its
        // roster is already cached.
        let _gate = self.scan_gate.lock().await;
        if let Some(roster) = self.cached() {
            return roster;
        }
        self.scan_and_publish_locked().await
    }

    /// `agents.refresh`. Always rescans, but never concurrently with another
    /// scan: two at once cost twice the subprocesses and race each other's
    /// roster into the event bus.
    pub async fn refresh(&self) -> AgentRoster {
        let _gate = self.scan_gate.lock().await;
        self.scan_and_publish_locked().await
    }

    /// Stops both loops. Called from the composition root's shutdown.
    ///
    /// Cancellation is cooperative: each loop is waiting on the token or on a
    /// tick, and leaves at the next one. Nothing outlives the token.
    pub fn shutdown(&self) {
        self.cancel.cancel();
        self.timelines_lock().clear();
    }

    /// [`Self::shutdown`], then wait for both loops to actually finish.
    ///
    /// Async, so a test can assert the follower is gone rather than trust that
    /// it is; the composition root's exit handler is synchronous and calls
    /// [`Self::shutdown`].
    pub async fn shutdown_and_join(&self) {
        self.shutdown();
        let tasks = std::mem::take(&mut *self.tasks.lock().unwrap_or_else(PoisonError::into_inner));
        for task in tasks {
            let _ = task.await;
        }
    }

    /// What the fast loop has done so far.
    pub fn follow_metrics(&self) -> FollowMetrics {
        *self.metrics.lock().unwrap_or_else(PoisonError::into_inner)
    }

    // -----------------------------------------------------------------------
    // Timeline (T230)
    //
    // The roster answers *which* agents are running; these answer what one of
    // them is doing. A timeline is opened by session id, followed on the fast
    // loop while it stays open, and freed when it closes. Nothing here writes
    // to a session, and nothing here is persisted: the timeline is rediscovered
    // from the transcript, so Gritty never claims on startup that it knows what
    // a session did while the app was closed.
    // -----------------------------------------------------------------------

    /// `agents.openTimeline`. Starts following the session's transcript and
    /// returns what is already there.
    ///
    /// Idempotent: opening a timeline that is already open returns its current
    /// contents and does **not** start a second follower, because two surfaces
    /// over one controller (T210's rule) must not double the I/O.
    pub async fn open_timeline(&self, session_id: &str) -> Result<AgentTimeline, CoreError> {
        self.require_session(session_id).await?;
        if let Some(open) = self.snapshot_of(session_id) {
            return Ok(open);
        }

        {
            let open = self.timelines_lock();
            if open.len() >= MAX_OPEN_TIMELINES {
                return Err(CoreError::conflict(format!(
                    "at most {MAX_OPEN_TIMELINES} agent timelines can be open at once"
                )));
            }
        }

        let mut timeline = self.read_timeline(session_id).await;
        // The caller has these entries in the response; the first batch must
        // not repeat them.
        timeline.forget_pending();
        let snapshot = timeline.snapshot(session_id);

        let mut open = self.timelines_lock();
        // Another call may have opened it while the read was in flight; its
        // follower is as good as this one, so the loser is discarded.
        match open.entry(session_id.to_owned()) {
            std::collections::hash_map::Entry::Occupied(existing) => {
                Ok(existing.get().snapshot(session_id))
            }
            std::collections::hash_map::Entry::Vacant(slot) => {
                tracing::debug!(
                    session = %session_id,
                    entries = snapshot.entries.len(),
                    "agent timeline opened"
                );
                slot.insert(timeline);
                Ok(snapshot)
            }
        }
    }

    /// `agents.closeTimeline`. Stops following it and frees the ring.
    ///
    /// Idempotent: closing a timeline that is not open is not an error, because
    /// a renderer that reloaded has no way to know which it left open. A
    /// session that was never in the roster is still `NotFound`.
    pub async fn close_timeline(&self, session_id: &str) -> Result<(), CoreError> {
        if self.timelines_lock().remove(session_id).is_some() {
            tracing::debug!(session = %session_id, "agent timeline closed");
            return Ok(());
        }
        self.require_session(session_id).await
    }

    /// `agents.timeline`. Reads the timeline without opening one, which is how
    /// a renderer resynchronises after a reload or a missed event — the T010
    /// convention of subscribing and then calling the domain's own read.
    ///
    /// A session nobody is following is still read, once and boundedly: this is
    /// a read command, and answering "nothing" for a session with a perfectly
    /// good transcript would be the kind of confident wrong answer the roster
    /// refuses elsewhere. No follower is started and no state is kept.
    pub async fn timeline(&self, session_id: &str) -> Result<AgentTimeline, CoreError> {
        self.require_session(session_id).await?;
        if let Some(open) = self.snapshot_of(session_id) {
            return Ok(open);
        }
        Ok(self.read_timeline(session_id).await.snapshot(session_id))
    }

    /// A session absent from the roster has no timeline. The roster is a
    /// snapshot, so a row can vanish between a click and this call.
    async fn require_session(&self, session_id: &str) -> Result<(), CoreError> {
        if self
            .list()
            .await
            .sessions
            .iter()
            .any(|s| s.id == session_id)
        {
            Ok(())
        } else {
            Err(CoreError::not_found(format!(
                "agent session {session_id} is not in the roster"
            )))
        }
    }

    /// A fresh ring filled from a bounded cold read of the session's
    /// transcript. A 400 MB transcript costs what a 4 KB one does.
    async fn read_timeline(&self, session_id: &str) -> OpenTimeline {
        let source = self.source_of(session_id);
        let root = self.workspaces.current_open().map(|(_, root)| root);
        let mut timeline = OpenTimeline::new(source.transcript.clone(), source.cwd, root);
        let Some(path) = source.transcript else {
            // Process evidence only: there is nothing to read, and
            // `transcriptEvidence: false` on the snapshot is what says so
            // rather than a notice Gritty invented.
            return timeline;
        };
        // Blocking work on a blocking thread: no command handler waits on a
        // file read (rules 8 and 10).
        let followed = tokio::task::spawn_blocking(move || gritty_agents::follow(&path, 0))
            .await
            .unwrap_or_else(|error| {
                tracing::warn!(%error, "agent timeline read failed");
                gritty_agents::Followed::default()
            });
        timeline.absorb(followed, self.config.home.as_deref());
        timeline.set_permission(self.permission_of(session_id));
        timeline
    }

    /// The permission prompt this session is stopped at, if any (T221).
    fn permission_of(&self, session_id: &str) -> Option<AgentTimelineEntry> {
        self.permissions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(session_id)
            .cloned()
    }

    fn snapshot_of(&self, session_id: &str) -> Option<AgentTimeline> {
        self.timelines_lock()
            .get(session_id)
            .map(|timeline| timeline.snapshot(session_id))
    }

    fn source_of(&self, session_id: &str) -> TimelineSource {
        self.sources
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(session_id)
            .cloned()
            .unwrap_or_default()
    }

    fn timelines_lock(&self) -> MutexGuard<'_, HashMap<String, OpenTimeline>> {
        self.timelines
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// The fast loop. Follows only what is open, so a tick with nothing open
    /// costs a wakeup and nothing else.
    async fn follow(self: Arc<Self>) {
        // Unlike the discovery loop, this one does *not* fire immediately:
        // nothing is open at startup, so the first tick would be a wakeup with
        // no work in it.
        let mut ticker = tokio::time::interval_at(
            tokio::time::Instant::now() + self.config.timeline_interval,
            self.config.timeline_interval,
        );
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                () = self.cancel.cancelled() => break,
                _ = ticker.tick() => {}
            }
            self.follow_once().await;
        }
        tracing::debug!("agent timeline following stopped");
    }

    /// One tick of the fast loop: refresh hook state, read what was appended to
    /// every open timeline, fold it, and emit at most one batch per session.
    ///
    /// The hook scan rides here rather than on the six-second discovery tick
    /// because a permission prompt reported six seconds late is not worth
    /// reporting (T221 §8). It is affordable at this rate for the same reason
    /// the follower is: a directory listing plus a stat and a few hundred bytes
    /// per file, bounded exactly as transcripts are — and, with nothing
    /// installed, one failed `read_dir` and nothing else.
    async fn follow_once(&self) {
        // First, and unconditionally: a hook that is *already waiting* checks
        // this on every poll, so a tick that scanned before it refreshed would
        // read as a dead Gritty for the width of the scan (T335, ADR-020).
        self.touch_marker();
        self.follow_hooks().await;
        let root = self.workspaces.current_open().map(|(_, root)| root);
        let jobs: Vec<(String, PathBuf, u64)> = {
            let mut open = self.timelines_lock();
            // A workspace change invalidates every path already resolved into
            // the ring, which is exactly why resolution happens when a record
            // is read. Re-pointing an open timeline at a different root would
            // mix two answers; closing it is the honest move.
            open.retain(|session_id, timeline| {
                let same = match (timeline.root.as_deref(), root.as_deref()) {
                    (Some(held), Some(current)) => held.path() == current.path(),
                    (None, None) => true,
                    _ => false,
                };
                if !same {
                    tracing::debug!(
                        session = %session_id,
                        "workspace changed; agent timeline closed"
                    );
                }
                same
            });
            open.iter()
                .filter_map(|(session_id, timeline)| {
                    timeline
                        .transcript
                        .clone()
                        .map(|path| (session_id.clone(), path, timeline.offset))
                })
                .collect()
        };

        let reads = jobs.len();
        let started = Instant::now();
        let followed = if jobs.is_empty() {
            Vec::new()
        } else {
            tokio::task::spawn_blocking(move || {
                jobs.into_iter()
                    .map(|(session_id, path, offset)| {
                        (session_id, gritty_agents::follow(&path, offset))
                    })
                    .collect::<Vec<_>>()
            })
            .await
            .unwrap_or_else(|error| {
                // A panicked read costs this tick, not the loop.
                tracing::warn!(%error, "agent timeline follow failed");
                Vec::new()
            })
        };

        let home = self.config.home.as_deref();
        let permissions = self
            .permissions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let batches: Vec<AgentTimelineAppendedPayload> = {
            let mut open = self.timelines_lock();
            for (session_id, followed) in followed {
                // Closed while the read was in flight: its ring is gone and
                // there is nobody to deliver to.
                if let Some(timeline) = open.get_mut(&session_id) {
                    timeline.absorb(followed, home);
                }
            }
            // Every open timeline, not only the ones that were read: entering
            // or leaving a permission prompt is a change worth a batch even when
            // the transcript stood still. `take_batch` still answers `None` when
            // nothing at all changed, which is what keeps a 400 ms loop from
            // being a 400 ms event storm (rule 8).
            open.iter_mut()
                .filter_map(|(session_id, timeline)| {
                    timeline.set_permission(permissions.get(session_id).cloned());
                    timeline.take_batch(session_id)
                })
                .collect()
        };
        self.record_tick(reads, batches.len(), started.elapsed());

        // What this tick says the agents wrote, taken before the batches are
        // consumed by the emit below (T326). Collected here and checked after,
        // so the rail is never waiting on a database.
        let touched: Vec<(String, Vec<WorkspacePath>)> = batches
            .iter()
            .filter_map(|batch| {
                let paths = touched_paths(&batch.entries);
                (!paths.is_empty()).then(|| (batch.session_id.clone(), paths))
            })
            .collect();

        // Emitted outside the lock: a sink may re-enter this manager.
        for batch in batches {
            if batch.dropped > 0 || batch.restarted {
                tracing::debug!(
                    session = %batch.session_id,
                    dropped = batch.dropped,
                    restarted = batch.restarted,
                    "agent timeline lost entries"
                );
            }
            self.events.emit(
                &EventOrigin {
                    // Machine-wide, like the roster: a session in another
                    // checkout is not this workspace's event.
                    workspace_id: None,
                    correlation_id: None,
                    actor: Actor::System,
                },
                CoreEvent::AgentTimelineAppended(batch),
            );
        }

        self.observe_ownership(touched).await;
    }

    /// Cross-check what this tick reported against the ownership registry
    /// (T326).
    ///
    /// The batch carries only a session id; the roster is what knows the
    /// provider, and T223's link is keyed on the pair precisely so that two
    /// providers reusing one session id cannot be confused for each other.
    /// The **cached** roster, never a scan: a check must not cost a `ps`.
    ///
    /// Everything here is best-effort by design. No registry, no roster row, no
    /// link, no open workspace: no finding, and no error either — a violation
    /// nobody can attribute is not one. A failure costs this tick, not the loop.
    async fn observe_ownership(&self, touched: Vec<(String, Vec<WorkspacePath>)>) {
        let Some(ownership) = self.config.ownership.clone() else {
            return;
        };
        if touched.is_empty() {
            return;
        }
        let Some(roster) = self.cached() else {
            return;
        };
        for (session_id, paths) in touched {
            let Some(provider) = roster
                .sessions
                .iter()
                .find(|session| session.id == session_id)
                .map(|session| session.provider)
            else {
                continue;
            };
            if let Err(error) = ownership.observe_touches(provider, session_id, paths).await {
                tracing::debug!(%error, "ownership violation check failed");
            }
        }
    }

    /// Re-read the hook state directory, and re-derive the roster when it
    /// changed (T221).
    ///
    /// The re-derivation runs against the *cached* discovery, so a hook write
    /// costs no `ps` and no `lsof` — and it still goes through the same publish,
    /// so a tick whose hook state changed without changing any derived activity
    /// emits nothing at all (rule 8, §16).
    async fn follow_hooks(&self) {
        let Some(options) = self.config.hooks.clone() else {
            return;
        };
        let states = tokio::task::spawn_blocking(move || {
            gritty_agents::hook::scan(&options, gritty_agents::now_ms())
        })
        .await
        .unwrap_or_else(|error| {
            tracing::warn!(%error, "agent hook state scan failed");
            Vec::new()
        });

        let found: HashMap<String, HookState> = states
            .into_iter()
            .map(|state| (state.session_id.clone(), state))
            .collect();
        // Before anything is published: an ask a standing grant already covers
        // is answered here, so it never becomes a row, a count or a banner. The
        // user said yes to this once and asked not to be asked again (T335).
        self.observe_asks(&found);
        let changed = {
            let mut held = self.hooks.lock().unwrap_or_else(PoisonError::into_inner);
            if *held == found {
                false
            } else {
                *held = found;
                true
            }
        };
        if changed {
            self.republish().await;
        }
    }

    /// Tell the waiting hooks that Gritty is alive (T335, ADR-020).
    ///
    /// On the fast loop, because that is the loop whose cadence the marker's
    /// freshness window was chosen against. Its *absence* is what stops a hook
    /// waiting at all, so failing to write it is safe in the direction that
    /// matters: agents keep prompting their own users.
    fn touch_marker(&self) {
        let Some(paths) = self.config.permissions.as_ref() else {
            return;
        };
        if let Err(error) = gritty_agents::verdict::touch_marker(&paths.marker) {
            // Debug, not warn: on a machine where the data directory is not
            // writable this would otherwise fire several times a second, and
            // the consequence is only that answering is unavailable.
            tracing::debug!(%error, "could not refresh the agent liveness marker");
        }
    }

    /// Turn this scan's asks into domain requests, and honour any standing
    /// grant that covers one.
    fn observe_asks(&self, states: &HashMap<String, HookState>) {
        let requests: Vec<_> = states
            .values()
            .filter(|state| state.phase == gritty_agents::HookPhase::Blocked)
            .filter_map(|state| {
                let ask = state.ask.as_ref()?;
                Some(permission::translate(
                    &state.session_id,
                    ask,
                    state.since_ms,
                    |target| shorten(Path::new(target), self.config.home.as_deref()),
                ))
            })
            .collect();
        // "For this run" is only true while the run is: a grant whose session
        // has left the state directory has nothing left to authorise.
        self.asks
            .retain_sessions(&states.keys().cloned().collect::<Vec<_>>());
        for (request, grant_id) in self.asks.observe(requests) {
            self.write_verdict(&permission::Permissions::auto_verdict(&request.ask_id));
            tracing::debug!(
                ask = %request.ask_id,
                grant = %grant_id,
                "a standing grant answered a permission request"
            );
        }
    }

    /// Put a verdict where the waiting hook will find it.
    ///
    /// Failing is not an error the user has to see: the hook's own deadline
    /// runs out and the agent falls back to its own prompt, which is the same
    /// place they would have answered before Gritty existed.
    fn write_verdict(&self, verdict: &gritty_agents::verdict::Verdict) {
        let Some(paths) = self.config.permissions.as_ref() else {
            return;
        };
        if let Err(error) = gritty_agents::verdict::write(&paths.verdicts, verdict) {
            tracing::warn!(%error, "could not write a permission verdict");
        }
    }

    /// Re-derive and publish the roster from the last scan's evidence, without
    /// scanning again. Nothing happens before the first scan, because there is
    /// no evidence to re-derive.
    async fn republish(&self) {
        let found = self
            .discovery
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(found) = found {
            let roster = self.translate(found);
            self.publish(roster);
        }
    }

    fn record_tick(&self, reads: usize, batches: usize, elapsed: Duration) {
        let mut metrics = self.metrics.lock().unwrap_or_else(PoisonError::into_inner);
        metrics.ticks = metrics.ticks.saturating_add(1);
        metrics.reads = metrics.reads.saturating_add(reads as u64);
        metrics.batches = metrics.batches.saturating_add(batches as u64);
        drop(metrics);
        if reads > 0 {
            // Counts and durations only; never a prompt, a reply or a tool
            // input (rule 13).
            tracing::trace!(reads, batches, ?elapsed, "agent timeline tick");
        }
    }

    async fn poll(self: Arc<Self>) {
        // Like the timeline loop, this does *not* fire immediately. `list`
        // already scans on demand when nothing is cached, and the renderer
        // calls it on mount — so an immediate tick only ever duplicated that
        // scan, paying for a second `ps`/`lsof` pair against the same moment.
        let mut ticker = tokio::time::interval_at(
            tokio::time::Instant::now() + self.config.poll_interval,
            self.config.poll_interval,
        );
        // A scan that overruns the interval must not queue up backlogged ticks
        // and then fire them back to back.
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                () = self.cancel.cancelled() => break,
                _ = ticker.tick() => {}
            }
            self.scan_and_publish().await;
        }
        tracing::debug!("agent discovery stopped");
    }

    /// Scans, stores the result, and emits only if it differs from the last one.
    /// Takes the gate, then scans. The poll loop's entry point.
    async fn scan_and_publish(&self) -> AgentRoster {
        let _gate = self.scan_gate.lock().await;
        self.scan_and_publish_locked().await
    }

    /// The scan itself. The caller must hold `scan_gate`.
    async fn scan_and_publish_locked(&self) -> AgentRoster {
        let found = self.scan().await;
        // Kept so the fast loop can re-derive against fresh hook state without
        // paying for the two subprocesses again.
        *self
            .discovery
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(found.clone());
        let roster = self.translate(found);
        self.publish(roster)
    }

    /// Stores a derived roster and emits it only if it differs from the last
    /// one. The single place `AgentSessionsChanged` is published from, so the
    /// six-second loop and the fast loop cannot disagree about what "changed"
    /// means.
    fn publish(&self, roster: AgentRoster) -> AgentRoster {
        let changed = {
            let mut held = self.lock();
            if held.as_ref() == Some(&roster) {
                false
            } else {
                *held = Some(roster.clone());
                true
            }
        };

        if changed {
            tracing::debug!(
                sessions = roster.sessions.len(),
                needs_you = roster.needs_you,
                "agent roster changed"
            );
            self.events.emit(
                &EventOrigin {
                    // Machine-wide: a session in another checkout is not this
                    // workspace's event, and tagging it with one would be a lie
                    // the renderer would have to undo.
                    workspace_id: None,
                    correlation_id: None,
                    actor: Actor::System,
                },
                CoreEvent::AgentSessionsChanged(roster.clone()),
            );
        }
        // After the event, and on every roster rather than only the ones that
        // differed: the tracker's own pruning depends on seeing the current
        // population, and an unchanged roster produces no transitions anyway.
        self.raise_attention(&roster);
        roster
    }

    /// Notify about whatever just started needing the user (T223).
    ///
    /// The decision is entirely [`AttentionTracker`]'s; this only supplies the
    /// clock, the settings and the platform. Held-lock discipline matters here:
    /// the tracker's mutex is released before anything reaches the platform, so
    /// a notifier that blocks cannot hold up the next scan.
    fn raise_attention(&self, roster: &AgentRoster) {
        let enabled = self.notifications_enabled.load(Ordering::Relaxed);
        let focused = self.window_focused.load(Ordering::Relaxed);
        let alerts = {
            let mut tracker = self
                .attention
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            tracker.advance(roster, gritty_agents::now_ms(), enabled, focused)
        };
        let Some(notifier) = self.config.notifier.as_deref() else {
            return;
        };
        for alert in &alerts {
            // Counts and the session id only: a title, a prompt or a working
            // directory never reaches the log (rule 13).
            tracing::debug!(session = %alert.session_id, "raising an attention notification");
            notifier.show(alert);
        }
    }

    /// `agents.notifications`. What the settings surface shows. Reads the
    /// platform's answer without asking for it, so opening settings never
    /// produces a permission prompt.
    pub fn notification_settings(&self) -> AgentNotificationSettings {
        let Some(notifier) = self.config.notifier.as_deref() else {
            return AgentNotificationSettings {
                enabled: false,
                supported: false,
                permission: AgentNotificationPermission::Denied,
            };
        };
        AgentNotificationSettings {
            enabled: self.notifications_enabled.load(Ordering::Relaxed),
            supported: notifier.supported(),
            permission: notifier.permission(),
        }
    }

    /// `agents.setNotifications`.
    ///
    /// Turning it **on** is the one thing that asks the platform for
    /// permission — never application startup. A refusal is reported as a value
    /// and leaves the setting off; nothing retries on a loop, and the roster is
    /// unaffected either way.
    pub fn set_notifications(&self, enabled: bool) -> AgentNotificationSettings {
        let Some(notifier) = self.config.notifier.as_deref() else {
            return self.notification_settings();
        };
        if !enabled {
            self.notifications_enabled.store(false, Ordering::Relaxed);
            return self.notification_settings();
        }
        if !notifier.supported() {
            return self.notification_settings();
        }
        let permission = notifier.request_permission();
        let granted = permission == AgentNotificationPermission::Granted;
        self.notifications_enabled.store(granted, Ordering::Relaxed);
        tracing::info!(
            granted,
            ?permission,
            "agent attention notifications requested"
        );
        AgentNotificationSettings {
            enabled: granted,
            supported: true,
            permission,
        }
    }

    /// Whether the Gritty window has focus, from the composition root's window
    /// events. Focus that is never reported stays `true`, and Gritty stays
    /// silent (T223 §3, and the failure table's "window focus unknown").
    /// Answer a permission prompt an agent is stopped at (T335, ADR-020).
    ///
    /// The one command in this module that acts rather than reports, and it
    /// acts only on a decision a person just took. It writes the verdict where
    /// the waiting hook will find it, records what was answered, and republishes
    /// so the card, the row, the banner, the count and the status bar all stop
    /// saying the same thing at the same moment.
    ///
    /// An ask that is no longer open is **not an error**. The user may have
    /// answered in the terminal instead, or the session may have ended holding
    /// the prompt; ADR-020 says both answer paths are live at once and the
    /// first one wins. The roster comes back either way, which is what the
    /// caller needs in order to stop drawing a prompt that is gone.
    pub async fn answer_permission(
        &self,
        ask_id: &str,
        verdict: AgentPermissionVerdict,
    ) -> AgentRoster {
        let now =
            Timestamp::from_millis(u64::try_from(gritty_agents::now_ms()).unwrap_or_default());
        let (answered, to_write) = self.asks.answer(ask_id, verdict, now);
        if let Some(to_write) = &to_write {
            self.write_verdict(to_write);
        }
        match answered {
            permission::Answered::Written(answer) => tracing::debug!(
                ask = %answer.ask_id,
                session = %answer.session_id,
                "a permission request was answered"
            ),
            permission::Answered::Unknown => tracing::debug!(
                ask = %ask_id,
                "a permission request was answered after it had already gone"
            ),
        }
        self.republish().await;
        self.list().await
    }

    /// Every standing grant, for the queue's "standing rules" row.
    ///
    /// The architecture addendum §8 asks that permission grants be
    /// *inspectable*; this is what makes them so, and it is why a grant carries
    /// the count of asks it has answered on the user's behalf.
    pub fn permission_grants(&self) -> AgentPermissionGrants {
        AgentPermissionGrants {
            grants: self.asks.grants(),
        }
    }

    /// Take back a standing grant, so the next matching ask returns to the user.
    ///
    /// What Undo calls. **Not a rollback**: by the time a verdict is read the
    /// tool has run, and what can still be taken back is only the standing part
    /// of the answer.
    pub async fn revoke_grant(&self, grant_id: &str) -> AgentRoster {
        if self.asks.revoke(grant_id) {
            self.republish().await;
        }
        self.list().await
    }

    pub fn set_window_focused(&self, focused: bool) {
        self.window_focused.store(focused, Ordering::Relaxed);
    }

    async fn scan(&self) -> Discovery {
        let options = self.config.scan.clone();
        let runner = Arc::clone(&self.config.runner);
        let owned = self.config.owned.clone();
        // Blocking work: two subprocesses and a pile of file reads. It never
        // runs on the async worker that a command handler is waiting on.
        tokio::task::spawn_blocking(move || {
            // Read *inside* the blocking task rather than when it was
            // scheduled: a run started while this tick was queued must still be
            // excluded by it.
            let owned = owned.map(|read| read()).unwrap_or_default();
            gritty_agents::discover(
                options.as_ref(),
                runner.as_ref(),
                gritty_agents::now_ms(),
                &owned,
            )
        })
        .await
        .unwrap_or_else(|error| {
            // A panicked scan costs this tick, not the loop.
            tracing::warn!(%error, "agent discovery scan failed");
            Discovery::default()
        })
    }

    /// Turns raw evidence into the domain roster: activity, the workspace it
    /// belongs to, and the order the island shows it in.
    ///
    /// This is also where the third source joins (T221). Hook state is matched
    /// on **session id**, the one join key that is exact — the process/transcript
    /// join falls back to the working directory — and it only ever joins: a
    /// state file with no matching row creates none, because the agent's own
    /// report is evidence about a session, not proof one exists (ADR-018 §7).
    fn translate(&self, found: Discovery) -> AgentRoster {
        let workspace_root = self
            .workspaces
            .current_open()
            .map(|(_, root)| root.path().to_path_buf());
        let now_ms = gritty_agents::now_ms();
        let hooks = self.hooks.lock().unwrap_or_else(PoisonError::into_inner);

        // Where each row's timeline is read from. Refreshed on every scan and
        // kept out of the roster itself, because neither the transcript path
        // nor the working directory may cross the protocol (ADR-010).
        *self.sources.lock().unwrap_or_else(PoisonError::into_inner) = found
            .sessions
            .iter()
            .map(|session| {
                (
                    domain_session_id(session),
                    TimelineSource {
                        transcript: session.transcript_path.clone(),
                        cwd: session.working_directory.clone(),
                    },
                )
            })
            .collect();

        let mut permissions = HashMap::new();
        let mut sessions: Vec<AgentSession> = found
            .sessions
            .iter()
            .map(|session| {
                let hook = hook_for(&hooks, session);
                let domain = self.to_domain(
                    session,
                    hook,
                    workspace_root.as_deref(),
                    now_ms,
                    found.process_evidence,
                );
                // A prompt is worth naming only while the row actually says it
                // is waiting on one: an entry derived from a hook the derivation
                // then discarded would contradict the status beside it.
                if domain.activity == AgentActivity::Blocked
                    && let Some(hook) = hook
                    && let Some(entry) = timeline::permission_entry(
                        hook.tool.as_deref().or(session
                            .outstanding_tool
                            .as_ref()
                            .map(|call| call.name.as_str())),
                        detail_for(hook, session),
                        hook.since_ms,
                    )
                {
                    permissions.insert(domain.id.clone(), entry);
                }
                domain
            })
            .collect();
        drop(hooks);
        *self
            .permissions
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = permissions;

        sessions.sort_by(|left, right| rank(left).cmp(&rank(right)).then(by_recency(left, right)));

        AgentRoster {
            needs_you: u32::try_from(
                sessions
                    .iter()
                    .filter(|session| session.activity.needs_you())
                    .count(),
            )
            .unwrap_or(u32::MAX),
            sessions,
            process_evidence: found.process_evidence,
            recent_answers: self.asks.recent(),
        }
    }

    fn to_domain(
        &self,
        found: &DiscoveredSession,
        hook: Option<&HookState>,
        workspace_root: Option<&Path>,
        now_ms: i64,
        process_evidence: bool,
    ) -> AgentSession {
        let directory = found.working_directory.as_deref();
        let (activity, activity_source) = activity_of(
            found,
            hook,
            now_ms,
            self.config.idle_after,
            process_evidence,
        );
        let evidence = match found.evidence {
            gritty_agents::Evidence::ProcessOnly => AgentEvidence::ProcessOnly,
            gritty_agents::Evidence::TranscriptOnly => AgentEvidence::TranscriptOnly,
            gritty_agents::Evidence::Both => AgentEvidence::Both,
        };
        AgentSession {
            id: domain_session_id(found),
            provider: match found.provider {
                gritty_agents::Provider::ClaudeCode => AgentProvider::ClaudeCode,
            },
            activity,
            activity_source,
            evidence: if hook.is_some() {
                evidence.with_hook()
            } else {
                evidence
            },
            title: found.title.clone(),
            last_prompt: found.last_prompt.clone(),
            last_reply: found.last_reply.clone(),
            root: directory.map(|path| shorten(path, self.config.home.as_deref())),
            root_name: directory.map(display_name),
            // Component-wise, so a sibling checkout named `gritty-old` is not
            // reported as inside `gritty`.
            in_workspace: match (directory, workspace_root) {
                (Some(directory), Some(root)) => directory.starts_with(root),
                _ => false,
            },
            git_branch: found.git_branch.clone(),
            model: found.model.clone(),
            tool: found.outstanding_tool.as_ref().map(|call| AgentToolCall {
                name: call.name.clone(),
                detail: call.detail.clone(),
            }),
            pid: found.process.as_ref().map(|process| process.pid),
            tty: found
                .process
                .as_ref()
                .and_then(|process| process.tty.clone()),
            last_active_at: found
                .last_active_ms
                .and_then(|millis| u64::try_from(millis).ok())
                .map(Timestamp::from_millis),
            // Read, never derived here. `follow_hooks` is what decides an ask
            // exists — including deciding a standing grant answers it without
            // anyone seeing it — so by the time a row is built the answer to
            // "is this session waiting on me" is already settled (T335).
            //
            // Looked up by the *agent's* own session id, which is all the hook
            // knows, and handed out under the domain id, which is the only one
            // the renderer may act on. Same join `hook_for` makes, same reason.
            pending: (activity == AgentActivity::Blocked)
                .then(|| self.asks.pending_for(&found.session_id))
                .flatten()
                .map(|mut ask| {
                    ask.session_id = domain_session_id(found);
                    ask
                }),
        }
    }

    // -----------------------------------------------------------------------
    // The hook integration (T221, ADR-018)
    //
    // The only thing in this module that writes anywhere outside Gritty, and
    // the only thing that needs the user's consent. It is still not control:
    // installing a hook makes the agent *tell* Gritty when it stops, and adds
    // no way to answer, cancel or prompt one.
    // -----------------------------------------------------------------------

    /// `agents.integrationStatus`. Reads the provider's configuration and says
    /// what is in it. Never writes.
    pub async fn integration_status(
        &self,
        provider: AgentProvider,
    ) -> Result<AgentIntegration, CoreError> {
        let target = self.hook_target(provider)?;
        let hook_binary = self.config.hook_binary.clone();
        let report = tokio::task::spawn_blocking(move || integration::status(&target))
            .await
            .map_err(|error| CoreError::internal(format!("integration status: {error}")))?;
        Ok(AgentIntegration {
            provider,
            status: integration_status(report.status),
            config_path: self.shorten(&report.config_path),
            events: report.events,
            detail: report.detail,
            hook_available: hook_binary.is_some_and(|path| path.is_file()),
        })
    }

    /// `agents.integrationInstall`. `apply: false` is the preview — the file,
    /// the events, the entry and the backup it would create — and writes
    /// nothing. `apply: true` performs exactly that change.
    pub async fn integration_install(
        &self,
        provider: AgentProvider,
        apply: bool,
    ) -> Result<AgentIntegrationChange, CoreError> {
        let target = self.hook_target(provider)?;
        let Some(hook_binary) = self.config.hook_binary.clone() else {
            return Err(CoreError::new(
                CoreErrorCode::Io,
                "this build cannot find its own agent hook binary, so it will not install one",
            ));
        };
        let Some(state_dir) = self.config.hooks.as_ref().map(|hooks| hooks.dir.clone()) else {
            return Err(CoreError::new(
                CoreErrorCode::Io,
                "there is no application data directory to write hook state into",
            ));
        };
        let now_ms = gritty_agents::now_ms();
        let change = tokio::task::spawn_blocking(move || {
            integration::install(&target, &hook_binary, &state_dir, apply, now_ms)
        })
        .await
        .map_err(|error| CoreError::internal(format!("integration install: {error}")))?
        .map_err(integration_error)?;
        if change.applied {
            tracing::info!(
                provider = ?provider,
                events = change.events.len(),
                "agent hook integration installed"
            );
        }
        Ok(self.to_domain_change(provider, change))
    }

    /// `agents.integrationUninstall`. Removes exactly the entries whose command
    /// runs Gritty's binary and leaves the rest alone.
    pub async fn integration_uninstall(
        &self,
        provider: AgentProvider,
        apply: bool,
    ) -> Result<AgentIntegrationChange, CoreError> {
        let target = self.hook_target(provider)?;
        let now_ms = gritty_agents::now_ms();
        let change =
            tokio::task::spawn_blocking(move || integration::uninstall(&target, apply, now_ms))
                .await
                .map_err(|error| CoreError::internal(format!("integration uninstall: {error}")))?
                .map_err(integration_error)?;
        if change.applied {
            tracing::info!(
                provider = ?provider,
                events = change.events.len(),
                "agent hook integration removed"
            );
        }
        Ok(self.to_domain_change(provider, change))
    }

    fn hook_target(&self, provider: AgentProvider) -> Result<HookTarget, CoreError> {
        let Some(home) = self.config.home.as_deref() else {
            return Err(CoreError::new(
                CoreErrorCode::Io,
                "no home directory, so there is no agent configuration to read",
            ));
        };
        Ok(HookTarget::for_provider(
            match provider {
                AgentProvider::ClaudeCode => gritty_agents::Provider::ClaudeCode,
            },
            home,
        ))
    }

    fn to_domain_change(
        &self,
        provider: AgentProvider,
        change: gritty_agents::IntegrationChange,
    ) -> AgentIntegrationChange {
        AgentIntegrationChange {
            provider,
            applied: change.applied,
            config_path: self.shorten(&change.config_path),
            backup_path: change.backup_path.as_deref().map(|path| self.shorten(path)),
            creates_config: change.creates_config,
            command: change.command,
            events: change
                .events
                .into_iter()
                .map(|event| AgentIntegrationEvent {
                    event: event.event,
                    foreign_entries: event.foreign_entries,
                })
                .collect(),
            status: integration_status(change.status),
        }
    }

    /// Paths cross this boundary shortened, like every other one: the renderer
    /// has no filesystem and opens nothing from here (ADR-010).
    fn shorten(&self, path: &Path) -> String {
        shorten(path, self.config.home.as_deref())
    }

    fn cached(&self) -> Option<AgentRoster> {
        self.lock().clone()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<AgentRoster>> {
        self.roster.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The adapter's status onto the wire's.
fn integration_status(status: gritty_agents::IntegrationStatus) -> AgentIntegrationStatus {
    match status {
        gritty_agents::IntegrationStatus::NotInstalled => AgentIntegrationStatus::NotInstalled,
        gritty_agents::IntegrationStatus::Installed => AgentIntegrationStatus::Installed,
        gritty_agents::IntegrationStatus::Broken => AgentIntegrationStatus::Broken,
        gritty_agents::IntegrationStatus::UnsupportedConfig => {
            AgentIntegrationStatus::UnsupportedConfig
        }
    }
}

/// A refusal onto the core envelope, keeping the reason the user needs.
///
/// A configuration Gritty cannot understand is a `Conflict` rather than an
/// `InvalidInput`: nothing is wrong with the request, the file is simply not in
/// a state this change can be made against — and the message carries the parse
/// error so the user can go and look.
fn integration_error(error: IntegrationError) -> CoreError {
    let code = match error {
        IntegrationError::Unsupported { .. } => CoreErrorCode::Conflict,
        IntegrationError::PermissionDenied { .. } => CoreErrorCode::PermissionDenied,
        IntegrationError::Unreadable { .. } => CoreErrorCode::Io,
    };
    CoreError::new(code, error.to_string())
}

/// The files one batch says were written, in order and without repeats (T326).
///
/// Only [`AgentTimelineKind::FileTouched`] with a `path` counts. A `FileTouched`
/// with none is a file outside the open workspace by construction, and there is
/// nothing in the registry it could be checked against; every other kind — a
/// tool call, a reply, a notice — is not a write at all. `display` is never
/// looked at: it is a string for a human, and matching on it would be matching
/// on something that was never a path (rule 13).
fn touched_paths(entries: &[AgentTimelineEntry]) -> Vec<WorkspacePath> {
    let mut seen = std::collections::HashSet::new();
    entries
        .iter()
        .filter_map(|entry| match &entry.kind {
            AgentTimelineKind::FileTouched {
                path: Some(path), ..
            } => Some(path.clone()),
            _ => None,
        })
        .filter(|path| seen.insert(path.clone()))
        .collect()
}

/// The roster row's id: unique across providers, stable for the life of the
/// session, and the key everything else — the timeline, its source — is
/// addressed by.
fn domain_session_id(found: &DiscoveredSession) -> String {
    format!(
        "{}:{}",
        found.provider.as_str().to_ascii_lowercase(),
        found.session_id
    )
}

/// The hook state for this session, joined on session id.
///
/// Exact, and only exact: a state file whose id matches nothing is ignored, and
/// the working-directory fallback the process join uses has no place here —
/// two sessions in one checkout would each claim the other's prompt.
fn hook_for<'a>(
    hooks: &'a HashMap<String, HookState>,
    found: &DiscoveredSession,
) -> Option<&'a HookState> {
    hooks
        .get(&found.session_id)
        .filter(|hook| hook.provider == found.provider)
}

/// The bounded preview of the tool call the prompt is about.
///
/// Taken from the transcript, and only when it names the same tool the hook
/// did. The two are describing one call, so pairing them is not a guess — but
/// pairing a *different* call's input to this prompt would be, which is why the
/// names have to agree.
fn detail_for<'a>(hook: &HookState, found: &'a DiscoveredSession) -> Option<&'a str> {
    let call = found.outstanding_tool.as_ref()?;
    if hook.tool.as_deref().is_some_and(|tool| tool != call.name) {
        return None;
    }
    call.detail.as_deref()
}

/// What the session is doing, and which source said so.
///
/// Order matters, and every step of it is a claim about which evidence beats
/// which:
///
/// | Value | Evidence |
/// |---|---|
/// | `ENDED` | no live process, or the process table was unreadable |
/// | `IDLE` | live process, transcript quiet past the idle threshold |
/// | `BLOCKED` | live process, hook phase `BLOCKED`, not stale |
/// | `WORKING` | live process, a `tool_use` with no matching `tool_result` |
/// | `WAITING` | live process, nothing outstanding |
///
/// A stale transcript outranks an outstanding tool call: a call with no result
/// for ten minutes is a session someone walked away from, not one that is
/// working, and `WORKING` on a dead-quiet session would be the roster's least
/// trustworthy claim.
///
/// `ENDED` outranks `BLOCKED` because an agent killed at a prompt is not
/// blocked, it is gone (ADR-018 §7). `IDLE` outranks it for the reason above: a
/// prompt nobody answered for half an hour is a session someone walked away
/// from. A hook phase older than the idle threshold is discarded outright, so a
/// state file can never hold a row at `BLOCKED` forever — not even one whose
/// transcript is still moving.
///
/// `process_evidence` being `false` means the process table was unreadable, so
/// *everything* would otherwise read as `ENDED`. It still does — there is no
/// honest alternative — and `AgentRoster::process_evidence` is what tells the
/// UI to explain itself rather than assert the machine is quiet.
fn activity_of(
    found: &DiscoveredSession,
    hook: Option<&HookState>,
    now_ms: i64,
    idle_after: Duration,
    process_evidence: bool,
) -> (AgentActivity, AgentActivitySource) {
    if !process_evidence || !found.evidence.is_running() {
        return (AgentActivity::Ended, AgentActivitySource::Process);
    }
    // A session with no transcript has no quiet time to measure, so it cannot
    // be called idle — only running, which is all the evidence supports.
    if let Some(last_active_ms) = found.last_active_ms {
        let quiet_ms = now_ms.saturating_sub(last_active_ms);
        if quiet_ms > i64::try_from(idle_after.as_millis()).unwrap_or(i64::MAX) {
            return (AgentActivity::Idle, AgentActivitySource::Transcript);
        }
    }
    if let Some(hook) = hook
        && hook.phase == HookPhase::Blocked
        && !hook.is_stale(now_ms, idle_after)
    {
        return (AgentActivity::Blocked, AgentActivitySource::Hook);
    }
    if found.outstanding_tool.is_some() {
        return (AgentActivity::Working, AgentActivitySource::Transcript);
    }
    (AgentActivity::Waiting, AgentActivitySource::Process)
}

/// Newest first, with an unknown time sorting last rather than first: absence
/// of evidence must not win a tie.
fn by_recency(left: &AgentSession, right: &AgentSession) -> std::cmp::Ordering {
    let millis = |session: &AgentSession| {
        session
            .last_active_at
            .map(Timestamp::as_millis)
            .unwrap_or_default()
    };
    millis(right).cmp(&millis(left))
}

/// The island's reading order: this workspace first, then whatever is blocked
/// on the user, then what is running, then the rest by recency.
///
/// `BLOCKED` sorts above `WAITING` because it is the more urgent half of "your
/// move": a finished turn waits patiently, a permission prompt has an agent
/// stopped behind it.
fn rank(session: &AgentSession) -> (bool, u8) {
    let activity = match session.activity {
        AgentActivity::Blocked => 0,
        AgentActivity::Waiting => 1,
        AgentActivity::Working => 2,
        AgentActivity::Idle => 3,
        AgentActivity::Ended => 4,
    };
    (!session.in_workspace, activity)
}

#[cfg(test)]
mod tests;
