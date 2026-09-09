/**
 * What a keystroke means while the quick open bar has focus (T130-U, §8).
 *
 * Matched on `event.code`, never `event.key`, for the reason
 * `explorer-keymap.ts`, `permission-chords.ts` and `terminal-chords.ts` all
 * give: on macOS a modifier composes, so `key` is whatever character came out
 * the other side, while `code` is the physical key a chord is actually about.
 * There is no type-ahead exception here — unlike the tree, quick open has a
 * real text field, and every printable character belongs to it.
 *
 * Pure and separate from the component, so the mapping is a test rather than a
 * click.
 */

/** The parts of a `KeyboardEvent` this matcher reads. */
export interface QuickOpenChord {
  readonly code: string;
  /**
   * Present so a real `KeyboardEvent` is assignable, and read by nothing here.
   * Deliberate: see the module note.
   */
  readonly key: string;
  readonly metaKey: boolean;
  readonly ctrlKey: boolean;
  readonly altKey: boolean;
  readonly shiftKey: boolean;
}

export type QuickOpenIntent =
  /** Move the focused row by `delta`, through the explorer's `moveFocus`. */
  | { readonly kind: 'move'; readonly delta: number }
  | { readonly kind: 'edge'; readonly edge: 'first' | 'last' }
  /**
   * Open what is focused. `preview` is the plain Enter: quick open is a
   * *look*, so it opens a preview tab the way the explorer's single click
   * does, and ⌘Enter is how you say you meant it.
   */
  | { readonly kind: 'accept'; readonly preview: boolean }
  | { readonly kind: 'dismiss' };

/** The command modifier, which either key may carry (the repo's convention). */
function command(event: QuickOpenChord): boolean {
  return event.metaKey || event.ctrlKey;
}

/**
 * The intent a keystroke carries, or `null` for one the bar does not claim.
 *
 * **`null` is the important half.** The field is a real `<input>` that keeps
 * focus and owns the keyboard (§10), so every unclaimed key has to reach it —
 * the same rule `explorer-keymap.ts:52` states for the tree, with more at stake
 * here, because swallowing a letter means the user cannot type. Only the eight
 * chords below are taken; everything else, including every printable
 * character, Tab, and any arrow carrying a modifier, falls through.
 *
 * Home and End are claimed, and that costs the caret shortcuts inside the
 * field. §8 asks for them by name, a list you are navigating is what those keys
 * mean while it is open, and on macOS ⌘← and ⌘→ still move the caret to the
 * ends of the line.
 */
export function quickOpenIntentFor(event: QuickOpenChord): QuickOpenIntent | null {
  const meta = command(event);
  switch (event.code) {
    case 'ArrowDown':
      // ⌘↓ and ⌥↓ belong to the field (end of line, end of text) and are left
      // to it, exactly as the tree leaves them to the window.
      return meta || event.altKey ? null : { kind: 'move', delta: 1 };
    case 'ArrowUp':
      return meta || event.altKey ? null : { kind: 'move', delta: -1 };
    case 'Home':
      return meta || event.altKey ? null : { kind: 'edge', edge: 'first' };
    case 'End':
      return meta || event.altKey ? null : { kind: 'edge', edge: 'last' };
    case 'Enter':
    case 'NumpadEnter':
      // ⇧Enter and ⌥Enter are unclaimed rather than aliased to Enter: neither
      // has a meaning here, and a chord that quietly does something else is
      // worse than one that does nothing.
      if (event.altKey || event.shiftKey) return null;
      return { kind: 'accept', preview: !meta };
    case 'Escape':
      // Shift is ignored — no one holds it on purpose — but a ⌘ or ⌥ Escape is
      // the system's or the app's, and is left alone.
      return meta || event.altKey ? null : { kind: 'dismiss' };
    default:
      return null;
  }
}
