//! Discovery of local coding-agent sessions (T220).
//!
//! This crate is an *adapter*, not domain: it reads evidence the machine
//! already produces and reports what it found. It starts nothing, stops
//! nothing, answers nothing and writes nothing — not to a session, not to an
//! agent's configuration. `gritty-core` translates what comes out of here into
//! the Gritty domain type that crosses the protocol (ADR-005: raw provider
//! output is never domain state).
//!
//! Two independent sources, deliberately kept separable because each is
//! blind where the other sees:
//!
//! | Source | Sees | Blind to |
//! |---|---|---|
//! | [`probe`] (`ps`, `lsof`) | that a process is alive, its terminal and root | what it is doing |
//! | [`transcript`] (session `.jsonl`) | the task, model, branch, tool in flight | whether it is still running |
//!
//! A session found by only one is still reported, with [`Evidence`] saying
//! which — an honest partial answer beats a confident invented one.
//!
//! A **third source** arrives with T221, and it is the only one the agent has
//! to cooperate with. Nothing in a transcript or a process table distinguishes
//! "waiting for the user to approve a tool" from "waiting for the user to
//! type", so [`hook`] does not guess: the agent's own hook system writes a
//! small state file and this crate reads it. [`integration`] is what puts that
//! hook into the agent's configuration — additively, backed up, and only on an
//! explicit user action (ADR-018).
//!
//! | Source | Sees | Blind to |
//! |---|---|---|
//! | [`hook`] (state file) | that the agent stopped at a permission prompt | everything the agent did not send |
//!
//! Hook state is *evidence, not authority*: it joins onto a session the other
//! two sources already found, never creates one on its own, and never outranks
//! the absence of a process.

pub mod hook;
pub mod integration;
pub mod json;
pub mod probe;
pub mod transcript;

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

pub use hook::{HookPhase, HookScanOptions, HookState, WriteOutcome, phase_of_event};
pub use integration::{
    HookTarget, IntegrationChange, IntegrationError, IntegrationReport, IntegrationStatus,
    PlannedEvent,
};
pub use probe::{CommandRunner, ProcessEvidence, Provider, SystemCommandRunner};
pub use transcript::{
    FollowOptions, Followed, FollowedEntry, FollowedRecord, NoticeLevel, ScanOptions, ToolCall,
    TranscriptEvidence, follow, follow_with,
};

/// Which sources agreed that a session exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Evidence {
    /// A live process, but no transcript we could read — a session that has
    /// only just started, or one whose transcript is outside the scan window.
    ProcessOnly,
    /// A transcript, but no live process: finished, or ended while we were
    /// not looking.
    TranscriptOnly,
    /// Both, joined by session id or by working directory.
    Both,
}

impl Evidence {
    /// Whether a process was seen alive during this scan.
    pub const fn is_running(self) -> bool {
        matches!(self, Self::ProcessOnly | Self::Both)
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ProcessOnly => "PROCESS_ONLY",
            Self::TranscriptOnly => "TRANSCRIPT_ONLY",
            Self::Both => "BOTH",
        }
    }
}

/// The live process behind a session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessFacts {
    pub pid: u32,
    pub tty: Option<String>,
}

/// One agent session, as the machine's evidence describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredSession {
    pub provider: Provider,
    /// The agent's own session id, or `pid-<pid>` for a process we could not
    /// tie to a transcript. Stable for as long as the session is.
    pub session_id: String,
    /// `None` only for a live process whose `lsof` we could not read.
    pub working_directory: Option<PathBuf>,
    pub transcript_path: Option<PathBuf>,
    pub title: Option<String>,
    pub last_prompt: Option<String>,
    pub last_reply: Option<String>,
    pub model: Option<String>,
    pub git_branch: Option<String>,
    pub outstanding_tool: Option<ToolCall>,
    /// When the transcript was last written. `None` for a session with no
    /// readable transcript: the scan time would be a fresh value on every poll,
    /// which would make an unchanging roster look like it changed every tick.
    pub last_active_ms: Option<i64>,
    pub process: Option<ProcessFacts>,
    pub evidence: Evidence,
}

/// Milliseconds since the Unix epoch, saturating rather than panicking on a
/// clock set before 1970.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|since| i64::try_from(since.as_millis()).ok())
        .unwrap_or_default()
}

/// What one scan found, and whether it could see processes at all.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Discovery {
    pub sessions: Vec<DiscoveredSession>,
    /// `false` when the process table could not be read. Every session then
    /// looks finished whether or not it is, so this has to reach the UI: "I
    /// cannot see processes" and "nothing is running" are different answers.
    pub process_evidence: bool,
}

