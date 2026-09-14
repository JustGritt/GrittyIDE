# Task PTY-FLAKE — Closing a terminal leaks background jobs, and the flake was the symptom

- **Phase:** 2
- **Status:** done
- **Depends on:** T090 (PTY spike, on `main`)

## Objective

`pty.rs::kill_terminates_the_whole_process_tree` failed intermittently under
load. The ticket was written as a flake fix. It is not one: probing the
escalation path showed the production kill path **permanently leaks orphaned
process groups**, and the intermittent test was the only thing reporting it.

## What was actually wrong

`escalate()` (`crates/terminal/src/system.rs`) signalled only
`target_groups()` — the shell's own pid/pgid plus whatever `tcgetpgrp`
reported as the pty's *foreground* group. A backgrounded job is never the
foreground group, so reaching it depended entirely on the shell forwarding
SIGHUP to its job table. That is not a guarantee:

- **zsh sometimes exits without forwarding.** 60 suite runs under 24 spinners
  and 2 fork loops on 8 cores (load 45–65): **17 failures (28%)**.
- **One case leaks every time, even idle.** `( sleep 300 & )` leaves the job in
  the subshell's process group with nothing in the shell's job table referring
  to it, so no forwarding could ever reach it — **6/6 idle, 60/60 under load**.

Two hypotheses recorded on the ticket were **refuted by measurement**, not
argued away. `ESCALATION_STEP` plays no part: on failing and passing runs
alike the only signal ever sent is the first SIGHUP, and the shell is reaped
~9 ms later, so the timer is never reached. And the survivor is not a zombie
mistaken for a live process — `ps` state `SN`, ppid 1, still running minutes
later.

## Required behavior

1. **`session_groups()`** reads the process table once *before* the first
   signal — it has to be before, because afterwards the children are reparented
   and the session is undiscoverable — and keeps every group whose members share
   the shell's session id. Membership is asked of the kernel via
   `getsid`/`getpgid`, never parsed out of `ps` output.
2. **`escalate()` does not stop at the shell.** Enumeration alone was not
   enough: 150 runs still gave 9 failures, with the probe showing the right
   group *was* found and signalled. A job forked moments earlier can sit between
   `fork` and `execve`, where a delivered SIGHUP may be blocked or ignored, and
   both states survive the exec. The ladder now runs until the shell is reaped
   **and** every known group is empty, so SIGTERM and SIGKILL follow up.

`ps` is a subprocess because there is no portable syscall for the process
table: Linux has `/proc`, macOS has a `sysctl` needing `unsafe`, and the
workspace denies `unsafe_code`. It mirrors `crates/agents/src/probe.rs` and
degrades to the previous two groups on failure. No new dependencies.

## Acceptance criteria

| Condition | Result |
|---|---|
| Before, 60 suite runs, load 45–65 | 17/60 flaky test · 60/60 detached-job leak |
| Enumeration only, 150 runs, load ~55 | 9 failures |
| Both changes, 250 runs, 56 spinners + 4 fork loops, load 8→112 | **0** |
| Both, exact committed source, 200 runs, load 47–70 | **0** |

`kill_reaches_a_job_detached_from_job_control` is the deterministic regression
test: it fails against the pre-fix `system.rs` naming the survivor, and passes
after. `kill_terminates_the_whole_process_tree` keeps its scenario and gains an
honest failure — it prints the survivor's `ps` line, distinguishes an unreaped
zombie from a leak, and SIGKILLs a survivor before asserting so a failure stops
poisoning the machine.

## Open, and not for an agent to decide

1. **This narrows `nohup`.** Ignoring SIGHUP no longer outlives a terminal
   close; leaving the session (`setsid`) still does. That is a product
   decision about what closing a terminal in an agent cockpit means, and it
   needs Alexis' sign-off rather than a commit message.
2. **Four documents now describe the old two-group behaviour** and are outside
   lane T, which owns only ADR-017: ADR-015 §Termination ("over *two* process
   groups"), `docs/architecture/terminal.md:61-64`, `security.md:163-164`,
   `overview.md:153-154`.
3. **Not verified on Linux** — no machine here, and CI is dark until the
   minute allowance resets. The `ps -Ao pid=` invocation and the `getsid`
   filter are POSIX by construction only.

## Completion report

Implemented on branch `pty-flake`. Files changed:
`crates/terminal/src/system.rs`, `crates/terminal/tests/pty.rs`. No new
dependencies, no `unsafe`, no protocol version, no migration.

Tests executed: `cargo test -p gritty-terminal --test pty` — 17 passed;
`pnpm check` — exit 0 (verified against the real exit status, not a piped
`tail`). The regression test was confirmed to fail against `e8ada81`'s
`system.rs` and pass against the fix.

Follow-ups: the four stale documents above; Linux verification; and the
observation that a leaked `sleep 300` from another agent's `cargo test` run was
caught in the wild during this work, which is the ticket's premise about wrong
conclusions actually happening.
