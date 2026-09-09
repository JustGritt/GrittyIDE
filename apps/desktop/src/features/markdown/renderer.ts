// Browser-only singletons: DOMPurify bound to the app window, the renderer,
// and the editor's highlighter behind a cache. Never import from node tests.
import { createMarkdownRenderer, createSanitizer } from '@gritty/markdown';
import { createCachedHighlighter } from './highlighter';

export const previewSanitizer = createSanitizer(window);
export const previewRenderer = createMarkdownRenderer(previewSanitizer);
// Imported dynamically, not at module scope: this module is on the startup
// path through `markdown-preview`, and Monaco is deliberately not (MONACO-DEFER).
export const previewHighlighter = createCachedHighlighter(async () => {
  const { createMonacoHighlighter } = await import('@gritty/editor/monaco');
  return createMonacoHighlighter();
});
