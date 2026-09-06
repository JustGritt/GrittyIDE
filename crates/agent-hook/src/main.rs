//! The `gritty-agent-hook` executable: read stdin, write one state file, exit
//! `0` — and, for a permission prompt alone, wait for the user's answer and
//! say it (T335, ADR-020).
//!
//! There is no logic here — it lives in the library beside this file, so it can
//! be tested without launching a process. What this file *is* is the promise
//! that nothing an agent runs can fail because of Gritty (ADR-018 §5): the exit
//! code is `0` on every path, including a payload that never arrives, a state
//! directory that cannot be written, a verdict that never comes, and a panic.
//!
//! **Stdout is used on exactly one path.** ADR-018 called stdout "a channel the
//! agent may parse" and wrote nothing to it for that reason. It still writes
//! nothing to it for every event but `PermissionRequest`, and on that event it
//! writes only what the agent asked to be told: a decision, in the shape that
//! agent documents. Silence remains the answer to every failure, because
//! silence is what leaves the agent's own prompt standing.

use std::io::{Read, Write};

use gritty_agent_hook::{Args, parse_args, record_and_answer};

fn main() {
    // A panic inside the hook is still the agent's process reporting a failing
    // hook. Catching it here keeps the exit code at zero, which is the one
    // thing this binary must never get wrong.
    let reply = std::panic::catch_unwind(record).unwrap_or(None);
    if let Some(reply) = reply {
        // Failure to write is silence, which is the safe answer: the agent
        // prompts its own user.
        let mut stdout = std::io::stdout().lock();
        let _ = stdout.write_all(reply.as_bytes());
        let _ = stdout.write_all(b"\n");
        let _ = stdout.flush();
    }
    std::process::exit(0);
}

fn record() -> Option<String> {
    let args: Args = parse_args(std::env::args().skip(1));
    let mut payload = String::new();
    // Bounded by the reader, not by trust: a hook payload is a few kilobytes,
    // and an agent that sent an unbounded one would otherwise be able to make
    // this process grow without limit.
    let mut stdin = std::io::stdin().lock().take(MAX_PAYLOAD_BYTES);
    if stdin.read_to_string(&mut payload).is_err() {
        return None;
    }
    let (_outcome, reply) = record_and_answer(
        &args,
        &payload,
        gritty_agents::now_ms(),
        gritty_agents::now_ms,
        std::thread::sleep,
    );
    reply
}

/// Largest stdin payload read. Claude Code's is well under a kilobyte; this is
/// generous, and it is a bound rather than an expectation.
const MAX_PAYLOAD_BYTES: u64 = 256 * 1024;
