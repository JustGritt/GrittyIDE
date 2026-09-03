import type { LinkTarget } from '@gritty/markdown';

/** What a click in the preview asks for, read from the renderer's `data-md-*` attributes. */
export type PreviewLinkAction = (LinkTarget & { readonly broken: boolean }) | null;

/**
 * The link (if any) an event hit, walking up from the event target to the
 * preview root. Every link, including unsupported ones rendered as spans,
 * carries `data-md-kind`, so the caller can prevent the default for all of them.
 */
export function linkActionFromEvent(
  target: EventTarget | null,
  root: HTMLElement,
): PreviewLinkAction {
  let node = target instanceof Node ? target : null;
  while (node && node !== root) {
    if (node instanceof HTMLElement && node.dataset.mdKind !== undefined) {
      return actionFromElement(node);
    }
    node = node.parentNode;
  }
  return null;
}

export function actionFromElement(element: HTMLElement): PreviewLinkAction {
  const data = element.dataset;
  const broken = data.mdBroken === 'true';
  switch (data.mdKind) {
    case 'document': {
      if (!data.mdPath) return null;
      const action: {
        -readonly [K in keyof Extract<LinkTarget, { kind: 'document' }>]: Extract<
          LinkTarget,
          { kind: 'document' }
        >[K];
      } & { broken: boolean } = { kind: 'document', path: data.mdPath, broken };
      const line = Number(data.mdLine);
      const column = Number(data.mdColumn);
      if (data.mdLine !== undefined && Number.isInteger(line) && line > 0) action.line = line;
      if (data.mdColumn !== undefined && Number.isInteger(column) && column > 0)
        action.column = column;
      if (data.mdFragment) action.fragment = data.mdFragment;
      return action;
    }
    case 'anchor':
      return data.mdAnchor ? { kind: 'anchor', id: data.mdAnchor, broken } : null;
    case 'external':
      return data.mdUrl ? { kind: 'external', url: data.mdUrl, broken } : null;
    case 'unsupported':
      return { kind: 'unsupported', reason: element.title || 'unsupported link', broken };
    default:
      return null;
  }
}
