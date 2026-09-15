import type { Accelerator, KeyChord } from '../shell/open-shortcuts';

/**
 * The find-in-files chord (SEARCH-CHORD): ⌘⇧F on macOS, Ctrl+Shift+F elsewhere.
 *
 * Pure, so the rules are tested without a window — the split
 * `settings-chord.ts` and `quick-open-chord.ts` use, with the same
 * `Accelerator`. The listening is `use-search-chord.ts`'s.
 *
 * The ninth hand-gated chord in the shell. The missing shortcut registry is
 * recorded debt with its own ticket, and this file is not where it gets paid.
 */

/** A chord read from `code` as well as `key`: on macOS a modifier composes the character. */
export interface SearchKeyChord extends KeyChord {
  readonly code: string;
}

/**
 * `KeyF` with the platform's own accelerator and Shift, and nothing else.
 *
 * Shift is what separates this from ⌘F, which is Monaco's find and must stay
 * so. Alt and the other platform's modifier are rejected rather than ignored:
 * ⌘⌥F is Monaco's replace, and a superset quietly doing the same thing is how
 * a listener claims a keystroke somebody else meant.
 */
export function isSearchChord(chord: SearchKeyChord, accelerator: Accelerator): boolean {
  const wanted = accelerator === 'meta' ? chord.metaKey : chord.ctrlKey;
  const other = accelerator === 'meta' ? chord.ctrlKey : chord.metaKey;
  if (!wanted || other || chord.altKey || !chord.shiftKey) return false;
  return chord.code === 'KeyF';
}
