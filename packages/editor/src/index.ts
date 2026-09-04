// Pure, node-testable part of the editor package. Everything that touches
// Monaco at runtime lives under `@gritty/editor/monaco`.
export { bindModel, type ModelBinding } from './binding';
export {
  type DocumentModelEntry,
  DocumentModelRegistry,
  type EnsureOptions,
  type ModelFactory,
  type ModelKey,
} from './document-models';
export {
  DEFAULT_EDITOR_PREFERENCES,
  type DocumentFlags,
  EDITOR_PREFERENCES_VERSION,
  type EditorOptions,
  type EditorPreferences,
  editorOptionsFor,
  LARGE_FILE_MAX_TOKENIZATION_LINE_LENGTH,
  MAX_FONT_SIZE,
  MAX_TOKENIZATION_LINE_LENGTH,
  MIN_FONT_SIZE,
  parseEditorPreferences,
  readOnlyReason,
  type SerializedEditorPreferences,
  serializeEditorPreferences,
  TAB_SIZES,
} from './editor-options';
export {
  detectEol,
  dominantEol,
  EOL_SEQUENCE,
  type Eol,
  type EolTarget,
  eolLabel,
  isEditableEol,
  normalizeEol,
} from './eol';
export { PLAINTEXT_LANGUAGE, resolveLanguageId } from './languages';
export { applyMinimalEdit, computeMinimalEdit, type MinimalEdit } from './minimal-edit';
export type {
  EolSequence,
  ModelEdit,
  ModelPosition,
  ModelRange,
  TextModelLike,
} from './text-model';
export {
  type EditorViewState,
  INITIAL_VIEW_STATE,
  type TextPos,
  viewStateFrom,
} from './view-state';
export {
  type WorkerFailure,
  type WorkerLike,
  WorkerStatus,
  type WorkerStatusListener,
  type WorkerStatusOptions,
  type WorkerStatusSnapshot,
} from './worker-status';
