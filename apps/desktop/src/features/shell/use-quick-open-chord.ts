import { useEffect, useRef } from 'react';
import type { Accelerator } from './open-shortcuts';
import { ownsRawKeystrokes } from './open-shortcuts';
import { matchQuickOpenShortcut, type QuickOpenShortcut } from './quick-open-chord';

/**
 * The window's half of the quick-open chord (T130-M §2).
 *
 * Only half, deliberately. This listener yields to anything that takes
 * keystrokes verbatim — `ownsRawKeystrokes` — which is what leaves a live shell
 * alone: on Linux and Windows the accelerator is Ctrl, and `Ctrl+P` is
 * readline's "previous command". Swallowing it in a terminal would be a
 * regression, and `matchOpenShortcut` already documents exactly this reasoning
 * for ⌘O.
 *
 * But Monaco puts a `<textarea>` under the cursor too, so this listener will
 * *not* fire while you are editing — quick open's primary case. That half is
 * registered with Monaco itself, through `EditorBinding.subscribeQuickOpen`.
 * Two registrations rather than one clever rule, because "the editor" and
 * "a live shell" want opposite answers to the same question.
 *
 * `accelerator` is `null` until the core has reported its platform; the chord
 * is inert until then rather than guessing which modifier is the command key,
 * exactly as `useOpenShortcuts` is.
 */
export function useQuickOpenChord(
  accelerator: Accelerator | null,
  onOpen: (shortcut: QuickOpenShortcut) => void,
): void {
  // Through a ref, so a new callback identity per render does not tear the
  // listener down and put it back on every keystroke elsewhere.
  const latest = useRef(onOpen);
  latest.current = onOpen;

  useEffect(() => {
    if (accelerator === null) return;
    const onKeyDown = (event: KeyboardEvent) => {
      const shortcut = matchQuickOpenShortcut(event, accelerator);
      if (shortcut === null) return;
      if (ownsRawKeystrokes(event.target as HTMLElement | null)) return;
      event.preventDefault();
      latest.current(shortcut);
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [accelerator]);
}
