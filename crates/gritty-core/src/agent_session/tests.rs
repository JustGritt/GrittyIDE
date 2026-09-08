//! T220 tests. Nothing here spawns a process or reads a real transcript: the
//! command runner is injected and the transcript root is a scratch directory.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use gritty_agents::hook::{AskVerb, HookAsk, HookPhase, HookState};
use gritty_agents::{
    CommandRunner, DiscoveredSession, Discovery, Evidence, ProcessFacts, Provider, ToolCall,
};

use super::*;
use crate::event::{EventEnvelope, EventSink, FanOutSink};
use crate::operation::OperationRegistry;

const MINUTE_MS: i64 = 60 * 1_000;
const NOW_MS: i64 = 1_800_000_000_000;

fn discovered(evidence: Evidence, last_active_ms: i64, tool: Option<&str>) -> DiscoveredSession {
    DiscoveredSession {
        provider: Provider::ClaudeCode,
        session_id: "s1".to_owned(),
        working_directory: Some(PathBuf::from("/Users/someone/code/gritty")),
        transcript_path: None,
        title: Some("a task".to_owned()),
        last_prompt: None,
        last_reply: None,
        model: None,
        git_branch: None,
        outstanding_tool: tool.map(|name| ToolCall {
            name: name.to_owned(),
            detail: None,
        }),
        last_active_ms: Some(last_active_ms),
        process: matches!(evidence, Evidence::ProcessOnly | Evidence::Both).then(|| ProcessFacts {
            pid: 42,
            tty: Some("/dev/ttys001".to_owned()),
        }),
        evidence,
    }
}

// ---------------------------------------------------------------- activity

/// What the derivation says, and which source said it.
fn derived(
    session: &DiscoveredSession,
    hook: Option<&HookState>,
    process_evidence: bool,
) -> (AgentActivity, AgentActivitySource) {
    activity_of(session, hook, NOW_MS, DEFAULT_IDLE_AFTER, process_evidence)
}

/// A state file the agent wrote `age_ms` ago.
fn hook(phase: HookPhase, age_ms: i64, tool: Option<&str>) -> HookState {
    HookState {
        version: gritty_agents::hook::HOOK_STATE_VERSION,
        session_id: "s1".to_owned(),
        provider: Provider::ClaudeCode,
        phase,
        since_ms: NOW_MS - age_ms,
        event: "PermissionRequest".to_owned(),
        tool: tool.map(str::to_owned),
        cwd: None,
        ask: None,
    }
}

#[test]
fn an_outstanding_tool_call_is_working() {
    let session = discovered(Evidence::Both, NOW_MS, Some("Bash"));
    assert_eq!(
        derived(&session, None, true),
        (AgentActivity::Working, AgentActivitySource::Transcript)
    );
}

#[test]
fn a_live_session_with_nothing_outstanding_is_waiting_on_the_user() {
    let session = discovered(Evidence::Both, NOW_MS, None);
    assert_eq!(
        derived(&session, None, true),
        (AgentActivity::Waiting, AgentActivitySource::Process)
    );
    assert!(AgentActivity::Waiting.needs_you());
}

#[test]
fn a_quiet_live_session_goes_idle_and_stops_asking_for_attention() {
    let session = discovered(Evidence::Both, NOW_MS - 30 * MINUTE_MS, None);
    assert_eq!(
        derived(&session, None, true),
        (AgentActivity::Idle, AgentActivitySource::Transcript)
    );
    assert!(!AgentActivity::Idle.needs_you());
}

#[test]
fn a_stale_transcript_outranks_an_outstanding_tool_call() {
    // A call with no result for half an hour is abandoned, not working.
    let session = discovered(Evidence::Both, NOW_MS - 30 * MINUTE_MS, Some("Bash"));
    assert_eq!(derived(&session, None, true).0, AgentActivity::Idle);
}

#[test]
fn no_process_means_ended() {
    let session = discovered(Evidence::TranscriptOnly, NOW_MS, Some("Bash"));
    assert_eq!(
        derived(&session, None, true),
        (AgentActivity::Ended, AgentActivitySource::Process)
    );
}

#[test]
fn a_blind_scan_reports_ended_rather_than_guessing() {
    // The process table was unreadable. Everything reads ENDED; the roster's
    // `process_evidence` flag is what says why.
    let session = discovered(Evidence::Both, NOW_MS, Some("Bash"));
    assert_eq!(derived(&session, None, false).0, AgentActivity::Ended);
}

// ------------------------------------------------ hook evidence (T221 §12)

#[test]
fn a_live_session_whose_hook_says_blocked_is_blocked() {
    let session = discovered(Evidence::Both, NOW_MS, None);
    assert_eq!(
        derived(
            &session,
            Some(&hook(HookPhase::Blocked, 0, Some("Bash"))),
            true
        ),
        (AgentActivity::Blocked, AgentActivitySource::Hook),
        "the one value poll-only discovery cannot reach"
    );
    assert!(AgentActivity::Blocked.needs_you());
    assert!(AgentActivity::Blocked.is_running());
}

#[test]
fn ended_outranks_blocked_because_an_agent_killed_at_a_prompt_is_gone() {
    // The state file still says BLOCKED. There is no process behind it.
    let session = discovered(Evidence::TranscriptOnly, NOW_MS, None);
    assert_eq!(
        derived(
            &session,
            Some(&hook(HookPhase::Blocked, 0, Some("Bash"))),
            true
        ),
        (AgentActivity::Ended, AgentActivitySource::Process)
    );
    // And the same when the process table itself was unreadable.
    let live = discovered(Evidence::Both, NOW_MS, None);
    assert_eq!(
        derived(&live, Some(&hook(HookPhase::Blocked, 0, None)), false).0,
        AgentActivity::Ended
    );
}

#[test]
fn idle_outranks_blocked_because_an_unanswered_prompt_is_a_session_walked_away_from() {
    let session = discovered(Evidence::Both, NOW_MS - 30 * MINUTE_MS, None);
    assert_eq!(
        derived(
            &session,
            Some(&hook(HookPhase::Blocked, 30 * MINUTE_MS, None)),
            true
        ),
        (AgentActivity::Idle, AgentActivitySource::Transcript)
    );
}

#[test]
fn a_stale_hook_phase_is_discarded_even_when_the_transcript_is_moving() {
    // The transcript is fresh, so the IDLE rule above does not fire — but a
    // BLOCKED phase from half an hour ago must not hold the row forever.
    let session = discovered(Evidence::Both, NOW_MS, Some("Bash"));
    assert_eq!(
        derived(
            &session,
            Some(&hook(HookPhase::Blocked, 30 * MINUTE_MS, None)),
            true
        ),
        (AgentActivity::Working, AgentActivitySource::Transcript)
    );
}

#[test]
fn blocked_outranks_working_because_the_prompt_is_why_the_tool_stopped() {
    let session = discovered(Evidence::Both, NOW_MS, Some("Bash"));
    assert_eq!(
        derived(
            &session,
            Some(&hook(HookPhase::Blocked, 0, Some("Bash"))),
            true
        )
        .0,
        AgentActivity::Blocked
    );
}

#[test]
fn no_phase_other_than_blocked_changes_what_the_row_says() {
    // The hook reports four phases; only one of them is a value the derivation
    // could not have reached on its own, and the rest change nothing.
    let session = discovered(Evidence::Both, NOW_MS, None);
    for phase in [HookPhase::Active, HookPhase::Settled, HookPhase::Ended] {
        assert_eq!(
            derived(&session, Some(&hook(phase, 0, None)), true),
            (AgentActivity::Waiting, AgentActivitySource::Process),
            "{phase:?}"
        );
    }
}

#[test]
fn blocked_is_unreachable_without_hook_evidence() {
    // Rule 12 by construction: no transcript shape, process state or elapsed
    // time may produce it. Every combination of the other two sources.
    for evidence in [
        Evidence::Both,
        Evidence::ProcessOnly,
        Evidence::TranscriptOnly,
    ] {
        for tool in [None, Some("Bash")] {
            for age in [0, 30 * MINUTE_MS] {
                for process_evidence in [true, false] {
                    let session = discovered(evidence, NOW_MS - age, tool);
                    assert_ne!(
                        derived(&session, None, process_evidence).0,
                        AgentActivity::Blocked,
                        "{evidence:?} {tool:?} {age} {process_evidence}"
                    );
                }
            }
        }
    }
}

// ------------------------------------------------------------------ manager

/// Replays canned command output.
struct FakeRunner(Option<String>);

impl CommandRunner for FakeRunner {
    fn run(&self, program: &str, _args: &[&str], _timeout: Duration) -> Option<String> {
        (program == "/bin/ps").then(|| self.0.clone()).flatten()
    }
}

#[derive(Default)]
struct RecordingSink(Mutex<Vec<EventEnvelope>>);

impl EventSink for RecordingSink {
    fn emit(&self, envelope: &EventEnvelope) -> Result<(), CoreError> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(envelope.clone());
        Ok(())
    }
}

impl RecordingSink {
    fn rosters(&self) -> Vec<AgentRoster> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter_map(|envelope| match &envelope.event {
                CoreEvent::AgentSessionsChanged(roster) => Some(roster.clone()),
                _ => None,
            })
            .collect()
    }
}

use crate::CoreError;

/// A manager whose poll loop is effectively off (a very long interval), so each
/// test drives it explicitly and nothing races.
fn manager(runner: Arc<dyn CommandRunner>) -> (Arc<AgentSessionManager>, Arc<RecordingSink>) {
    manager_with_hooks(runner, None)
}

/// The same, reading hook state from a scratch directory. Never from a real
/// application-data directory: no test in this file writes anywhere the running
/// application would look.
fn manager_with_hooks(
    runner: Arc<dyn CommandRunner>,
    hooks: Option<PathBuf>,
) -> (Arc<AgentSessionManager>, Arc<RecordingSink>) {
    manager_with(runner, hooks, None)
}

