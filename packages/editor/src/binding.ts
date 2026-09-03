import { detectEol, EOL_SEQUENCE, type Eol, isEditableEol, normalizeEol } from './eol';
import { computeMinimalEdit } from './minimal-edit';
import type { TextModelLike } from './text-model';

/**
 * One document's model as the UI drives it. Monaco is never the source of
 * truth: every content change is reported as a *touch* (no text is read per
 * keystroke), the text is read lazily when the debounced update is sent, and
 * text coming back from the core is applied as one minimal edit that is not
 * echoed as a touch.
 */
export interface ModelBinding {
  /** The full model text. Call it once per flush, not per keystroke. */
  read(): string;
  /**
   * Replace the model text with what the core holds. Returns the line-ending
   * kind of `text`; `mixed`/`cr` cannot be represented, so the model then
   * shows the text normalized to its own line ending and must stay read-only.
   */
  applyCoreText(text: string): Eol;
  dispose(): void;
}

export function bindModel(model: TextModelLike, onTouch: () => void): ModelBinding {
  let applying = false;
  const subscription = model.onDidChangeContent(() => {
    if (!applying) onTouch();
  });

  return {
    read: () => model.getValue(),
    applyCoreText(text) {
      const eol = detectEol(text);
      applying = true;
      try {
        if ((eol === 'lf' || eol === 'crlf') && model.getEOL() !== EOL_SEQUENCE[eol]) {
          model.setEOL(eol === 'lf' ? 0 : 1);
        }
        const target = isEditableEol(eol)
          ? text
          : normalizeEol(text, model.getEOL() === '\r\n' ? 'crlf' : 'lf');
        const edit = computeMinimalEdit(model.getValue(), target);
        if (edit !== null) {
          const start = model.getPositionAt(edit.start);
          const end = model.getPositionAt(edit.end);
          model.pushEditOperations(
            null,
            [
              {
                range: {
                  startLineNumber: start.lineNumber,
                  startColumn: start.column,
                  endLineNumber: end.lineNumber,
                  endColumn: end.column,
                },
                text: edit.text,
              },
            ],
            () => null,
          );
        }
      } finally {
        applying = false;
      }
      return eol;
    },
    dispose() {
      subscription.dispose();
    },
  };
}
