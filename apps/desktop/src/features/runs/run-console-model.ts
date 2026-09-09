import type { AgentRun, AgentRunEvent, AgentRunState } from '@gritty/protocol';
import { isTerminalRun, type RunLog } from './runs-reducer';

/**
 * What an owned run's console draws, and what its prompt box is allowed to do
 * (T310-U, §§6 and 9).
 *
 * Pure, so the whole of it — which rows a ring produces, how a loss is drawn,
 * what an unknown event becomes, and every state in which the box is refused
 * and why — is tested without React, a core or a DOM.
 *
 * Two rules it inherits rather than re-decides:
 *
 * 1. **The feed is the ring, not a transcript** (§6). The child writes an
 *    ordinary Claude Code transcript and repointing T230's follower at it would
 *    give a richer feed for free; that is rejected, because the console and the
 *    run's own state could then disagree — the inspectability failure the run
 *    machine exists to prevent. What is drawn here is what the supervisor
 *    recorded, and nothing else.
 * 2. **A loss the core reported is drawn, never repaired.** `dropped` becomes a
 *    row. Splicing the events either side of a gap into one conversation would
 *    be the UI inventing a continuity the core deliberately refused to claim.
 *
 * Every string here arrived from an agent through the core, which bounded and
 * whitespace-collapsed it at the ingest seam (hard rule 13). Nothing in this
 * file parses, resolves, concatenates or interprets one; the component renders
 * each into a text node and nowhere else.
 */

/**
 * Which class a row draws with.
 *
 * Deliberately the vocabulary `agent-timeline-model.ts` already uses, and the
 * classes `.agent-feed__row--*` already style: the two feeds show different
 * sources and there is no reason for them to *look* different. `thinking` and
 * `result` are absent because a run's event stream has neither — partial and
 * per-tool detail are what §6 says the coarse feed costs.
 */
export type RunFeedTone =
  | 'gap'
  | 'prompt'
  | 'assistant'
  | 'tool'
  | 'file'
  | 'notice'
  | 'permission'
  | 'unknown';

/** How a command ended, or that it is still outstanding. */
export type RunFeedOutcome = 'running' | 'ok' | 'fail';

/** One drawable line of an owned run's feed. */
export interface RunFeedRow {
  /** Stable across renders: the event's `seq`, or a fixed id for the gap. */
  readonly key: string;
  readonly tone: RunFeedTone;
  /** The short word in front of the row. Never empty. */
  readonly label: string;
  /** The row's body. Always plain text, always rendered as a text node. */
  readonly text: string;
  /** A quieter second line: a command's exit, an ask's detail. */
  readonly detail: string | null;
  /** When the core recorded it. Never `null` — the core stamps every record. */
  readonly at: number;
  /** Command rows only, so the row can carry its outcome down its left edge. */
  readonly outcome: RunFeedOutcome | null;
}

/** How much of an event from a newer core is shown, before it is clipped. */
const MAX_UNKNOWN_EVENT = 2000;

function plain(): Pick<RunFeedRow, 'detail' | 'outcome'> {
  return { detail: null, outcome: null };
}

/**
 * What one event says.
 *
 * Total over `AgentRunEvent` by exhaustion *and* by fallthrough, exactly as
 * `describeEvent` is: the `switch` covers every variant this build knows, and
 * anything else — a variant a newer core added — becomes an `unknown` row
 * showing what arrived. An event never vanishes for not being understood.
 *
 * `COMMAND_STARTED` and `COMMAND_FINISHED` are two rows, not one paired row.
 * The timeline pairs a tool call with its result because both carry a
 * `toolUseId`; a run's command events carry only the command text, and pairing
 * on that would attach the wrong result the first time an agent runs `cargo
 * test` twice. Two honest rows beat one that is sometimes a lie.
 */
function describe(event: AgentRunEvent): Omit<RunFeedRow, 'key' | 'at'> {
  switch (event.type) {
    case 'RUN_STARTED':
      return { tone: 'notice', label: 'Run', text: 'Started.', ...plain() };
    case 'RUN_FINISHED':
      return {
        tone: 'notice',
        label: 'Run',
        text: 'Finished.',
        ...plain(),
        detail: event.summary ?? null,
      };
    case 'PROMPT_STARTED':
      // The user's own words, as the *run* confirmed them. Nothing puts a row
      // here on the keystroke (§9): this is what `--replay-user-messages` is
      // passed for, and it is the only thing that draws what was typed.
      return { tone: 'prompt', label: 'You', text: event.text, ...plain() };
    case 'PROMPT_FINISHED':
      return {
        tone: 'assistant',
        label: 'Agent',
        // A turn that ended with no text still happened; saying so beats a
        // blank row that reads as a rendering fault.
        text: event.text ?? 'Answered with nothing to show.',
        ...plain(),
      };
    case 'FILE_READ':
      return { tone: 'file', label: 'Read', text: event.path.display, ...plain() };
    case 'FILE_WRITE_REQUESTED':
      return { tone: 'file', label: 'Writing', text: event.path.display, ...plain() };
    case 'FILE_WRITTEN':
      return { tone: 'file', label: 'Wrote', text: event.path.display, ...plain() };
    case 'COMMAND_STARTED':
      return {
        tone: 'tool',
        label: 'Ran',
        text: event.command,
        ...plain(),
        detail: event.cwd?.display ?? null,
        // Reads as *still running*, which is the honest state of a command
        // whose `COMMAND_FINISHED` has not arrived.
        outcome: 'running',
      };
    case 'COMMAND_FINISHED':
      return {
        tone: 'tool',
        label: event.ok ? 'Done' : 'Failed',
        text: event.command,
        ...plain(),
        // Not derivable from `ok`: a command killed by a signal has no exit
        // code, and saying "exit 0" for one would be an invention.
        detail: event.exitCode === undefined ? null : `exit ${event.exitCode}`,
        outcome: event.ok ? 'ok' : 'fail',
      };
    case 'PERMISSION_REQUESTED':
      return {
        tone: 'permission',
        label: 'Asks',
        text: event.tool,
        ...plain(),
        detail: event.detail ?? null,
      };
    case 'DECISION_REQUESTED':
      return { tone: 'permission', label: 'Asks', text: event.question, ...plain() };
    case 'RUN_BLOCKED':
      return { tone: 'notice', label: 'Blocked', text: event.reason, ...plain() };
    case 'RUN_RESUMED':
      return { tone: 'notice', label: 'Run', text: 'Resumed.', ...plain() };
    case 'RUN_FAILED':
      return {
        tone: 'notice',
        label: 'Failed',
        text: event.error.message,
        ...plain(),
        detail: event.error.code,
      };
    default:
      return {
        tone: 'unknown',
        label: 'Unrecognised',
        // Shown as it arrived rather than mapped onto a variant this build
        // would be guessing at.
        text: serialize(event),
        ...plain(),
      };
  }
}

