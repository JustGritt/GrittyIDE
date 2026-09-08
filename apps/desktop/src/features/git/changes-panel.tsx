import type { WorkspaceId } from '@gritty/protocol';
import { ChangesView } from './changes-view';
import { useGit } from './use-git';

/**
 * The **Changes** tab (T100-U): `git.status` as a list of changed paths, and
 * the selected file's `git.diff` as structured hunks underneath it.
 *
 * Read-only by construction, because the wire is: `git.status` and `git.diff`
 * are the only two git commands there are. Nothing here stages, commits,
 * checks out or fetches, and there is nothing to call that would.
 *
 * The container half — it owns the controller and nothing else. What it draws
 * is `changes-view.tsx`, which is where the reasoning about each arm lives.
 */
export function ChangesPanel({ workspaceId }: { workspaceId: WorkspaceId | null }) {
  const git = useGit(workspaceId);
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
    />
  );
}
