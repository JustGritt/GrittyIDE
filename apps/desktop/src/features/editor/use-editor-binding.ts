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
  const documentsRef = useRef(documents);
  documentsRef.current = documents;

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
    const unwatchTheme = watchColorScheme();
    setEngine({ host, registry });
    return () => {
      unwatchTheme();
      unsubscribeCursor();
      for (const detach of detachers.current.values()) detach();
      detachers.current.clear();
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

    const next: Record<string, Eol> = {};
    for (const id of order) {
      const known = registry.get(id);
      if (known) next[id] = known.eol;
    }
    setEols((previous) => (sameEols(previous, next) ? previous : next));
  }, [engine, documents.state, prefs]);

  useEffect(() => {
    engine?.registry.setTabSize(prefs.tabSize);
  }, [engine, prefs.tabSize]);

  const subscribeCursor = useCallback((listener: (position: CursorPosition) => void) => {
    cursorListeners.current.add(listener);
    return () => {
      cursorListeners.current.delete(listener);
    };
  }, []);

  const focus = useCallback(() => engine?.host.focus(), [engine]);

  return { containerRef, eols, subscribeCursor, focus };
}
