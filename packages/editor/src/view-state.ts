/**
 * The part of an editor's view state that is persisted (T060): plain numbers,
 * never Monaco's `ICodeEditorViewState`. Lines and columns are one-based, as
 * the editor and the core's `TextPosition` report them.
 */
export interface TextPos {
  readonly line: number;
  readonly column: number;
}

export interface EditorViewState {
  readonly cursor: TextPos;
  /** Selection anchor; equals `cursor` when nothing is selected. */
  readonly anchor: TextPos;
  /** First visible line. */
  readonly scrollLine: number;
}

export const INITIAL_VIEW_STATE: EditorViewState = {
  cursor: { line: 1, column: 1 },
  anchor: { line: 1, column: 1 },
  scrollLine: 1,
};

function isPos(value: unknown): value is TextPos {
  if (typeof value !== 'object' || value === null) return false;
  const pos = value as Record<string, unknown>;
  return (
    typeof pos.line === 'number' &&
    Number.isInteger(pos.line) &&
    pos.line >= 1 &&
    typeof pos.column === 'number' &&
    Number.isInteger(pos.column) &&
    pos.column >= 1
  );
}

/**
 * A view state from persisted parts; `null` when nothing usable was stored.
 * A missing anchor collapses to the cursor, a missing scroll line to the
 * cursor's line.
 */
export function viewStateFrom(parts: {
  cursor?: unknown;
  anchor?: unknown;
  scrollLine?: unknown;
}): EditorViewState | null {
  const cursor = isPos(parts.cursor) ? parts.cursor : null;
  const anchor = isPos(parts.anchor) ? parts.anchor : cursor;
  const scrollLine =
    typeof parts.scrollLine === 'number' &&
    Number.isInteger(parts.scrollLine) &&
    parts.scrollLine >= 1
      ? parts.scrollLine
      : cursor?.line;
  if (cursor === null || anchor === null || scrollLine === undefined) return null;
  return { cursor, anchor, scrollLine };
}
