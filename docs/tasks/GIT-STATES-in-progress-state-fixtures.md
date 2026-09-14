# Task GIT-STATES — Test the in-progress repository states git status names but never exercised

- **Phase:** 2
- **Status:** review
- **Depends on:** T100 (git baseline), ADR-023 (git read in-process through `gix`)

## Objective

Every `RepositoryState` arm `crates/git/src/status.rs::repository_state` can
return is asserted against a real repository left in that state.

## Motivation

ADR-023's Consequences require T100's fixtures to "cover a repository in each
interesting state (merging, rebasing, detached, conflicted, no upstream)", and
name `gix`'s status as younger than libgit2's on exactly these edges. Today
merging, detached, bare, unborn, submodule and no-upstream are tested.
`Rebasing`, `CherryPicking`, `Reverting`, `Bisecting` and `ApplyingMailbox` are
mapped from `gix::state::InProgress` and never exercised — so a mapping that is
wrong for one of them (a `git rebase --apply` reported as applying a mailbox,
say) would ship as a wrong label with green tests.

## Required reading

- `docs/decisions/ADR-023-git-library.md` (Consequences), `docs/decisions/ADR-029-git-write-backend.md`
- `docs/tasks/completed/T100-git-baseline.md`
- `crates/git/src/fixtures.rs` (the one place that may run the `git` binary, with user config off)
- `crates/git/src/status.rs` (`repository_state` and its tests), `crates/git/src/model.rs` (`RepositoryState`)

## Allowed scope

- `crates/git/src/fixtures.rs`, `crates/git/src/status.rs` (tests module; the
  `repository_state` mapping only if a test proves it wrong)
- this task file

## Explicit non-goals

- No new `RepositoryState` variant and no wire change: a mapping fix that
  would need one is recorded as a follow-up, not made.
- No sparse-checkout support work; a test that documents current behaviour is
  in scope, a fix is not.
- No new dependency.

## Required behavior

1. A fixture per state, built with the `git` binary the way
   `fixtures::merge_conflict` is: a rebase stopped on a conflict (merge
   backend), a `git rebase --apply` stopped on a conflict, an interactive
   rebase stopped on `edit`, a cherry-pick and a revert stopped on a conflict,
   a bisect in progress, and a `git am` stopped on a patch that does not apply.
2. A test per fixture asserting `RepositoryStatus.state`, and where the state
   leaves a conflict, that the conflicted file is flagged the way the merge
   test asserts it.
3. Where `gix` distinguishes a variant `RepositoryState` collapses (the two
   rebase backends; a single cherry-pick against a sequence), the test names
   which input it covers.

## Failure behavior

A fixture whose `git` command does not reach the intended state fails loudly
at construction (assert the marker directory exists), never as a misleading
status assertion.

## Acceptance criteria

- Every `RepositoryState` arm except `Clean`/`Bare`/`Merging` (already covered)
  has a passing test against a real repository.
- Any arm `gix` reports differently from `git status`'s own wording is either
  fixed in the mapping or recorded as a follow-up with the evidence.
- `cargo test -p gritty-git` and `pnpm check:rust` green.

## Tests required

The new unit tests in `crates/git/src/status.rs`.

## Commands to run

```
cargo test -p gritty-git
pnpm check:rust
```

## Completion report

### Summary

Every `RepositoryState` arm is now asserted against a real repository, and two
mappings the new tests proved wrong are fixed, both without a wire change.

Before writing a test, what `gix` 0.87.1 detects (`gix/src/repository/state.rs`)
was read, and the marker files Git 2.50.1 actually leaves were probed in a
scratch directory, along with `git status`'s wording for each. Two
disagreements between `gix` and `git status` came out of that probe:

1. **A stopped cherry-pick or revert sequence concluded with a plain
   `git commit` read as `Clean`.** The commit removes `CHERRY_PICK_HEAD` /
   `REVERT_HEAD`, which is all `gix`'s `state()` looks for, and leaves
   `.git/sequencer/todo` with the remaining commits. `git status` says
   "Cherry-pick currently in progress" / "Revert currently in progress"
   (`wt_status_get_state` → `sequencer_get_last_command`). This is an ordinary
   workflow — resolve, `git commit` — and Gritty would have told the user
   nothing was in progress while picks were still queued. Fixed:
   `pending_sequence` reads the first command of `sequencer/todo` (bounded to
   4 KiB, since it is repository content) when `gix` reports nothing, the way
   Git decides it: `pick`/`p` → `CherryPicking`, `revert` → `Reverting`,
   anything else → `Clean`.
2. **`gix`'s `ApplyMailboxRebase` mapped to `ApplyingMailbox`.** That variant
   is a `.git/rebase-apply` with neither an `applying` nor a `rebasing` marker,
   and Git's `wt_status_check_rebase` calls it a rebase — only `applying`
   makes it `git am`. Remapped to `Rebasing`. No current Git leaves this state
   behind (both writers in `am.c` write one marker), so the test builds a real
   apply-backend rebase and removes the marker — the one place this task
   edits `.git` by hand, and said so in the test's doc.

**Mutation-tested:** with both fixes reverted, exactly the three tests they
exist for fail (`a_cherry_pick_sequence_committed_mid_way_is_still_cherry_picking`,
`a_revert_sequence_committed_mid_way_is_still_reverting`,
`a_rebase_apply_directory_without_markers_is_rebasing`); the other 23 status
tests pass.

