# Task T101-C — The read contract: `git.log` and `git.show`

- **Phase:** 2
- **Status:** done
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
- ~~**No implementation.** The `gix` walk and blob read are `T101-R`.~~
  **Departed from deliberately** — see the completion report: a registered
  command name with no handler behind it is a client method that rejects at
  runtime, and nothing in the tree asserts against that state. The two tickets
  landed in one commit and are reviewed together.
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

### Summary

`git.log` and `git.show` exist end to end, at `PROTOCOL_VERSION` 2.21.0. The
Changes panel gained a **History** view on the first of them; the second has no
consumer yet and is the original side the diff tab (`T101-D`) will need.

**Built together with `T101-R` rather than landing alone**, and that is a
deliberate departure from the ticket split. `commands.ts` naming `git_log` with
no registered handler behind it is a client method that rejects at runtime, and
nothing in the tree asserts against that state — there is no test cross-checking
`COMMANDS` against `generate_handler!`. So the contract landed with its
implementation, in one commit, and the two tickets are reviewed together.

### Files changed

| File | Why |
|---|---|
| `docs/merge-queue.md` | claim 2.21.0 and 2.22.0 and ADR-029 **before** spending any |
| `docs/decisions/ADR-029-git-write-backend.md` | new; the write-backend decision, `Proposed` |
| `crates/git/src/model.rs` | `CommitSummary`, `CommitSignature`, `CommitPage`, `BlobContent`, `NoTextReason`, `MAX_SHOW_BYTES` |
| `crates/git/src/log.rs` | new; the paged walk and the path filter |
| `crates/git/src/show.rs` | new; the blob read and the decode |
| `crates/git/src/error.rs` | `NotFoundAtRevision` |
| `crates/git/src/fixtures.rs` | `history()` — four commits, one binary blob |
| `crates/gritty-core/src/git/wire.rs` | two requests, two three-armed responses |
| `crates/gritty-core/src/git/mod.rs` | `log`/`show` on `GitService`, `LOG_KIND`/`SHOW_KIND`, `DEFAULT_LOG_PAGE_SIZE` |
| `crates/gritty-core/src/protocol/{mod,export}.rs` | 2.21.0, four root exports |
| `packages/protocol/src/generated/*` | 9 generated types (never hand-edited) |
| `packages/protocol/src/{commands,client,types}.ts` | two names, two methods, nine re-exports |
| `packages/protocol/src/git-client.test.ts` | +3 tests |
| `apps/desktop/src-tauri/src/{commands,lib}.rs` | two adapters, two handler registrations |
| `apps/desktop/src-tauri/tests/commands.rs` | +3 integration tests |

### Tests actually executed

- `cargo test -p gritty-git --lib` — 16 new (8 `log`, 8 `show`)
- `cargo test -p gritty-desktop --test commands git_` — 6 passed, 3 new
- `pnpm exec vitest run packages/protocol` — 103 passed across 17 files
- `cargo test --workspace` — **exit 0**, 977 passed, 0 failed
- `cargo clippy --workspace --all-targets -- -D warnings` — clean
- `cargo fmt --all --check` — clean; `biome check .` — clean, 289 files
- `pnpm exec vitest run` (all projects) — 945 passed across 95 files
- **`pnpm check` — exit 0**, the whole gate including `check:generated`. Worth
  noting because it is the stage that usually cannot pass in this tree: another
  session's uncommitted work is present, and this time it neither breaks the
  typecheck nor leaves the generated directory dirty.

### Architectural decisions

**A `CommitPage` has no `total`.** `RepositoryStatus` and `DiffSummary` both
carry one because the walk that produces a page produces the whole list.
Counting every commit in a repository means walking its entire history to render
a number beside twenty rows, and reporting a count that was never computed would
be worse than reporting none. The page says only whether more follow.

**The log cursor is a commit id, not an offset.** Nothing is cached, so a page
boundary has to survive being recomputed; an offset silently skips a row when a
commit lands between two pages. A cursor naming no commit in this history yields
an empty page rather than restarting at the top — a boundary that finds nothing
rather than a page that lies.

**`git.show` reuses `gritty_workspace::decode_text`.** A blob and a file on disk
must not disagree about what "binary" means, or a diff would show text on one
side and refuse it on the other for the same file. The size cap is checked
*before* the decode, so an oversized blob is never copied into a `String` on its
way to being refused. `MAX_SHOW_BYTES` is 16 MiB against a document's 64 MiB,
because two blobs cross the boundary at once for one diff.

**No new dependency.** Both commands are `gix` calls inside the crate that
already has it.

### Known limitations

- **The path filter does not follow renames.** A file's history stops at the
  commit that renamed it. Rename detection over a whole history is far more
  expensive than the per-diff detection `diff.rs` does, and a partial answer
  presented as complete is what the failure table forbids. Documented in the
  module header; a caller who wants the earlier history asks again with the old
  path.
- **The `pending` arm is not proven end to end**, the same gap `PROTO-W2-GIT`
  recorded: provoking it needs the `#[cfg(test)]` `read_delay` hook inside
  `gritty-core`, unreachable from the desktop integration test. The hand-over
  itself is `git.status`'s and is tested there.
- `git.show` has no consumer yet — `T101-D` is the diff tab.
- **Not seen on a screen.** This session has no screen access.

### Follow-up tasks

| Ticket | Why |
|---|---|
| `T101-D` | the Monaco diff tab; blocked on `T310-U`'s uncommitted work in its three mount points |
| `T102-C/R/U` | the write surface; blocked on ADR-029 |
| `EXPLORER-FORCE-REPLAY` | `deleteAnyway` is dead code and the notice beside it replays the wrong action |
| `CONFIRM-DIALOG` | no shared confirm dialog; a discard would be the third hand-written copy |

### Remaining risks

- `gix`'s commit walk is younger than libgit2's. Shallow clones, grafts and
  replace refs are the likely edges; none is covered by a fixture.
- The history is read on demand and never invalidated, so a commit made outside
  Gritty is invisible until Refresh. T070 integration is a separate follow-up,
  as it is for the status.
