//! Process evidence: which agent CLIs are running right now.
//!
//! Two external commands, both read-only, both behind [`CommandRunner`] so no
//! test spawns a subprocess:
//!
//! - `ps -Ao pid=,ppid=,tty=,command=` once per scan, for the whole process
//!   table. One call, not one per candidate.
//! - `lsof -a -p <pid> -Fn` per candidate, for its working directory and any
//!   session file it happens to be holding open.
//!
//! Both are Unix-only, which is why this module is compiled behind `cfg(unix)`
//! for the real runner while the parsing is platform-independent and always
//! tested. On other platforms [`SystemCommandRunner`] returns nothing and the
//! roster degrades to transcript evidence (rule 11: no cross-platform claim
//! that was not tested).
//!
//! `lsof` failing is normal, not exceptional: it exits non-zero for processes
//! owned by another user, and on a machine with many open files it is the
//! slowest thing here. Every call is bounded by a timeout and a failure costs
//! that process its working directory, nothing more.

use std::ffi::OsStr;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Which agent CLI a process or transcript belongs to.
///
/// Only Claude Code is implemented (T220). A second provider is an added
/// variant plus an added arm in [`provider_of_command`] and in the transcript
/// scanner's root list — deliberately not a trait, because one match arm is
/// cheaper to read than an abstraction with a single implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Provider {
    ClaudeCode,
}

impl Provider {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ClaudeCode => "CLAUDE_CODE",
        }
    }
}

/// What the process table and `lsof` could tell us about one running agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessEvidence {
    pub provider: Provider,
    pub pid: u32,
    /// `/dev/ttys003`, or `None` for a process with no controlling terminal
    /// (an agent launched by a desktop app rather than a shell).
    pub tty: Option<String>,
    pub working_directory: Option<PathBuf>,
    /// Only when the process happened to be holding the file open. Claude Code
    /// appends and closes, so this is usually `None` — the working directory is
    /// the reliable join key, not this.
    pub transcript_path: Option<PathBuf>,
    /// Recovered from `--resume` / `--session-id` on the command line, or from
    /// the transcript path when one was open. Usually `None`.
    pub session_id: Option<String>,
}

/// Runs a short read-only command and returns its stdout, or `None` if it
/// failed, timed out or does not exist.
pub trait CommandRunner: Send + Sync {
    fn run(&self, program: &str, args: &[&str], timeout: Duration) -> Option<String>;
}

/// Timeout for the single process-table read.
const PS_TIMEOUT: Duration = Duration::from_millis(1_500);
/// Timeout for one `lsof`. Deliberately tight: it runs once per candidate, and
/// a missing working directory is a much better outcome than a stalled scan.
const LSOF_TIMEOUT: Duration = Duration::from_millis(400);
/// How often the timeout loop wakes while waiting for a child.
const POLL_STEP: Duration = Duration::from_millis(10);

/// The real runner. Spawns the child with no stdin and a discarded stderr, and
/// kills it if it outlives its timeout.
///
/// stdout is drained by a thread rather than read after the wait. `ps -A` on a
/// busy machine writes far more than a pipe buffer holds, and a child blocked
/// on a full pipe never exits — waiting first and reading afterwards deadlocks
/// until the timeout and then reports nothing, which looks exactly like "no
/// agents are running".
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemCommandRunner;

impl CommandRunner for SystemCommandRunner {
    fn run(&self, program: &str, args: &[&str], timeout: Duration) -> Option<String> {
        if !cfg!(unix) {
            return None;
        }
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;

        let mut stdout = child.stdout.take()?;
        let reader = std::thread::spawn(move || {
            let mut buffer = Vec::new();
            let _ = stdout.read_to_end(&mut buffer);
            buffer
        });

        let deadline = Instant::now() + timeout;
        let mut timed_out = false;
        loop {
            match child.try_wait() {
                // Exit status is ignored on purpose: `lsof` reports partial
                // results with a non-zero status all the time, and partial
                // results are exactly what we want.
                Ok(Some(_)) => break,
                Ok(None) if Instant::now() >= deadline => {
                    // Killing closes the pipe, which is what lets the reader
                    // thread finish instead of leaking.
                    let _ = child.kill();
                    let _ = child.wait();
                    timed_out = true;
                    break;
                }
                Ok(None) => std::thread::sleep(POLL_STEP),
                Err(_) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    timed_out = true;
                    break;
                }
            }
        }

        let output = reader.join().ok()?;
        if timed_out {
            tracing::debug!(program, "process probe timed out");
            return None;
        }
        Some(String::from_utf8_lossy(&output).into_owned())
    }
}

/// One row of the process table, before it is classified.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ProcessRow {
    pid: u32,
    tty: Option<String>,
    command: String,
}

