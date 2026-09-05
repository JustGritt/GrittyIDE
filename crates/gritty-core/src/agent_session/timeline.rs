//! The open-timeline ring, the fold from transcript records into domain
//! entries, and the path resolution that decides which touched files the
//! renderer may open (T230).
//!
//! **Why a ring.** A session's transcript grows without bound and a renderer
//! only ever draws a tail of it, so a bounded buffer that drops from the
//! *oldest* end is the shape — the same one the terminal uses for PTY output
//! (`gritty_terminal::batch`). What is copied from there is not the code but
//! the rule: report the loss, never hide it. A dropped entry is counted and the
//! count crosses the protocol, so the UI can draw a gap instead of splicing
//! unrelated entries together.
//!
//! **What `dropped` means on each side.** On [`AgentTimeline`] it is every
//! entry this ring has evicted since the timeline was opened, which is what a
//! client holding the snapshot needs. On the appended-batch event it is the
//! entries evicted since the previous batch, which is what a client mirroring
//! the ring needs to trim from its own head. Neither is an error and neither is
//! silent.
//!
//! **Why resolution happens here.** `file-history-delta.trackingPath` is
//! written relative to the agent's working directory when the file is inside it
//! and absolute when it is not. Only the core knows which workspace is open, so
//! only the core can say whether a touched file is one the renderer may open.
//! A path that lands outside the open root crosses the protocol as a display
//! string and nothing else (ADR-010): there is no absolute path on the wire and
//! nothing outside the root is openable from the WebView.

use std::collections::VecDeque;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use gritty_agents::transcript::{
    Followed, FollowedEntry, FollowedRecord, MAX_NOTICE, MAX_PREVIEW, MAX_TOOL_DETAIL,
    MAX_TRACKING_PATH, NoticeLevel,
};
use gritty_workspace::WorkspaceRoot;

use super::wire::{
    AgentNoticeLevel, AgentTimeline, AgentTimelineAppendedPayload, AgentTimelineEntry,
    AgentTimelineKind,
};
use crate::protocol::time::Timestamp;
use crate::workspace::recents::shorten;

/// Entries retained per open timeline.
///
/// Enough that scrolling back through a turn or two always works, small enough
/// that eight open timelines cost a few megabytes rather than a session's whole
/// history. Beyond it the oldest entries go and the loss is reported.
pub const MAX_TIMELINE_ENTRIES: usize = 500;

/// What one open timeline holds: where its follower is, what it has read, and
/// what has not yet been delivered.
pub(super) struct OpenTimeline {
    /// Byte offset the next follow call resumes from.
    pub(super) offset: u64,
    /// The transcript to follow. `None` for a session that is process evidence
    /// only — there is nothing to follow, and saying so is the answer.
    pub(super) transcript: Option<PathBuf>,
    /// The agent's working directory: what a relative `trackingPath` is
    /// relative to.
    pub(super) cwd: Option<PathBuf>,
    /// The workspace the touched paths were resolved against. A different one
    /// means the resolution no longer holds, which is why the manager closes
    /// the timeline instead of quietly re-pointing it.
    pub(super) root: Option<Arc<WorkspaceRoot>>,
    entries: VecDeque<AgentTimelineEntry>,
    /// Folded but not yet delivered. Drained once per fast-loop tick, so a
    /// hundred records become one event rather than a hundred (rule 8).
    undelivered: Vec<AgentTimelineEntry>,
    /// The permission prompt the session is stopped at (T221), held beside the
    /// ring rather than in it: it is a state that goes away, and a ring can
    /// only ever append. Set from hook evidence and from nothing else.
    permission: Option<AgentTimelineEntry>,
    /// Whether [`Self::permission`] changed since the last batch — the one
    /// thing that can make a tick emit with no entries appended.
    permission_changed: bool,
    dropped_total: u32,
    pending_dropped: u32,
    restarted: bool,
    pending_restarted: bool,
    next_seq: u64,
    readable: bool,
}

impl OpenTimeline {
    pub(super) fn new(
        transcript: Option<PathBuf>,
        cwd: Option<PathBuf>,
        root: Option<Arc<WorkspaceRoot>>,
    ) -> Self {
        Self {
            offset: 0,
            transcript,
            cwd,
            root,
            entries: VecDeque::new(),
            undelivered: Vec::new(),
            permission: None,
            permission_changed: false,
            dropped_total: 0,
            pending_dropped: 0,
            restarted: false,
            pending_restarted: false,
            next_seq: 0,
            readable: false,
        }
    }

