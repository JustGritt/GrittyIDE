# Task T310-RUN-PATHS — A run's file paths, made workspace-relative

- **Phase:** 4 (orchestration)
- **Status:** todo
- **Depends on:** T310-ANSWER (arm the host permission verdict for an owned run — same files, at review), T310-A (the Claude adapter)

## Objective

Every file path a real Claude run reports arrives as **outside the workspace**,
so nothing a run touched can be opened, and every consumer that wants "the
files this run changed" gets an empty list.

The cause is one function. `claude_driver::workspace_path` hands the adapter's
raw string straight to `WorkspacePath::parse`, which accepts only
workspace-relative paths — and Claude Code's file tools send **absolute** ones
(`file_path`, `notebook_path`). The fallback is `AgentRunPath::outside(raw)`,
so `AgentRunPath.path` is always `None` for a real run. Its own test encodes
the gap: `/etc/passwd` is correctly not openable, but neither is
`<root>/src/main.rs`, and nothing tests that it should be. T310-ANSWER's report
names the same limitation for asks ("a run's paths are absolute and not
classified against the workspace").

The fake driver hides this: its scripts use relative paths, so every UI built
against it shows openable files a real run never produces.

This task classifies a run's paths against the run's workspace root, the way
the observed side already does for a session's transcript (T230).

Split out of T310-U-FILES (files touched, on an owned run's console) on
2026-09-14, because that list is empty until this lands.

## Dependencies

- **T310-ANSWER** (at review) owns `crates/gritty-core/src/agent_run/**`; build
  after it lands.
- `WorkspaceRoot::relativize` and `WorkspacePath::from_native`
  (`crates/workspace/src/root.rs`, `path.rs`, ADR-010).
- The run's root: the Claude driver already holds its `ClaudeLaunch`, built
  with the workspace root's path (`apps/desktop/src-tauri/src/commands.rs`,
  `ClaudeLaunch::new(&session_id, root.path())`).

## Required reading

- ADR-010 (workspace path representation) and ADR-019 §5 (what path matching
  does not promise: symlinks, case aliases)
- `crates/gritty-core/src/agent_run/claude_driver.rs` — `tool_started_events`,
  `tool_finished_events`, `workspace_path`, and its test
  `only_a_path_inside_the_workspace_is_openable`
- `crates/gritty-core/src/agent_session/timeline.rs` — `fold_file_touched` and
  `lexical_absolute`, the rule to match
- `crates/agent-claude/src/stream.rs` — where `file_path` / `notebook_path`
  become a `ClaudeToolIntent`
- ADR-021 §4 — the durable run summary keeps only inside-workspace paths

## Allowed scope

- `crates/gritty-core/src/agent_run/claude_driver.rs` and `claude_driver/tests.rs`
- `crates/workspace/src/path.rs` (one new public lexical helper, below) and its tests
- `docs/architecture/agent-runs.md` (a sentence on path classification), this file

## Explicit non-goals

- **No wire change.** `AgentRunPath { path?, display }` already has the shape;
  this task makes `path` present when it should be.
- **No symlink or case resolution.** Lexical only, exactly as T230 chose: a
  `stat` per event is not paid, and ADR-019 §5 already records that a symlink
  or a case alias can make a path look outside. A path Claude reports through
  a symlinked directory stays outside, and that is the safe direction.
- **Not the observed side.** `agent_session/timeline.rs` keeps its private
  `lexical_absolute`; switching it to the shared helper is a follow-up, so this
  task does not touch the session lane's files.
- **Not the fake driver.** Its scripts are relative by construction and stay so.
- Not the files-touched list itself (T310-U-FILES).

## Required behavior

1. **Move the lexical rule where both lanes can reach it.** Add
   `gritty_workspace::path::lexical_absolute(base: Option<&Path>, raw: &str) ->
   Option<PathBuf>` — the same algorithm as `timeline.rs`'s private copy: an
   absolute `raw` is taken as is, a relative one is joined to `base`; `.` is
   dropped, `..` pops, and popping past the start is `None`. No filesystem
   access. The workspace crate owns path representation (ADR-010), so the two
   lanes share it without importing each other.
2. **Classify against the run's root.** The driver's path conversion takes the
   run's working directory (`launch.working_directory()`) as the base and root:
   - `~`-prefixed: outside, verbatim (unchanged — never expanded).
   - resolve with `lexical_absolute(Some(root), raw)`; if that yields a path
     and `WorkspacePath::from_native(root, &absolute)` succeeds and is not the
     root itself, the result is `AgentRunPath::inside(path)` with `display`
     set to the workspace-relative string;
   - otherwise outside, with `display` the raw string (verbatim, as today).
3. The same conversion serves `FILE_READ`, `FILE_WRITE_REQUESTED` and
   `FILE_WRITTEN`, and a permission ask's `target` stays display text (asks
   never carry an openable path).
4. `docs/architecture/agent-runs.md` says in one sentence that a run's paths
   are classified lexically against its root.

## Failure behavior

- A root path that is not canonical (it should always be — `WorkspaceRoot`
  canonicalizes) would make every path outside: fail closed, never open.
- `..` that climbs out of the root, a path on another volume, a Windows verbatim
  or UNC path: outside.
- Invalid UTF-8 in a component: outside.

## Acceptance criteria

- With root `R`, `R/src/main.rs` is inside with display `src/main.rs`;
  `src/main.rs` (relative) is inside; `R/../etc/passwd`, `/etc/passwd`,
  `~/.ssh/id_rsa`, `R` itself and `R/src/../../escape` are outside.
- A real Claude run that writes a file in the workspace records
  `FILE_WRITTEN` with `path` present — shown by a driver test against the stub
  child using an absolute path under a scratch root, and noted from one
  `pnpm dev` run in the report.
- Nothing that was outside before becomes inside except paths under the root.

## Tests required

- `crates/workspace/src/path.rs`: `lexical_absolute` table (absolute, relative
  with base, relative without base → `None`, `.`/`..`, popping past start).
- `claude_driver/tests.rs`: replace
  `only_a_path_inside_the_workspace_is_openable` with a table over a scratch
  root covering the acceptance cases, and a stub-child run whose `Write` tool
  uses an absolute path.
- Mutation check, stated in the report: drop the root-relative branch and the
  absolute-inside case fails; drop the "not the root itself" filter and the
  `R` case fails.

## Commands to run

```
cargo test -p gritty-workspace
cargo test -p gritty-core --lib agent_run
pnpm check
```

## Completion report

_Not started._

Follow-up to file on completion: switch `agent_session/timeline.rs` to the
shared `lexical_absolute`, deleting its private copy.