/// Every agent process the machine is running, with whatever `lsof` could add.
///
/// `None` means the process table itself could not be read — which is *not*
/// the same as "nothing is running", and the difference has to survive all the
/// way to the UI so it can say "cannot see processes" instead of "all quiet".
pub fn discover_processes(
    runner: &dyn CommandRunner,
    owned: &crate::OwnedSessions,
) -> Option<Vec<ProcessEvidence>> {
    let table = runner.run("/bin/ps", &["-Ao", "pid=,ppid=,tty=,command="], PS_TIMEOUT);
    let Some(table) = table else {
        tracing::debug!("process table unavailable; falling back to transcript evidence");
        return None;
    };

    let mut found = Vec::new();
    for row in parse_process_table(&table) {
        let Some(provider) = provider_of_command(&row.command) else {
            continue;
        };

        // Before `lsof`, not after: we already know from the command line
        // whether this is our own child, and skipping the probe for it is both
        // honest and free — `lsof` costs a subprocess and a timeout each.
        if owned.owns_process(row.pid, session_id_of_command(&row.command).as_deref()) {
            tracing::trace!(
                pid = row.pid,
                "skipping a process the caller already reports"
            );
            continue;
        }

        let lsof = runner.run(
            "/usr/sbin/lsof",
            &["-a", "-p", &row.pid.to_string(), "-Fn"],
            LSOF_TIMEOUT,
        );
        let working_directory = lsof.as_deref().and_then(working_directory_of);
        let transcript_path = lsof.as_deref().and_then(open_transcript_of);

        // A subagent runs in a worktree the parent owns. It is the parent's
        // activity, not a session of its own, so it never gets a row.
        if working_directory
            .as_deref()
            .is_some_and(is_subagent_worktree)
        {
            continue;
        }

        let session_id = transcript_path
            .as_deref()
            .and_then(session_id_of_transcript)
            .or_else(|| session_id_of_command(&row.command));

        found.push(ProcessEvidence {
            provider,
            pid: row.pid,
            tty: row.tty,
            working_directory,
            transcript_path,
            session_id,
        });
    }
    Some(found)
}

/// `pid ppid tty command…`, with the command keeping its own spaces.
///
/// The columns are padded, so consecutive spaces are one separator, not
/// several empty fields — splitting per whitespace *character* shifts every
/// column right and makes the tty look like a command.
fn parse_process_table(table: &str) -> Vec<ProcessRow> {
    table
        .lines()
        .filter_map(|line| {
            let mut rest = line.trim_start();
            let mut columns = ["", "", ""];
            for column in &mut columns {
                let end = rest.find(char::is_whitespace)?;
                *column = &rest[..end];
                rest = rest[end..].trim_start();
            }
            let [pid, _ppid, tty] = columns;
            let pid = pid.parse().ok()?;
            let command = rest.trim_end();
            (!command.is_empty()).then(|| ProcessRow {
                pid,
                tty: normalize_tty(tty),
                command: command.to_owned(),
            })
        })
        .collect()
}

/// `??` is `ps` for "no controlling terminal".
fn normalize_tty(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "??" || trimmed == "?" {
        return None;
    }
    if trimmed.starts_with("/dev/") {
        return Some(trimmed.to_owned());
    }
    Some(format!("/dev/{trimmed}"))
}

/// Classifies a `ps` command line.
///
/// Only the executable is considered, never the whole line. Matching anywhere
/// in the command would classify every shell that merely *mentions* an agent —
/// and Claude Code sources a snapshot from `~/.claude/` in each of its own
/// shells, so a substring rule reports the agent's children as more agents.
fn provider_of_command(command: &str) -> Option<Provider> {
    let executable = command.split_whitespace().next()?;
    let name = Path::new(executable).file_name().and_then(OsStr::to_str)?;
    (name == "claude").then_some(Provider::ClaudeCode)
}

fn is_subagent_worktree(path: &Path) -> bool {
    path.components()
        .any(|part| part.as_os_str() == "worktrees")
        && path.components().any(|part| part.as_os_str() == ".claude")
}

/// `lsof -Fn` emits records as `f<fd>` followed by `n<name>`; the working
/// directory is the name that follows the `cwd` descriptor.
fn working_directory_of(lsof: &str) -> Option<PathBuf> {
    let mut lines = lsof.lines();
    while let Some(line) = lines.next() {
        if line != "fcwd" {
            continue;
        }
        let name = lines.next()?.strip_prefix('n')?.trim();
        if name.starts_with('/') {
            return Some(PathBuf::from(name));
        }
    }
    None
}

