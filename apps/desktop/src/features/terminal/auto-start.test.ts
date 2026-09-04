import { describe, expect, it } from 'vitest';
import { shouldStartFirstTerminal } from './auto-start';

const base = { open: true, ready: true, sessions: 0, startedForThisOpening: false };

describe('shouldStartFirstTerminal', () => {
  it('starts a shell when the dock opens onto nothing', () => {
    expect(shouldStartFirstTerminal(base)).toBe(true);
  });

  it('does nothing while the dock is closed', () => {
    // The dock stays mounted so xterm keeps its scrollback; that must not be
    // mistaken for the user asking to see a terminal.
    expect(shouldStartFirstTerminal({ ...base, open: false })).toBe(false);
  });

  it('waits until the core has been asked what it already has', () => {
    // Starting before `terminal.list` answers would duplicate a session that
    // survived a renderer reload, and before `terminal.profiles` answers it
    // could start the wrong shell.
    expect(shouldStartFirstTerminal({ ...base, ready: false })).toBe(false);
  });

  it('leaves an existing session alone', () => {
    expect(shouldStartFirstTerminal({ ...base, sessions: 1 })).toBe(false);
  });

  it('starts at most one per opening, so the last tab can be closed', () => {
    // Otherwise closing the only terminal would immediately grow another and
    // the user could never get rid of it.
    expect(shouldStartFirstTerminal({ ...base, startedForThisOpening: true })).toBe(false);
  });

  it('does not retry a shell that failed to start', () => {
    // The latch is set before the spawn resolves, so a failure shows its error
    // envelope once rather than looping.
    expect(shouldStartFirstTerminal({ ...base, startedForThisOpening: true, sessions: 0 })).toBe(
      false,
    );
  });
});
