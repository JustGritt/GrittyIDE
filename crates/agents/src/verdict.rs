//! The answer travelling the other way: Gritty's verdict on a permission
//! prompt, and the liveness marker that makes waiting for one safe (T335,
//! ADR-020).
//!
//! [`crate::hook`] owns the file the agent writes. This module owns the file
//! Gritty writes, in the same shape and for the same reason: one writer, one
//! reader, one definition, so the two cannot drift.
//!
//! - [`write`] is what the core calls when the user answers. It is the only
//!   writer.
//! - [`take`] is what the hook binary calls. It is the only reader, and it
//!   removes the file as it reads: a verdict answers exactly one ask, once.
//!
//! **A file, still, and not a socket.** ADR-018 §6 rejected a socket because
//! Gritty is usually not running, and ADR-020 changes nothing about that
//! argument — it only adds a second file going the other way. A poll of a
//! directory needs nothing listening and has no connect failure for a binary
//! that must not have opinions to reason about.
//!
//! **The marker is the whole safety story.** [`is_live`] is what separates
//! "wait for an answer that is coming" from "hang an agent on a machine where
//! Gritty is not running". Everything else here is a bound on how long, and
//! every one of those bounds fails open: no marker, stale marker, no verdict,
//! unreadable verdict and malformed verdict all end the same way, with the
//! hook returning nothing and the agent asking its own user (ADR-020 §3).

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;

use crate::hook::{MAX_HOOK_ASK_ID, MAX_HOOK_DETAIL, bounded, sanitize_id};

/// Format version of the verdict file. Same policy as the state file's: a
/// newer version is skipped rather than guessed at.
pub const VERDICT_VERSION: u32 = 1;

/// A verdict file is two short strings. Anything larger is not one.
pub const MAX_VERDICT_BYTES: u64 = 4 * 1024;

/// How recent the marker must be for the hook to believe Gritty is alive.
///
/// The core refreshes it on the 400 ms loop it already runs, so this is
/// generous by a factor of ten — a machine under load, or a core paused in a
/// debugger, must not read as dead. It is also the bound on the other promise:
/// a force-killed Gritty releases a waiting agent within this long, because the
/// marker stops being refreshed the moment the process dies.
pub const MARKER_FRESH_FOR: Duration = Duration::from_secs(4);

/// How often the hook looks for its verdict. Short enough that answering feels
/// immediate, long enough that a waiting hook is not a busy loop.
pub const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// How long the hook will wait in total, absent any other instruction.
///
/// This is a *ceiling on being held*, not an expectation: the marker check
/// ends the wait far sooner whenever Gritty goes away. The installed hook
/// entry's own `timeout` must be comfortably larger, or Claude Code discards
/// the answer just as it arrives.
pub const DEFAULT_WAIT: Duration = Duration::from_secs(300);

/// What the user decided. Two values, because two is all the agent understands.
///
/// "Allow once" and "allow for this run" are the *same* answer to the agent —
/// yes — and differ only in what Gritty remembers afterwards. Keeping the
/// distinction on Gritty's side of the boundary is what lets the scope of a
/// grant be Gritty's own idea rather than something each provider has to have
/// a word for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Behavior {
    Allow,
    Deny,
}

impl Behavior {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Deny => "deny",
        }
    }

    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "allow" => Some(Self::Allow),
            "deny" => Some(Self::Deny),
            _ => None,
        }
    }
}

/// One answer, addressed to one ask.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    pub version: u32,
    /// The [`crate::hook::HookAsk::ask_id`] this answers. A verdict for any
    /// other ask is not late, it is wrong, and the hook ignores it.
    pub ask_id: String,
    pub behavior: Behavior,
    /// Why, for a denial. Measured to reach the model as feedback it reasons
    /// about (ADR-020), which is the difference between a refusal and a wall.
    pub message: Option<String>,
}

impl Verdict {
    fn clipped(&self) -> Self {
        Self {
            version: self.version,
            ask_id: bounded(&self.ask_id, MAX_HOOK_ASK_ID),
            behavior: self.behavior,
            message: self
                .message
                .as_deref()
                .map(|message| bounded(message, MAX_HOOK_DETAIL))
                .filter(|message| !message.is_empty()),
        }
    }

