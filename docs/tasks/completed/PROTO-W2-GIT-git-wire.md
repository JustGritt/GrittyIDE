# Task PROTO-W2-GIT — The git wire: `git.status` and `git.diff` reach the shell

- **Phase:** 2 (dogfood floor)
- **Status:** done
- **Depends on:** T100 (the git core), ADR-023 (the library choice)

## Objective

T100 shipped `GitService`, the wire types and the domain model, and **nothing
could call any of it**: `packages/protocol/src/commands.ts` had no `git.*` entry,
`apps/desktop/src-tauri/src/commands.rs` had no `git.` in it at all, and
`GitService` was referenced nowhere outside its own module. This task is the
wire between the two.

## Dependencies

`GitStatusRequest`, `GitDiffRequest` and their three-armed responses already
exist in `crates/gritty-core/src/git/wire.rs`, and `RepositoryStatus` and
`DiffSummary` already derive `TS` in `gritty-git`. Nothing about the contract
needed inventing; it needed exporting.

ADR-023 is `Proposed` and T100 already built on `gix` under the roadmap
amendment of 2026-09-06. This task inherits that amendment rather than
re-deciding it — accepting or rejecting ADR-023 is lane H's, and an agent may
not decide an ADR.

## Required reading

- `crates/gritty-core/src/git/wire.rs` — why all three arms are values
- `crates/gritty-core/src/git/mod.rs` — the budget hand-over
- `docs/decisions/ADR-023-git-library.md`
- `docs/merge-queue.md` — rule 2, claiming a version before spending it

## Allowed scope

- `crates/gritty-core/src/protocol/**` (the version, the exports)
- `crates/gritty-core/src/git/**`
- `apps/desktop/src-tauri/**`
- `packages/protocol/src/**`
- `docs/merge-queue.md`

## Explicit non-goals

- **No write surface.** Nothing here stages, commits, checks out, branches or
  fetches. Both commands are reads; a write surface is a later task and a
  deliberate decision rather than an omission.
- No UI. The panel is `T100-U` and is not this ticket.
- No change to the wire types T100 froze.
- Not the rest of `PROTO-W2` — see below.

## Required behavior

1. `PROTOCOL_VERSION` 2.17.0, claimed in `docs/merge-queue.md` **before** it is
   spent.
2. The four git root types are exported, and the sixteen types that follow
   from them generate into `packages/protocol/src/generated` and are
   re-exported from `types.ts`.
3. `git.status` and `git.diff` are in `COMMANDS`, on the client, and registered
   as Tauri handlers.
4. `GitService` is built in `Core`.
5. All three response arms reach the caller as values: `ready`,
   `notARepository`, `pending`.

## Failure behavior

- A workspace that is not open → `NotFound`, before anything is read.
- A `path` that leaves the workspace root → `InvalidInput`, while parsing.
- A `pageSize` outside `1..=5000` → `InvalidInput`.
- A read that outlives `DEFAULT_GIT_BUDGET` becomes a cancellable `Operation`
  rather than blocking the command; no new event family carries it.
- A workspace that is not a repository is **not** a failure.

## Acceptance criteria

- `core.git.status` and `core.git.diff` reach a real repository from the
  WebView's own IPC.
