import type {
  CommitPage,
  DiffSummary,
  EventEnvelope,
  FileStatus,
  Operation,
  OperationId,
  RepositoryStatus,
  WorkspaceId,
  WorkspacePath,
} from '@gritty/protocol';
import { describe, expect, it } from 'vitest';
import {
  DIFF_OPERATION_KIND,
  type GitState,
  gitReducer,
  historyCursor,
  initialGitState,
  LOG_OPERATION_KIND,
  STATUS_OPERATION_KIND,
  statusCursor,
} from './git-reducer';

const WS = 'ws-1' as WorkspaceId;
const OTHER_WS = 'ws-2' as WorkspaceId;
const OP = 'op-1' as OperationId;

function entry(path: string, overrides: Partial<FileStatus> = {}): FileStatus {
  return {
    path: path as WorkspacePath,
    index: 'UNMODIFIED',
    worktree: 'MODIFIED',
    kind: 'FILE',
    conflicted: false,
    ...overrides,
  };
}

function repository(overrides: Partial<RepositoryStatus> = {}): RepositoryStatus {
  return {
    branch: 'main',
    detached: false,
    head: 'abcdef1234',
    state: 'CLEAN',
    entries: [entry('a.txt')],
    total: 1,
    truncated: false,
    unrepresentable: 0,
    ...overrides,
  };
}

function summary(overrides: Partial<DiffSummary> = {}): DiffSummary {
  return {
    target: 'HEAD',
    resolved: 'abcdef1234',
    files: [],
    total: 0,
    truncated: false,
    unrepresentable: 0,
    ...overrides,
  };
}

function operation(kind: string, status: Operation['status'] = 'COMPLETED'): Operation {
  return {
    id: OP,
    kind,
    actor: { type: 'USER' },
    workspaceId: WS,
    status,
    createdAt: 1,
    cancellationRequested: false,
    correlationId: 'corr' as Operation['correlationId'],
  };
}

function event(
  payload: EventEnvelope['name'] extends never ? never : Omit<EventEnvelope, 'seq' | 'actor'>,
): EventEnvelope {
  return { seq: 1, actor: { type: 'USER' }, ...payload } as EventEnvelope;
}

function opened(): GitState {
  return gitReducer(initialGitState, { type: 'workspace', workspaceId: WS });
}

describe('the workspace a read belongs to', () => {
  it('starts idle so the hook knows to read', () => {
    expect(opened().status).toEqual({ kind: 'idle' });
    expect(opened().workspaceId).toBe(WS);
  });

  it('does not reset when the same workspace is announced again', () => {
    const state = gitReducer(opened(), {
      type: 'statusReady',
      repository: repository(),
      append: false,
    });
    expect(gitReducer(state, { type: 'workspace', workspaceId: WS })).toBe(state);
  });

  it('drops every read when the workspace changes', () => {
    let state = gitReducer(opened(), {
      type: 'statusReady',
      repository: repository(),
      append: false,
    });
    state = gitReducer(state, { type: 'select', path: 'a.txt' as WorkspacePath });
    state = gitReducer(state, { type: 'workspace', workspaceId: OTHER_WS });
    expect(state).toEqual({ ...initialGitState, workspaceId: OTHER_WS });
  });
});

describe('the three arms of a git.status response', () => {
  it('ready carries the repository', () => {
    const state = gitReducer(opened(), {
      type: 'statusReady',
      repository: repository(),
      append: false,
    });
    expect(state.status).toEqual({ kind: 'ready', value: repository() });
  });

  it('notARepository is a value, never an error', () => {
    const state = gitReducer(opened(), { type: 'statusNotARepository' });
    expect(state.status).toEqual({ kind: 'notARepository' });
    // Nothing to have selected in a folder that is not a repository.
    expect(state.selected).toBeNull();
    expect(state.diff).toEqual({ kind: 'idle' });
  });

  it('pending names the operation the read handed over to', () => {
    const state = gitReducer(opened(), { type: 'statusPending', operationId: OP });
    expect(state.status).toEqual({ kind: 'pending', operationId: OP });
  });

  it('a refusal stays a refusal, so the hook does not loop on it', () => {
    const error = { code: 'Io' as const, message: 'unreadable object', retryable: true };
    const state = gitReducer(opened(), { type: 'statusFailed', error });
    expect(state.status).toEqual({ kind: 'failed', error });
  });
});

