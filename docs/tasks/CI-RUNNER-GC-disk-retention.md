# Task CI-RUNNER-GC — The only CI machine has 4 GB left and nothing ever frees it

- **Phase:** Ops (cross-cutting; not on the phase/gate roadmap)
- **Status:** review
- **Depends on:** – (no upstream contract gate; motivated by CI-COST and
  sharpened by CI-LINUX-DARK — see Motivation)

## Objective

Give the self-hosted macOS runner — which is also Alexis' dev Mac, and since
CI-COST the *only* place `macos` CI jobs run — a retention policy that keeps
`target/` and any other unbounded build output under a disk budget, on its own,
without a human intervening. Today nothing prunes anything: the workflow was
deliberately written to keep `target/` and `node_modules/` forever (`clean:
false`, see What was measured), and the disk is already effectively full. This
task specs the policy, the trigger, and the refuse-to-start guard; it does not
implement them (see Explicit non-goals).

## What was measured

Everything below was run read-only, from this worktree
(`.claude/worktrees/agent-a86da5e935972155a`) or by reading files, on
**2026-09-06**. No `cargo`, `pnpm check`, or other build command was executed —
the whole point of this ticket is that the disk cannot currently absorb one.

**1. Free space**, `df -h /Users/alexistan`:

```
Filesystem      Size    Used   Avail Capacity iused ifree %iused  Mounted on
/dev/disk3s5   460Gi   433Gi   4.0Gi   100%    4.1M   42M    9%   /System/Volumes/Data
```

4.0 GiB free of 460 GiB, 100% capacity. This confirms the ticket's premise
exactly.

**2. `target/` at the repo root** (the main checkout, not this worktree —
`find .claude/worktrees -maxdepth 2 -name target` returned nothing, so neither
active worktree has built Rust yet):

```
target/                    84G   (du -sh)
target/debug/deps          60G
target/debug/incremental   35G
target/debug/build        1.9G
target/release            1.3G
target/release/bundle      13M   (target/release/bundle/macos/Gritty.app only)
```

`deps` measured 60G here, not the 45G cited when this ticket was filed — the
directory grew between then and now, which is itself evidence of "unbounded."
Also note: `deps` (60G) + `incremental` (35G) + `build` (1.9G) alone exceed the
84G reported for the whole of `target/`. That is not a measurement error I can
resolve from here — APFS clonefile/hard-link sharing makes per-directory `du`
sums an unreliable way to predict how many bytes deleting one of them actually
frees. **Whoever verifies this GC must confirm reclaimed space with a `df`
delta across the whole run, not by summing the `du` of what it deleted.**

**3. No `CARGO_TARGET_DIR` anywhere.** No `.cargo/config.toml` exists (checked
at the repo root — `ls .cargo` → no such file). `grep -rn
"CARGO_TARGET_DIR\|target-dir"` across `*.toml`, `*.yml`, `*.md` in the repo
returned nothing. Every `cargo` invocation — local or CI — writes to the
default `<workspace root>/target`.

**4. `.github/workflows/ci.yml`** — read in full (132 lines). The `macos` job
(self-hosted, `runs-on: [self-hosted, macOS, ARM64]`):

