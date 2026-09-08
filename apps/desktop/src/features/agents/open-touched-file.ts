import type { WorkspaceId, WorkspacePath } from '@gritty/protocol';
import type { OpenFileOptions, OpenFileResult } from '../documents/use-documents';

/**
 * The slice of `DocumentsController` the rail needs, and nothing else.
 *
 * A port rather than the whole controller so the order these two calls happen
 * in is testable without React, and so the rail cannot quietly grow a second
 * way into the document store. `DocumentsController` satisfies it structurally.
 */
export interface TouchedFileDocuments {
  /** `documents.status`: re-check the disk for every open document. */
  checkStatus(): Promise<void>;
  openFile(
    workspaceId: WorkspaceId,
    path: string,
    options?: OpenFileOptions,
  ): Promise<OpenFileResult>;
}

/**
 * Open a file the agent touched, in the editor beside the agent that touched it
 * (T232 §11, §12).
 *
 * **`documents.status` first.** External modification is checked on demand
 * today — T070's watcher does not exist — so a file the agent rewrote a second
 * ago would otherwise open from a buffer the core last read before the write,
 * or reuse an already-open tab whose contents are older still. Checking first
 * turns that into the ordinary external-change flow the document contract
 * already defines: the tab's `externalModificationState` moves, the banner the
 * editor already draws appears, and hard rule 7 holds because nothing is
 * resolved silently — a dirty buffer over a changed file surfaces the conflict
 * exactly as it does everywhere else.
 *
 * The reuse in §12 is `openFile`'s own: it looks the path up with
 * `findTabByPath` and activates the tab that is already open rather than asking
 * the core for a second one. Duplicating that here would be a second answer to
 * a question the document store already owns.
 *
 * `quiet` because the refusal belongs in the console's error envelope: nothing was
 * opened, so there is no editor pane for the pane's banner to sit above.
 */
export async function openTouchedFile(
  documents: TouchedFileDocuments,
  workspaceId: WorkspaceId,
  path: WorkspacePath,
): Promise<OpenFileResult> {
  await documents.checkStatus();
  return documents.openFile(workspaceId, path, { quiet: true });
}