describe('a read that became an operation', () => {
  it('completes on OperationCompleted with the payload', () => {
    let state = gitReducer(opened(), { type: 'statusStarted', append: false });
    state = gitReducer(state, { type: 'statusPending', operationId: OP });
    state = gitReducer(state, {
      type: 'event',
      event: event({
        name: 'OperationCompleted',
        payload: { operation: operation(STATUS_OPERATION_KIND), result: repository() },
      }),
    });
    expect(state.status).toEqual({ kind: 'ready', value: repository() });
  });

  it('keeps an outcome that arrives before the pending response, and applies it then', () => {
    let state = gitReducer(opened(), { type: 'statusStarted', append: false });
    state = gitReducer(state, {
      type: 'event',
      event: event({
        name: 'OperationCompleted',
        payload: { operation: operation(STATUS_OPERATION_KIND), result: repository() },
      }),
    });
    // The command response and the completion event are separate IPC messages
    // and their order is not guaranteed.
    expect(state.status).toEqual({ kind: 'loading' });
    expect(Object.keys(state.unmatched)).toEqual([OP]);
    state = gitReducer(state, { type: 'statusPending', operationId: OP });
    expect(state.status).toEqual({ kind: 'ready', value: repository() });
    expect(state.unmatched).toEqual({});
  });

  it('reports a completion with no result rather than showing an empty repository', () => {
    let state = gitReducer(opened(), { type: 'statusPending', operationId: OP });
    state = gitReducer(state, {
      type: 'event',
      event: event({
        name: 'OperationCompleted',
        payload: { operation: operation(STATUS_OPERATION_KIND) },
      }),
    });
    expect(state.status.kind).toBe('failed');
    expect(state.status.kind === 'failed' && state.status.error.code).toBe('Internal');
  });

  it('carries an OperationFailed error onto the read it belongs to', () => {
    let state = gitReducer(opened(), { type: 'statusPending', operationId: OP });
    const error = { code: 'Io' as const, message: 'corrupt object', retryable: false };
    state = gitReducer(state, {
      type: 'event',
      event: event({
        name: 'OperationFailed',
        payload: { operation: operation(STATUS_OPERATION_KIND, 'FAILED'), error },
      }),
    });
    expect(state.status).toEqual({ kind: 'failed', error });
  });

  it('is cancelled only when the event says so, never optimistically', () => {
    let state = gitReducer(opened(), { type: 'statusPending', operationId: OP });
    expect(state.status.kind).toBe('pending');
    state = gitReducer(state, {
      type: 'event',
      event: event({
        name: 'OperationCancelled',
        payload: { operation: operation(STATUS_OPERATION_KIND, 'CANCELLED') },
      }),
    });
    expect(state.status).toEqual({ kind: 'cancelled' });
  });

  it('routes by operation kind, so a diff completion never lands on the status', () => {
    let state = gitReducer(opened(), { type: 'statusPending', operationId: OP });
    state = gitReducer(state, {
      type: 'event',
      event: event({
        name: 'OperationCompleted',
        payload: { operation: operation(DIFF_OPERATION_KIND), result: summary() },
      }),
    });
    expect(state.status).toEqual({ kind: 'pending', operationId: OP });
  });

  it('ignores an operation that belongs to another domain entirely', () => {
    const state = gitReducer(opened(), { type: 'statusPending', operationId: OP });
    expect(
      gitReducer(state, {
        type: 'event',
        event: event({
          name: 'OperationCompleted',
          payload: { operation: operation('workspace.list'), result: {} },
        }),
      }),
    ).toBe(state);
  });

  it('bounds the outcomes it parks, so an unclaimed one cannot grow without limit', () => {
    let state = opened();
    for (let index = 0; index < 40; index += 1) {
      state = gitReducer(state, {
        type: 'event',
        event: event({
          name: 'OperationCompleted',
          payload: {
            operation: { ...operation(STATUS_OPERATION_KIND), id: `op-${index}` as OperationId },
            result: repository(),
          },
        }),
      });
    }
    expect(Object.keys(state.unmatched).length).toBeLessThanOrEqual(8);
    // The newest is what a response is most likely to claim.
    expect(state.unmatched['op-39']).toBeDefined();
  });
});