/** An event this build does not know, clipped to something a row can hold. */
function serialize(event: unknown): string {
  let text: string;
  try {
    text = JSON.stringify(event) ?? String(event);
  } catch {
    text = String(event);
  }
  return text.length > MAX_UNKNOWN_EVENT ? `${text.slice(0, MAX_UNKNOWN_EVENT)}…` : text;
}

/**
 * The rows the feed draws, oldest last.
 *
 * The gap comes first because it describes the *head* of the ring: the events
 * that are missing are the oldest ones.
 */
export function runFeedRows(log: RunLog): readonly RunFeedRow[] {
  const rows: RunFeedRow[] = [];
  if (log.dropped > 0) {
    rows.push({
      key: 'gap:dropped',
      tone: 'gap',
      label: 'Gap',
      text: `${log.dropped} earlier ${
        log.dropped === 1 ? 'event' : 'events'
      } dropped: the run outgrew the number the core keeps.`,
      detail: null,
      at: 0,
      outcome: null,
    });
  }
  for (const record of log.events) {
    rows.push({ key: `event:${record.seq}`, at: record.at, ...describe(record.event) });
  }
  return rows;
}

/**
 * Whether the prompt box may be used, and — when it may not — why.
 *
 * The six edges below are `AgentRunManager::prompt`'s own, in its own order, so
 * the box refuses exactly what the core would refuse and says the same thing
 * about it. It is a *mirror*, not a second policy: the authority is
 * `AgentRunState::accepts_input` in `crates/gritty-core/src/agent_run/state.rs`,
 * whose doc comment names this box as the surface that disables against it. The
 * mirror exists because the wire carries no `acceptsInput` flag, and the same
 * argument that puts `MAX_RUN_EVENTS`, `isTerminalRun` and `needsAttention` in
 * `runs-reducer.ts` puts this here.
 *
 * A refusal is never silent and never optimistic in the other direction: an
 * unrecognised state from a newer core is refused, because letting a user type
 * into a run this build cannot vouch for is the worse failure.
 */
export type PromptGate =
  /** There is a channel and it is open. */
  | { readonly kind: 'ready' }
  /** There is a channel; it is shut right now, and the row says why. */
  | { readonly kind: 'refused'; readonly reason: string }
  /**
   * There is no channel at all, so nothing is drawn — not even disabled.
   *
   * §9's rule, applied to the other lane it names: an observed session's
   * console gets no prompt box *and no disabled one either*, because a
   * greyed-out control implies a permission the user could acquire. A scripted
   * run is the same case — `runs.prompt` refuses it with "a FAKE run has no
   * prompt channel", and no waiting will open one.
   */
  | { readonly kind: 'absent' };

const ACCEPTS_INPUT: readonly AgentRunState[] = ['RUNNING', 'WAITING'];

export function promptGate(run: AgentRun): PromptGate {
  if (run.provider === 'FAKE') return { kind: 'absent' };
  if (isTerminalRun(run.state)) {
    return {
      kind: 'refused',
      reason: 'This run has ended. Another turn would have to be another run.',
    };
  }
  if (run.cancellationRequested) {
    return { kind: 'refused', reason: 'This run is stopping, so it cannot take another turn.' };
  }
  if (ACCEPTS_INPUT.includes(run.state)) return { kind: 'ready' };
  switch (run.state) {
    case 'STARTING':
      // The prompt `runs.start` was given is seeded into the inbox before the
      // run exists, so nothing is lost by refusing here — and a prompt queued
      // for a child that then never came up would be invisible.
      return {
        kind: 'refused',
        reason: 'The run is still starting; there is nothing to say it to.',
      };
    case 'BLOCKED':
    case 'NEEDS_DECISION':
      // One stop at a time: the answer to a block is a verdict, not a prompt.
      // T310-ANSWER is what will put that verdict here.
      return {
        kind: 'refused',
        reason: 'The run is stopped on something a prompt does not answer.',
      };
    case 'REVIEW':
      return { kind: 'refused', reason: 'The work is waiting to be looked at; it is your turn.' };
    default:
      return {
        kind: 'refused',
        reason: 'This run is in a state reported by a newer core than this build understands.',
      };
  }
}