    fn to_json(&self) -> String {
        let mut object = serde_json::Map::new();
        object.insert("version".to_owned(), Value::from(self.version));
        object.insert("askId".to_owned(), Value::from(self.ask_id.clone()));
        object.insert("behavior".to_owned(), Value::from(self.behavior.as_str()));
        if let Some(message) = &self.message {
            object.insert("message".to_owned(), Value::from(message.clone()));
        }
        Value::Object(object).to_string()
    }

    fn from_json(raw: &str) -> Option<Self> {
        let Ok(Value::Object(object)) = serde_json::from_str::<Value>(raw) else {
            return None;
        };
        let version = u32::try_from(object.get("version")?.as_u64()?).ok()?;
        if version > VERDICT_VERSION {
            return None;
        }
        let ask_id = bounded(object.get("askId")?.as_str()?, MAX_HOOK_ASK_ID);
        if ask_id.is_empty() {
            return None;
        }
        Some(Self {
            version,
            ask_id,
            behavior: Behavior::parse(object.get("behavior")?.as_str()?)?,
            message: object
                .get("message")
                .and_then(Value::as_str)
                .map(|message| bounded(message, MAX_HOOK_DETAIL))
                .filter(|message| !message.is_empty()),
        })
    }

    /// The JSON Claude Code honours, measured in ADR-020.
    ///
    /// The reply shape is **provider-specific and lives here**, not in the
    /// verdict file: `PreToolUse` takes `permissionDecision`,
    /// `PermissionRequest` takes `decision: {"behavior": …}`, and a second
    /// provider (T222) will spell it a third way. What crosses the disk is the
    /// decision; how a given agent likes to be told is an adapter's business.
    pub fn to_claude_code_reply(&self) -> String {
        let mut decision = serde_json::Map::new();
        decision.insert("behavior".to_owned(), Value::from(self.behavior.as_str()));
        if let Some(message) = &self.message {
            decision.insert("message".to_owned(), Value::from(message.clone()));
        }
        let mut specific = serde_json::Map::new();
        specific.insert("hookEventName".to_owned(), Value::from("PermissionRequest"));
        specific.insert("decision".to_owned(), Value::Object(decision));
        let mut root = serde_json::Map::new();
        root.insert("hookSpecificOutput".to_owned(), Value::Object(specific));
        Value::Object(root).to_string()
    }
}

/// Where verdicts live, beside the state files and under the same app-data
/// directory — never `/tmp`, never anywhere another user can write.
pub fn verdict_dir_from_data_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("agents").join("verdicts")
}

/// The marker Gritty touches while it is running.
pub fn marker_from_data_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("agents").join("live")
}

/// Where one ask's verdict lives.
///
/// The ask id is a file *name*, never a path — the same rule the state file's
/// session id lives under, and for the same reason: the id round-trips through
/// a payload Gritty does not control.
pub fn verdict_path(verdict_dir: &Path, ask_id: &str) -> Option<PathBuf> {
    let name = sanitize_id(ask_id)?;
    Some(verdict_dir.join(format!("{name}.json")))
}

/// Record the user's answer where the waiting hook will find it.
///
/// Atomic, so a hook polling every 50 ms never reads half a file, and so no
/// lock is needed on either side.
pub fn write(verdict_dir: &Path, verdict: &Verdict) -> Result<(), String> {
    let verdict = verdict.clipped();
    let path =
        verdict_path(verdict_dir, &verdict.ask_id).ok_or_else(|| "unusable ask id".to_owned())?;
    ensure_private_dir(verdict_dir)?;
    gritty_workspace::write_file_atomic(
        &path,
        verdict.to_json().as_bytes(),
        gritty_workspace::AtomicWriteFault::None,
    )
    .map_err(|error| error.to_string())?;
    restrict(&path);
    Ok(())
}

