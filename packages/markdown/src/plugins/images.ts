import type { MarkdownIt } from '../markdown-it-types';
import { inlineText } from './heading-ids';

const INLINE_IMAGE = /^data:image\//i;

/**
 * The WebView cannot read repository files, so only inline (`data:image/*`)
 * images render; anything else becomes a placeholder that names the path.
 */
export function imagePlaceholders(md: MarkdownIt): void {
  const fallback = md.renderer.rules.image;
  md.renderer.rules.image = (tokens, index, options, env, self) => {
    const token = tokens[index];
    if (!token) return '';
    const src = String(token.attrGet('src') ?? '');
    if (INLINE_IMAGE.test(src) && fallback) return fallback(tokens, index, options, env, self);
    const alt = inlineText(token.children);
    const escapeHtml = md.utils.escapeHtml;
    const label = alt !== '' ? alt : src;
    return `<span class="md-image" title="${escapeHtml(src)}">[image: ${escapeHtml(label)}]</span>`;
  };
}
