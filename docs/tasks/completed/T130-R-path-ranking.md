# Task T130-R — A fuzzy path scorer, and the walk that ranks with it

- **Phase:** 2
- **Status:** done
- **Depends on:** T080 (the walk, the ignore predicate and the cancellation shape it already proved)

> The ID is an addition to the project plan's §17 numbering, not from it, the
> same way T041, T042 and T045 are. T130 opens a family for quick-open because
> the feature is a *consumer* of search rather than part of it: T080's own
> module header disclaims it by name.

## Objective

The ranking half of quick open, in `crates/search`: a pure subsequence scorer
over workspace-relative paths, and a bounded, cancellable walk that returns the
best `limit` paths for a query rather than every path that contains it.

## Motivation

`crates/search/src/lib.rs:31-32` says what is missing, in the crate's own words:

> *"Not an index: nothing is stored and nothing is remembered between calls.
> **Not a ranked quick-open**: `SearchScope::Filenames` is plain substring or
> regex matching against the corpus already walked."*

`walk.rs:370-384` (`filename_match`) is `query.matcher().find(path.as_str().as_bytes())`
— a literal or regex hit, returned in walk order with no relevance signal at
all. `grep-regex` cannot express `epn → editor-pane.tsx`, and `SearchMatch`
carries a single `matchStart`/`matchLength` pair, which is one contiguous run
and cannot describe a subsequence hit's positions.

So neither the scoring nor the shape exists today. This ticket adds both, on the
walk T080 already built, and deliberately adds **no index**: T080's own non-goal
argued an index buys speed at the cost of a durable structure and its
invalidation problem, and the measured walk does not need one:

```
fixtures/large-repo, 10,429 files, search.query scope=filenames   94 ms
  (T080's own completion report, docs/tasks/completed/T080-search.md:448-451)
this repository,     11,318 files, `find` under the same 11-name predicate   39 ms
```

94 ms sits just inside `DEFAULT_LIST_BUDGET` (100 ms), which is exactly why
T130-C's response is a budget-then-operation union rather than a decoration —
a repository modestly larger than the fixture tips past it.

## Required reading

- `crates/search/src/lib.rs` — the crate header, and the two ADR-022 constraints
  it is bound by
- `crates/search/src/walk.rs:78-140` — `search()`, and the `WalkBuilder`
  configuration this ticket must share rather than copy
- `crates/search/src/query.rs:39-69` — `SearchScope`, `IgnorePredicate`,
  `default_ignore_predicate()`
- `crates/workspace/src/listing.rs:27-56` — `IGNORED_DIRECTORY_NAMES`,
  `is_ignored`, `CANCEL_CHECK_INTERVAL`
- `docs/decisions/ADR-022-search-engine.md` — decisions 1 and 2 in particular
- `AGENTS.md` hard rules 8, 10, 14

## Allowed scope

- `crates/search/**`
- `docs/tasks/completed/T130-R-path-ranking.md`

Not the root `Cargo.toml`: this ticket adds **no dependency** (see non-goals),
and that file is a shared magnet currently held by another lane.

## Explicit non-goals

- **No protocol, no wire, no command.** `workspace.findFiles` is T130-C's, and
  it does not exist yet. This crate knows nothing about `gritty-core`, exactly
  as its header already promises.
- **No index and no cache.** Nothing is stored between calls. New durable state
  with its own invalidation is what hard rule 9 pushes back on, and the number
  in Motivation says it is not needed. Record it as a follow-up if a real repository disproves
  the number.
- **No new dependency.** Not `nucleo`, `fuzzy-matcher`, `skim` or `fzf`. The
  dependency policy wants a justification and no overlap for each one; a
  subsequence scorer with boundary bonuses is small enough to own, and owning it
  keeps the scoring rules reviewable and the tie-breaks deterministic.
- **No content matching.** This ranks paths. `search.query` still owns grep.
- **No changes to `search()`'s behaviour.** Extracting the walker configuration
  it shares with this module must leave its results byte-identical.

## Required behavior

1. **A pure scorer, split from the walk.** `score_path(query, path) -> Option<PathScore>`
   touches no filesystem, so every scoring rule is table-tested. `None` means
   the query is not a subsequence of the path; scoring and matching are the same
   pass, not two.
