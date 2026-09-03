import * as monaco from 'monaco-editor';
import type { WorkerStatus } from '../worker-status';

/**
 * Worker constructors, injected by the app: Vite's `?worker` imports live in
 * `apps/desktop` (the only project with Vite's module types), so this package
 * stays free of bundler syntax.
 */
export interface WorkerFactories {
  readonly editor: () => Worker;
  readonly json: () => Worker;
  readonly css: () => Worker;
  readonly html: () => Worker;
  readonly typescript: () => Worker;
}

let installed = false;

/**
 * Point Monaco at the bundled workers and record their failures. Idempotent.
 * Monaco 0.56 would otherwise build each language worker itself
 * (`new Worker(new URL('x.worker.js', import.meta.url))`); routing every
 * construction through `getWorker` keeps the choice explicit and observable.
 * TypeScript/JavaScript semantic validation is off: the worker sees one file
 * at a time, so every import would be "cannot find module"; syntax diagnostics
 * stay on and still prove the worker runs.
 */
export function installMonacoEnvironment(factories: WorkerFactories, status: WorkerStatus): void {
  if (installed) return;
  installed = true;
  globalThis.MonacoEnvironment = {
    getWorker(_workerId: string, label: string): Worker {
      switch (label) {
        case 'json':
          return status.track(label, factories.json);
        case 'css':
        case 'scss':
        case 'less':
          return status.track(label, factories.css);
        case 'html':
        case 'handlebars':
        case 'razor':
          return status.track(label, factories.html);
        case 'typescript':
        case 'javascript':
          return status.track(label, factories.typescript);
        default:
          return status.track(label, factories.editor);
      }
    },
  };
  const diagnostics = { noSemanticValidation: true, noSyntaxValidation: false };
  monaco.typescript.typescriptDefaults.setDiagnosticsOptions(diagnostics);
  monaco.typescript.javascriptDefaults.setDiagnosticsOptions(diagnostics);
}
