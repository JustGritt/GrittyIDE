import { useEffect, useRef } from 'react';
import { type Accelerator, matchOpenShortcut, ownsRawKeystrokes } from './open-shortcuts';

export interface OpenShortcutHandlers {
  /** ⌘O / Ctrl+O — the native directory picker. Always available. */
  onOpenFolder(): void;
  /**
   * ⌘⇧O / Ctrl+⇧O — the path field. Only the launcher has one, so this is
   * `undefined` while a workspace is open and the chord is then left alone
   * rather than swallowed.
   */
  onOpenPath?: () => void;
}

/**
 * The window's open shortcuts (T049).
 *
 * One listener, registered by the shell, following the precedent the ⌘S
 * handler set in `editor-pane.tsx`: bail out when the focused element takes
 * keystrokes verbatim, so the editor and every live shell keep theirs.
 *
 * `accelerator` is `null` until the core has reported its platform; the
 * shortcuts are inert until then rather than guessing a modifier and stealing
 * the wrong one.
 */
export function useOpenShortcuts(
  accelerator: Accelerator | null,
  handlers: OpenShortcutHandlers,
): void {
  // Through a ref, so a new callback identity per render does not tear the
  // listener down and put it back on every keystroke elsewhere.
  const latest = useRef(handlers);
  latest.current = handlers;

  useEffect(() => {
    if (accelerator === null) return;
    const onKeyDown = (event: KeyboardEvent) => {
      const shortcut = matchOpenShortcut(event, accelerator);
      if (shortcut === null) return;
      if (ownsRawKeystrokes(event.target as HTMLElement | null)) return;
      const { onOpenFolder, onOpenPath } = latest.current;
      if (shortcut === 'path' && onOpenPath === undefined) return;
      event.preventDefault();
      if (shortcut === 'path') onOpenPath?.();
      else onOpenFolder();
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [accelerator]);
}
