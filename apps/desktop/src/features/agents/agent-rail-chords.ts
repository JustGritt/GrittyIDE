/**
 * ⌥G toggles the agent rail, which is the chord its header prints.
 *
 * Matched on `code`, never on `key`, for the reason `terminal-chords.ts`
 * documents: on macOS Option composes, so ⌥G arrives as `©` and a handler
 * keyed on the character would never fire. There is no central shortcut
 * registry, so the letter is chosen against the two listeners that already
 * exist — the terminal dock's ⌥1…⌥9 and the island's ⌥A — and the rail's
 * listener is additionally gated off while the island is open.
 */

/** What the header prints, so the hint and the handler cannot drift. */
export const AGENT_RAIL_CHORD_HINT = '⌥G';

export interface ChordEvent {
  readonly altKey: boolean;
  readonly ctrlKey: boolean;
  readonly metaKey: boolean;
  readonly shiftKey: boolean;
  readonly code: string;
}

/**
 * Whether a keystroke asks for the rail. Every other modifier is refused: ⌥⇧G
 * is a different chord and ⌘⌥G belongs to whatever claims it next.
 */
export function isAgentRailChord(event: ChordEvent): boolean {
  if (!event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return false;
  return event.code === 'KeyG';
}
