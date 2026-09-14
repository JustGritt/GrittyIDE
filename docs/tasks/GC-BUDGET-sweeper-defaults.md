# Task GC-BUDGET — Re-derive the sweeper's cache budget and disk floor for ADR-033's smaller builds

- **Phase:** tools
- **Status:** review
- **Depends on:** ADR-024 (one `target/` per checkout, swept), ADR-033 (`debug = "line-tables-only"`)

## Objective

Give `scripts/runner-gc.py`'s two sizing defaults — `sweep --cache-budget-gb`
(12) and `guard --floor-gb` (25) — derivations measured against the build this
repository produces *today*, and change them only if the numbers say so.

## Motivation

Both defaults carry dated derivations from CI-RUNNER-GC's 2026-09-06
measurement: 3.47 GiB of `deps`+`build`+`.fingerprint` for one clean debug
`pnpm check`, and ~5.7 GiB combined debug + release. Since then ADR-033 made
every dev build ~23% smaller, and the workspace has grown (`gritty-search`,
`gritty-git` on `gix`, `gritty-git-write`). ADR-033's Consequences and
T140-PROFILE's follow-ups both filed this; ADR-024's acceptance note repeats
it. Nobody took it.

The direction is not obvious. A budget that is too generous wastes disk; one
that is too tight evicts a live build's inputs, and every sweep then forces a
cold rebuild. The live set on this machine is not one `pnpm check` either:
`pnpm dev` builds the desktop binary without `cfg(test)`, and clippy's check
artifacts sit beside test artifacts in the same `deps`.

## Required reading

- `docs/decisions/ADR-024-target-dir-retention.md`, `docs/decisions/ADR-033-dev-profile-debuginfo.md`
- `docs/tasks/completed/CI-RUNNER-GC-disk-retention.md` (the original method and numbers)
- `docs/tasks/completed/T140-PROFILE-dev-profile-debuginfo.md` (why `df`, not `du`; why a quiet machine)
- `scripts/runner-gc.py` — `plan_cache` sums `lstat().st_size`, so the budget is in *logical* bytes

## Allowed scope

- `scripts/runner-gc.py` — the two defaults and their derivation comments only
- this task file

## Explicit non-goals

- No change to eviction logic, lock detection, or triggers.
- No `incremental = false` and no `sccache` — both separately filed by T140-PROFILE.
- No change to CI workflow arguments beyond what a changed default implies.

## Required behavior

1. Measure in a **private `CARGO_TARGET_DIR`** so the checkout's live `target/`
   and any concurrent session are untouched, and so the numbers are one clean
   generation.
2. Measure in the unit each default is compared against: the budget in
   `plan_cache`'s logical bytes per profile, the floor in free space consumed
   (`df` delta), with `du` recorded beside it.
3. Measure the live set, not one command: the gate's cargo steps
   (`clippy --workspace --all-targets`, `cargo test --workspace`) and the debug
   desktop binary `pnpm dev` builds; release as the `macos` job builds it.
4. Re-derive each default from those numbers with the reasoning written next
   to it, in the style of the existing comments.

## Failure behavior

A measurement taken while other builds saturate the machine is still valid for
*size* (the numbers this ticket needs) but not for time; record load average
beside it. If disk free space would fall under the guard's floor during the
measurement, stop rather than finish.

## Acceptance criteria

- Both defaults carry a derivation dated to this ticket's measurement.
- `python3 scripts/runner-gc.py selftest` passes.
- A dry-run `sweep` against the real machine prints a plan consistent with the
  new budget.

## Tests required

`runner-gc.py selftest`; a dry-run `sweep`; `guard` at the new floor.

## Commands to run

```
python3 scripts/runner-gc.py selftest
python3 scripts/runner-gc.py sweep
python3 scripts/runner-gc.py guard
```

## Completion report

### Summary

**Both defaults stay as they are: `--cache-budget-gb 12` and `--floor-gb 25`.**
Both now carry derivations measured today. The premise that ADR-033 left the
budget "conservative rather than wrong", too generous, turned out not to hold.
Each build did shrink, but the workspace grew about as much since 2026-09-06,
and the old figure never included the `pnpm dev` binary. The measurement also
found the actual reason this checkout's `target/` exceeds its budget, and that
reason is not the budget (see "Found on the way").

### Method

`measure.py` in the session scratchpad (not committed) built one clean
generation into a **private, empty `CARGO_TARGET_DIR`**, so the checkout's
`target/` and concurrent sessions were untouched. It ran the live set step by
step and recorded sizes after each step: summed `lstat` sizes per profile, the
exact unit `plan_cache` uses, alongside `du` and the `df` delta. The steps
were the gate's two cargo builds, the debug app `pnpm dev` builds, and the
release sidecar + app `pnpm tauri build` compiles. The release steps call
`cargo build --release -p gritty-agent-hook` and
`cargo build --release -p gritty-desktop --features tauri/custom-protocol`
directly, because `prepare-sidecar.mjs` copies from the repository's own
`target/` and would have staged a stale binary into the shared tree. The
bundling step (≈16 MiB) is omitted.

