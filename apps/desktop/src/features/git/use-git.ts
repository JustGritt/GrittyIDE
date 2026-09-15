import {
  CoreError,
  type GitCommitRequest,
  type GitDiffRequest,
  type GitDiscardRequest,
  type GitLogRequest,
  type GitStatusRequest,
  type GitWriteResponse,
  toCoreError,
  type WorkspaceId,
  type WorkspacePath,
} from '@gritty/protocol';
import { useCallback, useEffect, useReducer, useRef } from 'react';
import { core } from '../../core/client';
import { isDirtyDocumentRefusal } from '../explorer/use-explorer';
import { writeUnavailableReason } from './git-model';
import {
  type CommitDraft,
  gitReducer,
  historyCursor,
  initialGitState,
  pendingOperation,
  statusCursor,
  type WriteRequest,
  writeBusy,
} from './git-reducer';

/** The Changes tab as its one consumer sees it. */
export type GitController = ReturnType<typeof useGit>;

/**
 * Commit messages typed and not yet committed, by workspace.
 *
 * The panel that owns this hook unmounts whenever another sidebar tab is
 * showing, and its reducer goes with it. A message typed, then a glance at the
 * file tree, then back — or a hook that refuses while the tab is hidden — must
 * not cost the user what they wrote (T102-U, failure behaviour). Module scope,
 * so it lives exactly as long as the window, and is never written to disk: a
 * draft is not session state T060 promised to restore.
 */
const drafts = new Map<WorkspaceId, string>();

