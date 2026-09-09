# Task T130-C — The quick-open contract: `workspace.findFiles`

- **Phase:** 2
- **Status:** doing
- **Depends on:** T080, T130-R (land the contract *with* something behind it)

## Objective

One additive command, `workspace.findFiles`, that takes a query and returns the
best-ranked workspace paths with their match positions — the wire T130-U needs
and the only thing standing between T130-R's scorer and a user.

## Motivation

There is no command that can answer "which files look like `epn`". `workspace.list`
is strictly one directory level (`ListDirectoryRequest`, ≤5000 entries per page),
and there is no recursive listing command anywhere on the wire. `search.query`
walks the tree but returns matches in walk order with no score, streamed as
`SearchMatchesFound` events sized for content search.

The shape difference that settles it: `SearchMatch` carries a single
`matchStart`/`matchLength` pair. A substring hit is one contiguous run; a
subsequence hit (`epn` → **e**ditor-**p**a**n**e) is a *set* of positions and
cannot be expressed on that shape at all. Ranking client-side over `search.query`
would therefore also mean re-running the matcher in TypeScript purely to
highlight — the scorer written twice.

**On the block — resolved 2026-09-09, and the history is the point.** This was
never waiting on an ADR, only on a lease over the four wire seams, and the
holder changed twice while it waited: `W-scm` held them for T102-C/R (the git
write contract) until `588e8e5`, then `W-t120` took them the same hour for
T120-C (the settings contract) at 2.25.0. That ordering was correct — 2.25.0
had to be spent before 2.26.0 — and it landed at `75d27b5`, after which the
settings session narrowed its lease to `crates/settings/**`.

Two things this cost, both worth remembering. **The number went unclaimed for
the whole wait**, named only in `docs/merge-queue.md`'s "next free values" line,
which is exactly the state rule 2 exists to prevent: nothing stopped another
lane taking 2.26.0. It was claimed at `b1075e4` the minute the seams freed, and
that claim is the first thing this ticket did rather than the last.

And **a lease is not narrowed by re-claiming it**: `agent-lease.py claim`
appends a row and prints only the scope you passed, so the settings session
believed it had released `apps/desktop/src-tauri/src/lib.rs` while a wider row
still held it. `release` then `claim` is the only narrowing. That is now written
into `AGENTS.md`; this ticket was its second victim.

## Required reading

- `docs/merge-queue.md` — rules 2, 4 and 5, and the claim table
- `crates/gritty-core/src/git/wire.rs` — the request/response idiom, and
  `GitStatusResponse`'s three-arm union
- `crates/gritty-core/src/protocol/export.rs` — how a root type is registered
- `crates/gritty-core/src/search/mod.rs:96-160` — the budget-then-operation
  hand-over, and the `spawn_blocking` + `CancellationToken` pattern
- `crates/workspace/src/listing.rs` — `ListDirectoryResponse`, the union this
  one mirrors, and `DEFAULT_LIST_BUDGET`
- `docs/tasks/T101-C-git-read-contract.md` — why a registered command name with
  no handler behind it is not a state worth landing
- `AGENTS.md` hard rules 1, 8, 10

## Allowed scope

- `crates/gritty-core/src/workspace/**` (the manager method and its wire types)
- `crates/gritty-core/src/protocol/mod.rs` — the version line only
- `crates/gritty-core/src/protocol/export.rs` — the new root types
- `packages/protocol/src/{commands,client,types}.ts`
- `packages/protocol/src/generated/**` — **generated**; commit what
  `cargo test -p gritty-core` writes, never hand-edit or hand-merge
- `apps/desktop/src-tauri/src/{commands,lib}.rs`
- `docs/merge-queue.md` — the claim, written *before* the version is spent

## Explicit non-goals

- **No `CoreEvent` variant.** A bounded top-N response does not need one, and
  avoiding it keeps this off `crates/gritty-core/src/event/mod.rs` and the
  exhaustive match in `session/tap.rs` — the two extra seams merge-queue.md
  warns a variant always costs, and where two branches merge *cleanly and
  wrongly*.
- **No `SCHEMA_VERSION`, no migration.** Nothing about a query is persisted.
- **No scoring here.** The algorithm is T130-R's; this ticket carries its result
  across the wire.