2. **Case-insensitive subsequence, with an exact-case bonus.** `epn` matches
   `editor-pane.tsx`; `EPN` matches it too, but a run whose case matches scores
   higher, so `TSX` prefers `App.tsx` over `app.tsx.bak`.
3. **The bonuses that make a ranking feel right**, each of them a test:
   consecutive runs; a character following `/`, `_`, `-` or `.`; a lowercase→
   uppercase transition (camelCase); the basename over the directory portion;
   the start of the basename. Gaps and total path length are penalties.
   **`epn` → `editor-pane.tsx` above a path where the same letters are buried
   mid-word (`.../expand.rs`) is the acceptance test.** Choose the
   counter-example with care: a path like `expand-none.rs` gives its `n` a
   word-boundary bonus and is a genuinely competitive match, so it proves
   nothing about the ranking.
4. **Match positions come back with the score.** `PathScore.ranges` is a set of
   `(start, len)` spans over the path's **char** indices, merged so adjacent
   matches are one span. The UI must not re-run the matcher to highlight, and a
   byte index would be wrong the moment a path contains a non-ASCII character.
5. **A cheap prefilter before the expensive scorer.** An `O(path)` subsequence
   test rejects the large majority of paths; only survivors are scored. State
   the two-stage shape in the code — it is the reason a per-query walk is
   affordable at all.
6. **Bounded output, bounded memory.** `rank_files` keeps a heap of at most
   `limit` and reports `truncated` when it discarded a scored candidate. A
   repository ten times the size costs the same memory.
7. **Deterministic order.** Score descending, then workspace-relative path
   ascending. A tie must never depend on walk order, or a test cannot assert on
   a result list and a user sees rows move between identical queries.
8. **Cancellation between entries**, as `list_directory` and `search()` already
   do. A walk that cannot be cancelled is a renderer-blocking walk by hard
   rule 10.
9. **The ignore predicate stays one thing.** Reuse `default_ignore_predicate()`
   and the `WalkBuilder` configuration `search()` already sets — every gitignore
   source off, `follow_links(false)`, `sort_by_file_name` — by **extracting it
   into a shared helper**, not by writing a second one. ADR-022 decision 2 is
   that "ignored" means the same thing in the explorer, the watcher and search;
   a copy is how that stops being true.
10. **An empty query is not an error.** It returns the first `limit` paths in
    walk order with score 0 and no ranges — what a quick-open shows before a
    key is pressed.

## Failure behavior

- Root missing between the command and the walk → the same `SearchError::io`
  `search()` raises. "The search failed" and "there were no matches" are
  different answers and only one is true.
- Cancelled → `SearchError::Cancelled`, with nothing partial returned.
- A single unreadable directory → counted and skipped, never fatal. One refused
  directory must not lose the other ten thousand paths.
- A query longer than `MAX_QUERY_CHARS` → the existing `InvalidQuery`, not a
  silent truncation.
- A path that is not valid UTF-8 → skipped and counted. It cannot be scored, and
  guessing an encoding to rank it would be worse than omitting it.
- A symlinked directory is never descended (inherited from the shared walker).

## Acceptance criteria

- `score_path("epn", "apps/desktop/src/features/editor/editor-pane.tsx")` outranks
  every other path in `fixtures/` for that query.
- Ranking a fixture workspace twice returns an identical list.
- Cancelling mid-walk returns `Cancelled` promptly and allocates no result.
- `limit` is honoured exactly, and `truncated` is true when it bites.
- `search()`'s existing tests pass unchanged after the walker extraction.
- `cargo clippy --workspace --all-targets -- -D warnings` clean.

## Tests required

- **Table tests for the scorer** — the bulk of the work, and pure: subsequence
  hits and misses; each bonus asserted by an ordering pair rather than a
  magic number, so the constants can be tuned without rewriting the suite;
  empty query; query longer than the path; non-ASCII paths asserting char, not
  byte, ranges; adjacent matches merged into one span.
- **Determinism**: two runs over the same fixture, asserted equal, and a
  score-tie asserted to break on path.
- **Bounds**: `limit` of 1, 0 and more than the corpus; `truncated` set only
  when a scored candidate was actually discarded.
- **Cancellation**: a predicate that flips after N entries, asserting
  `Cancelled` and that the walk stopped rather than finishing quietly.
- **The ignore predicate is consulted per directory**, reusing T080's technique
  of injecting a counting predicate — the property that matters is that the
  walk *asks* before descending.
