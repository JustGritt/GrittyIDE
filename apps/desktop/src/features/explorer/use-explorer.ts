import {
  type CoreError,
  type ListDirectoryRequest,
  toCoreError,
  type Workspace,
  type WorkspaceId,
} from '@gritty/protocol';
import { useCallback, useEffect, useReducer, useRef, useState } from 'react';
import { core } from '../../core/client';
import {
  type ExplorerNode,
  explorerReducer,
  initialExplorerState,
  isExpandable,
} from './explorer-reducer';

export interface ExplorerOptions {
  /** Controlled by the session layout (T060); the local state follows it. */
  readonly showHidden?: boolean;
  onShowHiddenChange?(showHidden: boolean): void;
}

/**
 * The explorer as its consumers see it. The shell owns the instance (T049):
 * the launcher needs `openFolder` and the top bar needs `state.workspace`, and
 * a second instance would mean a second subscription and a second tree.
 */
export type ExplorerController = ReturnType<typeof useExplorer>;

export function useExplorer(options: ExplorerOptions = {}) {
  const [state, dispatch] = useReducer(explorerReducer, initialExplorerState);
  const { showHidden: controlledShowHidden, onShowHiddenChange } = options;
  useEffect(() => {
    if (controlledShowHidden !== undefined) {
      dispatch({ type: 'setShowHidden', showHidden: controlledShowHidden });
    }
  }, [controlledShowHidden]);
  const [error, setError] = useState<CoreError | null>(null);
  /** The action the core refused with `Conflict` (dirty documents), replayable with force. */
  const refused = useRef<{ kind: 'open'; path: string | undefined } | { kind: 'close' } | null>(
    null,
  );

  useEffect(() => {
    // Subscribe before asking for the current workspace so nothing emitted in
    // between is lost; the reducer treats both sources idempotently.
    const unsubscribe = core.subscribeAll((event) => dispatch({ type: 'event', event }));
    let active = true;
    core.workspace.current().then(
      (workspace) => {
        // A workspace only, never `null`. The state starts empty, so a null
        // snapshot can say nothing new — it can only overwrite one that
        // arrived while this call was in flight, and "nothing was open when
        // this was asked" is not "nothing is open now".
        if (active && workspace) dispatch({ type: 'workspace', workspace });
      },
      (cause: unknown) => {
        if (active) setError(toCoreError(cause));
      },
    );
    return () => {
      active = false;
      unsubscribe();
    };
  }, []);

  const load = useCallback(
    async (workspaceId: WorkspaceId, path: string, cursor: string | null) => {
      const append = cursor !== null;
      dispatch({ type: 'listStarted', path });
      try {
        const request: ListDirectoryRequest = { workspaceId, path };
        if (cursor !== null) request.cursor = cursor;
        const response = await core.workspace.list(request);
        if (response.status === 'ready') {
          dispatch({ type: 'pageLoaded', path, page: response.page, append });
        } else {
          dispatch({ type: 'listPending', path, operationId: response.operation.id, append });
        }
      } catch (cause: unknown) {
        dispatch({ type: 'listFailed', path, error: toCoreError(cause) });
      }
    },
    [],
  );

  // The root loads itself once a workspace is present; a failure stays on the
  // node (with a retry) instead of looping.
  const workspace = state.workspace;
  const root = state.nodes[''];
  useEffect(() => {
    if (workspace && root && root.children === null && !root.loading && root.error === null) {
      void load(workspace.id, '', null);
    }
  }, [workspace, root, load]);

  /** Runs an action, reporting a refusal instead of throwing. `undefined` on failure. */
  const run = useCallback(async <T>(action: () => Promise<T>): Promise<T | undefined> => {
    try {
      const result = await action();
      setError(null);
      return result;
    } catch (cause: unknown) {
      setError(toCoreError(cause));
      return undefined;
    }
  }, []);

  const toggle = useCallback(
    (node: ExplorerNode) => {
      if (!workspace || !isExpandable(node.entry)) return;
      dispatch({ type: 'toggle', path: node.path });
      if (!node.expanded && node.children === null && !node.loading) {
        void load(workspace.id, node.path, null);
      }
    },
    [workspace, load],
  );

  const reload = useCallback(
    (node: ExplorerNode) => {
      if (workspace && !node.loading) void load(workspace.id, node.path, null);
    },
    [workspace, load],
  );

  const loadMore = useCallback(
    (node: ExplorerNode) => {
      if (workspace && node.cursor !== null && !node.loading) {
        void load(workspace.id, node.path, node.cursor);
      }
    },
    [workspace, load],
  );

  /** Returns the workspace that was opened, so a caller can act on it at once
   *  rather than waiting for the state to come back around (T051). */
  const openFolder = useCallback(
    (path?: string, force = false) =>
      run(async () => {
        refused.current = { kind: 'open', path };
        const opened = await core.workspace.open(path, force ? { force } : undefined);
        refused.current = null;
        if (opened) dispatch({ type: 'workspace', workspace: opened });
        return opened;
      }),
    [run],
  );

  const closeWorkspace = useCallback(
    (force = false) =>
      run(async () => {
        if (!workspace) return;
        refused.current = { kind: 'close' };
        await core.workspace.close(workspace.id, force ? { force } : undefined);
        refused.current = null;
        dispatch({ type: 'workspace', workspace: null });
      }),
    [run, workspace],
  );

  /** Replay the refused open/close with `force`: the user chose to drop unsaved changes. */
  const discardAndRetry = useCallback(async () => {
    const last = refused.current;
    if (!last) return;
    if (last.kind === 'close') await closeWorkspace(true);
    else await openFolder(last.path, true);
  }, [closeWorkspace, openFolder]);

  const cancelListing = useCallback(
    (node: ExplorerNode) =>
      run(async () => {
        if (node.pending) await core.operations.cancel(node.pending.operationId);
      }),
    [run],
  );

  const setShowHidden = useCallback(
    (showHidden: boolean) => {
      dispatch({ type: 'setShowHidden', showHidden });
      onShowHiddenChange?.(showHidden);
    },
    [onShowHiddenChange],
  );
  /**
   * Take the workspace a command has already returned instead of waiting for
   * `WorkspaceOpened` to say it again. Two callers need this and neither can
   * rely on the event: the startup restore (T060) is answered by the core
   * before the WebView has finished registering its event listener, and
   * reopening the root that is already open (T045) is a no-op the core
   * announces to nobody. Idempotent — the reducer keeps the state it has when
   * the id matches.
   */
  const adoptWorkspace = useCallback(
    (workspace: Workspace) => dispatch({ type: 'workspace', workspace }),
    [],
  );

  const dismissNotice = useCallback(() => dispatch({ type: 'dismissNotice' }), []);
  const clearError = useCallback(() => setError(null), []);

  return {
    state,
    error,
    toggle,
    reload,
    loadMore,
    openFolder,
    adoptWorkspace,
    closeWorkspace,
    discardAndRetry,
    cancelListing,
    setShowHidden,
    dismissNotice,
    clearError,
  };
}
