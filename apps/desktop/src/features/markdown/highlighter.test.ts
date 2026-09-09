import type { CodeHighlighter } from '@gritty/editor/monaco';
import { describe, expect, it, vi } from 'vitest';
import { createCachedHighlighter } from './highlighter';

function fakeBase(): CodeHighlighter & { calls: number } {
  const base = {
    calls: 0,
    resolveLanguage: (alias: string) =>
      alias === 'ts' ? 'typescript' : alias === 'rust' ? 'rust' : null,
    colorize: vi.fn(async (languageId: string, code: string) => {
      base.calls += 1;
      return `<span class="mtk1">${languageId}:${code}</span>`;
    }),
  };
  return base;
}

describe('createCachedHighlighter', () => {
  it('misses synchronously, colorizes asynchronously, then hits', async () => {
    const base = fakeBase();
    const highlighter = createCachedHighlighter(async () => base, 2);
    expect(highlighter.sync('ts', 'a')).toBeNull();
    expect(await highlighter.async('ts', 'a')).toBe('<span class="mtk1">typescript:a</span>');
    expect(highlighter.sync('TS', 'a')).toBe('<span class="mtk1">typescript:a</span>');
    expect(await highlighter.async('ts', 'a')).toBe('<span class="mtk1">typescript:a</span>');
    expect(base.calls).toBe(1);
  });

  it('returns null for unknown languages without colorizing', async () => {
    const base = fakeBase();
    const highlighter = createCachedHighlighter(async () => base);
    expect(await highlighter.async('nope', 'x')).toBeNull();
    expect(base.calls).toBe(0);
  });

  it('does not load Monaco until a fence misses the cache, and loads it once', async () => {
    const base = fakeBase();
    const load = vi.fn(async () => base);
    const highlighter = createCachedHighlighter(load, 2);
    // A preview with no fences, or one served entirely from cache, must not be
    // what pulls in a 4.2 MB chunk (MONACO-DEFER).
    expect(highlighter.sync('ts', 'a')).toBeNull();
    expect(load).not.toHaveBeenCalled();
    await highlighter.async('ts', 'a');
    await highlighter.async('rust', 'b');
    await highlighter.async('nope', 'c');
    expect(load).toHaveBeenCalledTimes(1);
  });

  it('evicts the least recently used entry', async () => {
    const base = fakeBase();
    const highlighter = createCachedHighlighter(async () => base, 2);
    await highlighter.async('ts', '1');
    await highlighter.async('ts', '2');
    highlighter.sync('ts', '1'); // touch 1 so 2 is the oldest
    await highlighter.async('rust', '3');
    expect(highlighter.sync('ts', '2')).toBeNull();
    expect(highlighter.sync('ts', '1')).not.toBeNull();
    expect(highlighter.sync('rust', '3')).not.toBeNull();
  });
});
