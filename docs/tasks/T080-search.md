# Task T080 — Search scans the live workspace tree; nothing is indexed

- **Phase:** 2
- **Status:** todo
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
   (ADR-002), an untested cross-platform claim hard rule 11 forbids. This
   needs its own ADR before any crate code is written — take the next free
   number at PR time, not now, the way `docs/merge-queue.md` claims a
   migration number then rather than reserving one early, since sibling
   lanes writing specs concurrently could race for the same one.

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

_(filled in by the implementing session — it must record the ADR §1 requires
before any crate code is written, including the number claimed at PR time and
why the library option won or lost; the one-line dependency justification for
every crate entering `[workspace.dependencies]`; the exact `PROTOCOL_VERSION`
spent; and whether the "no result replay" non-goal is worth its own
follow-up task)_
