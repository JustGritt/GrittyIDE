import type { AgentRoster, AgentSession, EventEnvelope } from '@gritty/protocol';

/**
 * The agent roster as the renderer holds it (T220).
 *
 * Pure, so the ordering and the counts are unit-tested without React. There is
 * no local mutation to reconcile here — nothing in the UI can change an agent
 * session, because Gritty does not own these sessions — so the reducer only
 * ever replaces the roster with a newer one.
 */
export interface AgentsState {
  readonly roster: AgentRoster;
  /** False until the first roster arrives, so "none" and "not yet" differ. */
  readonly loaded: boolean;
}

export const initialAgentsState: AgentsState = {
  roster: { sessions: [], needsYou: 0, processEvidence: true },
  loaded: false,
};

export type AgentsAction =
  | { readonly type: 'loaded'; readonly roster: AgentRoster }
  | { readonly type: 'event'; readonly event: EventEnvelope };

export function agentsReducer(state: AgentsState, action: AgentsAction): AgentsState {
  switch (action.type) {
    case 'loaded':
      return { roster: action.roster, loaded: true };
    case 'event':
      // One event carries the whole roster, so there is nothing to merge: the
      // newest one is the truth. A consumer that missed an event converges on
      // the next one without any recovery path of its own.
      if (action.event.name !== 'AgentSessionsChanged') return state;
      return { roster: action.event.payload, loaded: true };
    default:
      return state;
  }
}

/**
 * The one session the island shows in full (T231 §7).
 *
 * The island is no longer the primary surface for agents — the rail is — so it
 * keeps a count and the row worth interrupting for. That is the first session
 * whose turn is over and whose move is yours; failing that, the first session
 * of the core's ranking, which already puts this workspace and live work at
 * the top. `null` only when the roster is empty.
 */
export function mostUrgent(roster: AgentRoster): AgentSession | null {
  return (
    roster.sessions.find((session) => session.activity === 'WAITING') ?? roster.sessions[0] ?? null
  );
}

/** How many are still alive — what the collapsed pill counts. */
export function runningCount(roster: AgentRoster): number {
  return roster.sessions.filter((session) => session.activity !== 'ENDED').length;
}

/**
 * What to show as the session's task, in order of how much it actually says.
 *
 * The agent's own title first — it is a summary of the whole session rather
 * than of one turn. Then the standing prompt, then the last reply. `null` when
 * the session has published none of them, which the UI renders as an explicit
 * "no transcript" rather than an empty row.
 */
export function taskLine(session: AgentSession): string | null {
  return session.title ?? session.lastPrompt ?? session.lastReply ?? null;
}

/** `Bash · cargo test`, bounded by the core, or `null` when nothing is running. */
export function toolLine(session: AgentSession): string | null {
  if (!session.tool) return null;
  return session.tool.detail ? `${session.tool.name} · ${session.tool.detail}` : session.tool.name;
}
