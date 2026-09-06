import type {
  ConflictId,
  Decision,
  DecisionId,
  LeaseId,
  OwnershipConflict,
  OwnershipMap,
  Workstream,
  WorkstreamId,
} from '@gritty/protocol';
import { describe, expect, it } from 'vitest';
import {
  attentionCount,
  attentionItems,
  filterItems,
  rosterOrder,
  stateTone,
  transitionLabel,
  waitedFor,
} from './attention';

function workstream(id: string, patch: Partial<Workstream> = {}): Workstream {
  return {
    id: id as WorkstreamId,
    workspaceId: 'ws-1' as Workstream['workspaceId'],
    title: id,
    state: 'IN_PROGRESS',
    allowedTransitions: ['BLOCKED', 'IN_REVIEW', 'READY', 'ABANDONED'],
    openDecisions: 0,
    createdAt: 1,
    updatedAt: 1,
    ...patch,
  };
}

const NO_OWNERSHIP: OwnershipMap = { leases: [], conflicts: [] };

function conflict(id: string, patch: Partial<OwnershipConflict> = {}): OwnershipConflict {
  return {
    id: id as ConflictId,
    workspaceId: 'ws-1' as OwnershipConflict['workspaceId'],
    requestedBy: 'b' as WorkstreamId,
    ownedBy: 'a' as WorkstreamId,
    holdingLeaseId: 'l-1' as LeaseId,
    target: { type: 'FILE', path: 'src/api/router.ts' },
    createdAt: 15,
    state: 'WAITING',
    ...patch,
  };
}

function ownership(conflicts: OwnershipConflict[]): OwnershipMap {
  return { leases: [], conflicts };
}

function decision(id: string, patch: Partial<Decision> = {}): Decision {
  return {
    id: id as DecisionId,
    workstreamId: 'a' as WorkstreamId,
    question: `question ${id}`,
    options: [],
    state: 'OPEN',
    raisedBy: { type: 'USER' },
    raisedAt: 10,
    ...patch,
  };
}

