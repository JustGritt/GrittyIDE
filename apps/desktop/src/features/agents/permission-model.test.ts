import type { AgentPermissionRequest } from '@gritty/protocol';
import { describe, expect, it } from 'vitest';
import { PERMISSION_CHORD_HINT, permissionVerdictFor } from './permission-chords';
import {
  askedAgo,
  blastRadius,
  changeSummary,
  diffSign,
  diffText,
  scopeLabel,
  verbLabel,
} from './permission-model';

function ask(overrides: Partial<AgentPermissionRequest> = {}): AgentPermissionRequest {
  return {
    askId: 'ask-1',
    sessionId: 'claude_code:s1',
    verb: 'WRITE_FILE',
    target: '~/p/crates/terminal/src/pty.rs',
    diff: [],
    added: 0,
    removed: 0,
    outsideRoot: false,
    sensitive: false,
    askedAt: 1_000,
    ...overrides,
  } as AgentPermissionRequest;
}

describe('what a permission card says', () => {
  it('names the verb in words a person reads rather than the wire value', () => {
    expect(verbLabel('WRITE_FILE')).toBe('write file');
    expect(verbLabel('RUN_COMMAND')).toBe('run command');
    expect(verbLabel('INSTALL_DEPENDENCY')).toBe('install dependency');
    expect(verbLabel('OTHER')).toBe('use tool');
  });

  it('leads the blast radius with the fact that changes an answer', () => {
    expect(blastRadius(ask({ outsideRoot: true, detail: 'reaches the network' }))).toEqual([
      'outside its worktree',
      'reaches the network',
    ]);
    // Nothing is invented for an ordinary write inside the worktree.
    expect(blastRadius(ask())).toEqual([]);
  });

  it('says out loud that a redacted file was not looked at', () => {
    // A card with no diff and no explanation reads as a card with nothing to
    // show, which is a different claim from "we deliberately did not look".
    expect(blastRadius(ask({ sensitive: true }))).toEqual([
      'contents not shown — this file usually holds secrets',
    ]);
  });

  it('reports the size of the whole change, not of the visible diff', () => {
    expect(changeSummary(ask({ added: 64, removed: 11 }))).toBe('+64 −11 · 1 file');
    // A command has no size, and a write of nothing has nothing to report.
    expect(changeSummary(ask({ verb: 'RUN_COMMAND', target: 'cargo test' }))).toBeNull();
    expect(changeSummary(ask())).toBeNull();
  });

  it('offers a scope only when the core could name one', () => {
    expect(scopeLabel(ask({ suggestedScope: '~/p/crates/terminal/src' }))).toBe(
      'Allow writes in ~/p/crates/terminal/src for this run',
    );
    expect(
      scopeLabel(ask({ verb: 'RUN_COMMAND', target: 'cargo publish', suggestedScope: 'cargo' })),
    ).toBe('Allow cargo for this run');
    // No scope, no middle answer. An unscoped standing grant is a blanket yes.
    expect(scopeLabel(ask())).toBeNull();
  });

  it('keeps a diff line’s indentation, which is the only thing its sign is not', () => {
    expect(diffSign('+  self.reflow(cols);')).toBe('add');
    expect(diffSign('-  self.grid.resize();')).toBe('remove');
    expect(diffSign(' fn resize() {')).toBe('context');
    expect(diffText('+  self.reflow(cols);')).toBe('  self.reflow(cols);');
  });

  it('counts an age in seconds while a prompt is still fresh', () => {
    // "0m" would read as broken for the case this is usually in.
    expect(askedAgo(1_000, 9_000)).toBe('8s');
    expect(askedAgo(1_000, 100_000)).toBe('1m 39s');
    expect(askedAgo(1_000, 7_300_000)).toBe('2h 1m');
    // A clock that disagrees never yields a negative age.
    expect(askedAgo(9_000, 1_000)).toBe('0s');
  });
});

describe('the keys artboard 12a prints', () => {
  const chord = (over: Partial<Parameters<typeof permissionVerdictFor>[0]> = {}) => ({
    code: 'Enter',
    metaKey: false,
    ctrlKey: false,
    altKey: false,
    shiftKey: false,
    ...over,
  });

  it('maps each printed hint to the verdict beside it', () => {
    expect(permissionVerdictFor(chord(), true)).toBe('ALLOW_ONCE');
    expect(permissionVerdictFor(chord({ metaKey: true }), true)).toBe('ALLOW_FOR_RUN');
    expect(permissionVerdictFor(chord({ ctrlKey: true }), true)).toBe('ALLOW_FOR_RUN');
    expect(permissionVerdictFor(chord({ code: 'Escape' }), true)).toBe('DENY');
    expect(PERMISSION_CHORD_HINT).toEqual({
      ALLOW_ONCE: '⏎',
      ALLOW_FOR_RUN: '⌘⏎',
      DENY: 'esc',
    });
  });

  it('does nothing rather than something else when there is no scope to grant', () => {
    // Falling back to ALLOW_ONCE would take a different decision from the one
    // the key is labelled with, on a surface where that matters most.
    expect(permissionVerdictFor(chord({ metaKey: true }), false)).toBeNull();
    expect(permissionVerdictFor(chord(), false)).toBe('ALLOW_ONCE');
  });

  it('ignores a keystroke carrying a modifier the card does not print', () => {
    expect(permissionVerdictFor(chord({ altKey: true }), true)).toBeNull();
    expect(permissionVerdictFor(chord({ shiftKey: true }), true)).toBeNull();
    expect(permissionVerdictFor(chord({ code: 'KeyA' }), true)).toBeNull();
    expect(permissionVerdictFor(chord({ code: 'Escape', metaKey: true }), true)).toBeNull();
  });

  it('accepts the numeric keypad’s Enter, which is a different code', () => {
    expect(permissionVerdictFor(chord({ code: 'NumpadEnter' }), true)).toBe('ALLOW_ONCE');
  });
});
