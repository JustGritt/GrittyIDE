# Task T100 — Git baseline: read-only status and diff as domain objects

- **Phase:** 2
- **Status:** todo
- **Depends on:** T020 (open workspace: `WorkspaceId`, canonical `WorkspaceRoot`)

## Objective

Give Gritty its first ability to look at Git at all. Two commands,
`git.status` and `git.diff(target)`, read the repository at the open
workspace's root and return typed domain objects — a repository status and
a structured diff — never a `git diff` string (hard rule 5). Nothing
writes, links a result to a workstream, or creates a second worktree or
branch. This is the read-only floor ADR-007 already specified; everything
built on Git afterward, starting with T400, waits on it existing.

## Motivation

`docs/plans/execution-board.md:89-103` (Lane G) makes the case: T100 is
Phase 2's last dogfood piece and the only path to T400, worktree-per-
workstream isolation — the differentiator that section says competing tools
(echoing the Herdr comparison ADR-018 opened) have chosen not to build.
Nothing in the tree today is prior art for this, despite two strings that
look like it (see "Required reading" and "Failure behavior" for exactly
where): a git-shaped field an agent reported about itself, and a fixture
whose `.git` only looks like a repository.

## Dependencies

T020 for `WorkspaceId`/`WorkspaceRoot`: both commands resolve against
whatever directory is already open. T010's operation model, event bus and
error envelope are reused without extension, as `workspace.list` already
does for its budget-then-operation split
(`crates/gritty-core/src/workspace/mod.rs:1-10`, `:38-40`, `:415-431`).
ADR-007 fixes the shape: read-only first, a domain ChangeSet derived from
Git, never raw diff text. T060/T090/T110 are parallel Phase 2 work per
`AGENTS.md`'s contract-gate diagram, not dependencies; T070 (watcher) is a
separate lane not integrated with here.

## Required reading

- `AGENTS.md` hard rules 2, 5, 6, 9, 10, 11, 12, 13
- `docs/decisions/ADR-007-git-workstreams.md`
- `docs/plans/execution-board.md:89-103` (Lane G), `:215-234` (reservations),
  `:238-257` (shared magnets)
- `docs/Gritty IDE Project Plan.md:231-259` (§6.2, the `git.status()` /
  `git.diff(target)` wire surface), `:260-298` (§7, `Change Set` — its
  `test status`/`review status` need a workstream link not built here),
  `:797` (§21, the open backend decision)
- `crates/gritty-core/src/workstream/mod.rs:1-24`, especially `:12-13`
  ("nothing here reaches into Git") — this task keeps that true
- `crates/gritty-core/src/operation/mod.rs`, `.../workspace/mod.rs`
  (budget-then-operation), `crates/workspace/src/listing.rs` (bounded,
  cancellable reads), `.../event/mod.rs:58-67`, `.../protocol/export.rs:1-5`
- `docs/architecture/security.md:9`, `:30-69`, `:76-79`, `:95-97`

## Allowed scope

```
crates/git/**                                (new — gritty-git)
crates/gritty-core/src/git/**                (new)
apps/desktop/src/features/git/**             (new)
```

Plus the shared magnets this touches but does not own
(`execution-board.md:238-257`): root `Cargo.toml` (the new `gritty-git` path
entry and one external backend entry, §2-3),
`packages/protocol/src/{generated/**,types.ts,client.ts}`,
`crates/gritty-core/src/protocol/{mod.rs,export.rs}`, and
`apps/desktop/src-tauri/src/{commands.rs,core.rs,lib.rs}` — claimed and
released in one pass at the end, since `commands.rs` is 1040 lines already.

## Explicit non-goals

- **No writes, no second worktree, no branch-per-workstream.** Stage,
  commit, checkout, push, fetch and T400's worktree isolation are future
  tasks; neither command changes a byte.
- **No `ChangeSet` with test/review status, no workstream awareness.** That
  entity is workstream-linked (hard rule 6); this builds its primitives, not
  itself. `src/git/**` never imports `src/workstream/**`, and the reverse
  stays true (`workstream/mod.rs:13`).
- **No file-watcher integration.** `git.status` is polled on demand; wiring
  it to T070 is a recorded follow-up, not built here (hard rule 14).
- **No remote operations, no credentials, no submodule recursion.**
  Ahead/behind (§5) reads only local tracking info, never fetching; a
  submodule path is reported, not entered.
- **No full-repository index or cache.** Hard rule 9 forbids a speculative
  caching layer without a concrete requirement and an ADR.
