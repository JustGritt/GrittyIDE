import type { AgentActivity, AgentActivitySource, AgentProvider } from '@gritty/protocol';

/**
 * How the renderer says what an agent session is doing (T231 §8/§9).
 *
 * One mapping for the whole UI — the rail and the island's reduced row both
 * come through here — so the word the user reads and the colour beside it
 * cannot drift apart between two surfaces.
 *
 * The wire value keeps T220's names. `WAITING` means *the user's move*, which
 * the rail says as **Done**: the turn finished. Renaming it on the wire would
 * ripple through the core for a word, so the translation lives here instead.
 */
export interface AgentStatus {
  /** What the row says, in words, because colour alone is not a status (§10). */
  readonly label: string;
  /**
   * Suffix of the `agent-status--*` class. The class is where the ADR-016
   * token and the marker's *shape* live, so the two travel together and the
   * rail stays readable without colour vision.
   */
  readonly tone: AgentStatusTone;
  /** Longer form for the row's tooltip; the label is what is drawn. */
  readonly hint: string;
}

export type AgentStatusTone = 'working' | 'blocked' | 'done' | 'idle' | 'ended' | 'unknown';

/**
 * Total over `AgentActivity` by construction: a value added to the wire fails
 * this file to compile rather than rendering as a blank row.
 */
const STATUSES: Record<AgentActivity, AgentStatus> = {
  WORKING: {
    label: 'Working',
    tone: 'working',
    hint: 'A tool call is outstanding.',
  },
  /**
   * The fifth status of §8, and the one T231 wrote down before the core could
   * produce it.
   *
   * `BLOCKED` needs hook evidence (T221, ADR-018): poll-only discovery cannot
   * tell a pending permission prompt from a finished turn, so until the agent
   * itself reported one there was nothing to draw. Now there is, and the label
   * and colour that were waiting here are what it lands as.
   */
  BLOCKED: {
    label: 'Pending validation',
    tone: 'blocked',
    hint: 'The agent is waiting for you to approve something.',
  },
  WAITING: {
    label: 'Done',
    tone: 'done',
    hint: 'The turn finished — your move.',
  },
  IDLE: {
    label: 'Idle',
    tone: 'idle',
    hint: 'Alive, but the transcript has been quiet.',
  },
  ENDED: {
    label: 'Ended',
    tone: 'ended',
    hint: 'No live process for this session.',
  },
};

/**
 * Widened for the lookup: the protocol is additive, so a newer core can send
 * an activity this build's union does not contain (`agents-reducer` makes the
 * same allowance for the same reason).
 */
const LOOKUP: Readonly<Record<string, AgentStatus | undefined>> = STATUSES;

/** The status to draw for an activity. Never blank, never invented. */
export function agentStatus(activity: AgentActivity): AgentStatus {
  return (
    LOOKUP[activity] ?? {
      // Shown as it arrived rather than mapped onto a status that would be a
      // claim about a value this build does not understand.
      label: activity,
      tone: 'unknown',
      hint: 'Reported by a newer core than this build understands.',
    }
  );
}

/**
 * Where a row's verdict came from, in the user's words (T221 §24).
 *
 * Three sources can disagree, so a row has to be able to say why it claims what
 * it claims — otherwise "Pending validation" is magic, and the one status the
 * user is meant to act on is the one they have least reason to trust.
 */
const SOURCES: Record<AgentActivitySource, string> = {
  PROCESS: 'from the process table',
  TRANSCRIPT: 'from the transcript',
  HOOK: 'reported by the agent itself',
};

export function activitySourceLabel(source: AgentActivitySource): string {
  return (SOURCES as Record<string, string | undefined>)[source] ?? 'from an unknown source';
}

/** Which CLI produced a session, in the user's words. */
const PROVIDERS: Record<AgentProvider, string> = {
  CLAUDE_CODE: 'Claude Code',
};

export function providerLabel(provider: AgentProvider): string {
  return (PROVIDERS as Record<string, string | undefined>)[provider] ?? provider;
}
