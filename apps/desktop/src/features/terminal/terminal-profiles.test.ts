import type { TerminalProfile } from '@gritty/protocol';
import { describe, expect, it } from 'vitest';
import { addCustomShell, selectedProfile, spawnShell } from './terminal-profiles';

function profile(overrides: Partial<TerminalProfile> & { path: string }): TerminalProfile {
  return {
    name: overrides.path.split('/').pop() ?? '',
    isDefault: false,
    isCustom: false,
    available: true,
    ...overrides,
  };
}

const zsh = profile({ path: '/bin/zsh', isDefault: true });
const bash = profile({ path: '/bin/bash' });
const profiles = [zsh, bash];

describe('selectedProfile', () => {
  it('uses the remembered shell when the machine still has it', () => {
    expect(selectedProfile(profiles, '/bin/bash')).toBe(bash);
  });

  it('falls back to the default when nothing is remembered', () => {
    expect(selectedProfile(profiles, null)).toBe(zsh);
  });

  it('falls back to the default when the remembered shell is gone', () => {
    // The session file outlives the machine's software: a shell uninstalled
    // since the choice was made must not leave the picker pointing at it.
    expect(selectedProfile(profiles, '/opt/homebrew/bin/fish')).toBe(zsh);
  });

  it('falls back when the remembered shell is listed but unavailable', () => {
    // A custom entry stays listed so the settings can explain why it broke;
    // that is not a reason to try to start it.
    const broken = profile({
      path: '/opt/homebrew/bin/fish',
      isCustom: true,
      available: false,
      unavailableReason: 'no such file',
    });
    expect(selectedProfile([zsh, broken], '/opt/homebrew/bin/fish')).toBe(zsh);
  });

  it('reports nothing when the machine reported no shells', () => {
    expect(selectedProfile([], '/bin/bash')).toBeNull();
  });

  it('reports nothing when no profile claims to be the default', () => {
    expect(selectedProfile([bash], null)).toBeNull();
  });
});

describe('spawnShell', () => {
  it('sends no shell for the default, so the core resolves it', () => {
    expect(spawnShell(zsh)).toBeUndefined();
    expect(spawnShell(null)).toBeUndefined();
  });

  it('sends the path for any other choice', () => {
    expect(spawnShell(bash)).toBe('/bin/bash');
  });
});

describe('addCustomShell', () => {
  it('appends a path, keeping the order the user added them in', () => {
    expect(addCustomShell(['/bin/fish'], '  /opt/homebrew/bin/nu  ', 16)).toEqual({
      shells: ['/bin/fish', '/opt/homebrew/bin/nu'],
    });
  });

  it('refuses what the core would refuse anyway, without a round trip', () => {
    expect(addCustomShell([], '', 16)).toHaveProperty('error');
    expect(addCustomShell([], 'bin/fish', 16)).toHaveProperty('error');
    expect(addCustomShell(['/bin/fish'], '/bin/fish', 16)).toHaveProperty('error');
  });

  it('is bounded, like the core request it feeds', () => {
    expect(addCustomShell(['/bin/a', '/bin/b'], '/bin/c', 2)).toEqual({
      error: 'At most 2 custom shells.',
    });
  });
});