describe('attention items', () => {
  it('lists open questions and work in review, oldest first', () => {
    const items = attentionItems(
      [
        workstream('a', { state: 'BLOCKED', openDecisions: 1 }),
        workstream('b', { state: 'IN_REVIEW', updatedAt: 5 }),
        workstream('c', { state: 'IN_PROGRESS' }),
      ],
      [decision('d1', { raisedAt: 20 })],
      NO_OWNERSHIP,
    );
    expect(items.map((item) => item.id)).toEqual(['review:b', 'question:d1']);
    expect(items[0]?.since).toBe(5);
  });

  it('ignores answered and dismissed questions', () => {
    const decisions = [
      decision('open'),
      decision('answered', { state: 'RESOLVED', chosen: 'core' }),
      decision('dropped', { state: 'DISMISSED' }),
    ];
    const items = attentionItems([workstream('a')], decisions, NO_OWNERSHIP);
    expect(items).toHaveLength(1);
    expect(attentionCount([workstream('a')], decisions, NO_OWNERSHIP)).toBe(1);
  });

  it('drops a question whose workstream the renderer has not got', () => {
    // The core cascades decisions away with their workstream; this only guards
    // the window between two events.
    expect(attentionItems([], [decision('orphan')], NO_OWNERSHIP)).toEqual([]);
  });

  it('breaks a tie on id so the order is stable across renders', () => {
    const items = attentionItems(
      [workstream('a'), workstream('b', { state: 'IN_REVIEW', updatedAt: 10 })],
      [decision('d1', { raisedAt: 10 })],
      NO_OWNERSHIP,
    );
    expect(items.map((item) => item.id)).toEqual(['question:d1', 'review:b']);
  });

  it('lists an ownership refusal beside the questions, oldest first', () => {
    const items = attentionItems(
      [workstream('a'), workstream('b')],
      [decision('d1', { raisedAt: 30 })],
      ownership([conflict('c1', { createdAt: 15 })]),
    );
    expect(items.map((item) => item.id)).toEqual(['conflict:c1', 'question:d1']);
    const [first] = items;
    expect(first?.kind).toBe('conflict');
    if (first?.kind === 'conflict') {
      // The row is filed under the workstream that is *waiting*, and names the
      // one in the way — which is the whole content of the refusal.
      expect(first.workstream.id).toBe('b');
      expect(first.owner.id).toBe('a');
    }
  });

  it('ignores conflicts that are no longer open', () => {
    const settled = ownership([
      conflict('resolved', { state: 'RESOLVED' }),
      conflict('cancelled', { state: 'CANCELLED' }),
      conflict('overridden', { state: 'OVERRIDDEN' }),
    ]);
    expect(attentionItems([workstream('a'), workstream('b')], [], settled)).toEqual([]);
  });

  it('lists an observed violation in the same queue, under the same filter', () => {
    // T326 adds a state, not a kind: a violation and a refusal are both
    // "someone is in someone else's way", so there is no fifth filter and the
    // row is filed the same way — under whoever wrote, naming whoever held it.
    const items = attentionItems(
      [workstream('a'), workstream('b')],
      [],
      ownership([conflict('v1', { state: 'OBSERVED', createdAt: 20 })]),
    );
    expect(items.map((item) => item.id)).toEqual(['conflict:v1']);
    const [first] = items;
    expect(first?.kind).toBe('conflict');
    if (first?.kind === 'conflict') {
      expect(first.workstream.id).toBe('b');
      expect(first.owner.id).toBe('a');
      expect(first.conflict.state).toBe('OBSERVED');
    }
    expect(filterItems(items, 'conflict')).toHaveLength(1);
    expect(
      attentionCount(
        [workstream('a'), workstream('b')],
        [],
        ownership([conflict('v1', { state: 'OBSERVED' })]),
      ),
    ).toBe(1);
  });

  it('drops a conflict whose two sides the renderer has not both got', () => {
    // Both cascade away with their workstream in the database; a row that
    // cannot say who is in the way is not worth drawing.
    const only = [workstream('b')];
    expect(attentionItems(only, [], ownership([conflict('c1')]))).toEqual([]);
  });

  it('filters by kind without losing the order', () => {
    const items = attentionItems(
      [workstream('a'), workstream('b', { state: 'IN_REVIEW', updatedAt: 30 })],
      [decision('d1', { raisedAt: 10 }), decision('d2', { raisedAt: 20 })],
      NO_OWNERSHIP,
    );
    expect(filterItems(items, 'all')).toHaveLength(3);
    expect(filterItems(items, 'question').map((item) => item.id)).toEqual([
      'question:d1',
      'question:d2',
    ]);
    expect(filterItems(items, 'review').map((item) => item.id)).toEqual(['review:b']);
    expect(filterItems(items, 'conflict')).toEqual([]);
  });
});

describe('roster order', () => {
  it('puts what needs you first, then what is moving, then the rest', () => {
    const ordered = rosterOrder([
      workstream('done', { state: 'DONE' }),
      workstream('draft', { state: 'DRAFT' }),
      workstream('blocked', { state: 'BLOCKED' }),
      workstream('review', { state: 'IN_REVIEW' }),
      workstream('running', { state: 'IN_PROGRESS' }),
      workstream('ready', { state: 'READY' }),
      workstream('dropped', { state: 'ABANDONED' }),
    ]);
    expect(ordered.map((item) => item.id)).toEqual([
      'blocked',
      'review',
      'running',
      'ready',
      'draft',
      'done',
      'dropped',
    ]);
  });

  it('keeps the most recently touched first within a group', () => {
    const ordered = rosterOrder([
      workstream('older', { updatedAt: 1 }),
      workstream('newer', { updatedAt: 9 }),
    ]);
    expect(ordered.map((item) => item.id)).toEqual(['newer', 'older']);
  });

  it('does not mutate what it is given', () => {
    const input = [workstream('a', { state: 'DONE' }), workstream('b', { state: 'BLOCKED' })];
    rosterOrder(input);
    expect(input.map((item) => item.id)).toEqual(['a', 'b']);
  });
});

