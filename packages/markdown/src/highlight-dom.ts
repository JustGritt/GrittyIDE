import type { PendingCode } from './render';
import type { Sanitizer } from './sanitize';

export type AsyncHighlighter = (lang: string, code: string) => Promise<string | null>;

export interface HighlightLimits {
  readonly maxBlocks: number;
  readonly maxBytes: number;
}

export const DEFAULT_HIGHLIGHT_LIMITS: HighlightLimits = { maxBlocks: 100, maxBytes: 200_000 };

/**
 * Fill the fenced blocks a render left escaped. Each result is sanitized and
 * written into the block found in the *current* root by its index, so a
 * result arriving after a re-render is dropped (the signal is aborted) or
 * lands in the block that still exists. Resolves with the number of blocks
 * colorized.
 */
export async function applyHighlights(
  root: ParentNode,
  pending: readonly PendingCode[],
  highlight: AsyncHighlighter,
  sanitizer: Sanitizer,
  signal: AbortSignal,
  limits: HighlightLimits = DEFAULT_HIGHLIGHT_LIMITS,
): Promise<number> {
  let applied = 0;
  let bytes = 0;
  for (const block of pending.slice(0, limits.maxBlocks)) {
    bytes += block.code.length;
    if (bytes > limits.maxBytes) break;
    let html: string | null;
    try {
      html = await highlight(block.lang, block.code);
    } catch {
      continue;
    }
    if (signal.aborted) return applied;
    if (html === null) continue;
    const target = root.querySelector(`[data-md-code="${block.index}"] > code`);
    if (!target) continue;
    target.innerHTML = sanitizer.sanitize(html);
    applied += 1;
  }
  return applied;
}
