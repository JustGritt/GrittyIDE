# Task LAST-PROMPT-NOTIFY — A task notification is not a session's last prompt

- **Phase:** maintenance (a follow-up of LAST-PROMPT-SHELL)
- **Status:** review
- **Depends on:** LAST-PROMPT-SHELL (a `!` shell command is not a session's
  last prompt), on `main`.

## Objective

A session's **last prompt** should never be a `<task-notification>…` record.
That record is a background task reporting back to the agent, not something
the user asked for.

## What the corpus says

The same 30-day corpus: 124 transcripts, Claude Code 2.1.226–2.1.270. The fold
is emulated as of `05b53d1`, after LAST-PROMPT-SHELL.

- **One route only.** 130 `<task-notification>` user message records set
  `last_prompt`. No `last-prompt` record ever starts with `<`: the CLI
  records only what was typed there.
- **It is the last tagged text that gets through.** No other `<…>` value sets
  `last_prompt` any more.
- **Visible, but less than it looks.** `last_prompt` reads
  `<task-notification> <task-id>…` for 14.4 h of 185 h of transcript time, in 31
  sessions. The roster row and the attention notification both prefer the
  agent's title (`agents-reducer.ts:70`, `attention.rs` `alert_for`), so the
  text is on screen only while a session has no `ai-title` yet: **1.1 h**.

## Required behavior

- The fold's `last_prompt` ignores user text that starts `<task-notification>`.
  The previous real prompt stands.
- The console timeline is **unchanged**. The model answers a task notification
  (119 of 121), so the timeline keeps the record that its reply is about. This
  is the same reasoning as for `!` commands.
- Turn detection is unchanged. `<task-notification>` still starts a turn.

## Allowed scope

- `crates/agents/src/transcript.rs`
- `docs/architecture/agent-sessions.md` (the `last-prompt` line)
- this task file

## Explicit non-goals

- No change to `is_command_echo` (it also filters the timeline and system
  notices).
- No change to `turn_boundary` or `prompt_boundary`.
- No rendering of a task notification as something friendlier in the timeline.

## Failure behavior

- A tail whose only user text is task notifications yields no last prompt. The
  row falls back to the title or the last reply.

## Tests required

- A task notification after a real prompt leaves the real prompt as
  `last_prompt`, both as string content and as a text block.
- A task notification still appears on the timeline.
- A task notification still starts a turn (`a_task_notification_starts_a_turn`,
  existing, untouched).

## Commands to run

```
cargo test -p gritty-agents
pnpm check
```

## Completion report

### Summary

A session's last prompt skips `<task-notification>` records, so the previous
real prompt stands. The timeline and turn detection are unchanged. With this
change, no tagged text in the corpus reaches `last_prompt`.

### Files changed

- `crates/agents/src/transcript.rs`: `is_shell_echo` becomes
  `is_cli_delivered` and adds `<task-notification>`; two tests.
- `docs/architecture/agent-sessions.md`: the `last-prompt` line.
- this task file.

### Tests actually executed, and results

- `cargo test -p gritty-agents`: 127 passed, including
  `a_task_notification_is_not_the_last_prompt`,
  `a_task_notification_stays_on_the_timeline`, and the untouched
  `a_task_notification_starts_a_turn`.
- **Mutation check**: removing `<task-notification>` from the predicate fails
  `a_task_notification_is_not_the_last_prompt`. The file was restored and
  byte-compared.
- **Real scanner over the real corpus.** A scratchpad binary cut every 30-day
  transcript right after each of the 130 task-notification records and ran
  `transcript::scan` on the prefix. **0 of 130** reported a last prompt starting
  `<` or `!`. The same run against the mutated source reported **130 of 130**,
  so the check can tell the two apart.
- `pnpm check`: exit 0. Biome (345 files), `tsc`, vitest **1346 passed**,
  `cargo fmt --check`, `clippy -D warnings`, `cargo test --workspace` **1258
  passed, 0 failed**, generated types fresh.

### Architectural decisions

- One predicate for "the CLI delivered this, the user did not ask for it",
  separate from `is_command_echo`. That keeps the preview rule out of the
  timeline, which shows what the model answered.

### Known limitations

- The timeline still shows a task notification's raw tagged text.

### Follow-up tasks

- The timeline could render a task notification or a `!` command as a
  labelled entry rather than raw `<…>` text. Not asked for.
