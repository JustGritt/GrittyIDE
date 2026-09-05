import {
  type EditorPreferences,
  parseEditorPreferences,
  serializeEditorPreferences,
} from '@gritty/editor';
import {
  type RecoveryCandidate,
  type SessionTab,
  toCoreError,
  type WorkspaceId,
} from '@gritty/protocol';
import { useCallback, useEffect, useReducer, useRef, useState } from 'react';
import { core } from '../../core/client';
import type { DocumentsController } from '../documents/use-documents';
import type { EditorBinding } from '../editor/use-editor-binding';
import type { ViewModes, ViewModesAction } from '../markdown/view-modes';
import { buildSessionTabs, type Layout, parseLayout, serializeLayout } from './persisted-layout';
import { applyTabs, type RestoreOutcome, runRestore } from './restore-flow';
import {
  initialSessionState,
  pendingChoices,
  type SessionState,
  sessionReducer,
} from './session-reducer';
import { SETTINGS_DELAY_MS, SessionWriter, TABS_DELAY_MS, VIEW_DELAY_MS } from './session-writer';

export interface SessionDeps {
  readonly documents: DocumentsController;
  readonly editor: EditorBinding;
  readonly prefs: EditorPreferences;
  setPrefs(prefs: EditorPreferences): void;
  readonly viewModes: ViewModes;
  dispatchViewMode(action: ViewModesAction): void;
  readonly layout: Layout;
  setLayout(layout: Layout): void;
}

export interface SessionController {
  readonly state: SessionState;
  /** Recover every same-as-disk candidate still waiting (the prompt's default). */
  restoreAll(): void;
  discardAll(): void;
  recover(path: string): void;
  discard(path: string): void;
  dismissNotice(index: number): void;
  dismissSkipped(): void;
  /**
   * Put a workspace's persisted tabs back after it was opened by hand (T051).
   * The startup restore runs once per WebView, so a workspace opened later
   * cannot go through it; this is the same code path, on demand.
   */
  resumeTabs(workspaceId: WorkspaceId, tabs: readonly SessionTab[]): Promise<void>;
}

/**
 * One restore per WebView lifetime: React StrictMode runs effects twice in
 * development, and a second `session.restore` would re-open tabs the first
 * one is still opening.
 */
let restoreInFlight: Promise<RestoreOutcome> | null = null;

/** Tests only. */
export function resetSessionRestore(): void {
  restoreInFlight = null;
}

/**
 * The session layer of the WebView (T060): runs the startup restore, keeps
 * what the UI owns persisted through a coalescing writer, loads the recovery
 * candidates of every workspace that opens, recovers changed/missing ones by
 * itself and asks about the rest, and surfaces persistence notices.
 */
