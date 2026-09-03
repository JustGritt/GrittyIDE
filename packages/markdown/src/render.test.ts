import { describe, expect, it } from 'vitest';
import { fixture, parse, testRenderer } from './testing/fixtures';

const renderer = testRenderer();

describe('createMarkdownRenderer', () => {
  it('renders the GFM fixture exactly as snapshotted', async () => {
    const result = renderer.render(fixture('gfm.md'), { path: 'gfm.md' });
    expect(result.failed).toBe(false);
    await expect(result.html).toMatchFileSnapshot('./__snapshots__/gfm.html');
  });

  it('mints prefixed, de-duplicated heading ids and reports the headings', () => {
    const result = renderer.render(fixture('gfm.md'), { path: 'gfm.md' });
    const root = parse(result.html);
    const ids = [...root.querySelectorAll('h1, h2, h3')].map((h) => h.id);
    expect(ids).toContain('md-gfm-fixture');
    expect(ids).toContain('md-duplicate');
    expect(ids).toContain('md-duplicate-1');
    expect(ids).toContain('md-ünïcödé--heading-with-punctuation');
    expect(ids.every((id) => id.startsWith('md-'))).toBe(true);
    expect(result.headings.find((h) => h.id === 'md-table')).toMatchObject({
      level: 2,
      text: 'Table',
    });
    expect(result.headings[0]?.line).toBe(0);
  });

  it('puts source lines on blocks, the same for LF and CRLF input', () => {
    const lf = 'para one\n\n# Heading\n\n- item\n\n```ts\ncode\n```\n';
    const crlf = lf.replace(/\n/g, '\r\n');
    const lines = (text: string) =>
      [...parse(renderer.render(text, { path: 'x.md' }).html).querySelectorAll('[data-line]')].map(
        (el) =>
          `${el.tagName.toLowerCase()}@${el.getAttribute('data-line')}-${el.getAttribute('data-line-end')}`,
      );
    expect(lines(lf).map((entry) => entry.split('-')[0])).toEqual([
      'p@0',
      'h1@2',
      'ul@4',
      'li@4',
      'pre@6',
    ]);
    expect(lines(lf)[4]).toBe('pre@6-9');
    expect(lines(crlf)).toEqual(lines(lf));
  });

  it('renders task lists as disabled checkboxes and leaves lookalikes alone', () => {
    const root = parse(renderer.render(fixture('gfm.md'), { path: 'gfm.md' }).html);
    const boxes = [...root.querySelectorAll('input')];
    expect(boxes).toHaveLength(3);
    expect(boxes.every((b) => b.type === 'checkbox' && b.disabled)).toBe(true);
    expect(boxes.map((b) => b.checked)).toEqual([false, true, true]);
    expect(root.querySelectorAll('li.md-task')).toHaveLength(3);
    expect(root.textContent).toContain('[ ]not a task');
    expect(root.textContent).toContain('[brackets]');
  });

  it('keeps table alignment without style attributes', () => {
    const root = parse(renderer.render(fixture('gfm.md'), { path: 'gfm.md' }).html);
    const headers = [...root.querySelectorAll('th')].map((th) => th.getAttribute('align'));
    expect(headers).toEqual(['left', 'center', 'right']);
    expect(root.querySelector('[style]')).toBeNull();
  });

  it('does not linkify bare file names, only scheme URLs', () => {
    const root = parse(renderer.render(fixture('gfm.md'), { path: 'gfm.md' }).html);
    const hrefs = [...root.querySelectorAll('a')].map((a) => a.getAttribute('href'));
    expect(hrefs).not.toContain('http://README.md');
    expect(hrefs).toContain('https://example.net/auto');
    expect(hrefs).toContain('https://example.org');
  });

  it('classifies links and rewrites hrefs to safe forms', () => {
    const root = parse(renderer.render(fixture('gfm.md'), { path: 'gfm.md' }).html);
    const byText = (text: string) =>
      [...root.querySelectorAll('a, span.md-link')].find((el) => el.textContent === text);
    expect(byText('Relative')).toMatchObject({
      dataset: { mdKind: 'document', mdPath: 'docs/target.ts' },
    });
    expect(byText('line and column')?.getAttribute('title')).toBe('docs/target.ts:3:5');
    expect(byText('line and column')).toMatchObject({ dataset: { mdLine: '3', mdColumn: '5' } });
    expect(byText('anchor')?.getAttribute('href')).toBe('#md-table');
    expect(byText('external')).toMatchObject({
      dataset: { mdKind: 'external', mdUrl: 'https://example.com/path?q=1' },
    });
    const mailto = byText('mailto');
    expect(mailto?.tagName).toBe('SPAN');
    expect(mailto?.getAttribute('href')).toBeNull();
    expect(mailto?.getAttribute('title')).toMatch(/mailto/);
  });

  it('renders inline data images and placeholders for anything else', () => {
    const root = parse(renderer.render(fixture('gfm.md'), { path: 'gfm.md' }).html);
    const images = root.querySelectorAll('img');
    expect(images).toHaveLength(1);
    expect(images[0]?.getAttribute('src')).toMatch(/^data:image\/png;base64,/);
    const placeholder = root.querySelector('.md-image');
    expect(placeholder?.textContent).toBe('[image: Relative image]');
    expect(placeholder?.getAttribute('title')).toBe('docs/pixel.png');
  });

  it('emits fenced code with a language for the async highlighter, or uses the sync one', () => {
    const plain = parse(renderer.render('```ts\nlet a = 1;\n```\n', { path: 'x.md' }).html);
    const pre = plain.querySelector('pre.md-code');
    expect(pre?.getAttribute('data-lang')).toBe('ts');
    expect(pre?.getAttribute('data-md-code')).toBe('0');
    expect(pre?.querySelector('code')?.innerHTML).toBe('let a = 1;\n');

    const result = renderer.render('```ts\nlet a = 1;\n```\n', {
      path: 'x.md',
      highlight: (lang, code) => `<span class="mtk1">${lang}:${code.trim()}</span>`,
    });
    expect(result.pendingCode).toEqual([]);
    expect(parse(result.html).querySelector('code')?.innerHTML).toBe(
      '<span class="mtk1">ts:let a = 1;</span>',
    );
  });

  it('marks links to targets a click found missing', () => {
    const result = renderer.render('[gone](docs/missing.md) [ok](docs/target.ts)', {
      path: 'x.md',
      brokenTargets: new Set(['docs/missing.md']),
    });
    const root = parse(result.html);
    expect(root.querySelector('[data-md-broken]')?.getAttribute('title')).toBe(
      'Not found: docs/missing.md',
    );
    expect(root.querySelectorAll('[data-md-broken]')).toHaveLength(1);
  });

  it('never throws on hostile nesting and marks a failure if the parser did', () => {
    const deep = `${'> '.repeat(500)}text\n${'- '.repeat(500)}item\n${'['.repeat(2000)}x${']'.repeat(2000)}`;
    const result = renderer.render(deep, { path: 'x.md' });
    expect(typeof result.html).toBe('string');
    expect(result.failed).toBe(false);
  });

  it('renders a long document within budget', () => {
    const text = fixture('long.md');
    const started = performance.now();
    const result = renderer.render(text, { path: 'long.md' });
    const elapsed = performance.now() - started;
    console.info(`long.md (${text.length} chars) rendered in ${elapsed.toFixed(1)} ms`);
    expect(result.html.length).toBeGreaterThan(100_000);
    // Generous bound for CI; the report records the measured value.
    expect(elapsed).toBeLessThan(2000);
  });
});
