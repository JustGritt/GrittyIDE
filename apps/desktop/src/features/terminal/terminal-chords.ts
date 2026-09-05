/**
 * ⌥1…⌥9 select a session, which is the chord the artboard prints on the bar.
 *
 * Matched on `code`, never on `key`: on macOS Option composes, so ⌥1 arrives as
 * `¡` and ⌥2 as `™`. `island.tsx` documents the same trap for ⌥A.
 */

/** What the bar prints, so the hint and the handler cannot drift. */
export const TERMINAL_CHORD_HINT = '⌥1…9';

/** The highest session a chord can reach. */
export const MAX_TERMINAL_CHORDS = 9;

export interface ChordEvent {
  readonly altKey: boolean;
  readonly ctrlKey: boolean;
  readonly metaKey: boolean;
  readonly shiftKey: boolean;
  readonly code: string;
}

/**
 * The zero-based session a keystroke asks for, or `null` if it asks for
 * nothing. Every other modifier is refused: ⌥⇧1 is a different chord, and
 * ⌘⌥1 belongs to whatever claims it next.
 */
export function terminalChordIndex(event: ChordEvent): number | null {
  if (!event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return null;
  const match = /^Digit([1-9])$/.exec(event.code);
  if (!match?.[1]) return null;
  return Number(match[1]) - 1;
}