/// Verdicts and the liveness marker go beside whatever state directory a test
/// gave the manager, so answering is exercised against real files rather than
/// stubbed out. Nothing here touches a real application-data directory.
fn permission_paths(state_dir: &Path) -> PermissionPaths {
    let parent = state_dir.parent().unwrap_or(state_dir);
    PermissionPaths {
        verdicts: parent.join("verdicts"),
        marker: parent.join("live"),
    }
}

/// The same, with an injected attention notifier (T223). Nothing in this file
/// reaches a real notification centre.
fn manager_with(
    runner: Arc<dyn CommandRunner>,
    hooks: Option<PathBuf>,
    notifier: Option<Arc<dyn AttentionNotifier>>,
) -> (Arc<AgentSessionManager>, Arc<RecordingSink>) {
    let sink = Arc::new(RecordingSink::default());
    let events = EventBus::new(FanOutSink::new(vec![sink.clone()]));
    let handle = Handle::current();
    let registry = OperationRegistry::new(handle.clone(), events.clone());
    let workspaces = WorkspaceManager::new(handle.clone(), events.clone(), registry);
    let manager = AgentSessionManager::start(
        handle,
        events,
        workspaces,
        AgentDiscoveryConfig {
            // No transcript root: this exercises process evidence alone.
            scan: None,
            runner,
            poll_interval: Duration::from_secs(3_600),
            timeline_interval: Duration::from_secs(3_600),
            idle_after: DEFAULT_IDLE_AFTER,
            home: Some(PathBuf::from("/Users/someone")),
            permissions: hooks.as_deref().map(permission_paths),
            hooks: hooks.map(gritty_agents::HookScanOptions::rooted_at),
            hook_binary: None,
            notifier,
            ownership: None,
            owned: None,
        },
    );
    (manager, sink)
}

/// The same, with the owned-session seam wired (T310-B §8).
fn manager_with_owned(
    runner: Arc<dyn CommandRunner>,
    owned: OwnedSessionsFn,
) -> (Arc<AgentSessionManager>, Arc<RecordingSink>) {
    let sink = Arc::new(RecordingSink::default());
    let events = EventBus::new(FanOutSink::new(vec![sink.clone()]));
    let handle = Handle::current();
    let registry = OperationRegistry::new(handle.clone(), events.clone());
    let workspaces = WorkspaceManager::new(handle.clone(), events.clone(), registry);
    let manager = AgentSessionManager::start(
        handle,
        events,
        workspaces,
        AgentDiscoveryConfig {
            scan: None,
            runner,
            poll_interval: Duration::from_secs(3_600),
            timeline_interval: Duration::from_secs(3_600),
            idle_after: DEFAULT_IDLE_AFTER,
            home: Some(PathBuf::from("/Users/someone")),
            permissions: None,
            hooks: None,
            hook_binary: None,
            notifier: None,
            ownership: None,
            owned: Some(owned),
        },
    );
    (manager, sink)
}

/// The exclusion set is read on **every** scan, not captured when the manager
/// was built (T310-B §8).
///
/// This is the assertion a captured-once implementation would fail and every
/// other test in this file would pass: a run started after the manager exists
/// has to be excluded by the *next* tick, not the one after a restart. The seam
/// is a closure precisely so this is possible, and it stands in for
/// `AgentRunManager` without importing one — which is also the proof that
/// `agent_session` learns nothing about runs.
#[tokio::test]
async fn the_owned_set_is_read_on_every_scan() {
    const OURS: &str = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
    let runner = Arc::new(FakeRunner(Some(format!(
        " 4242 900 ??       claude -p --session-id {OURS}\n 5815 900 ttys000  claude\n"
    ))));
    let live: Arc<std::sync::Mutex<Vec<String>>> = Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = Arc::clone(&live);
    let (manager, _sink) = manager_with_owned(
        runner,
        Arc::new(move || OwnedSessions::new(seen.lock().expect("not poisoned").clone(), [])),
    );

    // Nothing owned yet: both agents are reported.
    let before = manager.list().await;
    assert_eq!(before.sessions.len(), 2, "{:?}", before.sessions);

    // A run starts. The very next scan must already exclude it.
    live.lock().expect("not poisoned").push(OURS.to_owned());
    let after = manager.refresh().await;
    assert_eq!(
        after.sessions.len(),
        1,
        "the set was captured instead of read: {:?}",
        after.sessions
    );
    assert!(
        after.sessions.iter().all(|session| session.id != OURS),
        "the agent Gritty owns is not also an observed session"
    );
}

#[tokio::test]
async fn list_scans_once_and_publishes_the_roster() {
    let runner = Arc::new(FakeRunner(Some(" 42 1 ttys001  claude\n".to_owned())));
    let (manager, sink) = manager(runner);

    let roster = manager.list().await;
    assert_eq!(roster.sessions.len(), 1);
    assert!(roster.process_evidence);
    assert_eq!(roster.sessions[0].id, "claude_code:pid-42");
    assert_eq!(roster.sessions[0].pid, Some(42));
    assert_eq!(roster.sessions[0].evidence, AgentEvidence::ProcessOnly);
    // A process with no transcript is running, but what it is doing is unknown.
    assert_eq!(roster.sessions[0].activity, AgentActivity::Waiting);
    assert_eq!(roster.sessions[0].title, None);

    manager.shutdown();
    assert_eq!(sink.rosters().len(), 1, "the first roster is published");
}

#[tokio::test]
async fn an_unchanged_roster_emits_nothing() {
    let runner = Arc::new(FakeRunner(Some(" 42 1 ttys001  claude\n".to_owned())));
    let (manager, sink) = manager(runner);

    manager.list().await;
    manager.refresh().await;
    manager.refresh().await;
    manager.shutdown();

    assert_eq!(
        sink.rosters().len(),
        1,
        "three scans, one change, one event (rule 8)"
    );
}

#[tokio::test]
async fn a_changed_roster_emits_once_more() {
    let responses = Arc::new(Mutex::new(vec![
        " 42 1 ttys001  claude\n".to_owned(),
        String::new(),
    ]));

    struct Draining(Arc<Mutex<Vec<String>>>);
    impl CommandRunner for Draining {
        fn run(&self, program: &str, _args: &[&str], _timeout: Duration) -> Option<String> {
            if program != "/bin/ps" {
                return None;
            }
            let mut left = self.0.lock().unwrap_or_else(PoisonError::into_inner);
            Some(if left.is_empty() {
                String::new()
            } else {
                left.remove(0)
            })
        }
    }

    let (manager, sink) = manager(Arc::new(Draining(responses)));
    let first = manager.list().await;
    let second = manager.refresh().await;
    manager.shutdown();

    assert_eq!(first.sessions.len(), 1);
    assert!(second.sessions.is_empty(), "the agent exited");
    assert_eq!(sink.rosters().len(), 2);
}

#[tokio::test]
async fn a_blind_probe_says_so_instead_of_claiming_the_machine_is_quiet() {
    let (manager, _sink) = manager(Arc::new(FakeRunner(None)));
    let roster = manager.list().await;
    manager.shutdown();

    assert!(roster.sessions.is_empty());
    assert!(
        !roster.process_evidence,
        "an unreadable process table is not an empty one"
    );
}

// ------------------------------------------------------------------ ranking

fn ranked(activity: AgentActivity, in_workspace: bool, last_active_ms: u64) -> AgentSession {
    AgentSession {
        id: format!("{activity:?}-{in_workspace}-{last_active_ms}"),
        provider: AgentProvider::ClaudeCode,
        activity,
        activity_source: AgentActivitySource::Process,
        evidence: AgentEvidence::Both,
        title: None,
        last_prompt: None,
        last_reply: None,
        root: None,
        root_name: None,
        in_workspace,
        git_branch: None,
        model: None,
        tool: None,
        pid: None,
        tty: None,
        last_active_at: Some(Timestamp::from_millis(last_active_ms)),
        pending: None,
    }
}

#[test]
fn the_open_workspace_sorts_first_then_what_is_blocked_on_you() {
    let mut sessions = [
        ranked(AgentActivity::Ended, false, 900),
        ranked(AgentActivity::Working, true, 100),
        ranked(AgentActivity::Waiting, false, 100),
        ranked(AgentActivity::Waiting, true, 100),
        ranked(AgentActivity::Idle, true, 100),
    ];
    sessions.sort_by(|left, right| rank(left).cmp(&rank(right)).then(by_recency(left, right)));

    let order: Vec<_> = sessions
        .iter()
        .map(|session| (session.in_workspace, session.activity))
        .collect();
    assert_eq!(
        order,
        vec![
            (true, AgentActivity::Waiting),
            (true, AgentActivity::Working),
            (true, AgentActivity::Idle),
            (false, AgentActivity::Waiting),
            (false, AgentActivity::Ended),
        ]
    );
}

#[test]
fn recency_breaks_ties_within_a_rank() {
    let mut sessions = [
        ranked(AgentActivity::Waiting, true, 100),
        ranked(AgentActivity::Waiting, true, 900),
    ];
    sessions.sort_by(|left, right| rank(left).cmp(&rank(right)).then(by_recency(left, right)));
    assert_eq!(
        sessions[0].last_active_at.map(Timestamp::as_millis),
        Some(900)
    );
}

// -------------------------------------------------------------- translation

#[tokio::test]
async fn translation_shortens_the_path_and_places_it_against_the_workspace() {
    let (manager, _sink) = manager(Arc::new(FakeRunner(None)));
    let found = Discovery {
        sessions: vec![discovered(Evidence::Both, NOW_MS, Some("Bash"))],
        process_evidence: true,
    };

    let roster = manager.translate(found);
    manager.shutdown();

    let session = &roster.sessions[0];
    assert_eq!(session.root.as_deref(), Some("~/code/gritty"));
    assert_eq!(session.root_name.as_deref(), Some("gritty"));
    assert_eq!(session.tty.as_deref(), Some("/dev/ttys001"));
    assert_eq!(
        session.tool.as_ref().map(|tool| tool.name.as_str()),
        Some("Bash")
    );
    // No workspace is open in this test, so nothing is "here".
    assert!(!session.in_workspace);
}

