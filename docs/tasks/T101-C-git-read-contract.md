# Task T101-C — The read contract: `git.log` and `git.show`

- **Phase:** 2
- **Status:** in-progress
- **Depends on:** T100 (the git core), `PROTO-W2-GIT` (the wire it extends)

## Objective

Add the two remaining *reads* a source-control surface needs, as contract only:
`git.log` — a bounded, paged walk of commit history — and `git.show` — the
content of one file at one revision. Both follow `git.status`'s three-armed
shape exactly. No UI, and nothing writes.

## Motivation

`T100-U` shipped the Changes tab and recorded two limitations this unblocks. The
first is that a diff renders in a ~300px sidebar column, and its own follow-up
says the editor area is the right home. Monaco's diff editor needs an
*original* side, and nothing on the wire returns file content at a revision:
`git.diff` returns structured hunks, and hunks with bounded context cannot
reconstruct a whole file. `git.show` is that missing read.

The second is that there is no history at all. `git.log` is the other half of
what "source control" means as a phrase, and it is the surface a commit is
reviewed from.

Both are reads, so ADR-023 already covers them and neither waits on ADR-029.

## Dependencies

`gritty-git` for the repository handle and `PageOptions`/`page.rs` for the
paging, `gritty-core::git` for the `WorkspaceId` lookup, the budget hand-over
and the error envelope. Nothing about the operation model, the event bus or the
error envelope is extended — `git.status` already established every piece.

## Required reading

- `AGENTS.md` hard rules 2, 5, 9, 10, 13
- `crates/gritty-core/src/git/wire.rs` — why all three arms are values
- `crates/gritty-core/src/git/mod.rs` — the budget hand-over
- `crates/workspace/src/detect.rs` — the binary / too-large arms to mirror
- `docs/decisions/ADR-023-git-library.md`
- `docs/merge-queue.md` — rule 2, and the 2.21.0 claim already written there

## Allowed scope

- `crates/gritty-core/src/git/**`
- `crates/gritty-core/src/protocol/**` (the version, the exports)
- `packages/protocol/src/**`
- `apps/desktop/src-tauri/**`
- `docs/merge-queue.md`

## Explicit non-goals

- **No write surface.** Nothing stages, commits, discards or checks out. That is
  `T102-*`, behind ADR-029.
- **No implementation.** The `gix` walk and blob read are `T101-R`; this ticket
  is the contract, so the two can be reviewed apart the way `T300-C` and
  `T300-CORE` were.
- No UI — that is `T101-U`.
- No network. No `fetch`, and `git.log` walks local history only.
- No change to the wire types T100 froze.

## Required behavior

1. `PROTOCOL_VERSION` 2.21.0, claimed in `docs/merge-queue.md` **before** it is
   spent (already written there).
2. `GitLogRequest`/`GitLogResponse` and `GitShowRequest`/`GitShowResponse`, each
   with the same three arms as `GitStatusResponse`: `ready`, `notARepository`,
   `pending`. `notARepository` stays a **value**, not an error.
3. A `CommitSummary` domain type in `gritty-git`: id, short id, author and
   committer name/email/time, subject, body, parent ids. Untrusted content —
   decoded lossily, passed through, never interpreted, never logged.
4. `git.log` pages like `git.status`: `cursor`, `pageSize` in `1..=5000`,
   `truncated`, and an opaque continuation token.
5. `git.show` mirrors `crates/workspace/src/detect.rs` for content that is
   binary or over the size cap, rather than inventing new arms.
6. Both commands are in `COMMANDS`, on the typed client, and registered as
   Tauri handlers; the root types are exported and their generated types
   committed.
7. `client.ts`'s `git` doc block — which currently states that nothing here
   writes and that a write surface is "a later task and a deliberate decision" —
   is updated to describe four reads, and still says the write surface is not
   here.

## Failure behavior

- A workspace that is not open → `NotFound`, before anything is read.
- A `path` that leaves the workspace root → `InvalidInput`, while parsing.
- A `pageSize` outside `1..=5000` → `InvalidInput`.
- A revision that does not resolve → `NotFound`, naming the revision.
- A path that does not exist at that revision → `NotFound`.
- A read that outlives `DEFAULT_GIT_BUDGET` becomes a cancellable `Operation`;
  no new event family carries it.
- A repository with no commits yet (unborn branch) is **not** a failure: the log
  is an empty page.
- A workspace that is not a repository is **not** a failure.

## Acceptance criteria

- `core.git.log` and `core.git.show` reach a real repository from the WebView's
  own IPC.
- `pnpm check` green, generated types committed and fresh.

## Tests required

- Protocol client: the names, the request shapes, all three arms, the error
  envelope.
- Integration through the real IPC, against a real repository built at test
  time with `gritty_git::fixtures`.

## Commands to run

```
cargo test -p gritty-core --lib protocol
cargo test -p gritty-desktop --test commands
pnpm exec vitest run packages/protocol/src/git-client.test.ts
pnpm check
```

## Completion report

_Filled on completion._