    /// Folds what the follower read into the ring.
    ///
    /// `seq` keeps increasing across a restart: it orders entries and nothing
    /// else, so a consumer appends by it without ever reconciling.
    pub(super) fn absorb(&mut self, followed: Followed, home: Option<&Path>) {
        if followed.readable {
            // Sticky: a transcript that was readable and has since been deleted
            // still explains everything already in the ring.
            self.readable = true;
        }
        self.offset = followed.next_offset;

        if followed.restarted {
            // The file this view was built from is gone, or a gap opened in it.
            // Keeping the old entries would splice two unrelated stretches of
            // conversation into one.
            self.entries.clear();
            self.undelivered.clear();
            self.restarted = true;
            self.pending_restarted = true;
        }

        for entry in followed.entries {
            let seq = self.next_seq;
            self.next_seq = self.next_seq.saturating_add(1);
            let folded = fold(entry, seq, self.cwd.as_deref(), self.root.as_deref(), home);
            self.entries.push_back(folded.clone());
            self.undelivered.push(folded);
            if self.entries.len() > MAX_TIMELINE_ENTRIES {
                self.entries.pop_front();
                self.dropped_total = self.dropped_total.saturating_add(1);
                self.pending_dropped = self.pending_dropped.saturating_add(1);
            }
        }

        // The delta is bounded too. A burst that overflowed the ring must not
        // then cross the protocol whole; what the client cannot have is already
        // counted above.
        if self.undelivered.len() > MAX_TIMELINE_ENTRIES {
            self.undelivered
                .drain(..self.undelivered.len() - MAX_TIMELINE_ENTRIES);
        }
    }

    /// One coalesced batch, or **nothing at all** when nothing changed.
    ///
    /// An unchanged transcript emits no event: this is the terminal's flusher
    /// rule and the roster's, and it is what keeps a 300 ms loop from being a
    /// 300 ms event storm (rule 8).
    pub(super) fn take_batch(&mut self, session_id: &str) -> Option<AgentTimelineAppendedPayload> {
        let restarted = std::mem::take(&mut self.pending_restarted);
        let dropped = std::mem::take(&mut self.pending_dropped);
        let permission_changed = std::mem::take(&mut self.permission_changed);
        let entries = std::mem::take(&mut self.undelivered);
        (!entries.is_empty() || dropped > 0 || restarted || permission_changed).then(|| {
            AgentTimelineAppendedPayload {
                session_id: session_id.to_owned(),
                entries,
                dropped,
                restarted,
                // The whole current value, not a delta: a consumer replaces it.
                pending: self.permission.clone(),
            }
        })
    }

    /// Record what the session is stopped at, or that it no longer is (T221).
    ///
    /// Returns whether anything changed, which is what keeps a hook that
    /// rewrites the same phase three times a second from becoming three events
    /// (rule 8).
    pub(super) fn set_permission(&mut self, permission: Option<AgentTimelineEntry>) -> bool {
        if self.permission == permission {
            return false;
        }
        self.permission = permission;
        self.permission_changed = true;
        true
    }

    /// Everything the ring holds. The answer to `agents.openTimeline` and to
    /// `agents.timeline`.
    pub(super) fn snapshot(&self, session_id: &str) -> AgentTimeline {
        AgentTimeline {
            session_id: session_id.to_owned(),
            entries: self.entries.iter().cloned().collect(),
            dropped: self.dropped_total,
            restarted: self.restarted,
            transcript_evidence: self.readable,
            pending: self.permission.clone(),
        }
    }

    /// Drop an undelivered batch. Used after `openTimeline` answers, because
    /// the caller already has those entries in the response.
    pub(super) fn forget_pending(&mut self) {
        self.undelivered.clear();
        self.pending_dropped = 0;
        self.pending_restarted = false;
        self.permission_changed = false;
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }
}

