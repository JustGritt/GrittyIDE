//! The command line, which is fixed.
//!
//! Task §1: "The launch is fixed and explicit, never assembled from caller
//! input." Two things vary — the working directory, and the session id Gritty
//! minted — and **no flag does**. There is no way to reach
//! `--dangerously-skip-permissions` or `--allow-dangerously-skip-permissions`
//! from here, because there is no way to add a flag at all.
//!
//! # Every flag, and why it is load-bearing
//!
//! | Flag | Without it |
//! |---|---|
//! | `-p` | an interactive TUI, not a supervised child |
//! | `--input-format stream-json` | one prompt per process; §4's inbox has nothing to write to |
//! | `--output-format stream-json --verbose` | prose on stdout instead of the framed events this crate parses |
//! | `--session-id <uuid>` | the CLI mints an id Gritty does not know, and §8's exclusion set cannot name the child |
//! | `--replay-user-messages` | no `PROMPT_STARTED`, so the console would have to echo optimistically — the thing §9 forbids |
//! | `--permission-prompt-tool stdio` | the run is ungated: no `can_use_tool` arrives and the CLI decides alone |
//!
//! # `--permission-mode`, isolated
//!
//! The task left this open: both runs behind its channel table passed
//! `--permission-mode manual` and "its contribution was **not** isolated; a
//! builder should establish what it does before relying on it or removing it."
//!
//! Established, against `claude` 2.1.263 on darwin 25.6 on 2026-09-08, by
//! running the same prompt twice with only this flag differing:
//!
//! ```text
//! default   extra=(none)                     initMode=default  askArrived=true  sideEffect=false
//! manual    extra=--permission-mode manual   initMode=default  askArrived=true  sideEffect=false
//! ```
//!
//! `manual` is not even echoed back by `system`/`init`, which reports
//! `permissionMode: "default"` either way, and the `can_use_tool` request
//! arrives identically with the flag absent. It contributes nothing observable
//! next to `--permission-prompt-tool stdio`, so it is **not passed**: a flag
//! that changes nothing is a flag a later reader would assume is holding
//! something up.
//!
//! # `--tools` is not passed either
//!
//! For the same reason, one level up. Restricting the tool set is a statement
//! about what this provider may be asked to do, which is T305's question and
//! explicitly not this task's; picking a list here would be inventing a
//! capability model in an argv builder. The gate that actually protects the
//! workspace is `--permission-prompt-tool stdio` plus the deny-by-default
//! verdict, and that one is verified.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::error::ClaudeError;

/// The binary looked up on `PATH` when the caller names none.
pub const DEFAULT_PROGRAM: &str = "claude";

/// Everything the caller gets to decide about a launch.
///
/// Note what is *not* here: a model, a tool list, a permission mode, a system
/// prompt, extra directories, or any other flag. Adding one is a deliberate
/// edit to [`ClaudeLaunch::argv`] with a reason, not a field a command handler
/// can fill from a request.
#[derive(Debug, Clone)]
pub struct ClaudeLaunch {
    program: OsString,
    session_id: String,
    working_directory: PathBuf,
}

impl ClaudeLaunch {
    /// A launch in `working_directory`, under the session id Gritty minted.
    ///
    /// The id is validated here rather than at the spawn so a caller bug
    /// produces `InvalidInput` before a process exists, and so the id this
    /// crate reports back in [`crate::ClaudeReady`] can be compared with the
    /// one that was asked for.
    pub fn new(
        session_id: impl Into<String>,
        working_directory: impl Into<PathBuf>,
    ) -> Result<Self, ClaudeError> {
        let session_id = session_id.into();
        if !is_uuid(&session_id) {
            return Err(ClaudeError::InvalidSessionId { value: session_id });
        }
        Ok(Self {
            program: OsString::from(DEFAULT_PROGRAM),
            session_id,
            working_directory: working_directory.into(),
        })
    }

    /// Run a different binary.
    ///
    /// Not a flag, and deliberately the only thing about the *command* a caller
    /// may change: a user whose `claude` is not on `PATH` needs this, and the
    /// process tests need a stub child that behaves like the CLI without being
    /// it. Everything after `argv[0]` stays fixed.
    #[must_use]
    pub fn with_program(mut self, program: impl Into<OsString>) -> Self {
        self.program = program.into();
        self
    }

