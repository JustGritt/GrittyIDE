import type { WorkspaceId, WorkspacePath } from '@gritty/protocol';
import { useEffect, useState } from 'react';
import { ChangesView, type GitView } from './changes-view';
import { type DiscardPlan, discardPlan } from './git-model';
import { useGit } from './use-git';

/**
 * The **Changes** tab (T100-U): `git.status` as a list of changed paths, and
 * the selected file's `git.diff` as structured hunks underneath it — and, since
 * T102-U, the verbs: stage, unstage, discard and commit.
 *
 * The reads are in-process `gix`; the writes are the user's own `git`
 * (ADR-029), so a commit made here carries their identity, their signing key
 * and their hooks.
 *
 * The container half — it owns the controller and nothing else. What it draws
 * is `changes-view.tsx`, which is where the reasoning about each arm lives.
 */
export function ChangesPanel({ workspaceId }: { workspaceId: WorkspaceId | null }) {
  const git = useGit(workspaceId);
  // Deliberately not persisted, and deliberately not in the reducer: which half
  // of the panel you were last looking at is a view preference, and the reducer
  // owns reads.
  const [view, setView] = useState<GitView>('changes');
  // The discard waiting on its confirmation. Here rather than in the reducer
  // because nothing has been sent: it is a question on screen, not a write.
  const [discarding, setDiscarding] = useState<DiscardPlan | null>(null);

  // `showHistory` walks the history only while it is `idle`, so this is what
  // reads it the first time the History half is shown — and again after a
  // commit, which puts a history already read back to `idle`.
  const { showHistory } = git;
  useEffect(() => {
    if (view === 'history') showHistory();
  }, [view, showHistory]);

  const status = git.state.status;
  const { discard } = git;
  return (
    <ChangesView
      state={git.state}
      hasWorkspace={workspaceId !== null}
      onRefresh={git.refresh}
      onLoadMore={git.loadMore}
      onSelect={git.select}
      onReloadDiff={git.reloadDiff}
      onCancelStatus={git.cancelStatus}
      onCancelDiff={git.cancelDiff}
      view={view}
      onViewChange={setView}
      onLoadMoreHistory={git.loadMoreHistory}
      onCancelHistory={git.cancelHistory}
      writes={{
        canDiscardAndRetry: git.canDiscardAndRetry,
        discarding,
        onStage: git.stage,
        onUnstage: git.unstage,
        onRequestDiscard: (paths: readonly WorkspacePath[]) => {
          if (status.kind !== 'ready') return;
          setDiscarding(discardPlan(paths, status.value.entries));
        },
        onConfirmDiscard: () => {
          if (discarding !== null) discard(discarding.paths);
          setDiscarding(null);
        },
        onCancelDiscard: () => setDiscarding(null),
        onCommit: git.commit,
        onDraftChange: git.changeDraft,
        onDiscardAndRetry: git.discardAndRetry,
        onRetryWrite: git.retryWrite,
        onDismissWrite: git.dismissWrite,
        onCancelWrite: git.cancelWrite,
      }}
    />
  );
}
