# Task T140-PROFILE — Measure, then set the dev profile's debuginfo

- **Phase:** 2
- **Status:** doing
- **Depends on:** ADR-033 (the decision this measurement informs)

## Objective

Decide what `[profile.dev]` should be, on measurements rather than folklore,
and set it. The root `Cargo.toml` has **no `[profile.dev]` section at all**, so
every debuginfo-related default is one nobody chose.

## Motivation

`target/` reached 68 GB on a volume with 18 GiB free and cost time on three
separate occasions, most sharply as `ld: write() failed, errno=28` under a wall
of linker arguments — a disk failure wearing a compiler error's clothes.

`T140-SWEEP` bounds *accumulation*. This ticket attacks *what one build
produces*, which is the other half and the one nothing in this repository has
ever looked at. ADR-024's three rejection arguments are all about **sharing** a
target directory; none of them touches **shrinking** one.

Where the volume came from, measured 2026-09-09 before any change:

| | |
|---|---|
| `target/debug/deps` | 81.6 GB logical, 127,522 files |
| └ `.rcgu.o` codegen-unit objects | **52.0 GB (64%)** |
| └ `.rlib` | 17.0 GB |
| One codegen-unit object, by section | **≥69% DWARF debug info, 7.7% machine code** |
| Codegen units in one build of `gritty_core` | **256** |
| Generations of `libgritty_desktop_lib.rlib` kept | **32, 9.6 GB**, over three days |

The arithmetic that produces 82 GB is 256 codegen units × ~700 crates × every
historical build, at roughly 70% debug info — and on macOS
`split-debuginfo` defaults to `unpacked`, so those `.o` files are where DWARF
lives and they are never folded away.

## Method

`scripts/`-external measurement script, one run per candidate: `cargo clean`,
then `cargo build --workspace --all-targets` (what the gate builds), recording
the **`df` delta** — not `du`, and not `stat`. APFS clonefiles make both
overstate: this tree reported 81.6 GB by `stat` and 66 GB by `du`, and
CI-RUNNER-GC measured 4.715 GiB of `du` yielding a 2.76 GiB `df` delta.

**The machine must be quiet.** The first attempt was invalid: 16 orphaned
busy-loops from a dead session had been saturating every core for two days
(load average 66 — see `CARGO-FLAKE`). Every number below was taken after they
were killed, with load falling through 17 and 86 GiB free.

## Results

### Baseline — no `[profile.dev]`, Cargo defaults

| | |
|---|---|
| `df` delta for one clean build | **5 GiB** |
| Wall clock | **76 s** |
| `target/` (du) | 5.6 GB |
| `target/debug/deps` logical | 3.6 GB |
| └ `.rcgu.o` | 1.4 GB |
| └ `.rlib` | 1.3 GB |
| Files in `deps` | 7,282 |

**One clean build is 5.6 GB.** The observed 68 GB steady state was therefore
roughly *twelve generations* of accumulated history, not a large build — which
is what makes `T140-SWEEP` the bigger of the two levers, and this one the
multiplier on it. It also lines up with ADR-024's own figure of ~5.7 GiB for a
combined clean debug+release build.

### `debug = "line-tables-only"`

| | Baseline | line-tables-only | Change |
|---|---|---|---|
| `df` delta for one clean build | 5 GiB | **4 GiB** | −20% |
| Wall clock | 76 s | **65 s** | **−14%** |
| `target/` (du) | 5.6 GB | **4.3 GB** | −23% |
| `deps` logical | 3.6 GB | **2.7 GB** | −25% |
| └ `.rcgu.o` | 1.4 GB | **0.8 GB** | **−43%** |
| └ `.rlib` | 1.3 GB | 1.0 GB | −23% |
| Files in `deps` | 7,282 | 7,282 | unchanged |

The `.rcgu.o` figure is the one that matters: **−43%** on the category that was
64% of the 82 GB. Same file count — the objects are still there, each carrying
line tables instead of full DWARF.

The **14% faster build is a bonus nobody predicted**, and it is not noise on a
quiet machine: less debug info is less for rustc to emit and less for the
linker to read.

