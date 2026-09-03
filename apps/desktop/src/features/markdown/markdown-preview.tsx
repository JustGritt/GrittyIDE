import { applyHighlights } from '@gritty/markdown';
import type { DocumentId } from '@gritty/protocol';
import { useCallback, useEffect, useRef, useState } from 'react';
import type { EditorBinding } from '../editor/use-editor-binding';
import { linkActionFromEvent, type PreviewLinkAction } from './preview-links';
import { previewHighlighter, previewRenderer, previewSanitizer } from './renderer';
import { collectAnchors, type LineAnchor, previewScrollTop } from './scroll-sync';

/** Auto-rendering pauses above this size or render time; a Refresh button stays. */
export const PAUSE_TEXT_LENGTH = 512 * 1024;
export const PAUSE_RENDER_MS = 200;
/** Above this the preview refuses to render at all. */
export const REFUSE_TEXT_LENGTH = 4 * 1024 * 1024;
const MIN_DEBOUNCE_MS = 50;
const MAX_DEBOUNCE_MS = 500;
/** Scroll sync stays quiet this long after an anchor jump so it does not snap back. */
const ANCHOR_SYNC_SUSPEND_MS = 1000;

interface PreviewStatus {
  readonly paused: boolean;
  readonly refused: boolean;
  /** The source changed while paused. */
  readonly stale: boolean;
  readonly failed: boolean;
  readonly lastRenderMs: number | null;
  readonly textLength: number;
}

const INITIAL_STATUS: PreviewStatus = {
  paused: false,
  refused: false,
  stale: false,
  failed: false,
  lastRenderMs: null,
  textLength: 0,
};

export interface MarkdownPreviewProps {
  readonly documentId: DocumentId;
  readonly path: string;
  readonly largeFileMode: boolean;
  /** The store's text, used until the model exists. */
  readonly fallbackText: string | null;
  readonly editor: EditorBinding;
  readonly brokenTargets: ReadonlySet<string>;
  readonly onLink: (action: NonNullable<PreviewLinkAction>) => void;
  readonly onNotice: (message: string) => void;
}

function formatSize(length: number): string {
  return length >= 1024 * 1024
    ? `${(length / (1024 * 1024)).toFixed(1)} MiB`
    : `${Math.round(length / 1024)} KiB`;
}

/**
 * The rendered document. An imperative island like the Monaco container: the
 * sanitized HTML goes into `innerHTML` (never React state), links are handled
 * by delegated listeners (every click, middle click and context menu on a
 * link is prevented), the editor's scroll drives the preview's.
 */
