//! The `gritty-agent-hook` executable: read stdin, write one state file, exit
//! `0`.
//!
//! There is no logic here — it lives in the library beside this file, so it can
//! be tested without launching a process. What this file *is* is the promise
//! that nothing an agent runs can fail because of Gritty (ADR-018 §5): the exit
//! code is `0` on every path, including a payload that never arrives, a state
//! directory that cannot be written, and a panic.

use std::io::Read;

use gritty_agent_hook::{Args, parse_args, run};

fn main() {
    // A panic inside the hook is still the agent's process reporting a failing
    // hook. Catching it here keeps the exit code at zero, which is the one
    // thing this binary must never get wrong.
    let outcome = std::panic::catch_unwind(record);
    if let Err(_panic) = outcome {
        // Deliberately silent: anything on stderr is noise in somebody else's
        // terminal, and stdout is a channel the agent may parse.
    }
    std::process::exit(0);
}

fn record() {
    let args: Args = parse_args(std::env::args().skip(1));
    let mut payload = String::new();
    // Bounded by the reader, not by trust: a hook payload is a few kilobytes,
    // and an agent that sent an unbounded one would otherwise be able to make
    // this process grow without limit.
    let mut stdin = std::io::stdin().lock().take(MAX_PAYLOAD_BYTES);
    if stdin.read_to_string(&mut payload).is_err() {
        return;
    }
    let _ = run(&args, &payload, gritty_agents::now_ms());
}

/// Largest stdin payload read. Claude Code's is well under a kilobyte; this is
/// generous, and it is a bound rather than an expectation.
const MAX_PAYLOAD_BYTES: u64 = 256 * 1024;
