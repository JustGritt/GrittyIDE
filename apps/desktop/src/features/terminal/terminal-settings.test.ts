import type { JsonValue, SettingsSnapshot } from '@gritty/protocol';
import { describe, expect, it } from 'vitest';
import { DEFAULT_TERMINAL_LAYOUT, MAX_CUSTOM_SHELLS } from '../session/persisted-layout';
import { terminalSettingsFromSettings } from './terminal-settings';
import { DEFAULT_TERMINAL_FONT_SIZE } from './terminal-view';

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
  'terminal.shell': null,
  'terminal.customShells': [],
  'terminal.fontFamily': null,
  'terminal.fontSize': 13,
};

describe('terminalSettingsFromSettings', () => {
  it('is the dock defaults before any snapshot lands', () => {
    expect(terminalSettingsFromSettings(null)).toEqual(DEFAULT_TERMINAL_LAYOUT);
  });

  it('changes nothing on screen when the file sets nothing', () => {
    // The layout default leaves the size to the view, and the view's size must
    // be the registry's, or the first snapshot would resize every terminal.
    expect(REGISTRY_DEFAULTS['terminal.fontSize']).toBe(DEFAULT_TERMINAL_FONT_SIZE);
    expect(terminalSettingsFromSettings(snapshot(REGISTRY_DEFAULTS))).toEqual({
      ...DEFAULT_TERMINAL_LAYOUT,
      fontSize: DEFAULT_TERMINAL_FONT_SIZE,
    });
  });

  it('carries every terminal setting, the shell as the profile path', () => {
    expect(
      terminalSettingsFromSettings(
        snapshot({
          'terminal.shell': '/opt/homebrew/bin/fish',
          'terminal.customShells': ['/opt/homebrew/bin/fish', '/usr/local/bin/nu'],
          'terminal.fontFamily': 'JetBrainsMono Nerd Font',
          'terminal.fontSize': 16,
        }),
      ),
    ).toEqual({
      profilePath: '/opt/homebrew/bin/fish',
      customShells: ['/opt/homebrew/bin/fish', '/usr/local/bin/nu'],
      fontFamily: 'JetBrainsMono Nerd Font',
      fontSize: 16,
    });
  });

  it('falls back per field on a value the dock cannot use', () => {
    expect(
      terminalSettingsFromSettings(
        snapshot({
          'terminal.shell': '   ',
          'terminal.customShells': ['relative/fish', 7, '/bin/zsh', '/bin/zsh'],
          'terminal.fontFamily': '',
          'terminal.fontSize': 900,
        }),
      ),
    ).toEqual({ profilePath: null, customShells: ['/bin/zsh'], fontFamily: null, fontSize: null });
  });

  it('bounds the custom shells the way the layout did', () => {
    const many = Array.from({ length: MAX_CUSTOM_SHELLS + 4 }, (_, index) => `/bin/s${index}`);
    expect(
      terminalSettingsFromSettings(
        snapshot({ ...REGISTRY_DEFAULTS, 'terminal.customShells': many }),
      ).customShells,
    ).toHaveLength(MAX_CUSTOM_SHELLS);
  });

  it('ignores settings outside the terminal section', () => {
    expect(
      terminalSettingsFromSettings(
        snapshot({ ...REGISTRY_DEFAULTS, 'editor.fontSize': 20, 'workbench.theme': 'dark' }),
      ),
    ).toEqual(terminalSettingsFromSettings(snapshot(REGISTRY_DEFAULTS)));
  });
});
