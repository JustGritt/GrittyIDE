/**
 * The quick-open chord (T130-M): ⌘P / Ctrl+P, and ⌘⇧P / Ctrl+⇧P.
 *
 * Pure, so the rules are tested without a window — the same split
 * `open-shortcuts.ts` uses, and the same `Accelerator` and `ownsRawKeystrokes`.
 * The listening is `use-quick-open-chord.ts`'s, and the half that must fire
 * *inside* Monaco is registered with Monaco itself.
 */

import type { Accelerator, KeyChord } from './open-shortcuts';

/** Which mode the chord opens into. */
export type QuickOpenShortcut = 'files' | 'commands';

/**
 * A chord read from `code` as well as `key`.
 *
 * `code` is the physical key and `key` is what it produced. On macOS Option
 * composes — ⌥P is `π` — so a rule written against `key` matches the composed
 * character and not the key the user pressed. `explorer-keymap.ts`,
 * `terminal-chords.ts`, `permission-chords.ts` and `quick-open-keymap.ts` all
 * record this; this file is the fifth.
 */
export interface QuickOpenKeyChord extends KeyChord {
  readonly code: string;
}

/**
 * `KeyP` with the platform's own accelerator, and nothing else.
 *
 * Only the platform's accelerator counts, for the reason `matchOpenShortcut`
 * gives: accepting both would swallow Ctrl+P on macOS, where it belongs to
 * whatever has focus. The inverse matters more here than it did for ⌘O — on
 * Linux and Windows the accelerator *is* Ctrl, and `Ctrl+P` is readline's
 * "previous command", so a live shell must keep it. That is `ownsRawKeystrokes`'
 * job at the listener, not this function's.
 */
export function matchQuickOpenShortcut(
  chord: QuickOpenKeyChord,
  accelerator: Accelerator,
): QuickOpenShortcut | null {
  const wanted = accelerator === 'meta' ? chord.metaKey : chord.ctrlKey;
  const other = accelerator === 'meta' ? chord.ctrlKey : chord.metaKey;
  // A superset is not the chord: ⌘⌃P and ⌘⌥P belong to whatever else claims
  // them, and swallowing them here would be the tenth hand-gated listener
  // quietly becoming the eleventh.
  if (!wanted || other || chord.altKey) return null;
  if (chord.code !== 'KeyP') return null;
  return chord.shiftKey ? 'commands' : 'files';
}

/** What the field is pre-filled with, so the mode is the one the chord named. */
export function initialQueryFor(shortcut: QuickOpenShortcut): string {
  return shortcut === 'commands' ? '>' : '';
}
