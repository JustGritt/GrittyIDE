//! T220 tests. Nothing here spawns a process or reads a real transcript: the
//! command runner is injected and the transcript root is a scratch directory.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

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

#[test]
fn an_outstanding_tool_call_is_working() {
    let session = discovered(Evidence::Both, NOW_MS, Some("Bash"));
    assert_eq!(
        activity_of(&session, NOW_MS, DEFAULT_IDLE_AFTER, true),
        AgentActivity::Working
    );
}

#[test]
fn a_live_session_with_nothing_outstanding_is_waiting_on_the_user() {
    let session = discovered(Evidence::Both, NOW_MS, None);
    assert_eq!(
        activity_of(&session, NOW_MS, DEFAULT_IDLE_AFTER, true),
        AgentActivity::Waiting
    );
    assert!(AgentActivity::Waiting.needs_you());
}

#[test]
fn a_quiet_live_session_goes_idle_and_stops_asking_for_attention() {
    let session = discovered(Evidence::Both, NOW_MS - 30 * MINUTE_MS, None);
    assert_eq!(
        activity_of(&session, NOW_MS, DEFAULT_IDLE_AFTER, true),
        AgentActivity::Idle
    );
    assert!(!AgentActivity::Idle.needs_you());
}

#[test]
fn a_stale_transcript_outranks_an_outstanding_tool_call() {
    // A call with no result for half an hour is abandoned, not working.
    let session = discovered(Evidence::Both, NOW_MS - 30 * MINUTE_MS, Some("Bash"));
    assert_eq!(
        activity_of(&session, NOW_MS, DEFAULT_IDLE_AFTER, true),
        AgentActivity::Idle
    );
}

#[test]
fn no_process_means_ended() {
    let session = discovered(Evidence::TranscriptOnly, NOW_MS, Some("Bash"));
    assert_eq!(
        activity_of(&session, NOW_MS, DEFAULT_IDLE_AFTER, true),
        AgentActivity::Ended
    );
}

#[test]
fn a_blind_scan_reports_ended_rather_than_guessing() {
    // The process table was unreadable. Everything reads ENDED; the roster's
    // `process_evidence` flag is what says why.
    let session = discovered(Evidence::Both, NOW_MS, Some("Bash"));
    assert_eq!(
        activity_of(&session, NOW_MS, DEFAULT_IDLE_AFTER, false),
        AgentActivity::Ended
    );
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
            idle_after: DEFAULT_IDLE_AFTER,
            home: Some(PathBuf::from("/Users/someone")),
        },
    );
    (manager, sink)
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