/// Read one verdict and consume it.
///
/// Removed as it is read, so a verdict answers exactly one ask exactly once. A
/// file that outlives its ask — because the user answered in the terminal
/// first, or the session ended — is swept by [`sweep`] rather than waiting
/// around to answer something it was never about.
pub fn take(verdict_dir: &Path, ask_id: &str) -> Option<Verdict> {
    let path = verdict_path(verdict_dir, ask_id)?;
    let raw = read_bounded(&path)?;
    // Removed whether or not it parses: an unreadable verdict that stayed on
    // disk would be re-read on every poll for the rest of the wait.
    let _ = fs::remove_file(&path);
    let verdict = Verdict::from_json(&raw)?;
    // A file whose name and content disagree is not this ask's answer.
    (verdict.ask_id == ask_id).then_some(verdict)
}

/// Whether Gritty is running *now*, as far as a hook can tell.
///
/// This is the single check that separates a bounded wait from a hung agent,
/// so it is deliberately the most pessimistic thing in this module: a marker
/// that cannot be read, cannot be stat-ed, or carries a modification time this
/// process cannot place in the recent past all read as "not running".
pub fn is_live(marker: &Path, now_ms: i64) -> bool {
    let Ok(metadata) = fs::metadata(marker) else {
        return false;
    };
    let Ok(modified) = metadata.modified() else {
        return false;
    };
    let Ok(since_epoch) = modified.duration_since(std::time::UNIX_EPOCH) else {
        return false;
    };
    let Ok(modified_ms) = i64::try_from(since_epoch.as_millis()) else {
        return false;
    };
    // A marker stamped in the future is a clock disagreement, not a lie worth
    // punishing: it reads as fresh, exactly as `HookState::age_ms` treats one.
    let age_ms = now_ms.saturating_sub(modified_ms).max(0);
    age_ms <= i64::try_from(MARKER_FRESH_FOR.as_millis()).unwrap_or(i64::MAX)
}

/// Touch the marker. The core's fast loop calls this; nothing else does.
pub fn touch_marker(marker: &Path) -> Result<(), String> {
    if let Some(parent) = marker.parent() {
        ensure_private_dir(parent)?;
    }
    gritty_workspace::write_file_atomic(
        marker,
        b"gritty",
        gritty_workspace::AtomicWriteFault::None,
    )
    .map_err(|error| error.to_string())?;
    restrict(marker);
    Ok(())
}

/// How long a hook is willing to be held, and how often it looks.
#[derive(Debug, Clone)]
pub struct WaitOptions {
    pub verdict_dir: PathBuf,
    pub marker: PathBuf,
    pub deadline: Duration,
    pub poll: Duration,
}

impl WaitOptions {
    pub fn rooted_at(data_dir: &Path) -> Self {
        Self {
            verdict_dir: verdict_dir_from_data_dir(data_dir),
            marker: marker_from_data_dir(data_dir),
            deadline: DEFAULT_WAIT,
            poll: POLL_INTERVAL,
        }
    }
}

/// Why a wait ended. Every variant but [`WaitOutcome::Answered`] means the
/// hook says nothing and the agent asks its own user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WaitOutcome {
    /// The user answered in Gritty.
    Answered(Verdict),
    /// Gritty was not running when the ask arrived. The common case, and the
    /// one that must cost nothing: no poll, no sleep, no wait at all.
    NotRunning,
    /// Gritty was running and then stopped being. A force-kill lands here.
    WentAway,
    /// Gritty stayed alive and nobody answered in time.
    TimedOut,
}

