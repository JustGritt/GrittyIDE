import { describe, expect, it } from 'vitest';
import { applyHighlights } from './highlight-dom';
import { createSanitizer } from './sanitize';
import { parse, testRenderer } from './testing/fixtures';

const sanitizer = createSanitizer(window);
const renderer = testRenderer();

function deferred<T>() {
  let resolve: (value: T) => void = () => undefined;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

describe('applyHighlights', () => {
  const source = '```ts\nlet a = 1;\n```\n\n```rust\nfn f() {}\n```\n\n```nope\nx\n```\n';

  it('colorizes pending blocks in place and sanitizes what the highlighter returns', async () => {
    const result = renderer.render(source, { path: 'x.md' });
    const root = parse(result.html);
    expect(result.pendingCode.map((p) => p.lang)).toEqual(['ts', 'rust', 'nope']);
    const applied = await applyHighlights(
      root,
      result.pendingCode,
      async (lang, code) =>
        lang === 'nope'
          ? null
          : `<span class="mtk5">${lang}</span><script>alert(1)</script>${code.length}`,
      sanitizer,
      new AbortController().signal,
    );
    expect(applied).toBe(2);
    const codes = [...root.querySelectorAll('code')].map((c) => c.innerHTML);
    expect(codes[0]).toBe('<span class="mtk5">ts</span>11');
    expect(codes[1]).toBe('<span class="mtk5">rust</span>10');
    expect(codes[2]).toBe('x\n');
    expect(root.querySelector('script')).toBeNull();
  });

  it('drops results that arrive after the render was superseded', async () => {
    const result = renderer.render(source, { path: 'x.md' });
    const root = parse(result.html);
    const controller = new AbortController();
    const gate = deferred<string>();
    const run = applyHighlights(
      root,
      result.pendingCode,
      () => gate.promise,
      sanitizer,
      controller.signal,
    );
    controller.abort();
    gate.resolve('<span>late</span>');
    expect(await run).toBe(0);
    expect(root.querySelector('code')?.innerHTML).toBe('let a = 1;\n');
  });

  it('skips blocks a failing highlighter rejects and honours the limits', async () => {
    const result = renderer.render(source, { path: 'x.md' });
    const root = parse(result.html);
    const applied = await applyHighlights(
      root,
      result.pendingCode,
      async (lang) => {
        if (lang === 'ts') throw new Error('grammar failed');
        return `<span>${lang}</span>`;
      },
      sanitizer,
      new AbortController().signal,
      { maxBlocks: 2, maxBytes: 1000 },
    );
    expect(applied).toBe(1);
    expect(root.querySelectorAll('code')[0]?.innerHTML).toBe('let a = 1;\n');
    expect(root.querySelectorAll('code')[1]?.innerHTML).toBe('<span>rust</span>');
  });
});