/// Scan both sources and merge them.
///
/// `scan` is `None` when there is no transcript root to read (no home
/// directory, or a platform where the agent stores nothing locally); process
/// evidence still stands on its own.
pub fn discover(scan: Option<&ScanOptions>, runner: &dyn CommandRunner, now_ms: i64) -> Discovery {
    let transcripts = scan.map(|options| transcript::scan(options, now_ms));
    let processes = probe::discover_processes(runner);
    Discovery {
        process_evidence: processes.is_some(),
        sessions: merge(
            transcripts.unwrap_or_default(),
            processes.unwrap_or_default(),
        ),
    }
}

/// Joins process evidence onto transcript evidence.
///
/// Session id is the strong key. Working directory is the fallback, and it is
/// the one that carries most of the time: Claude Code appends to its
/// transcript and closes it, so `lsof` rarely names the file. Where a root has
/// several transcripts, the newest unclaimed one wins — which is why the scan
/// hands them over already sorted.
fn merge(
    transcripts: Vec<TranscriptEvidence>,
    processes: Vec<ProcessEvidence>,
) -> Vec<DiscoveredSession> {
    let mut claimed: Vec<Option<ProcessFacts>> = vec![None; transcripts.len()];
    let mut unmatched = Vec::new();

    for process in processes {
        let facts = ProcessFacts {
            pid: process.pid,
            tty: process.tty.clone(),
        };
        let matched = transcripts
            .iter()
            .enumerate()
            .position(|(index, candidate)| {
                if claimed[index].is_some() || candidate.provider != process.provider {
                    return false;
                }
                match process.session_id.as_deref() {
                    Some(id) => candidate.session_id == id,
                    None => process
                        .working_directory
                        .as_deref()
                        .is_some_and(|cwd| candidate.working_directory == cwd),
                }
            });

        match matched {
            Some(index) => claimed[index] = Some(facts),
            None => unmatched.push((process, facts)),
        }
    }

    let mut sessions: Vec<DiscoveredSession> = transcripts
        .into_iter()
        .zip(claimed)
        .map(|(found, process)| DiscoveredSession {
            evidence: if process.is_some() {
                Evidence::Both
            } else {
                Evidence::TranscriptOnly
            },
            provider: found.provider,
            session_id: found.session_id,
            working_directory: Some(found.working_directory),
            transcript_path: Some(found.transcript_path),
            title: found.title,
            last_prompt: found.last_prompt,
            last_reply: found.last_reply,
            model: found.model,
            git_branch: found.git_branch,
            outstanding_tool: found.outstanding_tool,
            last_active_ms: Some(found.last_active_ms),
            process,
        })
        .collect();

    sessions.extend(unmatched.into_iter().map(|(process, facts)| {
        DiscoveredSession {
            provider: process.provider,
            session_id: process
                .session_id
                .unwrap_or_else(|| format!("pid-{}", process.pid)),
            working_directory: process.working_directory,
            transcript_path: process.transcript_path,
            title: None,
            last_prompt: None,
            last_reply: None,
            model: None,
            git_branch: None,
            outstanding_tool: None,
            // Unknown, and deliberately not "now": see the field's docs.
            last_active_ms: None,
            process: Some(facts),
            evidence: Evidence::ProcessOnly,
        }
    }));

    // Running sessions first, then most recently active. The core re-ranks
    // against the open workspace; this order is what it re-ranks within.
    sessions.sort_by(|left, right| {
        right
            .evidence
            .is_running()
            .cmp(&left.evidence.is_running())
            .then(
                right
                    .last_active_ms
                    .unwrap_or_default()
                    .cmp(&left.last_active_ms.unwrap_or_default()),
            )
    });
    sessions
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn transcript(id: &str, cwd: &str, active_ms: i64) -> TranscriptEvidence {
        TranscriptEvidence {
            provider: Provider::ClaudeCode,
            session_id: id.to_owned(),
            working_directory: PathBuf::from(cwd),
            transcript_path: PathBuf::from(format!("/t/{id}.jsonl")),
            title: Some("a task".to_owned()),
            last_prompt: None,
            last_reply: None,
            model: None,
            git_branch: None,
            outstanding_tool: None,
            last_active_ms: active_ms,
        }
    }

    fn process(pid: u32, cwd: Option<&str>, session_id: Option<&str>) -> ProcessEvidence {
        ProcessEvidence {
            provider: Provider::ClaudeCode,
            pid,
            tty: Some("/dev/ttys001".to_owned()),
            working_directory: cwd.map(PathBuf::from),
            transcript_path: None,
            session_id: session_id.map(str::to_owned),
        }
    }

    #[test]
    fn session_id_joins_a_process_to_its_transcript() {
        let merged = merge(
            vec![
                transcript("s1", "/p/one", 10),
                transcript("s2", "/p/two", 9),
            ],
            vec![process(11, Some("/p/two"), Some("s2"))],
        );
        let joined = merged.iter().find(|s| s.session_id == "s2").expect("s2");
        assert_eq!(joined.evidence, Evidence::Both);
        assert_eq!(joined.process.as_ref().map(|p| p.pid), Some(11));

        let alone = merged.iter().find(|s| s.session_id == "s1").expect("s1");
        assert_eq!(alone.evidence, Evidence::TranscriptOnly);
        assert!(alone.process.is_none());
    }

    #[test]
    fn the_working_directory_joins_when_the_process_has_no_session_id() {
        let merged = merge(
            vec![transcript("s1", "/p/one", 10)],
            vec![process(11, Some("/p/one"), None)],
        );
        assert_eq!(merged.len(), 1, "one session, not two");
        assert_eq!(merged[0].evidence, Evidence::Both);
    }

    #[test]
    fn two_processes_in_one_root_claim_two_transcripts() {
        let merged = merge(
            vec![
                transcript("newer", "/p/one", 20),
                transcript("older", "/p/one", 10),
            ],
            vec![
                process(11, Some("/p/one"), None),
                process(12, Some("/p/one"), None),
            ],
        );
        assert_eq!(merged.len(), 2);
        assert!(
            merged.iter().all(|s| s.evidence == Evidence::Both),
            "a claimed transcript is never claimed twice"
        );
    }

    #[test]
    fn a_process_with_no_transcript_is_still_listed() {
        let merged = merge(Vec::new(), vec![process(11, Some("/p/new"), None)]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].evidence, Evidence::ProcessOnly);
        assert_eq!(merged[0].session_id, "pid-11");
        assert_eq!(merged[0].last_active_ms, None, "unknown, never invented");
        assert!(merged[0].title.is_none(), "nothing is invented");
    }

    #[test]
    fn a_process_without_a_working_directory_is_still_listed() {
        let merged = merge(Vec::new(), vec![process(11, None, None)]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].working_directory, None);
    }

    #[test]
    fn a_transcript_in_another_root_is_not_claimed() {
        let merged = merge(
            vec![transcript("s1", "/p/one", 10)],
            vec![process(11, Some("/p/elsewhere"), None)],
        );
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].evidence, Evidence::ProcessOnly);
        assert_eq!(merged[1].evidence, Evidence::TranscriptOnly);
    }

    #[test]
    fn running_sessions_sort_ahead_of_finished_ones() {
        let merged = merge(
            vec![
                transcript("recent-but-done", "/p/one", 900),
                transcript("older-but-live", "/p/two", 100),
            ],
            vec![process(11, Some("/p/two"), None)],
        );
        assert_eq!(merged[0].session_id, "older-but-live");
        assert!(merged[0].evidence.is_running());
        assert_eq!(merged[1].session_id, "recent-but-done");
    }

    #[test]
    fn discovery_without_a_transcript_root_still_reports_processes() {
        struct OnlyPs;
        impl CommandRunner for OnlyPs {
            fn run(
                &self,
                program: &str,
                _args: &[&str],
                _timeout: std::time::Duration,
            ) -> Option<String> {
                (program == "/bin/ps").then(|| " 42 1 ttys000  claude\n".to_owned())
            }
        }

        let found = discover(None, &OnlyPs, 100);
        assert!(found.process_evidence);
        assert_eq!(found.sessions.len(), 1);
        assert_eq!(found.sessions[0].session_id, "pid-42");
    }

    #[test]
    fn discovery_with_nothing_running_and_nothing_stored_is_empty() {
        struct Silent;
        impl CommandRunner for Silent {
            fn run(&self, _: &str, _: &[&str], _: std::time::Duration) -> Option<String> {
                None
            }
        }

        let options = ScanOptions::rooted_at(Path::new("/nonexistent/gritty").to_path_buf());
        let found = discover(Some(&options), &Silent, 100);
        assert!(found.sessions.is_empty());
        assert!(
            !found.process_evidence,
            "a silent runner means blind, not quiet"
        );
    }
}