#[tokio::test]
async fn needs_you_counts_only_the_sessions_waiting_on_the_user() {
    let (manager, _sink) = manager(Arc::new(FakeRunner(None)));
    let found = Discovery {
        sessions: vec![
            discovered(Evidence::Both, NOW_MS, None),
            discovered(Evidence::Both, NOW_MS, Some("Bash")),
            discovered(Evidence::TranscriptOnly, NOW_MS, None),
        ],
        process_evidence: true,
    };

    let roster = manager.translate(found);
    manager.shutdown();
    assert_eq!(roster.needs_you, 1);
}

// ----------------------------------------------------------------- timeline
//
// T230. The transcript is a scratch file the test writes; the fast loop is
// driven by hand (`follow_once`) so nothing races, and the loop's own counters
// are what "the follower stopped" is asserted against rather than a claim.

/// A scratch transcript root that cleans itself up.
struct TempTranscripts(PathBuf);

const TEST_SESSION: &str = "eca001ac-5b6f-40ff-b79a-9ae79ccc6483";

impl TempTranscripts {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "gritty-agent-timeline-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(path.join("-Users-a-p")).expect("create transcript root");
        Self(path)
    }

    fn transcript(&self, session: &str) -> PathBuf {
        self.0.join(format!("-Users-a-p/{session}.jsonl"))
    }

    /// Writes a session's opening records and returns its roster id.
    fn seed(&self, session: &str) -> String {
        let opening = format!(
            "{}\n{}\n",
            format_args!(
                r#"{{"type":"user","cwd":"/Users/a/p","sessionId":"{session}","timestamp":"2026-09-05T13:49:47.770Z","message":{{"role":"user","content":[{{"type":"text","text":"ship the rail"}}]}}}}"#
            ),
            format_args!(
                r#"{{"type":"file-history-delta","sessionId":"{session}","cwd":"/Users/a/p","trackingPath":"src/rail.rs"}}"#
            ),
        );
        std::fs::write(self.transcript(session), opening).expect("write transcript");
        format!("claude_code:{session}")
    }

    fn append(&self, session: &str, line: &str) {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(self.transcript(session))
            .expect("open transcript");
        writeln!(file, "{line}").expect("append");
    }

    /// A hook state directory inside the same scratch tree (T221).
    fn hook_dir(&self) -> PathBuf {
        self.0.join("hook-state")
    }

    fn hooks(&self) -> Option<gritty_agents::HookScanOptions> {
        Some(gritty_agents::HookScanOptions::rooted_at(self.hook_dir()))
    }

    /// Where the hook would look for a verdict, given where it writes state.
    fn verdicts(&self) -> PathBuf {
        self.0.join("verdicts")
    }

    /// Write what the agent's hook would have written for this session.
    fn record_phase(&self, session: &str, phase: HookPhase, tool: Option<&str>) {
        let state = HookState {
            version: gritty_agents::hook::HOOK_STATE_VERSION,
            session_id: session.to_owned(),
            provider: Provider::ClaudeCode,
            phase,
            since_ms: gritty_agents::now_ms(),
            event: "PermissionRequest".to_owned(),
            tool: tool.map(str::to_owned),
            cwd: Some("/Users/a/p".to_owned()),
            ask: None,
        };
        assert_eq!(
            gritty_agents::hook::write(&self.hook_dir(), &state),
            gritty_agents::WriteOutcome::Written
        );
    }

    /// The same, carrying the ask a permission prompt is about (T335).
    fn record_ask(&self, session: &str, ask_id: &str, verb: AskVerb, target: &str) {
        let state = HookState {
            version: gritty_agents::hook::HOOK_STATE_VERSION,
            session_id: session.to_owned(),
            provider: Provider::ClaudeCode,
            phase: HookPhase::Blocked,
            since_ms: gritty_agents::now_ms(),
            event: "PermissionRequest".to_owned(),
            tool: Some("Write".to_owned()),
            cwd: Some("/Users/a/p".to_owned()),
            ask: Some(HookAsk {
                ask_id: ask_id.to_owned(),
                verb,
                target: target.to_owned(),
                detail: None,
                diff: vec!["+fn main() {}".to_owned()],
                added: 1,
                removed: 0,
                outside_root: false,
                sensitive: false,
            }),
        };
        assert_eq!(
            gritty_agents::hook::write(&self.hook_dir(), &state),
            gritty_agents::WriteOutcome::Written
        );
    }

    fn verdict_for(&self, ask_id: &str) -> Option<gritty_agents::verdict::Verdict> {
        gritty_agents::verdict::take(&self.verdicts(), ask_id)
    }

    fn forget_phase(&self, session: &str) {
        let path = self.hook_dir().join(format!("{session}.json"));
        std::fs::remove_file(path).expect("remove hook state");
    }
}

impl Drop for TempTranscripts {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

impl RecordingSink {
    fn batches(&self) -> Vec<AgentTimelineAppendedPayload> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter_map(|envelope| match &envelope.event {
                CoreEvent::AgentTimelineAppended(batch) => Some(batch.clone()),
                _ => None,
            })
            .collect()
    }
}

/// A manager that reads `root` for transcripts and sees no processes, with both
/// loops effectively off so every tick in a test is one the test asked for.
fn manager_over(root: &TempTranscripts) -> (Arc<AgentSessionManager>, Arc<RecordingSink>) {
    manager_over_with(root, String::new())
}

/// The same, with a process table that reports one live agent — which is what
/// `BLOCKED` needs, since the absence of a process outranks it (T221).
fn manager_over_running(
    root: &TempTranscripts,
    ps: String,
) -> (Arc<AgentSessionManager>, Arc<RecordingSink>) {
    manager_over_with(root, ps)
}

/// A whole machine's worth of canned command output: the process table, and an
/// `lsof` that puts every candidate in the transcripts' working directory —
/// which is the join key the roster actually uses.
struct FakeMachine(String);

impl CommandRunner for FakeMachine {
    fn run(&self, program: &str, _args: &[&str], _timeout: Duration) -> Option<String> {
        match program {
            "/bin/ps" => Some(self.0.clone()),
            "/usr/sbin/lsof" => Some("fcwd\nn/Users/a/p\n".to_owned()),
            _ => None,
        }
    }
}

fn manager_over_with(
    root: &TempTranscripts,
    ps: String,
) -> (Arc<AgentSessionManager>, Arc<RecordingSink>) {
    let sink = Arc::new(RecordingSink::default());
    let events = EventBus::new(FanOutSink::new(vec![sink.clone()]));
    let handle = Handle::current();
    let registry = OperationRegistry::new(handle.clone(), events.clone());
    let workspaces = WorkspaceManager::new(handle.clone(), events.clone(), registry);
    let manager = AgentSessionManager::start(
        handle,
        events,
        workspaces,
        AgentDiscoveryConfig {
            scan: Some(gritty_agents::ScanOptions::rooted_at(root.0.clone())),
            runner: Arc::new(FakeMachine(ps)),
            poll_interval: Duration::from_secs(3_600),
            timeline_interval: Duration::from_secs(3_600),
            idle_after: DEFAULT_IDLE_AFTER,
            home: Some(PathBuf::from("/Users/someone")),
            permissions: Some(permission_paths(&root.hook_dir())),
            hooks: root.hooks(),
            hook_binary: None,
            notifier: None,
            ownership: None,
            owned: None,
        },
    );
    (manager, sink)
}

fn assistant_line(text: &str) -> String {
    format!(
        r#"{{"type":"assistant","sessionId":"{TEST_SESSION}","cwd":"/Users/a/p","message":{{"role":"assistant","content":[{{"type":"text","text":"{text}"}}]}}}}"#
    )
}

#[tokio::test]
async fn opening_a_timeline_returns_what_the_transcript_already_says() {
    let root = TempTranscripts::new("open");
    let id = root.seed(TEST_SESSION);
    let (manager, _sink) = manager_over(&root);

    let timeline = manager.open_timeline(&id).await.expect("open");
    manager.shutdown();

    assert_eq!(timeline.session_id, id);
    assert!(timeline.transcript_evidence);
    assert_eq!(timeline.dropped, 0);
    assert!(!timeline.restarted);
    assert!(matches!(
        &timeline.entries[0].kind,
        AgentTimelineKind::UserPrompt { text } if text == "ship the rail"
    ));
    // No workspace is open in this test, so a touched file is shown and not
    // openable — which is the same answer as one outside the root.
    assert!(matches!(
        &timeline.entries[1].kind,
        AgentTimelineKind::FileTouched { path: None, display } if display == "/Users/a/p/src/rail.rs"
    ));
}

#[tokio::test]
async fn following_delivers_only_what_was_appended_as_one_batch() {
    let root = TempTranscripts::new("append");
    let id = root.seed(TEST_SESSION);
    let (manager, sink) = manager_over(&root);
    manager.open_timeline(&id).await.expect("open");

    assert!(
        sink.batches().is_empty(),
        "the opening entries came back in the response, not as an event"
    );

    root.append(TEST_SESSION, &assistant_line("first"));
    root.append(TEST_SESSION, &assistant_line("second"));
    manager.follow_once().await;

    let batches = sink.batches();
    assert_eq!(
        batches.len(),
        1,
        "two records, one coalesced batch (rule 8)"
    );
    assert_eq!(batches[0].session_id, id);
    assert_eq!(batches[0].entries.len(), 2);
    assert_eq!(batches[0].dropped, 0);
    assert!(!batches[0].restarted);

    let timeline = manager.timeline(&id).await.expect("timeline");
    manager.shutdown();
    assert_eq!(timeline.entries.len(), 4, "the ring holds everything read");
    let seqs: Vec<u64> = timeline.entries.iter().map(|entry| entry.seq).collect();
    assert_eq!(seqs, vec![0, 1, 2, 3], "strictly increasing, appendable");
}

