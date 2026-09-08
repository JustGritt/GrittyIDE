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

_Reported below, under its own `## Completion report — T100-U` heading._

## Completion report — T100-U

### Summary

The **Changes** tab exists, and the sidebar header is a real tablist.

`git.status` becomes a grouped list of changed paths; selecting one calls
`git.diff` for that file alone and draws its structured hunks. Both reads carry
all three arms of their response as values — `ready`, `notARepository`, and
`pending` with a cancellable `Operation` whose payload arrives later on
`OperationCompleted`. Nothing is written: `git.status` and `git.diff` are the
only two git commands on the wire, so read-only is a property of the surface
rather than a promise this panel makes.

The header change is T233-S's own follow-up 4, and its comment in
`explorer-panel.tsx` was the spec for it: it renamed the label to **Files**,
left it an `<h2>`, and wrote down why — artboard 9a draws a second tab,
**Changes**, and there was nothing behind it, because no git command reached
the renderer. `PROTO-W2-GIT` put `git.status`/`git.diff` on the client at
`PROTOCOL_VERSION` 2.17.0, so the second tab now has somewhere to go and the
heading became what 9a drew.

**No wire change of any kind.** `packages/protocol/**`,
`apps/desktop/src-tauri/**` and `crates/**` are not in the diff; this ticket
claims no `PROTOCOL_VERSION` and no `LAYOUT_VERSION`.

### Files changed

| File | What |
|---|---|
| `features/git/git-model.ts` (+`git-model.test.ts`) | new — grouping, status codes, head/upstream lines, hunk line numbering; 22 tests |
| `features/git/git-reducer.ts` (+`git-reducer.test.ts`) | new — both reads, the operation hand-over and the race around it; 22 tests |
| `features/git/use-git.ts` | new — the controller: two commands, one subscription, cancel, refresh, paging |
| `features/git/changes-view.tsx` (+`changes-view-render.test.ts`) | new — everything the tab draws; 20 render tests |
| `features/git/changes-panel.tsx` | new — the container half, 30 lines, owns the controller and nothing else |
| `features/git/sidebar-tabs.tsx` (+`sidebar-tabs.dom.test.tsx`) | new — the tablist, roving tabindex, Arrow/Home/End; 8 DOM tests |
| `features/explorer/explorer-panel.tsx` | the header becomes `<SidebarTabs>`; the tree and the changes list become two `role="tabpanel"` panels |
| `App.tsx` | `sidebarTab` state and the `changes` element (composition only, +13 lines) |
| `app.css` | the `.git*` block, `.explorer__tablist` / `.explorer__tab`, `.explorer__panel`, and the two scrollers added to the narrow-viewport rule |
| `features/session/shell-restore.dom.test.tsx` | two assertions follow the heading→tab change, plus one new one for the Changes tab |

**No new dependency.** Nothing was added to `package.json` or the catalog.

### Architectural decisions

1. **The tablist lives in `features/git/`, not in a shared `components/`
   module.** There are exactly two tabs and one caller; a generic tab component
   with one consumer would be the speculative abstraction hard rule 2 warns
   about. A third section is what would justify hoisting it, and moving it then
   is a rename. `explorer-panel.tsx` imports it rather than owning it, which is
   what kept the edit to that shared magnet to the header and the two panels.
2. **Only the selected panel is in the document, and that is a behavioural
   decision, not a rendering one.** T100 rules watcher integration out, so
   `git.status` is polled on demand; "not mounted" is therefore the same thing
   as "not polling", and nothing reads the repository while the tree is
   showing. The alternative — both panels mounted with `hidden` — would have
   put a git read behind the file tree on every startup.
3. **`ChangesView` is split from `ChangesPanel`.** The container calls
   `useGit`; the view takes the state it produced. This is `agent-console.tsx`'s
   shape and it is what lets the render tests mount every arm — including the
   two a repository will not produce on demand — without constructing the core
   client's event channel at import. (Before the split the render test emitted a
   `window is not defined` diagnostic from the transport, which is exactly the
   unexplained warning the definition of done forbids.)
4. **The diff is asked for one file at a time, through `git.diff`'s `path`.**
   That is what the argument is for: the bounded way to reach the selected
   file's hunks instead of paging every changed file's hunks to get at one.
