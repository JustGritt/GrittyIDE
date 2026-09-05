import type { Terminal, WorkspaceId } from '@gritty/protocol';
import { describe, expect, it } from 'vitest';
import {
  describeCwd,
  MAX_TERMINAL_NAME_CHARS,
  normalizeTerminalName,
  pillLocation,
  pillStatus,
  pillTone,
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

const workspaceId = 'ws-1' as WorkspaceId;
const other = 'ws-2' as WorkspaceId;
const workspace = { id: workspaceId, name: 'GrittyIDE' };
const pill = (overrides: Partial<Terminal> = {}) =>
  ({ shell: '/bin/zsh', workspaceId, cwd: '', status: 'RUNNING', ...overrides }) as Terminal;

describe('pillTone', () => {
  it('is running while the shell is', () => {
    expect(pillTone(pill())).toBe('running');
  });

  it('is idle for a shell that ended the way it was asked to', () => {
    expect(pillTone(pill({ status: 'EXITED', exitCode: 0 }))).toBe('idle');
  });

  it('is failed for a non-zero exit', () => {
    expect(pillTone(pill({ status: 'EXITED', exitCode: 101 }))).toBe('failed');
  });

  it('is failed for a signal, whatever the code says', () => {
    expect(pillTone(pill({ status: 'EXITED', exitSignal: 'SIGKILL' }))).toBe('failed');
  });

  it('is idle when neither a code nor a signal was reported', () => {
    expect(pillTone(pill({ status: 'EXITED' }))).toBe('idle');
  });
});

describe('pillLocation', () => {
  it('names the workspace for a shell at its root', () => {
    expect(pillLocation(pill({ cwd: '' }), workspace)).toBe('GrittyIDE');
  });

  it('is the relative path for a shell deeper in', () => {
    expect(pillLocation(pill({ cwd: 'crates/terminal' }), workspace)).toBe('crates/terminal');
  });

  it('is ~ for a session with no workspace', () => {
    expect(pillLocation(pill({ workspaceId: undefined, cwd: undefined }), workspace)).toBe('~');
  });

  it('says nothing for a root shell of a workspace that is no longer open', () => {
    expect(pillLocation(pill({ workspaceId: other, cwd: '' }), workspace)).toBeNull();
  });

  it('says nothing for a root shell when no workspace is open', () => {
    expect(pillLocation(pill({ cwd: '' }), null)).toBeNull();
  });

  it('still shows a relative path from a workspace that is not the open one', () => {
    expect(pillLocation(pill({ workspaceId: other, cwd: 'src' }), workspace)).toBe('src');
  });
});

describe('pillStatus', () => {
  it('says nothing while the shell runs', () => {
    expect(pillStatus(pill())).toBeNull();
  });

  it('is the artboard’s “exit 101” for a failure', () => {
    expect(pillStatus(pill({ status: 'EXITED', exitCode: 101 }))).toBe('exit 101');
  });

  it('is “exited” for a clean end', () => {
    expect(pillStatus(pill({ status: 'EXITED', exitCode: 0 }))).toBe('exited');
  });

  it('names the signal that ended it', () => {
    expect(pillStatus(pill({ status: 'EXITED', exitSignal: 'SIGKILL' }))).toBe('killed SIGKILL');
  });

  it('is “exited” when nothing was reported', () => {
    expect(pillStatus(pill({ status: 'EXITED' }))).toBe('exited');
  });
});

describe('describeCwd', () => {
  it('spells out the three rootings for the tooltip', () => {
    expect(describeCwd(pill({ cwd: undefined }))).toBe('your home directory');
    expect(describeCwd(pill({ cwd: '' }))).toBe('the workspace root');
    expect(describeCwd(pill({ cwd: 'crates/terminal' }))).toBe('crates/terminal');
  });
});
