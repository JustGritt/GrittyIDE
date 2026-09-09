import {
  DocumentModelRegistry,
  type EditorPreferences,
  type EditorViewState,
  type Eol,
  editorOptionsFor,
  INITIAL_VIEW_STATE,
  isEditableEol,
} from '@gritty/editor';
import type { CursorPosition, EditorHost, ScrollState } from '@gritty/editor/monaco';
import type { DocumentId } from '@gritty/protocol';
import { type RefObject, useCallback, useEffect, useRef, useState } from 'react';
import type { DocumentsController } from '../documents/use-documents';
import type { MonacoModel } from './monaco-setup';

interface Engine {
  readonly host: EditorHost;
  readonly registry: DocumentModelRegistry<MonacoModel>;
}

export interface EditorBinding {
  /**
   * Ref for the pane's editor surface. A callback, not a ref object: the pane
   * mounts and unmounts with the workspace and the attention queue, and the
   * host has to follow that rather than be built once against whatever was on
   * screen when the window opened.
   */
  attachContainer(element: HTMLElement | null): void;
  /** The attached surface, for hit-testing focus; `null` while the pane is unmounted. */
  readonly containerRef: RefObject<HTMLElement | null>;
  /** Line endings of each open document as last received from the core. */
  readonly eols: Readonly<Record<string, Eol>>;
  subscribeCursor(listener: (position: CursorPosition) => void): () => void;
  subscribeScroll(listener: (state: ScrollState) => void): () => void;
  /**
   * Notified on every content change of the document's model, and once when
   * the model appears (a child's effect may subscribe before this binding's
   * effect created it). Derived UI only: the preview reads, never writes.
   */
  subscribeContent(documentId: DocumentId, listener: () => void): () => void;
  /** The model's current text, or `null` when the document has no model yet. */
  readText(documentId: DocumentId): string | null;
  /** Move the cursor once the document is shown (now, or after it becomes active). */
  revealPosition(documentId: DocumentId, position: CursorPosition): void;
  /**
   * The last known plain view state of a document (cursor, selection anchor,
   * first visible line), for the session (T060); `null` before anything is known.
   */
  readViewState(documentId: DocumentId): EditorViewState | null;
  /** Put a persisted view state back once the document is shown (now, or when it becomes active). */
  restoreView(documentId: DocumentId, state: EditorViewState): void;
  /** Fires after any cursor, selection or scroll change of the active document. */
  subscribeViewChange(listener: () => void): () => void;
  focus(): void;
  /**
   * Monaco is fetched on demand rather than at startup (MONACO-DEFER), so the
   * pane has three states to render, not two: still arriving, usable, or a
   * module that never loaded.
   */
  readonly engineStatus: EngineStatus;
}

/** `loading` until the Monaco chunk resolves; `failed` if it never does. */
export type EngineStatus = 'loading' | 'ready' | 'failed';

function sameEols(a: Record<string, Eol>, b: Record<string, Eol>): boolean {
  const keys = Object.keys(a);
  if (keys.length !== Object.keys(b).length) return false;
  return keys.every((key) => a[key] === b[key]);
}

/**
 * Glue between the document store and Monaco. Text enters a model only when
 * a tab's `bufferGeneration` moves (open, reload, refetch, replacement);
 * everything typed leaves through the reader the store asked for. Monaco
 * never decides what is dirty.
 */
