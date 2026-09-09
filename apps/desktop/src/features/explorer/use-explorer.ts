import {
  type CoreError,
  type DeleteMode,
  type FileOperationResponse,
  type FileOperationResult,
  type ListDirectoryRequest,
  type NewEntryKind,
  type TransferMode,
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
import type { Selection } from './explorer-selection';

/**
 * What a cut or copy is holding (T042). `MOVE` empties itself on paste, the way
 * a cut does everywhere; `COPY` stays, so one copy can be pasted repeatedly.
 */
export interface FileClipboard {
  readonly paths: readonly string[];
  readonly mode: TransferMode;
}

export interface ExplorerOptions {
  /** Controlled by the session layout (T060); the local state follows it. */
  readonly showHidden?: boolean;
  onShowHiddenChange?(showHidden: boolean): void;
}

/**
 * An action the core refused with `Conflict` because a document under it is
 * dirty, held so the notice can replay it with `force`.
 *
 * Three commands can earn that refusal — `workspace.open`, `workspace.close`
 * and `files.delete` — and each replays differently, so the kind travels with
 * the arguments the replay needs.
 */
type Refusal =
  | { kind: 'open'; path: string | undefined }
  | { kind: 'close' }
  | { kind: 'delete'; paths: readonly string[]; mode: DeleteMode };

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
  /**
   * The action the core refused with `Conflict` (dirty documents), replayable
   * with `force`.
   *
   * **One slot, not one per command.** The notice that offers the replay is a
   * single button, so it has to replay *the* refusal; two slots meant it had to
   * guess which, and it guessed wrong — `deleteAnyway` was defined, returned
   * and never called, while the notice wired the open/close replay for every
   * `Conflict` including a delete's (EXPLORER-FORCE-REPLAY). Last writer wins,
   * which is exactly "what the core most recently refused", and every action
   * clears it on success.
   */
  const refused = useRef<Refusal | null>(null);

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

  /**
   * Re-list what the watcher (T070) invalidated, with the same call a manual
   * reload makes. The reducer named the nodes; this runs them, and takes them
   * off the list first so a batch arriving mid-flight is not lost with them.
   *
   * Nothing new is subscribed to: `WorkspaceEntriesChanged` already reaches
   * the reducer through the one `subscribeAll` above.
   */
  const stale = state.stale;
  const refreshing = useRef<readonly string[] | null>(null);
  useEffect(() => {
    if (!workspace || stale.length === 0 || refreshing.current === stale) return;
    // React may run this effect twice for one state (StrictMode); the identity
    // guard keeps that from listing every directory twice.
    refreshing.current = stale;
    dispatch({ type: 'refreshing', paths: stale });
    for (const path of stale) void load(workspace.id, path, null);
  }, [workspace, stale, load]);

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

  // ---- Changing the tree (T042) ----

  const setSelection = useCallback(
    (selection: Selection) => dispatch({ type: 'selection', selection }),
    [],
  );

  /**
   * What a cut or copy is holding. Renderer state on purpose: the core has no
   * clipboard, and a pending paste is not a fact about the filesystem.
   */
  const [clipboard, setClipboard] = useState<FileClipboard | null>(null);

  /** What ⌘Z would reverse, as the core last reported it. */
  const [history, setHistory] = useState<{ undo?: string; redo?: string }>({});

  /** Take a command's answer: re-list what it changed, and remember the labels. */
  const settle = useCallback((result: FileOperationResult | undefined) => {
    if (!result) return undefined;
    if (result.changed.length > 0) {
      dispatch({ type: 'invalidate', paths: result.changed });
    }
    setHistory({ undo: result.undo, redo: result.redo });
    return result;
  }, []);

  /**
   * A `move`/`delete` answers `ready` or `pending`. The pending arm is settled
   * by `OperationCompleted` in the reducer, which carries the same `changed`,
   * so there is nothing to await here.
   */
  const settleResponse = useCallback(
    (response: FileOperationResponse | undefined) => {
      if (!response) return undefined;
      return response.status === 'ready' ? settle(response.result) : undefined;
    },
    [settle],
  );

  const createEntry = useCallback(
    (path: string, kind: NewEntryKind) =>
      run(async () => {
        if (!workspace) return undefined;
        return settle(await core.files.create({ workspaceId: workspace.id, path, kind }));
      }),
    [run, workspace, settle],
  );

  const renameEntry = useCallback(
    (path: string, newName: string) =>
      run(async () => {
        if (!workspace) return undefined;
        return settle(await core.files.rename({ workspaceId: workspace.id, path, newName }));
      }),
    [run, workspace, settle],
  );

  const transfer = useCallback(
    (paths: readonly string[], targetDir: string, mode: TransferMode) =>
      run(async () => {
        if (!workspace || paths.length === 0) return undefined;
        return settleResponse(
          await core.files.move({
            workspaceId: workspace.id,
            paths: [...paths],
            targetDir,
            mode,
          }),
        );
      }),
    [run, workspace, settleResponse],
  );

  const deleteEntries = useCallback(
    (paths: readonly string[], mode: DeleteMode, force = false) =>
      run(async () => {
        if (!workspace || paths.length === 0) return undefined;
        refused.current = { kind: 'delete', paths, mode };
        const response = await core.files.delete({
          workspaceId: workspace.id,
          paths: [...paths],
          mode,
          ...(force ? { force: true } : {}),
        });
        refused.current = null;
        return settleResponse(response);
      }),
    [run, workspace, settleResponse],
  );

  /**
   * Replay whichever action the core refused, with `force`: the user chose to
   * drop the unsaved changes.
   *
   * Declared after every action it can replay, because it calls them.
   */
  const discardAndRetry = useCallback(async () => {
    const last = refused.current;
    if (!last) return;
    if (last.kind === 'close') await closeWorkspace(true);
    else if (last.kind === 'open') await openFolder(last.path, true);
    else await deleteEntries(last.paths, last.mode, true);
  }, [closeWorkspace, openFolder, deleteEntries]);

  /** Show an entry in the system file manager. Resolved core-side. */
  const revealInFileManager = useCallback(
    (path: string) =>
      run(async () => {
        if (!workspace) return undefined;
        await core.shell.revealPath({ workspaceId: workspace.id, path });
        return undefined;
      }),
    [run, workspace],
  );

  const undo = useCallback(
    () =>
      run(async () => {
        if (!workspace) return undefined;
        return settle(await core.files.undo(workspace.id));
      }),
    [run, workspace, settle],
  );

  const redo = useCallback(
    () =>
      run(async () => {
        if (!workspace) return undefined;
        return settle(await core.files.redo(workspace.id));
      }),
    [run, workspace, settle],
  );

  /** Paste what the clipboard holds into `targetDir`. A cut is consumed. */
  const paste = useCallback(
    async (targetDir: string) => {
      if (!clipboard) return;
      const result = await transfer(clipboard.paths, targetDir, clipboard.mode);
      if (result && clipboard.mode === 'MOVE') setClipboard(null);
    },
    [clipboard, transfer],
  );

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
    // T042
    setSelection,
    clipboard,
    setClipboard,
    history,
    createEntry,
    renameEntry,
    transfer,
    deleteEntries,
    revealInFileManager,
    paste,
    undo,
    redo,
  };
}
