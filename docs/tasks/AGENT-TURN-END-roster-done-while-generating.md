# Task AGENT-TURN-END — The agent roster says Done while the model is still generating

- **Phase:** 3
- **Status:** doing
- **Depends on:** T220 (agent session discovery), T221 (agent hook bridge:
  earning `BLOCKED`), T223 (attention notifications), T231 (the agent rail).
  All four are on `main`.

## Objective

Say **Done** only when an agent's turn has actually ended. While the model is
thinking or writing, between tool calls or before the first one, the session
reads **Working**.

## Motivation

Found on 2026-09-14 by comparing Gritty's agents section with Open Island, a
menu-bar session monitor, on the same two Claude Code sessions. Gritty said
*Done* for both. Open Island said one of them was *in progress*, and Open
Island was right. Here is that session's transcript (`1f1ee121…`, local time):

| Time | Transcript |
|---|---|
| 10:50:00.7 | `tool_use` Write |
| 10:50:01.0 | `tool_result` for it |
| **10:50:04** | **screenshot: Gritty "Done", Open Island "in progress"** |
| 10:50:16.0 | `thinking` block |
| 10:50:18.8 | final reply text |
| 10:50:19.0 | `system/stop_hook_summary`, `system/turn_duration`: the turn ends |

Claude Code writes a block to the transcript only once it has finished
streaming. So while the model generates, the transcript shows a completed tool
call and nothing outstanding. `activity_of` (`crates/gritty-core/src/agent_session/mod.rs:1641`)
maps "live process, nothing outstanding" to `WAITING`, and `agent-status.ts:53`
labels `WAITING` **Done** ("The turn finished — your move"). The core never
had evidence that the turn had finished; the label claims it anyway.

This is more than a wrong word:

- **False attention.** T223 notifies on *entering* `WAITING`. A session that
  goes tool → generating → tool flaps `WORKING → WAITING → WORKING`, which is
  the flapping `NOTIFY_COOLDOWN`'s comment already works around.
- **A wrong count.** The section's "N waiting" and the roster's `needs_you`
  count every such session.
- **Minutes, not seconds.** With a long thinking phase, a session reads Done
  for the whole time between the prompt and its first tool call.

Open Island gets it right because it listens to hooks: `UserPromptSubmit`
marks the session running and `Stop` marks it done.

## What already exists

Established by reading the tree before writing this ticket:

- **The hook already records the phases this needs.** `crates/agent-hook/src/lib.rs:551`
  maps `UserPromptSubmit`, `SessionStart`, `PreToolUse`, `PostToolUse` and
  `PostToolUseFailure` to `HookPhase::Active`, and `Stop` and `StopFailure` to
  `HookPhase::Settled`. `HookState::since_ms` timestamps each one.
  `activity_of` reads only `Blocked` and discards the rest. The test
  `no_phase_other_than_blocked_changes_what_the_row_says`
  (`agent_session/tests.rs:203`) locks that in, on the premise that the other
  phases are "values the derivation could have reached on its own". The
  timeline above shows that premise is false.
- **The transcript marks the end of a turn.** It is not parsed for that today:
  `transcript.rs:698` turns `system` records into notices and nothing else.
- **The hook is optional.** On the machine where this was found, Gritty's
  integration was not installed at all. `~/.claude/settings.json` had only Open
  Island's hooks, and the hook state directory did not exist. The transcript
  path is therefore not a nice-to-have: it is the only evidence most installs
  will have.
- **No wire shape change is needed.** The generated protocol types carry no
  doc comments, and every value used below already exists. The rule at
  `crates/gritty-core/src/protocol/mod.rs:23` bumps the version for shape
  changes only, so no bump.

### The transcript corpus (measured, not assumed)

Scratchpad harness (not committed; rebuild it from this description) over this
machine's `~/.claude/projects/*/*.jsonl` modified in the last 30 days: 113
transcripts, Claude Code 2.1.226–2.1.263. A turn runs from a real user prompt
(not `isMeta`, not a sidechain, no `tool_result`, not starting with `<`) to the
next prompt, an interrupt line or end of file. The harness then records which
end-of-turn `system` subtypes appeared inside the turn.

