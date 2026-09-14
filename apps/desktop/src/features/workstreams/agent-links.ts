import type {
  AgentRoster,
  AgentSession,
  Workstream,
  WorkstreamAgentLink,
  WorkstreamId,
} from '@gritty/protocol';
import { agentStatus, providerLabel } from '../agents/agent-status';
import { taskLine } from '../agents/agents-reducer';

/**
 * Presenting the workstream ↔ agent-session link (T223).
 *
 * Pure, and this is where the join lives: the core stores the link and the core
 * publishes the roster, but it never joins them, because whether a session is
 * running *now* is a question about the present and the answer changes between
 * one read and the next. Putting the join here means a link whose session has
 * gone renders as **linked, not running** — which is what actually happened —
 * instead of disappearing or turning into an error.
 */

/** One link, with the roster row behind it when there still is one. */
export interface LinkedSession {
  readonly link: WorkstreamAgentLink;
  /** `null` when the session is not in the current roster. */
  readonly session: AgentSession | null;
}

/** A link identifies a session by the pair, never by the id alone. */
export function sameSession(
  link: WorkstreamAgentLink,
  session: Pick<AgentSession, 'id' | 'provider'>,
): boolean {
  return link.sessionId === session.id && link.provider === session.provider;
}

export function linksOf(
  links: readonly WorkstreamAgentLink[],
  workstreamId: WorkstreamId,
): WorkstreamAgentLink[] {
  return links.filter((link) => link.workstreamId === workstreamId);
}

/**
 * One workstream's links joined against the roster, in the order they were
 * made. Running sessions first, because those are the ones something can still
 * be learned from; within each group the link order is kept.
 */
export function linkedSessions(
  links: readonly WorkstreamAgentLink[],
  roster: AgentRoster,
  workstreamId: WorkstreamId,
): LinkedSession[] {
  const joined = linksOf(links, workstreamId).map((link) => ({
    link,
    session: roster.sessions.find((session) => sameSession(link, session)) ?? null,
  }));
  return [
    ...joined.filter((entry) => entry.session !== null),
    ...joined.filter((entry) => entry.session === null),
  ];
}

/** The workstreams a roster row is attached to. What a session row shows. */
export function workstreamsOfSession(
  links: readonly WorkstreamAgentLink[],
  workstreams: readonly Workstream[],
  session: Pick<AgentSession, 'id' | 'provider'>,
): Workstream[] {
  const linked = new Set(
    links.filter((link) => sameSession(link, session)).map((link) => link.workstreamId as string),
  );
  return workstreams.filter((workstream) => linked.has(workstream.id as string));
}

/**
 * What a linked row says about the session: its live activity when it is in
 * the roster, and "not running" when it is not.
 *
 * Never "ended", which would be a claim about the session; Gritty only knows
 * that it is not in the roster it can see.
 */
export function linkedStatusLabel(entry: LinkedSession): string {
  return entry.session ? agentStatus(entry.session).label : 'Not running';
}

export function linkedStatusHint(entry: LinkedSession): string {
  return entry.session
    ? agentStatus(entry.session).hint
    : 'This session is not in the roster any more. The link is kept: it records what worked on this.';
}

export function linkedStatusTone(entry: LinkedSession): string {
  return entry.session ? agentStatus(entry.session).tone : 'ended';
}

/**
 * What to call a linked session in a row. The agent's own words when it is
 * running; otherwise the provider and the session id, because a link to a
 * process that is gone still has to be identifiable enough to remove.
 */
export function linkedSessionLabel(entry: LinkedSession): string {
  const task = entry.session ? taskLine(entry.session) : null;
  return task ?? `${providerLabel(entry.link.provider)} · ${entry.link.sessionId}`;
}

/**
 * The roster row's one-line summary: "2 agents · 1 not running", or `null`
 * when there is nothing to say. Counting is worth doing here rather than in
 * the component so the island and the queue cannot disagree.
 */
export function linkedSummary(entries: readonly LinkedSession[]): string | null {
  if (entries.length === 0) return null;
  const gone = entries.filter((entry) => entry.session === null).length;
  const base = `${entries.length} agent${entries.length === 1 ? '' : 's'}`;
  return gone === 0 ? base : `${base} · ${gone} not running`;
}
