import type { MarkdownIt, StateCore, Token } from '../markdown-it-types';
import { describeTarget, type LinkTarget, resolveLink } from '../resolve-link';
import { anchorId } from '../slug';

export interface LinkEnv {
  readonly path: string;
  readonly brokenTargets?: ReadonlySet<string>;
}

/**
 * Classify every link (Markdown links, reference links and linkified URLs
 * alike) before sanitization and record the result in `data-md-*`
 * attributes; `href` is rewritten to a form the sanitizer keeps. The preview
 * intercepts every click and acts on the data attributes, never on `href`.
 */
export function repositoryLinks(md: MarkdownIt): void {
  md.core.ruler.push('gritty_links', (state: StateCore) => {
    const env = state.env as Partial<LinkEnv>;
    const fromPath = env.path ?? '';
    for (const block of state.tokens) {
      if (block.type !== 'inline' || !block.children) continue;
      const children = block.children;
      for (let index = 0; index < children.length; index += 1) {
        const token = children[index];
        if (token?.type !== 'link_open') continue;
        const target = resolveLink(String(token.attrGet('href') ?? ''), fromPath);
        annotate(token, target, env.brokenTargets);
        if (target.kind === 'unsupported') neutralize(children, index);
      }
    }
  });
}

function annotate(token: Token, target: LinkTarget, broken: ReadonlySet<string> | undefined): void {
  token.attrSet('data-md-kind', target.kind);
  token.attrJoin('class', `md-link md-link--${target.kind}`);
  token.attrSet('title', describeTarget(target));
  switch (target.kind) {
    case 'document':
      token.attrSet('href', target.path);
      token.attrSet('data-md-path', target.path);
      if (target.line !== undefined) token.attrSet('data-md-line', String(target.line));
      if (target.column !== undefined) token.attrSet('data-md-column', String(target.column));
      if (target.fragment !== undefined) token.attrSet('data-md-fragment', target.fragment);
      if (broken?.has(target.path)) {
        token.attrSet('data-md-broken', 'true');
        token.attrSet('title', `Not found: ${target.path}`);
      }
      return;
    case 'anchor': {
      const id = anchorId(target.id);
      token.attrSet('href', `#${id}`);
      token.attrSet('data-md-anchor', id);
      return;
    }
    case 'external':
      token.attrSet('href', target.url);
      token.attrSet('data-md-url', target.url);
      return;
    case 'unsupported':
      return;
  }
}

/** An unsupported link renders as an inert span with the reason as tooltip. */
function neutralize(children: Token[], openIndex: number): void {
  const open = children[openIndex];
  if (!open) return;
  open.tag = 'span';
  const href = open.attrIndex('href');
  if (href !== -1) open.attrs?.splice(href, 1);
  for (let index = openIndex + 1; index < children.length; index += 1) {
    const token = children[index];
    if (token?.type === 'link_close') {
      token.tag = 'span';
      return;
    }
  }
}
