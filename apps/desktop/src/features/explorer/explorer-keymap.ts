/**
 * What a keystroke means in the file tree (T042).
 *
 * Matched on `event.code`, never `event.key`, for the reason
 * `permission-chords.ts` and `terminal-chords.ts` both give: on macOS a
 * modifier composes, so `key` is whatever character came out the other side.
 * `code` is the physical key and is what a chord is actually about. The one
 * deliberate exception is type-ahead, which *is* about the character produced —
 * so it reads `key`, and only when no modifier is held.
 *
 * Pure and separate from the component, so the mapping is a test rather than a
 * click.
 */

export interface ExplorerChord {
  readonly code: string;
  /** Only consulted for type-ahead; see the module note. */
  readonly key: string;
  readonly metaKey: boolean;
  readonly ctrlKey: boolean;
  readonly altKey: boolean;
  readonly shiftKey: boolean;
}

export type ExplorerIntent =
  /** Move the focus by `delta` rows; `extend` is Shift, growing the range. */
  | { readonly kind: 'move'; readonly delta: number; readonly extend: boolean }
  | { readonly kind: 'edge'; readonly edge: 'first' | 'last'; readonly extend: boolean }
  /** Right: open a directory, or step into one that is already open. */
  | { readonly kind: 'expand' }
  /** Left: close a directory, or step out to its parent. */
  | { readonly kind: 'collapse' }
  | { readonly kind: 'open' }
  | { readonly kind: 'preview' }
  | { readonly kind: 'rename' }
  | { readonly kind: 'delete'; readonly permanent: boolean }
  | { readonly kind: 'cut' }
  | { readonly kind: 'copy' }
  | { readonly kind: 'paste' }
  | { readonly kind: 'duplicate' }
  | { readonly kind: 'undo' }
  | { readonly kind: 'redo' }
  | { readonly kind: 'selectAll' }
  | { readonly kind: 'copyPath'; readonly relative: boolean }
  | { readonly kind: 'typeahead'; readonly character: string };

/** The command modifier, which either key may carry (the repo's convention). */
function command(event: ExplorerChord): boolean {
  return event.metaKey || event.ctrlKey;
}

/**
 * The intent a keystroke carries, or `null` for one the tree does not claim —
 * which matters, because anything unclaimed must reach the window's own
 * shortcuts rather than being swallowed here.
 *
 * **Rename is F2, and Enter opens.** VS Code's macOS default is the other way
 * round (Enter renames, ⌘↓ opens), and it is not copied: the tree already opens
 * on click, so binding Enter to rename would make the most-pressed key in a
 * list do the one thing that changes the disk.
 */
export function explorerIntentFor(event: ExplorerChord): ExplorerIntent | null {
  const meta = command(event);

  // Type-ahead first only in the sense that it is the fallback; every chord
  // below wins over it.
  switch (event.code) {
    case 'ArrowDown':
      return meta || event.altKey ? null : { kind: 'move', delta: 1, extend: event.shiftKey };
    case 'ArrowUp':
      return meta || event.altKey ? null : { kind: 'move', delta: -1, extend: event.shiftKey };
    case 'PageDown':
      return { kind: 'move', delta: 10, extend: event.shiftKey };
    case 'PageUp':
      return { kind: 'move', delta: -10, extend: event.shiftKey };
    case 'Home':
      return { kind: 'edge', edge: 'first', extend: event.shiftKey };
    case 'End':
      return { kind: 'edge', edge: 'last', extend: event.shiftKey };
    case 'ArrowRight':
      return meta || event.altKey ? null : { kind: 'expand' };
    case 'ArrowLeft':
      return meta || event.altKey ? null : { kind: 'collapse' };
    case 'Enter':
    case 'NumpadEnter':
      return meta || event.altKey || event.shiftKey ? null : { kind: 'open' };
    case 'Space':
      return meta || event.altKey || event.shiftKey ? null : { kind: 'preview' };
    case 'F2':
      return meta || event.altKey || event.shiftKey ? null : { kind: 'rename' };
    // ⌫ and ⌦ both delete, with or without ⌘ — ⌘⌫ is the macOS habit and
    // plain ⌫ is everyone else's, and they should not mean different things.
    case 'Backspace':
    case 'Delete':
      return event.altKey ? null : { kind: 'delete', permanent: event.shiftKey };
    // The letter chords all `break` rather than returning `null` when their
    // modifier is absent, so a bare `x` or `a` still reaches type-ahead below.
    // Returning early here would make the letters that happen to carry a
    // shortcut the only ones you cannot type to jump with.
    case 'KeyX':
      if (meta && !event.altKey && !event.shiftKey) return { kind: 'cut' };
      break;
    case 'KeyC':
      // ⌥⌘C is Copy Path, ⇧⌥⌘C is Copy Relative Path — VS Code's bindings.
      if (meta && event.altKey) return { kind: 'copyPath', relative: event.shiftKey };
      if (meta && !event.shiftKey) return { kind: 'copy' };
      break;
    case 'KeyV':
      if (meta && !event.altKey && !event.shiftKey) return { kind: 'paste' };
      break;
    case 'KeyD':
      if (meta && !event.altKey && !event.shiftKey) return { kind: 'duplicate' };
      break;
    case 'KeyA':
      if (meta && !event.altKey && !event.shiftKey) return { kind: 'selectAll' };
      break;
    case 'KeyZ':
      if (meta && !event.altKey) return event.shiftKey ? { kind: 'redo' } : { kind: 'undo' };
      break;
    case 'KeyY':
      // Windows' redo. Harmless to accept everywhere.
      if (meta && !event.altKey && !event.shiftKey) return { kind: 'redo' };
      break;
    default:
      break;
  }

  // Type-ahead: one printable character, no command modifier. `altKey` is
  // excluded because on macOS it composes (⌥e is a dead key), and the composed
  // result is not what the user is spelling.
  if (meta || event.altKey) return null;
  if (event.key.length !== 1) return null;
  if (event.key === ' ') return null;
  return { kind: 'typeahead', character: event.key };
}
