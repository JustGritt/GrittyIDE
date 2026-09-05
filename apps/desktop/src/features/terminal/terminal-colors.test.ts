import { describe, expect, it } from 'vitest';
import { colorLabel, colorModifier, colorVariable, TERMINAL_COLORS } from './terminal-colors';

describe('the terminal colour palette', () => {
  it('offers the six ANSI hues the terminal already paints with', () => {
    expect(TERMINAL_COLORS).toEqual(['RED', 'GREEN', 'YELLOW', 'BLUE', 'MAGENTA', 'CYAN']);
  });

  it('draws each one with the token of the same name, so the ramps follow', () => {
    expect(colorVariable('RED')).toBe('var(--ansi-red)');
    expect(colorVariable('MAGENTA')).toBe('var(--ansi-magenta)');
  });

  it('names each one for a menu without shouting', () => {
    expect(colorLabel('RED')).toBe('Red');
    expect(colorLabel('MAGENTA')).toBe('Magenta');
  });

  it('gives the pill one class per hue', () => {
    expect(colorModifier('CYAN')).toBe('cyan');
  });

  it('has a token, a label and a class for every colour it offers', () => {
    for (const color of TERMINAL_COLORS) {
      expect(colorVariable(color)).toMatch(/^var\(--ansi-[a-z]+\)$/);
      expect(colorLabel(color)).toMatch(/^[A-Z][a-z]+$/);
      expect(colorModifier(color)).toMatch(/^[a-z]+$/);
    }
  });
});