Load averages of 10–94 on 8 cores during the build: two agent forks compiled at
the same time. Load does not affect sizes. The timings below are not
comparable to T140-PROFILE's.

### Results (cumulative, one clean generation)

| After step | debug cache (deps+build+.fp) | debug incremental | release cache | `du` target | `df` consumed | time |
|---|---|---|---|---|---|---|
| `clippy --workspace --all-targets` | 0.82 GiB | 0.43 GiB | – | 1.25 GiB | 1.37 GiB | 64 s |
| `test --workspace --no-run` | **3.48 GiB** | 2.02 GiB | – | 5.17 GiB | 5.41 GiB | 72 s |
| `build -p gritty-desktop -p gritty-agent-hook` | **4.15 GiB** | 2.61 GiB | – | 6.43 GiB | 6.51 GiB | 26 s |
| release sidecar | 4.15 GiB | 2.61 GiB | 0.10 GiB | 6.53 GiB | 6.60 GiB | 12 s |
| release app, `tauri/custom-protocol` | 4.15 GiB | 2.61 GiB | **1.40 GiB** | **7.95 GiB** | **7.87 GiB** | 216 s |

Of the debug live set, the workspace's own `gritty-*` crates are **1.77 GiB**
(`deps` 1.69 + `build` 0.08), and third-party crates are 2.10 GiB. The workspace
crates are what an edit rebuilds.

Against CI-RUNNER-GC's 2026-09-06 figures: the gate alone is 3.48 GiB
against 3.47, the same size. Release is 1.40 GiB against 1.24. A combined clean
generation is 7.95 GiB against 5.7, of which the `pnpm dev` binary accounts for
0.67 GiB of cache and 0.58 GiB of incremental.

### Derivations

- **Budget, 12 GiB per profile:** the debug live set (4.15), plus one whole
  dependency-bump generation beside it (4.15), plus two edit cycles of
  workspace crates (2 × 1.77), comes to 11.8 GiB. Lowering it evicts inputs that
  the next command reuses. Release has far more room than it needs, but it only
  grows when an app build runs, and making budgets per profile would change
  eviction logic, which is a non-goal here.
- **Floor, 25 GiB:** one cold macos job (≈8 GiB, plus node_modules and a
  65 MiB `target/fixtures`) leaves ≈16 GiB, room for about two more cold
  generations from concurrent lanes before a sweep. The ratio to a clean
  generation is now ~3.2x, not the 4.4x first intended, and the comment says
  when to raise it.

### Found on the way: the checkout is not being swept at all

The acceptance check "a dry-run `sweep` prints a plan consistent with the new
budget" **could not be met on the real checkout, and the reason matters more
than the budget.** The dry run reported:

```
/Users/alexistan/Desktop/CurrentProjects/GrittyIDE/target
  SKIPPED — live process rooted at …GrittyIDE: 76613 node …/@tauri-apps/cli/tauri.js dev
planned total: 0.000 GiB across 1 target root(s)
```

That `tauri dev` had been running for **6 h 04 min**. The process-table backstop
treats any process whose command line names the checkout as a live build, so a
long-lived dev session disables every sweep of that checkout for as long as it
runs: the Stop hook, the scheduled workflow and the CI post-job step alike. The
same tree held **27.9 GiB** of debug cache against the 12 GiB budget, plus
8.8 GiB of incremental that is supposed to be pruned unconditionally. The
authoritative check, Cargo's `.cargo-lock` flock, is only held while `tauri dev`
is actually rebuilding.

### Files changed

- `scripts/runner-gc.py`: the derivation comments on both defaults; no value
  and no logic changed.
- This task file.

### Tests actually executed

- `python3 scripts/runner-gc.py selftest`: exit 0, 8 tests OK.
- `python3 scripts/runner-gc.py sweep` (dry run): exit 0, skipped as above.
- `python3 scripts/runner-gc.py guard`: exit 0, 63.5 GiB free, floor 25.
- `measure.py`: exit 0, every step exit 0.

### Known limitations

- One measurement, as before. Not a distribution.
- The bundle step and `pnpm install` were not measured; the figures from
  2026-09-06 (16 MiB, 281 MiB) are cited unchanged.

### Follow-up tasks

- **GC-DEV-SKIP (a running `tauri dev` stops every sweep of its checkout).**
  Candidate fix: let the process scan ignore a dev server when the
  `.cargo-lock` flock is free. It needs its own ticket because it changes what
  counts as live, which CI-RUNNER-GC chose to fail safe on.
- Still open from T140-PROFILE: measure `incremental = false` locally (8.8 GiB
  of it here), and evaluate `sccache`.
