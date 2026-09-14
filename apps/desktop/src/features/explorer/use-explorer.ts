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
 * How many stale directories the explorer re-lists at once
 * (EXPLORER-RELIST-QUEUE). Enough that a handful of expanded folders refresh
 * together; few enough that a checkout with a hundred of them expanded does
 * not become a hundred concurrent listings.
 */
export const RELIST_CONCURRENCY = 4;

/** `paths` ordered root first, then by depth; ties keep their order. */
function shallowestFirst(paths: readonly string[]): string[] {
  const depth = (path: string) => (path === '' ? 0 : path.split('/').length);
  return [...paths].sort((left, right) => depth(left) - depth(right));
}

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
 * Whether `error` is the core refusing because documents under the action are
 * dirty — the one `Conflict` that replaying with `force` answers.
 *
 * `Conflict` alone is not enough: a move onto a taken name, an empty undo and
 * every `AlreadyExists` are `Conflict` too, and none of them is about unsaved
 * work (EXPLORER-DISCARD-NOTICE).
 */
export function isDirtyDocumentRefusal(error: CoreError | null): boolean {
  if (error?.code !== 'Conflict') return false;
  const dirty = detailsOf(error).dirtyDocuments;
  return Array.isArray(dirty) && dirty.length > 0;
}

/** An error's `details` as an object, or an empty one when it is anything else. */
function detailsOf(error: CoreError): Record<string, unknown> {
  const details = error.details;
  return typeof details === 'object' && details !== null && !Array.isArray(details)
    ? (details as Record<string, unknown>)
    : {};
}

/**
 * `details.reason` of an `Io` error the OS trash caused (the core's
 * `TRASH_REFUSED`): the one I/O failure the explorer answers, by offering to
 * delete permanently instead (TRASH-FALLBACK).
 */
const TRASH_REFUSED = 'TRASH_REFUSED';

/**
 * A Trash delete the OS refused, waiting for the user to choose a permanent
 * delete instead or to keep the files (TRASH-FALLBACK).
 */
export interface TrashRefusal {
  /** What the Trash did not take: the batch from the entry it refused on. */
  readonly paths: readonly string[];
  /** Carried from the refused request, so the dirty-document guard answers the same way. */
  readonly force: boolean;
  /** The core's message, which says why. */
  readonly reason: string;
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
  /**
   * The action the core refused with `Conflict` (dirty documents), replayable
   * with `force`.
   *
   * **One slot, not one per command.** The notice that offers the replay is a
   * single button, so it has to replay *the* refusal; two slots meant it had to
   * guess which, and it guessed wrong — `deleteAnyway` was defined, returned
   * and never called, while the notice wired the open/close replay for every
   * `Conflict` including a delete's (EXPLORER-FORCE-REPLAY). Last writer wins,
   * which is exactly "what the core most recently refused".
   *
   * **Written only by a dirty-document refusal.** It used to be written before
   * sending and cleared on success, so a delete that failed for any other
   * reason stayed armed, and the next unrelated `Conflict` offered to replay it
   * with `force` (EXPLORER-DISCARD-NOTICE). See `refusable`.
   */
  const refused = useRef<Refusal | null>(null);