- **No UI.** T130-U's.
- **No second search domain.** This is `workspace.*` because it answers "which
  paths exist", the same question `workspace.list` answers one level at a time.
  `search.*` stays the grep surface.

## Required behavior

1. **The command.** `workspace.findFiles` ↔ `workspace_find_files`, one
   `request` argument, per ADR-009.
2. **The request.** `{ workspaceId, query: String, limit?: u32 }`. `limit`
   defaults to 50 and is capped at 200 — outside `1..=200` is `InvalidInput`,
   not a clamp, matching `pageSize`'s treatment in `ListDirectoryRequest`.
3. **The response is a union, and the union earns its place.**
   `{status:"ready", matches, truncated}` | `{status:"pending", operation}`.
   T080 measured a filename-scope walk of 10,429 files at **94 ms** against a
   100 ms `DEFAULT_LIST_BUDGET`: a repository modestly larger tips past it, so
   the pending arm is the normal case on a big tree, not a decoration. The
   operation completes with the same `matches` payload in `OperationCompleted.result`.
4. **A match carries its highlight.** `{ path, score, ranges: [[start, len], …] }`,
   `ranges` in **char** indices over the workspace-relative path, merged and
   ascending. The UI highlights without re-running the matcher.
5. **Cancellation is `operations.cancel`**, as `search.query`'s is. No
   `findFiles.cancel`: one cancellation surface, already built.
6. **The blocking walk goes through `spawn_blocking` with a `CancellationToken`**,
   the `CancelOnDrop` + `tokio::select!` shape `search/mod.rs` already proved —
   `spawn_blocking` cannot be aborted, so the token is how the walk learns.
7. **Claim before spending.** `PROTOCOL_VERSION` **2.26.0** goes in
   `docs/merge-queue.md`'s claim table, with its seams listed, *before* the
   constant moves. Two branches agreeing on a number merge cleanly and wrongly.
8. **Land with T130-R behind it.** T101-C recorded why: a registered command
   name with no handler is not a state worth landing, and nothing in the tree
   asserts against it.

## Failure behavior

- Unknown `workspaceId` → `NotFound`. No workspace open → the same.
- `limit` outside `1..=200`, or a query longer than `MAX_QUERY_CHARS` →
  `InvalidInput`, before any walk is started.
- Root vanished between the command and the walk → the walk's `io` error, not
  an empty `ready` list. "It failed" and "there were none" are different answers.
- Cancelled → the operation ends `CANCELLED` and no late `COMPLETED` is
  accepted; the state machine already guarantees this.
- A panic in the walk → `Internal`, via `guard`, never a hung operation.
- Nothing from the repository reaches a log line (rule 13): paths are workspace
  data. Log counts and durations, not names.

## Acceptance criteria

- **Corrected 2026-09-09.** This criterion read "`findFiles({query: 'epn'})`
  returns `editor-pane.tsx` first against `fixtures/`", and it cannot pass:
  there is **no `.tsx` file anywhere under `fixtures/`**, and `large-repo` is
  generated and git-ignored (`crates/workspace/src/fixtures.rs`), so it is not
  a corpus an assertion may name. T130-R's version of this was a `score_path`
  unit assertion over a *string*, which is why nobody hit it.

  The replacement keeps what the original was for — a ranking judged over a
  corpus the assertion's author did not write — using the committed fixture:
  `findFiles({query: 'lib'})` over `fixtures/small-repo` returns
  `crates/demo/src/lib.rs` **above** `src/lib/util.ts` and
  `src/lib/nested/deep/leaf.ts`. That is a genuine three-way contest, and the
  winner wins on the whole-match signal (`lib` is the entire basename and
  starts it) rather than on any per-character bonus — the exact pair T130-R had
  to add after `epn` returned `pixel.png`.
- A query on a tree large enough to exceed the budget returns `pending`, and the
  operation completes with the same payload.
- Cancelling that operation ends it `CANCELLED` promptly.
- `pnpm check` green, **including `check:generated`** — the generated directory
  is committed in the same change.
- `docs/merge-queue.md` records 2.26.0 as spent, with the seam list.

