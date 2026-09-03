/**
 * The slice of Monaco's `ITextModel` the binding relies on. Keeping it
 * structural lets the registry and binding run in node against a fake model
 * (`src/testing/fake-model.ts`); the real model satisfies it as is.
 */
export interface ModelPosition {
  readonly lineNumber: number;
  readonly column: number;
}

export interface ModelRange {
  readonly startLineNumber: number;
  readonly startColumn: number;
  readonly endLineNumber: number;
  readonly endColumn: number;
}

export interface ModelEdit {
  readonly range: ModelRange;
  readonly text: string;
}

/** Monaco's `EndOfLineSequence`: LF = 0, CRLF = 1. */
export type EolSequence = 0 | 1;

export interface TextModelLike {
  getValue(): string;
  /** `"\n"` or `"\r\n"`. */
  getEOL(): string;
  setEOL(eol: EolSequence): void;
  getPositionAt(offset: number): ModelPosition;
  /** Undoable edit; inserted text is normalized to the model's line ending. */
  pushEditOperations(
    beforeCursorState: null,
    editOperations: ModelEdit[],
    cursorStateComputer: () => null,
  ): unknown;
  onDidChangeContent(listener: () => void): { dispose(): void };
  updateOptions(options: { tabSize?: number; insertSpaces?: boolean }): void;
  dispose(): void;
}