  /**
   * Send an action the core may refuse over dirty documents, and hold it for
   * the notice only when that is the answer. Success and every other failure
   * leave nothing to replay.
   */
  const refusable = useCallback(async <T>(refusal: Refusal, send: () => Promise<T>) => {
    refused.current = null;
    try {
      return await send();
    } catch (cause: unknown) {
      if (isDirtyDocumentRefusal(toCoreError(cause))) refused.current = refusal;
      throw cause;
    }
  }, []);

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
   * **A few at a time** (EXPLORER-RELIST-QUEUE). An `overflowed` batch, or a
   * file operation that stopped part-way, marks every loaded directory stale at
   * once; listing them all in one tick was one IPC call and one directory read
   * per expanded folder, competing. At most `RELIST_CONCURRENCY` run, the
   * shallowest first — a parent's listing can drop a child that no longer
   * exists before the child is listed — and each settled one frees a slot.
   *
   * A path named again while its listing is in flight stays stale and is
   * listed once more after that listing settles: never twice at once, never
   * dropped.
   *
   * Nothing new is subscribed to: `WorkspaceEntriesChanged` already reaches
   * the reducer through the one `subscribeAll` above.
   */
  const stale = state.stale;
  const inFlight = useRef(new Set<string>());
  const [slotsFreed, setSlotsFreed] = useState(0);
  const openId = workspace?.id;
  useEffect(() => {
    // A new workspace starts with a new set: the old one's listings still
    // settle into the set they were counted in, and cannot hold this one's
    // slots.
    void openId;
    inFlight.current = new Set();
  }, [openId]);
  useEffect(() => {
    void slotsFreed;
    if (!workspace || stale.length === 0) return;
    const flight = inFlight.current;
    const room = RELIST_CONCURRENCY - flight.size;
    if (room <= 0) return;
    // In-flight paths are skipped, not taken: that is also what keeps a
    // second run of this effect for the same state (StrictMode) from listing
    // anything twice.
    const taken = shallowestFirst(stale.filter((path) => !flight.has(path))).slice(0, room);
    if (taken.length === 0) return;
    for (const path of taken) flight.add(path);
    dispatch({ type: 'refreshing', paths: taken });
    for (const path of taken) {
      void load(workspace.id, path, null).finally(() => {
        flight.delete(path);
        setSlotsFreed((count) => count + 1);
      });
    }
  }, [workspace, stale, load, slotsFreed]);

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
        const opened = await refusable({ kind: 'open', path }, () =>
          core.workspace.open(path, force ? { force } : undefined),
        );
        if (opened) dispatch({ type: 'workspace', workspace: opened });
        return opened;
      }),
    [run, refusable],
  );

  const closeWorkspace = useCallback(
    (force = false) =>
      run(async () => {
        if (!workspace) return;
        await refusable({ kind: 'close' }, () =>
          core.workspace.close(workspace.id, force ? { force } : undefined),
        );
        dispatch({ type: 'workspace', workspace: null });
      }),
    [run, workspace, refusable],
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

  /** A Trash delete the OS refused, until the user answers it (TRASH-FALLBACK). */
  const [trashRefusal, setTrashRefusal] = useState<TrashRefusal | null>(null);
  // It names paths in one workspace; another workspace's tree is not what the
  // user would be agreeing to delete.
  const workspaceId = state.workspace?.id;
  useEffect(() => {
    void workspaceId;
    setTrashRefusal(null);
  }, [workspaceId]);

  /** Take a command's answer: re-list what it changed, and remember the labels. */
  const settle = useCallback((result: FileOperationResult | undefined) => {
    if (!result) return undefined;
    if (result.changed.length > 0) {
      dispatch({ type: 'invalidate', paths: result.changed });
    }
    setHistory({ undo: result.undo, redo: result.redo });
    return result;
  }, []);

  /** Re-list every directory the tree has loaded. */
  const refreshAll = useCallback(() => dispatch({ type: 'invalidate', paths: [], all: true }), []);

  /**
   * Send a move or delete, and re-list the whole loaded tree if it failed after
   * doing part of its work (FILES-PARTIAL).
   *
   * The core says so with `details.completed`: what was done stays done and is
   * settled core-side, but an error carries no `changed`, so the renderer
   * cannot know which directories moved under it. A refusal before any work
   * has no `completed`, changed nothing, and re-lists nothing. The error still
   * reaches `run`.
   */
  const relistIfPartial = useCallback(async <T>(send: () => Promise<T>) => {
    try {
      return await send();
    } catch (cause: unknown) {
      const completed = detailsOf(toCoreError(cause)).completed;
      if (typeof completed === 'number') dispatch({ type: 'invalidate', paths: [], all: true });
      throw cause;
    }
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
          await relistIfPartial(() =>
            core.files.move({
              workspaceId: workspace.id,
              paths: [...paths],
              targetDir,
              mode,
            }),
          ),
        );
      }),
    [run, workspace, settleResponse, relistIfPartial],
  );

  const deleteEntries = useCallback(
    (paths: readonly string[], mode: DeleteMode, force = false) =>
      run(async () => {
        if (!workspace || paths.length === 0) return undefined;
        setTrashRefusal(null);
        try {
          const response = await refusable({ kind: 'delete', paths, mode }, () =>
            relistIfPartial(() =>
              core.files.delete({
                workspaceId: workspace.id,
                paths: [...paths],
                mode,
                ...(force ? { force: true } : {}),
              }),
            ),
          );
          return settleResponse(response);
        } catch (cause: unknown) {
          const error = toCoreError(cause);
          const details = detailsOf(error);
          if (mode !== 'TRASH' || error.code !== 'Io' || details.reason !== TRASH_REFUSED) {
            throw cause;
          }
          // Not an error to show: a question to ask. The entries before the
          // refused one are already in the Trash (FILES-PARTIAL); the rest are
          // what a permanent delete would take.
          const completed = typeof details.completed === 'number' ? details.completed : 0;
          setTrashRefusal({ paths: paths.slice(completed), force, reason: error.message });
          return undefined;
        }
      }),
    [run, workspace, settleResponse, refusable, relistIfPartial],
  );

  /**
   * Answer a Trash refusal by deleting what is left permanently. Only ever
   * reached from the confirmation dialog, which says it cannot be undone.
   */
  const deletePermanentlyInstead = useCallback(async () => {
    const refusal = trashRefusal;
    if (!refusal) return;
    setTrashRefusal(null);
    await deleteEntries(refusal.paths, 'PERMANENT', refusal.force);
  }, [trashRefusal, deleteEntries]);

  /** Answer a Trash refusal by keeping the files. Sends nothing. */
  const dismissTrashRefusal = useCallback(() => setTrashRefusal(null), []);

  /**
   * Whether "discard them and continue" means something right now: the error
   * showing is a dirty-document refusal, and the slot holds the action that
   * earned it. Every action that can write the slot empties it before sending,
   * and only those three can earn this error, so the two cannot disagree.
   */
  const canDiscardAndRetry = isDirtyDocumentRefusal(error) && refused.current !== null;

  /**
   * Replay whichever action the core refused, with `force`: the user chose to
   * drop the unsaved changes.
   *
   * Declared after every action it can replay, because it calls them.
   */
  const discardAndRetry = useCallback(async () => {
    const last = refused.current;
    // Only while the refusal is the error on screen: a replay reached for any
    // other reason would force an action the user did not just see refused.
    if (!last || !canDiscardAndRetry) return;
    refused.current = null;
    if (last.kind === 'close') await closeWorkspace(true);
    else if (last.kind === 'open') await openFolder(last.path, true);
    else await deleteEntries(last.paths, last.mode, true);
  }, [canDiscardAndRetry, closeWorkspace, openFolder, deleteEntries]);

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
    canDiscardAndRetry,
    discardAndRetry,
    cancelListing,
    setShowHidden,
    dismissNotice,
    clearError,
    refreshAll,
    // T042
    setSelection,
    clipboard,
    setClipboard,
    history,
    createEntry,
    renameEntry,
    transfer,
    deleteEntries,
    trashRefusal,
    deletePermanentlyInstead,
    dismissTrashRefusal,
    revealInFileManager,
    paste,
    undo,
    redo,
  };
}
