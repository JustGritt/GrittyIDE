# Task TURN-START-QUEUED — A dequeued message or a `!` command starts a turn

- **Phase:** maintenance (implements TURN-START-DEQUEUE's recommendation)
- **Status:** review
- **Depends on:** TURN-START-DEQUEUE (measure whether a dequeued message should
  start a turn), AGENT-TURN-END (the agent roster says Done while the model is
  still generating). Both are on `main`.

## Objective

The transcript's turn classifier gains two in-progress boundaries:

1. `{"type":"queue-operation","operation":"dequeue"}`: the CLI handed queued
   input to the model.
2. User text starting `<bash-input>`: the user ran a `!` shell command, whose
   output the model answers.

Accepted by the product owner on 2026-09-14, on TURN-START-DEQUEUE's numbers.

## Evidence (from TURN-START-DEQUEUE, rechecked here)

| Candidate start | In progress but shown Done, removed | Ended but shown Working, added |
|---|---:|---:|
| dequeue | 1,082 s | 15 s |
| `<bash-input>` | 131 s | 7 s |

- The gain from a dequeue is queue-delivered **meta** input, which
  `turn_boundary` ignores: `Another Claude session sent a message` (14 in the
  corpus), `[Cross-session idle notice]` (1) and `usage limit has reset.
  Continue` (3). All 18 were answered by the model.
- The one `!` command counted as "no reply" was interrupted with Esc 7 s after
  its output landed, so the model was probably already generating. Effectively
  11 of 11 were answered.
- A dequeue in front of a queued slash command, which would read Working with no
  turn: the one corpus candidate (`/effort` next to a dequeue) turned out to be a
  dequeued task notification that the model answered. No instance was found.

## Allowed scope

- `crates/agents/src/transcript.rs`: `turn_boundary`, `prompt_boundary` and
  their doc comments, plus tests
- `docs/architecture/agent-sessions.md`: the boundary table
- `docs/decisions/ADR-018-agent-integration-bridge.md`: one paragraph in the
  AGENT-TURN-END amendment
- this task file

## Explicit non-goals

- Enqueue, `remove` and `popAll` are not boundaries.
- `<bash-stdout>`/`<bash-stderr>` are not boundaries. The input already
  started the turn.
- No pairing of a dequeue with its enqueue's content. That would be a queue model
  inside the fold, for a case the corpus never showed.
- No change to `BLOCKED` (only a newer *end* discards it), the timeline
  follower, the last-prompt preview, or the core's derivation.

## Failure behavior

- A dequeue or `!` command that no model turn follows reads **Working** until
  the next end marker, or for at most the idle threshold (5 min). After that the
  in-progress mark decays to no evidence (Alive, not Done). This is bounded, and
  it never claims Done.
- A dequeue with an unparseable timestamp is not a boundary (existing rule).

## Acceptance criteria

- After an ended turn, a dequeue reads in progress at the dequeue's timestamp,
  and so does a `<bash-input>`.
- Enqueue, remove, popAll, `<bash-stdout>` and `<bash-stderr>` leave the ended
  mark in place.
- Every dequeue and `<bash-input>` cut in the 30-day corpus, run through the
  **real** scanner (`gritty_agents::transcript::scan`), reads in progress at that
  record's timestamp.

## Commands to run

```
cargo test -p gritty-agents
pnpm check
```

## Completion report

### Summary

A `queue-operation` `dequeue` and a `<bash-input>` user record are in-progress
turn boundaries. After a turn ended, a session that the CLI hands a peer's
message, a usage-limit "continue" or a `!` command reads **Working** from that
record, instead of Done until the model's first record lands.

### Files changed

- `crates/agents/src/transcript.rs`: a `queue-operation` arm in
  `turn_boundary`, `<bash-input>` in `prompt_boundary`, and both doc comments
  (the old one claimed a `!ls` almost never starts a turn). Three tests added;
  `the_cli_talking_to_the_user_is_not_a_turn` now lists `<bash-stdout>` and
  `<bash-stderr>` where it listed `<bash-input>`.
- `docs/architecture/agent-sessions.md`: the boundary table.
- `docs/decisions/ADR-018-agent-integration-bridge.md`: a paragraph closing the
  AGENT-TURN-END amendment.
- this task file.

### Tests actually executed, and results

- `cargo test -p gritty-agents`: 125 passed.
- **Mutation check**, 3 mutations: dequeue never a start
  (`a_dequeue_starts_a_turn` fails), every queue operation a start
  (`only_a_dequeue_hands_input_over` fails), `<bash-input>` not a start
  (`a_shell_command_starts_a_turn` fails). **All 3 killed**; the file was
  restored and byte-compared.
- **Real scanner over the real corpus.** A scratchpad binary depending on
  `gritty-agents` by path cut each of the 30-day transcripts right after every
  dequeue and `<bash-input>`, and ran `transcript::scan` on the prefix. **173 cuts
  read in progress at that record's own timestamp, 0 read anything else.** 18 had
  no session to read: 17 are dequeues on line 2–3 of a file, before any record
  carries a `cwd`, and 1 is a cut whose last 128 KB is metadata and ~70 KB
  records without a `cwd`. That is the existing tail-window limit, and it is
  the same with or without this change.
- `pnpm check`: exit 0. Biome (345 files), `tsc`, vitest **1346 passed**,
  `cargo fmt --check`, `clippy -D warnings`, `cargo test --workspace` **1256
  passed, 0 failed**, generated types fresh.

### Architectural decisions

- A dequeue is a start **without** looking at what was dequeued. The dequeue
  record carries no content, and pairing it with its enqueue would put a queue
  model inside the fold for a case (a queued slash command) the corpus never
  showed.

### Known limitations

- A dequeued input that starts no model turn (a queued slash command, if the CLI
  queues those) reads Working until the next end marker or, at most, the idle
  threshold, then Alive rather than Done.
- A `!` command the model does not answer reads Working the same way. The one
  corpus instance was interrupted 7 s in, which ended it.

### Follow-up tasks

None new. `<task-notification>` as a last prompt is already recorded in
LAST-PROMPT-SHELL (a `!` shell command is not a session's last prompt).