### Which input covers which `gix` variant

| `gix::state::InProgress` | Fixture | `RepositoryState` |
|---|---|---|
| `RebaseInteractive` | `rebase_conflict` (plain `git rebase`: the merge backend writes `rebase-merge/interactive` even when not asked) and `interactive_rebase_edit` | `Rebasing` |
| `Rebase` | `rebase_apply_conflict` (`rebase-apply/rebasing`; current Git never writes a `rebase-merge` without `interactive`) | `Rebasing` |
| `ApplyMailboxRebase` | `rebase_apply_conflict` with the marker removed | `Rebasing` (was `ApplyingMailbox`) |
| `CherryPick` / `CherryPickSequence` | `cherry_pick_conflict` / `cherry_pick_sequence_conflict` | `CherryPicking` |
| `Revert` / `RevertSequence` | `revert_conflict` / `revert_sequence_conflict` | `Reverting` |
| `Bisect` | `bisecting` | `Bisecting` |
| `ApplyMailbox` | `am_conflict` (`git am --3way`) | `ApplyingMailbox` |
| none (`sequencer/todo` only) | `sequence_committed_mid_way(…)` | `CherryPicking` / `Reverting` (was `Clean`) |

Every fixture asserts its marker exists at construction, so a fixture that
misses its state fails there and not as a status assertion.

### Files changed

- `crates/git/src/fixtures.rs` — `diverged` and `rewritten` base histories,
  `assert_git_dir_has`, and ten fixtures: `rebase_conflict`,
  `rebase_apply_conflict`, `interactive_rebase_edit`, `cherry_pick_conflict`,
  `cherry_pick_sequence_conflict`, `revert_conflict`,
  `revert_sequence_conflict`, `sequence_committed_mid_way`, `bisecting`,
  `am_conflict`.
- `crates/git/src/status.rs` — `repository_state` mapping (the two fixes),
  `pending_sequence` + `TODO_HEAD_BYTES`, and 13 tests.
- this task file.

No wire change: `RepositoryState` is untouched, `packages/protocol/**` is not
in the diff, no `PROTOCOL_VERSION` claimed. No new dependency.

### Tests actually executed

- `cargo test -p gritty-git` → exit 0, **74 passed**, 0 failed (13 of them
  new), after the final formatting edit.
- Mutant run (`cargo test -p gritty-git --lib status::tests`, both fixes
  reverted) → exit 101, 3 failed / 23 passed, as above; file restored.
- `cargo clippy -p gritty-git --all-targets -- -D warnings` → exit 0.
- `cargo fmt --all --check` → exit 0.
- **The workspace gate, run by the committing session:**
  - `pnpm check`: exit 1, and not because of this task. `cargo fmt --all
    --check` stopped on `apps/desktop/src-tauri/tests/commands.rs`, an
    uncommitted edit inside the live WATCH-FLAKE lease (two load-sensitive
    watcher tests). `check:ts` ran before that step and vitest passed
    1,346/1,346.
  - `rustfmt --check` on this task's two files: exit 0.
  - `cargo clippy --workspace --all-targets -- -D warnings`: exit 0.
  - `cargo test --workspace`: exit 101. Its one failure was
    `a_directory_created_after_the_watch_started_is_followed` in
    `crates/workspace/tests/watch.rs`, the WATCH-FLAKE test that ticket is
    fixing.
  - `cargo test --workspace --no-fail-fast`: exit 0, 1,271 passed, 0 failed,
    `gritty-git` 74 of them.
  - `pnpm check:generated`: exit 0.

### Architectural decisions

- **The adapter reads one file `gix` does not.** ADR-023 decision 3 keeps
  `gix` types inside `gritty-git`; it does not require that every fact come
  through `gix`. `pending_sequence` is in-process, runs no program, reads a
  bounded head of one file, and mirrors a documented Git function, so it
  stays inside decisions 1 and 2. If `gix` grows the check upstream, the
  function should be deleted in favour of it.
- **Git's reader is the reference, not `gix`'s variant names.** Where they
  disagree (`ApplyMailboxRebase`), `git status`'s wording wins, because that
  is what the user compares Gritty against.

### Known limitations

- **One state per repository.** Git's status is a set of flags and can print
  two at once (a bisect with a cherry-pick stopped inside it); `gix` and
  `RepositoryState` report one, in `gix`'s precedence order. Not changed:
  it would be a wire change.
- **`interactive_rebase_edit` runs `sed` as the sequence editor through the
  shell**, like `hostile_filters`' filter script, so these fixtures assume a
  Unix shell. Fine while verification is macOS-only.
- **Sparse checkout is still untested** — the task allowed documenting it and
  this did not get there.
- `pending_sequence` treats a todo whose first command sits after more than
  4 KiB of leading whitespace as `Clean`. Git never writes one.

### Follow-up tasks

- **Sparse checkout status** — a fixture with `git sparse-checkout set` and a
  test that records what `git.status` reports for paths outside the cone.
- **Report upstream to `gix`** that `Repository::state()` misses a sequence
  left with only `sequencer/todo`, and that `ApplyMailboxRebase` disagrees
  with `wt_status_check_rebase`; delete `pending_sequence` if it lands.
