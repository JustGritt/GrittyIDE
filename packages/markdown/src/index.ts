export {
  type AsyncHighlighter,
  applyHighlights,
  DEFAULT_HIGHLIGHT_LIMITS,
  type HighlightLimits,
} from './highlight-dom';
export { isInstructionFile } from './instruction-files';
export { inlineText, type RenderedHeading } from './plugins/heading-ids';
export type { LinkEnv } from './plugins/links';
export {
  createMarkdownInstance,
  createMarkdownRenderer,
  type MarkdownRenderer,
  type PendingCode,
  type RenderOptions,
  type RenderResult,
  type SyncHighlighter,
} from './render';
export {
  describeTarget,
  dirname,
  joinWorkspacePath,
  type LinkTarget,
  resolveLink,
} from './resolve-link';
export {
  ALLOWED_ATTR,
  ALLOWED_TAGS,
  ALLOWED_URI_REGEXP,
  createSanitizer,
  ID_PREFIX,
  MAX_IMAGE_DATA_URI_LENGTH,
  type Sanitizer,
} from './sanitize';
export { anchorId, createSlugger, slugify } from './slug';
