# Task T222-SEAM — A provider registry, so providers stop being serial

- **Phase:** 3
- **Status:** todo
- **Depends on:** T220 (agent session discovery), T230 (the transcript follower)

## Objective

Every agent provider Gritty observes lands in the same two files:
- `crates/agents/src/probe.rs`: which process is an agent, which session it is
  running, which transcript it holds open;
- `crates/agents/src/transcript.rs`: where transcripts live, how a tail becomes
  a roster line, how appended bytes become timeline entries.

So providers are strictly serial. Two sessions adding two providers edit the
same functions, and `transcript.rs` is already the busiest file in the agent
lane. PLAN-FANOUT split T222 (more providers) for exactly this reason:
**T222-SEAM** extracts the provider-specific code behind a registry, and
**T222-CODEX** (Codex, as one provider file) adds Codex through it
(`docs/tasks/completed/T222-agent-providers.md`, split note).

This task is the extraction, with **no behavior change**: after it, Claude Code
is discovered, ranked and followed exactly as before, and adding a provider is
**one new file, one registration line and one enum variant**.

## Dependencies

- T220 and T230 on `main`: `gritty-agents`, `discover`, `merge`, the bounded
  scan, the follower.
- **Coordination, not a code dependency.** `transcript.rs` has had four
  tickets in a row: AGENT-TURN-END (the roster says Done while the model is
  still generating), LAST-PROMPT-SHELL (a `!` command is not a session's last
  prompt), TURN-START-QUEUED (a dequeued message or a `!` command starts a turn)
  and LAST-PROMPT-NOTIFY (a task notification is not a session's last prompt).
  All are committed. Before taking this ticket, check `agent-lease.py list` and
  `git status` for uncommitted work in `crates/agents/src/`. An extraction on
  top of someone else's uncommitted hunk entangles two tickets in one diff.

## Required reading

- `docs/architecture/agent-sessions.md`: "Adding a provider", "The timeline (T230)"
- `docs/tasks/completed/T222-agent-providers.md`: the split note, the
  non-goals (**no provider trait**), §4 (verify against a real installation),
  §12 (the scan budget is global)
- `crates/agents/src/probe.rs`: `Provider`, `provider_of_command`,
  `is_subagent_worktree`, `open_transcript_of`, `session_id_of_transcript`,
  `session_id_of_command`, `discover_processes`
- `crates/agents/src/transcript.rs`: `ScanOptions`, `scan`,
  `parse_transcript` (the hard-coded `Provider::ClaudeCode`), `Fold`, `follow`
  and `follow_with`, `absorb_entries`
- `crates/agents/src/lib.rs`: `discover`, `merge`
- `crates/gritty-core/src/agent_session/mod.rs`: `for_home` (the
  `.claude/projects` root) and the two `gritty_agents::follow` call sites

## Allowed scope

- `crates/agents/src/**`, including the new `providers/` directory
- `crates/gritty-core/src/agent_session/mod.rs`: building scan roots from the
  registry and passing the provider to the follower; nothing else
- `docs/architecture/agent-sessions.md` ("Adding a provider"), this file

## Explicit non-goals

- **No trait.** T222's non-goal stands, and this design keeps it. The registry
  is a static table of plain data and function pointers, one row per provider.
  If review concludes that is a trait by another name, stop and say so rather
  than widening.
- **No new provider**, Codex included. T222-CODEX adds it.
- **No wire change.** `AgentProvider` keeps its one variant, and no
  `PROTOCOL_VERSION` bump.
- **Not the hook bridge.** `hook.rs` (event names), `integration.rs`
  (`HookTarget`), `verdict.rs` (the reply shape) and `crates/agent-hook` keep
  their per-provider match arms. Each lives in its own file, so they are not
  what makes providers serial, and Codex's integration half is optional in
  T222.
- No change to ranking, merging, bounds or the scan budget.

## Required behavior

### The registry

1. `crates/agents/src/providers/mod.rs` defines:

   ```rust
   pub struct ProviderSpec {
       pub provider: Provider,
       /// Where this provider keeps transcripts, relative to the home directory.
       pub transcripts_under_home: &'static str,
       /// The executable name that classifies a process (probe.rs: never a substring).
       pub executable: &'static str,
       pub session_id_of_command: fn(&str) -> Option<String>,
       pub open_transcript_of: fn(lsof: &str) -> Option<PathBuf>,
       pub session_id_of_transcript: fn(&Path) -> Option<String>,
       pub is_subagent_worktree: fn(&Path) -> bool,
       /// Tail (and, for a provider that needs it, head) → one roster line.
       pub read_transcript: fn(&Path, &ReadBounds, modified_ms: i64) -> Option<TranscriptEvidence>,
       /// Appended bytes → timeline entries.
       pub follow: fn(&Path, from_offset: u64, FollowOptions) -> Followed,
   }

   pub static PROVIDERS: &[&ProviderSpec] = &[&claude_code::SPEC];
   ```

   Field names are the spec's suggestion. Their *set* is the contract: every
   place `probe.rs` or `transcript.rs` names Claude Code today becomes a field.
2. `providers/claude_code.rs` receives Claude Code's functions verbatim, moved
   and not rewritten:
   - from `probe.rs`: the executable name, session-id-from-flags, the lsof
     transcript match, the transcript-name rule, the worktree rule;
   - from `transcript.rs`: the record-format fold (`Fold`, turn boundaries, the
     tool detail, `is_cli_delivered` and siblings) and the follower's record
     parsing (`absorb_*`, `result_detail`, `hook_errors`).
3. **Generic machinery stays where it is:** bounded reads (`read_tail`,
   `read_window`, `file_len`), candidate collection, timestamp parsing,
   clipping, `merge`, ranking. A provider file calls these and does not copy
   them.

### Who reads the registry

4. `transcript::scan` walks one root per registered provider, under **one
   global** file budget (T222 §12), and tags each result with the provider
   whose root it came from. No hard-coded `Provider::ClaudeCode`.
5. `probe::discover_processes` classifies a process by looking its executable
   up in the registry, and uses that row's session-id and transcript functions.
6. `gritty_agents::follow` takes the provider. `agent_session/mod.rs` passes
   the session's provider at both call sites, and `for_home` builds its roots
   from `PROVIDERS` rather than naming `.claude/projects`.
7. `discover` and `scan` gain `_with(providers: &[&ProviderSpec], …)`
   variants that tests call with their own table. The plain functions call
   them with `PROVIDERS`.

### Docs

8. `docs/architecture/agent-sessions.md` "Adding a provider" is rewritten:
   one variant on `Provider`, one file in `providers/`, and one line in
   `PROVIDERS`. It says why this is a table and not a trait.

## Failure behavior

Unchanged from T220 and T230, and the tests must still show it:
- a missing root yields no transcripts;
- `ps` or `lsof` absent yields no process evidence;
- an oversized or torn transcript stays bounded;
- an unknown record is skipped.

A provider row whose root does not exist costs one failed `read_dir`, as
today.

## Acceptance criteria

- **No behavior change, measured on real data.** A scratchpad harness runs
  `discover` and `follow` over the real `~/.claude/projects` corpus before and
  after the change, and the outputs are identical (sessions, ranking, previews,
  turn marks; timeline entries for a sample of transcripts). The report states
  the corpus size.
- **A provider is one file.** A test-only provider (`#[cfg(test)]`, with its
  own fixture transcripts in a temp directory and its own executable name)
  registered through a test table is:
  - discovered from a fixture transcript;
  - classified from a fake `ps` line;
  - merged;
  - followed.

  All without editing any existing function.
- `grep -rn 'ClaudeCode\|\.claude' crates/agents/src` finds Claude Code only in
  `providers/claude_code.rs`, the `Provider` enum and the hook-bridge files the
  non-goals exempt.
- `cargo test -p gritty-agents` and `cargo test -p gritty-core --lib
  agent_session` pass with the existing tests unchanged apart from import paths.

## Tests required

- Existing `gritty-agents` tests move with the code they test, with no edits
  to their assertions.
- New: the test-only provider end to end (discover, probe, merge, follow).
- New: two providers share the scan budget. With a cap of N files and two
  roots, at most N transcripts are read in total.
- The before/after corpus harness above, kept in the scratchpad and not
  committed. Its result is recorded in the report.

## Commands to run

```
cargo test -p gritty-agents
cargo test -p gritty-core --lib agent_session
pnpm check
```

## Completion report

_Not started._

Follow-up to file on completion: give T222-CODEX (Codex, as one provider file)
its own task file, extracted from `docs/tasks/completed/T222-agent-providers.md`,
now that the seam it needs exists. A Codex installation is present on Alexis'
machine (`~/.local/bin/codex`, `~/.codex/`), which T222 §4 requires for
verification.
