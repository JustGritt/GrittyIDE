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

pub mod wire;

use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use gritty_agents::{
    CommandRunner, DiscoveredSession, Discovery, ScanOptions, SystemCommandRunner,
};
use tokio::runtime::Handle;
use tokio_util::sync::CancellationToken;

use crate::event::{CoreEvent, EventBus, EventOrigin};
use crate::protocol::actor::Actor;
use crate::protocol::time::Timestamp;
use crate::workspace::WorkspaceManager;
use crate::workspace::recents::{display_name, shorten};

pub use wire::{
    AgentActivity, AgentEvidence, AgentProvider, AgentRoster, AgentSession, AgentToolCall,
    ListAgentSessionsRequest,
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

/// What the composition root injects. Tests replace the runner and the root so
/// no test spawns a process or reads a real transcript.
pub struct AgentDiscoveryConfig {
    /// Where the transcripts are, or `None` to run on process evidence alone.
    pub scan: Option<ScanOptions>,
    pub runner: Arc<dyn CommandRunner>,
    pub poll_interval: Duration,
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
            cancel: CancellationToken::new(),
        });
        let poller = Arc::clone(&manager);
        handle.spawn(async move { poller.poll().await });
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

    /// Stops the poll loop. Called from the composition root's shutdown.
    pub fn shutdown(&self) {
        self.cancel.cancel();
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
            id: format!(
                "{}:{}",
                found.provider.as_str().to_ascii_lowercase(),
                found.session_id
            ),
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
