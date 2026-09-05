//! Agent session discovery (T220): what else on this machine is working.
//!
//! Gritty is an orchestration cockpit long before it orchestrates anything, and
//! the first question in `AGENTS.md` — *what is happening* — has an answer today
//! for the agents the user already runs in their own terminals. This module
//! polls for them, translates the evidence into domain state, and publishes a
//! roster.
//!
//! It reads and nothing else. There is no command here that starts, stops,
//! answers or approves a session, because Gritty does not own these sessions:
//! another terminal does. When T300 brings runs Gritty *does* own, they get
//! their own type and their own lifecycle; this one keeps meaning "someone
//! else's work, visible".
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

pub mod timeline;
pub mod wire;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use gritty_agents::{
    CommandRunner, DiscoveredSession, Discovery, ScanOptions, SystemCommandRunner,
};
use tokio::runtime::Handle;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::event::{CoreEvent, EventBus, EventOrigin};
use crate::protocol::actor::Actor;
use crate::protocol::error::CoreError;
use crate::protocol::time::Timestamp;
use crate::workspace::WorkspaceManager;
use crate::workspace::recents::{display_name, shorten};
use timeline::OpenTimeline;

pub use timeline::MAX_TIMELINE_ENTRIES;
pub use wire::{
    AgentActivity, AgentEvidence, AgentNoticeLevel, AgentProvider, AgentRoster, AgentSession,
    AgentTimeline, AgentTimelineAppendedPayload, AgentTimelineEntry, AgentTimelineKind,
    AgentTimelineRequest, AgentToolCall, ListAgentSessionsRequest,
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
    /// Used to shorten displayed paths; `None` shows them in full.
    pub home: Option<std::path::PathBuf>,
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
        }
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
    /// Every open timeline, by session id.
    timelines: Mutex<HashMap<String, OpenTimeline>>,
    /// The transcript and working directory behind each roster row, refreshed
    /// by every scan.
    sources: Mutex<HashMap<String, TimelineSource>>,
    metrics: Mutex<FollowMetrics>,
    /// Both loops, so shutdown can wait for them rather than assume.
    tasks: Mutex<Vec<JoinHandle<()>>>,
    cancel: CancellationToken,
}