export function useEditorBinding(
  documents: DocumentsController,
  prefs: EditorPreferences,
): EditorBinding {
  const containerRef = useRef<HTMLElement | null>(null);
  const [container, setContainer] = useState<HTMLElement | null>(null);
  /**
   * Monaco's own element, owned by this hook rather than by the pane. It is
   * moved into whichever surface is on screen, so one editor — with its
   * models, undo history, view states and unread keystrokes — outlives every
   * mount and unmount of the pane.
   */
  const surfaceRef = useRef<HTMLDivElement | null>(null);
  const [engine, setEngine] = useState<Engine | null>(null);
  const [engineStatus, setEngineStatus] = useState<EngineStatus>('loading');
  const [eols, setEols] = useState<Record<string, Eol>>({});
  const applied = useRef(new Map<DocumentId, number>());
  const detachers = useRef(new Map<DocumentId, () => void>());
  const cursorListeners = useRef(new Set<(position: CursorPosition) => void>());
  const scrollListeners = useRef(new Set<(state: ScrollState) => void>());
  const contentListeners = useRef(new Map<DocumentId, Set<() => void>>());
  const contentSubscriptions = useRef(new Map<DocumentId, { dispose(): void }>());
  const pendingReveal = useRef<{ documentId: DocumentId; position: CursorPosition } | null>(null);
  const viewStates = useRef(new Map<DocumentId, EditorViewState>());
  const pendingViews = useRef(new Map<DocumentId, EditorViewState>());
  const viewListeners = useRef(new Set<() => void>());
  const engineRef = useRef<Engine | null>(null);
  engineRef.current = engine;
  const documentsRef = useRef(documents);
  documentsRef.current = documents;

  const notifyContent = useCallback((documentId: DocumentId) => {
    for (const listener of [...(contentListeners.current.get(documentId) ?? [])]) listener();
  }, []);

  /** Record a change of the active document's view state and tell the session. */
  const trackView = useCallback((update: (previous: EditorViewState) => EditorViewState) => {
    const activeId = documentsRef.current.state.activeId;
    if (activeId === null) return;
    const previous = viewStates.current.get(activeId) ?? INITIAL_VIEW_STATE;
    viewStates.current.set(activeId, update(previous));
    for (const listener of [...viewListeners.current]) listener();
  }, []);

  const attachContainer = useCallback((element: HTMLElement | null) => {
    containerRef.current = element;
    setContainer(element);
  }, []);

  useEffect(() => {
    const surface = document.createElement('div');
    surface.className = 'editor__monaco-surface';
    surfaceRef.current = surface;
    // Monaco is ~4.2 MB and the app boots with no document open, so it is
    // fetched here rather than linked into the startup chunk (MONACO-DEFER).
    // The pane may unmount before the import settles: `disposeEngine` stays
    // null until there is something to dispose, and `cancelled` stops a late
    // arrival from building an editor into a surface already removed.
    let cancelled = false;
    let disposeEngine: (() => void) | null = null;

    void (async () => {
      let monaco: typeof import('@gritty/editor/monaco');
      let setup: typeof import('./monaco-setup');
      try {
        [monaco, setup] = await Promise.all([
          import('@gritty/editor/monaco'),
          import('./monaco-setup'),
        ]);
      } catch (error) {
        // A corrupt or missing chunk. The pane says so; the rest of the app —
        // explorer, terminal, git — is untouched by this failure.
        if (!cancelled) {
          console.error('the editor failed to load', error);
          setEngineStatus('failed');
        }
        return;
      }
      if (cancelled) return;
      const { createEditorHost, watchColorScheme } = monaco;
      const registry = new DocumentModelRegistry(setup.setupMonaco());
      const host = createEditorHost(surface);
      host.onSave(() => {
        const activeId = documentsRef.current.state.activeId;
        if (activeId !== null) void documentsRef.current.save(activeId);
      });
      const unsubscribeCursor = host.onCursorChange((position) => {
        for (const listener of [...cursorListeners.current]) listener(position);
      });
      const unsubscribeScroll = host.onScrollChange((state) => {
        for (const listener of [...scrollListeners.current]) listener(state);
        trackView((previous) => ({ ...previous, scrollLine: state.topLine }));
      });
      const unsubscribeSelection = host.onSelectionChange((selection) => {
        trackView((previous) => ({
          ...previous,
          cursor: { line: selection.cursor.lineNumber, column: selection.cursor.column },
          anchor: { line: selection.anchor.lineNumber, column: selection.anchor.column },
        }));
      });
      const unwatchTheme = watchColorScheme();
      disposeEngine = () => {
        unwatchTheme();
        unsubscribeCursor();
        unsubscribeScroll();
        unsubscribeSelection();
        host.dispose();
        registry.disposeAll();
      };
      setEngine({ host, registry });
      setEngineStatus('ready');
    })();

    return () => {
      cancelled = true;
      disposeEngine?.();
      for (const detach of detachers.current.values()) detach();
      detachers.current.clear();
      for (const subscription of contentSubscriptions.current.values()) subscription.dispose();
      contentSubscriptions.current.clear();
      applied.current.clear();
      viewStates.current.clear();
      pendingViews.current.clear();
      surface.remove();
      surfaceRef.current = null;
      setEngine(null);
      setEngineStatus('loading');
    };
  }, [trackView]);

  // Move the editor into the pane whenever one appears, and lay it out: the
  // element was last measured detached (zero-sized), and `automaticLayout`
  // alone would leave the first frame blank.
  useEffect(() => {
    const surface = surfaceRef.current;
    if (!container || !surface || !engine) return;
    container.appendChild(surface);
    engine.host.layout();
    return () => surface.remove();
  }, [container, engine]);

  useEffect(() => {
    if (!engine) return;
    const { host, registry } = engine;
    const { tabs, order, activeId } = documents.state;

    // Documents the store no longer has: models and view states go away.
    for (const id of [...applied.current.keys()]) {
      if (tabs[id]) continue;
      detachers.current.get(id)?.();
      detachers.current.delete(id);
      contentSubscriptions.current.get(id)?.dispose();
      contentSubscriptions.current.delete(id);
      registry.dispose(id);
      host.forgetViewState(id);
      applied.current.delete(id);
      viewStates.current.delete(id);
      pendingViews.current.delete(id);
    }

    for (const id of order) {
      const tab = tabs[id];
      if (!tab || tab.text === null) continue;
      if (!registry.get(id)) {
        const entry = registry.ensure(tab.document, tab.text, {
          largeFileMode: tab.document.largeFileMode,
          tabSize: prefs.tabSize,
          onTouch: () => documentsRef.current.touch(id),
        });
        detachers.current.set(id, documentsRef.current.attach(id, entry.binding.read));
        applied.current.set(id, tab.bufferGeneration);
        contentSubscriptions.current.set(
          id,
          entry.model.onDidChangeContent(() => notifyContent(id)),
        );
        notifyContent(id);
      } else if (applied.current.get(id) !== tab.bufferGeneration) {
        registry.applyCoreText(id, tab.text);
        applied.current.set(id, tab.bufferGeneration);
      }
    }

    const active = activeId === null ? null : (tabs[activeId] ?? null);
    const entry = active ? registry.get(active.document.id) : null;
    host.show(entry?.model ?? null, entry && active ? active.document.id : null);
    if (active && entry) {
      host.applyOptions(
        editorOptionsFor(prefs, {
          readOnly: active.document.readOnly,
          largeFileMode: active.document.largeFileMode,
          editableEol: isEditableEol(entry.eol),
        }),
      );
    }

    const pendingView = active && entry ? pendingViews.current.get(active.document.id) : undefined;
    if (pendingView && active) {
      pendingViews.current.delete(active.document.id);
      // After the browser laid the (possibly just unhidden) editor out.
      requestAnimationFrame(() => host.restoreView(pendingView));
    }
    const reveal = pendingReveal.current;
    if (reveal && active && entry && reveal.documentId === active.document.id) {
      pendingReveal.current = null;
      requestAnimationFrame(() => host.revealPosition(reveal.position));
    }

    const next: Record<string, Eol> = {};
    for (const id of order) {
      const known = registry.get(id);
      if (known) next[id] = known.eol;
    }
    setEols((previous) => (sameEols(previous, next) ? previous : next));
  }, [engine, documents.state, prefs, notifyContent]);

  useEffect(() => {
    engine?.registry.setTabSize(prefs.tabSize);
  }, [engine, prefs.tabSize]);

  const subscribeCursor = useCallback((listener: (position: CursorPosition) => void) => {
    cursorListeners.current.add(listener);
    return () => {
      cursorListeners.current.delete(listener);
    };
  }, []);

  const subscribeScroll = useCallback((listener: (state: ScrollState) => void) => {
    scrollListeners.current.add(listener);
    return () => {
      scrollListeners.current.delete(listener);
    };
  }, []);

  const subscribeContent = useCallback((documentId: DocumentId, listener: () => void) => {
    const listeners = contentListeners.current.get(documentId) ?? new Set();
    listeners.add(listener);
    contentListeners.current.set(documentId, listeners);
    return () => {
      listeners.delete(listener);
      if (listeners.size === 0) contentListeners.current.delete(documentId);
    };
  }, []);

  const readText = useCallback(
    (documentId: DocumentId) => engineRef.current?.registry.read(documentId) ?? null,
    [],
  );

  const revealPosition = useCallback((documentId: DocumentId, position: CursorPosition) => {
    pendingReveal.current = { documentId, position };
    const engine = engineRef.current;
    const activeId = documentsRef.current.state.activeId;
    if (engine && activeId === documentId && engine.registry.get(documentId)) {
      pendingReveal.current = null;
      requestAnimationFrame(() => engine.host.revealPosition(position));
    }
  }, []);

  const readViewState = useCallback(
    (documentId: DocumentId) => viewStates.current.get(documentId) ?? null,
    [],
  );

  const restoreView = useCallback((documentId: DocumentId, state: EditorViewState) => {
    // Known from now on, even for a tab that is never activated.
    viewStates.current.set(documentId, state);
    pendingViews.current.set(documentId, state);
    const engine = engineRef.current;
    const activeId = documentsRef.current.state.activeId;
    if (engine && activeId === documentId && engine.registry.get(documentId)) {
      pendingViews.current.delete(documentId);
      requestAnimationFrame(() => engine.host.restoreView(state));
    }
  }, []);

  const subscribeViewChange = useCallback((listener: () => void) => {
    viewListeners.current.add(listener);
    return () => {
      viewListeners.current.delete(listener);
    };
  }, []);

  const focus = useCallback(() => engine?.host.focus(), [engine]);

  return {
    attachContainer,
    containerRef,
    eols,
    subscribeCursor,
    subscribeScroll,
    subscribeContent,
    readText,
    revealPosition,
    readViewState,
    restoreView,
    subscribeViewChange,
    focus,
    engineStatus,
  };
}
