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

**Summary.** The four commands on the wire at `PROTOCOL_VERSION` 2.22.0, and
`crates/gritty-core/src/git/write.rs` — the module that composes `gritty-git`
and `gritty-git-write`, which ADR-029 decision 2 keeps from knowing about each
other. Landed together with T102-R, as the ticket required.

**Files changed.** `crates/gritty-core/src/git/{wire,mod,write}.rs`,
`crates/gritty-core/src/protocol/{mod,export}.rs`, `crates/gritty-core/Cargo.toml`,
`apps/desktop/src-tauri/src/{commands,lib,core}.rs`,
`packages/protocol/src/{commands,client,types}.ts`, the generated directory
(8 new types, 2 changed), `packages/protocol/src/git-write-client.test.ts`,
`Cargo.lock`, `docs/merge-queue.md`.

**Tests actually executed.** `cargo test -p gritty-core` — 499 passed.
`cargo test --workspace` green. `cargo clippy --workspace --all-targets -D
warnings` clean. `pnpm test` — 97 files, 958 tests. `pnpm typecheck` clean.
`pnpm check:generated` exit 0 after committing. 8 new protocol-client tests
cover the four names, the request shapes, the `Conflict`-then-`force` replay,
the `pending` arm and a hook's stderr reaching the caller.

**Architectural decisions.**

- *One `GitWriteResponse` for all four*, and *one `OperationKind`, `git.write`*
  (requirement 5) — a UI matching on the kind should not need to know the verb.
- *The response carries the re-read `RepositoryStatus`.* That is the concrete
  form of requirement 3: the panel converges on the response instead of issuing
  a `git.status` behind every click.
- *A discard splits its paths by tracking.* `git stash` will not take untracked
  files without `-u`, and `-u` would stash every untracked file rather than the
  named ones — the wholesale write decision 3 forbids. Tracked work is stashed;
  untracked goes to the Trash.
- *Whether a stash happened is read from `refs/stash` before and after*, not
  from git's "No local changes to save". A ref that moved is a fact; a message
  is a translation.

**Two additions the ticket did not specify.**

1. **`GitStatusResponse::Ready` gains `write: GitWriteAvailability`.** T102-U
   requires the affordances be disabled *with the reason*, and nothing carried
   the probe verdict — the panel would have learned on the first failed click.
   It is on the wire type, not on `RepositoryStatus`, because that type belongs
   to `gritty-git`.
2. **`GitService` takes the `DocumentTreeGuard`**, installed in `core.rs`
   exactly as `FileService`'s is, so a discard can refuse over a dirty buffer.

**Known limitations.**

- `git.commit` is the first operation in the tree to own a child process. It
  composes `registry.start` with the adapter's kill ladder; the composition is
  tested at the adapter level, and the operation-level cancellation path is not
  yet exercised end to end through the IPC.
- The integration test through the real IPC against a real repository
  (acceptance criterion 1) is **not** written. The four commands are registered
  and the protocol client is covered, but `apps/desktop/src-tauri/tests/commands.rs`
  has no git-write case yet. That is the honest gap in this ticket.

**Follow-up tasks.** The IPC integration test above. T102-U consumes this.

**Remaining risks.** `commands.rs`'s
`root_removed_while_open_fails_with_not_found_and_closes_the_workspace` failed
once under concurrent `cargo` load from another session in this shared
checkout, then passed 55/55 across three runs. Its assertion is on exact event
ordering, so it is timing-sensitive; recorded rather than hidden.
