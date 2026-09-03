import type { MarkdownIt, StateCore } from '../markdown-it-types';

const SELF_CLOSING = new Set(['fence', 'code_block', 'hr']);

/** `data-line` / `data-line-end` (0-based source lines) on every rendered block. */
export function sourceLines(md: MarkdownIt): void {
  md.core.ruler.push('gritty_source_lines', (state: StateCore) => {
    for (const token of state.tokens) {
      if (!token.map || token.hidden) continue;
      if (token.nesting !== 1 && !SELF_CLOSING.has(token.type)) continue;
      token.attrSet('data-line', String(token.map[0]));
      token.attrSet('data-line-end', String(token.map[1]));
    }
  });
}
