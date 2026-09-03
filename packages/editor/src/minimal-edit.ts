/**
 * The smallest single replacement that turns `oldText` into `newText`
 * (common prefix and suffix trimmed, never splitting a surrogate pair).
 * Applying core text to a model through one such edit instead of `setValue`
 * keeps the undo stack, cursor, folding and decorations of the untouched
 * regions (the same idea as VS Code's `ModelService.updateModel`).
 */
export interface MinimalEdit {
  /** Offset (UTF-16 code units) where the replaced span starts in `oldText`. */
  readonly start: number;
  /** Exclusive end offset of the replaced span in `oldText`. */
  readonly end: number;
  /** Replacement text. */
  readonly text: string;
}

function isHighSurrogate(code: number): boolean {
  return code >= 0xd800 && code <= 0xdbff;
}

function isLowSurrogate(code: number): boolean {
  return code >= 0xdc00 && code <= 0xdfff;
}

export function computeMinimalEdit(oldText: string, newText: string): MinimalEdit | null {
  if (oldText === newText) return null;
  const oldLength = oldText.length;
  const newLength = newText.length;
  const shorter = Math.min(oldLength, newLength);

  let prefix = 0;
  while (prefix < shorter && oldText.charCodeAt(prefix) === newText.charCodeAt(prefix)) {
    prefix += 1;
  }
  if (prefix > 0 && isHighSurrogate(oldText.charCodeAt(prefix - 1))) prefix -= 1;

  let suffix = 0;
  const maxSuffix = shorter - prefix;
  while (
    suffix < maxSuffix &&
    oldText.charCodeAt(oldLength - 1 - suffix) === newText.charCodeAt(newLength - 1 - suffix)
  ) {
    suffix += 1;
  }
  if (suffix > 0 && isLowSurrogate(oldText.charCodeAt(oldLength - suffix))) suffix -= 1;

  return {
    start: prefix,
    end: oldLength - suffix,
    text: newText.slice(prefix, newLength - suffix),
  };
}

/** Apply an edit to a string (used by tests and by the fake models). */
export function applyMinimalEdit(text: string, edit: MinimalEdit): string {
  return text.slice(0, edit.start) + edit.text + text.slice(edit.end);
}