/** One write on the wire. The request shapes differ only in their options. */
function send(workspaceId: WorkspaceId, request: WriteRequest): Promise<GitWriteResponse> {
  switch (request.verb) {
    case 'stage':
      return core.git.stage({ workspaceId, paths: [...request.paths] });
    case 'unstage':
      return core.git.unstage({ workspaceId, paths: [...request.paths] });
    case 'discard': {
      const discard: GitDiscardRequest = { workspaceId, paths: [...request.paths] };
      if (request.force) discard.force = true;
      return core.git.discard(discard);
    }
    case 'commit': {
      const commit: GitCommitRequest = { workspaceId, message: request.message };
      if (request.amend) commit.amend = true;
      if (request.noVerify) commit.noVerify = true;
      return core.git.commit(commit);
    }
  }
}

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
    const message = workspaceId !== null ? drafts.get(workspaceId) : undefined;
    dispatch({ type: 'workspace', workspaceId, message });
  }, [workspaceId]);

  // Keyed on the reducer's workspace, not the prop: on the first render the
  // reducer has not taken the workspace yet, and saving its empty draft under
  // the prop's id would erase the one being restored.
  const draftWorkspace = state.workspaceId;
  const draftMessage = state.draft.message;
  useEffect(() => {
    if (draftWorkspace === null) return;
    if (draftMessage === '') drafts.delete(draftWorkspace);
    else drafts.set(draftWorkspace, draftMessage);
  }, [draftWorkspace, draftMessage]);

  const loadStatus = useCallback(async (id: WorkspaceId, cursor: string | null) => {
    const append = cursor !== null;
    dispatch({ type: 'statusStarted', append });
    try {
      const request: GitStatusRequest = { workspaceId: id };
      if (cursor !== null) request.cursor = cursor;
      const response = await core.git.status(request);
      if (response.status === 'ready') {
        dispatch({
          type: 'statusReady',
          repository: response.repository,
          append,
          write: response.write,
        });
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

  /**
   * Send one write, unless one is already in flight or `git` cannot write here.
   *
   * Refused rather than queued: two writes racing for one index is how
   * `index.lock` refusals get manufactured, and the affordances are disabled
   * while `busy` anyway — this is the backstop, not the policy.
   *
   * `busy` alone is not enough, because it is read from the last render: two
   * clicks inside one frame both see "not busy", and the reducer ignoring the
   * second start does not unsend its request — a double-click on Commit with
   * Amend ticked would amend twice. `sending` is written synchronously, so the
   * second click finds it set. Once the response is in, `busy` has taken over.
   */
  const busy = writeBusy(state.write);
  const unavailable = writeUnavailableReason(state.availability) !== null;
  const sending = useRef(false);
  const runWrite = useCallback(
    async (request: WriteRequest) => {
      if (workspaceId === null || busy || unavailable || sending.current) return;
      if (request.verb !== 'commit' && request.paths.length === 0) return;
      sending.current = true;
      dispatch({ type: 'writeStarted', request });
      try {
        const response = await send(workspaceId, request);
        if (response.status === 'ready') {
          dispatch({ type: 'writeReady', result: response.result });
        } else if (response.status === 'notARepository') {
          dispatch({ type: 'writeNotARepository' });
        } else {
          dispatch({ type: 'writePending', operationId: response.operation.id });
        }
      } catch (cause: unknown) {
        dispatch({ type: 'writeFailed', error: toCoreError(cause) });
      } finally {
        sending.current = false;
      }
    },
    [workspaceId, busy, unavailable],
  );

  const stage = useCallback(
    (paths: readonly WorkspacePath[]) => void runWrite({ verb: 'stage', paths }),
    [runWrite],
  );
  const unstage = useCallback(
    (paths: readonly WorkspacePath[]) => void runWrite({ verb: 'unstage', paths }),
    [runWrite],
  );
  /** Only after the confirmation: nothing here asks. */
  const discard = useCallback(
    (paths: readonly WorkspacePath[]) => void runWrite({ verb: 'discard', paths, force: false }),
    [runWrite],
  );

  const draft = state.draft;
  const commit = useCallback(() => {
    if (draft.message.trim() === '') return;
    void runWrite({
      verb: 'commit',
      message: draft.message,
      amend: draft.amend,
      noVerify: draft.noVerify,
    });
  }, [draft, runWrite]);

  const changeDraft = useCallback(
    (change: Partial<CommitDraft>) => dispatch({ type: 'draftChanged', draft: change }),
    [],
  );

  /**
   * "Discard them and continue" means something only while the write on
   * screen is a discard the core refused over dirty documents. Every other
   * `Conflict` — a held lock, a hook's refusal — is not about unsaved work, and
   * replaying it with `force` would answer a question nobody asked
   * (EXPLORER-DISCARD-NOTICE).
   */
  const write = state.write;
  const canDiscardAndRetry =
    write.kind === 'failed' &&
    write.request.verb === 'discard' &&
    isDirtyDocumentRefusal(new CoreError(write.error));

  /** Replay the refused discard with `force`: the user chose to drop the buffers. */
  const discardAndRetry = useCallback(() => {
    if (!canDiscardAndRetry || write.kind !== 'failed' || write.request.verb !== 'discard') return;
    void runWrite({ ...write.request, force: true });
  }, [canDiscardAndRetry, write, runWrite]);

  /** Send the refused request again unchanged — offered only when the core says it may help. */
  const retryWrite = useCallback(() => {
    if (write.kind !== 'failed' || !write.error.retryable) return;
    void runWrite(write.request);
  }, [write, runWrite]);

  const dismissWrite = useCallback(() => dispatch({ type: 'writeDismissed' }), []);

  /**
   * Stop a commit whose hooks are running. The core kills the hook's process
   * group; the outcome arrives as `OperationCancelled`, never optimistically.
   */
  const cancelWrite = useCallback(() => {
    const operationId = pendingOperation(write);
    if (operationId === null) return;
    void core.operations
      .cancel(operationId)
      .catch((cause: unknown) => dispatch({ type: 'writeFailed', error: toCoreError(cause) }));
  }, [write]);

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
    stage,
    unstage,
    discard,
    commit,
    changeDraft,
    canDiscardAndRetry,
    discardAndRetry,
    retryWrite,
    dismissWrite,
    cancelWrite,
  };
}