## Tests required

- Rust unit tests on the manager method: limit validation, unknown workspace,
  the ready/pending split driven by an injected budget rather than by timing.
- Cancellation: start against a large fixture, cancel, assert `CANCELLED` and
  that no completion overwrites it.
- `packages/protocol/src/workspace-client.test.ts` (or the existing workspace
  client test) via `createFakeTransport()`: asserts
  `COMMANDS['workspace.findFiles'] === 'workspace_find_files'` and the exact
  `{ request: … }` payload.
- `apps/desktop/src-tauri/tests/commands.rs`: the adapter over the mock runtime,
  including a malformed request landing as `InvalidInput`.

## Commands to run

```
cargo test -p gritty-core
cargo test -p gritty-desktop --test commands
pnpm check
```

## Completion report

### Summary

`workspace.findFiles` at `PROTOCOL_VERSION` 2.26.0: one additive command, three
generated roots, no `CoreEvent` variant, no `SCHEMA_VERSION`, no migration. The
walk itself is T130-R's `rank_workspace_files`, which already returned exactly
the shape the wire needed — `RankedPath { path, score, ranges: Vec<(u32,u32)> }`
and `DEFAULT_FIND_LIMIT`/`MAX_FIND_LIMIT` of 50 and 200 — so this ticket is
carrying, not computing. The claim went into `docs/merge-queue.md` first, as
requirement 7 asks and as the block's own history argues for.

### Files changed

| File | What |
|---|---|
| `crates/gritty-core/src/workspace/find.rs` | New. `FindFilesRequest`, `FoundFile`, `FoundFiles`, `FindFilesResponse`, `WorkspaceManager::find_files`, and 9 tests. |
| `crates/gritty-core/src/workspace/mod.rs` | `pub mod find;`, and a `#[cfg(test)] find_delay` beside `listing_delay`. |
| `crates/gritty-core/src/protocol/mod.rs` | 2.25.0 → 2.26.0. |
| `crates/gritty-core/src/protocol/export.rs` | The three roots. |
| `packages/protocol/src/generated/**` | Generated; committed as written. |
| `packages/protocol/src/{commands,client,types}.ts` | The command row, the namespace method, four `export type` lines. |
| `packages/protocol/src/workspace-client.test.ts` | Three tests over `createFakeTransport()`. |
| `apps/desktop/src-tauri/src/{commands,lib}.rs` | The adapter and its `invoke_handler` row. |
| `apps/desktop/src-tauri/tests/commands.rs` | The adapter over the mock runtime. |
| `docs/merge-queue.md` | 2.26.0 claimed (`b1075e4`), then struck as spent. |

### Tests actually executed

| Command | Result |
|---|---|
| `cargo test -p gritty-core workspace::find` | **9 passed** |
| `cargo test -p gritty-desktop --test commands workspace_find_files` | **1 passed** |
| `pnpm exec vitest run --project protocol packages/protocol/src/workspace-client.test.ts` | **7 passed** |
| `pnpm check` | biome 304 files clean · typecheck clean · **vitest 1067 passed / 103 files** · `cargo fmt --check` clean · `cargo clippy --workspace --all-targets -- -D warnings` clean · `cargo test --workspace` **1194 passed, 0 failed** across 42 suites. `check:generated` red only until this commit existed — it asserts the generated directory is *committed*, and it is, here. |

**Two things went wrong in getting that number, and both are worth recording.**

*The gate was reported green when it was red.* Run as
`pnpm check > log 2>&1; echo "EXIT=$?"` inside a backgrounded command, the
status that came back was the compound command's — the `echo`'s — so a
TypeScript failure surfaced as exit 0. A plausible zero is worse than a visibly
broken one; the fix is to write the status *into the log* and read it there.
The settings session hit the zsh cousin the same day: `${PIPESTATUS[0]}` is a
bash-ism, and zsh's array is lowercase `pipestatus`, so it printed `EXIT=`.

