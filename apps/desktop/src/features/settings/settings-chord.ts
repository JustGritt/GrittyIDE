import type { Accelerator, KeyChord } from '../shell/open-shortcuts';

/**
 * The settings chord (T120-M §4): ⌘, on macOS, Ctrl+, elsewhere.
 *
 * Pure, so the rules are tested without a window — the split
 * `open-shortcuts.ts` and `quick-open-chord.ts` both use, with the same
 * `Accelerator`. The listening is `use-settings-chord.ts`'s.
 *
 * This is the **eighth** hand-gated chord in the shell and the fifth file to
 * apologise for there being no shortcut registry. It is real debt with a real
 * ticket behind it; bundling one here would double this ticket (rule 14).
 */

/**
 * A chord read from `code` as well as `key`.
 *
 * `code` is the physical key and `key` is what it produced. On macOS a modifier
 * composes, so a rule written against `key` matches the composed character
 * rather than the key the user pressed — `explorer-keymap.ts`,
 * `terminal-chords.ts`, `permission-chords.ts`, `quick-open-keymap.ts` and
 * `quick-open-chord.ts` all record this; this file is the sixth.
 */
export interface SettingsKeyChord extends KeyChord {
  readonly code: string;
}

/**
 * `Comma` with the platform's own accelerator, and nothing else.
 *
 * Only the platform's accelerator counts, for the reason `matchOpenShortcut`
 * gives: accepting both would swallow Ctrl+, on macOS, where it belongs to
 * whatever has focus.
 *
 * Shift and Alt are rejected rather than ignored. ⌘⇧, and ⌘⌥, are not this
 * chord, and a superset quietly doing the same thing is how a listener claims
 * a keystroke somebody else meant.
 */
export function isSettingsChord(chord: SettingsKeyChord, accelerator: Accelerator): boolean {
  const wanted = accelerator === 'meta' ? chord.metaKey : chord.ctrlKey;
  const other = accelerator === 'meta' ? chord.ctrlKey : chord.metaKey;
  if (!wanted || other || chord.altKey || chord.shiftKey) return false;
  return chord.code === 'Comma';
}
