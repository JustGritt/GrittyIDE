import { describe, expect, it } from 'vitest';
import { isInstructionFile } from './instruction-files';

describe('isInstructionFile', () => {
  it('recognizes README.md and AGENTS.md by name, anywhere, any case', () => {
    expect(isInstructionFile('README.md')).toBe(true);
    expect(isInstructionFile('docs/readme.MD')).toBe(true);
    expect(isInstructionFile('AGENTS.md')).toBe(true);
    expect(isInstructionFile('CLAUDE.md')).toBe(false);
    expect(isInstructionFile('README.txt')).toBe(false);
    expect(isInstructionFile('src/readme')).toBe(false);
  });
});