    pub fn program(&self) -> &OsString {
        &self.program
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn working_directory(&self) -> &Path {
        &self.working_directory
    }

    /// The arguments after `argv[0]`, in order.
    ///
    /// Returned as owned strings so a test can assert on the whole line
    /// without spawning anything — which is how `the_launch_line_is_fixed`
    /// keeps a future edit from quietly widening it.
    pub fn argv(&self) -> Vec<String> {
        vec![
            "-p".to_owned(),
            "--input-format".to_owned(),
            "stream-json".to_owned(),
            "--output-format".to_owned(),
            "stream-json".to_owned(),
            "--verbose".to_owned(),
            "--session-id".to_owned(),
            self.session_id.clone(),
            "--replay-user-messages".to_owned(),
            "--permission-prompt-tool".to_owned(),
            "stdio".to_owned(),
        ]
    }
}

/// The 8-4-4-4-12 hex shape, and nothing more.
///
/// Hand-rolled rather than pulling `uuid` in: this crate never *mints* an id —
/// Gritty does, task §8 depends on it — so all it needs is to refuse a value
/// the CLI would reject, and a parser that also generates would be more
/// surface than the question deserves.
fn is_uuid(value: &str) -> bool {
    const GROUPS: [usize; 5] = [8, 4, 4, 4, 12];
    let mut parts = value.split('-');
    for width in GROUPS {
        let Some(part) = parts.next() else {
            return false;
        };
        if part.len() != width || !part.bytes().all(|b| b.is_ascii_hexdigit()) {
            return false;
        }
    }
    parts.next().is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn launch() -> ClaudeLaunch {
        ClaudeLaunch::new("3f8a1c2e-9b40-4d17-8e55-6a2b7c4d9f10", "/workspace/demo")
            .expect("a valid uuid")
    }

    /// The whole point of this module. If this test needs changing, the change
    /// is a decision about what Gritty lets an agent do, and it belongs in a
    /// task file rather than in a diff.
    #[test]
    fn the_launch_line_is_fixed() {
        assert_eq!(
            launch().argv(),
            vec![
                "-p",
                "--input-format",
                "stream-json",
                "--output-format",
                "stream-json",
                "--verbose",
                "--session-id",
                "3f8a1c2e-9b40-4d17-8e55-6a2b7c4d9f10",
                "--replay-user-messages",
                "--permission-prompt-tool",
                "stdio",
            ]
        );
    }

    /// Rule: no flag that reaches permission bypass, ever. Asserted rather
    /// than reviewed, because the failure mode is silent.
    #[test]
    fn nothing_in_the_launch_line_can_skip_permissions() {
        let argv = launch().argv().join(" ");
        assert!(!argv.contains("dangerously"), "{argv}");
        assert!(!argv.contains("bypass"), "{argv}");
        assert!(!argv.contains("--permission-mode"), "{argv}");
        assert!(!argv.contains("--allowedTools"), "{argv}");
        assert!(!argv.contains("--add-dir"), "{argv}");
        // §"Explicit non-goals": a partial message is the unbounded
        // high-frequency forwarding rule 8 forbids.
        assert!(!argv.contains("--include-partial-messages"), "{argv}");
        // Taking over an observed session is T310-RESUME, filed not built.
        assert!(!argv.contains("--resume"), "{argv}");
        assert!(!argv.contains("--fork-session"), "{argv}");
    }

    #[test]
    fn the_default_program_is_looked_up_on_path() {
        assert_eq!(launch().program(), "claude");
        assert_eq!(
            launch().with_program("/opt/claude").program(),
            "/opt/claude"
        );
    }

    #[test]
    fn a_session_id_that_is_not_a_uuid_is_refused_before_anything_spawns() {
        for bad in [
            "",
            "nope",
            "3f8a1c2e9b404d178e556a2b7c4d9f10",
            "3f8a1c2e-9b40-4d17-8e55-6a2b7c4d9f1",
            "3f8a1c2e-9b40-4d17-8e55-6a2b7c4d9f10-extra",
            "3f8a1c2e-9b40-4d17-8e55-6a2b7c4d9g10",
            // A flag smuggled in through the one caller-supplied argv value.
            "--dangerously-skip-permissions",
        ] {
            let error = ClaudeLaunch::new(bad, "/workspace/demo")
                .expect_err("{bad} should not be accepted");
            assert_eq!(error.core_error_code(), "InvalidInput", "{bad}");
        }
    }

    #[test]
    fn a_uuid_is_accepted_in_either_case() {
        assert!(ClaudeLaunch::new("3F8A1C2E-9B40-4D17-8E55-6A2B7C4D9F10", "/w").is_ok());
        assert!(ClaudeLaunch::new("3f8a1c2e-9b40-4d17-8e55-6a2b7c4d9f10", "/w").is_ok());
    }
}
