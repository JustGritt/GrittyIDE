# Task TURN-START-DEQUEUE — Measure whether a dequeued message should start a turn

- **Phase:** maintenance (a follow-up of AGENT-TURN-END)
- **Status:** review
- **Depends on:** AGENT-TURN-END (the agent roster says Done while the model
  is still generating), committed on `main`.

## Objective

AGENT-TURN-END's corpus replay left **1.3 h in 30 days** of sessions shown
*Done* while the model was about to write. Its report blamed most of that on
autonomous resumes: a turn that ended, then input the CLI delivered from its
queue with no human prompt record. Its follow-up asked whether a
`{"type":"queue-operation","operation":"dequeue"}` record should count as a
turn **start**, measured first.

This ticket is the measurement and a recommendation. It changes no code.

## Method

A scratchpad harness (not committed; rebuild it from this description). It is a
line-for-line port of `turn_boundary`, `user_turn_boundary` and
`prompt_boundary` as of `c3e478b`, with the 5-minute decay of an in-progress
mark. It ran over every `~/.claude/projects/*/*.jsonl` modified in the last
30 days: 124 transcripts, Claude Code 2.1.226–2.1.270.

The replay runs two rules side by side, cut at every record: the current rule,
and the current rule plus "a dequeue is an in-progress mark at its timestamp".
Wherever they disagree, the interval up to the next record (capped at the idle
threshold) is scored against the same oracle AGENT-TURN-END used: *did the model
write again before the next end marker or human prompt?*

## What a dequeue is

177 `dequeue` records. The state the current rule was in when each one landed,
and the first boundary record after it:

| State at the dequeue | First boundary after it | Count |
|---|---|---:|
| ended | `<task-notification>` user record (~20 ms later) | 117 |
| ended | the model's own record, nothing before it | 18 |
| ended | a real prompt | 3 |
| no mark yet | a real prompt | 18 |
| no mark yet | `<task-notification>` | 1 |
| in progress (absorbed mid-turn) | `stop_hook_summary` / model / tool result | 15 |
| decayed | interrupt / model | 5 |

Most dequeues are followed within milliseconds by a record that already starts a
turn, so counting them changes nothing a 6-second poll can see. The 18
*ended → model* rows are the ones that matter. There the queue delivered a
**meta** user record, which `turn_boundary` deliberately ignores:

- `Another Claude session sent a message: <cross-session-message …>`, a peer
  session's `SendMessage` (the model wrote 10–31 s later), and
- `Your claude.ai usage limit has reset. Continue …` (the model wrote about
  4–5 min later, on 2026-09-08, in three sessions at once).

## Result

| Candidate start | In progress but shown Done, removed | Ended but shown Working, added |
|---|---:|---:|
| `queue-operation` `dequeue` | **1,082 s (0.30 h)** | **15 s** |
| (for comparison) `<bash-input>` | 131 s (0.04 h) | 7 s |

- **About a quarter of the 1.3 h that remains** after AGENT-TURN-END, at a
  72 : 1 benefit-to-cost ratio.
- It is concentrated. About 0.24 h is the single usage-limit-reset event, and
  the rest is peer messages of 10–30 s each. On a machine that never hits a
  usage limit and runs no cross-session messaging, the gain is close to zero.
- The one long "cost" the first cut showed (300 s, a task notification the
  model never answered) is not a cost of this rule. The task notification
  already reads Working under the current rule, so the two rules agree there,
  and the replay no longer counts it.
- A dequeue that lands mid-turn only refreshes an in-progress mark that an end
  marker then closes. It was harmless in all 15 cases.

## Recommendation

**Count a `dequeue` as an in-progress turn mark.** It is the only marker in
front of queue-delivered meta input, it is nearly free, and it fits the
classifier's stated bias ("missing a real start costs only the gap until the
model's first record"). It is one arm in `turn_boundary`, plus tests, one row
in the boundary table in `agent-sessions.md`, and a line in ADR-018's
AGENT-TURN-END amendment.

**Rejected alternative:** treating meta user records that start
`Another Claude session sent a message` as starts. That is narrower, it keys
off prose the CLI may reword, and it misses the usage-limit continue.

**Worth deciding at the same time:** `<bash-input>`. The corpus contradicts
`prompt_boundary`'s doc comment: 10 of 11 `!` commands were answered by the
model. Counting it gains 131 s for 7 s of cost. That is small, but the doc
comment is wrong either way.

## Allowed scope

- this task file (the measurement). An implementation is a separate ticket, once
  the recommendation is accepted.

## Completion report

### Summary

Measured, not implemented. A dequeue as a turn start removes 0.30 h per 30 days
of "Done while in progress" and adds 15 s of "Working after it ended". The
recommendation is yes, as its own ticket.

### Files changed

- `docs/tasks/TURN-START-DEQUEUE-measure.md` (this file)

### Tests actually executed

- The scratchpad replay described above. Two passes: a first cut whose cost
  column wrongly included time the current rule already read Working, and a
  corrected replay that scores only disagreements. The numbers here are from
  the corrected pass.
- No code changed, so no `pnpm check` was needed for this ticket.

### Known limitations

- One machine, one user's habits. The usage-limit event dominates the gain, and
  that event is bursty.
- The oracle ignores end markers, so it inherits AGENT-TURN-END's blind spot: an
  autonomous resume that happens *after* a turn ended counts as "in progress"
  for that whole gap.

### Follow-up tasks

- Implement a `dequeue` as an in-progress mark (the recommendation above).
- Decide `<bash-input>` as a turn start, and correct `prompt_boundary`'s doc
  comment either way.
