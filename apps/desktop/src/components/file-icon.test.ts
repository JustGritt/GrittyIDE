import { describe, expect, it } from 'vitest';
import { fileIconKind } from './file-icon';

describe('fileIconKind', () => {
  it('maps the extensions this repository is made of', () => {
    expect(fileIconKind('explorer-tree.tsx')).toBe('react');
    expect(fileIconKind('vitest.config.ts')).toBe('typescript');
    expect(fileIconKind('index.d.ts')).toBe('typescript');
    expect(fileIconKind('lib.rs')).toBe('rust');
    expect(fileIconKind('ADR-003-ui-core-boundary.md')).toBe('markdown');
    expect(fileIconKind('package.json')).toBe('json');
    expect(fileIconKind('tsconfig.tsbuildinfo')).toBe('json');
    expect(fileIconKind('Cargo.toml')).toBe('config');
    expect(fileIconKind('Cargo.lock')).toBe('lock');
    expect(fileIconKind('pnpm-workspace.yaml')).toBe('yaml');
    expect(fileIconKind('icon.icns')).toBe('image');
  });

  it('ignores case, so an upper-case extension still gets its icon', () => {
    expect(fileIconKind('NOTES.MD')).toBe('markdown');
    expect(fileIconKind('Photo.JPG')).toBe('image');
  });

  it('lets README and LICENSE outrank their extension', () => {
    expect(fileIconKind('README.md')).toBe('readme');
    expect(fileIconKind('readme')).toBe('readme');
    expect(fileIconKind('LICENSE')).toBe('license');
    expect(fileIconKind('LICENSE-MIT')).toBe('license');
    expect(fileIconKind('licenses.txt')).toBe('text');
    expect(fileIconKind('READMEFIRST.txt')).toBe('text');
  });

  it('reads a dotfile by its name, not as an extension', () => {
    expect(fileIconKind('.gitignore')).toBe('git');
    expect(fileIconKind('.gitattributes')).toBe('git');
    expect(fileIconKind('.editorconfig')).toBe('config');
    expect(fileIconKind('.nvmrc')).toBe('text');
    expect(fileIconKind('.env.local')).toBe('config');
    // An unknown dotfile is not "a file with the extension hidden-file".
    expect(fileIconKind('.hidden-file')).toBe('file');
  });

  it('falls back to the generic file for anything it does not know', () => {
    expect(fileIconKind('Makefile')).toBe('file');
    expect(fileIconKind('archive.bin')).toBe('file');
    expect(fileIconKind('trailing.')).toBe('file');
    expect(fileIconKind('')).toBe('file');
  });
});
