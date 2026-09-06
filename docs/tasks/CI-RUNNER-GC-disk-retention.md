# Task CI-RUNNER-GC — The only CI machine has 4 GB left and nothing ever frees it

- **Phase:** Ops (cross-cutting; not on the phase/gate roadmap)
- **Status:** doing
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

_(unstarted — this file is the specification; no implementation has started)_
