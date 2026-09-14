# ADR-001: Desktop runtime is Tauri 2

- **Status:** Accepted
- **Date:** 2026-09-02
- **Accepted:** 2026-09-09 by Alexis, on the Phase 0 measurements in
  `docs/plans/phase-0-spike.md` §C and on sustained development use. The
  acceptance is deliberately narrower than the original go/no-go: see
  *What this acceptance rests on*, which records what was measured, what was
  only automated, and what was never evaluated. Phase 2 is unblocked.

## Context

Gritty is a long-running IDE that will host agents, terminals, language servers
and indexers. Baseline memory, process supervision and a strict UI/core boundary
matter more than web-platform parity. The alternatives are Electron (full
Chromium, larger footprint, uniform WebView) and Tauri (system WebView, Rust
core, smaller footprint, WebView divergence risk).

## Decision

Use Tauri 2 with a Rust core and a Monaco frontend. The Phase 0 spike
(`docs/plans/phase-0-spike.md`) was the proving ground: WebView/Monaco
behavior, filesystem boundary, typed protocol, PTY, Markdown, persistence,
failure containment, Linux build, performance baseline. What it actually
answered, and what it did not, is recorded below.

The frontend half of that sentence is not decided here, and on review was
never argued here either: ADR-032 records why React, and why Monaco being
framework-agnostic means the two choices were always separable. The
go/no-go below is about the runtime alone.

Kill criteria (any one triggers an Electron evaluation while keeping the core
protocol):

- recurring Monaco or text-input defects on the primary platform (macOS WKWebView);
- unstable PTY/process behavior needing invasive workarounds;
- unacceptable maintenance caused by WebView divergence between macOS and Linux.

## What this acceptance rests on

The original exit condition was a completed Phase 0 with every kill criterion
answered. That is not what happened, and recording the gap is worth more than
a clean-looking status field (hard rule 12).

**Measured, and strong.** Cold launch 496 ms against a 2 s budget, warm 504 ms,
idle CPU 0.064 % of one core, `documents.open` 0.23 ms median, `workspace.list`
0.29 ms median over a 10 501-file fixture, 607 MiB RSS against Cursor's
1194 MiB on the same workspace — 51 % of the memory and 29 % of the idle CPU of
the Electron editor a user would otherwise run (`phase-0-spike.md` §C). This is
the half of the original question that Electron was the alternative to, and
Tauri wins it by a margin no WebView defect found so far would overturn.

**Automated, and green.** The filesystem boundary, the typed protocol, the
document/buffer contract, PTY lifecycle and process-tree kill, session
persistence and crash recovery all carry core and adapter tests that pass.

**Never evaluated — and this is the honest part.** `docs/testing/platform-matrix.md`
has 43 data rows and **not one of them is marked `✔`**; the only `✔` in the file
is in the legend. 17 rows are `–` (untested); the rest read "core and adapter
tests … pending the human pass". So of the three kill criteria:

1. *Recurring Monaco or text-input defects on macOS WKWebView* — **not
   evaluated.** Every Input row (shortcuts, multi-cursor, clipboard, IME and
   dead keys, drag and drop) and every Rendering row is untested by a person.
   No defect has been found; nothing has looked.
2. *Unstable PTY/process behaviour needing invasive workarounds* — **partly
   evaluated; one real defect found, and fixed inside the adapter.** The
   behaviour is covered by core tests. *Corrected 2026-09-14, before this
   acceptance was committed:* the 9 September text called `PTY-FLAKE`
   (`kill_terminates_the_whole_process_tree` flaky under load) a fixed wait in
   a test rather than a product defect. Its investigation showed the opposite:
   closing a terminal **leaked background jobs** — every time for a job detached
   from job control, 28 % of runs under load otherwise — because `escalate()`
   signalled only the shell's own and the foreground process group. The fix
   stayed in `crates/terminal/src/system.rs`: enumerate the session's groups
   before the first signal, and escalate until all are empty
   (`docs/tasks/completed/PTY-FLAKE-process-tree-kill.md`; 0 failures in 450
   runs under load). It needs a `ps` subprocess, because the process table has
   no portable syscall without `unsafe`, and it is unverified on Linux. Whether
   that counts as a workaround is the criterion's question; recorded here as a
   contained adapter fix, not as one.
3. *Unacceptable maintenance from WebView divergence between macOS and Linux* —
   **unevaluable.** Linux CI has been dark since the GitHub Actions allowance
   was exhausted (`CI-LINUX-DARK`; resets 2026-10-01), and the Linux column of
   the matrix is `–` in every row. ADR-002's "Linux stays continuously
   buildable through CI from T000" is not currently true.

Accepting on 1 and 3 unevaluated is a deliberate trade: the ADR was gating
every unstarted Phase 2 task (T070, T080, T100, T110) while the uncertainty it
protected against had stopped being informative — nobody was going to answer
those criteria by leaving the status at Proposed. The kill criteria are not
discarded; they move from a gate into the *Revisit when* below.

**Verification becomes granular rather than a single pass.** The remaining
platform-matrix rows are no longer treated as one blocking session to be run
before Phase 2. Each feature verifies its own rows as it lands, and fills them
in with the observation, the date and the machine. `phase-0-manual-pass.md`
stays as the method and the ordering; it stops being a gate.

## Consequences

- Rust + TypeScript toolchain; contributors need both.
- Platform-specific behavior must sit behind adapters and tests.
- The core protocol (T010) is designed to survive a shell swap.

## Revisit when

The kill criteria survive acceptance as reopening triggers. Any one of them
starts an Electron evaluation, and the core protocol (T010) was designed so
that such an evaluation does not touch UI feature code (ADR-003):

- **recurring Monaco or text-input defects on macOS WKWebView.** Still the
  likeliest place for this to fail, and still unmeasured. The first Input rows
  to be filled in are the evidence, whenever the features owning them land.
- **unstable PTY or process behaviour needing invasive workarounds** — a
  product defect, not a load-sensitive test. `PTY-FLAKE` was shown to be a
  product defect and was fixed within the adapter; a defect that cannot be
  fixed there, or the same leak reappearing on Linux, is the trigger.
- **unacceptable maintenance from WebView divergence between macOS and Linux.**
  This one cannot be observed at all until Linux CI comes back
  (`CI-LINUX-DARK`, 2026-10-01). Until then, do not claim Linux support from
  compilation (hard rule 11), and note that this acceptance is not evidence for
  it.

**Acceptance does not imply Windows readiness**, and does not amend ADR-002:
macOS is primary and dogfooded first, Linux is a Phase 7 hardening pass,
Windows is deferred to Phase 8.