describe('labels', () => {
  it('gives every state a tone and every move a verb', () => {
    const states = [
      'DRAFT',
      'READY',
      'IN_PROGRESS',
      'BLOCKED',
      'IN_REVIEW',
      'DONE',
      'ABANDONED',
    ] as const;
    for (const state of states) {
      expect(stateTone(state)).toBeTruthy();
      expect(transitionLabel(state)).toBeTruthy();
    }
    expect(stateTone('BLOCKED')).toBe('blocked');
    expect(transitionLabel('IN_PROGRESS')).toBe('Start');
  });

  it('writes a wait as minutes under an hour and coarser above it', () => {
    const now = 1_000_000;
    expect(waitedFor(now, now)).toBe('0:00');
    expect(waitedFor(now - 41_000, now)).toBe('0:41');
    expect(waitedFor(now - 401_000, now)).toBe('6:41');
    expect(waitedFor(now - 3_600_000 * 2 - 780_000, now)).toBe('2h 13m');
    expect(waitedFor(now - 3_600_000 * 25, now)).toBe('yesterday');
    expect(waitedFor(now - 3_600_000 * 72, now)).toBe('3 days ago');
  });

  it('never writes a negative wait when the clock disagrees', () => {
    expect(waitedFor(2_000, 1_000)).toBe('0:00');
  });
});

// ---------------------------------------------------------------------------
// The fourth kind (T335, artboard 12b)
// ---------------------------------------------------------------------------

function asking(id: string, askedAt: number): import('@gritty/protocol').AgentSession {
  return {
    id,
    provider: 'CLAUDE_CODE',
    activity: 'BLOCKED',
    activitySource: 'HOOK',
    evidence: 'BOTH',
    inWorkspace: true,
    title: `session ${id}`,
    pending: {
      askId: `ask-${id}`,
      sessionId: id,
      verb: 'RUN_COMMAND',
      target: 'cargo publish --dry-run',
      diff: [],
      added: 0,
      removed: 0,
      outsideRoot: false,
      sensitive: false,
      askedAt,
    },
  } as import('@gritty/protocol').AgentSession;
}

describe('a permission prompt is something waiting on you', () => {
  it('joins the queue in the same oldest-first order as everything else', () => {
    const items = attentionItems(
      [workstream('a', { state: 'IN_REVIEW', updatedAt: 500 })],
      [],
      NO_OWNERSHIP,
      [asking('s1', 100)],
    );
    expect(items.map((item) => item.kind)).toEqual(['permission', 'review']);
  });

  it('needs no workstream, because the link to one is opt-in and usually absent', () => {
    // The failure this guards against would be the worst one in the queue: the
    // most urgent item also being the one most often missing from it.
    const items = attentionItems([], [], NO_OWNERSHIP, [asking('s1', 100)]);
    expect(items).toHaveLength(1);
    expect(items[0]?.kind).toBe('permission');
  });

  it('counts toward the header, which is what the chip beside ⌥⌘A says', () => {
    expect(attentionCount([], [], NO_OWNERSHIP, [asking('s1', 1), asking('s2', 2)])).toBe(2);
    // And a session that is merely working is not waiting on anyone.
    const busy = { ...asking('s3', 3), pending: undefined };
    expect(attentionCount([], [], NO_OWNERSHIP, [busy])).toBe(0);
  });

  it('has its own filter, so triaging permissions does not mean reading past reviews', () => {
    const items = attentionItems([workstream('a', { state: 'IN_REVIEW' })], [], NO_OWNERSHIP, [
      asking('s1', 100),
    ]);
    expect(filterItems(items, 'permission').map((item) => item.id)).toEqual(['permission:ask-s1']);
    expect(filterItems(items, 'review')).toHaveLength(1);
    expect(filterItems(items, 'all')).toHaveLength(2);
  });

  it('keeps counting nothing when no sessions are passed at all', () => {
    // The island and the queue call this with and without the roster; the
    // default must be "no permissions", never a crash.
    expect(attentionItems([], [], NO_OWNERSHIP)).toEqual([]);
  });
});
