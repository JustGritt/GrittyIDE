import {
  DocumentModelRegistry,
  type EditorPreferences,
  type Eol,
  editorOptionsFor,
  isEditableEol,
} from '@gritty/editor';
import {
  type CursorPosition,
  createEditorHost,
  type EditorHost,
  type ScrollState,
  watchColorScheme,
} from '@gritty/editor/monaco';
import type { DocumentId } from '@gritty/protocol';
import { type RefObject, useCallback, useEffect, useRef, useState } from 'react';
import type { DocumentsController } from '../documents/use-documents';
import { type MonacoModel, setupMonaco } from './monaco-setup';

interface Engine {
  readonly host: EditorHost;
  readonly registry: DocumentModelRegistry<MonacoModel>;
}

export interface EditorBinding {
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
  focus(): void;
}

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
  const containerRef = useRef<HTMLElement>(null);
  const [engine, setEngine] = useState<Engine | null>(null);
  const [eols, setEols] = useState<Record<string, Eol>>({});
  const applied = useRef(new Map<DocumentId, number>());
  const detachers = useRef(new Map<DocumentId, () => void>());
  const cursorListeners = useRef(new Set<(position: CursorPosition) => void>());
  const scrollListeners = useRef(new Set<(state: ScrollState) => void>());
  const contentListeners = useRef(new Map<DocumentId, Set<() => void>>());
  const contentSubscriptions = useRef(new Map<DocumentId, { dispose(): void }>());
  const pendingReveal = useRef<{ documentId: DocumentId; position: CursorPosition } | null>(null);
  const engineRef = useRef<Engine | null>(null);
  engineRef.current = engine;
  const documentsRef = useRef(documents);
  documentsRef.current = documents;

  const notifyContent = useCallback((documentId: DocumentId) => {
    for (const listener of [...(contentListeners.current.get(documentId) ?? [])]) listener();
  }, []);

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;
    const registry = new DocumentModelRegistry(setupMonaco());
    const host = createEditorHost(container);
    host.onSave(() => {
      const activeId = documentsRef.current.state.activeId;
      if (activeId !== null) void documentsRef.current.save(activeId);
    });
    const unsubscribeCursor = host.onCursorChange((position) => {
      for (const listener of [...cursorListeners.current]) listener(position);
    });
    const unsubscribeScroll = host.onScrollChange((state) => {
      for (const listener of [...scrollListeners.current]) listener(state);
    });
    const unwatchTheme = watchColorScheme();
    setEngine({ host, registry });
    return () => {
      unwatchTheme();
      unsubscribeCursor();
      unsubscribeScroll();
      for (const detach of detachers.current.values()) detach();
      detachers.current.clear();
      for (const subscription of contentSubscriptions.current.values()) subscription.dispose();
      contentSubscriptions.current.clear();
      applied.current.clear();
      host.dispose();
      registry.disposeAll();
      setEngine(null);
    };
  }, []);

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

    const reveal = pendingReveal.current;
    if (reveal && active && entry && reveal.documentId === active.document.id) {
      pendingReveal.current = null;
      // After the browser laid the (possibly just unhidden) editor out.
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

  const focus = useCallback(() => engine?.host.focus(), [engine]);

  return {
    containerRef,
    eols,
    subscribeCursor,
    subscribeScroll,
    subscribeContent,
    readText,
    revealPosition,
    focus,
  };
}