/// Wait for a verdict, for as long as Gritty is alive and no longer than the
/// deadline.
///
/// `now` and `sleep` are injected so every branch of this function is a test
/// rather than a claim — including the one that matters most, releasing a hook
/// when Gritty is killed mid-wait, which is otherwise only observable by
/// killing a real process and waiting several seconds (rule 12).
pub fn await_verdict<N, S>(
    ask_id: &str,
    options: &WaitOptions,
    mut now: N,
    mut sleep: S,
) -> WaitOutcome
where
    N: FnMut() -> i64,
    S: FnMut(Duration),
{
    // Checked *before* anything else, because "Gritty is not running" is the
    // normal state of this machine and must be the cheapest path through here.
    if !is_live(&options.marker, now()) {
        return WaitOutcome::NotRunning;
    }
    let started_ms = now();
    let deadline_ms = i64::try_from(options.deadline.as_millis()).unwrap_or(i64::MAX);
    loop {
        if let Some(verdict) = take(&options.verdict_dir, ask_id) {
            return WaitOutcome::Answered(verdict);
        }
        let elapsed = now().saturating_sub(started_ms).max(0);
        if elapsed >= deadline_ms {
            return WaitOutcome::TimedOut;
        }
        sleep(options.poll);
        // Re-checked every pass rather than once at the top: this is what turns
        // "Gritty died" from a full-deadline hang into a release within
        // `MARKER_FRESH_FOR`.
        if !is_live(&options.marker, now()) {
            // One last look. The answer may have landed in the same instant the
            // core wrote it and exited, and throwing that away would deny an
            // ask the user actually allowed.
            return match take(&options.verdict_dir, ask_id) {
                Some(verdict) => WaitOutcome::Answered(verdict),
                None => WaitOutcome::WentAway,
            };
        }
    }
}

/// Remove verdicts nobody came back for.
///
/// A verdict outlives its ask whenever the user answered in the terminal first
/// or the session ended mid-prompt — both normal (ADR-020). Sweeping keeps the
/// directory from becoming a history nobody asked for, exactly as the state
/// directory's own age policy does.
pub fn sweep(verdict_dir: &Path, max_age: Duration) {
    let Ok(entries) = fs::read_dir(verdict_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        let too_old = metadata
            .modified()
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age > max_age);
        if too_old {
            let _ = fs::remove_file(entry.path());
        }
    }
}

fn read_bounded(path: &Path) -> Option<String> {
    let metadata = fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_VERDICT_BYTES {
        return None;
    }
    fs::read_to_string(path).ok()
}

/// Owner-only, and created as such: the directory carries the permission
/// because a file that lands by atomic rename can only be chmod-ed afterwards.
fn ensure_private_dir(dir: &Path) -> Result<(), String> {
    if dir.is_dir() {
        return Ok(());
    }
    fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));
    }
    Ok(())
}

