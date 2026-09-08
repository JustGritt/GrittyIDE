/**
 * ⌥G folds and unfolds the sidebar's Agents section (T233-C).
 *
 * The rail's chord, inherited: it meant "show me the agents" and it still does,
 * now that they are in the sidebar rather than in a column of their own. Taking
 * it over rather than picking a new letter also means a user who had learned it
 * does not have to unlearn it.
 *
 * `event.code`, never `event.key`: on macOS Option composes, so ⌥G arrives as
 * `©` and a matcher on `key` would never fire. `terminal-chords.ts` and
 * `permission-chords.ts` document the same trap.
 *
 * Modifiers are rejected rather than ignored. ⌥⌘A is the attention queue's
 * (`permission-banner.ts`), and a chord that fired on any superset of its own
 * modifiers would steal from whatever was more specific.
 */
export const AGENTS_SECTION_CHORD_HINT = '⌥G';

export interface ChordEvent {
  readonly altKey: boolean;
  readonly ctrlKey: boolean;
  readonly metaKey: boolean;
  readonly shiftKey: boolean;
  readonly code: string;
}

export function isAgentsSectionChord(event: ChordEvent): boolean {
  if (!event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return false;
  return event.code === 'KeyG';
}
