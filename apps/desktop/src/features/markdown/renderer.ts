// Browser-only singletons: DOMPurify bound to the app window, the renderer,
// and the editor's highlighter behind a cache. Never import from node tests.
import { createMonacoHighlighter } from '@gritty/editor/monaco';
import { createMarkdownRenderer, createSanitizer } from '@gritty/markdown';
import { createCachedHighlighter } from './highlighter';

export const previewSanitizer = createSanitizer(window);
export const previewRenderer = createMarkdownRenderer(previewSanitizer);
export const previewHighlighter = createCachedHighlighter(createMonacoHighlighter());