#[tokio::test]
async fn an_unchanged_transcript_emits_nothing() {
    let root = TempTranscripts::new("quiet");
    let id = root.seed(TEST_SESSION);
    let (manager, sink) = manager_over(&root);
    manager.open_timeline(&id).await.expect("open");

    manager.follow_once().await;
    manager.follow_once().await;
    manager.follow_once().await;
    manager.shutdown();

    assert!(
        sink.batches().is_empty(),
        "three ticks, nothing appended, no events at all"
    );
}

#[tokio::test]
async fn a_truncated_transcript_restarts_the_view_instead_of_splicing_it() {
    let root = TempTranscripts::new("truncate");
    let id = root.seed(TEST_SESSION);
    let (manager, sink) = manager_over(&root);
    manager.open_timeline(&id).await.expect("open");

    // A new session reused the file.
    std::fs::write(
        root.transcript(TEST_SESSION),
        format!("{}\n", assistant_line("a different file")),
    )
    .expect("replace");
    manager.follow_once().await;

    let batches = sink.batches();
    assert_eq!(batches.len(), 1);
    assert!(batches[0].restarted, "the UI is told its view was reset");

    let timeline = manager.timeline(&id).await.expect("timeline");
    manager.shutdown();
    assert!(timeline.restarted);
    assert_eq!(
        timeline.entries.len(),
        1,
        "the records from the replaced file are gone, not spliced onto"
    );
}

#[tokio::test]
async fn closing_a_timeline_stops_its_follower() {
    let root = TempTranscripts::new("close");
    let id = root.seed(TEST_SESSION);
    let (manager, _sink) = manager_over(&root);
    manager.open_timeline(&id).await.expect("open");

    let before = manager.follow_metrics().reads;
    manager.follow_once().await;
    let following = manager.follow_metrics().reads;
    assert_eq!(following, before + 1, "an open timeline is read");

    manager.close_timeline(&id).await.expect("close");
    root.append(TEST_SESSION, &assistant_line("nobody is listening"));
    manager.follow_once().await;
    manager.follow_once().await;

    assert_eq!(
        manager.follow_metrics().reads,
        following,
        "a closed timeline is not read again"
    );
    manager.shutdown();
}

#[tokio::test]
async fn opening_twice_returns_the_same_timeline_and_starts_no_second_follower() {
    let root = TempTranscripts::new("idempotent");
    let id = root.seed(TEST_SESSION);
    let (manager, _sink) = manager_over(&root);

    let first = manager.open_timeline(&id).await.expect("open");
    let again = manager.open_timeline(&id).await.expect("open again");
    assert_eq!(first, again);

    let before = manager.follow_metrics().reads;
    manager.follow_once().await;
    assert_eq!(
        manager.follow_metrics().reads,
        before + 1,
        "one follower, not two"
    );
    manager.shutdown();
}

#[tokio::test]
async fn a_timeline_for_a_session_that_is_not_in_the_roster_is_not_found() {
    let root = TempTranscripts::new("unknown");
    let (manager, _sink) = manager_over(&root);

    for error in [
        manager.open_timeline("claude_code:nope").await.unwrap_err(),
        manager.timeline("claude_code:nope").await.unwrap_err(),
        manager
            .close_timeline("claude_code:nope")
            .await
            .unwrap_err(),
    ] {
        assert_eq!(error.code, crate::protocol::error::CoreErrorCode::NotFound);
    }
    manager.shutdown();
}

#[tokio::test]
async fn closing_a_timeline_that_is_not_open_is_not_an_error() {
    let root = TempTranscripts::new("close-twice");
    let id = root.seed(TEST_SESSION);
    let (manager, _sink) = manager_over(&root);

    // A renderer that reloaded has no way to know what it left open.
    manager.close_timeline(&id).await.expect("never opened");
    manager.open_timeline(&id).await.expect("open");
    manager.close_timeline(&id).await.expect("close");
    manager.close_timeline(&id).await.expect("close again");
    manager.shutdown();
}

#[tokio::test]
async fn agents_timeline_reads_a_session_nobody_is_following() {
    let root = TempTranscripts::new("rehydrate");
    let id = root.seed(TEST_SESSION);
    let (manager, _sink) = manager_over(&root);

    let timeline = manager.timeline(&id).await.expect("timeline");
    assert_eq!(timeline.entries.len(), 2);
    assert!(timeline.transcript_evidence);

    // A read is a read: no follower was started by it.
    let before = manager.follow_metrics().reads;
    manager.follow_once().await;
    assert_eq!(manager.follow_metrics().reads, before);
    manager.shutdown();
}

#[tokio::test]
async fn a_session_with_no_transcript_has_a_timeline_that_says_so() {
    // Process evidence only: `ps` names a `claude`, nothing is on disk for it.
    let runner = Arc::new(FakeRunner(Some(" 42 1 ttys001  claude\n".to_owned())));
    let (manager, _sink) = manager(runner);

    let timeline = manager
        .open_timeline("claude_code:pid-42")
        .await
        .expect("open");
    assert!(timeline.entries.is_empty());
    assert!(
        !timeline.transcript_evidence,
        "nothing to read, which is not the same as the agent doing nothing"
    );

    // Following it costs no read at all.
    let before = manager.follow_metrics().reads;
    manager.follow_once().await;
    assert_eq!(manager.follow_metrics().reads, before);
    manager.shutdown();
}

#[tokio::test]
async fn only_a_bounded_number_of_timelines_may_be_open() {
    let root = TempTranscripts::new("bounded");
    let mut ids = Vec::new();
    for index in 0..MAX_OPEN_TIMELINES + 1 {
        ids.push(root.seed(&format!("11111111-2222-3333-4444-55555555555{index}")));
    }
    let (manager, _sink) = manager_over(&root);

    for id in ids.iter().take(MAX_OPEN_TIMELINES) {
        manager.open_timeline(id).await.expect("open");
    }
    let refused = manager
        .open_timeline(&ids[MAX_OPEN_TIMELINES])
        .await
        .unwrap_err();
    assert_eq!(
        refused.code,
        crate::protocol::error::CoreErrorCode::Conflict
    );

    // Closing one makes room again.
    manager.close_timeline(&ids[0]).await.expect("close");
    manager
        .open_timeline(&ids[MAX_OPEN_TIMELINES])
        .await
        .expect("room again");
    manager.shutdown();
}

#[tokio::test]
async fn shutdown_cancels_and_joins_both_loops() {
    let root = TempTranscripts::new("shutdown");
    let id = root.seed(TEST_SESSION);
    let (manager, _sink) = manager_over(&root);
    manager.open_timeline(&id).await.expect("open");

    // Returns only once both loops have left; no follower outlives the app.
    manager.shutdown_and_join().await;

    let after = manager.follow_metrics();
    root.append(TEST_SESSION, &assistant_line("after the end"));
    tokio::task::yield_now().await;
    assert_eq!(
        manager.follow_metrics(),
        after,
        "nothing runs after shutdown"
    );
}

// ===========================================================================
// The third source (T221)
//
// Everything here writes into a scratch directory. No test in this file reads
// or writes a real `~/.claude`, a real `~/.codex`, or the application-data
// directory the running application uses.
// ===========================================================================

/// A live `claude` process, so `BLOCKED` is reachable at all.
fn live_claude() -> String {
    " 42 1 ttys001  claude\n".to_owned()
}

#[tokio::test]
async fn hook_evidence_joins_by_session_id() {
    let root = TempTranscripts::new("hook-join");
    let id = root.seed(TEST_SESSION);
    let (manager, _sink) = manager_over(&root);

    // Without the hook, the roster is exactly what T220 shipped.
    let before = manager.refresh().await;
    let row = before
        .sessions
        .iter()
        .find(|s| s.id == id)
        .expect("the row");
    assert_eq!(row.evidence, AgentEvidence::TranscriptOnly);
    assert_ne!(row.activity, AgentActivity::Blocked);

    // The agent reports a prompt. There is still no live process, so `ENDED`
    // wins: an agent killed at a prompt is gone, not blocked (ADR-018 §7).
    root.record_phase(TEST_SESSION, HookPhase::Blocked, Some("Bash"));
    manager.follow_once().await;
    let after = manager.list().await;
    let row = after.sessions.iter().find(|s| s.id == id).expect("the row");
    assert_eq!(
        row.evidence,
        AgentEvidence::TranscriptAndHook,
        "the hook saw it, and the evidence says so"
    );
    assert_eq!(row.activity, AgentActivity::Ended);
    assert_eq!(row.activity_source, AgentActivitySource::Process);
    manager.shutdown();
}

#[tokio::test]
async fn a_state_file_for_an_unknown_session_creates_no_row() {
    let root = TempTranscripts::new("hook-orphan");
    let (manager, _sink) = manager_over(&root);
    root.record_phase("no-such-session", HookPhase::Blocked, Some("Bash"));

    manager.follow_once().await;
    let roster = manager.list().await;
    assert!(
        roster.sessions.is_empty(),
        "hook state is evidence about a session, not proof one exists"
    );
    manager.shutdown();
}

#[tokio::test]
async fn a_live_session_at_a_prompt_reads_blocked_and_says_the_hook_said_so() {
    let root = TempTranscripts::new("hook-blocked");
    let id = root.seed(TEST_SESSION);
    let (manager, sink) = manager_over_running(&root, live_claude());

    let before = manager.refresh().await;
    let row = before.sessions.iter().find(|s| s.id == id).expect("row");
    assert_eq!(row.evidence, AgentEvidence::Both);
    assert_eq!(row.activity, AgentActivity::Waiting);
    let emitted = sink.rosters().len();

    root.record_phase(TEST_SESSION, HookPhase::Blocked, Some("Bash"));
    manager.follow_once().await;

    let after = manager.list().await;
    let row = after.sessions.iter().find(|s| s.id == id).expect("row");
    assert_eq!(row.activity, AgentActivity::Blocked);
    assert_eq!(row.activity_source, AgentActivitySource::Hook);
    assert_eq!(row.evidence, AgentEvidence::All);
    assert_eq!(after.needs_you, 1);
    assert_eq!(
        sink.rosters().len(),
        emitted + 1,
        "the fast loop published the change, without a second process probe"
    );

    // Answering it clears the row again.
    root.record_phase(TEST_SESSION, HookPhase::Active, Some("Bash"));
    manager.follow_once().await;
    let cleared = manager.list().await;
    let row = cleared.sessions.iter().find(|s| s.id == id).expect("row");
    assert_ne!(row.activity, AgentActivity::Blocked);
    assert_eq!(row.evidence, AgentEvidence::All, "the hook still saw it");
    manager.shutdown();
}

