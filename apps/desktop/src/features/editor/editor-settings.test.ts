import { DEFAULT_EDITOR_PREFERENCES } from '@gritty/editor';
import type { JsonValue, SettingsSnapshot } from '@gritty/protocol';
import { describe, expect, it } from 'vitest';
import { editorPreferencesFromSettings } from './editor-settings';

function snapshot(values: Record<string, JsonValue>): SettingsSnapshot {
  return {
    values,
    explicit: [],
    modified: [],
    invalid: [],
    unknown: [],
    diagnostics: [],
    storage: { writable: true, path: '/home/a/settings.json' },
  };
}

const REGISTRY_DEFAULTS = {
  'editor.fontSize': 13,
  'editor.tabSize': '4',
  'editor.wordWrap': false,
  'editor.minimap': true,
};

describe('editorPreferencesFromSettings', () => {
  it('is the editor defaults before any snapshot lands', () => {
    expect(editorPreferencesFromSettings(null)).toEqual(DEFAULT_EDITOR_PREFERENCES);
  });

  it('changes nothing on screen when the file sets nothing', () => {
    // The registry's defaults and the editor's must agree, or the first
    // snapshot would visibly restyle an editor nobody configured.
    expect(editorPreferencesFromSettings(snapshot(REGISTRY_DEFAULTS))).toEqual(
      DEFAULT_EDITOR_PREFERENCES,
    );
  });

  it('carries every editor setting, translating the tab size choice to a number', () => {
    expect(
      editorPreferencesFromSettings(
        snapshot({
          'editor.fontSize': 18,
          'editor.tabSize': '2',
          'editor.wordWrap': true,
          'editor.minimap': false,
        }),
      ),
    ).toEqual({ fontSize: 18, tabSize: 2, wordWrap: true, minimap: false });
  });

  it('falls back per field on a value the editor cannot use', () => {
    expect(
      editorPreferencesFromSettings(
        snapshot({ ...REGISTRY_DEFAULTS, 'editor.fontSize': 900, 'editor.tabSize': '3' }),
      ),
    ).toEqual(DEFAULT_EDITOR_PREFERENCES);
  });

  it('ignores settings outside the editor section', () => {
    expect(
      editorPreferencesFromSettings(
        snapshot({ ...REGISTRY_DEFAULTS, 'terminal.fontSize': 20, 'workbench.theme': 'dark' }),
      ),
    ).toEqual(DEFAULT_EDITOR_PREFERENCES);
  });
});
