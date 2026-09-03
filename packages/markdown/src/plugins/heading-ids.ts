import type { MarkdownIt, StateCore, Token } from '../markdown-it-types';
import { createSlugger } from '../slug';

export interface RenderedHeading {
  readonly id: string;
  readonly level: number;
  readonly text: string;
  /** 0-based source line. */
  readonly line: number;
}

/** Plain text of an inline token tree (heading titles, image alt text). */
export function inlineText(children: readonly Token[] | null): string {
  if (!children) return '';
  let text = '';
  for (const child of children) {
    if (child.type === 'text' || child.type === 'code_inline') text += child.content;
    else if (child.type === 'softbreak' || child.type === 'hardbreak') text += ' ';
    else if (child.children) text += inlineText(child.children);
  }
  return text;
}

/** GitHub-style ids on headings; the list of headings goes to `env.headings`. */
export function headingIds(md: MarkdownIt): void {
  md.core.ruler.push('gritty_heading_ids', (state: StateCore) => {
    const slug = createSlugger();
    const headings: RenderedHeading[] = [];
    const tokens = state.tokens;
    for (let index = 0; index < tokens.length; index += 1) {
      const token = tokens[index];
      if (token?.type !== 'heading_open') continue;
      const inline = tokens[index + 1];
      const text = inline?.type === 'inline' ? inlineText(inline.children) : '';
      const id = slug(text);
      token.attrSet('id', id);
      headings.push({ id, level: Number(token.tag.slice(1)), text, line: token.map?.[0] ?? 0 });
    }
    (state.env as { headings?: RenderedHeading[] }).headings = headings;
  });
}
