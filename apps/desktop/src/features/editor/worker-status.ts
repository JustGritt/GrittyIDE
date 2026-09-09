import { WorkerStatus } from '@gritty/editor';

/**
 * Why Monaco's workers did not start, if they did not; shown by the editor pane.
 *
 * It lives apart from `monaco-setup` deliberately. The pane subscribes to this
 * on mount, and `monaco-setup` is only ever reached through a dynamic import
 * (MONACO-DEFER), so re-exporting it from there would put Monaco back on the
 * startup path through the pane's own import.
 */
export const workerStatus = new WorkerStatus({ violationTarget: document });
