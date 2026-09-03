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
  readonly typescript: () => Worker;
}

let installed = false;

/**
 * Point Monaco at the bundled workers and record their failures. Idempotent.
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
