# Task SEARCH-WALK-TIME — Time a search over large-repo against ADR-022's revisit trigger

- **Phase:** 2
- **Status:** review
- **Depends on:** T080 (search), ADR-022 (in-process search on ripgrep's crates)

## Objective

Answer ADR-022's second revisit trigger with a number: is a walk over
`fixtures/large-repo` too slow with cancellation checked the way it is today?
If it is not, say so in ADR-022 and change nothing else.

## Motivation

ADR-022 names the trigger ("a measured walk over `fixtures/large-repo` … too
slow with the token checked at `CANCEL_CHECK_INTERVAL`") without a threshold.
T080's report carries one debug-build figure each for a filename walk (94 ms)
and a content search (759 ms), printed by tests, taken before `gritty-git`,
T130's ranking and this week's concurrent sessions changed the tree around it.
Nobody has measured release, repeated runs, or how long a cancellation takes to
be honoured — which is the half of the trigger that is about cancellation.

Note the ADR's wording is looser than the code: the walk in `walk.rs` polls
`cancelled` between every file and between `SCAN_CHUNK_BYTES` chunks;
`CANCEL_CHECK_INTERVAL` (256) belongs to quick-open's ranking in `rank.rs`.

## Required reading

- `docs/decisions/ADR-022-search-engine.md`
- `docs/tasks/completed/T080-search.md` (the bounds and the existing measurement)
- `crates/search/src/walk.rs`, `crates/search/src/rank.rs`, `crates/search/tests/walk.rs`

## Allowed scope

- a measurement harness in the session scratchpad (not committed)
- `docs/decisions/ADR-022-search-engine.md` — a dated measurement note under
  "Revisit when", and nothing in its decisions
- this task file

## Explicit non-goals

- No change to the walker, the batcher, or any bound.
- No parallel walker, no index.

## Required behavior

1. Time, over the generated `large-repo`, in debug and release, several runs
   each: a filename-scope walk, a content search for a literal that matches
   nothing (the full scan), and quick-open ranking.
2. Time cancellation: from the flag being set mid-walk to the call returning.
3. State the threshold used and where it comes from before reading the result.

## Failure behavior

A run taken under load is marked with the load average and not used as the
headline figure.

## Acceptance criteria

- Numbers recorded here with method, build profile, run count and load.
- ADR-022 carries a one-paragraph dated note saying whether its trigger fired.

## Tests required

None added; `cargo test -p gritty-search` still green.

## Commands to run

```
cargo test -p gritty-search
```

## Completion report

### Summary

**ADR-022's trigger did not fire.** Nothing in the walk changes. ADR-022 carries
a dated note saying so, and saying that its first trigger is now ADR-034's
subject (the meaning of "ignored", drafted today as Proposed).

### Threshold, and where it comes from

The plan sets no search budget. Its only latency figure that fits is
"open normal source file ≤ 100 ms perceived"
(`docs/Gritty IDE Project Plan.md:470`), used here as the bar for **a
cancellation being honoured** and for a quick-open answer. A full search has no
completion budget: it streams batches every 50 ms (T080), and "repository
enumeration never blocks editor; progressive results" is the plan's requirement
for it. "Too slow" for a full scan is therefore read as **seconds on a
10,000-file tree**, the scale at which an index or a parallel walker would start
to be argued for.

### Method

A scratchpad harness (not committed) built against `gritty-search`'s public
API (`search`, `rank_workspace_files`), with a sink that counts. It ran once
debug (`line-tables-only`, as ADR-033 sets) and once release, 11 runs per row
after one untimed warm-up, so the page cache is hot. Cancellation is timed from
another thread setting the flag at 5 ms and at 50 ms into a full scan, until
`search` returns `Cancelled`.

Two corpora, both on macOS (8 cores, APFS):

- **`large-repo`** (`target/fixtures/large-repo`): 10,429 files, **148 KB** in
  total. It tests file count, not content volume.
- **This checkout**: 11,449 files, **21 MB**. 10,429 of those files are the
  gitignored generated copy at `fixtures/large-repo`, since search does not
  honour `.gitignore` (ADR-022 decision 2). The content volume is the real
  repository's.

**Load.** The reported run took 1-minute load averages of 5.3–6.7 on 8 cores,
with another session's `rustc` running. A first run at load 10–13, during this
session's own builds, gave the same numbers within noise. Neither run was on a
quiet machine; both are close enough to agree.

### Results (median, min–max in brackets)

| Corpus | Operation | Release | Debug |
|---|---|---|---|
| large-repo | filenames only | 24 ms (24–25) | 62 ms (60–126) |
| large-repo | content, no match (full scan) | **197 ms** (191–227) | **242 ms** (231–348) |
| large-repo | both scopes, literal `fn ` | 213 ms (191–311) | 272 ms (234–315) |
| large-repo | content, regex `\bimpl\s+\w+` | 231 ms (205–327) | 238 ms (228–276) |
| large-repo | quick-open rank `ts` | 26 ms (26–28) | 73 ms (72–76) |
| large-repo | cancel honoured, flag at 50 ms / 5 ms | 0.1 / 0.1 ms | 0.1 / **1.4 ms** (max 2.3) |
| checkout | filenames only | 36 ms | 98 ms |
| checkout | content, no match (full scan) | **238 ms** (234–294) | **591 ms** (562–819) |
| checkout | both scopes, `fn ` (3,482 matches) | 237 ms | 548 ms |
| checkout | content, regex (433 matches) | 243 ms | 622 ms |
| checkout | quick-open rank `ts` | 44 ms (41–70) | **157 ms** (155–163) |
| checkout | cancel honoured, flag at 50 ms / 5 ms | 0.1 / 0.0 ms | 0.0 / 0.8 ms (max 2.0) |

T080's figures, taken in a debug build before ADR-033, were 94 ms (filenames)
and 759 ms (content) on large-repo. Today's debug numbers are lower on both.

### Reading

- **Cancellation is 40× inside the bar at worst.** The worst case (1.4–2.3 ms,
  debug, flag at 5 ms) is the walker's setup before its first poll, not the
  gap between polls.
- **A full scan's cost is per file, not per byte.** Scanning 148 KB across
  10,429 files takes 197 ms in release, and 21 MB across a similar file count
  takes 238 ms, so `open` and `read` per file dominate. That points at the walk
  rather than the matcher. It is also the one thing `ignore`'s parallel walker
  would change, and at a quarter of a second there is nothing yet to buy with
  it.
- **Observation for quick-open, not this ADR.** In debug on the checkout, a
  ranked walk takes 157 ms, and `QUICK_OPEN_DEBOUNCE_MS` (120,
  `apps/desktop/src/features/quick-open/use-quick-open.ts:48`) adds to it, so
  about 280 ms from a pause to an answer in `pnpm dev`. Release is about
  165 ms. The shipped build is inside what the debounce comment calls "reads
  as instant"; the dev build is not. Recorded, not acted on.

### Files changed

- `docs/decisions/ADR-022-search-engine.md`: the dated note under
  "Revisit when".
- This task file.

### Tests actually executed

- The harness, twice (above). `cargo build` of it in debug and release: exit 0.
- `cargo test --workspace --no-fail-fast`: exit 0, 1,271 passed, 0 failed.
  `gritty-search` accounts for 34 unit, 18 `rank`, and 22 `walk` tests.

### Known limitations

- One machine, macOS, hot page cache. A cold-cache first search after boot
  will be slower, and is not measured.
- The checkout corpus is mostly the fixture again, by file count.
- The harness calls `gritty-search` directly. `gritty-core`'s batcher and the
  IPC hop are not in these numbers. Both are bounded per batch, not per file.

### Follow-up tasks

- None required by ADR-022.
- **ADR-022 wording** is corrected by the dated note rather than edited in
  place, since the ADR is accepted.
