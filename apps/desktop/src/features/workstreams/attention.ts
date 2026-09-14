import type {
  AgentPermissionRequest,
  AgentRun,
  AgentSession,
  Decision,
  OwnershipConflict,
  OwnershipMap,
  Workstream,
  WorkstreamState,
} from '@gritty/protocol';
import type { PermissionCard } from '../agents/permission-model';
import { type AskingRun, askingRuns, runAskCard } from '../runs/run-permission';
import { waitingConflicts } from './ownership';

/**
 * What "needs you" means, in one place (T210, artboards 4b and 5b).
 *
 * Pure: the island and the queue both read these, and they are unit-tested
 * without React. The rules themselves belong to the core — a decision blocks
 * its workstream there, not here — so this file only *presents* what the core
 * already decided.
 */

/** One thing waiting on the user, in the queue and in the island. */
export type AttentionItem =
  | {
      readonly kind: 'question';
      /** Stable across renders and unique across both kinds. */
      readonly id: string;
      readonly decision: Decision;
      readonly workstream: Workstream;
      /** When the wait started, for "oldest first". */
      readonly since: number;
    }
  | {
      readonly kind: 'review';
      readonly id: string;
      readonly workstream: Workstream;
      readonly since: number;
    }
  | {
      readonly kind: 'conflict';
      readonly id: string;
      readonly conflict: OwnershipConflict;
      /** The workstream that asked and is waiting — whose work is blocked. */
      readonly workstream: Workstream;
      /** The one already holding the ground. */
      readonly owner: Workstream;
      readonly since: number;
    }
  | {
      readonly kind: 'permission';
      readonly source: 'session';
      readonly id: string;
      /** The session that is stopped, so the row can name and reach it. */
      readonly session: AgentSession;
      readonly request: AgentPermissionRequest;
      readonly since: number;
    }
  | {
      /**
       * An ask from a run Gritty owns (T310-ANSWER). The same kind, the same
       * filter and the same card as a session's: a user should not have to
       * learn which lane an ask came from in order to answer it. `source` is
       * for the one thing that does differ — where the answer is sent.
       */
      readonly kind: 'permission';
      readonly source: 'run';
      readonly id: string;
      readonly run: AskingRun;
      /** The run's workstream, which is what names it; `null` while loading. */
      readonly workstream: Workstream | null;
      /** The ask, phrased for the card. The id to answer is `run.pending.askId`. */
      readonly request: PermissionCard;
      readonly since: number;
    };

/**
 * The one kind with no workstream required, and deliberately so (T335).
 *
 * The other three are all *about* a workstream — the domain object Gritty owns.
 * A session's permission prompt is about a session Gritty merely observes, and
 * the link between the two (T223's `workstream_agent_link`) is opt-in and
 * usually absent. Requiring one would mean the most urgent item in the queue
 * were the one most likely to be missing from it. A run's ask does have a
 * workstream, and carries it when it is loaded, but is not dropped when not.
 */
export function isPermission(
  item: AttentionItem,
): item is Extract<AttentionItem, { kind: 'permission' }> {
  return item.kind === 'permission';
}

/**
 * Everything waiting on the user, oldest first — the order you triage in, and
 * the order artboard 4b sorts by.
 *
 * Four kinds, because four are all the domain can honestly produce: open
 * decisions, work handed back for review, ownership refusals (T325), and
 * permission prompts an agent is stopped at — an observed session's (T335) or
 * an owned run's (T310-ANSWER). Failures are not a kind yet; inventing one
 * would mean showing the user a category nothing puts anything in.
 */
export function attentionItems(
  workstreams: readonly Workstream[],
  decisions: readonly Decision[],
  ownership: OwnershipMap,
  sessions: readonly AgentSession[] = [],
  runs: readonly AgentRun[] = [],
): AttentionItem[] {
  const byId = new Map(workstreams.map((workstream) => [workstream.id as string, workstream]));
  const items: AttentionItem[] = [];
  for (const decision of decisions) {
    if (decision.state !== 'OPEN') continue;
    const workstream = byId.get(decision.workstreamId as string);
    // A decision whose workstream is gone would be unanswerable; the core
    // cascades them away, so this only guards a partially-loaded renderer.
    if (!workstream) continue;
    items.push({
      kind: 'question',
      id: `question:${decision.id}`,
      decision,
      workstream,
      since: decision.raisedAt,
    });
  }
  for (const workstream of workstreams) {
    if (workstream.state !== 'IN_REVIEW') continue;
    items.push({
      kind: 'review',
      id: `review:${workstream.id}`,
      workstream,
      // When it entered review, which is the last time it changed.
      since: workstream.updatedAt,
    });
  }
  for (const conflict of waitingConflicts(ownership)) {
    const workstream = byId.get(conflict.requestedBy as string);
    const owner = byId.get(conflict.ownedBy as string);
    // Both sides cascade away with their workstream in the database, so this
    // only guards a partially-loaded renderer. A row that cannot name who is
    // in the way is not worth showing.
    if (!workstream || !owner) continue;
    items.push({
      kind: 'conflict',
      id: `conflict:${conflict.id}`,
      conflict,
      workstream,
      owner,
      since: conflict.createdAt,
    });
  }
  for (const session of sessions) {
    if (!session.pending) continue;
    items.push({
      kind: 'permission',
      source: 'session',
      id: `permission:${session.pending.askId}`,
      session,
      request: session.pending,
      since: Number(session.pending.askedAt),
    });
  }
  for (const run of askingRuns(runs)) {
    items.push({
      kind: 'permission',
      source: 'run',
      // Its own prefix: the two lanes mint ask ids independently, and an item
      // id has to be unique across both.
      id: `permission-run:${run.pending.askId}`,
      run,
      workstream: byId.get(run.workstreamId as string) ?? null,
      request: runAskCard(run.pending),
      since: Number(run.pending.askedAt),
    });
  }
  return items.sort((left, right) => left.since - right.since || left.id.localeCompare(right.id));
}