describe('paging a status list', () => {
  it('offers a cursor only while the list is truncated', () => {
    expect(statusCursor(opened())).toBeNull();
    const complete = gitReducer(opened(), {
      type: 'statusReady',
      repository: repository(),
      append: false,
    });
    expect(statusCursor(complete)).toBeNull();
    const more = gitReducer(opened(), {
      type: 'statusReady',
      repository: repository({ truncated: true, cursor: 'c1', total: 3 }),
      append: false,
    });
    expect(statusCursor(more)).toBe('c1');
  });

  it('keeps the page already shown while the next one is in flight', () => {
    let state = gitReducer(opened(), {
      type: 'statusReady',
      repository: repository({ truncated: true, cursor: 'c1', total: 3 }),
      append: false,
    });
    state = gitReducer(state, { type: 'statusStarted', append: true });
    expect(state.status.kind).toBe('ready');
    expect(state.appending).toBe(true);
  });

  it('appends the next page and adds up what could not be represented', () => {
    let state = gitReducer(opened(), {
      type: 'statusReady',
      repository: repository({ truncated: true, cursor: 'c1', total: 3, unrepresentable: 1 }),
      append: false,
    });
    state = gitReducer(state, { type: 'statusStarted', append: true });
    state = gitReducer(state, {
      type: 'statusReady',
      append: true,
      repository: repository({
        entries: [entry('b.txt'), entry('c.txt')],
        total: 3,
        unrepresentable: 2,
      }),
    });
    expect(state.appending).toBe(false);
    const value = state.status.kind === 'ready' ? state.status.value : null;
    expect(value?.entries.map((one) => one.path)).toEqual(['a.txt', 'b.txt', 'c.txt']);
    expect(value?.truncated).toBe(false);
    // Each page reports what it dropped; shrinking that count would hide a file.
    expect(value?.unrepresentable).toBe(3);
  });
});

describe('the selected file and its diff', () => {
  it('starts a fresh read when the selection changes', () => {
    let state = gitReducer(opened(), { type: 'diffReady', diff: summary() });
    state = gitReducer(state, { type: 'select', path: 'a.txt' as WorkspacePath });
    expect(state.selected).toBe('a.txt');
    expect(state.diff).toEqual({ kind: 'idle' });
  });

  it('does nothing when the same file is selected again', () => {
    const state = gitReducer(opened(), { type: 'select', path: 'a.txt' as WorkspacePath });
    expect(gitReducer(state, { type: 'select', path: 'a.txt' as WorkspacePath })).toBe(state);
  });

  it('carries the three arms of git.diff the same way', () => {
    const ready = gitReducer(opened(), { type: 'diffReady', diff: summary() });
    expect(ready.diff).toEqual({ kind: 'ready', value: summary() });
    expect(gitReducer(opened(), { type: 'diffNotARepository' }).diff).toEqual({
      kind: 'notARepository',
    });
    expect(gitReducer(opened(), { type: 'diffPending', operationId: OP }).diff).toEqual({
      kind: 'pending',
      operationId: OP,
    });
  });

  it('completes its own operation without touching the status read', () => {
    let state = gitReducer(opened(), {
      type: 'statusReady',
      repository: repository(),
      append: false,
    });
    state = gitReducer(state, { type: 'select', path: 'a.txt' as WorkspacePath });
    state = gitReducer(state, { type: 'diffPending', operationId: OP });
    state = gitReducer(state, {
      type: 'event',
      event: event({
        name: 'OperationCompleted',
        payload: { operation: operation(DIFF_OPERATION_KIND), result: summary() },
      }),
    });
    expect(state.diff).toEqual({ kind: 'ready', value: summary() });
    expect(state.status).toEqual({ kind: 'ready', value: repository() });
  });
});

