import {
  type CoreError,
  type ListDirectoryRequest,
  toCoreError,
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
        if (active) dispatch({ type: 'workspace', workspace });
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

  const run = useCallback(async (action: () => Promise<unknown>) => {
    try {
      await action();
      setError(null);
    } catch (cause: unknown) {
      setError(toCoreError(cause));
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

  const openFolder = useCallback(
    (path?: string, force = false) =>
      run(async () => {
        refused.current = { kind: 'open', path };
        const opened = await core.workspace.open(path, force ? { force } : undefined);
        refused.current = null;
        if (opened) dispatch({ type: 'workspace', workspace: opened });
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
  const dismissNotice = useCallback(() => dispatch({ type: 'dismissNotice' }), []);
  const clearError = useCallback(() => setError(null), []);

  return {
    state,
    error,
    toggle,
    reload,
    loadMore,
    openFolder,
    closeWorkspace,
    discardAndRetry,
    cancelListing,
    setShowHidden,
    dismissNotice,
    clearError,
  };
}
