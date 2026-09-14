import { createMonacoModelFactory, installMonacoEnvironment } from '@gritty/editor/monaco';
// Vite bundles each worker as its own chunk, served from the app origin
// (`worker-src 'self'` in the CSP). The editor package takes them as factories.
import EditorWorker from 'monaco-editor/editor/editor.worker?worker';
import CssWorker from 'monaco-editor/language/css/css.worker?worker';
import HtmlWorker from 'monaco-editor/language/html/html.worker?worker';
import JsonWorker from 'monaco-editor/language/json/json.worker?worker';
import TsWorker from 'monaco-editor/language/typescript/ts.worker?worker';
import { workerStatus } from './worker-status';

// This module pulls Monaco, so nothing may import it statically: it is reached
// only through `import('./monaco-setup')` in `use-editor-binding`. See
// `docs/tasks/completed/MONACO-DEFER-startup-import.md`. `workerStatus` is exported from
// `./worker-status` for the same reason.

export type MonacoModelFactory = ReturnType<typeof createMonacoModelFactory>;
export type MonacoModel = ReturnType<MonacoModelFactory['create']>;

let factory: MonacoModelFactory | null = null;

/** Install the worker environment once and return the (single) model factory. */
export function setupMonaco(): MonacoModelFactory {
  installMonacoEnvironment(
    {
      editor: () => new EditorWorker(),
      json: () => new JsonWorker(),
      css: () => new CssWorker(),
      html: () => new HtmlWorker(),
      typescript: () => new TsWorker(),
    },
    workerStatus,
  );
  if (factory === null) factory = createMonacoModelFactory();
  return factory;
}
