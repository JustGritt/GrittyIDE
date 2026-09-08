# Task T080 — Search scans the live workspace tree; nothing is indexed

- **Phase:** 2
- **Status:** review
- **Depends on:** T070 (file watcher — ignore rules and invalidation this
  task consumes), T020 (workspace root, path policy, the cancellable-listing
  pattern this task's walker extends), T010 (operation model, event bus,
  protocol conventions)

## Objective

Add `search.query(text, options)` (`docs/Gritty IDE Project Plan.md:243`): a
command that scans the open workspace for matching file content and names and
streams results as found, cancellable like every long-running core operation
— "Filename/text search with cancellation and streamed results... Large
search can be cancelled immediately" (`:650`), not a full-text index. Nothing
searches today; the word appears only in a test fixture and a comment about
Monaco's own single-buffer find (`editor-pane.tsx:180`, in-buffer, out of
scope). The runtime-boundary table already carries the constraint this task
lives under — an indexer/search worker "must not: Block opening/editing
files" — hard rule 10, written before this task existed.

## Dependencies

- **T070**, for `pub fn is_ignored(path: &WorkspacePath) -> bool`
  (`crates/workspace/src/listing.rs`, next to `IGNORED_DIRECTORY_NAMES`, per
  T070's own draft) and its invalidation on filesystem change. It answers by
  name and nested path against that static list — it is not `.gitignore`
  parsing, and this task does not add any; a project's own ignore patterns
  stay unscanned until that is a separate, deliberate task.
- **T020**, for `WorkspaceRoot`/`WorkspacePath` and `list_directory`
  (`crates/workspace/src/listing.rs`) as the cancellable-walk pattern to
  extend, cursor-free here since results stream rather than page.
- **T010**, for `OperationRegistry`/`OperationContext`/`EventBus` and the
  error envelope; no new mechanism is added to any of them.

## Required reading

- `docs/Gritty IDE Project Plan.md:243,650,799` — the wire name, T080's row,
  the open "ripgrep process, Rust library, or hybrid" decision (§1).
- `docs/plans/execution-board.md:71-90` (Lane W, reserved protocol minor
  `2.12`) and `:215-231` (`gritty-search` at `crates/search/`; `commands.rs`
  is contended — write its adapter in one commit).
- `docs/plans/phase-0-spike.md:30` — row B3, quoted under Tests required.
- `crates/workspace/src/listing.rs` ("nothing is filtered",
  `CANCEL_CHECK_INTERVAL`, `IGNORED_DIRECTORY_NAMES`) and
  `crates/workspace/src/root.rs:1-9` (symlink-escape protection).
- `crates/gritty-core/src/workspace/mod.rs:1-10,210-234,366-403,448-459` —
  `workspace.list`'s budget-then-operation doc, `WorkspaceManager`, its
  `spawn_blocking`, and `require`, all reused here.
- `crates/gritty-core/src/operation/{mod.rs,registry.rs,progress.rs,demo.rs}`
  — the model this task reuses; `terminal/wire.rs:228-240`
  (`TerminalOutputPayload.truncated_bytes`) — the batch-event pattern to mirror.
- `docs/architecture/security.md:71-100,139-167` — bounded reads, binary is
  terminal, only ids/sizes/counts in logs, never content.
- `fixtures/README.md` — `small-repo`, `large-repo` (10,500 files),
  `weird-files` (binary, invalid UTF-8, a 100,000-character line).

## Allowed scope

> **Split by `PLAN-FANOUT` (2026-09-06).** This spec now covers two board
> tickets: **T080** is everything below *except* `features/search/**`, and
> **T080-U** (lane `Wu`) is that directory alone. Both build behind
> `PROTO-W3`, which owns the wire and the magnets, so neither bumps
> `PROTOCOL_VERSION` or hand-edits `src/generated`. The command adapter and
> the `CoreEvent` variant stay with T080.

- `crates/search/**` (new, `gritty-search`): the walk, ignore consultation,
  content/filename matching, batching.
- `crates/gritty-core/src/search/**` (new): wire types, `SearchManager`,
  operation wiring; `event/mod.rs` (one new `CoreEvent` variant); `lib.rs`
  (`pub mod search;`).
- `Cargo.toml` (the `gritty-search` member, one `[workspace.dependencies]`
  entry for the matching library — shared magnet, claim briefly),
  `crates/gritty-core/Cargo.toml`, `crates/search/Cargo.toml` (new).
- `apps/desktop/src-tauri/src/{commands.rs,core.rs,lib.rs}` (the
  `search.query` adapter, `SearchManager` next to `workspaces`,
  `core.rs:47-52,96`); `packages/protocol/src/{client.ts,commands.ts,types.ts}`
  + tests, `packages/protocol/src/generated/**` (regenerated, never hand-edited).
- `apps/desktop/src/features/search/**` (new); `docs/architecture/security.md`
  (a short subsection only if needed); this file.

## Explicit non-goals

- **No persistent index.** Hard rule 9 forbids speculative infrastructure
  without an ADR and a concrete requirement; a live walk, proven against
  `fixtures/large-repo`'s 10,500 files, is the requirement this task has. An
  index adds a durable structure and its own invalidation problem,
  duplicating what T070 owns more narrowly. Revisit only if a measured walk
  is too slow.
- **No search-and-replace.** `search.query` reads; a mutating variant changes
  files the user never opened and is a separate risk profile and task.
- **No spawned `ripgrep` process.** See Required behavior §1.
- **No fuzzy quick-open palette, no cross-workspace search.** T050 already
  deferred filename quick-open as its own follow-up
  (`docs/tasks/T050-markdown.md:197`); the command palette is separate,
  ranking-heavy UX, and `options.scope: "filenames"` here is plain
  substring/regex matching against the corpus already walked. Exactly one
  workspace is open at a time regardless.
- **No result replay after a renderer reload.** `Operation`'s snapshot
  (`crates/gritty-core/src/operation/mod.rs:64`) never carries a domain
  result stream, so a reload loses already-emitted batches the way it loses
  unpersisted terminal scrollback. That is a gap in the operation model in
  general; record it as a follow-up, not something built once for search.
- **No migration.** Search persists nothing; `SCHEMA_VERSION` is untouched.

## Required behavior

### The implementation decision

1. `docs/Gritty IDE Project Plan.md:799` leaves "Search/index implementation:
   ripgrep process, Rust library, or hybrid" open. This task recommends the
   **in-process Rust library** option — the `grep`/`grep-searcher`/
   `grep-matcher`/`grep-regex`/`ignore` family ripgrep's own engine is
   published as — over spawning `rg`. Cancellation: hard rule 10 gives the
   library free cancellation through `OperationContext`'s token, as
   `list_directory` already gets from a `cancelled: &dyn Fn() -> bool`
   closure; a spawned process instead needs its own process-tree kill, and
   the only such logic in the tree is ADR-015's PTY-shaped signal sequence
   built for a shell — no bare process-supervisor crate exists to reuse.
   Translation: hard rule 4 requires raw external output to pass through an
   adapter before it is domain state; a library's native structs skip
   building that adapter. Packaging: a spawned process needs a located,
   bundled or system `rg` binary on a platform still one accepted OS deep
   (ADR-002), an untested cross-platform claim hard rule 11 forbids.
   **Decided: `docs/decisions/ADR-022-search-engine.md`** takes the
   in-process option, on `grep-searcher` + `grep-regex` + `ignore` (not the
   `grep` facade). Two constraints it adds that this spec did not have:
   `ignore` is taken for its walker with **every gitignore source turned
   off**, because Gritty's "ignored" is `IGNORED_DIRECTORY_NAMES` and must
   stay one thing across the explorer, the watcher and search; and binary
   detection stays T030's test, not `grep-searcher`'s.

### The command, the operation and the walk

2. `search.query` fast-validates before anything is created: an empty or
   all-whitespace `query`, or an uncompilable `useRegex` pattern, is
   `InvalidInput` with no operation and no event — mirroring
   `StartDemoRequest::validate` (`operation/demo.rs`).
3. Unlike `workspace.list`'s budget-then-operation fallback
   (`ListDirectoryResponse::{Ready,Pending}`, `workspace/mod.rs:98-105`),
   `search.query` **always** creates an operation: a listing is bounded (one
   level, ≤5000 entries) and completes as a page, but a search is open-ended
   and its output is a stream, so there is no inline case to fall back to. A
   new `SearchManager` mirrors `WorkspaceManager`'s shape
   (`workspace/mod.rs:210-234`) — `Arc<EventBus>`, `Arc<OperationRegistry>`,
   the tokio `Handle` — constructed once in `core.rs` next to `workspaces`
   (`:96`), reusing `require`'s "workspace is not open" error (`:448-459`).
4. The request carries `workspaceId`, `query`, and `options`: `caseSensitive`,
   `wholeWord`, `useRegex` (`bool`, default `false`), `scope` (`"content" |
   "filenames" | "both"`, default `"both"`), `includeIgnored` (`bool`,
   default `false`, §8) — every field but `query` optional, camelCase,
   `#[ts(optional)]`, matching every other request here.
5. The walk runs on the blocking pool (`spawn_blocking`, as `workspace/mod.rs:394-403`
   does for a listing), calling T070's `is_ignored` per entry before
   descending or reading. A symlinked directory is never walked into (no
   cycle detection needed instead), following `root.rs`'s discipline of never
   reading through a link not proven to resolve inside the root; a symlinked
   file is read only when it does.

### Matches, batching and completion

6. A file is skipped as binary — and counted, not silently dropped — under
   the test T030 already applies: not valid UTF-8, or a NUL byte present
   (`security.md:71-79`). Its content is never decoded, so it never matches.
7. Matches stream through a new event, not `OperationProgress` (whose
   `message` is a human string, not match data): `SearchMatchesFound` carries
   the operation id, a bounded batch, and `truncatedMatches: u64`, mirroring
   `TerminalOutputPayload.truncated_bytes` (`terminal/wire.rs:228-240`) — a
   non-zero count means matches genuinely did not arrive, never a silent gap.
   Batches are throttled like `OperationProgress` (`operation/progress.rs`),
   per hard rule 8. Each snippet is bounded to a fixed length, `truncated:
   bool` set when the source line exceeded it (`long-lines/one-line-100k.txt`
   exercises this); a filename match is the same envelope, line/column absent.
8. `options.includeIgnored` defaults `false`; `true` scans directories T070
   flags ignored too, mirroring the explorer's stance — such directories are
   listed with `ignoredHint: true`, never hidden (`listing.rs:24-36`) — so
   search is never blind to what the tree already shows.
9. Cancellation reuses `operations.cancel`; there is no `search.cancel`. The
   walker checks at least as often as `list_directory`'s
   `CANCEL_CHECK_INTERVAL` (256), between files and at a bounded byte
   interval within one large file, so cancelling mid-scan of the 12 MiB
   `generated/large-text.txt` fixture does not wait for it to finish; the
   registry's own no-`OperationCompleted`-after-cancel guarantee is reused,
   not reimplemented.
10. Generic scan progress (files scanned, and a total once known) uses
    `OperationContext::report(current, total, message)`, sharing no event
    with match streaming. `OperationCompleted.result` carries only a summary
    — files scanned, skipped by reason, matches found, whether anything
    truncated — never the match list, which already streamed.
11. Protocol version bumps MINOR (additive): a new request/response pair and
    one new `CoreEvent` variant; no existing wire shape changes.

## Failure behavior

| Situation | Behavior |
|---|---|
| Empty/whitespace `query`, or `useRegex` fails to compile | `InvalidInput`, no operation created |
| No workspace open, or `workspaceId` mismatched | `NotFound`, `require`'s wording (`workspace/mod.rs:448-459`) |
| A file vanishes or turns unreadable mid-walk | Skipped and counted; not an error of the walk as a whole |
| A file is binary or not valid UTF-8 | Skipped, counted by reason, never decoded |
| A single line exceeds the bound | Match reported `truncated: true`; file not skipped |
| Cancellation requested | Stops within one check interval; no further batches; ends `CANCELLED`; delivered batches stay on screen |
| Workspace root disappears mid-walk | Ends `FAILED` with `Io` |
| Renderer reloads mid-search | `operations.list()` rehydrates status; matches already streamed do not replay |

## Acceptance criteria

- A content-mode query against `fixtures/small-repo` for a known token
  returns correct path/line/column; a filename-mode query for a partial name
  returns matching paths with no line or column. Cancelling a query against
  `fixtures/large-repo` stops within one check interval: no further
  `SearchMatchesFound`, no `OperationCompleted`, `seq` contiguous — the shape
  phase-0-spike.md's B3 re-run needs.
- Every file under `fixtures/weird-files/{binary,encoding}` is skipped and
  counted, never carried in an event; `long-lines/one-line-100k.txt` yields
  one truncated match, not a dropped file.
- Directories T070 flags ignored are excluded by default, included when
  `options.includeIgnored` is set.
- `Workspace`, `Document` and `DirectoryPage` wire shapes are unchanged
  (diffed generated types); `pnpm check` green.

## Tests required

- **Unit (Rust):** fast validation rejects an empty query and a bad regex
  without creating an operation; binary/invalid-UTF-8 files skipped and
  counted by reason; a symlinked directory never walked, a symlinked file
  read only inside the root; an oversized line truncated; the ignore
  predicate consulted per directory before descending (a fake T070
  interface); cancellation timing against a synthetic slow walker, mirroring
  `WorkspaceManager`'s `#[cfg(test)] listing_delay` hook
  (`workspace/mod.rs:219,391`).
- **Integration:** a full run against `fixtures/small-repo` (`content` and
  `filenames` scope) and `fixtures/large-repo` for volume and cancellation —
  the re-run `docs/plans/phase-0-spike.md:30` names: "progress observed →
  cancel → `CANCELLED` in < 1 s... `seq` contiguous... re-run with search
  (T080)".
- **Unit (TS):** the client method; a reducer folding `SearchMatchesFound`
  batches into a growing result list; a reload mid-search shows the
  operation `RUNNING` with no matches replayed.
- **Manual:** a query against the real open workspace from `pnpm dev`,
  cancelled mid-way, confirming it stops cleanly.

## Commands to run

```
cargo test -p gritty-search
cargo test -p gritty-core
cargo clippy --workspace --all-targets -- -D warnings
pnpm check
```

## Completion report

_Filled in by the session that built **T080** and **PROTO-W3** together
(2026-09-08). PROTO-W3 has no task file of its own; its report is the "The
wire" section below._

### Summary

`search.query` exists, is callable from the WebView, and streams. A new
`gritty-search` crate walks the open workspace once, consults T070's
`is_ignored` before descending, classifies each file with T030's binary test,
matches with ripgrep's own engine and hands matches to a sink. A new
`gritty_core::search` module turns that into an operation, batches the matches
into a bounded `SearchMatchesFound` event and completes with a summary of
counts. `apps/desktop/src-tauri` builds the manager and exposes the one
command; `@gritty/protocol` names it and types it.

Nothing is indexed, nothing is persisted, nothing is written, and no `rg`
process is spawned.

### The ADR this is built on, and the debt it carries

**ADR-022 is `Proposed`, not accepted.** This session built on its decision —
in-process, on `grep-searcher` + `grep-regex` + `ignore`, with every gitignore
source turned off and binary detection left as T030's — and did **not** decide
it. That is the same position `PROTO-W2-GIT` took on ADR-023/`gix` under the
2026-09-06 roadmap amendment, and it is recorded here as an inherited debt
rather than a settled question: an agent may draft an ADR and may not accept
one. If ADR-022 is rejected, this crate is what has to be rewritten.

The ADR's §1 requirement — "record the ADR before any crate code is written" —
was met by ADR-022 already existing when this work started.

### Versions and migrations spent

- `PROTOCOL_VERSION` **2.18.0** (`crates/gritty-core/src/protocol/mod.rs`),
  claimed for PROTO-W3 in `docs/merge-queue.md` before it was spent. MINOR, and
  additive only: one new request/response pair, one new `CoreEvent` variant, no
  existing wire shape changed.
- **No `SCHEMA_VERSION` and no migration.** A search is a read; nothing about
  one is persisted. `Intent::from_event` drops `SearchMatchesFound`
  deliberately, with the reason in the code: a persisted batch would let a
  restart present stale matches as current ones.
- `2.19.0` is untouched — it is reserved for `T310-B`.

### Dependencies added

Four crates, all `Unlicense OR MIT`, all in `[workspace.dependencies]`, all used
only by `crates/search`. None overlaps anything already in the tree: no crate
here matched text or walked a tree before this one.

| Crate | Version | One-line justification |
|---|---|---|
| `grep-searcher` | 0.1.17 | Drives a `Sink` **we own** over a slice we already read — which is what makes cancellation free and means there is no external output to adapt (ADR-022). |
| `grep-regex` | 0.1.14 | Compiles the query, literal or regex, with the case/word/CRLF knobs the request already has; linear-time, so a user-typed pattern cannot be made to backtrack. |
| `grep-matcher` | 0.1.9 | The `Matcher` trait both of the above speak. Already transitive through both; named directly because the trait must be in scope to locate a match *inside* a reported line, which is where the column comes from. |
| `ignore` | 0.4.33 | The directory walker, taken with `git_ignore`/`git_global`/`git_exclude`/`ignore`/`parents`/`hidden` all off, per ADR-022 decision 2. |

`grep` 0.4.1 (the facade) is deliberately **not** taken: it re-exports
`grep-cli` and `grep-printer`, which exist to build a terminal program.

### What bounds the stream (hard rule 8)

Match delivery is batched, throttled and queue-bounded in
`crates/gritty-core/src/search/batch.rs`, and every bound reports itself:

| Bound | Value | At the bound |
|---|---|---|
| matches per event | 200 | the rest waits for the next interval |
| events per search | 1 per 50 ms | the same `MIN_PROGRESS_INTERVAL` `OperationProgress` uses, so the renderer sees one rhythm |
| queued matches | 2 000 | further matches are dropped and counted into `truncatedMatches` |
| matches in total | 20 000 | the walk is asked to stop; the summary says `limitReached` |
| snippet | 400 characters | windowed around the match, `truncated: true` |
| bytes per file | 64 MiB (`MAX_DOCUMENT_BYTES`) | counted as `skipped.tooLarge`, never read |

A dropped match is never silent: `truncatedMatches` travels with the next batch
the way `TerminalOutputPayload.truncated_bytes` does, and the total is repeated
in `SearchCompletion`. The final drain empties the queue in batches of 200, so
even the flush is bounded (at most ten events).

### What bounds the work (hard rule 10)

- Validation is synchronous and creates nothing. The walk runs on the blocking
  pool; no command handler and no renderer thread ever waits on it.
- `cancelled` is polled **between every file** — strictly more often than
  `list_directory`'s every-256-entries, because reading and matching a file
  costs far more than a dirent — and between 1 MiB chunks *inside* one file, so
  a 12 MiB file does not have to finish before a cancel is observed. Chunks are
  cut at line boundaries, so line numbers stay exact across them.
- Cancellation is `operations.cancel` and nothing else. There is no
  `search.cancel`, and the registry's "no `OperationCompleted` after a cancel"
  guarantee is reused, not reimplemented.

### The wire (PROTO-W3)

- `PROTOCOL_VERSION` 2.18.0.
- `SearchQueryRequest`, `SearchOptions`, `SearchMatchesFoundPayload` and
  `SearchCompletion` are exported roots; `SearchMatch`, `SearchSummary`,
  `SkipCounts`, `MatchKind` and `SearchScope` follow transitively from
  `gritty-search`. Nine files in `packages/protocol/src/generated`, written by
  `cargo test -p gritty-core` and never by hand.
- `search.query` → `search_query` is in `COMMANDS`, on `CoreClient.search`, and
  registered as a Tauri handler.
- `SearchManager` is built in `Core` next to `git`.
- **`search.query` returns the `Operation` directly** — no `Ready`/`Pending`
  union. `workspace.list` and `git.status` have one because a listing is
  bounded and *can* answer inline within a budget; a search is open-ended and
  its output is a stream, so there is no inline arm to fall back to and
  inventing one would be a shape nobody could take.

### Files changed

**New**

- `crates/search/Cargo.toml`, `crates/search/src/{lib,error,model,query,walk}.rs`,
  `crates/search/tests/walk.rs`
- `crates/gritty-core/src/search/{mod,wire,batch}.rs`
- `packages/protocol/src/search-client.test.ts`
- `packages/protocol/src/generated/{SearchQueryRequest,SearchOptions,SearchMatch,SearchMatchesFoundPayload,SearchCompletion,SearchSummary,SkipCounts,SearchScope,MatchKind}.ts` (generated)

**Changed**

- `Cargo.toml` (the `gritty-search` member and four `[workspace.dependencies]`
  entries), `crates/gritty-core/Cargo.toml`
- `crates/gritty-core/src/lib.rs` (`pub mod search;` and the module map),
  `crates/gritty-core/src/event/mod.rs` (one `CoreEvent` variant, its `name()`
  arm, and `operation_id()` — a batch names the operation it streams from),
  `crates/gritty-core/src/session/tap.rs` (the variant is deliberately not
  persisted), `crates/gritty-core/src/protocol/mod.rs` (2.18.0),
  `crates/gritty-core/src/protocol/export.rs` (four new roots)
- `apps/desktop/src-tauri/src/{commands,core,lib}.rs`,
  `apps/desktop/src-tauri/tests/commands.rs`
- `packages/protocol/src/{commands,types,client}.ts`,
  `packages/protocol/src/generated/{CoreEvent,version}.ts` (generated)
- `docs/architecture/security.md` (a "Searching the workspace" section),
  `docs/merge-queue.md`, this file

Not touched, deliberately: `apps/desktop/src/features/search/**` is `T080-U`.

### Tests executed, and what they actually reported

Every command below was run in this session on macOS (`darwin 25.6.0`,
`cargo 1.98.0`, `node v24.16.0`).

| Command | Result |
|---|---|
| `cargo test -p gritty-search` | **ok** — 13 unit + 22 integration, 0 failed |
| `cargo test -p gritty-core` | **ok** — 463 unit + 3 `crash_recovery`, 0 failed (18 of those are new: 6 batcher, 3 wire, 9 manager) |
| `cargo test -p gritty-desktop --test commands` | **ok** — 48 passed, 0 failed (4 new) |
| `cargo test --workspace` | **ok** — exit 0, 20 test binaries, 0 failed |
| `cargo clippy --workspace --all-targets -- -D warnings` | **exit 0** |
| `cargo fmt --all --check` | **exit 0** |
| `npx biome check .` | **exit 0** — 273 files |
| `npx tsc --noEmit -p tsconfig.json` + `pnpm -r run typecheck` | **exit 0** both |
| `npx vitest run` | **exit 0** — 86 files, 805 tests (10 new) |
| `pnpm check:generated` | **exit 0** after the generated files were committed |

`pnpm check` was **not** run as one command: two other sessions are live in this
working tree (`W-t310a` in `crates/agent-claude/**`, `W-t100u` in
`apps/desktop/src/features/git/**`), and `check:generated` cannot pass before
the generated files are committed. It was decomposed into the rows above, each
run with a real exit status, and every row is green.

What the tests cover, by the spec's list:

- **the walk** — small-repo content and filename scope; large-repo walked whole
  (10,429 files); `progress` observed while it ran.
- **ignore consultation** — pruned by default, scanned under `includeIgnored`,
  and an injected fake predicate proves the walk asks about `drop` and then
  never descends into `drop/deep`.
- **matching** — path/line/column exact (`util.ts:1:17`), case sensitivity,
  whole word, regex, `^`/`$` anchoring per line, filename matches with no
  position, UTF-16 columns across an astral emoji.
- **batching** — six unit tests on the batcher (bounded batch, throttle, drop
  and count, cap stops the walk, drain splits, an empty batch still reports a
  drop), plus the manager and IPC tests asserting `operationId` and
  `truncatedMatches` on every batch.
- **cancellation mid-stream** — four ways: between files (large-repo, polls
  bounded), *inside* one 4 MiB file (matches delivered but far fewer than the
  file holds), through `SearchManager` (ends `CANCELLED`, no
  `OperationCompleted`, no batch after the cancellation, `seq` contiguous), and
  through the real IPC with `operations.cancel` (same four assertions).
- **invalid input** — empty, whitespace, over-long, line-spanning and
  uncompilable queries, each `InvalidInput` on field `query`, with
  `registry.list()` empty and no event emitted.
- **filesystem errors** — binary and invalid-UTF-8 skipped and counted and
  never carried in an event; a mode-`000` file counted as `unreadable` without
  failing the walk; a file above the byte bound counted as `tooLarge`; a
  symlinked directory never walked into; a symlink out of the root counted as
  `outsideRoot` and never read; a root that disappeared failing with `Io`
  rather than reporting "no matches".
- **volume and chunking** — the 12 MiB `generated/large-text.txt` scanned with
  an exact line number (90,001) across nine chunks; a synthetic three-chunk file
  keeping its absolute line number in the third.

**Measured, since the "no index" non-goal rests on it** (debug build, macOS,
`fixtures/large-repo`, printed by the tests): **10,429 files walked in 94 ms**
for filename scope, and **content-searched in 759 ms**. A release build is
faster still. Nothing here argues for an index.

**The manual check was not performed.** The spec asks for "a query against the
real open workspace from `pnpm dev`, cancelled mid-way". This session has no
display and no UI to type into — `features/search/**` is `T080-U` — so there is
nothing to click. Rule 12 forbids claiming it; it belongs to `T080-U`'s manual
pass or to the standing manual-pass ticket. What *is* proven end to end is the
IPC: `apps/desktop/src-tauri/tests/commands.rs` drives the real adapters on
Tauri's mock runtime, including the cancellation.

### Architectural decisions taken here

1. **`search.query` always returns an `Operation`** (no `Ready`/`Pending`
   union), for the reason in "The wire" above.
2. **Batching lives in `gritty-core`, not in the walk.** `gritty-search`
   exposes a `SearchSink` trait and knows nothing about events; the throttle,
   the queue bound and the match cap are all on the core side, where the event
   bus is. That is also what lets the walk be tested with a plain collector.
3. **The batcher does not reuse `operation::progress::Throttle`.** That type
   measures with `tokio::time::Instant` — the runtime's clock, which a paused
   test clock freezes. The batcher runs on the blocking pool, so it measures
   with `std::time::Instant` and takes `now` as an argument, which is what makes
   the bounds table testable without sleeping. It uses the *same*
   `MIN_PROGRESS_INTERVAL` constant, so the two stay one number.
4. **Options are a nested `options` object**, not flattened onto the request.
   The spec refers to `options.includeIgnored` and `options.scope` by name, and
   the plan's signature is `search.query(text, options)`.
5. **Columns and snippet offsets are UTF-16 code units, one-based.** That is
   what Monaco's `IPosition.column` means and what `TextPosition` (T060)
   already records, so a match can be navigated to without a second conversion.
6. **Filename matching is against the whole workspace-relative path**, not the
   basename, so `src/lib` finds what a user means by it. The snippet is the
   path; line and column are absent.
7. **The searcher never memory-maps.** It is handed a slice already read, which
   is what `detect_content` needed anyway — and it means no file truncated
   underneath the process can fault it, with no `unsafe` in the crate.
8. **A root that vanished fails rather than reporting nothing.** The walk
   checks `WorkspaceRoot::is_present` before it starts, because "the search
   failed" and "there were no matches" are different answers and only one of
   them is true.

### Known limitations

1. **One match per line.** The search is line-oriented (ripgrep's default with
   `multi_line` off), so a line containing the query three times is reported
   once, with `matchStart`/`matchLength` pointing at the first occurrence. Every
   editor's find-in-files list works this way; highlighting the rest is the
   panel's job if it wants it.
2. **No `.gitignore`.** ADR-022 decision 2: `ignore` is taken for its walker
   with every gitignore source off. A generated directory whose name is not one
   of T070's eleven — `.next`, `vendor`, `coverage` — *is* searched, and a
   user's own ignore rules are not honoured. This is the explorer's behaviour
   today, so search is consistent with the product rather than worse than it,
   which is the whole point of that decision; users will still meet it.
3. **No result replay after a renderer reload.** A reload gets the operation
   back `RUNNING` and none of the matches it already showed. Asserted rather
   than assumed, in `search-client.test.ts`. See the follow-up below.
4. **The overall cap is 20 000 matches**, after which the walk stops with
   `limitReached: true` and the rest of the tree is unsearched. There is no way
   to ask for more.
5. **A root that disappears *truly* mid-walk** — after the pre-walk
   `is_present` check — surfaces as counted `unreadable` entries and a
   *completed* summary with `filesScanned: 0`, not `FAILED` with `Io` as the
   spec's failure table says. The pre-walk check covers the realistic window
   (between `require` and the blocking job starting); closing the rest would
   mean re-`stat`ing the root per directory.
6. **`options.includeIgnored: true` does not consult the ignore predicate at
   all** — the walker's filter is simply not installed — so
   `skipped.ignoredDirectories` is 0 rather than a count of what was included.
7. **Windows is unverified.** The symlink handling and the walk compile there
   and nothing more; rule 11 forbids claiming otherwise. The symlink tests are
   `#[cfg(unix)]`.
8. **No UI.** `T080-U` owns the panel, and it has not been built, so no user can
   run a search yet.

### Follow-up tasks (recorded, not built — rule 14)

- **`T080-U`** — the search panel. Already a board ticket; this is the wire it
  consumes.
- **Result replay after a renderer reload.** `Operation`'s snapshot never
  carries a domain result stream, so a reload loses already-emitted batches the
  way it loses unpersisted terminal scrollback. That is a gap in the operation
  model *in general*, not something to build once for search, and it wants its
  own ticket against the operation model rather than a search-shaped patch.
  T080's non-goal list asked this session to say whether it is worth one:
  **it is**, but only once a second consumer wants it — terminal scrollback and
  search results would then share one answer instead of two.
- **Real gitignore support**, across the explorer, the watcher and search at
  once, answering what `ignoredHint` means afterwards. ADR-022 decision 3 says
  this is a separate decision; `ignore` is now in the tree to implement it.
- **Search and replace.** An explicit non-goal here: it writes files the user
  never opened and is a different risk profile.
- **Accept or reject ADR-022.** It is `Proposed` and this crate is built on it.
- **Several matches per line**, if the panel wants to highlight all of them.
- **A `maxMatches` request option**, if 20 000 turns out to be the wrong number
  for anyone.

### Remaining risks

- **ADR-022 is undecided.** If it is rejected in favour of spawning `rg`,
  `crates/search` is rewritten. Low probability — the ADR argues three hard
  rules against the alternative — but it is the one open question.
- **Pre-1.0 dependencies.** `grep-searcher` is 0.1.17 and these crates are
  versioned for ripgrep's benefit. A minor bump may break compilation; it
  cannot break behaviour silently, because the sink and the tests are ours. The
  CRLF/line-terminator mismatch this session hit is exactly that failure mode:
  a hard error at the seam, not wrong results.
- **The bounds are guesses informed by one machine.** 200/50 ms/2000/20 000 are
  defensible and reported, but nobody has watched a real user search a real
  monorepo through a real panel yet. `truncatedMatches` is the instrument that
  will say if they are wrong.
- **Untrusted content is handled defensively but the surface is real.** A
  100,000-character line, invalid UTF-8, a byte range landing off a character
  boundary and a hostile regex are each tested or bounded; the general risk of
  running a regex engine over an attacker's file remains, and is why the
  pattern size, DFA size and nesting limits exist.