### `split-debuginfo = "packed"` — measured, and rejected

| | Baseline | packed | Change |
|---|---|---|---|
| `df` delta | 5 GiB | 6 GiB | **+20% worse** |
| Wall clock | 76 s | 87 s | **+14% slower** |
| `target/` (du) | 5.6 GB | 6.7 GB | **+20% worse** |
| `deps` logical | 3.6 GB | 3.8 GB | +6% |
| └ `.rcgu.o` | 1.4 GB | **0.0 GB** | eliminated |
| Files in `deps` | 7,282 | **1,380** | −81% |

It did exactly what it promises and the promise was not the thing we wanted.
`dsymutil` folds the codegen-unit objects into `.dSYM` bundles, so `.rcgu.o`
goes to **zero** and the file count in `deps` falls by 81% — but the DWARF is
**relocated, not removed**, and per-binary `.dSYM` bundles duplicate what shared
object files did not. Total disk goes *up*.

This is why the plan said to measure it rather than reason about it. A change
that empties the directory you were staring at while growing the total is
exactly what gets adopted on a `du` of the wrong subdirectory.

### Verdict

**`debug = "line-tables-only"`**, adopted. Smallest disk, fastest build, and
the backtraces that matter are intact.

## Acceptance criteria

- A `[profile.dev]` chosen on numbers, with all candidates recorded — including
  any rejected — so nobody re-measures.
- A panic backtrace still names a file and line under the adopted setting.
  This is the thing being traded away; check it rather than assume it.
- `scripts/runner-gc.py`'s `--cache-budget-gb` / `--floor-gb` re-derived if the
  baseline moves: their current defaults carry dated derivations from the
  *old* profile's numbers.
- `pnpm check` green on a quiet machine.

## Blast radius

- The **self-hosted macOS CI job** runs in this same checkout against this same
  `target/`.
- The **hosted Linux job** uses `Swatinem/rust-cache@v2`; its key changes and it
  pays one cold rebuild.
- Every agent worktree.

## Completion report

### Summary

`[profile.dev] debug = "line-tables-only"` in the root `Cargo.toml`, with the
reasoning inline. **23% less disk per build and 14% faster**, verified against
two rejected alternatives rather than picked.

### Tests actually executed

| Check | Result |
|---|---|
| Three clean `cargo build --workspace --all-targets` runs | see the table above |
| Backtrace probe under the adopted setting | **frames carry `at ./src/testing.rs:47:9`** |
| `pnpm check` | run after adoption |

The backtrace probe was a temporary `#[test] #[ignore]` panic added to
`crates/settings/src/testing.rs`, run with `RUST_BACKTRACE=1`, and reverted —
`git status` confirms the file is clean. Hard rule 12 says prove recovery
behaviour rather than claim it; the same standard applies to a debugging
capability being traded away.

### Architectural decisions

- **`split-debuginfo` left at the platform default**, because measuring it
  showed `packed` is worse than doing nothing. Recorded in ADR-033 so nobody
  re-derives it from first principles and reaches the intuitive wrong answer.
- **One thing changed.** `incremental` and `opt-level` remain defaults.

### Known limitations

- **A stepping debugger loses variable inspection** in dev builds. Set
  `debug = true` locally for a session that needs it.
- **The numbers are one run each, not a distribution** — the same limitation
  `runner-gc.py`'s own defaults carry. They were taken on a quiet machine with
  86 GiB free, which is the part that was wrong the first time.
- **`--cache-budget-gb` is now conservative rather than calibrated.** Its
  12 GiB default came from the *old* profile's 3.47 GiB measurement. Filed
  below; a too-generous budget wastes disk, a too-tight one evicts live inputs.

### Follow-up tasks

- Re-derive `runner-gc.py`'s `--cache-budget-gb` and `--floor-gb` against the
  new baseline.
- Measure `incremental = false` for local dev. CI already sets
  `CARGO_INCREMENTAL=0`, so only local development pays for it, and
  `incremental` was 20 GB of the 88 GB one session measured earlier.
- Evaluate `sccache` — a shared compilation cache that dedupes across
  checkouts without the build lock ADR-024 rejected sharing for.