- `pnpm check` green, generated types committed and fresh.
  (See the report: another session's concurrent edit fails one stage.)

## Tests required

- Protocol client: the names, the request shapes, all three arms, the error
  envelope.
- Integration through the real IPC, against a real repository built at test
  time.

## Commands to run

```
cargo test -p gritty-core --lib protocol
cargo test -p gritty-desktop --test commands
pnpm exec vitest run packages/protocol/src/git-client.test.ts
pnpm check
```

## Completion report

### Summary

The git core is reachable. `PROTOCOL_VERSION` is 2.17.0 (claimed in the merge
queue first, per rule 2), the four root types export and bring sixteen
transitively, `git.status` and `git.diff` are on the client and registered as
handlers, and `GitService` is built in `Core` beside the other managers.

No wire type changed. T100's contract was already right; this task exported it.

### Files changed

| File | Why |
|---|---|
| `docs/merge-queue.md` | claim 2.17.0; record that `PROTO-W2` has shrunk |
| `crates/gritty-core/src/protocol/mod.rs` | 2.16.0 → 2.17.0 |
| `crates/gritty-core/src/protocol/export.rs` | four root types |
| `packages/protocol/src/generated/*` | sixteen generated types (never hand-edited) |
| `packages/protocol/src/commands.ts` | two names |
| `packages/protocol/src/client.ts` | the `git` namespace and its documentation |
| `packages/protocol/src/types.ts` | sixteen re-exports |
| `packages/protocol/src/git-client.test.ts` | new: 7 tests |
| `apps/desktop/src-tauri/src/core.rs` | build `GitService` |
| `apps/desktop/src-tauri/src/commands.rs` | two adapters |
| `apps/desktop/src-tauri/src/lib.rs` | two `generate_handler!` entries |
| `apps/desktop/src-tauri/Cargo.toml` | `gritty-git` with `fixtures`, dev only |
| `apps/desktop/src-tauri/tests/commands.rs` | 4 integration tests |

### Tests actually executed

- `pnpm exec vitest run packages/protocol/src/git-client.test.ts` — 7 passed
- `pnpm exec vitest run packages/protocol` — 84 passed across 15 files
- `cargo test -p gritty-desktop --test commands` — 44 passed (4 new)
- `cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
  — **exit 0**, 877 tests
- `cargo fmt --all --check` — clean; `biome check .` — clean, 262 files
- `pnpm --filter @gritty/protocol typecheck` — clean

**`pnpm check` as a whole did not pass, and not because of this work.** Another
session shares this working tree and was mid-edit on
`apps/desktop/src/features/session/persisted-layout.ts`: it added a required
`agents` field to `Layout` without yet updating `persisted-layout.test.ts`, so
`apps/desktop`'s typecheck fails on a fixture that is missing it. That file is
outside this ticket's scope and outside its lease, so it was left alone.

Every stage of the gate was therefore run separately, and every one that covers
this ticket's files is green — the list above is that decomposition rather than
a summary of one command. The one stage that cannot be run before committing is
`check:generated`, which asserts `packages/protocol/src/generated` is clean in
git and so only passes once the new files are committed.

**The manual click-through was not run, and this task does not claim it** — the
same limit T300-WIRE recorded, and for the same reason: this session has no
screen access. There is also nothing yet to click, since the UI is `T100-U`.
What was executed is the IPC: the integration tests build a real repository with
`gritty_git::fixtures::clean()`, open it as a workspace, and invoke the adapters
the way the WebView does.

### Architectural decisions

**One new dependency, dev-only:** `gritty-git` with `features = ["fixtures"]`
in `apps/desktop/src-tauri`'s `[dev-dependencies]`. `gritty-core` already takes
exactly this for its own T100 tests, so it is a precedent rather than a new
one, and the feature is compiled out of the production build. The alternative —
testing the adapters only against a plain directory — would have exercised the
`notARepository` arm and left `ready` unproven across the serialization
boundary, which is the arm a wire task most needs to prove.

**`DEFAULT_GIT_BUDGET` is passed by the adapter, not defaulted in the service.**
It matches how `workspace.list` does it: the budget is a policy of the shell,
and the service takes it as an argument so a test can shorten it.

**`PROTO-W2` shrank and should be re-scoped, not taken as written.** It was
"watcher, git, terminal blocks, diagnostics, violations, in one PR". T070 and
T326 shipped their own wire, this ticket takes git, and what remains is terminal
blocks (behind ADR-017, undecided) and diagnostics (T110, unbuilt) — two things
that will not travel together. Recorded in `docs/merge-queue.md`.

### Known limitations

- **Read-only.** No staging, committing, branching, checkout or fetch. That is
  the non-goal above, not an oversight, but it does mean "git support" is a
  bigger claim than what exists.
- Nothing consumes these commands yet: `T100-U` is the panel.
- The `pending` arm is covered by a client test with a fake transport and by
  T100's own core tests, but **not** end to end through the IPC — provoking it
  needs a repository slow enough to outlive the budget, and the test hook that
  does that (`read_delay`) is `#[cfg(test)]` inside `gritty-core` and not
  reachable from the desktop integration test. The hand-over itself is T100's
  and is tested there.
- ADR-023 is still `Proposed`.

### Follow-up tasks

| Ticket | Why |
|---|---|
| `T100-U` | the panel, already on the board |
| `PROTO-W2` | re-scope to terminal blocks + diagnostics, or split and close |

### Remaining risks

`RepositoryStatus` serialization is now proven for a clean repository and one
uncommitted change. The states T100 tests in the core — mid-rebase, mid-merge,
conflicted, hostile filters — are not re-proven across the IPC, and a
serialization problem specific to one of those would show up first in the UI.
