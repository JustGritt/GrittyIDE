# Task T102-C — The write contract: `git.stage`, `git.unstage`, `git.discard`, `git.commit`

- **Phase:** 2
- **Status:** todo (ADR-029 accepted 2026-09-09)
- **Depends on:** ADR-029, T100, T101-C

## Objective

The contract for the four writes that turn the git panel from an inspector into
a working source-control view. Contract only: the request and response shapes,
the generated types, the client methods and the command names.

## Motivation

`git-reducer.ts:22` states the gap in as many words — *"There is no stage,
commit, checkout or fetch on the wire to call."* The panel already groups
entries into **Staged** and **Changes** bands because that is what the
repository reports; it simply cannot move a file between them.

## Dependencies

**ADR-029 was accepted on 2026-09-09**, so this is unblocked. It decides that
writes shell out to `git` while reads stay on `gix`; the whole shape of `T102-R`
follows from it, and `PROTOCOL_VERSION` 2.22.0 is a live claim rather than a
reservation.

## Required reading

- `docs/decisions/ADR-029-git-write-backend.md`
- `AGENTS.md` hard rules 5, 7, 9, 10, 13
- `crates/gritty-core/src/workspace/files.rs:9-12`, `:136-159` — the rule that
  every mutation names what to re-read, and `FileOperationResult`
- `crates/gritty-core/src/workspace/files.rs:176-196`, `:521-535` — the
  `DocumentTreeGuard` dirty-buffer refusal and its `force` escape
- `crates/gritty-core/src/git/wire.rs` — the three arms to mirror

## Allowed scope

- `crates/gritty-core/src/git/**`
- `crates/gritty-core/src/protocol/**`
- `packages/protocol/src/**`
- `apps/desktop/src-tauri/**`
- `docs/merge-queue.md`

## Explicit non-goals

- **No implementation** — that is `T102-R`. This ticket may not land a
  registered command with no handler behind it; land it with `T102-R` or not at
  all (the note `T101-C` records).
- No UI (`T102-U`).
- **No remote.** No fetch, pull, push, clone or credentials. A different risk
  surface and a further ADR.
- No branch, checkout or worktree — T400's lane, and hard rule 7.
- No merge-conflict resolution.

## Required behavior

1. `PROTOCOL_VERSION` 2.22.0, already claimed in `docs/merge-queue.md`.
2. Four requests, each carrying `workspaceId` and explicit paths:
   `GitStageRequest { workspaceId, paths[] }`, `GitUnstageRequest` the same,
   `GitDiscardRequest { workspaceId, paths[], force? }`, and
   `GitCommitRequest { workspaceId, message, amend?, signoff?, noVerify? }`.
   **No "all" flag on any of them** — the pathspec is always explicit, because
   Gritty ships a terminal and a wildcard write could sweep up what another
   process changed a moment ago (ADR-029 decision 3).
3. **The response says what changed**, modelled on `FileOperationResult`: the
   paths whose `FileStatus` moved, and the repository facts that moved with
   them, so the panel converges on the response rather than re-polling
   `git.status`. Nothing emits a new `CoreEvent`; the response is the signal.
4. `git.commit` carries the three arms `git.status` has, because a `pre-commit`
   hook can outlive any budget (hard rule 10, ADR-029 decision 6). The other
   three are fast and may answer directly, with the same budget escape.
5. A single `OperationKind` — `git.write` — the way `files.transfer` covers both
   `files.move` and `files.delete`, rather than four kinds the UI must match on.
6. `client.ts`'s `git` doc block stops saying nothing here writes.

## Failure behavior

- A workspace that is not open → `NotFound`.
- A path that leaves the workspace root → `InvalidInput`, while parsing.
- An empty `paths` → `InvalidInput` naming the field.
- An empty or whitespace-only commit message → `InvalidInput`. Never a commit
  with no message.
- **A dirty open buffer on a path being discarded → `Conflict`** with
  `details.dirtyDocuments`, and a `force` escape — hard rule 7, and exactly what
  `files.delete` already does through `DocumentTreeGuard::dirty_under`.
- `git` missing or too old → the write half reports itself unavailable with the
  reason, and no command silently no-ops (ADR-029 decision 5).
- `index.lock` held by another process → a named, retryable failure. Never a
  silent retry loop.
- A hook that refuses the commit → the hook's exit status and its output reach
  the caller as a refusal, not as a success with nothing committed.

## Acceptance criteria

- The four commands reach a real repository from the WebView's own IPC.
- `pnpm check` green, generated types committed and fresh.

## Tests required

- Protocol client: the four names, the request shapes, the `Conflict`-then-`force`
  replay, and the error envelope.
- Integration through the real IPC against a real repository.

## Commands to run

```
cargo test -p gritty-core --lib protocol
cargo test -p gritty-desktop --test commands
pnpm exec vitest run packages/protocol/src/git-client.test.ts
pnpm check
```

## Completion report

_Blocked; not started._