/// A session file the process is holding open, when it happens to be.
fn open_transcript_of(lsof: &str) -> Option<PathBuf> {
    lsof.lines()
        .filter_map(|line| line.strip_prefix('n'))
        .map(str::trim)
        .find(|name| name.contains("/.claude/projects/") && name.ends_with(".jsonl"))
        .map(PathBuf::from)
}

/// Claude Code names each transcript after its session id.
fn session_id_of_transcript(path: &Path) -> Option<String> {
    let stem = path.file_stem().and_then(OsStr::to_str)?;
    is_uuid(stem).then(|| stem.to_ascii_lowercase())
}

/// `--resume <id>`, `--resume=<id>`, `--session-id <id>`, `-r <id>`.
fn session_id_of_command(command: &str) -> Option<String> {
    let tokens: Vec<&str> = command.split_whitespace().collect();
    for (index, token) in tokens.iter().enumerate() {
        let candidate = match *token {
            "--resume" | "--session-id" | "-r" => tokens.get(index + 1).copied(),
            _ => token
                .split_once('=')
                .filter(|(flag, _)| *flag == "--resume" || *flag == "--session-id")
                .map(|(_, value)| value),
        };
        if let Some(value) = candidate.filter(|value| is_uuid(value)) {
            return Some(value.to_ascii_lowercase());
        }
    }
    None
}