/** How many things are waiting. What the collapsed island counts. */
export function attentionCount(
  workstreams: readonly Workstream[],
  decisions: readonly Decision[],
  ownership: OwnershipMap,
  sessions: readonly AgentSession[] = [],
  runs: readonly AgentRun[] = [],
): number {
  return attentionItems(workstreams, decisions, ownership, sessions, runs).length;
}

/** The queue's filter tabs, with their counts. */
export type AttentionFilter = 'all' | 'permission' | 'question' | 'review' | 'conflict';

export function filterItems(
  items: readonly AttentionItem[],
  filter: AttentionFilter,
): AttentionItem[] {
  return filter === 'all' ? [...items] : items.filter((item) => item.kind === filter);
}

/**
 * The roster's order: what needs you, then what is moving, then the rest.
 * Within a group, most recently touched first — the thing you were just
 * working on stays where you left it.
 */
export function rosterOrder(workstreams: readonly Workstream[]): Workstream[] {
  return [...workstreams].sort(
    (left, right) => rank(left.state) - rank(right.state) || right.updatedAt - left.updatedAt,
  );
}

function rank(state: WorkstreamState): number {
  switch (state) {
    case 'BLOCKED':
      return 0;
    case 'IN_REVIEW':
      return 1;
    case 'IN_PROGRESS':
      return 2;
    case 'READY':
      return 3;
    case 'DRAFT':
      return 4;
    case 'DONE':
      return 5;
    case 'ABANDONED':
      return 6;
  }
}

/** The dot's colour role. Matches the tokens, not a palette of its own. */
export type StateTone = 'active' | 'waiting' | 'blocked' | 'done' | 'idle';

export function stateTone(state: WorkstreamState): StateTone {
  switch (state) {
    case 'IN_PROGRESS':
      return 'active';
    case 'IN_REVIEW':
      return 'waiting';
    case 'BLOCKED':
      return 'blocked';
    case 'DONE':
      return 'done';
    case 'DRAFT':
    case 'READY':
    case 'ABANDONED':
      return 'idle';
  }
}

/** Sentence case, because these are read in a row, not shouted. */
export function stateLabel(state: WorkstreamState): string {
  switch (state) {
    case 'DRAFT':
      return 'Draft';
    case 'READY':
      return 'Ready';
    case 'IN_PROGRESS':
      return 'In progress';
    case 'BLOCKED':
      return 'Blocked';
    case 'IN_REVIEW':
      return 'In review';
    case 'DONE':
      return 'Done';
    case 'ABANDONED':
      return 'Abandoned';
  }
}

/**
 * The verb on the button that makes the move. "Start" reads better than
 * "In progress" on something you are about to do.
 */
export function transitionLabel(to: WorkstreamState): string {
  switch (to) {
    case 'READY':
      return 'Ready';
    case 'IN_PROGRESS':
      return 'Start';
    case 'BLOCKED':
      return 'Block';
    case 'IN_REVIEW':
      return 'Review';
    case 'DONE':
      return 'Done';
    case 'ABANDONED':
      return 'Abandon';
    case 'DRAFT':
      return 'Reopen';
  }
}

/**
 * How long something has been waiting, as the island writes it: `6:41` under
 * an hour, `2h 13m` above it. Coarse on purpose — a queue that ticks every
 * second invites watching instead of working.
 */
export function waitedFor(since: number, now: number): string {
  const seconds = Math.max(0, Math.floor((now - since) / 1000));
  if (seconds < 3600) {
    const minutes = Math.floor(seconds / 60);
    return `${minutes}:${String(seconds % 60).padStart(2, '0')}`;
  }
  const hours = Math.floor(seconds / 3600);
  if (hours < 24) return `${hours}h ${Math.floor((seconds % 3600) / 60)}m`;
  const days = Math.floor(hours / 24);
  return days === 1 ? 'yesterday' : `${days} days ago`;
}
