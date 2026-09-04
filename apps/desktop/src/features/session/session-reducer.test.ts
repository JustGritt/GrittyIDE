import type { EventEnvelope, SessionRestoreResponse, WorkspaceId } from '@gritty/protocol';
import { describe, expect, it } from 'vitest';
import {
  automaticCandidates,
  hasPrompt,
  initialSessionState,
  pendingChoices,
  type SessionState,
  sessionReducer,
} from './session-reducer';

const ws = 'ws-1' as WorkspaceId;

const response: SessionRestoreResponse = {
  workspace: { id: ws, root: '/repo', name: 'repo', openedAt: 1 },
  tabs: [],
  persistence: { available: true, healthy: true, databasePath: '/data/gritty.sqlite' },
  notices: [{ kind: 'DATABASE_MOVED_ASIDE', message: 'moved', path: '/data/old' }],
};

function loaded(): SessionState {
  const started = sessionReducer(initialSessionState, { type: 'restoreStarted' });
  const finished = sessionReducer(started, { type: 'restoreFinished', response });
  return sessionReducer(finished, {
    type: 'candidatesLoaded',
    workspaceId: ws,
    candidates: [
      { path: 'a.md', state: 'SAME_AS_DISK', updatedAt: 1, size: 3 },
      { path: 'b.md', state: 'DISK_CHANGED', updatedAt: 2, size: 4 },
      { path: 'c.md', state: 'FILE_MISSING', updatedAt: 3, size: 5 },
    ],
  });
}

describe('sessionReducer', () => {
  it('walks idle → restoring → ready and keeps the persistence status and notices', () => {
    const started = sessionReducer(initialSessionState, { type: 'restoreStarted' });
    expect(started.phase).toBe('restoring');
    const finished = sessionReducer(started, { type: 'restoreFinished', response });
    expect(finished.persistence).toEqual(response.persistence);
    expect(finished.notices).toHaveLength(1);
    const twice = sessionReducer(finished, { type: 'restoreFinished', response });
    expect(twice.notices).toHaveLength(1);
    const settled = sessionReducer(twice, { type: 'restoreSettled', skipped: ['gone.md'] });
    expect(settled.phase).toBe('ready');
    expect(settled.skipped).toEqual(['gone.md']);
    expect(sessionReducer(settled, { type: 'dismissSkipped' }).skipped).toEqual([]);
    expect(sessionReducer(settled, { type: 'dismissNotice', index: 0 }).notices).toEqual([]);
  });

  it('a failed restore reports unavailable persistence and still settles', () => {
    const failed = sessionReducer(sessionReducer(initialSessionState, { type: 'restoreStarted' }), {
      type: 'restoreFailed',
      error: { code: 'Timeout', message: 'core did not answer', retryable: true },
    });
    expect(failed.persistence).toMatchObject({ available: false, healthy: false });
    expect(failed.notices[0]?.kind).toBe('PERSISTENCE_UNAVAILABLE');
    expect(failed.notices[0]?.message).toContain('core did not answer');
    expect(sessionReducer(failed, { type: 'restoreSettled', skipped: [] }).phase).toBe('ready');
  });

  it('splits candidates into choices and automatic recoveries', () => {
    const state = loaded();
    expect(hasPrompt(state)).toBe(true);
    expect(pendingChoices(state).map((c) => c.path)).toEqual(['a.md']);
    expect(automaticCandidates(state).map((c) => c.path)).toEqual(['b.md', 'c.md']);
  });

  it('tracks each candidate through restoring, failure and removal', () => {
    let state = loaded();
    state = sessionReducer(state, { type: 'candidateStarted', path: 'a.md' });
    expect(pendingChoices(state)).toEqual([]);
    expect(state.candidates?.items[0]).toMatchObject({ status: 'restoring', error: null });
    state = sessionReducer(state, { type: 'candidateFailed', path: 'a.md', message: 'edited' });
    expect(state.candidates?.items[0]).toMatchObject({ status: 'failed', error: 'edited' });
    expect(pendingChoices(state).map((c) => c.path)).toEqual(['a.md']);
    state = sessionReducer(state, { type: 'candidateStarted', path: 'b.md' });
    expect(automaticCandidates(state).map((c) => c.path)).toEqual(['c.md']);
    state = sessionReducer(state, { type: 'candidateRemoved', path: 'b.md' });
    state = sessionReducer(state, { type: 'candidateRemoved', path: 'c.md' });
    state = sessionReducer(state, { type: 'candidateRemoved', path: 'a.md' });
    expect(hasPrompt(state)).toBe(false);
    expect(sessionReducer(state, { type: 'candidateRemoved', path: 'zzz' })).toBe(state);
  });

  it('drops the candidates when their workspace closes and follows persistence events', () => {
    const state = loaded();
    const closed: EventEnvelope = {
      name: 'WorkspaceClosed',
      seq: 9,
      actor: { type: 'USER' },
      payload: { workspaceId: ws, root: '/repo', reason: 'USER' },
    };
    expect(sessionReducer(state, { type: 'event', event: closed }).candidates).toBeNull();
    const other: EventEnvelope = {
      ...closed,
      payload: { workspaceId: 'ws-2' as WorkspaceId, root: '/other', reason: 'USER' },
    };
    expect(sessionReducer(state, { type: 'event', event: other })).toBe(state);
    const degraded: EventEnvelope = {
      name: 'PersistenceStatusChanged',
      seq: 10,
      actor: { type: 'SYSTEM' },
      payload: {
        status: { available: true, healthy: false, message: 'disk full' },
        notice: { kind: 'DATABASE_MOVED_ASIDE', message: 'replaced', path: '/data/x' },
      },
    };
    const after = sessionReducer(state, { type: 'event', event: degraded });
    expect(after.persistence).toMatchObject({ healthy: false, message: 'disk full' });
    expect(after.notices).toHaveLength(2);
  });
});