/// The permission prompt a session is stopped at, as a timeline entry (T221).
///
/// Produced from hook evidence and from nothing else — no transcript pattern
/// may synthesise one, because a prompt and a running tool look identical in a
/// transcript (rule 12).
///
/// `detail` comes from the *transcript's* outstanding tool call rather than
/// from the state file, and only when the two name the same tool. The state
/// file deliberately carries no tool input: a `Bash` command line or an `Edit`
/// payload is exactly the kind of thing that holds a secret, and putting it in
/// a file on disk to display it would be the wrong trade (rule 13). The
/// transcript already has the core's bounded preview of the same call, so the
/// row can say `Bash · cargo test` without the hook ever having written it.
pub(super) fn permission_entry(
    tool: Option<&str>,
    detail: Option<&str>,
    since_ms: i64,
) -> Option<AgentTimelineEntry> {
    // Blocked on *something* is still worth a status; blocked on a tool nobody
    // named is not worth an entry that would have to invent its subject.
    let tool = tool?;
    Some(AgentTimelineEntry {
        // Not a position in the ring: this entry is always the last thing in
        // the feed, and it vanishes rather than being superseded.
        seq: u64::MAX,
        at: u64::try_from(since_ms).ok().map(Timestamp::from_millis),
        sidechain: false,
        kind: AgentTimelineKind::PermissionPending {
            tool: bounded(tool, MAX_TOOL_DETAIL),
            detail: detail.map(|detail| bounded(detail, MAX_TOOL_DETAIL)),
        },
    })
}

/// One adapter record to one domain entry.
fn fold(
    entry: FollowedEntry,
    seq: u64,
    cwd: Option<&Path>,
    root: Option<&WorkspaceRoot>,
    home: Option<&Path>,
) -> AgentTimelineEntry {
    AgentTimelineEntry {
        seq,
        // A negative epoch is a clock nobody should trust; no time is better
        // than a wrong one.
        at: entry
            .at_ms
            .and_then(|millis| u64::try_from(millis).ok())
            .map(Timestamp::from_millis),
        sidechain: entry.sidechain,
        kind: fold_kind(entry.record, cwd, root, home),
    }
}

/// The provider-neutral kind for a record the adapter recognised (rule 4).
///
/// Every string is bounded again here. The adapter already clipped it; this is
/// the core doing it at the boundary it owns, so an adapter that grows a field
/// tomorrow cannot put an unbounded one on the wire (rule 13).
fn fold_kind(
    record: FollowedRecord,
    cwd: Option<&Path>,
    root: Option<&WorkspaceRoot>,
    home: Option<&Path>,
) -> AgentTimelineKind {
    match record {
        FollowedRecord::UserPrompt { text } => AgentTimelineKind::UserPrompt {
            text: bounded(&text, MAX_PREVIEW),
        },
        FollowedRecord::AssistantText { text } => AgentTimelineKind::AssistantText {
            text: bounded(&text, MAX_PREVIEW),
        },
        FollowedRecord::Thinking { text } => AgentTimelineKind::Thinking {
            text: bounded(&text, MAX_PREVIEW),
        },
        FollowedRecord::ToolCall {
            tool_use_id,
            name,
            detail,
        } => AgentTimelineKind::ToolCall {
            tool_use_id: tool_use_id.map(|id| bounded(&id, MAX_TOOL_DETAIL)),
            name: bounded(&name, MAX_TOOL_DETAIL),
            detail: detail.map(|detail| bounded(&detail, MAX_TOOL_DETAIL)),
        },
        FollowedRecord::ToolResult {
            tool_use_id,
            ok,
            detail,
        } => AgentTimelineKind::ToolResult {
            tool_use_id: tool_use_id.map(|id| bounded(&id, MAX_TOOL_DETAIL)),
            ok,
            detail: detail.map(|detail| bounded(&detail, MAX_TOOL_DETAIL)),
        },
        FollowedRecord::FileTouched { tracking_path } => {
            fold_file_touched(&tracking_path, cwd, root, home)
        }
        FollowedRecord::Notice { level, text } => AgentTimelineKind::Notice {
            level: match level {
                NoticeLevel::Info => AgentNoticeLevel::Info,
                NoticeLevel::Warning => AgentNoticeLevel::Warning,
                NoticeLevel::Error => AgentNoticeLevel::Error,
            },
            text: bounded(&text, MAX_NOTICE),
        },
    }
}

