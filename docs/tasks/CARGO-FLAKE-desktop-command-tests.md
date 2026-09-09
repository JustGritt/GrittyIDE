# Task CARGO-FLAKE — three desktop command tests fail under a loaded machine

- **Phase:** 2
- **Status:** todo
- **Depends on:** –

Sibling of `MONACO-JSDOM-FLAKE` / `MARKDOWN-BUDGET-FLAKE` and `PTY-FLAKE`, in
the Rust suite rather than the TypeScript one. Filed with **two sightings from
two sessions**, and — importantly — **without a diagnosis**, because the one
that suggested itself could not be separated from a confound.

## What was seen

**Sighting 1 (quick-open lane, 2026-09-09).** Under a full
`cargo test --workspace`:

```
gritty-desktop --test commands::root_removed_while_open_fails_with_not_found_and_closes_the_workspace
```

failed once, then passed in isolation and on the next full run.

**Sighting 2 (settings lane, 2026-09-09, ~2h later).** Under a full
`cargo test --workspace`, three failures together:

```
root_removed_while_open_fails_with_not_found_and_closes_the_workspace
session_recovery_round_trip_through_the_adapters      (commands.rs:145)
closing_a_workspace_interrupts_the_runs_inside_it     (commands.rs:1758)
```

All three then passed:

- individually (`--test commands <name>`), and
- as the whole binary (`cargo test -p gritty-desktop --test commands`) —
  **56 passed, 0 failed**.

Both were assertion panics, not I/O errors.

## Why there is no diagnosis here

The obvious reading is load: three agent sessions were running `cargo` against
one `target/` simultaneously, and `Blocking waiting for file lock on build
directory` was in the output. That matches the family.

**But the second sighting happened while the disk was nearly full** — `df`
showed 396 MiB free of 460 GB, and another session hit
`ld: write() failed, errno=28` and `rustc-LLVM ERROR: IO failure on output
stream` in the same window. So "the machine was busy" and "the machine was out
of space" cannot be told apart after the fact, and the conditions are not
reproducible now that 20 GB has been freed.

A load diagnosis was drafted and **withdrawn** rather than recorded. It would
have been absorbed by the existing flake family without ever being tested,
which is the failure mode this section exists to prevent: an untested
explanation inherited as fact is worse than an open question.

## The confound was much larger than "three cargo runs" (2026-09-09, later)

Both sightings happened while **16 orphaned processes were saturating every
core**, and had been for two days.

They were `zsh` busy-loops — `for i in $(seq 1 $((NCPU * 2))); do (while :; do
:; done) & done` — spawned by a dead session deliberately generating load to
reproduce `MARKDOWN-BUDGET-FLAKE`. The script's own `kill $LOADPIDS` cleanup
line never ran, so the loops were reparented to init and kept running.
`ps` showed each at 40–65% CPU with an elapsed time of `02-00:12`, and
**load average was 66**. Killing them dropped the one-minute average to 17
within five minutes.

So the earlier note is right that the diagnosis was untested, and understates
the problem: the load was neither "three sessions running cargo" nor disk
pressure, but a **deliberate load generator nobody turned off**. Any timing- or
scheduler-sensitive test run on this machine between 2026-09-07 and 2026-09-09
was measured against a machine with 16 spinning cores, and that includes both
sightings here — and very likely some of the sightings behind
`MONACO-JSDOM-FLAKE`, `MARKDOWN-BUDGET-FLAKE` and `PTY-FLAKE`, since the
generator was created to feed exactly that investigation.

`SIGTERM` does not stop a tight `while :; do :; done` promptly — the loop never
reaches a signal check between iterations. It took roughly two minutes to die.

**This does not prove the three tests are sound.** It removes the confound that
made the evidence unreadable, which is what the section below now asks for.

## What would settle it

1. Reproduce on a **quiet machine with space** — no load generators, load
   average near idle, tens of GB free. As of 2026-09-09 16:15 that is finally
   possible: the loops are dead and a sweep took free space from 18 GiB to
   76 GiB. If the three pass repeatedly there, this ticket closes as "the
   machine was lying", and the note above is the finding.
2. If they do not, reproduce with **space pressure only** — a single
   `cargo test --workspace` against a nearly-full volume.
3. Only then read the tests. All three touch workspace teardown
   (`root_removed…`, `closing_a_workspace…`) or the session adapters, so a
   shared temp-directory or timing assumption is the thing to look for — but
   look after the environment is ruled in or out, not before.

`ADR-024` (target retention, `Proposed`) is upstream of the disk half: a swept
`target/` makes condition 2 much rarer. It is undecided, and has now cost time
on three separate occasions.

## Explicit non-goals

- Do not "fix" the three tests by adding sleeps or widening timeouts. That is
  what `MONACO-JSDOM-FLAKE` records as the wrong answer: it hides the signal
  rather than making the test measure what it claims to.
- Do not fold these into `VITEST-FLAKES-load-sensitive`. Different runner,
  different suite, and — unlike those two — **no wall-clock assertion has been
  identified** in any of the three. The family resemblance is a hypothesis.

## Acceptance criteria

- Either a reproduction that names the cause, or a recorded negative result
  saying which of the two conditions was ruled out and how.
- If it is the tests: they pass under load and under space pressure, and the
  assertion that was environment-sensitive is named.

## Tests required

Whatever the diagnosis implies. Nothing to add until there is one.

## Completion report

*(filled on completion)*