fn restrict(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    let _ = path;
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    /// A scratch directory that cleans itself up. Nothing here ever touches a
    /// real app-data directory.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "gritty-verdict-{name}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("create");
            Self(path)
        }

        fn verdicts(&self) -> PathBuf {
            self.0.join("verdicts")
        }

        fn marker(&self) -> PathBuf {
            self.0.join("live")
        }

        fn options(&self) -> WaitOptions {
            WaitOptions {
                verdict_dir: self.verdicts(),
                marker: self.marker(),
                deadline: Duration::from_millis(1_000),
                poll: Duration::from_millis(50),
            }
        }

        fn answer(&self, ask_id: &str, behavior: Behavior) {
            write(
                &self.verdicts(),
                &Verdict {
                    version: VERDICT_VERSION,
                    ask_id: ask_id.to_owned(),
                    behavior,
                    message: None,
                },
            )
            .expect("write verdict");
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// A clock that advances by the amounts the code under test sleeps for, so
    /// a five-minute deadline costs a test no time at all.
    struct Clock {
        now: Cell<i64>,
        sleeps: Cell<u32>,
    }

    impl Clock {
        fn new() -> Self {
            Self {
                now: Cell::new(1_000_000),
                sleeps: Cell::new(0),
            }
        }
    }

    #[test]
    fn without_a_marker_nothing_is_waited_for_at_all() {
        // The common case on every machine where Gritty is not running, and the
        // one that has to cost nothing: not a poll, not a sleep (ADR-020 §2).
        let dir = TempDir::new("no-marker");
        let clock = Clock::new();
        let outcome = await_verdict(
            "ask-1",
            &dir.options(),
            || clock.now.get(),
            |_| clock.sleeps.set(clock.sleeps.get() + 1),
        );
        assert_eq!(outcome, WaitOutcome::NotRunning);
        assert_eq!(clock.sleeps.get(), 0, "it did not even wait once");
    }

    #[test]
    fn a_stale_marker_is_not_a_running_gritty() {
        let dir = TempDir::new("stale");
        touch_marker(&dir.marker()).expect("marker");
        let clock = Clock::new();
        // An hour after the marker was touched, by the caller's own clock.
        let ahead = crate::now_ms() + 60 * 60 * 1_000;
        let outcome = await_verdict(
            "ask-1",
            &dir.options(),
            || ahead,
            |_| clock.sleeps.set(clock.sleeps.get() + 1),
        );
        assert_eq!(outcome, WaitOutcome::NotRunning);
        assert_eq!(clock.sleeps.get(), 0);
    }

    #[test]
    fn an_answer_already_waiting_is_taken_on_the_first_look() {
        let dir = TempDir::new("answered");
        touch_marker(&dir.marker()).expect("marker");
        dir.answer("ask-1", Behavior::Allow);
        let clock = Clock::new();
        let outcome = await_verdict("ask-1", &dir.options(), crate::now_ms, |_| {
            clock.sleeps.set(clock.sleeps.get() + 1)
        });
        assert!(matches!(outcome, WaitOutcome::Answered(_)));
        assert_eq!(clock.sleeps.get(), 0);
        assert!(
            !verdict_path(&dir.verdicts(), "ask-1")
                .expect("path")
                .exists(),
            "consumed as it was read"
        );
    }

    #[test]
    fn a_marker_that_goes_away_mid_wait_releases_rather_than_running_the_deadline_out() {
        let dir = TempDir::new("went-away");
        touch_marker(&dir.marker()).expect("marker");
        let clock = Clock::new();
        let outcome = await_verdict(
            "ask-1",
            &dir.options(),
            || clock.now.get(),
            |slept| {
                clock.now.set(clock.now.get() + slept.as_millis() as i64);
                clock.sleeps.set(clock.sleeps.get() + 1);
                // Gritty is force-killed on the first pass.
                let _ = fs::remove_file(dir.marker());
            },
        );
        assert_eq!(outcome, WaitOutcome::WentAway);
        assert_eq!(clock.sleeps.get(), 1, "one poll, then it let go");
    }

    #[test]
    fn an_answer_that_lands_as_gritty_exits_is_still_honoured() {
        // The race the "one last look" exists for: the core writes the verdict
        // and stops refreshing the marker in the same instant. Throwing that
        // answer away would deny an ask the user actually allowed.
        let dir = TempDir::new("last-look");
        touch_marker(&dir.marker()).expect("marker");
        let clock = Clock::new();
        let outcome = await_verdict(
            "ask-1",
            &dir.options(),
            || clock.now.get(),
            |slept| {
                clock.now.set(clock.now.get() + slept.as_millis() as i64);
                dir.answer("ask-1", Behavior::Allow);
                let _ = fs::remove_file(dir.marker());
            },
        );
        match outcome {
            WaitOutcome::Answered(verdict) => assert_eq!(verdict.behavior, Behavior::Allow),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn nobody_answering_runs_out_the_deadline_and_says_so() {
        let dir = TempDir::new("timeout");
        touch_marker(&dir.marker()).expect("marker");
        let clock = Clock::new();
        let outcome = await_verdict(
            "ask-1",
            &dir.options(),
            || clock.now.get(),
            |slept| {
                clock.now.set(clock.now.get() + slept.as_millis() as i64);
                clock.sleeps.set(clock.sleeps.get() + 1);
                // Kept alive throughout: this is Gritty running and the user
                // being somewhere else.
                touch_marker(&dir.marker()).expect("marker");
            },
        );
        assert_eq!(outcome, WaitOutcome::TimedOut);
        assert_eq!(clock.sleeps.get(), 20, "1000ms deadline at 50ms a poll");
    }

    #[test]
    fn a_verdict_that_cannot_be_read_is_the_same_as_no_verdict_and_never_read_twice() {
        let dir = TempDir::new("malformed");
        fs::create_dir_all(dir.verdicts()).expect("create");
        let path = verdict_path(&dir.verdicts(), "ask-1").expect("path");
        fs::write(&path, b"{not json").expect("write");
        assert_eq!(take(&dir.verdicts(), "ask-1"), None);
        assert!(
            !path.exists(),
            "removed even though it did not parse, or every poll would re-read it"
        );
    }

    #[test]
    fn a_verdict_whose_name_and_content_disagree_is_not_this_asks_answer() {
        let dir = TempDir::new("mismatch");
        fs::create_dir_all(dir.verdicts()).expect("create");
        let path = verdict_path(&dir.verdicts(), "ask-1").expect("path");
        fs::write(
            &path,
            br#"{"version":1,"askId":"ask-2","behavior":"allow"}"#,
        )
        .expect("write");
        assert_eq!(take(&dir.verdicts(), "ask-1"), None);
    }

    #[test]
    fn a_newer_verdict_format_is_skipped_rather_than_guessed_at() {
        let dir = TempDir::new("newer");
        fs::create_dir_all(dir.verdicts()).expect("create");
        let path = verdict_path(&dir.verdicts(), "ask-1").expect("path");
        fs::write(
            &path,
            br#"{"version":99,"askId":"ask-1","behavior":"allow"}"#,
        )
        .expect("write");
        assert_eq!(take(&dir.verdicts(), "ask-1"), None);
    }

    #[test]
    fn an_ask_id_cannot_choose_where_the_verdict_is_written() {
        // The sibling of `hook`'s `a_session_id_cannot_choose_where_the_hook_writes`.
        // The id round-trips through a payload Gritty does not control, so it is
        // a file name and never a path.
        for hostile in ["../../etc/passwd", "/etc/passwd", "..", "a/b", ""] {
            assert_eq!(
                verdict_path(Path::new("/tmp/verdicts"), hostile)
                    .map(|path| path.to_string_lossy().into_owned()),
                match hostile {
                    "../../etc/passwd" => Some("/tmp/verdicts/etcpasswd.json".to_owned()),
                    "/etc/passwd" => Some("/tmp/verdicts/etcpasswd.json".to_owned()),
                    "a/b" => Some("/tmp/verdicts/ab.json".to_owned()),
                    _ => None,
                },
                "{hostile}"
            );
        }
    }

    #[test]
    fn the_reply_is_the_shape_the_agent_was_measured_to_honour() {
        let allow = Verdict {
            version: VERDICT_VERSION,
            ask_id: "ask-1".to_owned(),
            behavior: Behavior::Allow,
            message: None,
        };
        assert_eq!(
            allow.to_claude_code_reply(),
            r#"{"hookSpecificOutput":{"decision":{"behavior":"allow"},"hookEventName":"PermissionRequest"}}"#
        );
        let deny = Verdict {
            behavior: Behavior::Deny,
            message: Some("held by your editor session".to_owned()),
            ..allow
        };
        assert_eq!(
            deny.to_claude_code_reply(),
            r#"{"hookSpecificOutput":{"decision":{"behavior":"deny","message":"held by your editor session"},"hookEventName":"PermissionRequest"}}"#
        );
    }

    #[test]
    fn a_message_is_bounded_before_it_reaches_the_disk() {
        let dir = TempDir::new("bounded");
        write(
            &dir.verdicts(),
            &Verdict {
                version: VERDICT_VERSION,
                ask_id: "ask-1".to_owned(),
                behavior: Behavior::Deny,
                message: Some("x".repeat(10_000)),
            },
        )
        .expect("write");
        let taken = take(&dir.verdicts(), "ask-1").expect("a verdict");
        assert!(
            taken.message.expect("a message").chars().count() <= MAX_HOOK_DETAIL,
            "clipped on the way out, as everything crossing this boundary is"
        );
    }

    #[test]
    fn sweeping_removes_what_nobody_came_back_for() {
        let dir = TempDir::new("sweep");
        dir.answer("ask-1", Behavior::Allow);
        sweep(&dir.verdicts(), Duration::from_secs(60 * 60));
        assert!(
            verdict_path(&dir.verdicts(), "ask-1")
                .expect("path")
                .exists(),
            "a fresh verdict is still somebody's answer"
        );
        sweep(&dir.verdicts(), Duration::ZERO);
        assert!(
            !verdict_path(&dir.verdicts(), "ask-1")
                .expect("path")
                .exists()
        );
    }
}