*The disk filled, and it did not look like a disk problem.* `pnpm check` died at
`ld: write() failed, errno=28` under a wall of linker arguments, reading exactly
like `error: could not compile gritty-core`. `df` showed **396 MiB free of
460 GB**, with this repo's `target/` at 88 GB (64 GB `deps`, 20 GB
`incremental`). Deleting `target/debug/incremental` freed 20 GB and cost one
recompile of the local crates. `ADR-024` (one `target/` per checkout, swept by
budget and age) proposes the fix and has been `Proposed` since 2026-09-06.

### Architectural decisions

**`FoundFiles` is one type, used twice.** The `ready` arm carries `matches` and
`truncated` inline and the operation completes with the same struct, so a
caller that must handle both arms writes one mapper rather than two. The
alternative — `ListDirectoryResponse`'s `Ready { page: DirectoryPage }` nesting
— would have read worse from TypeScript for a two-field payload.

**`FoundFile`, not `FileMatch`.** The row model in
`apps/desktop/src/features/quick-open/quick-open-model.ts` already has a
`FileMatch`, and a generated type of the same name would have collided in the
one file that has to import both. Naming it here was free; renaming it later
would not have been.

**Validation reuses `Scorer::new` rather than re-checking `MAX_QUERY_CHARS`.**
An over-long query must be refused before an operation exists, and `Scorer::new`
is the single place that rule and its message live. Calling it up front is the
same check, not a second copy of it — the shape `search.query` uses when it
compiles its pattern before creating anything.

**No third `CancelOnDrop`.** `mod.rs` and `files.rs` each define one; `find.rs`
uses `super::CancelOnDrop`, since a child module can see its parent's private
items. The `files.rs` duplicate is pre-existing and out of scope (rule 14).

### The acceptance criterion could not pass, and the replacement is over a real corpus

The original named `editor-pane.tsx` "against `fixtures/`". **There is no `.tsx`
file anywhere under `fixtures/`**, and `large-repo` is generated and git-ignored,
so it is not a corpus an assertion may name. See the corrected criterion above.

The replacement asserts `lib` over `fixtures/small-repo`. It was chosen by
*probing* the fixture rather than by predicting it — the ranking was printed for
seven candidate queries first, and `lib` picked because it is a genuine
three-way contest:

```
158  crates/demo/src/lib.rs        ← lib is the whole basename, and starts it
 81  src/lib/util.ts               ← lib is a directory component
 79  src/lib/nested/deep/leaf.ts
```

That is the same signal T130-R had to add after `epn` returned `pixel.png`, and
the same lesson: a table test proves the rule you thought of, not the ranking.

### Known limitations

- **`ranges` are char indices and the TypeScript consumer must respect that.**
  Slicing a path with `String.prototype.slice` is wrong past the BMP, because JS
  indexes UTF-16 code units. `Array.from(path)` is the fix, and the generated
  doc comment on `FoundFile.ranges` says so. Nothing enforces it yet — the first
  consumer is the not-yet-built quick-open component.
- **The pending arm has no consumer.** Nothing in the tree awaits
  `OperationCompleted` for a `workspace.findFiles` operation yet, so the arm is
  proven by a Rust test and not by a UI. On any real repository it is the normal
  answer, so the component must handle it from its first line.
- **Ignoring is still the eleven directory names** inherited from T130-R: no
  `.gitignore` support, so a large ignored-but-unnamed build directory appears
  in results. ADR-022 decision 2 already defers this.
- **The ranking has still never been judged on a screen.** Two fixtures and nine
  hand-run queries are not a person using it.

### Follow-up tasks

| Ticket | Why |
|---|---|
| The quick-open component and controller | The wire exists now; this is what was blocked on it. |
| T130-M (mount the bar in the app bar) | Unblocked separately: T310-U landed at `f51c83c`, so the App.tsx/app.css wall in its block note is gone. |
| `BOARD-STALE-REASON` | Filed this session: `block` sets a ticket's `reason` and `move` never clears it, so `show` prints an old block reason under a current status as if it were present tense. It misled this ticket and two others today. |
| ADR-024 needs a verdict | Not new — drafted 2026-09-06, still `Proposed`. The disk filled during this ticket (396 MiB free of 460 GB; `target/` at 88 GB) and stopped `pnpm check` at the linker with `errno=28`. That is the third time this failure mode has cost a session time, and the fix has been written and waiting three days. |
