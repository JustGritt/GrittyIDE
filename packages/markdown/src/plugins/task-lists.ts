import type { MarkdownIt, StateCore } from '../markdown-it-types';

const MARKER = /^\[([ xX])\] /;

/**
 * GFM task lists: `- [ ] text` / `- [x] text` render a disabled checkbox. The
 * checkbox is its own token with its own renderer rule; no raw HTML is ever
 * injected into the token stream.
 */
export function taskLists(md: MarkdownIt): void {
  md.core.ruler.push('gritty_task_lists', (state: StateCore) => {
    const tokens = state.tokens;
    for (let index = 2; index < tokens.length; index += 1) {
      const inline = tokens[index];
      const paragraph = tokens[index - 1];
      const item = tokens[index - 2];
      if (
        inline?.type !== 'inline' ||
        paragraph?.type !== 'paragraph_open' ||
        item?.type !== 'list_item_open'
      ) {
        continue;
      }
      const first = inline.children?.[0];
      if (first?.type !== 'text') continue;
      const match = MARKER.exec(first.content);
      if (!match) continue;
      first.content = first.content.slice(match[0].length);
      const checkbox = new state.Token('task_checkbox', 'input', 0);
      checkbox.attrSet('type', 'checkbox');
      checkbox.attrSet('disabled', '');
      checkbox.attrSet('class', 'md-task__box');
      if (match[1] !== ' ') checkbox.attrSet('checked', '');
      inline.children?.unshift(checkbox);
      item.attrJoin('class', 'md-task');
    }
  });

  md.renderer.rules.task_checkbox = (tokens, index, _options, _env, self) => {
    const token = tokens[index];
    return token ? `<input${self.renderAttrs(token)}> ` : '';
  };
}