5. **The `pending` race is handled the way `explorer-reducer.ts` handles it.**
   The command response and the completion event are separate IPC messages with
   no ordering guarantee, so an outcome that arrives first is parked in a
   **bounded** map (8 entries) and claimed when the response names it. Outcomes
   are routed by `operation.kind`, so a `git.diff` completion can never land on
   the status read and vice versa — asserted, not assumed.
6. **Cancellation is cooperative and never optimistic.** The Cancel button asks
   `operations.cancel`; the state changes when `OperationCancelled` says it did.
   A refusal to cancel is rendered on the read it belongs to.
7. **`notARepository` is drawn as a value, never as an error envelope**, and it
   says *why* a folder inside a repository reads the same way (T100's decision 3
   — the repository is opened at the workspace root and never discovered above
   it). That turns follow-up 1 of the core half from a mystery into a sentence.
8. **A partially staged file appears under both Staged and Changes**, because
   that is what it is. Picking one band would be a claim the repository does not
   make. A conflicted file is only ever under Conflicts.
9. **Hunks are drawn as numbered columns, never as text.** The sign is its own
   column and the content the core sent is untouched, which is what keeps a line
   that legitimately starts with `-` from reading as a removal. The `@@` header
   is *composed* from the hunk's four numbers rather than parsed out of a string
   — hard rule 5, asserted against the rendered markup.
10. **Colour is never the only channel.** The two-column code has a tooltip that
    says the same thing in words; a conflict has a `conflict` badge as well as a
    colour; a truncated hunk, a truncated file list and an unrepresentable path
    each say what happened in a sentence, the way
    `OutputBatch.truncated_bytes` established for terminal output.
11. **No new colour value and no invented layout.** T100 §10 says no artboard
    exists and to keep the panel plain. The bar is the explorer's 32px section
    bar and the diff rows are `permission-prompt.tsx`'s, down to `--ok-wash` /
    `--error-wash`, so the light ramp follows without a second palette.
12. **The tab is not persisted.** `persisted-layout.ts` belongs to another
    ticket this wave, and a session that reopened on Changes would read the
    repository before the user asked it to.

### Tests actually executed, with results

- `npx vitest run --project desktop src/features/git` — **4 files, 72 tests,
  all passed.** 22 model, 22 reducer, 20 render, 8 DOM.
- `npx vitest run --project desktop` — **55 files, 616 tests, all passed.**
  The baseline before this work was 51 files / 544 tests, so nothing existing
  broke and 4 files / 72 tests were added.
- `npx vitest run` (whole workspace) — **85 files, 795 tests, all passed.**
  `packages/editor`'s `model-eol` DOM test (`MONACO-JSDOM-FLAKE`) did not fire
  on any of these runs; it is not this ticket's and is not claimed either way.
- `pnpm typecheck` — all four projects clean.
- `npx biome check .` — **272 files, clean**, no fixes pending.
- `pnpm check:ts` (lint + typecheck + the whole test suite) — **exit 0**.
- `check:generated` — passed when run (20:05), with
  `packages/protocol/src/generated` clean. It has since gone red in this shared
  working tree because `W-t080` regenerated types for `PROTO-W3`; **none of
  those files is this ticket's**, which touches no Rust and no protocol source.
- `npx vite build` in `apps/desktop` — built in 3.12s. The >500 kB chunk
  warning is Monaco's and pre-existing.
- **`pnpm check` as a whole was not run, and this report does not claim it.**
  Its `check:rust` stage compiles two other sessions' in-flight crates
  (`W-t310a` in `crates/agent-claude/**`, `W-t080` in `crates/search/**` and
  `crates/gritty-core/src/search/**`), so its result would not be this
  ticket's. No Rust is touched here. Every stage that covers this ticket's
  files is listed above, and that list is the decomposition rather than a
  summary of one command.
- **The manual pass was not run and is not claimed.** There is no screen access
  on this machine and the chrome-devtools MCP server would not connect — the
  same limit T233-C and `PROTO-W2-GIT` recorded. Nothing here has been seen on
  a screen.

Three tests exist specifically to stop this from rotting:

