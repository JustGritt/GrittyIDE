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
            timeline_interval: Duration::from_secs(3_600),
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
            runner: Arc::new(FakeRunner(Some(String::new()))),
            poll_interval: Duration::from_secs(3_600),
            timeline_interval: Duration::from_secs(3_600),
            idle_after: DEFAULT_IDLE_AFTER,
            home: Some(PathBuf::from("/Users/someone")),
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
