import type {
  AgentPermissionVerdict,
  AgentRun,
  AgentRunPermissionAsk,
  AgentRunPermissionVerdict,
} from '@gritty/protocol';
import type { PermissionCard } from '../agents/permission-model';

/**
 * A run's permission ask, for the card T335 already draws (T310-ANSWER).
 *
 * "No second permission UI": an owned run's ask is drawn by the same
 * `PermissionPrompt`, answered with the same chords, and listed in the same
 * queue as an observed session's. This file is the whole of the difference —
 * a phrasing in one direction and a verdict in the other — and it is pure so
 * both halves are a test rather than a click.
 */

/**
 * The ask as the card reads it.
 *
 * The run's verbs are spelled as a subset of the session's, so the verb maps by
 * name. What a run cannot say is said as absence: no diff and no line counts
 * (the core does not look at a write's content, deliberately), nothing outside
 * a root (a run's paths are not classified that way), nothing redacted, and —
 * the one that matters — **no suggested scope**, so the card offers no "allow
 * for this run" and `⌘⏎` does nothing. A run takes no remembered grants
 * (`T310-GRANTS`).
 *
 * For a tool nothing more specific is known about, the tool's name leads the
 * aside: "use tool · /etc/hosts" is not enough to answer on, "Read" is.
 */
export function runAskCard(ask: AgentRunPermissionAsk): PermissionCard {
  const detail =
    ask.verb === 'OTHER' ? [ask.tool, ask.detail].filter(Boolean).join(' · ') : ask.detail;
  return {
    verb: ask.verb,
    target: ask.target,
    ...(detail ? { detail } : {}),
    diff: [],
    added: 0,
    removed: 0,
    outsideRoot: false,
    sensitive: false,
    askedAt: ask.askedAt,
  };
}

/**
 * The card's answer, as a run takes it — or `null` for one a run does not take.
 *
 * `ALLOW_FOR_RUN` cannot come from a run's card (it offers no scope), and a run
 * would refuse it; mapping it to either of the other two would be a decision
 * the user did not make, so it maps to nothing.
 */
export function runVerdict(verdict: AgentPermissionVerdict): AgentRunPermissionVerdict | null {
  switch (verdict) {
    case 'ALLOW_ONCE':
      return 'ALLOW_ONCE';
    case 'DENY':
      return 'DENY';
    case 'ALLOW_FOR_RUN':
      return null;
  }
}

/** A run with an ask outstanding, and the ask. */
export type AskingRun = AgentRun & { readonly pending: AgentRunPermissionAsk };

/** Every run stopped at an ask, oldest ask first — the order the queue triages in. */
export function askingRuns(runs: readonly AgentRun[]): AskingRun[] {
  return runs
    .filter((run): run is AskingRun => run.pending !== undefined)
    .sort((left, right) => Number(left.pending.askedAt) - Number(right.pending.askedAt));
}