- **Extraction regression**: the existing `walk.rs` suite, unchanged.

## Commands to run

```
cargo test -p gritty-search
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
pnpm check
```

## Completion report

### Summary

`crates/search` gained a second entry point. `rank_files` walks the tree once,
scores every path that survives an `O(path)` subsequence prefilter, and returns
the best `limit` with their match positions. The scorer is a pure
dynamic-programming table split from the walk, so all nineteen of its rules are
tested without a filesystem. No index, no cache, no new dependency, and no
protocol.

### Files changed

| File | Why |
|---|---|
| `crates/search/src/rank.rs` | New. `Scorer`, `score_path`, `rank_files`, `rank_workspace_files`, and 19 unit tests. |
| `crates/search/tests/rank.rs` | New. 18 integration tests over `small-repo`, `large-repo` and scratch trees. |
| `crates/search/src/walk.rs` | Extracted `build_walker` and `entry_path` from `search()` so both walks share one configuration; no behaviour change. |
| `crates/search/src/lib.rs` | Exports, and the header paragraph that said "Not a ranked quick-open" — which this ticket made untrue. |
| `docs/tasks/T130-{C,R,U,S,M}-*.md` | New. The five-way split. |

### Tests actually executed

| Command | Result |
|---|---|
| `cargo test -p gritty-search` | **72 passed, 0 failed** — 32 lib (19 of them `rank::`), 18 `tests/rank.rs`, 22 pre-existing `tests/walk.rs` |
| `cargo clippy -p gritty-search --all-targets -- -D warnings` | clean |
| `cargo fmt --all --check` | clean |
| `pnpm lint` (biome, 290 files) | clean |
| `pnpm typecheck` | clean |
| `pnpm test` (vitest) | **958 passed, 97 files** |
| `cargo check --workspace --all-targets` | clean |
| `cargo test --workspace` | **no failures** |
| `pnpm check` (whole gate) | **red — not on this ticket.** See below. |

The 22 pre-existing `walk.rs` tests passing unchanged is the evidence that the
`build_walker` extraction was behaviour-preserving.

**`pnpm check` does not currently pass in this tree, and none of it is this
ticket's.** The tree is shared with `W-scm`, which is mid-flight on T102-C/R
inside its `packages/protocol/src/**` lease. Two states were observed an hour
apart, which is worth recording because the second looks like a regression of
the first and is not:

1. *Earlier:* the Rust workspace did not compile — `crates/gritty-core/src/git/mod.rs:117`
   referenced `gritty_git_write` before it was a workspace dependency (4 errors,
   all in that file). `W-scm` has since fixed this; re-verified clean here.
2. *Now:* the TypeScript half is red instead — two **fixable**
   `assist/source/organizeImports` errors in `packages/protocol/src/{client,types}.ts`,
   a `TS2339` union-narrowing error in their new `src/git-write-client.test.ts`,
   and `check:generated` stale on six untracked `generated/Git*.ts`.

Nothing in either state is in `crates/search`. **This ticket has therefore been
verified by decomposition rather than by the whole gate**: its own crate, the
whole Rust workspace, and vitest — with biome, `typecheck` and `check:generated`
outstanding on another lane's files. Re-run `pnpm check` once T102 lands; no
change to this crate should be needed.

One further note for whoever re-runs it: `cargo test --workspace` failed
`gritty-desktop --test commands::root_removed_while_open_…` on one run and
passed it in isolation and on the next full run. It looks like another
load-sensitive flake of the kind `VITEST-FLAKES-load-sensitive` and `PTY-FLAKE`
already track, and it is unrelated to this crate — but it is recorded rather
than dismissed.

### Architectural decisions

**No index, and the measurement is why.** T080's own report measured a
filename-scope walk of 10,429 files at 94 ms; a `find` over this repository's
11,318 non-ignored files is 39 ms. A durable corpus would buy speed and pay for
it with an invalidation problem, which is what hard rule 9 pushes back on. The
prefilter is what makes the walk affordable: scoring is `O(query × path)` and
almost nothing reaches it.

**The walker was extracted, not copied.** ADR-022 decision 2 says "ignored"
means one thing across the explorer, the watcher and search. A second
`WalkBuilder` with its own switches is precisely how that stops being true, so
`search()` and `rank_files` now call one `build_walker`.

