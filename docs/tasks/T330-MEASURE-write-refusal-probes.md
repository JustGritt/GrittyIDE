# Task T330-MEASURE — Measure write refusal before building it

- **Phase:** 4 (orchestration)
- **Status:** todo
- **Depends on:** ADR-018A (the hook may refuse a write on ground another workstream holds, accepted 2026-09-14)

## Objective

ADR-018A decides that Gritty's installed `PreToolUse` hook may refuse a
file-tool write on ground another workstream holds. Several things it rests on
were read in the documentation, not executed. Its own table marks two rows
*unverified*, and its acceptance defers a decision until they are measured
(ADR-018A, "What was checked", item 7 of the acceptance):

- whether a `PreToolUse` **deny** still holds under `bypassPermissions`;
- whether `PreToolUse` fires at all on the **owned-run launch line** (`-p`,
  stream-json, `--permission-prompt-tool stdio`), and so whether the user's
  installed entry covers a run Gritty starts or Gritty would have to pass
  `--settings`;
- how the decisions of **several matching hooks combine**. Undocumented, and
  this repository already runs one (`scripts/agent-lease.py guard`);
- what the **fence read** adds to every tool call.

This task executes each question against a real `claude` binary and records the
answers, so T330-R (the fence, the matcher and the hook's refusal) is built on
measured behaviour (hard rule 12). **It writes no product code.**

Split out of T330 (write interception) on 2026-09-14, following the
TURN-START-DEQUEUE precedent: measure, then build.

## Dependencies

- A logged-in `claude` CLI on the machine (2.1.272 at the time of writing). The
  version used is recorded with every result.
- `crates/agent-hook` built, for the process-launch baseline.

## Required reading

- `docs/decisions/ADR-018-agent-integration-bridge.md`: the ADR-018A amendment in full
- `docs/decisions/ADR-020-agent-permission-verdicts.md`: `PreToolUse` vs
  `PermissionRequest` reply shapes
- `crates/agents/src/integration.rs`: `append_entry` (matcher `*`, no
  `PreToolUse` timeout today), `hook_command`
- `crates/agent-claude/src/launch.rs`: the owned-run launch line
- `crates/agent-hook/src/main.rs`, `lib.rs`: exit-code guarantee, `classify`
- `docs/tasks/completed/T335-agent-permissions.md` and ADR-020's evidence, for
  how a hook probe was run before

## Allowed scope

- The scratchpad, for probe harnesses, throwaway repositories and settings files
- This file (the results)
- `docs/decisions/ADR-018-agent-integration-bridge.md`: **only** the ADR-018A
  "What was checked" table, where each *unverified* row gains its measured
  answer and a pointer here. The decision text is not edited; if a result
  contradicts the decision, say so in the report and stop.

## Explicit non-goals

- No product code, no installer change, no fence format. A prototype fence
  reader for the cost measurement lives in the scratchpad and is not committed.
- Never the user's real configuration. No probe reads or writes
  `~/.claude/settings.json`, `~/.claude/sessions/*` or any credential file. Hooks
  are injected per run with `claude --settings <file>` and
  `--setting-sources local`.
- No shell-write or MCP-write measurement beyond confirming what ADR-018A's
  table already says (as command text only, and the server's shape).

## Required behavior

### How to probe (established, reuse it)

- An interactive session is driven through a pty (`pty.fork` + `select`) with
  `claude --settings <hooks.json> --setting-sources local --model <cheapest>`,
  over a throwaway git repository in the scratchpad.
- The folder-trust dialog is answered first (Down, then Enter). Strip every
  `CLAUDE*` environment variable in the child: an inherited
  `CLAUDE_CODE_CHILD_SESSION` silently turns transcript saving off.
- The owned-run line is probed without a pty, as T310-ANSWER did: `subprocess`
  plus stream-json, writing a user turn and reading `control_request`s.
- A **denying hook** for these probes is a small script that prints ADR-018A
  §4's exact JSON (`hookSpecificOutput.permissionDecision: "deny"` with a
  reason) and exits 0 for a named target. It logs every invocation (event,
  tool, `agent_id`, `tool_input` keys) to a file, so "did it fire" is read from
  the log rather than inferred from the outcome.
- **Every probe checks the side effect on disk** (was the file written) and
  what the model was told (the reason in the transcript or stream).

### The questions, one probe each

| # | Question | Pass condition to record |
|---|---|---|
| M1 | Default mode: does a `PreToolUse` deny stop `Write`, `Edit`, `MultiEdit` and `NotebookEdit`, and does the model see the reason? | file unchanged; reason quoted in the transcript |
| M2 | `--permission-mode bypassPermissions` (and `--dangerously-skip-permissions`): does the same deny hold? | as M1 |
| M3a | Owned-run line, hooks from `--settings`: does `PreToolUse` fire, and does deny hold? | hook log shows the call; file unchanged |
| M3b | Owned-run line, hooks from a project-local `.claude/settings.json` in the throwaway repo, standing in for an installed entry with no `--settings`: same | as M3a |
| M3c | On the owned-run line, does a denied `PreToolUse` still produce a `can_use_tool` ask? (ADR-018A §7 says never) | no `control_request` for the refused call |
| M4 | Two matching hooks: deny + silent, deny + allow, allow + silent. What is the combined outcome? | recorded per pair |
| M5 | A `PreToolUse` hook that outlives its `timeout` (set to 2 s, hook sleeps 10 s): does the call proceed, and after how long? | file written; elapsed recorded |
| M6 | A subagent's `Write`: does `PreToolUse` fire with `agent_id` / `agent_type`, and does deny hold? | log shows `agent_id`; file unchanged |
| M7 | Exit 0 with invalid JSON: does the call proceed (the fail-open ADR-018A relies on)? | file written |
| M8 | The payload's target field per tool, especially `NotebookEdit` (`notebook_path` vs `file_path`) | field names recorded |
| M9 | **Fence cost.** Wall time of one hook invocation, baseline (today's `gritty-agent-hook` on a `PreToolUse`) vs baseline plus a prototype fence read, parse and match. Fences of 1, 20 and 200 leases × 10 linked sessions, 1,000 invocations each | p50 / p99 / max for each size, and the machine's load |

### What the report must conclude

1. **Attach route for owned runs:** M3a/M3b decide it. If the installed entry
   fires under `-p`, owned runs can be enforced with no launch change. If only
   `--settings` works, the decision ADR-018A deferred has to be made, and this
   task files an ADR (claim the next number in `docs/merge-queue.md`) rather
   than making it. If neither works, owned runs stay observed and the ADR's
   *Revisit when* is triggered.
2. **`bypassPermissions`:** M2's result is written into the ADR-018A table. If a
   deny does not hold, enforcement must say so per session (ADR-018A §8).
3. **Several hooks:** the combination rule from M4, and whether this
   repository's own lease guard and Gritty's hook can coexist without one
   masking the other.
4. **Timeout value** for the installed `PreToolUse` entry, from M5 and M9: the
   smallest value comfortably above M9's p99.
5. **Fence bounds** (bytes, entries) that keep M9's p99 below the value from
   item 4 on this machine.

## Failure behavior

- A probe that cannot run (not logged in, CLI version changed mid-task, a trust
  dialog variant) is recorded as **not measured**, with the reason. Never as a
  pass.
- A result that differs between two runs is run five times and recorded as a
  distribution, not a verdict.

## Acceptance criteria

- M1–M9 each have a recorded result: measured, or not measured with a reason.
  Each names the CLI version, the exact command line, and where the side effect
  was checked.
- ADR-018A's *unverified* rows are replaced with measured answers linking here.
- The five conclusions above are written, each traceable to a probe.
- No file outside the allowed scope changed; the user's `~/.claude` untouched.

## Tests required

None in the repository. The probes are the tests, and their harnesses are
described in the report precisely enough to re-run: command lines, settings
JSON, the denying script.

## Commands to run

```
claude --version
cargo build -p gritty-agent-hook      # the M9 baseline binary
# probes: scratchpad harnesses, as described in the report
```

## Completion report

_Not started._