#[tokio::test]
async fn a_hook_write_that_changes_no_verdict_emits_nothing() {
    // Rule 8, and the one the fast loop makes non-negotiable: hook state moves
    // far more often than the process table, and most of its moves change no
    // roster row at all.
    let root = TempTranscripts::new("hook-quiet");
    root.seed(TEST_SESSION);
    let (manager, sink) = manager_over_running(&root, live_claude());
    manager.refresh().await;
    // The *first* state file does change the row — its evidence now says the
    // hook saw this session — and publishing that is correct.
    root.record_phase(TEST_SESSION, HookPhase::Active, Some("Bash"));
    manager.follow_once().await;
    let emitted = sink.rosters().len();

    // From here the agent keeps reporting, and none of it changes a verdict.
    for phase in [HookPhase::Settled, HookPhase::Active, HookPhase::Settled] {
        root.record_phase(TEST_SESSION, phase, Some("Bash"));
        manager.follow_once().await;
    }
    assert_eq!(
        sink.rosters().len(),
        emitted,
        "three hook writes, no derived change, no events"
    );

    // And a tick with no hook write at all is free.
    manager.follow_once().await;
    assert_eq!(sink.rosters().len(), emitted);
    manager.shutdown();
}

#[tokio::test]
async fn the_prompt_becomes_a_permission_pending_entry_naming_what_is_waiting() {
    let root = TempTranscripts::new("hook-timeline");
    let id = root.seed(TEST_SESSION);
    // A tool call in flight: the transcript's own bounded preview of the very
    // call the prompt is about.
    root.append(
        TEST_SESSION,
        &format!(
            r#"{{"type":"assistant","sessionId":"{TEST_SESSION}","cwd":"/Users/a/p","message":{{"role":"assistant","content":[{{"type":"tool_use","id":"toolu_1","name":"Bash","input":{{"command":"cargo test"}}}}]}}}}"#
        ),
    );
    let (manager, sink) = manager_over_running(&root, live_claude());
    manager.refresh().await;
    manager.open_timeline(&id).await.expect("open");

    root.record_phase(TEST_SESSION, HookPhase::Blocked, Some("Bash"));
    manager.follow_once().await;

    let timeline = manager.timeline(&id).await.expect("timeline");
    let pending = timeline.pending.clone().expect("a pending prompt");
    assert_eq!(
        pending.kind,
        AgentTimelineKind::PermissionPending {
            tool: "Bash".to_owned(),
            detail: Some("cargo test".to_owned()),
        },
        "the hook names the tool; the transcript supplies the bounded detail"
    );
    let batch = sink.batches().last().cloned().expect("a batch");
    assert_eq!(batch.pending, Some(pending));
    assert!(
        batch.entries.is_empty(),
        "a batch for the prompt alone, with no transcript records in it"
    );

    // Answering it takes the entry away again — it is a state, not an event.
    root.record_phase(TEST_SESSION, HookPhase::Active, Some("Bash"));
    manager.follow_once().await;
    assert_eq!(
        manager.timeline(&id).await.expect("timeline").pending,
        None,
        "an answered prompt does not sit in the feed claiming to be waiting"
    );
    manager.shutdown();
}

#[tokio::test]
async fn a_permission_prompt_alone_does_not_re_emit_while_it_stands() {
    let root = TempTranscripts::new("hook-timeline-quiet");
    let id = root.seed(TEST_SESSION);
    let (manager, sink) = manager_over_running(&root, live_claude());
    manager.refresh().await;
    manager.open_timeline(&id).await.expect("open");
    root.record_phase(TEST_SESSION, HookPhase::Blocked, Some("Bash"));
    manager.follow_once().await;
    let batches = sink.batches().len();

    manager.follow_once().await;
    manager.follow_once().await;
    assert_eq!(
        sink.batches().len(),
        batches,
        "the prompt is unchanged, so nothing is said about it again (rule 8)"
    );
    manager.shutdown();
}

#[tokio::test]
async fn hook_state_removed_from_under_the_roster_clears_the_prompt() {
    let root = TempTranscripts::new("hook-swept");
    let id = root.seed(TEST_SESSION);
    let (manager, _sink) = manager_over_running(&root, live_claude());
    manager.refresh().await;
    root.record_phase(TEST_SESSION, HookPhase::Blocked, Some("Bash"));
    manager.follow_once().await;
    let blocked = manager.list().await;
    assert_eq!(
        blocked
            .sessions
            .iter()
            .find(|s| s.id == id)
            .expect("row")
            .activity,
        AgentActivity::Blocked
    );

    root.forget_phase(TEST_SESSION);
    manager.follow_once().await;
    let cleared = manager.list().await;
    let row = cleared.sessions.iter().find(|s| s.id == id).expect("row");
    assert_ne!(row.activity, AgentActivity::Blocked);
    assert_eq!(
        row.evidence,
        AgentEvidence::Both,
        "with the file gone, the hook no longer saw it either"
    );
    manager.shutdown();
}

#[tokio::test]
async fn with_nothing_installed_the_roster_is_what_t220_shipped() {
    let root = TempTranscripts::new("hook-absent");
    let id = root.seed(TEST_SESSION);
    // The hook directory is configured and will simply never exist, which is
    // what a machine with no integration installed looks like.
    let (manager, sink) = manager_over_running(&root, live_claude());
    let first = manager.refresh().await;
    for _ in 0..3 {
        manager.follow_once().await;
    }
    let after = manager.list().await;

    assert_eq!(first, after, "the fast loop changed nothing");
    assert_eq!(sink.rosters().len(), 1, "and published nothing more");
    let row = after.sessions.iter().find(|s| s.id == id).expect("row");
    assert_eq!(row.evidence, AgentEvidence::Both);
    assert_eq!(row.activity_source, AgentActivitySource::Process);
    manager.shutdown();
}

// ------------------------------------------------------------- integration

/// A scratch home carrying a `.claude/settings.json`, and the data directory
/// the installed command line would point the hook at. Never a real one.
struct TempHome(PathBuf);

impl TempHome {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "gritty-core-integration-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(path.join(".claude")).expect("create scratch home");
        Self(path)
    }

    fn config(&self) -> PathBuf {
        self.0.join(".claude").join("settings.json")
    }

    fn hook_binary(&self) -> PathBuf {
        let path = self.0.join("gritty-agent-hook");
        std::fs::write(&path, b"#!/bin/sh\n").expect("write hook binary");
        path
    }

    fn manager(&self, hook_binary: Option<PathBuf>) -> Arc<AgentSessionManager> {
        let sink = Arc::new(RecordingSink::default());
        let events = EventBus::new(FanOutSink::new(vec![sink]));
        let handle = Handle::current();
        let registry = OperationRegistry::new(handle.clone(), events.clone());
        let workspaces = WorkspaceManager::new(handle.clone(), events.clone(), registry);
        AgentSessionManager::start(
            handle,
            events,
            workspaces,
            AgentDiscoveryConfig {
                scan: None,
                runner: Arc::new(FakeRunner(Some(String::new()))),
                poll_interval: Duration::from_secs(3_600),
                timeline_interval: Duration::from_secs(3_600),
                idle_after: DEFAULT_IDLE_AFTER,
                home: Some(self.0.clone()),
                permissions: None,
                hooks: Some(gritty_agents::HookScanOptions::rooted_at(
                    self.0.join("data/agents/state"),
                )),
                hook_binary,
                notifier: None,
                ownership: None,
                owned: None,
            },
        )
    }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
async fn integration_status_reports_an_absent_config_as_not_installed() {
    let home = TempHome::new("status");
    let manager = home.manager(Some(home.hook_binary()));
    let status = manager
        .integration_status(AgentProvider::ClaudeCode)
        .await
        .expect("status");
    assert_eq!(status.status, AgentIntegrationStatus::NotInstalled);
    assert!(status.hook_available);
    assert!(
        status.config_path.ends_with(".claude/settings.json"),
        "{}",
        status.config_path
    );
    manager.shutdown();
}

#[tokio::test]
async fn a_preview_shows_the_change_and_writes_nothing() {
    let home = TempHome::new("preview");
    std::fs::write(
        home.config(),
        r#"{"model":"opus","hooks":{"Stop":[{"matcher":"*","hooks":[{"type":"command","command":"'/other/OtherHooks' --source claude"}]}]}}"#,
    )
    .expect("write config");
    let before = std::fs::read_to_string(home.config()).expect("read");
    let manager = home.manager(Some(home.hook_binary()));

    let plan = manager
        .integration_install(AgentProvider::ClaudeCode, false)
        .await
        .expect("plan");
    assert!(!plan.applied);
    assert!(!plan.creates_config);
    assert!(plan.backup_path.is_some(), "the backup it would make");
    assert!(plan.command.contains("gritty-agent-hook"));
    assert!(plan.command.contains("--state-dir"));
    let stop = plan
        .events
        .iter()
        .find(|event| event.event == "Stop")
        .expect("Stop");
    assert_eq!(
        stop.foreign_entries, 1,
        "the incumbent is counted, not hidden"
    );
    assert_eq!(
        std::fs::read_to_string(home.config()).expect("read"),
        before,
        "a preview writes nothing"
    );
    manager.shutdown();
}

