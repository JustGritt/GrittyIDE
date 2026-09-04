import type { editor } from 'monaco-editor';

/** Editor preferences; persisted by the session (T060) as a versioned JSON blob. */
export interface EditorPreferences {
  readonly fontSize: number;
  readonly tabSize: number;
  readonly wordWrap: boolean;
  readonly minimap: boolean;
}

export const DEFAULT_EDITOR_PREFERENCES: EditorPreferences = {
  fontSize: 13,
  tabSize: 4,
  wordWrap: false,
  minimap: true,
};

export const MIN_FONT_SIZE = 8;
export const MAX_FONT_SIZE = 32;
export const TAB_SIZES: readonly number[] = [2, 4, 8];
/** Producer version of the persisted blob; bump when a field changes meaning. */
export const EDITOR_PREFERENCES_VERSION = 1;

/**
 * Rebuild preferences from whatever was persisted. Every field falls back to
 * its default when missing or out of range, so an older, newer or damaged
 * blob never breaks the editor.
 */
export function parseEditorPreferences(value: unknown): EditorPreferences {
  if (typeof value !== 'object' || value === null) return DEFAULT_EDITOR_PREFERENCES;
  const raw = value as Record<string, unknown>;
  const fontSize =
    typeof raw.fontSize === 'number' &&
    Number.isInteger(raw.fontSize) &&
    raw.fontSize >= MIN_FONT_SIZE &&
    raw.fontSize <= MAX_FONT_SIZE
      ? raw.fontSize
      : DEFAULT_EDITOR_PREFERENCES.fontSize;
  const tabSize =
    typeof raw.tabSize === 'number' && TAB_SIZES.includes(raw.tabSize)
      ? raw.tabSize
      : DEFAULT_EDITOR_PREFERENCES.tabSize;
  const wordWrap =
    typeof raw.wordWrap === 'boolean' ? raw.wordWrap : DEFAULT_EDITOR_PREFERENCES.wordWrap;
  const minimap =
    typeof raw.minimap === 'boolean' ? raw.minimap : DEFAULT_EDITOR_PREFERENCES.minimap;
  return { fontSize, tabSize, wordWrap, minimap };
}

/**
 * The blob `parseEditorPreferences` reads back. A type alias, not an
 * interface: only aliases get the implicit index signature that makes them
 * assignable to the protocol's `JsonValue`.
 */
export type SerializedEditorPreferences = {
  readonly version: number;
  readonly fontSize: number;
  readonly tabSize: number;
  readonly wordWrap: boolean;
  readonly minimap: boolean;
};

export function serializeEditorPreferences(prefs: EditorPreferences): SerializedEditorPreferences {
  return { version: EDITOR_PREFERENCES_VERSION, ...prefs };
}

/** What the document contract says about the active document. */
export interface DocumentFlags {
  /** `Document.readOnly`: the core refuses `documents.update`. */
  readonly readOnly: boolean;
  /** `Document.largeFileMode`: degrade every expensive feature explicitly. */
  readonly largeFileMode: boolean;
  /** `false` for mixed / lone-CR line endings, which a model cannot hold verbatim. */
  readonly editableEol: boolean;
}

/** Monaco's default is 20 000; the 12 MiB fixture needs nothing more than plaintext anyway. */
export const MAX_TOKENIZATION_LINE_LENGTH = 20_000;
export const LARGE_FILE_MAX_TOKENIZATION_LINE_LENGTH = 1_000;

/** Why the editor is read-only, as shown when the user types; `null` when editable. */
export function readOnlyReason(flags: DocumentFlags): string | null {
  if (!flags.editableEol) {
    return 'Mixed line endings: normalize them (see the notice above) before editing.';
  }
  if (flags.readOnly && flags.largeFileMode) {
    return 'Large file opened read-only. Use "Enable editing" above to edit it.';
  }
  if (flags.readOnly) return 'This file is read-only (no write permission).';
  return null;
}

/**
 * Editor options for a document. Monaco's own large-file heuristics (20 MB or
 * 300k lines) do not trigger for files just above the core's 10 MiB
 * threshold, so large-file mode lists every degradation explicitly.
 */
export type EditorOptions = editor.IEditorOptions & editor.IGlobalEditorOptions;

export function editorOptionsFor(prefs: EditorPreferences, flags: DocumentFlags): EditorOptions {
  const reason = readOnlyReason(flags);
  const base: EditorOptions = {
    fontSize: prefs.fontSize,
    wordWrap: prefs.wordWrap ? 'on' : 'off',
    minimap: { enabled: prefs.minimap },
    readOnly: reason !== null,
    readOnlyMessage: reason === null ? undefined : { value: reason },
    folding: true,
    bracketPairColorization: { enabled: true },
    matchBrackets: 'always',
    occurrencesHighlight: 'singleFile',
    selectionHighlight: true,
    renderValidationDecorations: 'editable',
    links: true,
    codeLens: false,
    stickyScroll: { enabled: true },
    quickSuggestions: true,
    wordBasedSuggestions: 'currentDocument',
    largeFileOptimizations: true,
    maxTokenizationLineLength: MAX_TOKENIZATION_LINE_LENGTH,
    scrollBeyondLastLine: false,
    automaticLayout: true,
  };
  if (!flags.largeFileMode) return base;
  return {
    ...base,
    minimap: { enabled: false },
    folding: false,
    wordWrap: 'off',
    bracketPairColorization: { enabled: false },
    matchBrackets: 'never',
    occurrencesHighlight: 'off',
    selectionHighlight: false,
    renderValidationDecorations: 'off',
    links: false,
    stickyScroll: { enabled: false },
    quickSuggestions: false,
    wordBasedSuggestions: 'off',
    maxTokenizationLineLength: LARGE_FILE_MAX_TOKENIZATION_LINE_LENGTH,
  };
}
