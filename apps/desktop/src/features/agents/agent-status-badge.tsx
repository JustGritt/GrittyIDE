import { type AgentStatusSubject, agentStatus } from './agent-status';

/**
 * The status of one session: a word, and a shape that means the same thing.
 *
 * Colour comes from an ADR-016 token through the tone class, and the same class
 * draws the marker's shape — a filled disc, a diamond, a square, a ring, a bar.
 * Status is never carried by hue alone (T231 §10): five statuses that differ
 * only in colour are four statuses and a guess for anyone who cannot see the
 * difference between them.
 *
 * Shared by the rail and the island's reduced row, so the two surfaces cannot
 * end up calling the same activity by two different names. It takes the source
 * too, because the same `WAITING` is Done or Alive depending on what saw it.
 */
export function AgentStatusBadge({ session }: { session: AgentStatusSubject }) {
  const status = agentStatus(session);
  return (
    <span className={`agent-status agent-status--${status.tone}`} title={status.hint}>
      <span className="agent-status__mark" aria-hidden="true" />
      {status.label}
    </span>
  );
}
