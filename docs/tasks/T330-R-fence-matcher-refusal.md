# Task T330-R — The fence, the matcher, and the hook's refusal

- **Phase:** 4 (orchestration)
- **Status:** todo
- **Depends on:** T330-MEASURE (measure write refusal before building it), T325 (workspace ownership registry), T326 (observed ownership violations), T223 (session ↔ workstream links), T221 / T335 (the installed hook and its verdict files)

## Objective

Build the core and hook half of ADR-018A: Gritty's installed `PreToolUse` hook
refuses a `Write`, `Edit`, `MultiEdit` or `NotebookEdit` whose target lies on
ground a **different** workstream holds, when all of these are true:
- Gritty is running;
- enforcement was separately consented to for that provider;
- the session is linked to a workstream.

The core decides who may write where and publishes it as a file, the
**fence**. The hook only relativises a path, matches it with ownership's own
matcher, and prints a denial. Every failure is silence.

The user-facing half is T330-U (the refusal surface). Until both have landed,
`docs/architecture/ownership.md` keeps saying ownership is observed
(ADR-018A §8).

Split out of T330 (write interception) on 2026-09-14.

## Dependencies

- **T330-MEASURE's conclusions are inputs, not suggestions.** They set:
  - the `PreToolUse` timeout;
  - the fence bounds;
  - whether `bypassPermissions` needs a per-session caveat;
  - how Gritty's hook coexists with other `PreToolUse` hooks.

  If MEASURE found that a deny does not hold in default mode, stop: ADR-018A's
  premise is false.
- T325 `OwnershipManager` and `OwnershipTarget::covers`; T326's conflict row,
  dedupe and queue plumbing; T223's `(provider, session_id)` links; T221/T335's
  installer, liveness marker and state files.

## Required reading

- `docs/decisions/ADR-018-agent-integration-bridge.md`: ADR-018A §1–§8, the
  enforcement table, *Not decided here*
- ADR-019 (ownership: §4 one matcher, §5 what matching does not promise, §6
  staleness, §8 override), ADR-020, ADR-010
- `docs/tasks/T330-MEASURE-write-refusal-probes.md` and its report
- `crates/gritty-core/src/ownership/{mod.rs,target.rs,wire.rs}`
- `crates/gritty-core/src/agent_session/mod.rs`: `observe_ownership`, the fast
  loop's marker touch, `follow_hooks`
- `crates/agents/src/{hook.rs,integration.rs,verdict.rs}`
- `crates/agent-hook/src/{main.rs,lib.rs}`
- `crates/persistence/src/store.rs`: `agent_links`
- `docs/merge-queue.md`: claim the protocol minor at take time

## Allowed scope

- `crates/ownership-match/**` (new leaf crate; name open to review)
- `crates/gritty-core/src/ownership/**`
- `crates/gritty-core/src/agent_session/**`
- `crates/agents/src/**`
- `crates/agent-hook/**`
- `crates/gritty-core/src/protocol/{mod,export}.rs`,
  `packages/protocol/src/**` (generated types are regenerated, never
  hand-edited)
- `apps/desktop/src-tauri/src/**`: command adapters for the consent switch only
- `Cargo.toml` (workspace member), `docs/architecture/agent-sessions.md` (the
  hook bridge section), `docs/merge-queue.md`, this file

## Explicit non-goals

- **Owned runs.** Gritty-started runs stay observed unless T330-MEASURE showed
  that the installed entry fires under `-p`, in which case they are covered
  with no launch change. Passing `--settings` at launch needs an ADR first
  (ADR-018A acceptance, item 7) and is a follow-up, not this task.
- Shell writes, MCP writes, Codex, read leases, a per-lease enforcement mode,
  and a one-shot exemption (ADR-018A *Not decided here*).
- The user's own `files.*` writes inside a lease (ADR-028 leaves it open).
- Any UI beyond what the consent command needs from the desktop adapter.
  T330-U draws everything.
- No `allow`, no `ask`, no exit 2, no waiting, no round trip to Gritty from
  the hook.
- Not flipping `docs/architecture/ownership.md`'s "observed" line (T330-U).

## Required behavior

