import type { CodeHighlighter } from '@gritty/editor/monaco';
import type { AsyncHighlighter, SyncHighlighter } from '@gritty/markdown';

export interface PreviewHighlighter {
  /** Cache hits only: what a synchronous render can colorize without waiting. */
  readonly sync: SyncHighlighter;
  /** Resolve the fence language and colorize; results feed the cache. */
  readonly async: AsyncHighlighter;
}

/**
 * A small LRU in front of the editor's highlighter, keyed by language and
 * code, so re-rendering while typing keeps already colorized blocks and
 * only new or changed blocks go through the asynchronous pass.
 *
 * `load` is called at most once, on the first fence that misses the cache.
 * Monaco is not on the startup path (MONACO-DEFER), and a preview of a file
 * with no code fences must not be what pulls it in.
 */
export function createCachedHighlighter(
  load: () => Promise<CodeHighlighter>,
  capacity = 500,
): PreviewHighlighter {
  let base: Promise<CodeHighlighter> | null = null;
  const highlighter = () => {
    base ??= load();
    return base;
  };
  const cache = new Map<string, string>();
  const key = (lang: string, code: string) => `${lang.toLowerCase()}\0${code}`;
  const remember = (k: string, html: string) => {
    cache.delete(k);
    cache.set(k, html);
    if (cache.size > capacity) {
      const oldest = cache.keys().next().value;
      if (oldest !== undefined) cache.delete(oldest);
    }
  };
  return {
    sync(lang, code) {
      const k = key(lang, code);
      const hit = cache.get(k);
      if (hit === undefined) return null;
      remember(k, hit);
      return hit;
    },
    async async(lang, code) {
      const k = key(lang, code);
      const hit = cache.get(k);
      if (hit !== undefined) return hit;
      const monaco = await highlighter();
      const languageId = monaco.resolveLanguage(lang);
      if (languageId === null) return null;
      const html = await monaco.colorize(languageId, code);
      remember(k, html);
      return html;
    },
  };
}
