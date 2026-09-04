import { type CoreError, toCoreError } from '@gritty/protocol';
import { useCallback, useEffect, useRef, useState } from 'react';
import { core } from '../../core/client';
import type { useDocuments } from '../documents/use-documents';
import { type DropPlan, describePlan, planDrop } from './drop-policy';

export type DropNotice =
  /** Files were dropped that no open workspace contains. */
  | { kind: 'offerFolder'; path: string; files: string[] }
  /** `workspace.open` was refused because documents have unsaved changes. */
  | { kind: 'confirmReplace'; path: string }
  /** Something was done, or partly done. */
  | { kind: 'done'; message: string }
  | { kind: 'error'; error: CoreError };

/**
 * Files and directories dropped onto the window (`PathsDropped`).
 *
 * The core classifies, `planDrop` decides, and this performs the plan with the
 * commands that already exist. Two rules it keeps: a drop never replaces the
 * workspace without asking when documents are dirty, and a file that Gritty
 * cannot address is reported rather than dropped on the floor.
 */
export function useFileDrop(documents: ReturnType<typeof useDocuments>) {
  const [hovering, setHovering] = useState(false);
  const [notice, setNotice] = useState<DropNotice | null>(null);
  /** The folder open refused with `Conflict`, replayable with force. */
  const refused = useRef<string | null>(null);

  const openFolder = useCallback(async (path: string, force = false) => {
    try {
      await core.workspace.open(path, force ? { force } : undefined);
      refused.current = null;
      setNotice(null);
    } catch (cause: unknown) {
      const error = toCoreError(cause);
      if (error.code === 'Conflict') {
        refused.current = path;
        setNotice({ kind: 'confirmReplace', path });
      } else {
        setNotice({ kind: 'error', error });
      }
    }
  }, []);

  const perform = useCallback(
    (plan: DropPlan) => {
      for (const file of plan.openFiles) void documents.openFile(file.workspaceId, file.path);
      if (plan.offerFolder) {
        setNotice({ kind: 'offerFolder', ...plan.offerFolder });
      } else if (plan.openWorkspace) {
        void openFolder(plan.openWorkspace);
      } else {
        const message = describePlan(plan);
        setNotice(message ? { kind: 'done', message } : null);
      }
    },
    [documents, openFolder],
  );

  useEffect(() => {
    const unsubscribeHover = core.subscribe('DropHoverChanged', (event) =>
      setHovering(event.payload.active),
    );
    const unsubscribeDrop = core.subscribe('PathsDropped', (event) => {
      setHovering(false);
      perform(planDrop(event.payload.items));
    });
    return () => {
      unsubscribeHover();
      unsubscribeDrop();
    };
  }, [perform]);

  const confirmReplace = useCallback(() => {
    const path = refused.current;
    if (path) void openFolder(path, true);
  }, [openFolder]);

  const acceptFolder = useCallback((path: string) => void openFolder(path), [openFolder]);

  const dismiss = useCallback(() => setNotice(null), []);

  return { hovering, notice, confirmReplace, acceptFolder, dismiss };
}
