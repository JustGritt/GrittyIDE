import markdownit from 'markdown-it';
import type { MarkdownIt } from './markdown-it-types';
import { headingIds, type RenderedHeading } from './plugins/heading-ids';
import { imagePlaceholders } from './plugins/images';
import { repositoryLinks } from './plugins/links';
import { sourceLines } from './plugins/source-lines';
import { tableAlignment } from './plugins/tables';
import { taskLists } from './plugins/task-lists';
import type { Sanitizer } from './sanitize';

/** Synchronous highlighter (a cache in front of Monaco); `null` = not available yet. */
export type SyncHighlighter = (lang: string, code: string) => string | null;

export interface RenderOptions {
  /** Workspace path of the document; relative links resolve against its directory. */
  readonly path: string;
  readonly highlight?: SyncHighlighter;
  /** Document paths a previous click found missing; their links render as broken. */
  readonly brokenTargets?: ReadonlySet<string>;
}

export interface PendingCode {
  readonly index: number;
  readonly lang: string;
  readonly code: string;
}

export interface RenderResult {
  /** Sanitized HTML, safe for `innerHTML`. */
  readonly html: string;
  readonly headings: readonly RenderedHeading[];
  /** Fenced blocks the synchronous highlighter could not colorize yet. */
  readonly pendingCode: readonly PendingCode[];
  /** The parser threw; `html` is the escaped source. Should never happen. */
  readonly failed: boolean;
}

export interface MarkdownRenderer {
  render(text: string, options: RenderOptions): RenderResult;
}

// A type alias (not an interface) so it satisfies markdown-it's indexable `Env`.
type RenderEnv = {
  path: string;
  brokenTargets: ReadonlySet<string> | undefined;
  highlight: SyncHighlighter | undefined;
  headings: RenderedHeading[];
  pendingCode: PendingCode[];
};

/** Nesting deeper than this is rendered flat; bounds recursion on hostile input. */
const MAX_NESTING = 100;

export function createMarkdownInstance(): MarkdownIt {
  const md = markdownit({
    html: false,
    xhtmlOut: false,
    breaks: false,
    linkify: true,
    typographer: false,
    maxNesting: MAX_NESTING,
  });
  // `README.md` in prose is not a Moldovan domain: only `scheme://` autolinks.
  md.linkify.set({ fuzzyLink: false, fuzzyEmail: false, fuzzyIP: false });
  md.use(sourceLines);
  md.use(headingIds);
  md.use(taskLists);
  md.use(tableAlignment);
  md.use(imagePlaceholders);
  md.use(repositoryLinks);

  md.renderer.rules.fence = (tokens, index, _options, env, self) => {
    const token = tokens[index];
    if (!token) return '';
    const renderEnv = env as unknown as RenderEnv;
    const lang = token.info.trim().split(/\s+/)[0] ?? '';
    const code = token.content;
    const attrs: Array<[string, string]> = [['class', 'md-code']];
    if (lang !== '') attrs.push(['data-lang', lang]);
    if (token.map) {
      attrs.push(['data-line', String(token.map[0])], ['data-line-end', String(token.map[1])]);
    }
    let body = lang !== '' && renderEnv.highlight ? renderEnv.highlight(lang, code) : null;
    if (body === null) {
      if (lang !== '') {
        const pending = renderEnv.pendingCode.length;
        renderEnv.pendingCode.push({ index: pending, lang, code });
        attrs.push(['data-md-code', String(pending)]);
      }
      body = md.utils.escapeHtml(code);
    }
    const rendered = attrs
      .map(([name, value]) => ` ${name}="${md.utils.escapeHtml(value)}"`)
      .join('');
    void self;
    return `<pre${rendered}><code>${body}</code></pre>\n`;
  };
  return md;
}

export function createMarkdownRenderer(sanitizer: Sanitizer): MarkdownRenderer {
  const md = createMarkdownInstance();
  return {
    render(text, options) {
      const env: RenderEnv = {
        path: options.path,
        brokenTargets: options.brokenTargets,
        highlight: options.highlight,
        headings: [],
        pendingCode: [],
      };
      let html: string;
      let failed = false;
      try {
        html = md.render(text, env);
      } catch {
        failed = true;
        html = `<pre class="md-fallback">${md.utils.escapeHtml(text)}</pre>`;
      }
      return {
        html: sanitizer.sanitize(html),
        headings: env.headings,
        pendingCode: env.pendingCode,
        failed,
      };
    },
  };
}
