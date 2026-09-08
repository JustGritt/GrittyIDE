import { describe, expect, it } from 'vitest';
import { checkName, checkNewPath, copyName, isWindowsReserved, splitName } from './explorer-names';

describe('checkName', () => {
  it('accepts ordinary names, including dotfiles and Unicode', () => {
    for (const name of ['index.ts', '.gitignore', 'Makefile', 'café.md', 'a b.txt']) {
      expect(checkName(name), name).toBeNull();
    }
  });

  it('refuses names that would not survive the path policy', () => {
    expect(checkName('')).toMatch(/empty/i);
    expect(checkName('.')).toMatch(/not a name/i);
    expect(checkName('..')).toMatch(/not a name/i);
    expect(checkName('a/b')).toMatch(/"\/"/);
    expect(checkName('a\\b')).toMatch(/\\/);
    expect(checkName('a\0b')).toMatch(/null/i);
  });

  it('refuses leading and trailing whitespace, which the tree cannot show', () => {
    expect(checkName(' a.ts')).toMatch(/space/i);
    expect(checkName('a.ts ')).toMatch(/space/i);
    expect(checkName('a.ts.')).toMatch(/dot/i);
  });
});

describe('checkNewPath', () => {
  it('accepts a nested path, because one create makes the directories', () => {
    expect(checkNewPath('a/b/c.ts')).toBeNull();
    expect(checkNewPath('c.ts')).toBeNull();
  });

  it('refuses an absolute path, a trailing slash and an empty segment', () => {
    expect(checkNewPath('/etc/passwd')).toMatch(/relative/i);
    expect(checkNewPath('a/')).toMatch(/"\/"/);
    expect(checkNewPath('a//b')).toMatch(/empty segment/i);
  });

  it('holds every segment to the same rule, so traversal cannot sneak in', () => {
    expect(checkNewPath('a/../b')).toMatch(/not a name/i);
    expect(checkNewPath('../b')).toMatch(/not a name/i);
  });
});

describe('splitName', () => {
  it('splits at the last dot and keeps a dotfile whole', () => {
    expect(splitName('a.ts')).toEqual(['a', '.ts']);
    expect(splitName('a.tar.gz')).toEqual(['a.tar', '.gz']);
    expect(splitName('.gitignore')).toEqual(['.gitignore', '']);
    expect(splitName('Makefile')).toEqual(['Makefile', '']);
  });
});

describe('copyName', () => {
  it('leaves a free name alone', () => {
    expect(copyName('a.ts', () => false)).toBe('a.ts');
  });

  it('adds " copy", then numbers, before the extension', () => {
    expect(copyName('a.ts', (candidate) => candidate === 'a.ts')).toBe('a copy.ts');
    const takenTwice = (candidate: string) => candidate === 'a.ts' || candidate === 'a copy.ts';
    expect(copyName('a.ts', takenTwice)).toBe('a copy 2.ts');
  });

  it('appends to a dotfile rather than splitting its name', () => {
    expect(copyName('.gitignore', (candidate) => candidate === '.gitignore')).toBe(
      '.gitignore copy',
    );
  });
});

describe('isWindowsReserved', () => {
  it('flags the reserved device names, with or without an extension', () => {
    for (const name of ['con', 'AUX', 'nul.txt', 'com1', 'LPT9.ts']) {
      expect(isWindowsReserved(name), name).toBe(true);
    }
  });

  it('does not flag names that merely start with one', () => {
    for (const name of ['console.ts', 'auxiliary.md', 'nullable.rs']) {
      expect(isWindowsReserved(name), name).toBe(false);
    }
  });
});