/// A touched file, placed against the open workspace.
///
/// Inside it the entry carries a [`gritty_workspace::WorkspacePath`] the UI may
/// open. Outside it — another checkout, the agent's own state directory, a
/// scratch file in `/tmp` — the entry carries a display string and no path at
/// all, which is what stops anything outside the root being opened from the
/// renderer.
fn fold_file_touched(
    tracking_path: &str,
    cwd: Option<&Path>,
    root: Option<&WorkspaceRoot>,
    home: Option<&Path>,
) -> AgentTimelineKind {
    let absolute = lexical_absolute(cwd, tracking_path);
    let path = absolute
        .as_deref()
        .zip(root)
        .and_then(|(absolute, root)| root.relativize(absolute))
        // The root itself is not a file, and "open the workspace" is not an
        // action a timeline entry offers.
        .filter(|path| !path.is_root());
    let display = match (&path, absolute.as_deref()) {
        (Some(path), _) => path.as_str().to_owned(),
        (None, Some(absolute)) => shorten(absolute, home),
        (None, None) => tracking_path.to_owned(),
    };
    AgentTimelineKind::FileTouched {
        path,
        display: bounded(&display, MAX_TRACKING_PATH),
    }
}

/// The absolute path a `trackingPath` names, without touching the filesystem.
///
/// Lexical on purpose. Following symlinks would mean a `stat` per entry on a
/// loop that runs three times a second, and the renderer cannot open the result
/// without [`WorkspaceRoot::resolve`] checking it physically again anyway — so
/// the cheap answer here is also the safe one. A path that climbs out of
/// everything it was resolved against yields `None` rather than being clamped:
/// nothing may turn into a root by accident.
fn lexical_absolute(base: Option<&Path>, raw: &str) -> Option<PathBuf> {
    let raw = Path::new(raw);
    let mut resolved = if raw.is_absolute() {
        PathBuf::new()
    } else {
        base?.to_path_buf()
    };
    for component in raw.components() {
        match component {
            Component::Prefix(prefix) => resolved = PathBuf::from(prefix.as_os_str()),
            Component::RootDir => resolved.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                if !resolved.pop() {
                    return None;
                }
            }
            Component::Normal(name) => resolved.push(name),
        }
    }
    resolved.is_absolute().then_some(resolved)
}

/// The last bound before a string crosses the protocol.
fn bounded(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    let kept: String = text.chars().take(limit.saturating_sub(1)).collect();
    format!("{kept}…")
}

#[cfg(test)]
mod tests {
    use gritty_agents::{Followed, FollowedEntry, FollowedRecord, NoticeLevel};
    use gritty_workspace::WorkspacePath;

    use super::*;

    /// A scratch directory that cleans itself up, so a real [`WorkspaceRoot`]
    /// can be opened: path resolution is the one thing here that cannot be
    /// tested against an imaginary root.
    struct TempRoot(PathBuf);

    impl TempRoot {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "gritty-timeline-{name}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(path.join("src")).expect("create temp root");
            Self(path)
        }