#[tokio::test]
async fn installing_then_uninstalling_leaves_the_other_tool_alone() {
    let home = TempHome::new("roundtrip");
    let original = r#"{"model":"opus","hooks":{"PreToolUse":[{"matcher":"*","hooks":[{"type":"command","command":"'/other/OtherHooks' --source claude"}]}]}}"#;
    std::fs::write(home.config(), original).expect("write config");
    let manager = home.manager(Some(home.hook_binary()));

    let installed = manager
        .integration_install(AgentProvider::ClaudeCode, true)
        .await
        .expect("install");
    assert!(installed.applied);
    assert_eq!(installed.status, AgentIntegrationStatus::Installed);
    assert_eq!(
        manager
            .integration_status(AgentProvider::ClaudeCode)
            .await
            .expect("status")
            .status,
        AgentIntegrationStatus::Installed
    );

    manager
        .integration_uninstall(AgentProvider::ClaudeCode, true)
        .await
        .expect("uninstall");
    let after: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(home.config()).expect("read"))
            .expect("parse");
    let before: serde_json::Value = serde_json::from_str(original).expect("parse");
    assert_eq!(after, before, "back to exactly where it started");
    manager.shutdown();
}

#[tokio::test]
async fn a_malformed_config_is_refused_with_its_parse_error() {
    let home = TempHome::new("malformed");
    std::fs::write(home.config(), "{ oops").expect("write config");
    let manager = home.manager(Some(home.hook_binary()));

    let status = manager
        .integration_status(AgentProvider::ClaudeCode)
        .await
        .expect("status");
    assert_eq!(status.status, AgentIntegrationStatus::UnsupportedConfig);
    assert!(status.detail.is_some());

    let refused = manager
        .integration_install(AgentProvider::ClaudeCode, true)
        .await
        .expect_err("refused");
    assert_eq!(
        refused.code,
        crate::protocol::error::CoreErrorCode::Conflict
    );
    assert_eq!(
        std::fs::read_to_string(home.config()).expect("read"),
        "{ oops",
        "never rewritten, never repaired (ADR-018 §4)"
    );
    manager.shutdown();
}

#[tokio::test]
async fn a_build_that_cannot_find_its_own_hook_binary_installs_nothing() {
    let home = TempHome::new("no-binary");
    let manager = home.manager(None);
    let status = manager
        .integration_status(AgentProvider::ClaudeCode)
        .await
        .expect("status");
    assert!(!status.hook_available, "so the UI offers nothing");

    let refused = manager
        .integration_install(AgentProvider::ClaudeCode, true)
        .await
        .expect_err("refused");
    assert_eq!(refused.code, crate::protocol::error::CoreErrorCode::Io);
    assert!(
        !home.config().exists(),
        "and no entry that would be broken the moment it was written"
    );
    manager.shutdown();
}

// ---------------------------------------------------------------------------
// Attention notifications (T223)
//
// The policy itself is tested in `attention.rs`, without a manager or a clock.
// What is tested here is the *wiring*: that the manager arms nothing by
// itself, that a refusal leaves it off, and that a roster transition observed
// by the real publish path reaches the platform exactly once.
// ---------------------------------------------------------------------------

/// A `ps` whose output the test changes between scans, so a roster can
/// actually transition.
struct SwitchableRunner(Mutex<String>);

impl SwitchableRunner {
    fn new(initial: &str) -> Self {
        Self(Mutex::new(initial.to_owned()))
    }

    fn set(&self, ps: &str) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = ps.to_owned();
    }
}

impl CommandRunner for SwitchableRunner {
    fn run(&self, program: &str, _args: &[&str], _timeout: Duration) -> Option<String> {
        (program == "/bin/ps").then(|| {
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        })
    }
}

struct RecordingNotifier {
    answer: AgentNotificationPermission,
    asked: Mutex<u32>,
    shown: Mutex<Vec<AttentionAlert>>,
}

impl RecordingNotifier {
    fn answering(answer: AgentNotificationPermission) -> Arc<Self> {
        Arc::new(Self {
            answer,
            asked: Mutex::new(0),
            shown: Mutex::new(Vec::new()),
        })
    }

