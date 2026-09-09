# Task T130-C — The quick-open contract: `workspace.findFiles`

- **Phase:** 2
- **Status:** blocked (on a lease, not a decision — see Motivation)
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

**On the block:** this is not waiting on an ADR. `PROTOCOL_VERSION` 2.26.0 is
free, but `docs/merge-queue.md`, `crates/gritty-core/src/protocol/**`,
`packages/protocol/src/**`, `apps/desktop/src-tauri/**` and `Cargo.toml` are all
held by `W-scm` for T102-C/R (claimed 2026-09-09T08:33Z). The number cannot even
be *claimed* until that releases. Do not route around the lease.

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

- `core.workspace.findFiles({workspaceId, query: 'epn'})` from the typed client
  returns `editor-pane.tsx` first against `fixtures/`.
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

*(filled on completion)*
