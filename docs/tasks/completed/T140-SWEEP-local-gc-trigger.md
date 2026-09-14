# Task T140-SWEEP — Fire the build-cache sweeper where the growth is

- **Phase:** 2
- **Status:** done
- **Depends on:** CI-RUNNER-GC (the sweeper this wires up)

## Objective

Give `scripts/runner-gc.py` a local trigger. It already does the hard part; it
has simply never run anywhere the disk actually fills.

## Motivation

`scripts/runner-gc.py` fires from **GitHub Actions only** — a `guard` before the
`macos` job, a `sweep --apply` after it, and a scheduled workflow at 06:00/18:00
UTC that needs the self-hosted runner service to be up. Local development is
where the growth happens and where nothing swept at all.

The evidence that the policy was not holding: `HANDOFF.md` records `target/`
going **49 G → 67 G in a day** with ADR-024 nominally in force, and the disk has
now cost time on three occasions — most sharply when a session hit
`ld: write() failed, errno=28` with 396 MiB free, which renders as
`error: could not compile gritty-core` and reads like your own breakage.

Measured 2026-09-09, and the reason a sweep alone is not the whole answer:
`target/debug/deps` is **81.6 GB logical across 127,522 files**, of which
**52.0 GB is `.rcgu.o` codegen-unit objects**. Shrinking those is `T140-PROFILE`
and `ADR-033`; this ticket bounds what accumulates regardless.

## Explicit non-goals

- **Rewriting `runner-gc.py`.** It discovers every worktree's target root via
  `git worktree list`, validates by Cargo's own `CACHEDIR.TAG` markers, evicts
  oldest-first past a budget, honours a 15-minute recency floor, skips a root a
  live build holds, and is dry-run by default. It works; it was unwired.
- Changing `--cache-budget-gb` or `--floor-gb`. Their defaults carry dated
  derivations from real measurements; `T140-PROFILE` re-derives them if the
  profile moves.
- A shared `CARGO_TARGET_DIR` — see ADR-024, and ADR-033's note on why the
  measurement makes it the smallest lever.

## Required behavior

1. `SessionStart` runs `guard --floor-gb 25`, so a session that cannot afford a
   build is told at the top rather than at the linker.
2. `SessionEnd` runs `sweep --apply`, after the existing lease and board
   session-end hooks. A session ends far more often than the runner is awake,
   and a sweep at session end cannot interrupt that session's own build.
3. Neither may fail or delay the session.

## Failure behavior

- **`guard` exits 1 below the floor and writes to stderr.** Both matter: the
  message is on the wrong stream to reach the session, and a non-zero exit from
  a startup hook is not something to risk. Hence `2>&1 || true` — the warning
  lands on stdout, where `SessionStart` turns it into context, and the exit is
  always 0.
- **A live build is skipped, not pruned underneath.** Verified rather than
  assumed: the sweep declined this checkout while a `tauri dev` held it, naming
  the pid and the process.
- Both hooks carry a `timeout` so neither can hang session start or exit.

## Acceptance criteria

- `.claude/settings.json` stays valid JSON with all five pre-existing hooks
  intact.
- Both commands exit 0 when piped the hook's stdin payload, and the guard's
  message still appears.
- The sweeper still refuses a target root a live build holds.

## Tests actually executed

2026-09-09, macOS, this checkout.

| Check | Result |
|---|---|
| `echo '{}' \| … guard --floor-gb 25 2>&1 \|\| true` | **exit 0**, warning printed: `18.1 GiB free … floor is 25.0 GiB` |
| `echo '{}' \| … sweep --apply 2>&1 \|\| true` | **exit 0**, reported the `df` delta and its APFS caveat |
| Live-build skip | **SKIPPED — live process rooted at …: 5825 node … tauri.js dev** |
| `python3 -c "json.load(...)"` | valid |
| `jq -e` on both event paths | both commands present |
| Hook count per event | SessionStart 2, PreToolUse 1, SessionEnd 3 — nothing lost |

## Known limitations

- **A long-running `pnpm dev` blocks the sweep indefinitely.** The process
  heuristic treats `tauri dev` like any build holding the root, which is correct
  — it rebuilds on change — but it means a session with a dev server up all day
  never sweeps. Observed live while writing this ticket: the sweep declined,
  correctly, and reclaimed nothing.
- **The `SessionEnd` sweep cannot reclaim this session's own peak.** It runs
  after the work, so the high-water mark during a session is still unbounded;
  the `SessionStart` guard is what catches that, one session late.
- **`df` is the only honest measure.** APFS clonefiles make both `du` and
  `stat` overstate: this tree reports 81.6 GB by `stat` and 66 GB by `du`, and
  CI-RUNNER-GC measured 4.715 GiB of `du` yielding a 2.76 GiB `df` delta. The
  sweeper already reports the `df` delta and says so.

## Follow-up tasks

- `T140-PROFILE` / `ADR-033` — shrink what each build produces. This ticket
  bounds accumulation; it does nothing about the 52 GB of debuginfo objects
  that accumulate.
- A sweep trigger that survives a long-lived dev server — a pre-build guard, or
  a sweep that prunes roots other than the one being held.

## Commands to run

```
python3 -c "import json;json.load(open('.claude/settings.json'))"
jq -e -r '.hooks.SessionEnd[].hooks[].command' .claude/settings.json
```