        fn root(&self) -> Arc<WorkspaceRoot> {
            Arc::new(WorkspaceRoot::open(&self.0).expect("open root"))
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn entry(record: FollowedRecord) -> FollowedEntry {
        FollowedEntry {
            at_ms: Some(1_788_616_187_770),
            sidechain: false,
            record,
        }
    }

    fn read(entries: Vec<FollowedEntry>) -> Followed {
        Followed {
            entries,
            next_offset: 0,
            restarted: false,
            readable: true,
        }
    }

    fn kinds(timeline: &OpenTimeline) -> Vec<AgentTimelineKind> {
        timeline
            .entries
            .iter()
            .map(|entry| entry.kind.clone())
            .collect()
    }

    // --------------------------------------------------------------- the fold

    #[test]
    fn every_record_becomes_its_provider_neutral_kind() {
        let mut timeline = OpenTimeline::new(None, None, None);
        timeline.absorb(
            read(vec![
                entry(FollowedRecord::UserPrompt {
                    text: "ship it".to_owned(),
                }),
                entry(FollowedRecord::AssistantText {
                    text: "on it".to_owned(),
                }),
                entry(FollowedRecord::Thinking {
                    text: "weighing".to_owned(),
                }),
                entry(FollowedRecord::ToolCall {
                    tool_use_id: Some("toolu_1".to_owned()),
                    name: "Bash".to_owned(),
                    detail: Some("cargo test".to_owned()),
                }),
                entry(FollowedRecord::ToolResult {
                    tool_use_id: Some("toolu_1".to_owned()),
                    ok: false,
                    detail: Some("exit 1".to_owned()),
                }),
                entry(FollowedRecord::Notice {
                    level: NoticeLevel::Warning,
                    text: "a hook complained".to_owned(),
                }),
            ]),
            None,
        );

        assert_eq!(
            kinds(&timeline),
            vec![
                AgentTimelineKind::UserPrompt {
                    text: "ship it".to_owned()
                },
                AgentTimelineKind::AssistantText {
                    text: "on it".to_owned()
                },
                AgentTimelineKind::Thinking {
                    text: "weighing".to_owned()
                },
                AgentTimelineKind::ToolCall {
                    tool_use_id: Some("toolu_1".to_owned()),
                    name: "Bash".to_owned(),
                    detail: Some("cargo test".to_owned()),
                },
                AgentTimelineKind::ToolResult {
                    tool_use_id: Some("toolu_1".to_owned()),
                    ok: false,
                    detail: Some("exit 1".to_owned()),
                },
                AgentTimelineKind::Notice {
                    level: AgentNoticeLevel::Warning,
                    text: "a hook complained".to_owned(),
                },
            ]
        );
    }

    #[test]
    fn seq_increases_strictly_so_a_consumer_appends_without_reconciling() {
        let mut timeline = OpenTimeline::new(None, None, None);
        for _ in 0..3 {
            timeline.absorb(
                read(vec![entry(FollowedRecord::AssistantText {
                    text: "a".to_owned(),
                })]),
                None,
            );
        }
        let seqs: Vec<u64> = timeline.entries.iter().map(|entry| entry.seq).collect();
        assert_eq!(seqs, vec![0, 1, 2]);
        assert_eq!(
            timeline.entries[0].at,
            Some(Timestamp::from_millis(1_788_616_187_770))
        );
    }

    #[test]
    fn a_subagent_entry_is_marked_rather_than_merged() {
        let mut timeline = OpenTimeline::new(None, None, None);
        let mut sidechained = entry(FollowedRecord::AssistantText {
            text: "subagent".to_owned(),
        });
        sidechained.sidechain = true;
        timeline.absorb(
            read(vec![
                entry(FollowedRecord::AssistantText {
                    text: "parent".to_owned(),
                }),
                sidechained,
            ]),
            None,
        );
        let flags: Vec<bool> = timeline
            .entries
            .iter()
            .map(|entry| entry.sidechain)
            .collect();
        assert_eq!(flags, vec![false, true]);
    }

    // ---------------------------------------------------------- path policy

    fn touched(
        tracking: &str,
        cwd: Option<&Path>,
        root: Option<&WorkspaceRoot>,
    ) -> AgentTimelineKind {
        fold_kind(
            FollowedRecord::FileTouched {
                tracking_path: tracking.to_owned(),
            },
            cwd,
            root,
            None,
        )
    }

    #[test]
    fn a_file_inside_the_open_workspace_is_one_the_ui_may_open() {
        let temp = TempRoot::new("inside");
        let root = temp.root();
        let kind = touched("src/main.rs", Some(root.path()), Some(&root));
        let AgentTimelineKind::FileTouched { path, display } = kind else {
            panic!("file touched");
        };
        assert_eq!(
            path.as_ref().map(WorkspacePath::as_str),
            Some("src/main.rs")
        );
        assert_eq!(display, "src/main.rs");
    }

    #[test]
    fn a_file_outside_the_open_workspace_has_nothing_to_open() {
        let temp = TempRoot::new("outside");
        let root = temp.root();
        // The agent's own state directory: absolute, and nowhere near the root.
        let kind = touched(
            "/Users/someone/.claude/plans/a-plan.md",
            Some(root.path()),
            Some(&root),
        );
        let AgentTimelineKind::FileTouched { path, display } = kind else {
            panic!("file touched");
        };
        assert_eq!(path, None, "nothing outside the root is openable (ADR-010)");
        assert_eq!(display, "/Users/someone/.claude/plans/a-plan.md");
    }

    #[test]
    fn an_absolute_path_inside_the_root_still_resolves() {
        let temp = TempRoot::new("absolute");
        let root = temp.root();
        let inside = root.path().join("src/main.rs");
        let kind = touched(&inside.to_string_lossy(), Some(root.path()), Some(&root));
        let AgentTimelineKind::FileTouched { path, .. } = kind else {
            panic!("file touched");
        };
        assert_eq!(
            path.as_ref().map(WorkspacePath::as_str),
            Some("src/main.rs")
        );
    }

    #[test]
    fn with_no_workspace_open_nothing_is_openable() {
        let kind = touched("src/main.rs", Some(Path::new("/Users/a/p")), None);
        let AgentTimelineKind::FileTouched { path, display } = kind else {
            panic!("file touched");
        };
        assert_eq!(path, None);
        assert_eq!(display, "/Users/a/p/src/main.rs");
    }

    #[test]
    fn a_relative_path_with_no_working_directory_is_shown_and_not_resolved() {
        let kind = touched("src/main.rs", None, None);
        let AgentTimelineKind::FileTouched { path, display } = kind else {
            panic!("file touched");
        };
        assert_eq!(path, None);
        assert_eq!(display, "src/main.rs");
    }

    #[test]
    fn a_path_that_climbs_out_of_the_root_is_not_openable() {
        let temp = TempRoot::new("escape");
        let root = temp.root();
        let kind = touched("../elsewhere/secrets.env", Some(root.path()), Some(&root));
        let AgentTimelineKind::FileTouched { path, .. } = kind else {
            panic!("file touched");
        };
        assert_eq!(path, None, "the sibling directory is not in the workspace");
    }

    #[test]
    fn lexical_resolution_normalizes_without_touching_the_filesystem() {
        let base = Path::new("/Users/a/p");
        assert_eq!(
            lexical_absolute(Some(base), "./src/./main.rs"),
            Some(PathBuf::from("/Users/a/p/src/main.rs"))
        );
        assert_eq!(
            lexical_absolute(Some(base), "src/../lib.rs"),
            Some(PathBuf::from("/Users/a/p/lib.rs"))
        );
        assert_eq!(
            lexical_absolute(Some(base), "/tmp/scratch"),
            Some(PathBuf::from("/tmp/scratch"))
        );
        // Climbing past the top of everything it was resolved against.
        assert_eq!(lexical_absolute(Some(Path::new("/")), ".."), None);
        assert_eq!(lexical_absolute(None, "src/main.rs"), None);
    }

    // ------------------------------------------------------------- bounding

    #[test]
    fn every_kind_is_bounded_before_it_crosses_the_protocol() {
        let long = "x".repeat(2_000);
        let mut timeline = OpenTimeline::new(None, Some(PathBuf::from("/Users/a/p")), None);
        timeline.absorb(
            read(vec![
                entry(FollowedRecord::UserPrompt { text: long.clone() }),
                entry(FollowedRecord::AssistantText { text: long.clone() }),
                entry(FollowedRecord::Thinking { text: long.clone() }),
                entry(FollowedRecord::ToolCall {
                    tool_use_id: Some(long.clone()),
                    name: long.clone(),
                    detail: Some(long.clone()),
                }),
                entry(FollowedRecord::ToolResult {
                    tool_use_id: Some(long.clone()),
                    ok: true,
                    detail: Some(long.clone()),
                }),
                entry(FollowedRecord::FileTouched {
                    tracking_path: long.clone(),
                }),
                entry(FollowedRecord::Notice {
                    level: NoticeLevel::Info,
                    text: long.clone(),
                }),
            ]),
            None,
        );

        for kind in kinds(&timeline) {
            let (text, limit) = match &kind {
                AgentTimelineKind::UserPrompt { text }
                | AgentTimelineKind::AssistantText { text }
                | AgentTimelineKind::Thinking { text } => (text.clone(), MAX_PREVIEW),
                AgentTimelineKind::ToolCall { name, detail, .. } => {
                    assert_eq!(name.chars().count(), MAX_TOOL_DETAIL);
                    (detail.clone().unwrap_or_default(), MAX_TOOL_DETAIL)
                }
                AgentTimelineKind::ToolResult { detail, .. } => {
                    (detail.clone().unwrap_or_default(), MAX_TOOL_DETAIL)
                }
                AgentTimelineKind::FileTouched { display, .. } => {
                    (display.clone(), MAX_TRACKING_PATH)
                }
                AgentTimelineKind::Notice { text, .. } => (text.clone(), MAX_NOTICE),
                AgentTimelineKind::PermissionPending { .. } => unreachable!("T221 only"),
            };
            assert_eq!(text.chars().count(), limit, "{kind:?}");
            assert!(text.ends_with('…'));
        }
    }

    // ----------------------------------------------------------- the ring

    #[test]
    fn the_ring_drops_from_the_oldest_end_and_reports_the_loss() {
        let mut timeline = OpenTimeline::new(None, None, None);
        let overflow = 3;
        timeline.absorb(
            read(
                (0..MAX_TIMELINE_ENTRIES + overflow)
                    .map(|index| {
                        entry(FollowedRecord::AssistantText {
                            text: index.to_string(),
                        })
                    })
                    .collect(),
            ),
            None,
        );

        assert_eq!(timeline.len(), MAX_TIMELINE_ENTRIES, "the ring is bounded");
        let snapshot = timeline.snapshot("s");
        assert_eq!(snapshot.dropped, overflow as u32, "the loss is reported");
        assert!(
            matches!(
                &snapshot.entries[0].kind,
                AgentTimelineKind::AssistantText { text } if text == &overflow.to_string()
            ),
            "the oldest entries went, the newest survived"
        );
        assert_eq!(
            snapshot.entries.last().map(|entry| entry.seq),
            Some(MAX_TIMELINE_ENTRIES as u64 + overflow as u64 - 1)
        );
    }

    #[test]
    fn nothing_read_means_no_batch_at_all() {
        let mut timeline = OpenTimeline::new(None, None, None);
        assert!(timeline.take_batch("s").is_none());

        timeline.absorb(read(Vec::new()), None);
        assert!(
            timeline.take_batch("s").is_none(),
            "an unchanged transcript emits nothing (rule 8)"
        );

        timeline.absorb(
            read(vec![entry(FollowedRecord::AssistantText {
                text: "a".to_owned(),
            })]),
            None,
        );
        let batch = timeline.take_batch("s").expect("a batch");
        assert_eq!(batch.entries.len(), 1);
        assert_eq!(batch.dropped, 0);
        assert!(!batch.restarted);
        assert!(
            timeline.take_batch("s").is_none(),
            "a batch is delivered once"
        );
    }

    #[test]
    fn many_records_in_one_tick_become_one_batch() {
        let mut timeline = OpenTimeline::new(None, None, None);
        timeline.absorb(
            read(
                (0..50)
                    .map(|_| {
                        entry(FollowedRecord::AssistantText {
                            text: "a".to_owned(),
                        })
                    })
                    .collect(),
            ),
            None,
        );
        let batch = timeline.take_batch("s").expect("a batch");
        assert_eq!(batch.entries.len(), 50, "coalesced, not fifty events");
    }

    #[test]
    fn a_restart_resets_the_view_instead_of_splicing_it() {
        let mut timeline = OpenTimeline::new(None, None, None);
        timeline.absorb(
            read(vec![entry(FollowedRecord::AssistantText {
                text: "before".to_owned(),
            })]),
            None,
        );
        timeline.take_batch("s").expect("first batch");

        timeline.absorb(
            Followed {
                entries: vec![entry(FollowedRecord::AssistantText {
                    text: "after".to_owned(),
                })],
                next_offset: 12,
                restarted: true,
                readable: true,
            },
            None,
        );

        let batch = timeline.take_batch("s").expect("a batch");
        assert!(batch.restarted);
        assert_eq!(batch.entries.len(), 1);
        let snapshot = timeline.snapshot("s");
        assert_eq!(snapshot.entries.len(), 1, "the old view was dropped");
        assert!(snapshot.restarted);
        assert_eq!(
            snapshot.entries[0].seq, 1,
            "seq keeps increasing across a restart"
        );
        assert_eq!(timeline.offset, 12);
    }

    #[test]
    fn an_unreadable_transcript_is_reported_rather_than_treated_as_silence() {
        let mut timeline = OpenTimeline::new(None, None, None);
        timeline.absorb(Followed::default(), None);
        assert!(
            !timeline.snapshot("s").transcript_evidence,
            "nothing to read is not the same as the agent doing nothing"
        );

        timeline.absorb(read(Vec::new()), None);
        assert!(timeline.snapshot("s").transcript_evidence);

        // Deleted while followed: what was already read stays readable.
        timeline.absorb(Followed::default(), None);
        assert!(timeline.snapshot("s").transcript_evidence);
    }
}
