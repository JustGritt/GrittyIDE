# ADR-033: The dev profile is set deliberately, and debug info is the thing it sets

- **Status:** Proposed
- **Date:** 2026-09-09
- **Decided by:** nobody yet. Drafted by the agent that measured it;
  `docs/plans/execution-board.md` reserves every ADR verdict for lane H, so this
  stays `Proposed` until Alexis records one — as ADR-017, ADR-021, ADR-022,
  ADR-023, ADR-024 and ADR-028 also do.
- **Relates to:** ADR-024 (target retention), which this does **not** amend —
  see §1.

## Context

`target/` reached 68 GB on a volume with 18 GiB free, and the disk has cost
real time on three separate occasions. The sharpest was
`ld: write() failed, errno=28` under a wall of linker arguments, which renders
as `error: could not compile gritty-core` and reads like your own broken build.

ADR-024 asked whether `CARGO_TARGET_DIR` should be shared across worktrees and
answered no, pairing that with a per-directory sweeper (`scripts/runner-gc.py`,
CI-RUNNER-GC). Both halves of that answer are about **how many** target
directories exist and **how long** their contents live.

**Neither half asks what a single build produces**, and nothing else in this
repository ever has. The root `Cargo.toml` has **no `[profile.dev]` section at
all**: `debug`, `split-debuginfo`, `incremental` and `opt-level` are every one
of them a Cargo default nobody chose. A repository-wide grep finds no mention
of `split-debuginfo`, `sccache`, or a linker choice, in any document or config.

That is the gap this ADR fills, and it is genuinely open ground rather than a
reversal: ADR-024's three rejection arguments are the Cargo build lock,
generated-output corruption, and "sharing does not solve disk anyway". All
three are about *sharing*. None of them touches *shrinking*.

### What the measurement found

Taken 2026-09-09 on a quiet machine, after killing 16 orphaned busy-loops that
had been saturating every core for two days (`CARGO-FLAKE` — the first attempt
at these numbers was invalid, at load average 66).

Where the 82 GB was:

