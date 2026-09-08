import type { WorkspaceId } from '@gritty/protocol';
import { useState } from 'react';
import { ChangesView, type GitView } from './changes-view';
import { useGit } from './use-git';

/**
 * The **Changes** tab (T100-U): `git.status` as a list of changed paths, and
 * the selected file's `git.diff` as structured hunks underneath it.
 *
 * Read-only by construction, because the wire is: the four `git.*` commands are
 * all reads. Nothing here stages, commits, checks out or fetches, and there is
 * nothing to call that would — a write surface needs ADR-029 accepted first.
 *
 * The container half — it owns the controller and nothing else. What it draws
 * is `changes-view.tsx`, which is where the reasoning about each arm lives.
 */
export function ChangesPanel({ workspaceId }: { workspaceId: WorkspaceId | null }) {
  const git = useGit(workspaceId);
  // Deliberately not persisted, and deliberately not in the reducer: which half
  // of the panel you were last looking at is a view preference, and the reducer
  // owns reads. `showHistory` is what starts the walk, so switching to History
  // is also what asks for it — the first time only.
  const [view, setView] = useState<GitView>('changes');
  const { showHistory } = git;
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
      onViewChange={(next) => {
        setView(next);
        if (next === 'history') showHistory();
      }}
      onLoadMoreHistory={git.loadMoreHistory}
      onCancelHistory={git.cancelHistory}
    />
  );
}