| How the turn ended | Turns | End-of-turn record seen |
|---|---:|---|
| Next prompt | 215 | `system/turn_duration` |
| Next prompt | 2 | `system/stop_hook_summary` only (headless probe sessions) |
| Next prompt | 8 | none; the prompt got no assistant reply before the next one |
| `[Request interrupted by user…]` | 3 | `turn_duration` |
| `[Request interrupted by user…]` | **6** | **none: the interrupt line is the only marker** |
| End of file | 81 | `turn_duration` |
| End of file | 17 | none (live mid-turn, or killed mid-turn) |

So an interrupt has to count as an end of turn in its own right, and
`stop_hook_summary` is a second, weaker end marker.

## Decisions already made

Settled with the product owner before this ticket was written. Do not reopen
them inside the task; record disagreement as a follow-up.

1. **No new `AgentActivity` value.** `WORKING` widens from "a tool call is
   outstanding" to "the agent's turn is in progress": a tool in flight, *or*
   the model generating. `tool` stays present only when a call is outstanding.
2. **`WAITING` with evidence means the turn ended.** Its `activitySource` is
   `HOOK` or `TRANSCRIPT`, and only then does the UI say **Done**, count it as
   needing you, and notify.
3. **`WAITING` without evidence gets a weaker word.** A live process with no
   turn evidence either way (typically `PROCESS_ONLY`, no readable transcript)
   stays `WAITING` with source `PROCESS`, as today, but the UI labels it
   **Alive** ("The process is running; Gritty cannot see whether its turn is
   over"). It does not count as needing you and does not notify.
4. **Both sources, newest wins.** When the hook and the transcript both have
   turn evidence, the one with the later timestamp decides, and a tie goes to
   the hook. This is what stops a hook `ACTIVE` from outliving an interrupt
   that fired no `Stop` (unverified; see *Required behavior* §1). Process
   absence still outranks everything (ADR-018 §7).
5. **`BLOCKED` stays hook-only.** Nothing here gives the transcript a way to
   reach it.

## Required reading

- `docs/decisions/ADR-018-agent-integration-bridge.md`, §6 and §7 and the
  Consequences (being amended)
- `docs/architecture/agent-sessions.md`: the derivation tables (around lines
  72 and 337) and the label mapping (around line 285)
- `docs/tasks/completed/T220-agent-session-discovery.md`,
  `docs/tasks/completed/T221-agent-hook-bridge.md`,
  `docs/tasks/completed/T223-agent-attention.md`
- `crates/gritty-core/src/agent_session/mod.rs` (`activity_of`, `detail_for`),
  `crates/agents/src/transcript.rs` (`parse_transcript`, `is_command_echo`),
  `crates/agents/src/hook.rs` (`HookState`, `is_stale`)
- `apps/desktop/src/features/agents/agent-status.ts` and its callers
- Memory notes: "Validate heuristics on a real corpus" and "Probing Claude Code
  hooks"

## Allowed scope

- `crates/agents/src/transcript.rs`, `crates/agents/src/hook.rs` (reading
  only; the hook binary's phase mapping does not change)
- `crates/agents/src/lib.rs` (`DiscoveredSession` carries the turn mark;
  added when planning, because the merge is where transcript fields reach the
  core)
- `crates/gritty-core/src/agent_session/**`
- `apps/desktop/src/features/agents/**`
- `apps/desktop/src/features/workstreams/agent-links.ts` (a caller of
  `agentStatus`)
- `apps/desktop/src/app.css`: one `.agent-status--alive` rule, so Alive has
  its own mark (T231 §10). A shared file, leased only while it is edited.
- `docs/decisions/ADR-018-agent-integration-bridge.md` (amendment)
- `docs/architecture/agent-sessions.md`
- this task file

## Explicit non-goals

- No new `AgentActivity` or `AgentActivitySource` value, and no protocol shape
  change.
- No change to the hook binary (`crates/agent-hook`), which events it
  registers, or how it is installed. Installing the integration stays a user
  action in Settings.
- No change to `agent_run` (runs Gritty owns have their own state machine) or
  `agent-fake`.
- No change to the idle threshold (`DEFAULT_IDLE_AFTER`, 5 min) or the poll
  interval.
- No change to the timeline follower (`transcript::follow`).
- No other providers. Codex et al. get turn evidence when they get an adapter.

## Required behavior

1. **Probe before coding.** Use the pty harness from the "Probing Claude Code
   hooks" memory to find out whether Claude Code fires `Stop` (or
   `StopFailure`) when a turn is interrupted with Esc, and what an API error
   or rate-limit turn leaves in the transcript. Record both answers in the
   completion report and the ADR amendment. Decision 4 is designed to be
   correct either way; the probe is there to prove it.
2. **Transcript turn evidence.** The scanner's fold records the *latest
   turn-boundary record* in the tail, with its kind and timestamp:
   - **In progress:** a user prompt (not `isMeta`, not a sidechain, not a
     `tool_result`, not a command echo or local-command output, as
     `is_command_echo` already defines), a `tool_result`, or an assistant
     record of any content.
   - **Ended:** `system/turn_duration`, `system/stop_hook_summary`, or a user
     record whose text starts `[Request interrupted by user`.
   - Everything else (attachments, `queue-operation`, `ai-title`,
     `last-prompt`, `mode`, file-history, other `system` subtypes) is not a
     boundary.
   - A tail with no boundary record yields no turn evidence.
3. **Hook turn evidence.** A non-stale `Active` phase is *in progress* and a
   non-stale `Settled` phase is *ended*, each at its `since_ms`. `Ended` keeps
   its current meaning (none: the process table decides).
4. **Derivation**, in this precedence:

   | Value | Evidence | `activitySource` |
   |---|---|---|
   | `ENDED` | no live process, or process table unreadable | `PROCESS` |
   | `IDLE` | live, transcript quiet past the idle threshold | `TRANSCRIPT` |
   | `BLOCKED` | live, hook phase `BLOCKED`, not stale | `HOOK` |
   | `WORKING` | live, a `tool_use` with no `tool_result` | `TRANSCRIPT` |
   | `WORKING` | live, newest turn evidence is *in progress* | whichever source it came from |
   | `WAITING` | live, newest turn evidence is *ended* | whichever source it came from |
   | `WAITING` | live, no turn evidence | `PROCESS` |

5. **Labels.** `agentStatus` takes the activity *and* its source. `WAITING`
   from `HOOK`/`TRANSCRIPT` is Done; from `PROCESS` it is Alive (decision 3),
   with its own tone. Every caller passes the source; none keeps a second
   mapping.
6. **Needs you.** `needs_you` (the roster count, the attention edge in
   `attention.rs`, the section's "waiting" count) is true for `BLOCKED`, and
   for `WAITING` only when its source is not `PROCESS`. It moves from
   `AgentActivity` to something that sees the source; the pure attention
   policy stays pure.
7. **Docs.** Amend ADR-018: hook phases now inform `WORKING`/`WAITING` as
   evidence, `BLOCKED` stays hook-only, the integration stays optional, and
   record the probe results. Update both derivation tables and the label
   mapping in `agent-sessions.md`, and the doc comments on
   `AgentActivity::Working`, `AgentActivity::Waiting` and
   `AgentActivitySource::Hook` ("Only ever the source of `BLOCKED`" is no
   longer true).

## Failure behavior

- **A turn ends with no marker at all** (a Claude Code version that stops
  writing `turn_duration`, or a turn shape the corpus did not contain): the row
  reads Working until the idle threshold, then Idle. It is bounded at 5 minutes
  and never claims Done. Name this in the known limitations.
- **Hook `ACTIVE` with no later `Stop`** (interrupt, crash): a newer transcript
  end marker wins (decision 4); otherwise `is_stale` drops the phase after the
  idle threshold.
- **Process killed mid-turn:** `ENDED`, unchanged.
- **Unreadable or oversized transcript, malformed timestamps:** no turn
  evidence from that source, never a guess. A record with an unparseable
  timestamp is not a boundary.
- **Clock skew between hook and transcript:** both are written on the same
  machine. A backwards clock yields age 0 (existing `HookState` rule), and a tie
  goes to the hook.

## Acceptance criteria

- The timeline above, replayed as a fixture, reads `WORKING` at every poll from
  the prompt until `turn_duration`, then `WAITING`/`TRANSCRIPT` (Done).
- Without the hook installed, a live Claude Code session that is thinking reads
  Working, and flips to Done within one poll of the reply finishing.
- An interrupted turn reads Done, with or without the hook.
- A session that goes tool → generating → tool raises no attention notification
  and never enters the "waiting" count.
- A `PROCESS_ONLY` session reads Alive, not Done, and does not notify.
- `BLOCKED` remains unreachable without hook evidence (the existing test passes
  untouched).
- The corpus harness, rerun on the implementer's machine, reports the share of
  ended turns the new rule classifies as ended. That number is in the completion
  report.

## Tests required

- `activity_of` table test covering every row of §4, including each ordering
  of hook versus transcript evidence (hook `ACTIVE` newer than transcript end →
  `WORKING`/`HOOK`; transcript interrupt newer than hook `ACTIVE` →
  `WAITING`/`TRANSCRIPT`; hook `SETTLED` → `WAITING`/`HOOK`; stale hook phases
  ignored; tie → hook).
- Replace `no_phase_other_than_blocked_changes_what_the_row_says` with a test
  that says what `ACTIVE` and `SETTLED` now do.
- `transcript.rs` fold tests: each boundary kind; sidechain, `isMeta`,
  command-echo and local-command-output prompts ignored; a tail that starts
  mid-turn; a tail with no boundary.
- Attention: no notification for `WAITING`/`PROCESS`; the tool → generating →
  tool regression.
- `agent-status.ts`: Done versus Alive by source; the section's waiting count.
- Manual, scoped to this feature: in `pnpm dev`, send a Claude Code session a
  prompt that thinks for a while. The row stays Working until the reply lands,
  then Done, matching Open Island. Repeat with an Esc interrupt.

## Commands to run

```
cargo test -p gritty-core
cargo test -p gritty-agents
pnpm check
```

## Completion report

### Summary

A session reads **Working** for its whole turn — including the model thinking
or writing, which leaves nothing in the transcript — and **Done** only once a
hook or the transcript saw the turn end. A live session with no turn evidence
reads **Alive**: not counted, not notified, sorted with running work. Hook
`ACTIVE`/`SETTLED` phases now inform the verdict; newest evidence wins; a newer
transcript end discards a stale `BLOCKED`. No wire shape change.

### Files changed

- `crates/agents/src/transcript.rs`: `TurnState`, `TurnMark`, the boundary
  classifier (`turn_boundary`, `user_turn_boundary`, `prompt_boundary`), the
  fold latching the newest mark with its record timestamp, and an end clearing
  outstanding calls. 11 tests.
- `crates/agents/src/lib.rs`: `DiscoveredSession.turn`, re-exports.
- `crates/gritty-core/src/agent_session/mod.rs`: `activity_of` (the precedence
  table), `newest_turn`, decay of in-progress marks, `BLOCKED` discarded by a
  newer end, roster count and rank via `needs_you`.
- `crates/gritty-core/src/agent_session/wire.rs`: `AgentSession::needs_you`
  replaces `AgentActivity::needs_you`; doc comments on `Working`, `Waiting`,
  `AgentActivitySource`.
- `crates/gritty-core/src/agent_session/attention.rs`: the edge compares
  `(activity, needs_you)`. 2 tests.
- `crates/gritty-core/src/agent_session/tests.rs`: derivation, ranking, count
  and manager tests replaced or added (see below).
- `apps/desktop/src/features/agents/agent-status.ts`: `agentStatus` takes the
  source; Alive; `sessionNeedsYou`, `turnEnded`.
- `agent-status-badge.tsx`, `agent-console.tsx`, `agent-timeline.tsx`,
  `agents-panel.tsx`, `roster-model.ts`, `agents-reducer.ts`,
  `workstreams/agent-links.ts`: pass the session, use the shared rule.
- Tests: `agent-status.test.ts`, `roster-model.test.ts`,
  `agents-reducer.test.ts`, `agents-section-render.test.ts`.
- `apps/desktop/src/app.css`: `.agent-status--alive` (half-filled disc).
- `docs/decisions/ADR-018-agent-integration-bridge.md`: AGENT-TURN-END amendment.
- `docs/architecture/agent-sessions.md`: both derivation tables, the boundary
  table, the label mapping, the notification diagram.

### Tests actually executed, and results

- `cargo test -p gritty-agents`: 120 passed (before formatting; see `pnpm check`).
- `cargo test -p gritty-core --lib agent_session`: 112 passed.
- `vitest run src/features/agents src/features/workstreams`: 294 passed, then
  the render test file again, 23 passed. `tsc --noEmit`: clean.
- **Mutation check**, 9 mutations of the new rules (interrupt not closing the
  call, `stop_hook_summary` ignored, interrupt not an end, tagged CLI text
  starting a turn, tie to the transcript, newer end never clearing `BLOCKED`,
  no decay, Alive needing you, Alive ranking with needs-you): **all 9 killed**,
  files restored and re-verified.
- **Probe (§1)**, pty-driven Claude Code 2.1.270 with logging hooks:
  Esc during generation fires **no `Stop`** and writes no `turn_duration`, only
  the interrupt line; Esc at a Bash permission prompt fires **no hook after
  `PermissionRequest`**, and the transcript gets an error result, the interrupt
  and `turn_duration`. Failed and rate-limited turns in the corpus all wrote
  `turn_duration` (41 of 41). Full table in the ADR-018 amendment.
- **Live**, the real `transcript::scan` polled every 250 ms beside the same
  probe: Working 0.3 s after the prompt and held through 7 s of generation
  (the old rule said Done throughout); Done 0.2 s after `Stop`, 0.4 s after an
  Esc during generation, 0.3 s after an Esc at the permission prompt.
- **Corpus replay**, the real scanner after every record of 114 transcripts
  (30 days, 62 885 cuts), scored against an oracle that ignores end markers
  (did the model write again before the next human input?), time-weighted:
  in progress but shown Done **56.4 h → 1.3 h**; ended but shown Working 0.2 h;
  Alive 4.7 h, of which ≈3 h is sessions before their first prompt. Most of the
  remaining "Done while in progress" is the oracle, not the classifier: a turn
  that ended, then an autonomous resume (a dequeued `<task-notification>` or a
  peer message delivered as a meta prompt) with no human record.
- **Visual**: the real `AgentStatusBadge` SSR-rendered with the real `app.css`
  and tokens, both themes, screenshotted — six statuses, six distinct marks.
- `pnpm check` (second run, after `rustfmt` on this ticket's three Rust files):
  Biome, `tsc`, vitest **1319 passed**, `cargo fmt --check`, `clippy -D
  warnings`, `cargo test --workspace` **1215 passed, 0 failed** — and
  **`check:generated` failed**: ts-rs carries *field* doc comments, so the
  reworded `AgentRoster.needs_you` comment regenerates
  `packages/protocol/src/generated/AgentRoster.ts` (comment only, no shape).
  That path is inside T310-ANSWER's (arm the host permission verdict for an
  owned run) lease. With that session's explicit OK, the one file was leased
  and regenerated by the `export_bindings` test: `AgentRoster.ts` is the only
  generated file that differs, comment only, and it ships with this ticket so
  `check:generated` is clean once committed. The ticket's claim "the generated
  types carry no doc comments" was wrong for fields.

### Architectural decisions

- No new wire value: `WAITING`'s meaning depends on `activitySource`, and the
  one rule that reads both lives in `AgentSession::needs_you`, mirrored once in
  `sessionNeedsYou`.
- Turn marks carry the record's timestamp (the scanner otherwise uses mtime),
  because newest-wins compares them with the hook's `since_ms`; the existing
  strict `parse_timestamp_ms` is reused, no dependency added.
- **Added during implementation, beyond the ticket's text:** an in-progress
  transcript mark decays after the idle threshold, and an end does not. Without
  it the ticket's own failure-behaviour promise ("bounded at 5 minutes") did not
  hold, because non-model records keep the file's mtime fresh and `IDLE` never
  fires. Recorded in ADR-018 and `agent-sessions.md`.
- `BLOCKED` stays hook-only; the transcript may only discard it, with a *newer
  end*.

### Known limitations

- A turn started by a slash command or skill (`<command-name>` plus a meta
  prompt) or by a dequeued notification reads its previous state until the
  model's first record — seconds, not minutes, in the corpus.
- A session before its first prompt reads Alive rather than Done.
- Without the hook, a Stop hook that runs long (another tool's) keeps a finished
  turn at Working until `stop_hook_summary` is written.
- A model thinking for more than 5 minutes with nothing written reads Alive
  (in-progress mark decayed) or Idle, as before.

### Follow-up tasks

- `apps/desktop/src/features/agents/agent-notification-setting.tsx` has had no
  importer since T120-A (make the editor and terminal obey their settings)
  deleted the popover that mounted it — delete it or mount it, in its own
  ticket.
- `is_command_echo` does not cover `<bash-input>`/`<bash-stdout>`, so a `!ls`
  can still become a row's "last prompt" (display only; turn detection already
  ignores it).
- Consider a `queue-operation` `dequeue` as a turn start, measured first: the
  corpus shows it precedes autonomous resumes.
- Gritty's own hook integration is not installed on the machine this was found
  on; the Settings page offers it.