Land it as reviewable commits in this order. Each keeps `pnpm check` green.

### 1. One matcher, in a leaf crate

- Extract ownership's segment matcher (`parse_segments`, `matches_path`,
  `crates/gritty-core/src/ownership/target.rs`) into a new dependency-free
  crate. It operates on `&str` segments and has no `CoreError`, `ts_rs` or
  `WorkspacePath` types.
- `OwnershipTarget::covers` delegates to it, and every existing ownership
  test passes unchanged. `gritty-agents` and `agent-hook` link the leaf crate,
  never `gritty-core`.
- New-crate justification for the report: ADR-018A §3 requires one matcher
  shared by the core and the hook, and the hook must not link `gritty-core`.

### 2. The fence: the core writes it

- **Format:** a versioned, bounded JSON file in the app-data directory beside
  the verdicts (`verdict_dir_from_data_dir`'s parent), written atomically
  (temp file then rename). It holds:
  - `version`;
  - the canonical workspace root;
  - for each linked `(provider, session_id)`, the list of **forbidden
    targets**. Each target carries the pattern, the lease id, the holder
    workstream's name (clipped, control characters stripped) and a
    pre-composed refusal reason (ADR-018A §6 wording, bounded). A lease's note
    never goes in.
- **Forbidden means** (ADR-018A §2): the lease is held, not expired, not stale,
  and its holder is not among the workstreams linked to that session. A session
  with no link has **no entry** and is never refused.
- **Rewritten on** every `OwnershipChanged` and on every link or unlink
  (`workstreams.linkAgent` / `unlinkAgent`). No event exists for a link change
  today, so hook the command path directly.
- **Removed** when enforcement is off for every provider, or when no workspace
  is open.
- **Bounded:** bytes and entries from T330-MEASURE's conclusion. Past the
  bound, the file is **not written** (the hook then stays silent), and the core
  logs the overflow once.

### 3. The hook refuses: `agent-hook`

- **Only when** the installed command line carries the consent flag (for
  example `--enforce`; unknown flags are already ignored), the event is
  `PreToolUse`, and the tool is one of the four.
- **Target field per tool** is taken from T330-MEASURE's M8 result.
  **Fold-in fix:** `classify` reads `file_path` for `NotebookEdit`
  (`crates/agent-hook/src/lib.rs`), whereas the adapter reads `notebook_path`
  (`crates/agent-claude/src/stream.rs`). Correct it for both observation and
  refusal.
- **The chain** (ADR-018A §4). Any failure means silence and exit 0:
  1. liveness marker fresh (`verdict::is_live`), else silence;
  2. fence readable, within bounds, known version, and its root contains the
     target, else silence;
  3. target relativised with `WorkspacePath` rules (`..`, a backslash, or
     outside the root means silence);
  4. matched against that session's forbidden targets with the leaf matcher,
     else silence;
  5. on a match, print ADR-018A §4's `hookSpecificOutput` JSON with the fence's
     reason, then exit 0.
- **The refusal is recorded before printing**, in an append-only, bounded
  refusal log per session beside the hook state. It is not a field in the
  state file: `hook::write` overwrites that file per event, and the next
  `PostToolUseFailure` would erase a refusal before the core's loop reads it.
  Each record holds target, lease id, time and tool. If the bump is needed,
  `HOOK_STATE_VERSION` moves.

### 4. The installer: consent and timeout

- **Consent is per provider.** A new command
  (`agents.setEnforcement { provider, enabled }`) rewrites Gritty's recognised
  `PreToolUse` entry to add or remove the flag, through the same
  diff-shown-then-applied flow install uses (ADR-018 §3). Uninstall removes the
  flag with the entry. Turning enforcement off leaves observation installed.
- **An already-installed entry without the flag** is recognised as installed
  and not enforcing. Nothing is upgraded silently.
- The `PreToolUse` entry gains an explicit `timeout` (T330-MEASURE's
  conclusion).
- `IntegrationStatus` reporting gains whether the entry is enforcing, so
  T330-U can draw it.

### 5. The core records a refusal as its own conflict state

- The fast loop reads the refusal logs for linked sessions (as `follow_hooks`
  reads state) and records a new `ConflictState::Refused` through the same
  path T326 uses for `Observed`:
  - dedupe on (requested_by, lease, target);
  - `requested_by` is the session's workstream;
  - `target` is the file.
- `Refused` is a write that **did not** happen, and must never be merged with
  `Observed`.
- The protocol gains `REFUSED` and a per-session enforcement fact
  (`ENFORCED` | `OBSERVED`, ADR-018A §8), computed from:
  - consent;
  - entry recognised;
  - Gritty running, which is trivially true from the core's side;
  - the session is linked.

  This is additive, one minor bump, claimed in `docs/merge-queue.md`.

## Failure behavior

- **Every hook failure is silence:**
  - no consent flag;
  - stale marker;
  - missing, torn, oversized or unknown-version fence;
  - wrong root;
  - unresolvable target;
  - a panic (already caught);
  - a refusal log that cannot be appended.

  In the last case the hook still refuses; the user just will not see it. Say
  so in the docs.
- The core failing to write the fence leaves the previous fence in place until
  the marker goes stale, because the fence is only trusted while Gritty is live.
  A crash between writing the temp file and renaming it leaves the old fence.
- **A fence one write behind the registry** (ADR-018A *Consequences*): a
  just-released lease still refuses once, and the agent retries. A just-claimed
  one lets a write through, which T326 records as `Observed`.
- **Force-kill test (hard rule 12):** kill the desktop process while a fence
  exists, and show that the hook stops refusing within the marker's 4 s.

## Acceptance criteria

- **In-process tests against the real hook binary and a scratch app-data
  directory** show, for each of these, deny JSON on stdout plus a refusal
  record, or silence with no record, as appropriate:
  - a linked session writing inside another workstream's live lease is
    refused;
  - its own lease is not refused;
  - an unlinked session, a stale lease and an expired lease are not refused;
  - Gritty not running (stale marker) is not refused;
  - no consent flag is not refused.
- One live probe, using T330-MEASURE's harness, repeats the refused case
  against the real `claude` CLI with the entry installed via `--settings`. The
  file is unchanged, and the model's reason names the holder.
- A refusal appears as a `REFUSED` conflict on the wire within one fast-loop
  tick, deduplicated across retries.
- The extracted matcher passes every existing ownership test, plus a
  table-driven test run through both the core's and the hook's entry points
  with the same cases.
- `gritty-agent-hook`'s exit code is 0 in every test, including a fence of
  garbage bytes and a fence of 10× the bound.
- The force-kill test above passes.
- `NotebookEdit`'s target is read from the right field in both paths.

## Tests required

- **Leaf crate:** the matcher table (moved), plus fuzz-style cases for `..`,
  empty segments, globs at the root and trailing slashes.
- **`crates/agents`:**
  - fence write/read round trip;
  - bounds;
  - atomic replace;
  - unknown version rejected;
  - refusal log append bounds.
- **`crates/agent-hook`:** the chain, one test per silent exit and one per
  refusal, spawning the built binary as `installed_command.rs` already does.
- **`gritty-core`:**
  - fence recomputed on acquire, release, override, a workstream going
    `DONE`, link and unlink;
  - `Refused` recorded, deduplicated and never merged with `Observed`;
  - the per-session enforcement fact.
- **Installer:** setting enforcement on or off rewrites only the flag; the
  diff is shown; uninstall removes both; an unflagged entry reads as not
  enforcing.
- **Mutation checks, stated in the report:**
  - drop the stale-lease filter and a test fails;
  - make an unlinked session get an entry and a test fails;
  - return exit 2 on a match and a test fails.

## Commands to run

```
cargo test -p ownership-match        # or the name chosen
cargo test -p gritty-agents
cargo test -p gritty-agent-hook
cargo test -p gritty-core --lib ownership agent_session
cargo test -p gritty-core            # regenerates packages/protocol/src/generated
pnpm check
```

## Completion report

_Not started._

Follow-ups to consider filing on completion: an ADR on attaching the hook to
owned runs with `--settings` (if T330-MEASURE showed it is the only route); a
`files.*` user-write rule inside a lease (ADR-028).