// T101-U: the history read. A third `Read` beside status and diff, sharing the
// one `unmatched` map — so what needs proving is that an outcome reaches the
// read it belongs to and never one of the other two.
describe('the history read', () => {
  function commitPage(overrides: Partial<CommitPage> = {}): CommitPage {
    return {
      start: 'HEAD',
      resolved: 'a'.repeat(40),
      commits: [
        {
          id: 'a'.repeat(40),
          shortId: 'aaaaaaa',
          parents: [],
          author: { name: 'Ada', email: 'ada@example.com', time: 1, offsetSeconds: 0 },
          committer: { name: 'Ada', email: 'ada@example.com', time: 1, offsetSeconds: 0 },
          subject: 'first',
        },
      ],
      truncated: false,
      ...overrides,
    };
  }

  it('is idle until it is asked for, so opening the panel walks no history', () => {
    expect(opened().history).toEqual({ kind: 'idle' });
  });

  it('appends a page rather than replacing the one already shown', () => {
    let state = gitReducer(opened(), { type: 'logStarted', append: false });
    state = gitReducer(state, {
      type: 'logReady',
      page: commitPage({ truncated: true, cursor: 'a'.repeat(40) }),
      append: false,
    });
    expect(historyCursor(state)).toBe('a'.repeat(40));

    const second = commitPage({
      commits: commitPage().commits.map((only) => ({
        ...only,
        id: 'b'.repeat(40),
        subject: 'second',
      })),
    });
    state = gitReducer(state, { type: 'logStarted', append: true });
    // The page already shown is kept while the next one is in flight.
    expect(state.history.kind).toBe('ready');
    expect(state.appendingHistory).toBe(true);

    state = gitReducer(state, { type: 'logReady', page: second, append: true });
    expect(state.history.kind === 'ready' && state.history.value.commits).toHaveLength(2);
    expect(state.appendingHistory).toBe(false);
    // The newer page's boundary wins: it describes where to continue from.
    expect(historyCursor(state)).toBeNull();
  });

  it('routes a completion to the history and not to the status or the diff', () => {
    let state = gitReducer(opened(), { type: 'logStarted', append: false });
    state = gitReducer(state, { type: 'logPending', operationId: OP });
    state = gitReducer(state, {
      type: 'event',
      event: event({
        name: 'OperationCompleted',
        payload: { operation: operation(LOG_OPERATION_KIND), result: commitPage() },
      }),
    });
    expect(state.history).toEqual({ kind: 'ready', value: commitPage() });
    expect(state.status).toEqual({ kind: 'idle' });
    expect(state.diff).toEqual({ kind: 'idle' });
  });

  it('keeps an outcome that arrives before the pending response, and applies it then', () => {
    let state = gitReducer(opened(), { type: 'logStarted', append: false });
    state = gitReducer(state, {
      type: 'event',
      event: event({
        name: 'OperationCompleted',
        payload: { operation: operation(LOG_OPERATION_KIND), result: commitPage() },
      }),
    });
    expect(state.history).toEqual({ kind: 'loading' });
    expect(Object.keys(state.unmatched)).toEqual([OP]);

    state = gitReducer(state, { type: 'logPending', operationId: OP });
    expect(state.history).toEqual({ kind: 'ready', value: commitPage() });
    expect(state.unmatched).toEqual({});
  });

  it('reports a completion with no result rather than an empty history', () => {
    let state = gitReducer(opened(), { type: 'logStarted', append: false });
    state = gitReducer(state, { type: 'logPending', operationId: OP });
    state = gitReducer(state, {
      type: 'event',
      event: event({
        name: 'OperationCompleted',
        payload: { operation: operation(LOG_OPERATION_KIND), result: undefined },
      }),
    });
    // "Partial data shown as complete" is what the failure table forbids: an
    // empty commit list would read as a repository with no history.
    expect(state.history.kind).toBe('failed');
  });

  it('drops the history when the workspace changes', () => {
    let state = gitReducer(opened(), { type: 'logStarted', append: false });
    state = gitReducer(state, { type: 'logReady', page: commitPage(), append: false });
    state = gitReducer(state, { type: 'workspace', workspaceId: OTHER_WS });
    expect(state.history).toEqual({ kind: 'idle' });
  });
});