export function useSession(deps: SessionDeps): SessionController {
  const [state, dispatch] = useReducer(sessionReducer, initialSessionState);
  const depsRef = useRef(deps);
  depsRef.current = deps;
  const workspaceRef = useRef<WorkspaceId | null>(null);
  const candidatesFor = useRef<WorkspaceId | null>(null);

  const [writer] = useState(
    () =>
      new SessionWriter({
        snapshot: () => {
          const { documents, viewModes, editor, prefs, layout } = depsRef.current;
          const workspaceId = workspaceRef.current;
          const tabs = workspaceId
            ? buildSessionTabs({
                workspaceId,
                order: documents.state.order,
                tabs: documents.state.tabs,
                activeId: documents.state.activeId,
                viewModes,
                readViewState: editor.readViewState,
              })
            : undefined;
          return {
            ...(workspaceId ? { workspaceId } : {}),
            ...(tabs ? { tabs } : {}),
            preferences: serializeEditorPreferences(prefs),
            layout: serializeLayout(layout),
          };
        },
        send: (request) => core.session.update(request),
        onError: (error) => {
          console.warn('[gritty session] update failed', toCoreError(error));
        },
      }),
  );

  const recoverOne = useCallback(
    async (workspaceId: WorkspaceId, path: string, activate: boolean) => {
      dispatch({ type: 'candidateStarted', path });
      try {
        const content = await core.session.recover({ workspaceId, path });
        depsRef.current.documents.adopt(content, activate);
        dispatch({ type: 'candidateRemoved', path });
      } catch (cause: unknown) {
        dispatch({ type: 'candidateFailed', path, message: toCoreError(cause).message });
      }
    },
    [],
  );

  const discardOne = useCallback(async (workspaceId: WorkspaceId, path: string) => {
    try {
      await core.session.discard({ workspaceId, path });
      dispatch({ type: 'candidateRemoved', path });
    } catch (cause: unknown) {
      dispatch({ type: 'candidateFailed', path, message: toCoreError(cause).message });
    }
  }, []);

  const loadCandidates = useCallback(
    async (workspaceId: WorkspaceId) => {
      if (candidatesFor.current === workspaceId) return;
      candidatesFor.current = workspaceId;
      let candidates: RecoveryCandidate[];
      try {
        candidates = await core.session.candidates({ workspaceId });
      } catch (cause: unknown) {
        console.warn('[gritty session] candidates failed', toCoreError(cause));
        return;
      }
      if (candidatesFor.current !== workspaceId) return;
      dispatch({ type: 'candidatesLoaded', workspaceId, candidates });
      // A file that moved on or off the disk is recovered right away: nothing
      // touches the disk, and the conflict/deleted banners are the UI for it.
      for (const candidate of candidates) {
        if (candidate.state === 'SAME_AS_DISK') continue;
        if (candidatesFor.current !== workspaceId) return;
        await recoverOne(workspaceId, candidate.path, false);
      }
    },
    [recoverOne],
  );

  // Startup: events first, then the single-flight restore.
  useEffect(() => {
    let mounted = true;
    const unsubscribe = core.subscribeAll((event) => {
      dispatch({ type: 'event', event });
      if (event.name === 'WorkspaceOpened') {
        workspaceRef.current = event.payload.workspace.id;
        void loadCandidates(event.payload.workspace.id);
      } else if (event.name === 'WorkspaceClosed') {
        if (workspaceRef.current === event.payload.workspaceId) workspaceRef.current = null;
        if (candidatesFor.current === event.payload.workspaceId) candidatesFor.current = null;
      }
    });
    const first = restoreInFlight === null;
    if (restoreInFlight === null) {
      restoreInFlight = runRestore({
        restore: () => core.session.restore(),
        rehydrate: () => depsRef.current.documents.rehydrate(),
        openFile: (workspaceId, path) =>
          depsRef.current.documents.openFile(workspaceId, path, { quiet: true }),
        applyPreferences: (value) => depsRef.current.setPrefs(parseEditorPreferences(value)),
        applyLayout: (value) => depsRef.current.setLayout(parseLayout(value)),
        applyOrder: (ids, activeId) => {
          const { documents } = depsRef.current;
          for (const [index, id] of ids.entries()) documents.reorder(id, index);
          if (activeId !== null) documents.activate(activeId);
        },
        setViewMode: (documentId, mode) =>
          depsRef.current.dispatchViewMode({ type: 'set', documentId, mode }),
        restoreView: (documentId, view) => depsRef.current.editor.restoreView(documentId, view),
        dispatch,
        toCoreError,
      });
    }
    restoreInFlight
      .then((outcome) => {
        if (!mounted) return;
        if (!first) {
          // A later mount joins the finished restore.
          if (outcome.response) dispatch({ type: 'restoreFinished', response: outcome.response });
          dispatch({ type: 'restoreSettled', skipped: outcome.skipped });
        }
        const workspace = outcome.response?.workspace;
        if (workspace) {
          workspaceRef.current = workspace.id;
          void loadCandidates(workspace.id);
        }
        writer.enable();
        writer.schedule('tabs', TABS_DELAY_MS);
      })
      .catch((cause: unknown) => {
        // `runRestore` reports its own failures; anything else is a bug worth seeing.
        console.error('[gritty session] restore flow failed', cause);
        dispatch({ type: 'restoreSettled', skipped: [] });
        writer.enable();
      });
    return () => {
      mounted = false;
      unsubscribe();
    };
  }, [loadCandidates, writer]);

  // What the UI owns, persisted as it changes (no-ops until the restore settled).
  const { order, activeId } = deps.documents.state;
  const { viewModes, prefs, layout, editor } = deps;
  // biome-ignore lint/correctness/useExhaustiveDependencies: the write is *caused* by these values changing; the snapshot reads them through the ref at send time
  useEffect(() => {
    writer.schedule('tabs', TABS_DELAY_MS);
  }, [writer, order, activeId, viewModes]);
  useEffect(
    () => editor.subscribeViewChange(() => writer.schedule('tabs', VIEW_DELAY_MS)),
    [writer, editor.subscribeViewChange],
  );
  // biome-ignore lint/correctness/useExhaustiveDependencies: same as above, for the preferences
  useEffect(() => {
    writer.schedule('preferences', SETTINGS_DELAY_MS);
  }, [writer, prefs]);
  // biome-ignore lint/correctness/useExhaustiveDependencies: same as above, for the layout
  useEffect(() => {
    writer.schedule('layout', SETTINGS_DELAY_MS);
  }, [writer, layout]);
  useEffect(() => {
    const flush = () => void writer.flush();
    window.addEventListener('pagehide', flush);
    return () => {
      window.removeEventListener('pagehide', flush);
      void writer.flush();
    };
  }, [writer]);

  const stateRef = useRef(state);
  stateRef.current = state;

  const restoreAll = useCallback(() => {
    const candidates = stateRef.current.candidates;
    if (!candidates) return;
    const waiting = pendingChoices(stateRef.current);
    void (async () => {
      for (const [index, item] of waiting.entries()) {
        await recoverOne(candidates.workspaceId, item.path, index === 0);
      }
    })();
  }, [recoverOne]);

  const discardAll = useCallback(() => {
    const candidates = stateRef.current.candidates;
    if (!candidates) return;
    const waiting = pendingChoices(stateRef.current);
    void (async () => {
      for (const item of waiting) await discardOne(candidates.workspaceId, item.path);
    })();
  }, [discardOne]);

  const recover = useCallback(
    (path: string) => {
      const candidates = stateRef.current.candidates;
      if (candidates) void recoverOne(candidates.workspaceId, path, true);
    },
    [recoverOne],
  );

  const discard = useCallback(
    (path: string) => {
      const candidates = stateRef.current.candidates;
      if (candidates) void discardOne(candidates.workspaceId, path);
    },
    [discardOne],
  );

  const dismissNotice = useCallback(
    (index: number) => dispatch({ type: 'dismissNotice', index }),
    [],
  );
  const dismissSkipped = useCallback(() => dispatch({ type: 'dismissSkipped' }), []);

  /**
   * Reopen a set of persisted tabs into a workspace that was just opened by
   * hand (T051, the launcher's resume rail).
   *
   * The startup restore runs once per WebView, so a workspace opened later
   * cannot go through it; this is the same `applyTabs` it uses, called on
   * demand. Tabs whose file has gone are reported the same way too.
   */
  const resumeTabs = useCallback(async (workspaceId: WorkspaceId, tabs: readonly SessionTab[]) => {
    const { skipped } = await applyTabs(
      {
        openFile: (id, path) => depsRef.current.documents.openFile(id, path, { quiet: true }),
        applyOrder: (ids, activeId) => {
          const { documents } = depsRef.current;
          for (const [index, id] of ids.entries()) documents.reorder(id, index);
          if (activeId !== null) documents.activate(activeId);
        },
        setViewMode: (documentId, mode) =>
          depsRef.current.dispatchViewMode({ type: 'set', documentId, mode }),
        restoreView: (documentId, view) => depsRef.current.editor.restoreView(documentId, view),
      },
      workspaceId,
      tabs,
    );
    if (skipped.length > 0) dispatch({ type: 'restoreSettled', skipped });
  }, []);

  return {
    state,
    restoreAll,
    discardAll,
    recover,
    discard,
    dismissNotice,
    dismissSkipped,
    resumeTabs,
  };
}
