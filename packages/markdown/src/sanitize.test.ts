import type { WindowLike } from 'dompurify';
import { describe, expect, it } from 'vitest';
import { createSanitizer } from './sanitize';
import { fixture, parse, testRenderer } from './testing/fixtures';

const sanitizer = createSanitizer(window);
const renderer = testRenderer();

const UNSAFE_URL = /^(?:javascript|vbscript|file|data:text|data:application)/i;

describe('sanitizer', () => {
  it('throws instead of silently passing input through without a DOM', () => {
    expect(() => createSanitizer({} as WindowLike)).toThrow(/not supported/);
  });

  it('renders the XSS fixture without any script, handler, unsafe URL or forbidden element', () => {
    const result = renderer.render(fixture('xss.md'), { path: 'xss.md' });
    const html = result.html;
    expect(html.toLowerCase()).not.toContain('<script');
    expect(html).not.toMatch(
      /<(?:iframe|object|embed|style|svg|math|form|button|base|meta|link)\b/i,
    );
    const root = parse(html);
    for (const element of root.querySelectorAll('*')) {
      for (const attr of element.attributes) {
        expect(attr.name.startsWith('on'), `${element.tagName} ${attr.name}`).toBe(false);
        expect(attr.name, `${element.tagName}`).not.toBe('style');
        expect(attr.name).not.toBe('target');
        if (attr.name === 'href' || attr.name === 'src') {
          expect(attr.value, `${element.tagName} ${attr.name}=${attr.value}`).not.toMatch(
            UNSAFE_URL,
          );
        }
      }
    }
    expect(root.querySelector('#root')).toBeNull();
    expect(root.querySelectorAll('input')).toHaveLength(0);
    expect(root.querySelector('img[src^="data:image/svg"]')).toBeNull();
    // Benign content survived the two layers.
    expect(root.querySelector('strong')?.textContent).toBe('bold');
    expect(root.querySelector('a[data-md-kind="external"]')?.getAttribute('href')).toBe(
      'https://example.com',
    );
    expect(root.querySelector('a[data-md-kind="document"]')?.getAttribute('data-md-path')).toBe(
      'docs/target.ts',
    );
    expect(root.querySelector('table')).not.toBeNull();
    expect(root.textContent).toContain("alert('raw html script')");
    expect(root.querySelector('code')?.textContent).toBe('<script>');
  });

  it('applies the allowlist and hooks to arbitrary HTML', () => {
    const clean = (html: string) => sanitizer.sanitize(html);
    expect(clean('<img src="data:image/png;base64,AAAA" alt="a">')).toBe(
      '<img src="data:image/png;base64,AAAA" alt="a">',
    );
    expect(clean('<img src="data:image/svg+xml;base64,AAAA">')).toBe('<img>');
    expect(clean('<img src="https://example.com/x.png">')).toBe('<img>');
    expect(clean('<img src="docs/x.png">')).toBe('<img>');
    expect(clean('<a href="https://example.com" target="_blank">x</a>')).toBe(
      '<a href="https://example.com">x</a>',
    );
    expect(clean('<a href="mailto:x@y">x</a>')).toBe('<a>x</a>');
    expect(clean('<a href="javascript:alert(1)">x</a>')).toBe('<a>x</a>');
    expect(clean('<a href="docs/x.ts">x</a>')).toBe('<a href="docs/x.ts">x</a>');
    expect(clean('<a href="#md-x">x</a>')).toBe('<a href="#md-x">x</a>');
    expect(clean('<input type="text">')).toBe('');
    expect(clean('<input type="checkbox">')).toBe('');
    expect(clean('<input type="checkbox" disabled="" checked="" class="c">')).toBe(
      '<input type="checkbox" disabled="" checked="" class="c">',
    );
    expect(clean('<h2 id="root">x</h2>')).toBe('<h2>x</h2>');
    expect(clean('<h2 id="md-root">x</h2>')).toBe('<h2 id="md-root">x</h2>');
    expect(clean('<td align="right" style="color:red">1</td>')).toBe('1');
    expect(
      clean('<table><tbody><tr><td align="right">1</td><td align="up">2</td></tr></tbody></table>'),
    ).toBe('<table><tbody><tr><td align="right">1</td><td>2</td></tr></tbody></table>');
    expect(clean('<ol start="5"><li>x</li></ol>')).toBe('<ol start="5"><li>x</li></ol>');
    expect(clean('<span onclick="x()" data-md-kind="document">x</span>')).toBe(
      '<span data-md-kind="document">x</span>',
    );
    expect(clean('<style>*{}</style><p>p</p>')).toBe('<p>p</p>');
    expect(clean('<pre><code><span class="mtk1">kw</span><br></code></pre>')).toBe(
      '<pre><code><span class="mtk1">kw</span><br></code></pre>',
    );
  });
});
