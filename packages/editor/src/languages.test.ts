import { describe, expect, it } from 'vitest';
import { PLAINTEXT_LANGUAGE, resolveLanguageId } from './languages';

// A slice of what Monaco 0.56 registers at import time, not the real registry.
const REGISTERED = new Set([
  'plaintext',
  'typescript',
  'javascript',
  'rust',
  'json',
  'markdown',
  'yaml',
  'shell',
  'powershell',
  'dockerfile',
  'ini',
]);

describe('resolveLanguageId', () => {
  it('keeps ids Monaco registers', () => {
    for (const id of ['typescript', 'rust', 'json', 'markdown', 'powershell', 'dockerfile']) {
      expect(resolveLanguageId(id, REGISTERED)).toBe(id);
    }
  });

  it('falls back to plaintext for ids the core emits but Monaco lacks', () => {
    for (const id of ['toml', 'ignore', 'dotenv', 'makefile', 'cmake', 'unknown-language']) {
      expect(resolveLanguageId(id, REGISTERED)).toBe(PLAINTEXT_LANGUAGE);
    }
  });
});
