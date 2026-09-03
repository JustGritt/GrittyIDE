/** @vitest-environment jsdom */
import { describe, expect, it } from 'vitest';
import { linkActionFromEvent } from './preview-links';

function root(html: string): HTMLElement {
  const element = document.createElement('div');
  element.innerHTML = html;
  return element;
}

describe('linkActionFromEvent', () => {
  it('reads document links with optional line, column and fragment from the nearest link', () => {
    const el = root(
      '<p><a data-md-kind="document" data-md-path="docs/x.ts" data-md-line="3" data-md-column="5" data-md-fragment="sec"><strong>bold</strong></a></p>',
    );
    const strong = el.querySelector('strong');
    expect(linkActionFromEvent(strong, el)).toEqual({
      kind: 'document',
      path: 'docs/x.ts',
      line: 3,
      column: 5,
      fragment: 'sec',
      broken: false,
    });
  });

  it('reads anchors, external links, unsupported spans and broken marks', () => {
    const el = root(
      '<a data-md-kind="anchor" data-md-anchor="md-x">a</a>' +
        '<a data-md-kind="external" data-md-url="https://e.com">b</a>' +
        '<span data-md-kind="unsupported" title="no mailto">c</span>' +
        '<a data-md-kind="document" data-md-path="gone.md" data-md-broken="true">d</a>' +
        '<em>plain</em>',
    );
    const [a, b, c, d] = ['a', 'b', 'c', 'd'].map((text) =>
      [...el.querySelectorAll('a, span')].find((n) => n.textContent === text),
    );
    expect(linkActionFromEvent(a ?? null, el)).toEqual({
      kind: 'anchor',
      id: 'md-x',
      broken: false,
    });
    expect(linkActionFromEvent(b ?? null, el)).toEqual({
      kind: 'external',
      url: 'https://e.com',
      broken: false,
    });
    expect(linkActionFromEvent(c ?? null, el)).toEqual({
      kind: 'unsupported',
      reason: 'no mailto',
      broken: false,
    });
    expect(linkActionFromEvent(d ?? null, el)).toMatchObject({
      kind: 'document',
      path: 'gone.md',
      broken: true,
    });
    expect(linkActionFromEvent(el.querySelector('em'), el)).toBeNull();
    expect(linkActionFromEvent(null, el)).toBeNull();
  });

  it('ignores malformed line numbers and never walks above the root', () => {
    const outer = root(
      '<div id="outside" data-md-kind="external" data-md-url="https://x"><div id="inner"><a data-md-kind="document" data-md-path="p" data-md-line="abc">z</a></div></div>',
    );
    const inner = outer.querySelector<HTMLElement>('#inner');
    if (!inner) throw new Error('inner');
    expect(linkActionFromEvent(inner.querySelector('a'), inner)).toEqual({
      kind: 'document',
      path: 'p',
      broken: false,
    });
    expect(linkActionFromEvent(inner, inner)).toBeNull();
  });
});
