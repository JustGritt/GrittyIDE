import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { detectEol, dominantEol, isEditableEol, normalizeEol } from './eol';

function fixture(name: string): string {
  return readFileSync(
    new URL(`../../../fixtures/weird-files/encoding/${name}`, import.meta.url),
    'utf8',
  );
}

describe('detectEol', () => {
  it('classifies the fixtures the core keeps verbatim', () => {
    expect(detectEol(fixture('crlf.txt'))).toBe('crlf');
    expect(detectEol(fixture('mixed-eol.txt'))).toBe('mixed');
    expect(detectEol(fixture('utf8-bom.txt'))).toBe('lf');
  });

  it('distinguishes lone CR, empty and single-line text', () => {
    expect(detectEol('a\rb\rc')).toBe('cr');
    expect(detectEol('')).toBe('none');
    expect(detectEol('one line without newline')).toBe('none');
    expect(detectEol('a\n')).toBe('lf');
    expect(detectEol('a\r\n')).toBe('crlf');
    expect(detectEol('a\r\nb\n')).toBe('mixed');
    expect(detectEol('a\r\nb\r')).toBe('mixed');
  });

  it('only lets consistent text into a model', () => {
    expect(isEditableEol('lf')).toBe(true);
    expect(isEditableEol('crlf')).toBe(true);
    expect(isEditableEol('none')).toBe(true);
    expect(isEditableEol('cr')).toBe(false);
    expect(isEditableEol('mixed')).toBe(false);
  });
});

describe('normalizeEol', () => {
  it('is the identity for already consistent text', () => {
    const crlf = fixture('crlf.txt');
    expect(normalizeEol(crlf, 'crlf')).toBe(crlf);
    expect(normalizeEol('a\nb\n', 'lf')).toBe('a\nb\n');
  });

  it('rewrites every kind of line ending', () => {
    expect(normalizeEol(fixture('mixed-eol.txt'), 'lf')).toBe('a\nb\nc\nd\n');
    expect(normalizeEol(fixture('mixed-eol.txt'), 'crlf')).toBe('a\r\nb\r\nc\r\nd\r\n');
    expect(normalizeEol('a\rb', 'lf')).toBe('a\nb');
    expect(normalizeEol(fixture('crlf.txt'), 'lf')).not.toContain('\r');
  });
});

describe('dominantEol', () => {
  it('mirrors what Monaco would pick', () => {
    expect(dominantEol('a\nb\r\nc\rd\n')).toBe('lf'); // 2 CR-carrying of 4
    expect(dominantEol('a\r\nb\r\nc\n')).toBe('crlf');
    expect(dominantEol('a\rb\rc\n')).toBe('crlf');
    expect(dominantEol('')).toBe('lf');
  });
});
