import { describe, expect, it } from 'vitest';
import { AGENT_RAIL_CHORD_HINT, type ChordEvent, isAgentRailChord } from './agent-rail-chords';

function chord(overrides: Partial<ChordEvent> = {}): ChordEvent {
  return {
    altKey: true,
    ctrlKey: false,
    metaKey: false,
    shiftKey: false,
    code: 'KeyG',
    ...overrides,
  };
}

describe('isAgentRailChord', () => {
  it('matches ⌥G', () => {
    expect(isAgentRailChord(chord())).toBe(true);
  });

  it('matches on the physical key, not the character Option composed', () => {
    // ⌥G is `©` on a US macOS layout, so a handler keyed on `key` would never
    // fire — `terminal-chords.ts` documents the same trap for ⌥1.
    expect(isAgentRailChord({ ...chord(), code: 'KeyG' })).toBe(true);
  });

  it('ignores G without Option', () => {
    expect(isAgentRailChord(chord({ altKey: false }))).toBe(false);
  });

  it('ignores every other modifier combination', () => {
    expect(isAgentRailChord(chord({ metaKey: true }))).toBe(false);
    expect(isAgentRailChord(chord({ ctrlKey: true }))).toBe(false);
    expect(isAgentRailChord(chord({ shiftKey: true }))).toBe(false);
  });

  it('leaves the chords the other listeners already own', () => {
    // The island's ⌥A and the dock's ⌥1…⌥9. There is no central registry, so
    // this is the check that the fourth listener does not collide with them.
    expect(isAgentRailChord(chord({ code: 'KeyA' }))).toBe(false);
    for (let digit = 1; digit <= 9; digit += 1) {
      expect(isAgentRailChord(chord({ code: `Digit${digit}` }))).toBe(false);
    }
  });

  it('prints the chord it matches', () => {
    expect(AGENT_RAIL_CHORD_HINT).toBe('⌥G');
  });
});
