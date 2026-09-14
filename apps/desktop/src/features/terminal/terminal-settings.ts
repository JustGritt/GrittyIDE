import type { SettingsSnapshot } from '@gritty/protocol';
import {
  DEFAULT_TERMINAL_LAYOUT,
  parseLayout,
  type TerminalLayout,
} from '../session/persisted-layout';

/**
 * What the terminal dock is configured from: the `terminal.*` settings in force
 * (T120-A, ADR-031).
 *
 * `settings.json` is the source of truth for these four, and the settings page
 * is the one place three of them are edited; the fourth, `terminal.shell`, is
 * also written by the dock's shell picker, which is a shortcut to the same key
 * rather than a second copy of it. The layout blob's `terminal` section is no
 * longer read for them; it is restored and written back unchanged, because it
 * is what the one-time seed (`settings-seed.ts`) moves across on the first
 * launch that has the file.
 *
 * The result keeps the `TerminalLayout` shape the dock already consumes, and is
 * read through `parseLayout`, which already knows the terminal's bounds — the
 * same belt and braces `editor-settings.ts` gets from `parseEditorPreferences`.
 * The core validates each value before it reaches a snapshot; a value the dock
 * still cannot use falls back to its default rather than reaching a spawn.
 *
 * `terminal.shell` becomes `profilePath`: the setting was named for what it
 * means, the field for how T095 found it.
 *
 * The dock's defaults until the first snapshot lands. The registry's font size
 * default is the terminal's (13), so the first snapshot does not resize a
 * terminal nobody configured.
 */
export function terminalSettingsFromSettings(snapshot: SettingsSnapshot | null): TerminalLayout {
  if (snapshot === null) return DEFAULT_TERMINAL_LAYOUT;
  const { values } = snapshot;
  return parseLayout({
    terminal: {
      profilePath: values['terminal.shell'],
      customShells: values['terminal.customShells'],
      fontFamily: values['terminal.fontFamily'],
      fontSize: values['terminal.fontSize'],
    },
  }).terminal;
}
