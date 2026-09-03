import { dominantEol, EOL_SEQUENCE, normalizeEol } from '../eol';
import type { EolSequence, ModelEdit, ModelPosition, TextModelLike } from '../text-model';

/**
 * A `TextModelLike` that mirrors the Monaco behaviors the binding depends on:
 * line endings are normalized to the dominant one at creation, inserted text
 * is normalized to the model's line ending, edits fire `onDidChangeContent`
 * synchronously.
 */
export class FakeTextModel implements TextModelLike {
  value: string;
  eol: string;
  disposed = false;
  options: { tabSize?: number; insertSpaces?: boolean } = {};
  readonly edits: ModelEdit[] = [];
  getValueCalls = 0;
  readonly #listeners = new Set<() => void>();

  constructor(text: string) {
    const target = dominantEol(text);
    this.eol = EOL_SEQUENCE[target];
    this.value = normalizeEol(text, target);
  }

  getValue(): string {
    this.getValueCalls += 1;
    return this.value;
  }

  getEOL(): string {
    return this.eol;
  }

  setEOL(eol: EolSequence): void {
    const target = eol === 0 ? 'lf' : 'crlf';
    this.eol = EOL_SEQUENCE[target];
    this.value = normalizeEol(this.value, target);
    this.#fire();
  }

  getPositionAt(offset: number): ModelPosition {
    const clamped = Math.max(0, Math.min(offset, this.value.length));
    let lineNumber = 1;
    let lineStart = 0;
    let index = this.value.indexOf(this.eol);
    while (index !== -1 && index + this.eol.length <= clamped) {
      lineNumber += 1;
      lineStart = index + this.eol.length;
      index = this.value.indexOf(this.eol, lineStart);
    }
    return { lineNumber, column: clamped - lineStart + 1 };
  }

  offsetAt(position: ModelPosition): number {
    let offset = 0;
    for (let line = 1; line < position.lineNumber; line += 1) {
      const next = this.value.indexOf(this.eol, offset);
      if (next === -1) break;
      offset = next + this.eol.length;
    }
    return offset + position.column - 1;
  }

  pushEditOperations(_before: null, edits: ModelEdit[], _computer: () => null): null {
    const target = this.eol === '\r\n' ? 'crlf' : 'lf';
    const resolved = edits
      .map((edit) => ({
        start: this.offsetAt({
          lineNumber: edit.range.startLineNumber,
          column: edit.range.startColumn,
        }),
        end: this.offsetAt({ lineNumber: edit.range.endLineNumber, column: edit.range.endColumn }),
        text: normalizeEol(edit.text, target),
      }))
      .sort((a, b) => b.start - a.start);
    for (const edit of resolved) {
      this.value = this.value.slice(0, edit.start) + edit.text + this.value.slice(edit.end);
    }
    this.edits.push(...edits);
    this.#fire();
    return null;
  }

  onDidChangeContent(listener: () => void): { dispose(): void } {
    this.#listeners.add(listener);
    return {
      dispose: () => {
        this.#listeners.delete(listener);
      },
    };
  }

  updateOptions(options: { tabSize?: number; insertSpaces?: boolean }): void {
    this.options = { ...this.options, ...options };
  }

  dispose(): void {
    this.disposed = true;
    this.#listeners.clear();
  }

  /** Simulate the user typing: the whole text becomes `value`. */
  type(value: string): void {
    this.value = value;
    this.#fire();
  }

  #fire(): void {
    for (const listener of [...this.#listeners]) listener();
  }
}