- **No invented visual design.** No git-panel artboard exists yet (§10).

## Required behavior

### Backend decision — open, needs an ADR before implementation

1. `docs/Gritty IDE Project Plan.md:797` leaves this open: shell out to
   `git`, or an in-process library (`git2`/libgit2, or `gix`).
2. **Recommendation: `gix` (gitoxide), in-process, no external `git`
   process.** Hard rule 13's untrusted-content concern is sharpest here:
   `gix` is memory-safe end-to-end, where shelling out means parsing
   another program's porcelain text (the string-shaped-diff problem hard
   rule 5 forbids, generalized to status) and inherits the user's own git
   config as ambient authority (aliases, `core.pager`, credential helpers).
   Its iterator-based traversal also composes with cancellation (§6) rather
   than a spawned process's lifecycle. Against `git2`: both avoid the
   shell-out concerns, but `git2` wraps a C library, a larger memory-safety
   surface for attacker-influenced data. **Decided:
   `docs/decisions/ADR-023-git-library.md`** takes `gix`, read-only, as
   exactly one new *external* `[workspace.dependencies]` entry, pinned
   exact, with the dependency-policy justification. One constraint it adds
   that this spec did not have: the adapter must open repositories with
   external filters and `textconv` disabled, **proven by a test** against a
   fixture whose `.git/config` and `.gitattributes` ask for one — a library
   is not "no process" by construction, only if configured that way.

### `git.status`

3. New crate `gritty-git` at `crates/git/` (pre-assigned,
   `execution-board.md:229-230`), joining the `crates/*` member glob
   (`Cargo.toml:3`) automatically; it still needs its own
   `[workspace.dependencies]` path entry, like the existing internal
   entries at `Cargo.toml:17-22` (none of which carry a justification
   comment — that convention is for external crates, §2), so
   `crates/gritty-core` can depend on it the way it already depends on
   `gritty-workspace`. `crates/gritty-core/src/git` exposes `git.status`
   against the open workspace's root; a mismatched `workspace_id` fails
   `NotFound`, matching `workspace/mod.rs`'s precedent.
4. The response is a tagged union — `Ready` (a `RepositoryStatus`),
   `NotARepository`, or `Pending` (an `Operation`) — mirroring
   `ListDirectoryResponse`'s `{status: "ready"|"pending"}` shape
   (`workspace/mod.rs:98-106`). A plain, non-Git folder is a value, not an
   error (`security.md:63`, "nothing is filtered").
5. `RepositoryStatus` carries: branch, or absent with `detached: true` and
   the HEAD commit id; a `RepositoryState` flag (clean, merging, rebasing,
   cherry-picking, bisecting, reverting, bare); a bounded, paginated
   per-file list (path, index/worktree state, conflicted flag, rename
   source) using the `DirectoryPage` cursor shape; and local ahead/behind,
   absent without an upstream.
6. A status that outlives a short budget hands off to a cancellable
   `git.status` operation like `workspace.list`'s `DEFAULT_LIST_BUDGET`
   hand-off (`workspace/mod.rs:6-10`, `:38-40`): it returns
   `Pending { operation }`, the page arrives on `OperationCompleted`, and no
   new event family is needed — the existing `Operation*` events
   (`event/mod.rs:63-67`) are reused. Cancellation is checked at a bounded
   interval independent of repository size, proven by test, not asserted
   from code shape (rule 12).

### `git.diff`

7. `git.diff` takes an optional revision `target`, defaulting to `HEAD`
   (`Project Plan:244-245`), and returns the same three-way tag as
   `git.status`. `Ready` carries a `DiffSummary`: per file, path (and old
   path for a rename), change kind, addition/deletion counts, a `binary`
   flag, and — for non-binary files — bounded hunks (start/length, lines
   tagged context/added/removed). **No field ever carries the whole diff as
   one string** — hard rule 5, checked in "Acceptance criteria". A binary
   file carries `binary: true` and its size only (`security.md:76-79`).
   Files and hunks are each bounded with their own `truncated` flag — the
   "say what happened" discipline `OutputBatch.truncated_bytes` established
   for terminal output (`T097-shell-integration-blocks.md:107-109`).

### Untrusted content and wiring

8. Branch names, paths and commit text follow the workspace's decode
   policy — lossy UTF-8, passed through, never interpreted
   (`security.md:59-60`) — and are never logged; tracing records ids, paths
   and counts only (`security.md:95-97`). A repository root or `.git`
   reached only through a symlink resolves under the same policy as any
   other workspace path (`security.md:47-54`); no git-specific carve-out.
