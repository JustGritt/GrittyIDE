import { useEffect, useRef } from 'react';
import type { Accelerator } from '../shell/open-shortcuts';
import { isSettingsChord } from './settings-chord';

/**
 * The window's half of the settings chord (T120-M §4).
 *
 * **It does not yield to `ownsRawKeystrokes`, and that is the one way it
 * differs from `useQuickOpenChord`.** That listener stands aside for anything
 * taking keystrokes verbatim, because on Linux and Windows its accelerator is
 * Ctrl and `Ctrl+P` is readline's "previous command" — swallowing it in a live
 * shell would be a regression. `Comma` is not a readline binding, no shell
 * reads `Ctrl+,` as a control character, and on macOS ⌘, is the platform's
 * universal Preferences chord and belongs to the application everywhere.
 *
 * Yielding here would cost the case that matters most — you are editing a file,
 * Monaco has a `<textarea>` under the cursor, and you press ⌘, — and would buy
 * nothing, since quick open's answer to the same problem is a second
 * registration inside Monaco, which this chord does not need.
 *
 * `accelerator` is `null` until the core has reported its platform; the chord
 * is inert until then rather than guessing which modifier is the command key,
 * exactly as `useOpenShortcuts` and `useQuickOpenChord` are.
 */
export function useSettingsChord(accelerator: Accelerator | null, onOpen: () => void): void {
  // Through a ref, so a new callback identity per render does not tear the
  // listener down and put it back on every keystroke elsewhere.
  const latest = useRef(onOpen);
  latest.current = onOpen;

  useEffect(() => {
    if (accelerator === null) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (!isSettingsChord(event, accelerator)) return;
      event.preventDefault();
      latest.current();
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [accelerator]);
}