/// `8-4-4-4-12` hex. Written out rather than pulled from `uuid` because this
/// only ever validates a shape that came off a path or a command line.
fn is_uuid(value: &str) -> bool {
    const GROUPS: [usize; 5] = [8, 4, 4, 4, 12];
    let mut groups = value.split('-');
    for width in GROUPS {
        let Some(group) = groups.next() else {
            return false;
        };
        if group.len() != width || !group.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return false;
        }
    }
    groups.next().is_none()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use crate::OwnedSessions;

    use super::*;

    /// Replays captured command output; nothing here spawns a process.
    #[derive(Default)]
    struct FakeRunner {
        responses: HashMap<String, String>,
        calls: Mutex<Vec<String>>,
    }

    impl FakeRunner {
        fn with(pairs: &[(&str, &str)]) -> Self {
            Self {
                responses: pairs
                    .iter()
                    .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                    .collect(),
                calls: Mutex::new(Vec::new()),
            }
        }
    }

    impl CommandRunner for FakeRunner {
        fn run(&self, program: &str, args: &[&str], _timeout: Duration) -> Option<String> {
            let key = format!("{program} {}", args.join(" "));
            self.calls.lock().expect("calls").push(key.clone());
            self.responses.get(&key).cloned()
        }
    }

    const PS_KEY: &str = "/bin/ps -Ao pid=,ppid=,tty=,command=";

    /// Real output from a machine running three sessions, including the shell
    /// snapshot line that a substring match would misclassify.
    const PS_OUTPUT: &str = concat!(
        " 7789  4908 ??       /bin/zsh -c source /Users/a/.claude/shell-snapshots/snapshot-zsh.sh\n",
        " 5815 22161 ttys000  claude\n",
        " 6942  6463 ttys002  claude --resume 11111111-2222-3333-4444-555555555555\n",
        "  501     1 ??       /usr/bin/login -pf someone\n",
    );

    fn lsof_key(pid: u32) -> String {
        format!("/usr/sbin/lsof -a -p {pid} -Fn")
    }

    #[test]
    fn process_table_splits_pid_tty_and_command() {
        let rows = parse_process_table(PS_OUTPUT);
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[1].pid, 5815);
        assert_eq!(rows[1].tty.as_deref(), Some("/dev/ttys000"));
        assert_eq!(rows[1].command, "claude");
        // No controlling terminal stays absent rather than becoming "/dev/??".
        assert_eq!(rows[0].tty, None);
    }

    #[test]
    fn process_table_tolerates_blank_and_short_lines() {
        assert!(parse_process_table("\n  \nnot-a-row\n1 2\n").is_empty());
    }

    #[test]
    fn only_the_executable_classifies_a_process() {
        assert_eq!(provider_of_command("claude"), Some(Provider::ClaudeCode));
        assert_eq!(
            provider_of_command("/Users/a/.local/bin/claude --resume x"),
            Some(Provider::ClaudeCode)
        );
        // The regression this rule exists for: a shell that merely mentions
        // ~/.claude is not an agent.
        assert_eq!(
            provider_of_command("/bin/zsh -c source /Users/a/.claude/shell-snapshots/snapshot.sh"),
            None
        );
        assert_eq!(provider_of_command("codex"), None);
        assert_eq!(provider_of_command(""), None);
    }

    #[test]
    fn working_directory_comes_from_the_cwd_descriptor() {
        let lsof = "p5815\nfcwd\nn/Users/a/project\nftxt\nn/usr/bin/node\n";
        assert_eq!(
            working_directory_of(lsof),
            Some(PathBuf::from("/Users/a/project"))
        );
        assert_eq!(working_directory_of("p5815\nftxt\nn/usr/bin/node\n"), None);
        // A relative name is not a working directory.
        assert_eq!(working_directory_of("fcwd\nnrelative/path\n"), None);
    }

    #[test]
    fn open_transcript_is_found_when_held_open() {
        let lsof = "fcwd\nn/Users/a/p\nf12\nn/Users/a/.claude/projects/-Users-a-p/abc.jsonl\n";
        assert_eq!(
            open_transcript_of(lsof),
            Some(PathBuf::from(
                "/Users/a/.claude/projects/-Users-a-p/abc.jsonl"
            ))
        );
        assert_eq!(open_transcript_of("fcwd\nn/Users/a/p\n"), None);
    }

    #[test]
    fn session_id_is_recovered_from_path_or_command_line() {
        let id = "11111111-2222-3333-4444-555555555555";
        assert_eq!(
            session_id_of_transcript(Path::new(&format!("/x/{id}.jsonl"))),
            Some(id.to_owned())
        );
        assert_eq!(session_id_of_transcript(Path::new("/x/notes.jsonl")), None);
        assert_eq!(
            session_id_of_command(&format!("claude --resume {id}")),
            Some(id.to_owned())
        );
        assert_eq!(
            session_id_of_command(&format!("claude --session-id={id}")),
            Some(id.to_owned())
        );
        assert_eq!(session_id_of_command("claude --resume nonsense"), None);
        assert_eq!(session_id_of_command("claude"), None);
    }

    #[test]
    fn uuid_shape_is_checked_strictly() {
        assert!(is_uuid("11111111-2222-3333-4444-555555555555"));
        assert!(!is_uuid("11111111-2222-3333-4444-5555555555555"));
        assert!(!is_uuid("11111111-2222-3333-4444"));
        assert!(!is_uuid("gggggggg-2222-3333-4444-555555555555"));
        assert!(!is_uuid("11111111-2222-3333-4444-555555555555-6"));
    }

    #[test]
    fn subagent_worktrees_are_not_sessions() {
        assert!(is_subagent_worktree(Path::new(
            "/Users/a/p/.claude/worktrees/agent-1"
        )));
        assert!(!is_subagent_worktree(Path::new("/Users/a/p")));
        assert!(!is_subagent_worktree(Path::new("/Users/a/worktrees/x")));
    }

    #[test]
    fn discovery_joins_the_process_table_with_lsof() {
        let runner = FakeRunner::with(&[
            (PS_KEY, PS_OUTPUT),
            (&lsof_key(5815), "fcwd\nn/Users/a/one\n"),
            (
                &lsof_key(6942),
                "fcwd\nn/Users/a/two\nf9\nn/Users/a/.claude/projects/-Users-a-two/11111111-2222-3333-4444-555555555555.jsonl\n",
            ),
        ]);

        let found = discover_processes(&runner, &OwnedSessions::default()).expect("process table");

        assert_eq!(found.len(), 2, "only the two claude rows are agents");
        assert_eq!(found[0].pid, 5815);
        assert_eq!(
            found[0].working_directory,
            Some(PathBuf::from("/Users/a/one"))
        );
        assert_eq!(found[0].session_id, None, "no evidence, so no guess");
        assert_eq!(
            found[1].session_id.as_deref(),
            Some("11111111-2222-3333-4444-555555555555")
        );
        assert_eq!(found[1].tty.as_deref(), Some("/dev/ttys002"));
    }

    #[test]
    fn a_process_survives_lsof_failing() {
        let runner = FakeRunner::with(&[(PS_KEY, " 5815 1 ttys000  claude\n")]);
        let found = discover_processes(&runner, &OwnedSessions::default()).expect("process table");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].working_directory, None);
        assert_eq!(found[0].transcript_path, None);
    }

    #[test]
    fn no_process_table_yields_no_evidence_and_no_panic() {
        let runner = FakeRunner::default();
        assert_eq!(
            discover_processes(&runner, &OwnedSessions::default()),
            None,
            "unreadable, not empty"
        );
    }

    #[test]
    fn subagent_processes_are_skipped() {
        let runner = FakeRunner::with(&[
            (PS_KEY, " 5815 1 ttys000  claude\n"),
            (
                &lsof_key(5815),
                "fcwd\nn/Users/a/p/.claude/worktrees/agent-7\n",
            ),
        ]);
        assert!(
            discover_processes(&runner, &OwnedSessions::default())
                .expect("process table")
                .is_empty()
        );
    }
}
