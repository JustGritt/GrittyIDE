import type { DocumentId } from '@gritty/protocol';
import * as monaco from 'monaco-editor';
import {
  DEFAULT_EDITOR_PREFERENCES,
  type EditorOptions,
  editorOptionsFor,
} from '../editor-options';

export type EditorTheme = 'vs' | 'vs-dark';

export interface CursorPosition {
  readonly lineNumber: number;
  readonly column: number;
}

/**
 * One Monaco editor instance for the pane. Tabs switch models; the view state
 * (cursor, selection, scroll, folding) of each document is kept per id.
 */
export interface EditorHost {
  show(model: monaco.editor.ITextModel | null, documentId: DocumentId | null): void;
  applyOptions(options: EditorOptions): void;
  focus(): void;
  /** Cmd/Ctrl+S while the editor has focus. */
  onSave(callback: () => void): void;
  onCursorChange(callback: (position: CursorPosition) => void): () => void;
  forgetViewState(documentId: DocumentId): void;
  dispose(): void;
}

export function createEditorHost(container: HTMLElement): EditorHost {
  const editor = monaco.editor.create(container, {
    ...editorOptionsFor(DEFAULT_EDITOR_PREFERENCES, {
      readOnly: false,
      largeFileMode: false,
      editableEol: true,
    }),
    model: null,
  });
  const viewStates = new Map<DocumentId, monaco.editor.ICodeEditorViewState | null>();
  let current: DocumentId | null = null;

  return {
    show(model, documentId) {
      if (current !== null && current !== documentId) {
        viewStates.set(current, editor.saveViewState());
      }
      if (editor.getModel() !== model) editor.setModel(model);
      if (documentId !== null && documentId !== current) {
        const state = viewStates.get(documentId);
        if (state) editor.restoreViewState(state);
      }
      current = documentId;
    },
    applyOptions(options) {
      editor.updateOptions(options);
    },
    focus() {
      editor.focus();
    },
    onSave(callback) {
      editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS, callback);
    },
    onCursorChange(callback) {
      const subscription = editor.onDidChangeCursorPosition((event) => {
        callback({ lineNumber: event.position.lineNumber, column: event.position.column });
      });
      return () => subscription.dispose();
    },
    forgetViewState(documentId) {
      viewStates.delete(documentId);
      if (current === documentId) {
        editor.setModel(null);
        current = null;
      }
    },
    dispose() {
      editor.setModel(null);
      editor.dispose();
      viewStates.clear();
    },
  };
}

export function setEditorTheme(theme: EditorTheme): void {
  monaco.editor.setTheme(theme);
}
