# Task T100 — Git baseline: read-only status and diff as domain objects

- **Phase:** 2
- **Status:** review
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

> **Split by `PLAN-FANOUT` (2026-09-06).** This spec now covers two board
> tickets: **T100** is everything below *except* `features/git/**`, and
> **T100-U** (lane `Gu`) is that directory alone. Both build behind
> `PROTO-W2`, which owns the wire and the magnets, so neither touches
> `packages/protocol` or `PROTOCOL_VERSION`. The command adapters in
> `src-tauri/src/commands.rs` stay with T100.

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

### T100 (lane G — core half)

_(T100-U, lane `Gu`, owns `apps/desktop/src/features/git/**` and reports
separately below.)_

**Summary.** `crates/git/` (package `gritty-git`) reads a repository through
`gix`, and `crates/gritty-core/src/git/` turns that into `git.status` and
`git.diff` with the `workspace.list` budget-then-operation split. Both stop at
the magnet: **no wire change of any kind**. `docs/tasks/T100-git-baseline.md:62-63`
defers the wire to `PROTO-W2`, and `docs/merge-queue.md:114-116` records that
**T100 claims no protocol version**. So `PROTOCOL_VERSION` is untouched
(still `2.12.0`), no `CoreEvent` variant was added, and
`packages/protocol/**`, `crates/gritty-core/src/protocol/**` and
`apps/desktop/src-tauri/src/commands.rs` are not in the diff. The wire surface
the later ticket needs is written out below rather than built.

**Files changed.**

| Path | What |
|---|---|
| `crates/git/Cargo.toml` | new package `gritty-git`, plus the off-by-default `fixtures` feature |
| `crates/git/src/lib.rs` | module map and the four "will not do" boundaries |
| `crates/git/src/open.rs` | ADR-023 decision 2 in code, and the upstream-ref fallback it costs |
| `crates/git/src/model.rs` | `RepositoryStatus`, `DiffSummary` and everything under them |
| `crates/git/src/status.rs` | the three status streams merged per path |
| `crates/git/src/diff.rs` | changed paths, both sides read directly, structured hunks |
| `crates/git/src/page.rs` | pagination and the cancellation-check bound |
| `crates/git/src/path.rs` | Git bytes → `WorkspacePath` |
| `crates/git/src/fixtures.rs` | repositories built at test time (test-only) |
| `crates/gritty-core/src/git/mod.rs` | `GitService`, the budget hand-over, `GitError` → `CoreError` |
| `crates/gritty-core/src/git/wire.rs` | request/response types, `TS`-derived and ready to export |
| `Cargo.toml` | the `gritty-git` path entry and one external `gix` entry |
| `Cargo.lock` | resolved `gix` tree |
| `crates/gritty-core/Cargo.toml` | depends on `gritty-git`; dev-dependency enables `fixtures` |
| `crates/gritty-core/src/lib.rs` | `pub mod git;` and its line in the module map |

`crates/gritty-core/src/lib.rs` is the one file outside the lane's two
directories. It is a two-line module registration, without which
`src/git/**` is not compiled at all; it is not on `execution-board.md`'s
magnet list.

**Dependency justification.** One new external `[workspace.dependencies]`
entry, as ADR-023 budgets:

```
gix = { version = "0.87.1", default-features = false,
        features = ["status", "revision", "max-performance-safe", "sha1"] }
```

Read a repository in-process: pure Rust end to end, so the one subsystem whose
inputs are an attacker's file format never reaches a C parser, and no `git`
subprocess inherits the user's configuration as ambient authority. It overlaps
nothing in the tree. `default-features = false` is load-bearing rather than
tidiness — it drops `extras`, which is what keeps the network transports,
credential helpers, prompts and `signal-hook` out of the build: with these
features there is no code path to a remote at all. `max-performance-safe`
brings `parallel`, which the status iterator needs to be interruptible (without
it `gix` buffers the whole status before yielding anything and the read cannot
be cancelled). The internal `gritty-git` path entry carries no justification
comment, matching the existing internal entries. **No second dependency was
added:** the test fixtures hand-roll their temporary directory the way
`gritty-agent-hook`'s tests do, rather than pulling in `tempfile`.

**§2 recommendation: followed, and ADR-023 is filed**
(`docs/decisions/ADR-023-git-library.md`, still `Proposed`; the roadmap
amendment of 2026-09-06 accepts building against it). Nothing during the build
gave a reason to reconsider `gix` — status, diff, rev-parsing and ahead/behind
were all reachable. Its one constraint beyond this spec is decision 2, and that
turned out to be the hardest requirement in the ticket; see the next section.

**Architectural decisions.**

1. **Nothing repository-influenced can run a program — the mechanism.**
   `gix` will honour a `.git/config` that names a clean/smudge filter or a
   `diff.<driver>.textconv`, so "in-process" had to be made true rather than
   assumed. `gix::open::Options::isolated()` removes the *user's* config
   (system, `~/.gitconfig`, `GIT_*`, `include.path`), and
   `filter_config_section(|_| false)` removes the *repository's* ability to
   define a driver: `gix` gates `filter.*` and `diff.*` driver extraction
   behind exactly that predicate, so both lists come back empty and there is
   nothing left to execute. The hook is a `fn(&gix_config::file::Metadata)`,
   which describes the config *file* and not the section, so a narrower rule
   ("reject `filter.*`, keep `remote.*`") is not expressible — refusing
   everything is the only shape with no gap.