impl AgentSessionManager {
    /// Builds the manager and starts polling. The first tick fires immediately,
    /// so the roster is populated without waiting out an interval.
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
            timelines: Mutex::new(HashMap::new()),
            sources: Mutex::new(HashMap::new()),
            metrics: Mutex::new(FollowMetrics::default()),
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
        self.scan_and_publish().await
    }

    /// `agents.refresh`. Always rescans.
    pub async fn refresh(&self) -> AgentRoster {
        self.scan_and_publish().await
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
        timeline
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

    /// One tick of the fast loop: read what was appended to every open
    /// timeline, fold it, and emit at most one batch per session.
    async fn follow_once(&self) {
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

        if jobs.is_empty() {
            self.record_tick(0, 0, Duration::ZERO);
            return;
        }

        let reads = jobs.len();
        let started = Instant::now();
        let followed = tokio::task::spawn_blocking(move || {
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
        });

        let home = self.config.home.as_deref();
        let batches: Vec<AgentTimelineAppendedPayload> = {
            let mut open = self.timelines_lock();
            followed
                .into_iter()
                .filter_map(|(session_id, followed)| {
                    // Closed while the read was in flight: its ring is gone and
                    // there is nobody to deliver to.
                    let timeline = open.get_mut(&session_id)?;
                    timeline.absorb(followed, home);
                    timeline.take_batch(&session_id)
                })
                .collect()
        };
        self.record_tick(reads, batches.len(), started.elapsed());

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
        let mut ticker = tokio::time::interval(self.config.poll_interval);
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
    async fn scan_and_publish(&self) -> AgentRoster {
        let roster = self.scan().await;
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
        roster
    }

    async fn scan(&self) -> AgentRoster {
        let options = self.config.scan.clone();
        let runner = Arc::clone(&self.config.runner);
        // Blocking work: two subprocesses and a pile of file reads. It never
        // runs on the async worker that a command handler is waiting on.
        let found = tokio::task::spawn_blocking(move || {
            gritty_agents::discover(options.as_ref(), runner.as_ref(), gritty_agents::now_ms())
        })
        .await
        .unwrap_or_else(|error| {
            // A panicked scan costs this tick, not the loop.
            tracing::warn!(%error, "agent discovery scan failed");
            Discovery::default()
        });

        self.translate(found)
    }

    /// Turns raw evidence into the domain roster: activity, the workspace it
    /// belongs to, and the order the island shows it in.
    fn translate(&self, found: Discovery) -> AgentRoster {
        let workspace_root = self
            .workspaces
            .current_open()
            .map(|(_, root)| root.path().to_path_buf());
        let now_ms = gritty_agents::now_ms();

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

        let mut sessions: Vec<AgentSession> = found
            .sessions
            .iter()
            .map(|session| {
                self.to_domain(
                    session,
                    workspace_root.as_deref(),
                    now_ms,
                    found.process_evidence,
                )
            })
            .collect();

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
        }
    }

    fn to_domain(
        &self,
        found: &DiscoveredSession,
        workspace_root: Option<&Path>,
        now_ms: i64,
        process_evidence: bool,
    ) -> AgentSession {
        let directory = found.working_directory.as_deref();
        AgentSession {
            id: domain_session_id(found),
            provider: match found.provider {
                gritty_agents::Provider::ClaudeCode => AgentProvider::ClaudeCode,
            },
            activity: activity_of(found, now_ms, self.config.idle_after, process_evidence),
            evidence: match found.evidence {
                gritty_agents::Evidence::ProcessOnly => AgentEvidence::ProcessOnly,
                gritty_agents::Evidence::TranscriptOnly => AgentEvidence::TranscriptOnly,
                gritty_agents::Evidence::Both => AgentEvidence::Both,
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
        }
    }

    fn cached(&self) -> Option<AgentRoster> {
        self.lock().clone()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<AgentRoster>> {
        self.roster.lock().unwrap_or_else(PoisonError::into_inner)
    }
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

/// What the session is doing, from evidence alone.
///
/// Order matters. A stale transcript outranks an outstanding tool call: a call
/// with no result for ten minutes is a session someone walked away from, not
/// one that is working, and `WORKING` on a dead-quiet session would be the
/// roster's least trustworthy claim.
///
/// `process_evidence` being `false` means the process table was unreadable, so
/// *everything* would otherwise read as `ENDED`. It still does — there is no
/// honest alternative — and `AgentRoster::process_evidence` is what tells the
/// UI to explain itself rather than assert the machine is quiet.
fn activity_of(
    found: &DiscoveredSession,
    now_ms: i64,
    idle_after: Duration,
    process_evidence: bool,
) -> AgentActivity {
    if !process_evidence || !found.evidence.is_running() {
        return AgentActivity::Ended;
    }
    // A session with no transcript has no quiet time to measure, so it cannot
    // be called idle — only running, which is all the evidence supports.
    if let Some(last_active_ms) = found.last_active_ms {
        let quiet_ms = now_ms.saturating_sub(last_active_ms);
        if quiet_ms > i64::try_from(idle_after.as_millis()).unwrap_or(i64::MAX) {
            return AgentActivity::Idle;
        }
    }
    if found.outstanding_tool.is_some() {
        return AgentActivity::Working;
    }
    AgentActivity::Waiting
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
fn rank(session: &AgentSession) -> (bool, u8) {
    let activity = match session.activity {
        AgentActivity::Waiting => 0,
        AgentActivity::Working => 1,
        AgentActivity::Idle => 2,
        AgentActivity::Ended => 3,
    };
    (!session.in_workspace, activity)
}

#[cfg(test)]
mod tests;
