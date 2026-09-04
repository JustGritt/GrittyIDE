import type { Terminal } from '@gritty/protocol';
import { describe, expect, it } from 'vitest';
import {
  MAX_TERMINAL_NAME_CHARS,
  normalizeTerminalName,
  shellName,
  terminalLabel,
} from './terminal-naming';

const session = (overrides: Partial<Terminal> = {}) =>
  ({ shell: '/bin/zsh', ...overrides }) as Terminal;

describe('shellName', () => {
  it('is the shell that runs in the session', () => {
    expect(shellName('/opt/homebrew/bin/fish')).toBe('fish');
  });

  it('never produces an empty label', () => {
    expect(shellName('')).toBe('shell');
    expect(shellName('/')).toBe('shell');
  });
});

describe('terminalLabel', () => {
  it('prefers the name the user gave', () => {
    expect(terminalLabel(session({ name: 'claude' }))).toBe('claude');
  });

  it('falls back to the shell when there is no name', () => {
    expect(terminalLabel(session())).toBe('zsh');
  });
});

describe('normalizeTerminalName', () => {
  it('trims what the user typed', () => {
    expect(normalizeTerminalName('  claude  ')).toEqual({ name: 'claude' });
  });

  it('reads a blank field as "clear the label", not as an error', () => {
    // A tab with no text is unclickable, so an empty name is never stored.
    expect(normalizeTerminalName('')).toEqual({ name: null });
    expect(normalizeTerminalName('   ')).toEqual({ name: null });
  });

  it('refuses what a tab cannot draw', () => {
    expect(normalizeTerminalName('two\nlines')).toHaveProperty('error');
    expect(normalizeTerminalName('escape\u001b[31m')).toHaveProperty('error');
  });

  it('counts characters, not UTF-16 units', () => {
    // An emoji is one character on screen whatever it costs in the string.
    const emoji = '\u{1f642}'.repeat(MAX_TERMINAL_NAME_CHARS);
    expect(normalizeTerminalName(emoji)).toEqual({ name: emoji });
    expect(normalizeTerminalName('x'.repeat(MAX_TERMINAL_NAME_CHARS + 1))).toHaveProperty('error');
  });
});