- Only runs on `workflow_dispatch` or a PR labelled `ci:macos` — not on every
  push to `main` (the comment: "macOS is verified locally by `pnpm check` on
  every change anyway"). Lower frequency than the ticket's framing might
  suggest, but the growth mechanism is still unbounded.
- `actions/checkout@v7` with **`clean: false`**, and the comment says exactly
  why: *"keeps `target/` and `node_modules/` between runs. The default (`git
  clean -ffdx`) deletes both, which on a persistent runner means a cold Rust
  build every time. This is also why no cache action follows: the directory a
  cache would restore never goes away."* This is the root cause, stated
  candidly in the file: retention was chosen to be infinite, on purpose, with
  no follow-up plan.
- Sets `CARGO_INCREMENTAL: "0"` workflow-wide (top-level `env:`, line 26).
  **CI itself should not be producing `target/debug/incremental` growth at
  all** — that flag disables it. The 35G measured above is therefore most
  likely produced by local `cargo test`/`cargo check`/`pnpm check` runs on this
  same machine (by Alexis, or by an agent working directly in the main
  checkout rather than a worktree), not by the CI job. **This ticket's target
  is broader than "the runner": it is this machine's one shared `target/`,
  written to by CI and by ordinary local development with no separation
  between them.**
- Ends with `pnpm tauri build --bundles app --ci` and
  `actions/upload-artifact@v7` uploading
  `target/release/bundle/macos/Gritty.app` with `retention-days: 7`. That
  retention is **GitHub-side** (the hosted copy of the artifact); it does
  nothing to the local file, which stays in `target/release/bundle/` after the
  job ends.
- The Linux job is GitHub-hosted (`ubuntu-24.04`) and uses
  `Swatinem/rust-cache@v2`, a *hosted* cache — irrelevant to this machine's
  disk and out of scope here.

**5. `docs/plans/execution-board.md`, "Why lanes should run in separate
worktrees"**: `cargo test -p gritty-core` regenerates
`packages/protocol/src/generated/` as a side effect, and two sessions running
`pnpm check` in one checkout corrupt each other's output — "Leases do not
help: this is not an edit, it is a build." The plan's answer is one
`git worktree` per active lane, and `git worktree list` here shows two already
(`agent-a86da5e935972155a`, `agent-ae880c9f9510954e8`, both locked, neither with
a `target/` yet). Cargo's default (`<workspace root>/target`, unset
`CARGO_TARGET_DIR`) means each of those worktrees will grow its **own**,
separate ~84 GB-class `target/` the first time something in it runs
`cargo build` or `cargo test` — on a machine that has 4.0 GiB free *before*
that happens. See the CARGO_TARGET_DIR section below.

**6. Other candidate directories**, measured for completeness:

```
node_modules/ (repo root)               281M   — pnpm-managed, per-project
~/.local/share/pnpm/store               1.6G   — pnpm's global content store
fixtures/                                53M   — large-repo/, weird-files/generated/ are gitignored (regenerated), rest is tracked
target/release/bundle                    13M   — one Gritty.app, from Sep 4
```

No `release/` directory exists in this repository (that convention belongs to
the sibling `placeholder_pkill_STV` mobile project referenced in this
account's global instructions, not to GrittyIDE — confirmed, `ls release`
fails here). The only local build deliverable in this repo is
`target/release/bundle/macos/Gritty.app`.

## Motivation

Since PR #31 (CI-COST) the self-hosted macOS runner **is** the dev Mac, and
GitHub-hosted Linux CI is dark until the Actions allowance resets on 1 Oct
(CI-LINUX-DARK, `docs/tasks/README.md:68`) or a spending limit is set. There is
currently no fallback: when this disk fills, `pnpm check` stops passing
locally *and* the one CI lane that still runs on demand (`macos`) stops
working, at the same time, for the same reason. A disk-space failure here is
not "CI is slower" — it is "there is no CI and no working local build."

`docs/plans/execution-board.md:295-306` (see also its worktree section quoted
above) wants parallel agent lanes, each in its own `git worktree` under
`.claude/worktrees/`. That plan is currently unusable for anything that
touches Rust: the machine cannot afford a second `target/`, let alone one per
lane, and `pnpm check` (which runs `cargo test --workspace`, `package.json:19`)
is one of the two commands `AGENTS.md` calls "the definition of checks pass."

## Dependencies

None that gate this from a contract standpoint — this is an ops/tooling ticket,
not a step in the T000→T410 chain. Context, not a blocker:

- **CI-COST** (PR #31) made this machine the only place `macos` CI runs, and
  chose `clean: false` deliberately without a cleanup follow-up.
- **CI-LINUX-DARK** removed the fallback: while it is blocked, this machine's
  disk is a single point of failure for all CI.

## Required reading

- `.github/workflows/ci.yml` — full file (132 lines); specifically the
  `macos` job (lines 78–131), the `clean: false` comment (lines 92–98), the
  workflow-wide `CARGO_INCREMENTAL: "0"` (line 26), and the artifact upload
  (lines 124–131).
- `docs/plans/execution-board.md`, "Why lanes should run in separate
  worktrees" section, and its "Merge discipline" section immediately after —
  both describe the concurrency assumptions this task's `CARGO_TARGET_DIR`
  recommendation depends on.
- `docs/decisions/ADR-020-agent-permission-verdicts.md`, "What was measured" —
  the evidence convention this task file follows: claims carry dated,
  reproducible commands and raw output.
- `AGENTS.md` — repository map (`fixtures/` row: "never rely on a developer's
  real project"), hard rules 9 (no speculative infrastructure), 12 (don't
  claim untested behavior), 14 (don't broaden scope); the priority order
  `correctness → recoverability → inspectability → responsiveness`.
- `package.json` — `check`, `check:rust`, `check:generated` scripts: what a CI
  or local invocation actually runs, and therefore what the GC must not run
  out from under.
- `docs/tasks/README.md:67-69` — the three CI tickets (CI-COST, CI-LINUX-DARK,
  CI-RUNNER-GC) as currently recorded on the board.
- `.gitignore` — confirms `/target/`, `node_modules/`, and the two generated
  fixture subdirectories are the only build-output paths already understood to
  be disposable-and-regenerable from the repository's own point of view.

## Allowed scope

For the implementing session (not for this spec):

- A new script under `scripts/` (e.g. `scripts/runner-gc.py`, matching the
  existing `agent-board.py` / `agent-lease.py` style: stdlib only, dry-run
  first).
- `.github/workflows/ci.yml` — wiring a disk-guard step and a post-job GC step
  into the `macos` job only. Do not touch the `linux` job (hosted, hosted
  cache, not this problem).
- `docs/decisions/` — a new ADR if the implementer judges the retention policy
  and the CARGO_TARGET_DIR decision durable enough to bind future tasks the
  way ADR-020 binds hook-wait behavior. Not mandatory for this ticket, but the
  natural place if written.
- Its own task file's Completion report.

Not in scope for the implementer: any change to `crates/` or `apps/` domain
code, and no change to what `pnpm check` verifies — this is exclusively about
what happens to build *output*, before and after the commands that produce it.

## Explicit non-goals

- **This spec makes no code or workflow changes.** It is the task file only.
  Anyone implementing it starts a new task from a clean allowed-scope, per the
  Allowed scope above.
- Not a fix for CI-COST (why the runner is self-hosted) or CI-LINUX-DARK (why
  there's no fallback) — those are separate tickets with separate causes.
- Not a general disk-monitoring or alerting system for the machine as a whole
  (hard rule 9: no speculative infrastructure). Scope is `target/` and other
  build output this repository's own tooling produces.
- Not touching the pnpm global store (`~/.local/share/pnpm/store`) or any
  other machine-wide, cross-project cache. At 1.6 GB measured today it is not
  the problem, and it is shared with projects this repository has no
  authority over (this account's global instructions reference at least one
  sibling project). `pnpm store prune` already exists for that store if it
  ever needs attention — a different ticket.
- Not choosing or writing a specific eviction algorithm today. Community tools
  exist for exactly this shape of problem (`cargo-sweep`, `cargo-cache`) — the
  implementer should evaluate them against a bespoke `rm -rf` script rather
  than this spec deciding for them (see Design options).
- Not fixing the fact that `fixtures/large-repo/` and
  `fixtures/weird-files/generated/` are gitignored-and-presumably-regenerated —
  they are 53 MB total, not a contributor to this problem, and out of scope.

## Required behavior

### 1. Classify build output before deleting any of it

| Path (repo-root `target/`, and the same shape per worktree) | Nature | Cost of deleting |
|---|---|---|
| `target/debug/incremental`, `target/release/incremental` | Pure incremental-compile cache. CI never uses it (`CARGO_INCREMENTAL=0`); only local dev benefits. | None to correctness. Slower *next* local build only. **Safest, highest-value prune target** — 35G measured, zero CI cost. |
| `target/{debug,release}/deps`, `.../build` (build-script output + fingerprints) | Compiled dependency artifacts and build-script state. Real cache: losing it forces a full dependency rebuild. | Correct either way, but expensive in time (this workspace's dependency graph includes Tauri). Prune by age/size threshold, never wholesale on every run. |
| `target/release/bundle/**` (currently `target/release/bundle/macos/Gritty.app`, 13M) | The `macos` job's own deliverable, already pushed to GitHub via `actions/upload-artifact` by the time the job ends. | Safe to prune **after** the upload step succeeds; recoverable from the 7-day-retained GitHub artifact or a rebuild. Keep the most recent one; do not prune the one from the run currently in flight. |
| `fixtures/` | Deterministic test repositories (`AGENTS.md` repo map: "never rely on a developer's real project"). 53M. | **Never.** Not a disk-pressure contributor; not in scope. |
| `Cargo.lock`, `pnpm-lock.yaml`, source, `.git/` | Not build output. | **Never.** A GC bug that walks outside an explicit, named `target`-shaped root and reaches these is the failure mode this spec is most worried about — see Failure behavior and Acceptance criteria. |
| `node_modules/` (281M), pnpm global store (1.6G) | Regenerable (`pnpm install --frozen-lockfile`), but small relative to the 84G problem, and the store is shared machine-wide. | Out of scope (see Explicit non-goals); not worth the risk this ticket is trying to reduce. |

### 2. A retention policy with three complementary triggers, not one

The three questions in the ticket ("post-job? scheduled? threshold-based?")
are not alternatives — each catches a failure mode the others miss:

- **Post-job, in the `macos` CI job itself, after `actions/upload-artifact`
  completes** (success or failure of the build step — a failed build still
  leaves `target/debug/incremental` and partial `deps`/`build` state behind,
  and a failed job is exactly when nobody is watching the disk). Unconditionally
  clear `incremental` (CI never needed it, per the `CARGO_INCREMENTAL=0`
  finding above); prune `deps`/`build` entries by an age or last-used threshold
  (see Design options for cargo-sweep-style "keep last N" as the likely
  mechanism); prune superseded entries under `target/release/bundle` once the
  current run's artifact has uploaded.
- **Scheduled, independent of any CI job running.** The 35G of `incremental`
  measured today is plausibly *local dev*, not CI (see What was measured,
  point 4) — a policy that only fires from inside the `macos` job never
  touches growth that CI-adjacent local work causes. A daily or twice-daily
  scheduled sweep (cron, or a `workflow_dispatch`-triggered scheduled run) is
  the backstop for growth the job-triggered path cannot see.
- **Threshold-based, and it is the one that must run *before* any build
  starts, not only after.** Check free space at the top of the `macos` job
  (and ideally as something a person can invoke before an expensive local
  `cargo build`). If free space is below a floor, **refuse to start the job**
  with a clear, loud message — not let `cargo`/`rustc`/the linker die
  mid-build with an opaque "No space left on device" forty minutes in. Given
  today's 4.0 GiB free, this repository is *already* below any sane floor;
  the first thing this GC must do, once built, is reclaim enough to clear that
  floor and keep it clear afterward.

### 3. Should `CARGO_TARGET_DIR` be shared across worktrees and CI?

**Recommendation: no. Keep the default — one `target/` per checkout (the main
tree, and one per active `.claude/worktrees/*` lane) — and make the GC sweep
all of them, rather than consolidating into one shared directory.**

Reasons:

- `docs/plans/execution-board.md`'s own argument for per-worktree lanes is
  that two sessions sharing one build both write
  `packages/protocol/src/generated/` at once and corrupt each other — "this is
  not an edit, it is a build," which leases cannot fix. A shared
  `CARGO_TARGET_DIR` reintroduces exactly that hazard, and adds a second one on
  top: Cargo takes its own lock on the target directory for the duration of a
  build/test invocation. Two lanes both running `cargo test -p gritty-core`
  against one shared target directory would not merely risk corrupting
  generated output — one lane's `cargo` would **block, silently, for the
  entire duration of the other lane's build**, which for this workspace
  (Tauri + its dependency graph) is minutes, not seconds. That is a
  responsiveness and inspectability failure (an agent's build looks hung, with
  no visible cause) directly against the priority order in `AGENTS.md`
  (`correctness → recoverability → inspectability → responsiveness`), on top
  of being the disk-cost trade-off the ticket already names.
- Disk cost of *not* sharing is real and immediate: at today's measured 84G
  steady-state size (times, in the worst case, one per active lane) against
  4.0 GiB free, this machine cannot currently afford even one additional full
  `target/`. That is the argument *for* fixing retention with real urgency —
  not for merging directories to save space. A GC that keeps each `target/`
  small makes "one per lane" affordable again; a shared directory makes lanes
  serialize on each other, which defeats the reason they are separate
  worktrees at all.
- The CI `macos` job runs in the main checkout (`actions/checkout@v7`, no
  worktree), so it already has its own `target/`, distinct from any lane's.
  The GC's job is to sweep **every** `target/`-shaped directory this machine
  has (main checkout plus each currently-locked `.claude/worktrees/*` entry —
  `git worktree list` is how to enumerate them), not to assume there is one.

### 4. The disk guard

- Runs first, before any `cargo`/`pnpm` command that could consume large
  amounts of disk.
- Compares free space (`df`) against a floor chosen with headroom for a
  from-scratch debug build of this workspace plus a release bundle build —
  the implementer should measure a clean build's peak `target/` growth rather
  than guess; this spec does not have that number (it was not safe to measure
  today, on a 4.0 GiB-free machine, without risking the exact failure this
  ticket exists to prevent).
- On failure: **the job fails immediately, loudly, with the free-space number
  and the floor in the message** — not a mid-build compiler or linker error
  that looks like a code problem.

## Failure behavior

- **GC runs while a build holds the target-directory lock** (a local `cargo`
  invocation, or a concurrent CI job). The GC must detect this — Cargo's own
  lock file in the target directory, or a live `cargo`/`rustc`/`tauri`-build
  process rooted at that target directory — and **skip that directory this
  pass, never force through it.** Deleting `incremental` or `.fingerprint`
  entries out from under a live build does not fail cleanly: Cargo may see
  stale or missing fingerprints mid-compile and either silently recompile a
  unit it thought was done, or (worse, for `deps`) have the linker fail
  against a file that vanished mid-link. "Detect and skip, try again next
  trigger" is the only acceptable behavior; there is no safe forced case.
- **GC deletes something a job still needs.** Within one `macos` job this is
  ordering, not locking: the GC step must run strictly after
  `actions/upload-artifact`, never before or in parallel with it. Across
  concurrent jobs: `ci.yml`'s top-level `concurrency: group: ci-${{
  github.ref }}, cancel-in-progress: true` already serializes runs *on the
  same ref*, but the `macos` job triggers on `workflow_dispatch` or a labelled
  PR from **any** ref — two different refs could in principle both dispatch a
  `macos` job at once. Whether that can actually happen concurrently on one
  physical runner depends on how many runner instances are registered on this
  machine, which this spec did not check (no destructive or build command was
  run to find out, and runner registration is not visible from the
  repository). **Flag this as an open question for the implementer to verify
  before assuming single-job-at-a-time is safe** — if more than one runner
  process can be scheduled here, the same lock-awareness the GC needs against
  local builds also applies between concurrent CI jobs.
- **The disk guard itself must fail safe.** If it cannot determine free space
  (e.g. `df` unavailable or unparseable), it should refuse to start the job
  rather than silently proceeding — the whole point is "fail loudly before the
  build," and a broken guard that fails open reintroduces the mid-build death
  it exists to prevent.

## Acceptance criteria

- **Dry-run is the default**, and it is indistinguishable from a real run
  except that nothing is deleted: the GC prints exactly what it would remove
  and the space it expects to reclaim, against the classification table above.
  A real deletion requires an explicit flag (e.g. `--apply`), matching the
  existing `scripts/agent-board.py` / `scripts/agent-lease.py` convention of
  being safe-by-default command-line tools.
- The GC **never** deletes anything outside an explicit, named allowlist of
  `target`-shaped roots it was given — no directory traversal from a computed
  "parent of X" or similar; each root is named or discovered by an explicit,
  narrow rule (e.g. "a directory containing `CACHEDIR.TAG` and a `.cargo-lock`
  or `.rustc_info.json`" — Cargo's own target-directory markers), and anything
  that doesn't match is left alone. `fixtures/`, `Cargo.lock`, `pnpm-lock.yaml`
  and everything under `.git/` are never touched, and a test proves it (see
  Tests required).
- The disk guard blocks a job start when free space is below the chosen floor,
  and the failure message states the measured free space and the floor.
- The GC detects an in-progress build (lock file or live process rooted at the
  target directory it is about to touch) and skips that directory rather than
  deleting from under it.
- Reclaimed space is verified with a `df` delta across the whole GC run — not
  by summing `du` of the paths it deleted (see What was measured, point 2, for
  why the two disagree on this machine).
- `pnpm check` still passes after a GC pass runs against a warm build (i.e.
  the GC's classification is actually correct — nothing it prunes is something
  `pnpm check` unconditionally needs).

## Tests required

- **Never against the real machine's real `target/`.** Build a scratch
  directory tree that mimics the shape in the classification table (dummy
  files under fake `deps/`, `incremental/`, `build/`, `bundle/`, a fake
  `fixtures/`, a fake `Cargo.lock`) under this session's scratchpad or a temp
  dir, and run the GC — dry-run and `--apply` both — against that copy only.
  Assert: the allowlisted "never" paths survive `--apply` byte for byte; the
  "safe to prune" paths are gone; the size-classification table's expensive
  paths are pruned only past whatever threshold the implementation chooses,
  and provably not below it.
- A test that plants a lock file (or forks a long-lived dummy process) rooted
  at the scratch target directory and asserts the GC skips that directory
  entirely on that pass.
- A test for the disk-guard's own failure path: simulate free space under the
  floor (a stub for whatever measures it, not the real disk) and assert the
  job/script refuses to start, with the expected message.
- Manual, only once the implementer has reclaimed real space safely: confirm
  `df -h /Users/alexistan` shows meaningfully more free space after a real
  `--apply` run against the actual machine, and that a subsequent `pnpm check`
  still passes.

## Commands to run

For the implementing session (none of these were run for this spec — see What
was measured):

```
pnpm check                    # must still pass after any GC pass against a warm build
python3 scripts/<gc-script>.py --dry-run       # default; must be silent-safe
python3 scripts/<gc-script>.py --apply         # only against a scratch tree in tests;
                                                # against the real machine only once trusted
df -h /Users/alexistan        # before/after, to verify reclaimed space
```

## Design options (decisions for the implementing session)

- **Eviction mechanism: bespoke deletion logic vs. an existing tool.**
  `cargo-sweep` and `cargo-cache` both exist specifically to prune stale
  `target/` entries by last-use or age while keeping what recent builds still
  need — evaluate them before writing bespoke `rm -rf` logic; a project this
  size re-deriving Cargo's own fingerprint/staleness rules is exactly the kind
  of duplicated policy `AGENTS.md` hard rule 2 warns about ("do not create a
  new architectural abstraction when an existing contract owns that
  responsibility"). If neither fits (e.g. because of the bundle-pruning or
  disk-guard requirements above, which are this repo's own concern, not
  Cargo's), say so explicitly in the completion report rather than silently
  reaching for a custom script.
- **Exact free-space floor and "keep last N" values.** This spec does not set
  numbers — it was not safe to measure a from-scratch build's peak disk use on
  a 4.0 GiB-free machine today. The implementer should measure this once real
  headroom exists (after the first manual/emergency cleanup this ticket's
  urgency implies happens anyway) and record the number with its own dated
  evidence, per the ADR-020 convention this file follows.
- **Whether the CARGO_TARGET_DIR / per-lane-target decision above needs its
  own ADR.** Recommended if the implementer wants it to bind future worktree
  or CI tasks the way ADR-020 binds hook-wait behavior; not mandatory for this
  ticket to land.

## Completion report

**Status note:** this ticket was already at `review` when this report was
written — that status belonged to the *spec* (PR #36, session `A-runner-gc`,
which wrote this task file and measured 4.0 GiB free / `target/` at 84G, then
stopped: "no implementation has started"). This report is the implementation,
done by a separate session in its own worktree
(`worktree-agent-ae118de9a06ec662a`), on 2026-09-06. Per instruction from the
coordinating session, the ticket is left at `review` rather than moved.

### Fresh measurements superseding the spec's

The spec's numbers (4.0 GiB free, `target/` at 84G) were already stale by the
time this session started, for a mundane reason: **the coordinating session
ran `cargo clean` immediately before launching this implementation**,
removing **261,844 files / 95.9 GiB** and leaving **87 GiB free**. That single
fact is the strongest evidence for this ticket that exists: **95.9 GiB
accumulated in about a week, and nothing but a human noticing ever freed it.**
Before that clean, `target/debug/deps` alone was 62G across 229,476 files,
*none older than 7 days* — this was not old cruft sitting untouched, it was
one week of ordinary branch churn, which is exactly the "unbounded" the
ticket is about (age-only eviction would not have touched any of it).

By the time this session took the ticket (after a required rebase onto
`e8ada81`), the picture had already moved again: `df -h /Users/alexistan`
showed 460Gi/352Gi used/**84Gi avail**, and `git worktree list` showed **six**
live worktrees (main checkout + five agent lanes), of which only one
(`pty-flake`) had built anything (98 MiB); the other four had no `target/`
yet. This is the real shape of the problem the spec anticipated in its
"Should `CARGO_TARGET_DIR` be shared" section: not one `target/` on one
checkout, but N of them appearing and disappearing under
`.claude/worktrees/`, some with builds actively running while this GC runs.

**A from-scratch `pnpm check` in this worktree** (fmt, clippy
`--all-targets -D warnings`, `cargo test --workspace` incl. doctests) peaked
at `target/debug/`: **4.5 GiB** total — `deps` 3.1G, `incremental` 922M,
`build` 360M, `.fingerprint` 13M. **`pnpm tauri build --bundles app --ci`**
(the `macos` job's own release step), run immediately after in the same
worktree, peaked at `target/release/`: **1.24 GiB** — `deps` 1.1G, `build`
136M, `.fingerprint` 7.8M, `incremental` 0 (release doesn't use it) — plus a
16 MiB `Gritty.app` bundle. Combined, one full clean build of this workspace
(debug + release) measured **~5.7 GiB in `target/`**, real and dated, not
estimated — this is what the disk-guard floor and cache budgets below are
sized against.

### Summary

Implemented the three-trigger retention policy the spec called for:
`scripts/runner-gc.py`, a stdlib-only Python CLI (`guard`, `sweep`,
`selftest`) matching the `agent-board.py`/`agent-lease.py` convention, wired
into `.github/workflows/ci.yml`'s `macos` job (disk guard before any
pnpm/cargo command; post-job sweep after `actions/upload-artifact`, gated
`if: always()`) and into a new scheduled workflow,
`.github/workflows/runner-gc-scheduled.yml`, for the backstop trigger that
doesn't depend on a CI job running at all. `docs/decisions/ADR-024` records
the CARGO_TARGET_DIR decision (keep the default, one `target/` per checkout,
sweep each independently) as durable.

The tool was **actually executed against the real machine**, not just
written: a dry run, a real `--apply` run that reclaimed disk across five live
target directories while correctly skipping a sixth it caught mid-build, and
a full `pnpm check:rust` + `check:generated` pass afterward to confirm the
classification didn't break anything. See "What was actually run" below.

### Files changed

- `scripts/runner-gc.py` (new) — the GC/guard tool. ~600 lines incl. an
  in-file `selftest` subcommand (8 `unittest` cases against scratch
  directories only, per the spec's "never against the real machine's real
  `target/`").
- `.github/workflows/ci.yml` — two new steps in the `macos` job only (the
  `linux` job is untouched, as scoped): a disk guard right after checkout,
  before `pnpm/action-setup`; a post-job sweep after
  `actions/upload-artifact`, `if: always()`.
- `.github/workflows/runner-gc-scheduled.yml` (new) — the independent
  scheduled trigger (`cron: "0 6,18 * * *"` plus `workflow_dispatch` with a
  `dry_run` input), on the same self-hosted runner, its own `concurrency`
  group so it never overlaps itself or the `macos` job's own sweep.
- `docs/decisions/ADR-024-target-dir-retention.md` (new) + `docs/decisions/README.md`
  (index row) — the CARGO_TARGET_DIR decision, with the dated measurements
  above as its evidence.
- This file's Completion report.

No `crates/`, `apps/`, or `packages/` file was touched. No `PROTOCOL_VERSION`,
`SCHEMA_VERSION`, or migration was claimed or spent.

### Design decisions

- **Bespoke script, not `cargo-sweep`/`cargo-cache`.** Both were checked
  (`which cargo-sweep cargo-cache` → not found; neither installed on this
  machine). Evaluated against the spec's own requirements: neither tool
  discovers a fleet of worktree-shaped target directories, prunes
  `target/release/bundle` specifically after an upload step, gates a job
  start on free space, detects a live build via lock+process heuristics for
  *this repo's* concurrency model, or reports a verified `df` delta instead
  of a `du` sum. Those five things are this ticket's actual requirements and
  are orchestration this repo owns, not something Cargo's own tooling is
  positioned to know (worktree layout, CI step ordering, this machine's dual
  role as dev box and runner). Installing either would also mean a
  `cargo install` build on the exact machine this ticket is trying to
  protect. One bespoke, stdlib-only script covers all of it, matching hard
  rule 2's spirit (not re-deriving Cargo's *fingerprinting*, which this
  script never attempts — it evicts whole deps/build/.fingerprint entries by
  mtime, not by re-implementing staleness detection).
- **Allowlist enforcement is structural, not a filter.** `plan_incremental`,
  `plan_cache`, and `plan_bundle` can only ever construct paths by joining
  `ALLOWED_PROFILES`/`ALLOWED_SUBDIRS` names onto an already-`is_target_root`
  -verified root; `assert_safe` re-derives and re-checks that invariant
  immediately before every single deletion, in both dry-run and apply (dry
  run also calls it, and reports the same errors, so a bad plan is visible
  before `--apply` is ever passed). `is_target_root` requires **both**
  `CACHEDIR.TAG` and (`.rustc_info.json` or `.cargo-lock`) — Cargo's own
  markers — never a directory named `target` by convention alone.
- **Lock detection: `.cargo-lock` flock first, process-table scan as a named
  backstop.** The flock check is authoritative (it's the same file and same
  locking mechanism Cargo itself uses); the `ps` scan is explicitly
  documented as a heuristic, matching `agent-lease.py`'s own honest framing
  of its shell-write guard. Every failure mode here — can't open the lock
  file, can't flock it, can't enumerate processes — fails **safe** (treated
  as locked, skip), never open.
- **Budget + age eviction, not "delete everything" and not pure age.** The
  spec's own point-4 measurement (62G, zero files older than 7 days) proves
  pure age-based eviction would have pruned nothing from the actual growth
  observed — so `plan_cache` evicts oldest-mtime-first *only past a size
  budget*, and separately never touches anything younger than
  `--min-age-minutes` (default 15) even when over budget, as defense in
  depth against a build that starts writing in the gap between planning and
  deleting.
- **`CARGO_TARGET_DIR` stays per-checkout (ADR-024).** Not shared across
  worktrees or with CI. A shared target directory would reintroduce, at the
  build-lock level, the exact hazard `execution-board.md` created worktrees
  to avoid at the file level (`packages/protocol/src/generated` corruption) —
  see the ADR for the full argument. The GC's job is enumeration
  (`git worktree list`, validated per-root) not consolidation.
- **`.fingerprint` swept alongside `deps`/`build`.** Not named separately in
  the spec's classification table, but it is the same nature and cost
  (per-unit build cache, real but reproducible) as the two directories it
  sits beside, and leaving it out while sweeping its siblings would grow it
  unboundedly by omission. Recorded here as an explicit extension of the
  spec's table, not a silent one.

### What was actually run (with results)

All commands executed from `worktree-agent-ae118de9a06ec662a`, 2026-09-06,
`export PATH="/opt/homebrew/opt/rustup/bin:$PATH"` where cargo was needed.

1. **`python3 scripts/runner-gc.py selftest`** — 8 tests, all pass:
   allowlist-never-escapes (fixtures/, Cargo.lock, pnpm-lock.yaml, `.git/`
   survive `--apply` byte for byte), dry-run deletes nothing while still
   reporting a plan, incremental + over-budget cache pruned while an entry
   newer than `min_age_seconds` survives even over budget, bundle keeps
   newest N, a held `.cargo-lock` flock causes a full skip, `is_target_root`
   rejects a bare directory and one with only `CACHEDIR.TAG`, and a
   regression test for the nested-worktree false positive below.
   ```
   Ran 8 tests in 0.679s
   OK
   ```
2. **Dry run against the real machine** (`python3 scripts/runner-gc.py
   sweep`) — first run surfaced a real bug: the main checkout's `target/`
   was reported "SKIPPED — live process rooted at
   .../GrittyIDE: ... apps/desktop/node_modules/.../tauri.js" even though
   nothing was building in the main checkout itself. Root cause:
   `.claude/worktrees/<lane>` lives *inside* the main checkout's own
   directory tree, so a worktree's absolute path always contains the main
   checkout's path as a string prefix — the process-scan backstop's naive
   substring match therefore attributed every worktree's build to the parent
   checkout too. Fixed (`is_build_in_progress` now takes `other_checkouts`
   and only counts a match not better explained by a more specific nested
   checkout path) and covered by
   `test_nested_worktree_build_is_not_mistaken_for_parent_checkout`. Rerun
   after the fix correctly attributed each build to its own worktree.
3. **Real `--apply` run against the live machine**, six worktrees, five with
   a `target/`:
   ```
   /Users/alexistan/Desktop/CurrentProjects/GrittyIDE/target
     removed 0.563 GiB  .../target/debug/incremental
   /Users/.../worktrees/agent-a59fa0e69530bd598/target
     SKIPPED — live process rooted at .../agent-a59fa0e69530bd598:
       62121 rustc --crate-name brotli ...
   /Users/.../worktrees/agent-a5ded56b2cd14746c/target
     removed 1.254 GiB  .../target/debug/incremental
   /Users/.../worktrees/agent-abc87d7c043ef69f9/target
     removed 1.291 GiB  .../target/debug/incremental
   /Users/.../worktrees/agent-ad7645a48bb5845a6/target
     removed 0.051 GiB  .../target/debug/incremental
   /Users/.../worktrees/agent-ae118de9a06ec662a/target (this session's own)
     removed 1.556 GiB  .../target/debug/incremental
     removed 0.000 GiB  .../target/release/incremental
   planned total: 4.715 GiB across 6 target root(s)
   df delta at /Users/alexistan: 73.34 GiB -> 76.11 GiB free (+2.76 GiB)
   ```
   Two things worth flagging as evidence the design works, not just the
   numbers: **(a)** `agent-a59fa0e69530bd598` was genuinely mid-build (a live
   peer agent session, one of six running in parallel) — the mid-sweep
   re-check caught it and the directory was left untouched, live, in
   production conditions, not a simulated test. **(b)** the reclaimed space
   (+2.76 GiB) is meaningfully smaller than the summed `du` of what was
   deleted (4.715 GiB) — this is exactly the APFS clonefile/hard-link-sharing
   effect the spec's "What was measured" point 2 warned about, now confirmed
   with a real before/after `df`, not asserted from a formula.
4. **`pnpm check:rust`** (fmt, clippy `--all-targets -D warnings`,
   `cargo test --workspace` incl. doctests), run *after* the `--apply` pass
   above against this session's own now-pruned, still-warm `target/`:
   passed with zero errors and zero `ELIFECYCLE` failures. **`pnpm
   check:generated`**: exit 0. This is the acceptance criterion that
   actually matters for a `target/`-retention change ("nothing it prunes is
   something `pnpm check` unconditionally needs") and it holds.
5. **`pnpm check` (the full command, incl. `pnpm test`/vitest across the JS
   workspace)** was attempted twice and failed both times, on the same two
   tests, neither touched by this ticket: `packages/markdown/src/render.test.ts`
   ("renders a long document within budget", asserts `<2000ms`, measured
   2227–3090ms across runs) and `packages/editor/src/monaco/model-eol.dom.test.ts`
   (`beforeAll` hook timed out at 10000ms loading Monaco under jsdom).
   Confirmed environmental rather than a regression from this change: **(a)**
   neither test touches `target/`, Rust, or anything this GC prunes; **(b)**
   re-running `packages/markdown`'s suite alone, in isolation, passed cleanly
   (12/12); **(c)** at the time of both failing runs this machine had **12
   concurrent `cargo build`/`test`/`clippy`/`rustc` processes** from the five
   other agent sessions building in parallel — both failures are
   textbook symptoms of CPU-starved timing/hook-timeout assertions under
   that load, not something `scripts/runner-gc.py` or the workflow changes
   caused. Reported honestly per hard rule 12 rather than claimed as passing.
6. **Disk guard**, both paths exercised directly:
   `python3 scripts/runner-gc.py guard --floor-gb 25` → `OK` (exit 0, 75.6
   GiB free); `--floor-gb 999999` → refuses with the measured-vs-floor
   numbers in the message (exit 1).

### Known limitations

- **Cache-budget and floor defaults are reasoned from one real measurement
  each, not a distribution.** `--cache-budget-gb 12` (per profile, per
  target root) is ~3.5x this session's one measured clean-debug-build figure
  (3.47 GiB in deps+build+.fingerprint); `--floor-gb 25` is ~4.4x the
  measured combined debug+release clean-build peak (~5.7 GiB). Both are
  documented, dated, and in-code (see the argparse `help`/comments in
  `scripts/runner-gc.py`), but a single sample is not a distribution —
  revisit once real sweep history exists.
- **The process-scan lock backstop is still a heuristic**, even after the
  nested-worktree fix: a build that doesn't put its checkout path in its
  command line (unusual, but possible) would not be caught by it. The
  `.cargo-lock` flock check is authoritative and does not have this gap;
  the process scan is explicitly a backstop, not the primary guarantee.
- **The spec's "open question" about concurrent runner processes on this
  physical machine was not resolved** — runner registration count is not
  visible from the repository, and no destructive test was run to probe it.
  If more than one runner process can be scheduled here, the CI-to-CI
  concurrency case in the spec's Failure behavior section needs the same
  lock-awareness this GC already has against local builds — it already has
  it (the flock/process check doesn't care whether the other holder is a
  human's `cargo build` or a second CI job), but this was not verified
  end-to-end against two real concurrent jobs.
- **A `schedule`-triggered workflow on a self-hosted runner only fires if the
  runner service is up at that wall-clock time.** This machine is also the
  dev Mac; a missed sweep (machine asleep/off at 06:00 or 18:00 UTC) is
  possible and invisible from inside the repository. No alerting was built
  (would be the "general disk-monitoring system" the spec explicitly places
  out of scope).

### Follow-up items (not touched — out of this ticket's scope)

1. **A fresh `git worktree`'s first `cargo test`/`clippy` on `gritty-desktop`
   fails** unless `apps/desktop/scripts/prepare-sidecar.mjs` has staged the
   `gritty-agent-hook` sidecar binary at least once. That staging normally
   happens via `pnpm tauri build`'s `beforeBuildCommand`, but `cargo test
   --workspace`/`pnpm check` invoke cargo directly and never trigger it. On
   the persistent self-hosted runner (`clean: false`) this is invisible
   because the binary already exists from a prior build; discovered here
   only because this worktree's `target/` was genuinely cold. Worked around
   locally (`node apps/desktop/scripts/prepare-sidecar.mjs --debug`) to get
   real measurements; not fixed, since it touches `apps/desktop/**`, outside
   this ticket's Allowed scope. Worth its own ticket.
2. **Two apparently pre-existing, load-sensitive test flakes** observed under
   six-way concurrent build contention: `packages/markdown/src/render.test.ts`
   (hard-coded `<2000ms` performance budget) and
   `packages/editor/src/monaco/model-eol.dom.test.ts` (10s `beforeAll`
   timeout loading Monaco under jsdom). Both are outside this ticket's scope
   (no TypeScript touched) but may be the same class of issue as the
   already-tracked `JSDOM-VITEST` ticket referenced in
   `docs/merge-queue.md` (PR #37) — worth a look there.
3. **Board mechanics friction, for the record:** `agent-board.py take
   CI-RUNNER-GC --force` was blocked by this session's own permission
   classifier (a destructive/override action). Worked around by claiming the
   file lease directly (`agent-lease.py claim --agent W-runnergc --scope
   'scripts/**' '.github/workflows/**' 'docs/decisions/**'
   'docs/tasks/CI-RUNNER-GC-*.md'`), since `agent-lease.py list` showed no
   leases actually held — the spec-writing session (`A-runner-gc`) had
   already released its lease on moving the ticket to `review`, it just
   hadn't cleared the board's `holder` field. Net effect: this ticket's board
   entry still shows `A-runner-gc` as holder, not this implementation
   session — a bookkeeping discrepancy the coordinating session should be
   aware of, not a concurrency risk (verified no live lease existed before
   claiming).
4. **Numbers should be revisited after this policy has run for real**, per
   the spec's own Design Options note — once `scripts/runner-gc.py sweep`
   has a history of real sweeps on this machine, the one-sample defaults
   above can be replaced with ones backed by a distribution.

### Remaining risks

- If this machine ever runs more `cargo`/`rustc` processes than the process
  table shows at the instant of a scan (a very short-lived spawn between the
  `ps` snapshot and the deletion), the mid-sweep re-check narrows but does
  not eliminate the race — no forced case exists that would, per the spec's
  own Failure behavior section, and none was added here.
- The scheduled workflow's silent-miss-on-sleep limitation (above) means the
  scheduled trigger is a *backstop*, not a guarantee; the CI post-job sweep
  and the pre-build disk guard remain the two triggers that fire
  synchronously with actual disk-consuming work.