**The scorer is a DP table, not a greedy scan.** A greedy left-to-right match is
cheaper and ranks `epn` against `editor-pane.tsx` wrongly — it takes the first
`e` and never discovers that a later alignment scores higher. The table also
gives the match positions for free, by walking its parents back out.

**Gap-opening costs more than the boundary bonus it can reach** (`-14` against
`+12`). This was found by a failing test, not by design: with a flat `-1` gap
cost, `a-b-c.ts` beat `abc.ts` for `abc`, because every separator paid for the
jump that reached it. A solid run must win.

**No positional penalty on the first character.** The first draft charged `-1`
per character of offset, which is a penalty on *directory depth* — it made
`apps/desktop/src/features/editor/editor-pane.tsx` lose to a shallow path with
the same letters buried mid-word. Depth is not irrelevance. Path length is paid
for once, at the end.

**Ranges are char indices, not bytes**, because the UI slices the path string to
highlight it and a byte offset is wrong for any non-ASCII path. There is a test.

**The heap evicts rather than the walk stopping at `limit`.** Stopping early
would return the first `limit` matches, not the best; `keeping_the_best_limit_is_not_keeping_the_first_limit`
puts the single best match behind forty worse ones to prove it survives.

### The ranking was wrong, and only a real corpus showed it

The suite passed and the ranking was still wrong. Run against this repository
rather than against fixtures, `epn` returned:

```
1.  79  fixtures/weird-files/binary/pixel.png       ← wrong
2.  73  apps/desktop/src/features/editor/editor-pane.tsx
```

`pixel.png` matched `e`(pix**e**l) `p`(**p**ng) `n`(p**n**g) — one boundary hit
plus one consecutive hit, on a short path. Every per-character bonus was working
as specified. **The acceptance test passed only because the counter-example was
chosen by the same person who wrote the scorer**, which is the failure mode a
table test cannot catch: it proves the rule you thought of, not the ranking.

Two signals were missing, both about the match *as a whole* rather than any
character in it, so neither could be expressed as a positional bonus:

- `WHOLLY_IN_BASENAME_BONUS` — every matched character is in the file name, not
  borrowed from a directory above it.
- `STARTS_THE_BASENAME_BONUS` — the query begins exactly where the file name
  begins. This is the strongest signal a quick-open has, and it is why fzf gives
  the first character its own multiplier.

The first alone did not fix it (`pixel.png`'s match is *also* wholly in its
basename — the first analysis was wrong and the test said so). The second is
what settles it. Both are pinned by
`a_match_wholly_inside_the_basename_beats_one_that_borrows_a_directory`, which
uses the real losing path, and by
`a_directory_query_still_matches_across_the_separator`, which stops the basename
reward becoming a basename *requirement*.

Re-validated across nine hand-written queries against this repository —
`epn`, `appbar`, `rank`, `wksp`, `adr29`, `qopen`, `tabstrip`, `usedoc`, `mq` —
each returning the intended file first, with no regression in the other eight.
Walks were 36–107 ms over 11,352 files, which is the 94 ms this ticket was
designed around.

### Known limitations

- **Ignoring is still the eleven directory names.** No `.gitignore` support, so
  a repository with a large ignored-but-unnamed build directory will see it in
  quick-open results. ADR-022 decision 2 already defers real gitignore.
- **The constants are tuned against a handful of ordering tests**, not against a
  corpus of real queries. They are asserted relatively so they can be retuned
  without rewriting the suite, but nobody has judged the ranking on a screen.
- **`rank_files` is synchronous and blocking.** It takes a `cancelled` closure
  and nothing else; wrapping it in `spawn_blocking` with a `CancellationToken`
  is T130-C's job, as `search/mod.rs` already does.
- **The scorer allocates a `parent` table of `query × path`.** Bounded by
  `MAX_QUERY_CHARS` (1,024) and the OS path limit, and reused across paths by
  `Scorer`, but a pathological query against a pathological path is a large
  allocation. Not reachable through the contract's 200-char cap on real input.

### Follow-up tasks

| Ticket | Why |
|---|---|
| T130-C | The wire. Blocked on `W-scm`'s lease, not on a decision. |
| A file index | Only if a measured walk on a real repository is too slow. Do not build it on the strength of the two numbers above. |
| Real `.gitignore` support | Would improve every surface, not just this one. ADR-022 names it. |
| Ranking judged on screen | The constants need a person, not a test. |
