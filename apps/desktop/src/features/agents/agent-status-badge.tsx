import type { AgentActivity } from '@gritty/protocol';
import { agentStatus } from './agent-status';

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
 * end up calling the same activity by two different names.
 */
export function AgentStatusBadge({ activity }: { activity: AgentActivity }) {
  const status = agentStatus(activity);
  return (
    <span className={`agent-status agent-status--${status.tone}`} title={status.hint}>
      <span className="agent-status__mark" aria-hidden="true" />
      {status.label}
    </span>
  );
}