| | |
|---|---|
| `target/debug/deps` | 81.6 GB logical, 127,522 files |
| └ `.rcgu.o` codegen-unit objects | **52.0 GB — 64%** |
| └ `.rlib` | 17.0 GB |
| One codegen-unit object, by section | **≥69% DWARF, 7.7% machine code** |
| Codegen units per crate | **256** (Cargo's dev default) |
| Generations of one rlib kept | **32, 9.6 GB**, over three days |

And, decisively, what one *clean* build costs:

| | |
|---|---|
| One clean `cargo build --workspace --all-targets` | **5 GiB** by `df`, 5.6 GB by `du`, 76 s |

**So the 68 GB was about twelve generations of history, not a large build.**
That is what makes the sweeper the larger lever and this decision the
multiplier on it: every generation the sweeper has not yet evicted is this
size, and this ADR decides how big "this size" is.

The mechanism, since it is not obvious: Rust splits each crate into 256 codegen
units in the dev profile and emits one object file per unit; on macOS
`split-debuginfo` defaults to `unpacked`, which leaves DWARF in those loose
`.o` files rather than folding it into a binary or a `.dSYM`. Multiply 256
units by ~700 crates by every historical build, at roughly 70% debug info, and
82 GB is the result.

## Decision

**`[profile.dev] debug = "line-tables-only"`, and nothing else.**
`split-debuginfo` stays at the platform default, which the measurement below
says is the right call for a reason nobody would have guessed.

Three candidates, each a clean `cargo build --workspace --all-targets` on a
quiet machine:

| | Baseline (defaults) | **line-tables-only** | packed |
|---|---|---|---|
| `df` delta | 5 GiB | **4 GiB** | 6 GiB |
| Wall clock | 76 s | **65 s** | 87 s |
| `target/` (du) | 5.6 GB | **4.3 GB** | 6.7 GB |
| `deps` logical | 3.6 GB | **2.7 GB** | 3.8 GB |
| └ `.rcgu.o` | 1.4 GB | 0.8 GB | **0.0 GB** |
| └ `.rlib` | 1.3 GB | 1.0 GB | 1.3 GB |
| Files in `deps` | 7,282 | 7,282 | **1,380** |

**`line-tables-only` wins on every axis that matters**: 23% less disk per
build, and **14% faster**. The speed-up was not predicted and is not noise on a
quiet machine — less debug info is less for rustc to emit and less for the
linker to read.

**`split-debuginfo = "packed"` is worse than doing nothing**, and this is the
finding worth keeping. It does exactly what it promises — `.rcgu.o` drops to
**zero** and the file count in `deps` falls 81%, because `dsymutil` folds those
objects into `.dSYM` bundles. But the DWARF is not removed, it is *relocated*,
and per-binary `.dSYM` bundles duplicate what shared object files did not:
total disk goes **up** 20% against the baseline and the build is **14%
slower**. A change that empties the directory you were looking at while growing
the total is precisely the shape of thing that gets adopted on a `du` of the
wrong subdirectory.

**The debugging trade was checked, not assumed.** Under `line-tables-only` a
panic still produces frames carrying file and line:

```
2: tmp_panic_for_backtrace_check
         at ./src/testing.rs:47:9
```

What is given up is type and variable information in a stepping debugger.
Every failure in this codebase arrives as a panic or a test assertion, so the
part that is kept is the part that is used.

## Alternatives

- **Do nothing.** The status quo cost time three times in three days and
  produced a failure mode that does not look like a disk problem. `T140-SWEEP`
  alone bounds accumulation but leaves each generation at its default size.
- **`debug = 0`.** The largest possible reduction, and the worst trade:
  backtraces lose line numbers entirely, so a panicking test says what happened
  and nothing about where. On a workspace this size that is a tax on every
  failure, paid to save disk that the sweeper already reclaims.
- **A shared `CARGO_TARGET_DIR`** — ADR-024's question, and still no. Worth
  recording that its *second* rejection argument does not hold: sharing would
  not reintroduce the `packages/protocol/src/generated/` corruption, because
  those types are written into the **source** tree and worktrees have separate
  source trees. The build-lock argument stands on its own, and the measurement
  adds a new one: sharing saves one ~5.6 GB baseline per worktree while the
  accumulated generations are additive either way, so it addresses the smaller
  half of the problem.
- **`sccache`.** A shared compilation cache dedupes across checkouts *without*
  a build lock, which is the version of "share it" that survives ADR-024's
  objection. Never considered in this repository. Deliberately not decided
  here: it is a new dependency, it needs its own measurement, and this ADR is
  about a setting that costs nothing to adopt or revert.

## Consequences

- **Every build on this machine is 23% smaller and 14% faster.** Multiplied
  across the ~12 generations a sweep interval accumulates, that is the
  difference between a 68 GB steady state and roughly 52 GB — real, and still
  second to `T140-SWEEP`, which is what stops the accumulation at all.
- **A stepping debugger loses variable inspection** in dev builds. Anyone who
  needs it sets `debug = true` locally for that session; it is one line and it
  reverts.
- **The self-hosted macOS CI job shares this checkout's `target/`**, so it
  inherits both the smaller builds and the one cold rebuild the change costs.
  The hosted Linux job's `Swatinem/rust-cache@v2` key changes and it pays the
  same one-time rebuild.
- **`scripts/runner-gc.py`'s defaults are now conservative rather than wrong.**
  `--cache-budget-gb 12` was derived from a 3.47 GiB measurement of the old
  profile's `deps`+`build`+`.fingerprint`; the new baseline is smaller, so the
  budget is a larger multiple of a build than intended. Re-deriving it is
  filed, not done — a budget that is too generous wastes disk, while one that
  is too tight evicts a live build's inputs.
- **`incremental` and `opt-level` remain untouched defaults.** This ADR
  deliberately changes one thing. `incremental` is the obvious next candidate
  (CI already sets `CARGO_INCREMENTAL=0`, so only local dev pays for it) and
  wants its own measurement.

## Revisit when

A second real measurement contradicts these, `sccache` is evaluated, or the
toolchain's `split-debuginfo` default changes on macOS.
