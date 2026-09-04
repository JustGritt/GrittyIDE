import type { DocumentId } from '@gritty/protocol';
import * as monaco from 'monaco-editor';
import {
  DEFAULT_EDITOR_PREFERENCES,
  type EditorOptions,
  editorOptionsFor,
} from '../editor-options';
import type { EditorViewState } from '../view-state';

export type EditorTheme = 'vs' | 'vs-dark';

export interface CursorPosition {
  readonly lineNumber: number;
  readonly column: number;
}

/** What the preview needs to follow the editor: the first visible line, and whether the end is in view. */
export interface ScrollState {
  readonly topLine: number;
  readonly atBottom: boolean;
}

/** The primary selection: where it started and where the cursor is. */
export interface SelectionState {
  readonly anchor: CursorPosition;
  readonly cursor: CursorPosition;
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
  /** Every selection change (a plain cursor move included): anchor and head as plain numbers. */
  onSelectionChange(callback: (selection: SelectionState) => void): () => void;
  /** Vertical scrolls only; silent while the editor is hidden (no visible range). */
  onScrollChange(callback: (state: ScrollState) => void): () => void;
  /** Move the cursor (clamped by Monaco), scroll it into view, focus. */
  revealPosition(position: CursorPosition): void;
  /**
   * Put a persisted view state back (T060): selection, then the first
   * visible line. Clamped by Monaco; never focuses, never persists Monaco's
   * own view-state object.
   */
  restoreView(state: EditorViewState): void;
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
    onSelectionChange(callback) {
      const subscription = editor.onDidChangeCursorSelection((event) => {
        const selection = event.selection;
        callback({
          anchor: {
            lineNumber: selection.selectionStartLineNumber,
            column: selection.selectionStartColumn,
          },
          cursor: { lineNumber: selection.positionLineNumber, column: selection.positionColumn },
        });
      });
      return () => subscription.dispose();
    },
    onScrollChange(callback) {
      const subscription = editor.onDidScrollChange((event) => {
        if (!event.scrollTopChanged) return;
        const first = editor.getVisibleRanges()[0];
        if (!first) return;
        const atBottom =
          editor.getScrollTop() + editor.getLayoutInfo().height >= editor.getScrollHeight() - 1;
        callback({ topLine: first.startLineNumber, atBottom });
      });
      return () => subscription.dispose();
    },
    revealPosition(position) {
      const target = { lineNumber: position.lineNumber, column: position.column };
      editor.setPosition(target);
      editor.revealPositionInCenter(target);
      editor.focus();
    },
    restoreView(state) {
      if (!editor.getModel()) return;
      editor.setSelection({
        selectionStartLineNumber: state.anchor.line,
        selectionStartColumn: state.anchor.column,
        positionLineNumber: state.cursor.line,
        positionColumn: state.cursor.column,
      });
      editor.setScrollTop(editor.getTopForLineNumber(Math.max(1, state.scrollLine)));
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