2. **What that costs, and how it is paid back.** The same predicate hides
   `remote.*`/`branch.*` from `gix`'s remote resolution, which would have taken
   ahead/behind (§5) with it. `open::upstream_ref_name` therefore reads
   `branch.<name>.merge` and `branch.<name>.remote` straight out of the
   resolved config and reproduces the default
   `refs/remotes/<remote>/<branch>` mapping. Reading a value is not running a
   program. **Known limitation:** a repository with a non-default fetch
   refspec reports *no* upstream rather than a wrong one.
3. **The repository is opened at the workspace root, never discovered above
   it.** A workspace opened at `repo/crates/git` would otherwise report paths
   outside the workspace root, which ADR-010 exists to prevent. The cost is
   that opening a subdirectory of a repository answers `NotARepository`
   (follow-up 1).
4. **`git.diff` reads both sides itself instead of using `gix`'s blob
   pipeline.** Which paths changed comes from the status machinery with the
   target tree substituted for HEAD; what each side *contains* is then read
   from the tree and from the worktree directly. Two reasons: the change kind
   falls out of "which side exists" so no HEAD→index and index→worktree merge
   rules are needed (that merge is where diff views usually acquire wrong
   badges), and the `textconv` code path is *absent* rather than merely
   disabled. **Known limitation:** a `binary`/`-diff` gitattribute is not
   honoured; the NUL-byte heuristic Git falls back on without one is used.
5. **Hunks are structured, never text.** `gix::diff::blob::UnifiedDiff` with a
   `ConsumeHunk` collector yields 1-based line numbers and per-line
   Context/Add/Remove tags, copied into `DiffHunk`/`DiffLine`. No `+`/`-`
   prefix is ever written into a string. Asserted against the *serialized*
   response, not the Rust types, by
   `git::tests::no_response_field_carries_a_whole_diff_as_one_string`.
6. **A symlink is compared by its target and never followed** — faithful to
   Git, and the reason no link in the worktree can read a file outside it.
7. **`unrepresentable` counts, rather than hides, paths with no protocol
   representation** (in practice a filename containing a backslash, which is
   unaddressable on Windows and therefore in the protocol). Silently dropping
   one would be the "partial data shown as complete" the failure table forbids.
8. **`gritty-git` is a boundary, not a backend abstraction** (ADR-023 decision
   3): no trait with one implementation. `gix` types never leave the crate.

**Tests actually executed, with results.**

- `cargo test -p gritty-git` — **45 passed, 0 failed.** Repositories built at
  test time cover clean, staged, unstaged, untracked, deleted, staged rename,
  ignored (on and off), merge conflict, detached HEAD, unborn branch, bare,
  submodule (reported, not entered), ahead/behind, paging with a cursor;
  diff additions/deletions/renames/binary/symlink/arbitrary revision/unknown
  revision/path filter/hunk bounds/bare/paging; and path decoding.
- `cargo test -p gritty-core` — **328 passed, 0 failed**, of which 9 are the
  new `git::tests`: the `Ready`/`NotARepository`/`Pending` union, `NotFound`
  on a `workspace_id` mismatch, `InvalidInput` naming `target` and `pageSize`,
  both budget hand-overs completing on `OperationCompleted` with the payload,
  cancellation, and the rule-5 check above.
- **The cancellation check, run and measured.**
  `page::tests::cancellation_is_noticed_within_one_check_interval` measures
  the bound exactly — at most `CANCEL_CHECK_INTERVAL` (256) further items are
  consumed after the predicate flips, over a 10 240-item sequence, with the
  consumed count observed rather than inferred.
  `status::tests::cancelling_a_large_status_abandons_the_work_rather_than_finishing_it`
  runs a real 3 000-file repository twice: it measures what the whole read
  costs, then cancels a tenth of the way in and asserts the cancelled run
  returns `Cancelled` in less time — which is what shows the flag reaches
  `gix`'s producer threads rather than the result being computed and thrown
  away. The core-level budget tests use a `#[cfg(test)]` read-delay hook, the
  same one `WorkspaceManager::listing_delay` exists for, so the hand-over is a
  decision rather than a race.
- **ADR-023 decision 2, run and proven.**
  `open::tests::no_program_named_by_the_repository_can_run` builds a repository
  whose `.git/config` defines `filter.evil.{clean,smudge}` and
  `diff.evil.textconv` and whose `.gitattributes` points two patterns at them,
  each command being a script that touches a marker file. It asserts (a) the
  **control** — the same repository opened with `gix`'s defaults *does* produce
  a driver, so the test cannot go vacuous after a `gix` upgrade; (b) opened the
  way Gritty opens repositories the driver list is empty; (c) a full status and
  a full diff over the targeted files leave the marker untouched. Unix-only,
  because the fixture needs an executable script.
