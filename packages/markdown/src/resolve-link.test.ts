import { describe, expect, it } from 'vitest';
import { describeTarget, joinWorkspacePath, resolveLink } from './resolve-link';

describe('resolveLink', () => {
  const from = 'docs/guide/intro.md';

  it.each([
    ['same-dir.md', { kind: 'document', path: 'docs/guide/same-dir.md' }],
    ['./same-dir.md', { kind: 'document', path: 'docs/guide/same-dir.md' }],
    ['../api.md', { kind: 'document', path: 'docs/api.md' }],
    ['../../src/main.rs', { kind: 'document', path: 'src/main.rs' }],
    ['/README.md', { kind: 'document', path: 'README.md' }],
    ['sub/../x.md', { kind: 'document', path: 'docs/guide/x.md' }],
    ['target.ts:3', { kind: 'document', path: 'docs/guide/target.ts', line: 3 }],
    ['target.ts:3:5', { kind: 'document', path: 'docs/guide/target.ts', line: 3, column: 5 }],
    ['../deep.md#section', { kind: 'document', path: 'docs/deep.md', fragment: 'section' }],
    ['x.md?query=1', { kind: 'document', path: 'docs/guide/x.md' }],
    ['file%20with%20spaces.md', { kind: 'document', path: 'docs/guide/file with spaces.md' }],
    ['h%C3%A9llo.md', { kind: 'document', path: 'docs/guide/héllo.md' }],
    ['#Links Fixture', { kind: 'anchor', id: 'Links Fixture' }],
    ['https://example.com/a?b=c#d', { kind: 'external', url: 'https://example.com/a?b=c#d' }],
    ['http://example.com', { kind: 'external', url: 'http://example.com' }],
  ])('resolves %s', (href, expected) => {
    expect(resolveLink(href, from)).toEqual(expected);
  });

  it.each([
    ['', /empty/],
    ['#', /empty anchor/],
    ['../../../escape.md', /outside/],
    ['/../escape.md', /outside/],
    ['mailto:someone@example.com', /'mailto:'/],
    ['ftp://example.com/x', /'ftp:'/],
    ['file:///etc/passwd', /'file:'/],
    ['javascript:alert(1)', /'javascript:'/],
    ['C:/Users/x', /'c:'/],
    ['docs\\target.ts', /backslash/],
    ['docs/', /not a file/],
    ['./', /not a file/],
    ['/', /not a file/],
    ['%E0%A4%A', /percent/],
    ['%6Aavascript:alert(1)', /'javascript:'/],
  ])('refuses %s', (href, reason) => {
    const target = resolveLink(href, from);
    expect(target.kind).toBe('unsupported');
    expect(describeTarget(target)).toMatch(reason);
  });

  it('resolves from the root document', () => {
    expect(resolveLink('gfm.md', 'links.md')).toEqual({ kind: 'document', path: 'gfm.md' });
    expect(resolveLink('docs/target.ts:3:5', 'links.md')).toEqual({
      kind: 'document',
      path: 'docs/target.ts',
      line: 3,
      column: 5,
    });
    expect(resolveLink('../escape.md', 'links.md').kind).toBe('unsupported');
  });

  it('describes targets for tooltips', () => {
    expect(describeTarget(resolveLink('target.ts:3:5#frag', from))).toBe(
      'docs/guide/target.ts:3:5#frag',
    );
    expect(describeTarget(resolveLink('#x y', from))).toBe('#x y');
    expect(describeTarget(resolveLink('https://e.com', from))).toBe('https://e.com');
  });

  it('joins paths against the wire grammar', () => {
    expect(joinWorkspacePath('', 'a/b')).toBe('a/b');
    expect(joinWorkspacePath('a', './b//c/.')).toBe('a/b/c');
    expect(joinWorkspacePath('a/b', '../../c')).toBe('c');
    expect(joinWorkspacePath('a', '../../c')).toBeNull();
  });
});
