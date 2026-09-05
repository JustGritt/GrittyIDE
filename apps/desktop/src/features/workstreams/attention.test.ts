import type { Decision, DecisionId, Workstream, WorkstreamId } from '@gritty/protocol';
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
    const items = attentionItems([workstream('a')], decisions);
    expect(items).toHaveLength(1);
    expect(attentionCount([workstream('a')], decisions)).toBe(1);
  });

  it('drops a question whose workstream the renderer has not got', () => {
    // The core cascades decisions away with their workstream; this only guards
    // the window between two events.
    expect(attentionItems([], [decision('orphan')])).toEqual([]);
  });

  it('breaks a tie on id so the order is stable across renders', () => {
    const items = attentionItems(
      [workstream('a'), workstream('b', { state: 'IN_REVIEW', updatedAt: 10 })],
      [decision('d1', { raisedAt: 10 })],
    );
    expect(items.map((item) => item.id)).toEqual(['question:d1', 'review:b']);
  });

  it('filters by kind without losing the order', () => {
    const items = attentionItems(
      [workstream('a'), workstream('b', { state: 'IN_REVIEW', updatedAt: 30 })],
      [decision('d1', { raisedAt: 10 }), decision('d2', { raisedAt: 20 })],
    );
    expect(filterItems(items, 'all')).toHaveLength(3);
    expect(filterItems(items, 'question').map((item) => item.id)).toEqual([
      'question:d1',
      'question:d2',
    ]);
    expect(filterItems(items, 'review').map((item) => item.id)).toEqual(['review:b']);
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