export function MarkdownPreview({
  documentId,
  path,
  largeFileMode,
  fallbackText,
  editor,
  brokenTargets,
  onLink,
  onNotice,
}: MarkdownPreviewProps) {
  const rootRef = useRef<HTMLDivElement>(null);
  const [status, setStatus] = useState<PreviewStatus>(INITIAL_STATUS);
  const lastRenderMs = useRef(0);
  const highlightAbort = useRef<AbortController | null>(null);
  const anchors = useRef<LineAnchor[]>([]);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const frame = useRef<number | null>(null);
  const suspendSyncUntil = useRef(0);
  const onLinkRef = useRef(onLink);
  onLinkRef.current = onLink;
  const onNoticeRef = useRef(onNotice);
  onNoticeRef.current = onNotice;

  const renderNow = useCallback(
    (force: boolean) => {
      const root = rootRef.current;
      if (!root) return;
      const text = editor.readText(documentId) ?? fallbackText ?? '';
      if (text.length > REFUSE_TEXT_LENGTH) {
        setStatus((s) => ({ ...s, refused: true, paused: true, textLength: text.length }));
        return;
      }
      const tooExpensive =
        largeFileMode || text.length > PAUSE_TEXT_LENGTH || lastRenderMs.current > PAUSE_RENDER_MS;
      if (!force && tooExpensive) {
        setStatus((s) =>
          s.paused && s.stale ? s : { ...s, paused: true, stale: true, textLength: text.length },
        );
        return;
      }
      const started = performance.now();
      const result = previewRenderer.render(text, {
        path,
        highlight: previewHighlighter.sync,
        brokenTargets,
      });
      highlightAbort.current?.abort();
      const controller = new AbortController();
      highlightAbort.current = controller;
      root.innerHTML = result.html;
      anchors.current = collectAnchors(root);
      const elapsed = performance.now() - started;
      lastRenderMs.current = elapsed;
      setStatus({
        paused: false,
        refused: false,
        stale: false,
        failed: result.failed,
        lastRenderMs: elapsed,
        textLength: text.length,
      });
      void applyHighlights(
        root,
        result.pendingCode,
        previewHighlighter.async,
        previewSanitizer,
        controller.signal,
      ).then(() => {
        if (!controller.signal.aborted) anchors.current = collectAnchors(root);
      });
    },
    [editor, documentId, path, largeFileMode, fallbackText, brokenTargets],
  );

  // Render on every model change, debounced by what the last render cost.
  useEffect(() => {
    const schedule = () => {
      if (timer.current !== null) clearTimeout(timer.current);
      const delay = Math.min(MAX_DEBOUNCE_MS, Math.max(MIN_DEBOUNCE_MS, 3 * lastRenderMs.current));
      timer.current = setTimeout(() => {
        timer.current = null;
        renderNow(false);
      }, delay);
    };
    const unsubscribe = editor.subscribeContent(documentId, schedule);
    renderNow(false);
    return () => {
      unsubscribe();
      if (timer.current !== null) clearTimeout(timer.current);
      highlightAbort.current?.abort();
    };
  }, [editor, documentId, renderNow]);

  // Source → preview scroll sync.
  useEffect(
    () =>
      editor.subscribeScroll((state) => {
        const root = rootRef.current;
        if (!root || performance.now() < suspendSyncUntil.current) return;
        if (frame.current !== null) cancelAnimationFrame(frame.current);
        frame.current = requestAnimationFrame(() => {
          frame.current = null;
          root.scrollTop = previewScrollTop(
            anchors.current,
            state.topLine,
            state.atBottom,
            root.scrollHeight,
            root.clientHeight,
          );
        });
      }),
    [editor],
  );

  // Links: nothing in the preview ever navigates the WebView.
  useEffect(() => {
    const root = rootRef.current;
    if (!root) return;
    const onPointer = (event: MouseEvent) => {
      const action = linkActionFromEvent(event.target, root);
      if (!action) return;
      event.preventDefault();
      if (event.type !== 'click' || event.button !== 0) return;
      if (action.kind === 'anchor') {
        const target = root.querySelector<HTMLElement>(`#${CSS.escape(action.id)}`);
        if (!target) {
          onNoticeRef.current(`Anchor not found in this document: #${action.id}`);
          return;
        }
        suspendSyncUntil.current = performance.now() + ANCHOR_SYNC_SUSPEND_MS;
        root.scrollTop = target.offsetTop;
        return;
      }
      onLinkRef.current(action);
    };
    root.addEventListener('click', onPointer);
    root.addEventListener('auxclick', onPointer);
    root.addEventListener('contextmenu', onPointer);
    return () => {
      root.removeEventListener('click', onPointer);
      root.removeEventListener('auxclick', onPointer);
      root.removeEventListener('contextmenu', onPointer);
    };
  }, []);

  let banner: string | null = null;
  if (status.refused) {
    banner = `Preview disabled: ${formatSize(status.textLength)} of Markdown is above the ${formatSize(REFUSE_TEXT_LENGTH)} limit.`;
  } else if (status.paused) {
    const why = largeFileMode
      ? 'the document is in large-file mode'
      : status.textLength > PAUSE_TEXT_LENGTH
        ? `${formatSize(status.textLength)} of Markdown`
        : `the last render took ${Math.round(lastRenderMs.current)} ms`;
    banner = `Preview paused (${why}); it no longer follows your edits.${status.stale ? ' The source changed since the last render.' : ''}`;
  } else if (status.failed) {
    banner = 'The Markdown parser failed on this document; showing the source as text.';
  }

  return (
    <section className="md-preview" aria-label={`Preview of ${path}`}>
      {banner && (
        <div className="md-preview__banner" role="status">
          <span>{banner}</span>
          {!status.refused && (
            <button type="button" onClick={() => renderNow(true)}>
              Refresh
            </button>
          )}
        </div>
      )}
      <div ref={rootRef} className="md-preview__content md-content" />
      {status.lastRenderMs !== null && (
        <div className="md-preview__meta">rendered in {status.lastRenderMs.toFixed(0)} ms</div>
      )}
    </section>
  );
}
