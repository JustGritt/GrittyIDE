import { describe, expect, it } from 'vitest';
import {
  DEFAULT_EDITOR_PREFERENCES,
  editorOptionsFor,
  LARGE_FILE_MAX_TOKENIZATION_LINE_LENGTH,
  MAX_TOKENIZATION_LINE_LENGTH,
  parseEditorPreferences,
  readOnlyReason,
  serializeEditorPreferences,
} from './editor-options';

const editable = { readOnly: false, largeFileMode: false, editableEol: true };

describe('editorOptionsFor', () => {
  it('maps preferences for a normal editable document', () => {
    const options = editorOptionsFor(
      { ...DEFAULT_EDITOR_PREFERENCES, fontSize: 15, wordWrap: true, minimap: false },
      editable,
    );
    expect(options).toMatchObject({
      fontSize: 15,
      wordWrap: 'on',
      minimap: { enabled: false },
      readOnly: false,
      folding: true,
      matchBrackets: 'always',
      largeFileOptimizations: true,
      maxTokenizationLineLength: MAX_TOKENIZATION_LINE_LENGTH,
      automaticLayout: true,
    });
    expect(options.readOnlyMessage).toBeUndefined();
  });

  it('degrades every expensive feature in large-file mode, whatever the preferences say', () => {
    const options = editorOptionsFor(
      { ...DEFAULT_EDITOR_PREFERENCES, wordWrap: true, minimap: true },
      { readOnly: true, largeFileMode: true, editableEol: true },
    );
    expect(options).toMatchObject({
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
      largeFileOptimizations: true,
      maxTokenizationLineLength: LARGE_FILE_MAX_TOKENIZATION_LINE_LENGTH,
      readOnly: true,
    });
    expect(options.readOnlyMessage?.value).toContain('Enable editing');
  });

  it('explains why the editor is read-only', () => {
    expect(readOnlyReason(editable)).toBeNull();
    expect(readOnlyReason({ ...editable, readOnly: true })).toMatch(/no write permission/);
    expect(readOnlyReason({ ...editable, readOnly: true, largeFileMode: true })).toMatch(
      /Enable editing/,
    );
    expect(readOnlyReason({ ...editable, editableEol: false })).toMatch(/Mixed line endings/);
    expect(
      editorOptionsFor(DEFAULT_EDITOR_PREFERENCES, { ...editable, editableEol: false }),
    ).toMatchObject({ readOnly: true });
  });
});

describe('parseEditorPreferences', () => {
  it('round-trips a serialized blob', () => {
    const prefs = { fontSize: 15, tabSize: 2, wordWrap: true, minimap: false };
    const blob = serializeEditorPreferences(prefs);
    expect(blob.version).toBe(1);
    expect(parseEditorPreferences(blob)).toEqual(prefs);
  });

  it('falls back per field on missing, wrong-typed or out-of-range values', () => {
    expect(parseEditorPreferences(null)).toEqual(DEFAULT_EDITOR_PREFERENCES);
    expect(parseEditorPreferences('nope')).toEqual(DEFAULT_EDITOR_PREFERENCES);
    expect(parseEditorPreferences({ version: 99 })).toEqual(DEFAULT_EDITOR_PREFERENCES);
    expect(
      parseEditorPreferences({ fontSize: 200, tabSize: 3, wordWrap: 'yes', minimap: false }),
    ).toEqual({ ...DEFAULT_EDITOR_PREFERENCES, minimap: false });
    expect(parseEditorPreferences({ fontSize: 12.5 }).fontSize).toBe(
      DEFAULT_EDITOR_PREFERENCES.fontSize,
    );
    expect(parseEditorPreferences({ fontSize: 8, tabSize: 8 })).toMatchObject({
      fontSize: 8,
      tabSize: 8,
    });
  });
});
