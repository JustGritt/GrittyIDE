# Task BOARD-STALE-REASON — A block reason outlives its block, and show prints it as present tense

- **Phase:** tools
- **Status:** review
- **Depends on:** –

## Objective

`agent-board.py block` writes a ticket's `reason`, and nothing ever retires it.
`list` printed it in parentheses under every status and `show` printed it
underneath the current one, so a wall that had come down read as a wall still
standing. On 2026-09-14 the live board showed stale reasons under TODO (T101-D,
the diff as an editor tab; T102-U, staging and the commit box), REVIEW (T120-M,
mount the settings tab) and DONE (T100-U, T102-C, T102-R, T130-C, T130-M). A
`note` goes stale the same way: it is replaced only by the next note, and
nothing says when it was written.

Alexis chose the fix on 2026-09-14: **keep the reason, and label it** —
nothing is erased, and whatever is printed says when it was true.

## Dependencies

None. The board log already stamps every event with `ts`.

## Required reading

- `AGENTS.md`, *Picking up work*
- `scripts/agent-board.py`: `resolve()`, `render_view()`, `cmd_show()`
- `scripts/test_agent_lease.py` (the harness shape this task's tests copy)

## Allowed scope

- `scripts/agent-board.py`
- `scripts/test_agent_board.py` (new)
- this file

## Explicit non-goals

- No change to the log format. The fold derives everything from events that
  already exist.
- No change to `render --index` or `AGENT-BOARD.md`; neither prints a reason.
- Not fixing the misuse of `block` as a note channel ("Not blocked; recorded
  so…"). The label makes that misuse legible, which is enough.
- Not fixing `drop --to done`, which skips the reviewer rule `move` enforces —
  filed as a follow-up.

## Required behavior

1. The fold keeps `reason_ts` (the block event's `ts`) and `reason_live`.
   `block` sets it live; any event that sets a status other than `blocked`
   (`move`, `drop`, `take`, `add --status`) retires it. `meta` and
   `session-end` leave it alone. A `move --to blocked` without a reason does
   not revive an old one.
2. `list` prints `(reason)` only while the reason is live.
3. `show` prints a live reason plainly, and a retired one as
   `(history: blocked <ts>, since unblocked) <reason>`.
4. The fold keeps `note_ts` and `note_status` beside `note`; `show` prints
   `(<ts>, moving to <status>) <note>`.

## Failure behavior

An event without `ts` (none exist, but the log is hand-editable) prints `?` in
place of the stamp rather than failing.

## Acceptance criteria

- On the live board, `show T101-D` labels its T310-U wall as history and
  `list` no longer prints it; `list` still prints CI-LINUX-DARK's live reason.
- Each behavior above is covered by a test that fails when the behavior is
  removed.

## Tests required

`scripts/test_agent_board.py`: the fold on synthetic events (live while
blocked; retired by `move`, `take`, `drop` and `add --status`; not revived by a
reasonless `move --to blocked`; kept live across `meta`; restamped by a second
block; a note's stamp and status) and `list`/`show` driven as a subprocess
against a scratch git repository whose log is written by hand.

## Commands to run

```
python3 -m unittest scripts/test_agent_board.py scripts/test_agent_lease.py
```

Neither is part of `pnpm check`, which runs no Python.

## Completion report

- **Summary.** A block reason and a note now say when they were true. The fold
  tracks `reason_live`, `reason_ts`, `note_ts` and `note_status`; `list` prints
  only live reasons; `show` labels a retired reason as history and stamps every
  note with its time and the status it was written with.
- **Files changed.** `scripts/agent-board.py`, `scripts/test_agent_board.py`
  (new), this file.
- **Tests actually executed.**
  `python3 -m unittest scripts/test_agent_board.py scripts/test_agent_lease.py`
  — 26 tests, OK (9 new, 17 existing).
  Mutation check on scratch copies of the script, one change each, all
  caught: `list` printing every reason again (1 failure); `set_status` no
  longer retiring the reason (7 failures); `show` printing every reason
  plainly (1 failure).
  Live board: `show T101-D` prints
  `reason   (history: blocked 2026-09-08T22:01Z, since unblocked) …` and
  `note     (2026-09-09T09:19Z, moving to todo) Unblocked: …`; `list --status
  todo` no longer prints the stale walls under T101-D and T102-U.
- **Architectural decisions.** Liveness is a flag the fold computes, not a
  status comparison: a ticket blocked, unblocked and then moved back to
  `blocked` without a reason must not present the first wall as current.
- **Known limitations.** A `move --to blocked --note …` on a ticket that is
  already blocked retires nothing (the status does not change), so the live
  reason stays live — intended, but it means the note and the reason can both
  be current.
- **Follow-up tasks.**
  - `BOARD-DROP-DONE` — `drop --to done` skips the `--reviewed-by` guard that
    `move --to done` enforces (`cmd_drop`), so an agent can close its own
    ticket by handing it back.