9. New types join `export_bindings` (`protocol/export.rs:42-50`) beside the
   existing `workspace::*`/`workstream::*` groups; command adapters join
   `commands.rs`'s per-domain grouping. `PROTOCOL_VERSION` (currently
   `2.10.0`, `protocol/mod.rs:27`) bumps MINOR, additive only; no migration,
   since this task persists nothing.
10. A minimal panel under `apps/desktop/src/features/git/` lists a
    `git.status` result and renders a selected file's `git.diff` from its
    structured hunks, never a raw string. No design artboard exists for
    this yet — keep it plain rather than inventing one.

## Failure behavior

| Situation | Behavior |
|---|---|
| Workspace root is not a Git repository | `NotARepository`, not an error |
| Bare repository | `RepositoryState::Bare`; a working-tree diff returns an empty, labeled result, not an error |
| Detached HEAD | `branch` absent, `detached: true`, `head` set — never backfilled from `AgentSession.git_branch` (`agent_session/wire.rs:183`), a different, untrusted source |
| Merge, rebase or bisect in progress | `RepositoryState` names which; conflicted files are flagged per entry, not one opaque error |
| `index.lock` held by another process | Reads need no index lock; stale-lock contention fails `Io`, retryable; no lock file is touched |
| Submodule path | Reported as an entry, never entered |
| Repository root or `.git` reached via a symlink | Same symlink policy as any workspace path (§8); no exception |
| Repository large enough to threaten the cancellation budget | The bounded check interval (§6) still fires, proven by test, not claimed from code shape |
| `git` absent from `PATH` | Not applicable under the recommended backend (§2); kept because the backend ADR is still open — a future decision to shell out must re-handle this |
| Corrupt or unreadable object | `Io` (or a dedicated unreadable state if the backend distinguishes it) — never a panic, never partial data shown as complete |

## Acceptance criteria

- On a real Git repository built at test time, `git.status` reports correct
  branch, HEAD and per-file states across staged/unstaged/untracked/deleted/
  renamed changes; on a plain folder it returns `NotARepository`.
- No field in `GitStatusResponse`/`GitDiffResponse` is an untyped string
  carrying a whole diff or status blob, checked against the generated
  TypeScript bindings; a binary file's diff entry carries `binary: true`
  and no content; cancelling a `git.status` operation on a large synthetic
  repository stops work within one check interval, measured, not assumed.
- `pnpm check` is green; `gritty-git` and `crates/gritty-core/src/git`
  compile with no `#![allow(unsafe_code)]`; generated types are fresh; no
  repository content appears in tracing output for either command.

## Tests required

- **Unit (Rust):** `RepositoryStatus` against repositories built at test
  time — clean, staged, unstaged, untracked, deleted, renamed, conflicted,
  detached HEAD, bare; `NotARepository` on a plain directory; `workspace_id`
  mismatch to `NotFound`; `DiffSummary`/hunk construction with a binary file
  and a rename; large-repository cancellation, proving the check-interval
  bound rather than inferring it.
- **Unit (TS):** client methods and the panel's rendering of the
  `Ready`/`NotARepository`/`Pending` tagged union.
- **Integration:** both commands through the Tauri adapter against a
  **real** repository built at test time (`tempfile::TempDir` plus either
  the chosen backend's init/commit calls or a test-only shelled `git`) —
  `fixtures/small-repo` and `large-repo` both lack a real `.git`
  (`fixtures.rs:121-123`) and cannot stand in.
- **Manual:** a workspace with real uncommitted changes, a merge conflict
  and a detached HEAD, confirming the diff view never shows one text blob.

## Commands to run

```
cargo test -p gritty-git
cargo test -p gritty-core
cargo clippy --workspace --all-targets -- -D warnings
pnpm check
```

## Completion report

_(filled in by the implementing session)_ — must record: the dependency
justification for the new external backend entry (the internal `gritty-git`
path entry needs none); whether the §2 recommendation was followed or
overridden, and why; that the backend-choice ADR was actually filed; the
`PROTOCOL_VERSION` claimed in `docs/merge-queue.md` (2.13 is a planning
reservation, not the record — `execution-board.md:219-220`); which of the
cancellation, platform-build and manual checks actually ran, with results;
and any follow-up recorded rather than built.