- `cargo clippy --workspace --all-targets -- -D warnings` — clean.
- `pnpm check` — **green, exit 0**, run locally in full (GitHub Actions is dark
  per `CI-LINUX-DARK`, so this is the only evidence). `check:generated` passes
  because `packages/protocol/src/generated` is untouched.
- **Platform build: macOS (aarch64) only.** Nothing here is
  platform-conditional except the two Unix-only tests (the filter fixture needs
  `chmod +x`; the symlink diff test needs `std::os::unix::fs::symlink`), which
  are `#[cfg(unix)]`. No Windows or Linux build was run. `gix` adds no C
  toolchain, so no new platform requirement was introduced.
- **Manual check: not run.** The task's manual pass wants the diff *view*, which
  is T100-U's panel and does not exist in this half. What this half can be
  checked against is covered by the fixtures above; the manual pass belongs
  with T100-U.

**Follow-ups recorded, not built.**

1. **Opening a subdirectory of a repository reports `NotARepository`.** Deliberate
   (decision 3), but a user who opens `repo/crates/git` gets an empty panel with
   no explanation. A future ticket could report "this is inside a repository
   rooted at X" as a distinct arm.
2. **No watcher integration.** `git.status` is polled on demand; wiring it to
   T070's invalidation is explicitly out of scope here.
3. **A non-default fetch refspec yields no upstream** (decision 2). Reading
   `remote.<name>.fetch` and matching it would fix it.
4. **`binary`/`-diff` gitattributes are not honoured** (decision 4).
5. **Untracked files do not appear in `git.diff`**, matching `git diff HEAD`.
   A review-oriented view may later want them as additions; that is a product
   decision, not an oversight.
6. **`gix` is pre-1.0 with a history of breaking minors** (ADR-023's standing
   consequence). Upgrades are deliberate tasks.
7. **The sidecar must be staged before `cargo test --workspace` succeeds** in a
   fresh worktree (`apps/desktop/scripts/prepare-sidecar.mjs`); `pnpm check`
   does not do it. Hit while running the checks here, and already filed as
   `SIDECAR-FRESH-CHECKOUT` — noted only as a second sighting.

**Remaining risks.**

- Decision 2's mechanism depends on `gix` continuing to gate driver extraction
  behind `filter_config_section`. The control assertion in the test is what
  turns a silent regression into a failing test, but it is a `gix` internal and
  a future version could route around it. That is the risk ADR-023's "revisit
  when" clause is for.
- `gix`'s status is younger than libgit2's, and ADR-023 names submodules,
  sparse checkout and conflicted entries as the likely edges. Submodules and
  conflicts have fixtures here; **sparse checkout does not** and is untested.
- The large-repository cancellation test compares two timed runs, so it is
  timing-sensitive under heavy parallel load, though it cancels at 10% of a
  freshly measured baseline and so has a wide margin. The exact bound it backs
  up is proven by a deterministic unit test.

### Wire surface required by `PROTO-W2`

Nothing below exists yet; this is the input the wire ticket needs.

**Commands (2, both read-only).**

| Command | Request | Response |
|---|---|---|
| `git.status` | `GitStatusRequest` | `GitStatusResponse` |
| `git.diff` | `GitDiffRequest` | `GitDiffResponse` |

Both are `async` on `GitService` and take `(request, Actor, CorrelationId, DEFAULT_GIT_BUDGET)`,
exactly like `workspace.list`. The adapters belong in `commands.rs`'s
per-domain grouping, and `GitService::new(workspaces, registry, handle)` needs
wiring in `core.rs`/`lib.rs`.

**Event variants: none.** Both commands reuse the existing `Operation*` events;
a `Pending` response completes on `OperationCompleted`, whose `result` is a
serialized `RepositoryStatus` or `DiffSummary`.

**`PROTOCOL_VERSION`: MINOR bump, additive only. No migration** — this task
persists nothing and `SCHEMA_VERSION` is untouched.

**Types to add to `export_bindings` (`protocol/export.rs`),** as a
`gritty_git::*` / `git::wire::*` group beside the existing ones. Dependencies
export transitively, so the roots are:

```
GitStatusRequest, GitStatusResponse, GitDiffRequest, GitDiffResponse
```

which pull in, from `gritty_git::model`:

```
RepositoryStatus, RepositoryState, UpstreamStatus, FileStatus, FileState, EntryKind,
DiffSummary, FileDiff, DiffChangeKind, DiffHunk, DiffLine, DiffLineKind
```

All twelve already derive `Serialize`/`Deserialize`/`TS` with
`#[serde(rename_all = "camelCase")]`, and the enums use
`SCREAMING_SNAKE_CASE`, matching the existing protocol conventions. The two
responses are `#[serde(tag = "status")]` unions with arms
`ready` / `notARepository` / `pending`, mirroring `ListDirectoryResponse`.

**Note for whoever writes it:** `gritty-core` re-exports nothing from
`gritty-git` today. `export.rs` will need `use gritty_git::{...}` directly, or
`crate::git::wire` for the four request/response roots.

### T100-U (lane Gu — the panel)

_(not this session)_
