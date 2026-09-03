// Browser-only entry: imports monaco-editor at runtime. Never import this from
// node tests; the pure pieces live in `@gritty/editor`.
export {
  type CursorPosition,
  createEditorHost,
  type EditorHost,
  type EditorTheme,
  type ScrollState,
  setEditorTheme,
} from './editor-host';
export { createMonacoModelFactory, modelUri } from './model-factory';
export { installMonacoEnvironment, type WorkerFactories } from './monaco-environment';
export { watchColorScheme } from './theme';
