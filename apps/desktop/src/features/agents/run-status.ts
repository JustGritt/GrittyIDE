import type { AgentRunState } from '@gritty/protocol';
import type { AgentStatusTone } from './agent-status';

/**
 * How the renderer says what a run Gritty **owns** has got to (T233-M).
 *
 * The sibling of `agent-status.ts`, and deliberately built the same way: one
 * total mapping for the whole UI, so the word the user reads and the colour
 * beside it cannot drift apart between two surfaces. What it maps is the other
 * half of `docs/architecture/agent-runs.md`'s table — `AgentRunState`, ten
 * values the core's transition table produced, rather than `AgentActivity`,
 * five values inferred from evidence.
 *
 * It exists because there was no mapping at all. `runs-panel.tsx` wrote
 * `ops__row--${run.state.toLowerCase()}` and inherited the operations panel's
 * four rules, so of ten run states exactly three had a colour and `FINISHED`
 * had none — the operations CSS spells that one `completed`. A dev panel could
 * live with that; a row in the sidebar cannot.
 */
export interface RunStatus {
  /** What the row says, in words, because colour alone is not a status. */
  readonly label: string;
  /** Suffix of the `agent-status--*` class: the ADR-016 token *and* the marker shape. */
  readonly tone: RunStatusTone;
  /** Longer form for the row's tooltip; the label is what is drawn. */
  readonly hint: string;
}

/**
 * The session tones, plus the one a run needs and a session cannot have.
 *
 * A session has no failure state — the roster infers what a process is doing
 * and a process that died is `ENDED`, not failed. A run has `FAILED`, because
 * Gritty started it and is answerable for how it went. Drawing that in the grey
 * `ended` tone would be the UI quietly downgrading the one outcome the user
 * most needs to see, so `failed` is its own tone rather than a reused one.
 *
 * Widening `AgentStatusTone` itself would have made every session row's type
 * admit a value no session can ever hold. This union is the narrower change.
 */
export type RunStatusTone = AgentStatusTone | 'failed';

/**
 * Total over `AgentRunState` by construction: a state added to the wire fails
 * this file to compile rather than rendering as a blank row.
 *
 * The ten states collapse onto six tones, and the collapse is where the
 * judgement is. Three groups:
 *
 * - **In flight** (`working`) — `STARTING`, `RUNNING`. `STARTING` is kept
 *   distinct in words because the state exists precisely to say that a
 *   process being alive is not the same as it being usable (Phase 0 §D).
 * - **Stopped, and it is your move** (`blocked`, `done`) — the four states the
 *   run manager can rest in. `REVIEW` is toned `done` rather than `blocked`:
 *   nothing is being withheld from the run, it finished a unit of work and is
 *   waiting to be looked at, which is the same thing a session's `WAITING`
 *   means and is drawn the same way.
 * - **Terminal** — `FINISHED` reads as a success, the other three do not, and
 *   `INTERRUPTED` is deliberately neither `FAILED` nor `CANCELLED`: "the agent
 *   broke", "we stopped being able to watch it" and "someone asked it to stop"
 *   are three different answers and a user deciding whether to retry needs to
 *   know which.
 */
const STATUSES: Record<AgentRunState, RunStatus> = {
  STARTING: {
    label: 'Starting',
    tone: 'working',
    hint: 'The process exists, but the run is not usable yet.',
  },
  RUNNING: {
    label: 'Working',
    tone: 'working',
    hint: 'A step is in flight.',
  },
  WAITING: {
    label: 'Waiting for you',
    tone: 'blocked',
    hint: 'The run stopped and cannot go on until you answer.',
  },
  NEEDS_DECISION: {
    label: 'Needs a decision',
    tone: 'blocked',
    hint: 'The run asked a question and is holding until it is answered.',
  },
  BLOCKED: {
    label: 'Blocked',
    tone: 'blocked',
    hint: 'The run stopped at something it may not do on its own.',
  },
  REVIEW: {
    label: 'Ready for review',
    tone: 'done',
    hint: 'The work is finished and waiting to be looked at.',
  },
  FINISHED: {
    label: 'Finished',
    tone: 'ended',
    hint: 'The run completed. A second attempt would be a second run.',
  },
  FAILED: {
    label: 'Failed',
    tone: 'failed',
    hint: 'The run itself broke. Its error says how.',
  },
  CANCELLED: {
    label: 'Cancelled',
    tone: 'ended',
    hint: 'You stopped it.',
  },
  INTERRUPTED: {
    label: 'Interrupted',
    tone: 'failed',
    hint: 'Gritty stopped being able to supervise it, so it cannot say how it ended.',
  },
};

/**
 * Widened for the lookup: the protocol is additive, so a newer core can send a
 * state this build's union does not contain (`agent-status.ts` and
 * `runs-reducer.ts` make the same allowance for the same reason).
 */
const LOOKUP: Readonly<Record<string, RunStatus | undefined>> = STATUSES;

/** The status to draw for a run state. Never blank, never invented. */
export function runStatus(state: AgentRunState): RunStatus {
  return (
    LOOKUP[state] ?? {
      // Shown as it arrived rather than mapped onto a status that would be a
      // claim about a value this build does not understand.
      label: state,
      tone: 'unknown',
      hint: 'Reported by a newer core than this build understands.',
    }
  );
}
