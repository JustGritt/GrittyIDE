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
export {
  type WorkerFailure,
  type WorkerLike,
  WorkerStatus,
  type WorkerStatusListener,
  type WorkerStatusOptions,
  type WorkerStatusSnapshot,
} from './worker-status';