    fn shown(&self) -> Vec<AttentionAlert> {
        self.shown
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn asked(&self) -> u32 {
        *self.asked.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl AttentionNotifier for RecordingNotifier {
    fn supported(&self) -> bool {
        true
    }

    fn permission(&self) -> AgentNotificationPermission {
        // Reading the state never counts as asking.
        self.answer
    }

    fn request_permission(&self) -> AgentNotificationPermission {
        *self.asked.lock().unwrap_or_else(PoisonError::into_inner) += 1;
        self.answer
    }

    fn show(&self, alert: &AttentionAlert) {
        self.shown
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(alert.clone());
    }
}

const ONE_CLAUDE: &str = " 42 1 ttys001  claude\n";

#[tokio::test]
async fn notifications_are_off_until_the_user_asks_and_asking_is_what_prompts() {
    let notifier = RecordingNotifier::answering(AgentNotificationPermission::Granted);
    let (manager, _sink) = manager_with(
        Arc::new(FakeRunner(Some(ONE_CLAUDE.to_owned()))),
        None,
        Some(notifier.clone()),
    );

    let settings = manager.notification_settings();
    assert!(!settings.enabled, "off by default");
    assert!(settings.supported);
    assert_eq!(
        notifier.asked(),
        0,
        "reading the setting must never produce a permission prompt"
    );

    let enabled = manager.set_notifications(true);
    assert!(enabled.enabled);
    assert_eq!(notifier.asked(), 1, "the first enable is what asks");
    assert!(manager.notification_settings().enabled);

    manager.set_notifications(false);
    assert!(!manager.notification_settings().enabled);
    assert_eq!(notifier.asked(), 1, "turning it off asks nobody");
    manager.shutdown();
}

#[tokio::test]
async fn a_refusal_leaves_the_setting_off_and_is_reported_rather_than_retried() {
    let notifier = RecordingNotifier::answering(AgentNotificationPermission::Denied);
    let (manager, _sink) = manager_with(
        Arc::new(FakeRunner(Some(ONE_CLAUDE.to_owned()))),
        None,
        Some(notifier.clone()),
    );

    let settings = manager.set_notifications(true);
    assert!(!settings.enabled);
    assert_eq!(settings.permission, AgentNotificationPermission::Denied);
    assert!(settings.supported);
    // The roster is unaffected by any of this.
    assert_eq!(manager.list().await.sessions.len(), 1);
    assert!(notifier.shown().is_empty());
    manager.shutdown();
}

#[tokio::test]
async fn a_build_without_a_notifier_reports_the_setting_as_unsupported() {
    let (manager, _sink) = manager(Arc::new(FakeRunner(Some(ONE_CLAUDE.to_owned()))));
    let settings = manager.notification_settings();
    assert!(!settings.supported, "the setting is absent, not broken");
    assert!(!settings.enabled);
    // And enabling it changes nothing rather than pretending to work.
    assert!(!manager.set_notifications(true).enabled);
    manager.shutdown();
}

#[tokio::test]
async fn a_session_that_arrives_needing_you_notifies_once_and_not_per_poll() {
    let runner = Arc::new(SwitchableRunner::new(""));
    let notifier = RecordingNotifier::answering(AgentNotificationPermission::Granted);
    let (manager, _sink) = manager_with(runner.clone(), None, Some(notifier.clone()));
    manager.set_window_focused(false);
    assert!(manager.set_notifications(true).enabled);

    // The first roster is what Gritty has just noticed; it notifies nothing.
    assert!(manager.list().await.sessions.is_empty());
    assert!(notifier.shown().is_empty());

    runner.set(ONE_CLAUDE);
    let roster = manager.refresh().await;
    assert_eq!(roster.sessions[0].activity, AgentActivity::Waiting);
    let shown = notifier.shown();
    assert_eq!(shown.len(), 1, "{shown:?}");
    assert_eq!(shown[0].session_id, "claude_code:pid-42");

    // Still waiting on the next two polls: the roster is polled, and this is
    // exactly the level-triggered notification rule 8 forbids.
    manager.refresh().await;
    manager.refresh().await;
    assert_eq!(notifier.shown().len(), 1);
    manager.shutdown();
}

#[tokio::test]
async fn nothing_is_raised_while_the_window_has_focus() {
    let runner = Arc::new(SwitchableRunner::new(""));
    let notifier = RecordingNotifier::answering(AgentNotificationPermission::Granted);
    let (manager, _sink) = manager_with(runner.clone(), None, Some(notifier.clone()));
    assert!(manager.set_notifications(true).enabled);

    manager.list().await;
    runner.set(ONE_CLAUDE);
    manager.refresh().await;
    assert!(
        notifier.shown().is_empty(),
        "a manager starts focused, and while it is focused the island is the notification"
    );
    manager.shutdown();
}

// ---------------------------------------------------------------------------
// Permissions (T335, ADR-020)
//
// The one place this module acts rather than reports. What these assert is not
// that a verdict file appears — the hook's own tests do that end to end — but
// the domain rules around it: that an ask reaches the roster where a background
// agent's prompt is visible, that answering removes it everywhere at once, and
// that "for this run" means exactly that.
// ---------------------------------------------------------------------------

/// A live session with a prompt the hook described.
async fn asking(
    root: &TempTranscripts,
    ask_id: &str,
    target: &str,
) -> (Arc<AgentSessionManager>, String) {
    let id = root.seed(TEST_SESSION);
    let (manager, _sink) = manager_over_running(root, live_claude());
    manager.refresh().await;
    root.record_ask(TEST_SESSION, ask_id, AskVerb::WriteFile, target);
    manager.follow_once().await;
    (manager, id)
}

#[tokio::test]
async fn an_ask_reaches_the_roster_so_an_unwatched_agent_is_still_visible() {
    // The structural reason this lives on the session and not only on the
    // timeline: only one timeline is followed at a time, and artboard 12b is
    // about the agents nobody is watching.
    let root = TempTranscripts::new("ask-roster");
    let (manager, id) = asking(
        &root,
        "ask-1",
        "/Users/someone/p/crates/terminal/src/pty.rs",
    )
    .await;

    let roster = manager.list().await;
    let row = roster
        .sessions
        .iter()
        .find(|s| s.id == id)
        .expect("the row");
    let pending = row.pending.clone().expect("the ask");
    assert_eq!(pending.ask_id, "ask-1");
    assert_eq!(pending.verb, AgentPermissionVerb::WriteFile);
    assert_eq!(
        pending.target, "~/p/crates/terminal/src/pty.rs",
        "shortened by the core, because the renderer has no home directory"
    );
    assert_eq!(
        pending.suggested_scope.as_deref(),
        Some("~/p/crates/terminal/src"),
        "and it can say what \"for this run\" would cover"
    );
    manager.shutdown();
}

#[tokio::test]
async fn answering_writes_the_verdict_the_hook_is_waiting_for_and_clears_the_row() {
    let root = TempTranscripts::new("ask-answer");
    let (manager, id) = asking(&root, "ask-1", "/Users/someone/p/src/main.rs").await;

    let roster = manager
        .answer_permission("ask-1", AgentPermissionVerdict::Deny)
        .await;
    let verdict = root.verdict_for("ask-1").expect("a verdict on disk");
    assert_eq!(verdict.behavior, gritty_agents::verdict::Behavior::Deny);
    assert!(
        verdict.message.is_some(),
        "a denial says who decided, because the message reaches the model"
    );

    let row = roster
        .sessions
        .iter()
        .find(|s| s.id == id)
        .expect("the row");
    assert_eq!(
        row.pending, None,
        "the card, the row and the count go together"
    );
    assert_eq!(roster.recent_answers.len(), 1);
    assert_eq!(
        roster.recent_answers[0].grant_id, None,
        "a denial grants nothing, so there is nothing to undo"
    );
    manager.shutdown();
}

#[tokio::test]
async fn allowing_for_the_run_answers_the_next_ask_without_anyone_seeing_it() {
    let root = TempTranscripts::new("ask-grant");
    let (manager, id) = asking(&root, "ask-1", "/Users/someone/p/src/main.rs").await;
    manager
        .answer_permission("ask-1", AgentPermissionVerdict::AllowForRun)
        .await;
    assert!(root.verdict_for("ask-1").is_some());

    // The agent asks again, in the same place. Nobody is troubled by it.
    root.record_ask(
        TEST_SESSION,
        "ask-2",
        AskVerb::WriteFile,
        "/Users/someone/p/src/lib.rs",
    );
    manager.follow_once().await;
    let roster = manager.list().await;
    let row = roster
        .sessions
        .iter()
        .find(|s| s.id == id)
        .expect("the row");
    assert_eq!(row.pending, None, "the grant answered it");
    assert_eq!(
        root.verdict_for("ask-2").expect("a verdict").behavior,
        gritty_agents::verdict::Behavior::Allow
    );

    let grants = manager.permission_grants().grants;
    assert_eq!(grants.len(), 1);
    assert_eq!(grants[0].used, 1, "and the standing rule says how often");
    manager.shutdown();
}

#[tokio::test]
async fn undoing_a_grant_brings_the_question_back() {
    let root = TempTranscripts::new("ask-undo");
    let (manager, id) = asking(&root, "ask-1", "/Users/someone/p/src/main.rs").await;
    let roster = manager
        .answer_permission("ask-1", AgentPermissionVerdict::AllowForRun)
        .await;
    let grant_id = roster.recent_answers[0]
        .grant_id
        .clone()
        .expect("an undoable answer");

    manager.revoke_grant(&grant_id).await;
    assert!(manager.permission_grants().grants.is_empty());

    root.record_ask(
        TEST_SESSION,
        "ask-2",
        AskVerb::WriteFile,
        "/Users/someone/p/src/lib.rs",
    );
    manager.follow_once().await;
    let roster = manager.list().await;
    let row = roster
        .sessions
        .iter()
        .find(|s| s.id == id)
        .expect("the row");
    assert_eq!(
        row.pending.as_ref().map(|ask| ask.ask_id.as_str()),
        Some("ask-2"),
        "revoking a grant is what makes it a question again"
    );
    assert!(root.verdict_for("ask-2").is_none());
    manager.shutdown();
}

#[tokio::test]
async fn an_ask_answered_in_the_terminal_simply_goes() {
    // ADR-020: both answer paths are live at once and the first one wins. The
    // prompt vanishing without Gritty's verdict is normal, not an error.
    let root = TempTranscripts::new("ask-elsewhere");
    let (manager, id) = asking(&root, "ask-1", "/Users/someone/p/src/main.rs").await;

    root.record_phase(TEST_SESSION, HookPhase::Active, Some("Write"));
    manager.follow_once().await;
    let roster = manager.list().await;
    let row = roster
        .sessions
        .iter()
        .find(|s| s.id == id)
        .expect("the row");
    assert_eq!(row.pending, None);

    // And answering it now is not an error, it is a no-op with a fresh roster.
    let roster = manager
        .answer_permission("ask-1", AgentPermissionVerdict::AllowOnce)
        .await;
    assert!(root.verdict_for("ask-1").is_none(), "nothing was written");
    assert!(roster.recent_answers.is_empty());
    manager.shutdown();
}

#[tokio::test]
async fn the_liveness_marker_is_what_lets_a_hook_wait_at_all() {
    // Without it every hook returns instantly and no prompt is ever answerable
    // in Gritty, so its absence is a silent, total loss of the feature —
    // exactly the kind of thing worth one test.
    let root = TempTranscripts::new("marker");
    root.seed(TEST_SESSION);
    let (manager, _sink) = manager_over_running(&root, live_claude());
    let marker = root.0.join("live");
    assert!(!marker.exists(), "nothing before the loop has run");

    manager.follow_once().await;
    assert!(
        gritty_agents::verdict::is_live(&marker, gritty_agents::now_ms()),
        "the fast loop refreshes it"
    );
    manager.shutdown();
}

// ------------------------------------------- T326: violations, through a tick
use crate::protocol::ids::CorrelationId;

impl TempTranscripts {
    /// [`Self::seed`], for a session whose working directory is a real path on
    /// this machine — which is what makes a touched file resolve *inside* an
    /// open workspace, and so what makes it checkable at all (T326).
    fn seed_in(&self, session: &str, cwd: &str) -> String {
        let opening = format!(
            r#"{{"type":"user","cwd":"{cwd}","sessionId":"{session}","timestamp":"2026-09-05T13:49:47.770Z","message":{{"role":"user","content":[{{"type":"text","text":"ship the rail"}}]}}}}"#
        );
        std::fs::write(self.transcript(session), format!("{opening}\n")).expect("write transcript");
        format!("claude_code:{session}")
    }
}

//
// The detector itself is unit-tested in `ownership::tests`, against leases,
// links and paths. What is only provable here is the wiring: that a real tick
// over a real transcript reaches the registry at all, and that what it hands
// over is what `absorb` actually kept.

/// Everything one violation needs: a workspace on disk, a session database, a
/// lease held by one workstream, and a transcript whose working directory *is*
/// that workspace — which is what makes `FileTouched` carry a path at all.
struct ViolationRig {
    _db: gritty_persistence::testing::TempDir,
    _work: gritty_persistence::testing::TempDir,
    transcripts: TempTranscripts,
    manager: Arc<AgentSessionManager>,
    ownership: Arc<crate::ownership::OwnershipManager>,
    workspace: crate::workspace::Workspace,
    sink: Arc<RecordingSink>,
    session: String,
}

impl ViolationRig {
    /// `holder` claims `target`; `toucher` is linked to the session that will
    /// write. Both workstreams exist before the first tick.
    async fn new(name: &str) -> Self {
        use crate::ownership::{AcquireOwnershipRequest, OwnershipManager, OwnershipTarget};
        use crate::workspace::UnsavedPolicy;
        use crate::workstream::{CreateWorkstreamRequest, LinkAgentRequest, WorkstreamManager};
        use gritty_persistence::testing::TempDir;
        use gritty_persistence::{DB_FILE_NAME, Database};
        use gritty_workspace::WorkspacePath;

        let db = TempDir::new(&format!("t326-db-{name}"));
        let work = TempDir::new(&format!("t326-ws-{name}"));
        std::fs::create_dir_all(work.join("src/auth")).expect("seed the workspace");
        std::fs::write(work.join("src/auth/session.ts"), "// hi\n").expect("seed a file");
        let database = Arc::new(
            Database::open(&db.join(DB_FILE_NAME))
                .expect("open the test database")
                .database,
        );

        let sink = Arc::new(RecordingSink::default());
        let events = EventBus::new(FanOutSink::new(vec![sink.clone()]));
        let handle = Handle::current();
        let registry = OperationRegistry::new(handle.clone(), events.clone());
        let workspaces = WorkspaceManager::new(handle.clone(), events.clone(), registry);
        let workspace = workspaces
            .open(
                work.path(),
                UnsavedPolicy::Refuse,
                Actor::User,
                CorrelationId::new(),
            )
            .expect("open the workspace");
        let workstreams = WorkstreamManager::new(
            handle.clone(),
            events.clone(),
            workspaces.clone(),
            Some(database.clone()),
        );
        let ownership = OwnershipManager::new(
            handle.clone(),
            events.clone(),
            workspaces.clone(),
            Some(database),
        );

        // The canonical root, which is what the workspace opened as and
        // therefore what a transcript has to say for its paths to resolve.
        let cwd = workspace.root.clone();
        let transcripts = TempTranscripts::new(&format!("t326-{name}"));
        let session = transcripts.seed_in(TEST_SESSION, &cwd);

        let holder = workstreams
            .create(
                CreateWorkstreamRequest {
                    workspace_id: workspace.id.clone(),
                    title: "Authentication".to_owned(),
                    goal: None,
                },
                Actor::User,
                CorrelationId::new(),
            )
            .await
            .expect("create the holder");
        let toucher = workstreams
            .create(
                CreateWorkstreamRequest {
                    workspace_id: workspace.id.clone(),
                    title: "Payments".to_owned(),
                    goal: None,
                },
                Actor::User,
                CorrelationId::new(),
            )
            .await
            .expect("create the toucher");
        ownership
            .acquire(
                AcquireOwnershipRequest {
                    workspace_id: workspace.id.clone(),
                    workstream_id: holder.id.clone(),
                    target: OwnershipTarget::Directory {
                        path: WorkspacePath::parse("src/auth").expect("path"),
                    },
                    expires_at: None,
                    note: None,
                },
                Actor::User,
                CorrelationId::new(),
            )
            .await
            .expect("claim src/auth");
        workstreams
            .link_agent(
                LinkAgentRequest {
                    workspace_id: workspace.id.clone(),
                    workstream_id: toucher.id.clone(),
                    provider: AgentProvider::ClaudeCode,
                    session_id: session.clone(),
                },
                CorrelationId::new(),
            )
            .await
            .expect("link the session");

        let manager = AgentSessionManager::start(
            handle,
            events,
            workspaces,
            AgentDiscoveryConfig {
                scan: Some(gritty_agents::ScanOptions::rooted_at(transcripts.0.clone())),
                runner: Arc::new(FakeMachine(String::new())),
                poll_interval: Duration::from_secs(3_600),
                timeline_interval: Duration::from_secs(3_600),
                idle_after: DEFAULT_IDLE_AFTER,
                home: Some(PathBuf::from("/Users/someone")),
                permissions: None,
                hooks: None,
                hook_binary: None,
                notifier: None,
                ownership: Some(ownership.clone()),
                owned: None,
            },
        );

        Self {
            _db: db,
            _work: work,
            transcripts,
            manager,
            ownership,
            workspace,
            sink,
            session,
        }
    }

    async fn conflicts(&self) -> Vec<crate::ownership::OwnershipConflict> {
        self.ownership
            .list(
                crate::ownership::ListOwnershipRequest {
                    workspace_id: self.workspace.id.clone(),
                },
                CorrelationId::new(),
            )
            .await
            .expect("list ownership")
            .conflicts
    }

    /// What the agent's transcript says when it writes a file, with the working
    /// directory this rig's workspace actually opened at.
    fn touch_line(&self, tracking: &str) -> String {
        format!(
            r#"{{"type":"file-history-delta","sessionId":"{TEST_SESSION}","cwd":"{}","trackingPath":"{tracking}"}}"#,
            self.workspace.root
        )
    }

    fn ownership_events(&self) -> usize {
        self.sink
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|envelope| matches!(envelope.event, CoreEvent::OwnershipChanged(_)))
            .count()
    }
}

/// The integration case: one fake transcript, one `FileTouched` inside a lease
/// another workstream holds, one real tick — one persisted `OBSERVED` conflict
/// and one `OwnershipChanged`.
#[tokio::test]
async fn one_tick_records_a_violation_and_says_so_once() {
    let rig = ViolationRig::new("tick").await;
    rig.manager
        .open_timeline(&rig.session)
        .await
        .expect("open the timeline");
    let before = rig.ownership_events();

    rig.transcripts
        .append(TEST_SESSION, &rig.touch_line("src/auth/session.ts"));
    rig.manager.follow_once().await;

    let batches = rig.sink.batches();
    assert_eq!(batches.len(), 1, "one tick, one batch");
    assert!(
        matches!(
            &batches[0].entries[0].kind,
            AgentTimelineKind::FileTouched { path: Some(path), .. }
                if path.as_str() == "src/auth/session.ts"
        ),
        "the path resolved inside the open workspace, or there is nothing to check"
    );

    let observed: Vec<_> = rig
        .conflicts()
        .await
        .into_iter()
        .filter(|conflict| conflict.state == crate::ownership::ConflictState::Observed)
        .collect();
    assert_eq!(observed.len(), 1, "one violation, persisted");
    assert_eq!(
        rig.ownership_events() - before,
        1,
        "and said once, not once per tick"
    );

    // A second tick with nothing new writes nothing and says nothing.
    let after = rig.ownership_events();
    rig.manager.follow_once().await;
    assert_eq!(rig.ownership_events(), after);
    rig.manager.shutdown();
    assert_eq!(rig.conflicts().await.len(), 1);
}

/// A file the agent touched *outside* any lease is not a violation, and the
/// tick that carried it still costs nothing.
#[tokio::test]
async fn a_touch_outside_every_lease_records_nothing() {
    let rig = ViolationRig::new("free").await;
    rig.manager
        .open_timeline(&rig.session)
        .await
        .expect("open the timeline");
    let before = rig.ownership_events();

    rig.transcripts
        .append(TEST_SESSION, &rig.touch_line("README.md"));
    rig.manager.follow_once().await;
    rig.manager.shutdown();

    assert!(rig.conflicts().await.is_empty());
    assert_eq!(rig.ownership_events(), before);
}

/// The ring-overflow row of the failure table, asserted rather than reasoned
/// about: when one tick appends more than the ring holds, `absorb` truncates
/// `undelivered` *before* the batch is emitted, so the oldest touches are never
/// handed to the detector at all. That is a real gap — a file written and never
/// checked — and this pins what the detector actually saw.
#[tokio::test]
async fn a_tick_over_the_ring_checks_only_what_survived_it() {
    let rig = ViolationRig::new("ring").await;
    rig.manager
        .open_timeline(&rig.session)
        .await
        .expect("open the timeline");

    // Comfortably more than the ring, each one a distinct file inside the
    // lease, so every survivor is its own violation.
    let overflow = MAX_TIMELINE_ENTRIES + 40;
    for index in 0..overflow {
        rig.transcripts.append(
            TEST_SESSION,
            &rig.touch_line(&format!("src/auth/f{index}.ts")),
        );
    }
    rig.manager.follow_once().await;

    let batches = rig.sink.batches();
    assert_eq!(batches.len(), 1);
    let batch = &batches[0];
    assert!(batch.dropped > 0, "the ring reported the loss (T230)");
    let delivered = touched_paths(&batch.entries);
    assert_eq!(
        delivered.len(),
        MAX_TIMELINE_ENTRIES,
        "the detector sees the batch, and the batch is the ring"
    );
    assert!(
        !delivered
            .iter()
            .any(|path| path.as_str() == "src/auth/f0.ts"),
        "the oldest touch was pruned before the batch existed, so it is never checked"
    );

    let conflicts = rig.conflicts().await;
    rig.manager.shutdown();
    assert!(
        conflicts
            .iter()
            .all(|conflict| conflict.state == crate::ownership::ConflictState::Observed),
        "every row this tick produced is a violation"
    );
    // The map is bounded at `MAX_CONFLICTS` on the *read*, which is T325's
    // bound and unmodified here: 500 surviving touches wrote 500 rows and the
    // snapshot shows the newest 200 of them. That bound is now reachable by an
    // agent rather than only by a user asking 200 times, which is the honest
    // cost of reusing the row (T326 §6) and is recorded as a follow-up.
    assert_eq!(conflicts.len(), crate::ownership::MAX_CONFLICTS);
    assert!(
        !conflicts.iter().any(|conflict| conflict.target
            == crate::ownership::OwnershipTarget::File {
                path: gritty_workspace::WorkspacePath::parse("src/auth/f0.ts").expect("path"),
            }),
        "the pruned touch produced no row, because the detector never saw it"
    );
}

/// A build with no ownership registry behaves exactly as T230 did: the rail is
/// unchanged and the check is the only thing that does not happen.
#[tokio::test]
async fn without_a_registry_the_rail_is_unchanged() {
    let root = TempTranscripts::new("t326-none");
    let id = root.seed(TEST_SESSION);
    let (manager, sink) = manager_over(&root);
    manager.open_timeline(&id).await.expect("open");

    root.append(TEST_SESSION, &assistant_line("still fine"));
    manager.follow_once().await;
    manager.shutdown();

    assert_eq!(sink.batches().len(), 1);
}

/// What a batch hands to the detector, and what it deliberately does not.
///
/// A `FileTouched` with no path is a file outside the open workspace by
/// construction (T230, ADR-010): there is no `WorkspacePath` for it, so there
/// is nothing in the registry it could be compared with. `display` is a string
/// for a human and is never matched on — matching it would be matching
/// something that was never a path (rule 13).
#[test]
fn only_a_resolved_touched_path_is_checked() {
    fn entry(seq: u64, kind: AgentTimelineKind) -> AgentTimelineEntry {
        AgentTimelineEntry {
            seq,
            at: None,
            kind,
            sidechain: false,
        }
    }
    fn touched(raw: Option<&str>, display: &str) -> AgentTimelineKind {
        AgentTimelineKind::FileTouched {
            path: raw.map(|value| WorkspacePath::parse(value).expect("valid path")),
            display: display.to_owned(),
        }
    }

    let entries = vec![
        entry(
            1,
            AgentTimelineKind::AssistantText {
                text: "src/auth/session.ts".to_owned(),
            },
        ),
        entry(
            2,
            touched(Some("src/auth/session.ts"), "src/auth/session.ts"),
        ),
        // Outside the open workspace: shown, never resolved, never checked.
        entry(3, touched(None, "/Users/a/elsewhere/secrets.env")),
        // The same file again in one batch is still one path to check.
        entry(
            4,
            touched(Some("src/auth/session.ts"), "src/auth/session.ts"),
        ),
        entry(5, touched(Some("src/auth/token.ts"), "src/auth/token.ts")),
        entry(
            6,
            AgentTimelineKind::ToolCall {
                tool_use_id: None,
                name: "Write".to_owned(),
                detail: Some("src/auth/other.ts".to_owned()),
            },
        ),
    ];

    let checked = touched_paths(&entries);
    let paths: Vec<&str> = checked.iter().map(WorkspacePath::as_str).collect();
    assert_eq!(paths, ["src/auth/session.ts", "src/auth/token.ts"]);
    assert!(touched_paths(&[]).is_empty());
}
