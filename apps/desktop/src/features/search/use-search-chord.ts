import { useEffect, useRef } from 'react';
import type { Accelerator } from '../shell/open-shortcuts';
import { isSearchChord } from './search-chord';

/**
 * The window's listener for ⌘⇧F / Ctrl+Shift+F (SEARCH-CHORD).
 *
 * **It does not yield to `ownsRawKeystrokes`**, for the reason
 * `useSettingsChord` gives for ⌘,: the case that matters is a cursor in a file,
 * where Monaco's `<textarea>` has focus, and yielding would lose exactly that.
 * Quick open yields because Ctrl+P is readline's "previous command"; readline
 * has no Ctrl+Shift+F, and a legacy terminal cannot even encode Shift with Ctrl
 * and a letter, so a live shell is not denied anything it could receive.
 *
 * No Monaco registration either, unlike ⌘P. Monaco 0.56 binds ⌘F, ⌘⌥F, ⇧⌥F
 * and ⌘K ⌘F, and nothing on ⌘⇧F, so the keydown bubbles out of the buffer to
 * this listener unhandled.
 *
 * Inert while `accelerator` is `null` (the core has not reported its
 * platform) or `enabled` is false (no workspace, so no panel to open).
 */
export function useSearchChord(
  accelerator: Accelerator | null,
  enabled: boolean,
  onOpen: () => void,
): void {
  // Through a ref, so a new callback identity per render does not tear the
  // listener down and put it back on every keystroke elsewhere.
  const latest = useRef(onOpen);
  latest.current = onOpen;

  useEffect(() => {
    if (accelerator === null || !enabled) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (!isSearchChord(event, accelerator)) return;
      event.preventDefault();
      latest.current();
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [accelerator, enabled]);
}
