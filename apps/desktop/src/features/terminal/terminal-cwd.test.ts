import { describe, expect, it } from 'vitest';
import { cwdForDocument } from './terminal-cwd';

describe('cwdForDocument', () => {
  it('is the folder holding the document', () => {
    expect(cwdForDocument('crates/terminal/src/system.rs')).toBe('crates/terminal/src');
  });

  it('is the workspace root for a top-level file', () => {
    // '' is what the core reads as the root, so a README tab roots a shell
    // exactly where "New terminal" did before this existed.
    expect(cwdForDocument('README.md')).toBe('');
  });

  it('is one level up for a file one level down', () => {
    expect(cwdForDocument('docs/tasks/README.md')).toBe('docs/tasks');
  });

  it('has nothing to follow when no document is open', () => {
    expect(cwdForDocument(null)).toBeUndefined();
    expect(cwdForDocument(undefined)).toBeUndefined();
  });

  it('treats a path that is already a folder-shaped string literally', () => {
    // It is only ever handed a document path, and a document is a file; a
    // trailing segment is a file name whether or not it has an extension.
    expect(cwdForDocument('Makefile')).toBe('');
    expect(cwdForDocument('scripts/build')).toBe('scripts');
  });
});
