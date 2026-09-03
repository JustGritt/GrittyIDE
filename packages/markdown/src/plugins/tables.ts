import type { MarkdownIt, StateCore } from '../markdown-it-types';

const ALIGN = /^text-align:\s*(left|center|right)\s*;?$/i;

/** markdown-it expresses column alignment as `style`; the sanitizer forbids `style`. */
export function tableAlignment(md: MarkdownIt): void {
  md.core.ruler.push('gritty_table_alignment', (state: StateCore) => {
    for (const token of state.tokens) {
      if (token.type !== 'th_open' && token.type !== 'td_open') continue;
      const style = token.attrGet('style');
      if (style === null) continue;
      const match = ALIGN.exec(String(style));
      const index = token.attrIndex('style');
      if (index !== -1) token.attrs?.splice(index, 1);
      const align = match?.[1];
      if (align !== undefined) token.attrSet('align', align.toLowerCase());
    }
  });
}
