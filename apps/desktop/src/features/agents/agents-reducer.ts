import type { AgentActivity, AgentRoster, AgentSession, EventEnvelope } from '@gritty/protocol';

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

/** Sessions running in the workspace that is open now. */
export function sessionsHere(roster: AgentRoster): AgentSession[] {
  return roster.sessions.filter((session) => session.inWorkspace);
}

/** Sessions running anywhere else on the machine. */
export function sessionsElsewhere(roster: AgentRoster): AgentSession[] {
  return roster.sessions.filter((session) => !session.inWorkspace);
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

/** One-word label for an activity, matching the dot beside it. */
export function activityLabel(activity: AgentActivity): string {
  switch (activity) {
    case 'WORKING':
      return 'Working';
    case 'WAITING':
      return 'Your move';
    case 'IDLE':
      return 'Idle';
    case 'ENDED':
      return 'Ended';
    default:
      // A provider or activity added by a newer core: show it rather than
      // crash, since the protocol is additive by design.
      return activity;
  }
}

/** Maps an activity onto the island's existing dot tones. */
export function activityTone(activity: AgentActivity): string {
  switch (activity) {
    case 'WORKING':
      return 'active';
    case 'WAITING':
      return 'waiting';
    case 'IDLE':
      return 'idle';
    default:
      return 'done';
  }
}

/** `Bash · cargo test`, bounded by the core, or `null` when nothing is running. */
export function toolLine(session: AgentSession): string | null {
  if (!session.tool) return null;
  return session.tool.detail ? `${session.tool.name} · ${session.tool.detail}` : session.tool.name;
}
