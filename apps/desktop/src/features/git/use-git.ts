import {
  type GitDiffRequest,
  type GitLogRequest,
  type GitStatusRequest,
  toCoreError,
  type WorkspaceId,
  type WorkspacePath,
} from '@gritty/protocol';
import { useCallback, useEffect, useReducer } from 'react';
import { core } from '../../core/client';
import {
  gitReducer,
  historyCursor,
  initialGitState,
  pendingOperation,
  statusCursor,
} from './git-reducer';

/** The Changes tab as its one consumer sees it. */
export type GitController = ReturnType<typeof useGit>;

/**
 * `git.status` for the open workspace, and `git.diff` for the file the user
 * selected (T100-U).
 *
 * **Polled on demand, never watched.** T100 rules watcher integration out
 * explicitly, so the repository is read when the tab mounts, when the
 * workspace changes, and when the user asks — and at no other time. Nothing
 * here runs while the Files tab is showing, because the component that owns
 * this hook is not mounted then.
 *
 * Both calls answer one of three ways and the middle one is why this hook is
 * more than a fetch: a read that outlives its budget comes back `pending` with
 * an `Operation`, and the payload arrives later on `OperationCompleted`. The
 * subscription below is what catches it, and it is opened **before** the first
 * command so a completion cannot land in the gap.
 */
export function useGit(workspaceId: WorkspaceId | null) {
  const [state, dispatch] = useReducer(gitReducer, initialGitState);

  useEffect(() => {
    const unsubscribe = core.subscribeAll((event) => dispatch({ type: 'event', event }));
    return unsubscribe;
  }, []);

  useEffect(() => {
    dispatch({ type: 'workspace', workspaceId });
  }, [workspaceId]);

  const loadStatus = useCallback(async (id: WorkspaceId, cursor: string | null) => {
    const append = cursor !== null;
    dispatch({ type: 'statusStarted', append });
    try {
      const request: GitStatusRequest = { workspaceId: id };
      if (cursor !== null) request.cursor = cursor;
      const response = await core.git.status(request);
      if (response.status === 'ready') {
        dispatch({ type: 'statusReady', repository: response.repository, append });
      } else if (response.status === 'notARepository') {
        dispatch({ type: 'statusNotARepository' });
      } else {
        dispatch({ type: 'statusPending', operationId: response.operation.id });
      }
    } catch (cause: unknown) {
      dispatch({ type: 'statusFailed', error: toCoreError(cause) });
    }
  }, []);

  const loadDiff = useCallback(async (id: WorkspaceId, path: WorkspacePath) => {
    dispatch({ type: 'diffStarted' });
    try {
      const request: GitDiffRequest = { workspaceId: id, path };
      const response = await core.git.diff(request);
      if (response.status === 'ready') {
        dispatch({ type: 'diffReady', diff: response.diff });
      } else if (response.status === 'notARepository') {
        dispatch({ type: 'diffNotARepository' });
      } else {
        dispatch({ type: 'diffPending', operationId: response.operation.id });
      }
    } catch (cause: unknown) {
      dispatch({ type: 'diffFailed', error: toCoreError(cause) });
    }
  }, []);

  const loadHistory = useCallback(async (id: WorkspaceId, cursor: string | null) => {
    const append = cursor !== null;
    dispatch({ type: 'logStarted', append });
    try {
      const request: GitLogRequest = { workspaceId: id };
      if (cursor !== null) request.cursor = cursor;
      const response = await core.git.log(request);
      if (response.status === 'ready') {
        dispatch({ type: 'logReady', page: response.page, append });
      } else if (response.status === 'notARepository') {
        dispatch({ type: 'logNotARepository' });
      } else {
        dispatch({ type: 'logPending', operationId: response.operation.id });
      }
    } catch (cause: unknown) {
      dispatch({ type: 'logFailed', error: toCoreError(cause) });
    }
  }, []);

  // The first read runs itself; a failure stays `failed` (with a Retry beside
  // it) rather than looping, which is the explorer's rule for the same reason.
  const statusKind = state.status.kind;
  useEffect(() => {
    if (workspaceId !== null && statusKind === 'idle') void loadStatus(workspaceId, null);
  }, [workspaceId, statusKind, loadStatus]);

  const selected = state.selected;
  const diffKind = state.diff.kind;
  useEffect(() => {
    if (workspaceId !== null && selected !== null && diffKind === 'idle') {
      void loadDiff(workspaceId, selected);
    }
  }, [workspaceId, selected, diffKind, loadDiff]);

  /**
   * History is read on demand, not on mount.
   *
   * The Changes list is what the panel opens on, and walking a history nobody
   * has asked to see would be a second read of every repository on every
   * workspace change. `showHistory` is the ask.
   */
  const historyKind = state.history.kind;
  const showHistory = useCallback(() => {
    if (workspaceId !== null && historyKind === 'idle') void loadHistory(workspaceId, null);
  }, [workspaceId, historyKind, loadHistory]);

  /** Read the repository again from the first page, and the open file with it. */
  const refresh = useCallback(() => {
    if (workspaceId === null) return;
    void loadStatus(workspaceId, null);
    if (selected !== null) void loadDiff(workspaceId, selected);
    // Only when it has been looked at: a refresh must not start a read the
    // user never asked for.
    if (historyKind !== 'idle') void loadHistory(workspaceId, null);
  }, [workspaceId, selected, historyKind, loadStatus, loadDiff, loadHistory]);

  const loadMore = useCallback(() => {
    const cursor = statusCursor(state);
    if (workspaceId === null || cursor === null || state.appending) return;
    void loadStatus(workspaceId, cursor);
  }, [workspaceId, state, loadStatus]);

  const loadMoreHistory = useCallback(() => {
    const cursor = historyCursor(state);
    if (workspaceId === null || cursor === null || state.appendingHistory) return;
    void loadHistory(workspaceId, cursor);
  }, [workspaceId, state, loadHistory]);

  const select = useCallback(
    (path: WorkspacePath | null) => dispatch({ type: 'select', path }),
    [],
  );

  /** Ask again for the file already selected — the retry after a refused diff. */
  const reloadDiff = useCallback(() => {
    if (workspaceId !== null && selected !== null) void loadDiff(workspaceId, selected);
  }, [workspaceId, selected, loadDiff]);

  /**
   * Cancellation is cooperative and never optimistic: this asks, and the state
   * changes when `OperationCancelled` says it did. A refusal to cancel is
   * reported on the read it belongs to rather than swallowed.
   */
  const cancelStatus = useCallback(() => {
    const operationId = pendingOperation(state.status);
    if (operationId === null) return;
    void core.operations
      .cancel(operationId)
      .catch((cause: unknown) => dispatch({ type: 'statusFailed', error: toCoreError(cause) }));
  }, [state.status]);

  const cancelDiff = useCallback(() => {
    const operationId = pendingOperation(state.diff);
    if (operationId === null) return;
    void core.operations
      .cancel(operationId)
      .catch((cause: unknown) => dispatch({ type: 'diffFailed', error: toCoreError(cause) }));
  }, [state.diff]);

  const cancelHistory = useCallback(() => {
    const operationId = pendingOperation(state.history);
    if (operationId === null) return;
    void core.operations
      .cancel(operationId)
      .catch((cause: unknown) => dispatch({ type: 'logFailed', error: toCoreError(cause) }));
  }, [state.history]);

  return {
    state,
    refresh,
    loadMore,
    select,
    reloadDiff,
    cancelStatus,
    cancelDiff,
    showHistory,
    loadMoreHistory,
    cancelHistory,
  };
}