- `changes-view-render.test.ts` renders **every** arm of both reads, including
  `pending` with its Cancel, `notARepository`, a `CoreError` envelope, a
  cancelled read, an empty repository (no commits) and a clean one — the arms a
  happy-path test never reaches.
- The same file asserts hard rule 5 on the *markup*: the content of a diff line
  is never prefixed into a `+`/`-` string, and a hostile branch name, path and
  diff line are escaped rather than rendered as markup (hard rule 13).
- `shell-restore.dom.test.tsx` is the only test that mounts the real `App`, so
  it now asserts the Changes tab is present after a startup restore, beside the
  existing assertions that the Agents section is there and the rail is gone.

### Known limitations

- **Not seen on a screen.** No visual-regression tooling, no screen access, and
  the chrome-devtools MCP server would not connect. The light/dark and 800×600
  lines are unverified and are the first thing a manual pass should check.
  Every value is an ADR-016 token, which is the argument that the light ramp
  follows — not evidence that it does.
- **The diff shares the sidebar column with the list**, so a wide hunk scrolls
  horizontally in ~300px. That is the plain panel T100 §10 asks for rather than
  a design decision; the editor area would be the right home for it (follow-up
  1).
- **Nothing refreshes itself.** The list is read on mount, on a workspace
  change, and when Refresh is pressed. T100 rules watcher integration out
  explicitly, so a change made outside Gritty is invisible until asked for.
- **Switching to Files while a read is pending abandons the operation rather
  than cancelling it.** The panel unmounts, the core finishes a read nobody
  claims, and coming back re-reads. It is bounded and read-only, so it wastes
  work rather than breaking anything (follow-up 4).
- **A failed "Load more" drops the page already shown** and offers a retry that
  re-reads from the first page. Simpler than a second error slot, and the
  refusal is never swallowed — but it does cost the user their place.
- **No shortcut reaches the tab.** ⌥G belongs to the Agents section and the
  shortcut registry T335 asked for is still unbuilt; a seventh hand-gated
  `keydown` listener is not what this ticket should add.
- **`formatBytes` duplicates `explorer-panel.tsx`'s `formatSize`** — three
  lines of presentation, duplicated across a ticket boundary the way T233-M
  duplicated `titleCase`, rather than exported across one.
- The states T100 tests in the core — mid-rebase, mid-merge, conflicted,
  hostile filters — are rendered here from hand-built fixtures, not from a real
  repository through the IPC. That gap is `PROTO-W2-GIT`'s recorded risk and
  this ticket does not close it.

### Follow-up tasks

| Follow-up | Why |
|---|---|
| A diff opens in the editor area as a tab, the way T233-C moved the agent console there | ADR-026 already made the argument for a wide surface; a hunk in a 300px column is the same complaint |
| Clicking a changed file opens it in the editor beside its diff | The obvious next gesture; it needs `documents.openFile` wired through, which is scope this ticket did not take |
| A count on the Changes tab | Needs `git.status` polled while the Files tab is showing, which needs T070's invalidation first — otherwise it is a background poll with no trigger |
| Cancel a pending read when the tab is left, or keep the operation and re-adopt it on return | See the limitation above |
| The shortcut registry (T335), then a chord for each sidebar section | This is now the seventh listener that would want one |
| Persist the selected sidebar section in `Layout` | Deliberately not done here; `persisted-layout.ts` is another ticket's file this wave |

### Remaining risks

- The `pending` arm is proven by the reducer and by the rendered markup, but
  **never end to end**: provoking it needs a repository slow enough to outlive
  `DEFAULT_GIT_BUDGET`, and `PROTO-W2-GIT` recorded that the test hook which
  does that is `#[cfg(test)]` inside `gritty-core` and unreachable from outside.
  If the core ever stopped emitting `OperationCompleted` for a git read, the
  panel would sit on "Reading the repository (operation)…" and only a manual
  pass would notice.
- `groupEntries` decides the bands from `FileState` values. A newer core adding
  a state would put entries in `unstaged` by default rather than in a band named
  for it — the letter and the tooltip would show the new value, but the grouping
  would be a guess. The lookups degrade loudly; the grouping degrades quietly.
- The panel reads `RepositoryStatus.entries` to decide whether a file with no
  diff is untracked. If a status page is truncated past the selected file, that
  explanation is not shown and the pane says only "No difference against HEAD".
