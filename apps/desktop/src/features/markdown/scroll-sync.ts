/** A rendered block and where it starts in the preview (offset from the scroll container's top). */
export interface LineAnchor {
  /** 0-based source line (`data-line`). */
  readonly line: number;
  readonly top: number;
}

/**
 * Where the preview should scroll so the block containing the editor's first
 * visible line sits at the top: linear interpolation between the nearest
 * anchors, the end of the preview when the editor shows its end.
 */
export function previewScrollTop(
  anchors: readonly LineAnchor[],
  topLine: number,
  atBottom: boolean,
  scrollHeight: number,
  clientHeight: number,
): number {
  if (atBottom) return Math.max(0, scrollHeight - clientHeight);
  const line = topLine - 1; // editor lines are 1-based, anchors 0-based
  let before: LineAnchor | null = null;
  let after: LineAnchor | null = null;
  for (const anchor of anchors) {
    if (anchor.line <= line) before = anchor;
    else {
      after = anchor;
      break;
    }
  }
  if (!before) return 0;
  if (!after || after.line === before.line) return Math.max(0, before.top);
  const ratio = (line - before.line) / (after.line - before.line);
  return Math.max(0, before.top + (after.top - before.top) * ratio);
}

/** Anchors from the rendered preview, one per source line, in document order. */
export function collectAnchors(root: HTMLElement): LineAnchor[] {
  const anchors: LineAnchor[] = [];
  let lastLine = -1;
  for (const element of root.querySelectorAll<HTMLElement>('[data-line]')) {
    const line = Number(element.dataset.line);
    if (!Number.isFinite(line) || line <= lastLine) continue;
    anchors.push({ line, top: element.offsetTop });
    lastLine = line;
  }
  return anchors;
}
