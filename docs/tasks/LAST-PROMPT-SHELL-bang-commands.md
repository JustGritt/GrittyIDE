# Task LAST-PROMPT-SHELL — A `!` shell command is not a session's last prompt

- **Phase:** maintenance (a follow-up of AGENT-TURN-END)
- **Status:** review
- **Depends on:** AGENT-TURN-END (the agent roster says Done while the model
  is still generating), committed on `main`.

## Objective

A session's **last prompt** (the roster row's fallback task text, and the body
of an attention notification) should never be a shell command the user ran
with `!` in Claude Code.

## What the corpus says

Measured over this machine's transcripts from the last 30 days: 124 files,
Claude Code 2.1.226–2.1.270. A `!` command reaches `last_prompt` by **two**
routes, not the one AGENT-TURN-END's follow-up named:

| Record | Seen | Why it gets through today |
|---|---:|---|
| user text `<bash-input>…</bash-input>`, `<bash-stdout>…` | 11 + 11 | `is_command_echo` knows `<command-name>` and `<local-command…` only |
| `{"type":"last-prompt","lastPrompt":"! export PATH=…"}` | 67 | written by the CLI with the `!` prefix, and preferred over message text |

Fixing only the tag check would still let the `last-prompt` record, the more
common route, put `! export PATH=…` on the row.

Also measured: **10 of the 11 `!` commands were answered by the model**, 3 to
58 s later, with no prompt in between. Claude Code hands the command's output to
the model. The doc comment on `prompt_boundary` says a `!ls` "almost never
starts a turn", and that is wrong for `!`. This ticket does not act on that. See
the follow-ups.

## Required behavior

- The fold's `last_prompt` ignores user text that starts `<bash-input>`,
  `<bash-stdout>` or `<bash-stderr>`, and `last-prompt` records whose text starts
  `!`. The previous real prompt stands.
- The `!` rule applies only to the `last-prompt` record, where the prefix is
  evidenced. Text typed in the TUI cannot start with `!`, because that key enters
  shell mode and is written as `<bash-input>`, but headless `-p` input could.
- The console timeline is **unchanged**: `is_command_echo` keeps its meaning.
  The model answers a `!` command, so hiding it from the timeline would leave a
  reply with no visible cause.

## Allowed scope

- `crates/agents/src/transcript.rs`
- `docs/architecture/agent-sessions.md` (the `last-prompt` line)
- this task file

## Explicit non-goals

- No change to turn detection (`turn_boundary`, `prompt_boundary`).
- No change to the timeline follower's prompt entries.
- `<task-notification>` as a last prompt is the same kind of wart, but it was
  not asked for. It is recorded as a follow-up.

## Failure behavior

- A tail whose only prompts are `!` commands yields no last prompt. The row
  falls back to the title or the last reply, as it does for a tail with no
  prompt.

## Tests required

- `transcript.rs`: `<bash-input>` and `<bash-stdout>` user text, and a
  `last-prompt` record of `! ls`, after a real prompt, leave the real prompt
  as `last_prompt`.

## Commands to run

```
cargo test -p gritty-agents
pnpm check
```

## Completion report

### Summary

A session's last prompt skips `!` shell commands on both routes: tagged
`<bash-input>`/`<bash-stdout>`/`<bash-stderr>` message text, and a
`last-prompt` record starting `!`. The previous real prompt stands. The console
timeline still shows the command, because the model's reply is about it.

### Files changed

- `crates/agents/src/transcript.rs`: `is_shell_echo`, used by `Fold::set_prompt`;
  the `!` rule on the `last-prompt` arm; two tests.
- `docs/architecture/agent-sessions.md`: the `last-prompt` line.
- this task file.

### Tests actually executed, and results

- `cargo test -p gritty-agents transcript`: 49 passed, including
  `a_shell_command_is_not_the_last_prompt` and
  `a_shell_command_stays_on_the_timeline`.
- **Mutation check**: dropping the `!` rule, and dropping `is_shell_echo` from
  `set_prompt`, each fail `a_shell_command_is_not_the_last_prompt`. The file was
  restored and byte-compared.
- `pnpm check`: run once, on the tree with this
  ticket and NOTIFY-SETTING-ORPHAN (delete the notification switch nothing
  mounts). Exit 0: Biome (345 files), `tsc`, vitest **1346 passed**,
  `cargo fmt --check`, `clippy -D warnings`, `cargo test --workspace` **1253
  passed, 0 failed**, generated types fresh.

### Architectural decisions

- `is_command_echo` is not widened. It also filters the timeline, and the corpus
  shows the model answers `!` commands, so the timeline keeps them. A separate
  predicate keeps the preview rule from leaking into the console.

### Known limitations

- A headless `-p` prompt that genuinely starts with `!` is skipped as a last
  prompt when it arrives through a `last-prompt` record.

### Follow-up tasks

- `<task-notification>…` can still become a last prompt: 124 of them in the
  corpus, the same kind of wart, and not asked for here.
- The corpus contradicts `prompt_boundary`'s doc comment: 10 of 11 `!` commands
  were answered by the model, so a `<bash-input>` starts a turn. Recorded, with
  numbers, in TURN-START-DEQUEUE (measure whether a dequeued message should
  start a turn).
