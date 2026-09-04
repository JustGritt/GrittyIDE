/**
 * The shell's open shortcuts (T049): ⌘O / Ctrl+O and ⌘⇧O / Ctrl+⇧O.
 *
 * Pure, so the rules can be tested without a window. The hook beside this file
 * does the listening.
 */

export type OpenShortcut = 'folder' | 'path';

/** Which modifier is "the command key" on this platform. */
export type Accelerator = 'meta' | 'ctrl';

/** The parts of a `KeyboardEvent` a chord is made of. */
export interface KeyChord {
  readonly key: string;
  readonly metaKey: boolean;
  readonly ctrlKey: boolean;
  readonly shiftKey: boolean;
  readonly altKey: boolean;
}

/**
 * Derived from the core's `HealthReport.os` (`std::env::consts::OS`) rather
 * than from `navigator`: the Rust side already knows what it is running on,
 * and `navigator.platform` is deprecated and lies under emulation.
 */
export function acceleratorForOs(os: string): Accelerator {
  return os === 'macos' ? 'meta' : 'ctrl';
}

/**
 * Only the platform's own accelerator counts. Accepting both would mean
 * swallowing Ctrl+O on macOS, where it belongs to whatever has focus — a
 * shell in the terminal dock reads it as a control character.
 */
export function matchOpenShortcut(chord: KeyChord, accelerator: Accelerator): OpenShortcut | null {
  const wanted = accelerator === 'meta' ? chord.metaKey : chord.ctrlKey;
  const other = accelerator === 'meta' ? chord.ctrlKey : chord.metaKey;
  if (!wanted || other || chord.altKey) return null;
  if (chord.key.toLowerCase() !== 'o') return null;
  return chord.shiftKey ? 'path' : 'folder';
}

/**
 * Whether the focused element takes keystrokes verbatim and must keep them.
 *
 * Monaco and xterm both put a `<textarea>` under the cursor, so one rule
 * covers the editor and every live shell. The same idea as the ⌘S handler in
 * `editor-pane.tsx`, phrased so it can be tested without a DOM.
 */
export function ownsRawKeystrokes(
  target: { tagName?: string; isContentEditable?: boolean } | null | undefined,
): boolean {
  if (!target) return false;
  if (target.isContentEditable === true) return true;
  return target.tagName?.toUpperCase() === 'TEXTAREA';
}
