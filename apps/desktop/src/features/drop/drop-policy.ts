import type { DroppedPath, WorkspaceId } from '@gritty/protocol';

/**
 * What one drop gesture should do. Pure: the core classified the paths, this
 * decides, and `use-file-drop` performs it. Keeping it separate is what lets
 * the rules be tested without a window, a workspace or a filesystem.
 */
export type DropPlan = {
  /** Absolute path of a directory to open as the workspace. */
  openWorkspace?: string;
  /** Files to open as tabs, already known to be inside the open workspace. */
  openFiles: { workspaceId: WorkspaceId; path: string }[];
  /**
   * Files that no open workspace contains: Gritty keys documents by root plus
   * relative path, so there is nothing to open until this folder is a
   * workspace. The UI offers it rather than swapping the workspace silently.
   */
  offerFolder?: { path: string; files: string[] };
  /** Anything not acted on, phrased for the user. */
  skipped: string[];
};

/**
 * Opening a hundred tabs from one gesture is never what someone meant. Beyond
 * this the rest is reported as skipped instead.
 */
export const MAX_FILES_PER_DROP = 20;

export function planDrop(items: DroppedPath[]): DropPlan {
  const plan: DropPlan = { openFiles: [], skipped: [] };

  const directories = items.filter((item) => item.kind === 'DIRECTORY');
  const inside = items.filter((item) => item.kind === 'FILE_IN_WORKSPACE');
  const outside = items.filter((item) => item.kind === 'FILE_OUTSIDE_WORKSPACE');
  const unresolvable = items.filter((item) => item.kind === 'UNRESOLVABLE');
  const unsupported = items.filter((item) => item.kind === 'UNSUPPORTED');

  for (const item of unresolvable) plan.skipped.push(`${item.name} no longer exists`);
  for (const item of unsupported) plan.skipped.push(`${item.name} is not a file or folder`);

  // A folder is an unambiguous "work on this", so it wins over anything else
  // in the same gesture: the files' workspace-relative identity would change
  // under them the moment the root changed.
  const [firstDirectory, ...otherDirectories] = directories;
  if (firstDirectory) {
    plan.openWorkspace = firstDirectory.path;
    for (const item of otherDirectories) {
      plan.skipped.push(`${item.name} (one folder at a time)`);
    }
    for (const item of [...inside, ...outside]) {
      plan.skipped.push(`${item.name} (dropped with a folder)`);
    }
    return plan;
  }

  if (inside.length > 0) {
    for (const item of inside.slice(0, MAX_FILES_PER_DROP)) {
      if (item.workspaceId && item.workspacePath !== undefined) {
        plan.openFiles.push({ workspaceId: item.workspaceId, path: item.workspacePath });
      }
    }
    const over = inside.length - MAX_FILES_PER_DROP;
    if (over > 0) plan.skipped.push(`${over} more file${over === 1 ? '' : 's'}`);
  }

  if (outside.length > 0) {
    // One offer, not one per file: the useful question is which folder to open.
    const folder = commonParent(outside);
    if (folder) {
      plan.offerFolder = { path: folder, files: outside.map((item) => item.name) };
    } else {
      for (const item of outside) plan.skipped.push(`${item.name} is outside the workspace`);
    }
  }

  return plan;
}

/** The parent all the files share, or the first one's when they differ. */
function commonParent(items: DroppedPath[]): string | undefined {
  const parents = items.map((item) => item.parent).filter((p): p is string => Boolean(p));
  if (parents.length === 0) return undefined;
  const first = parents[0];
  return parents.every((parent) => parent === first) ? first : first;
}

/** One sentence describing what a plan did, or undefined when it did nothing. */
export function describePlan(plan: DropPlan): string | undefined {
  const parts: string[] = [];
  if (plan.openWorkspace) parts.push(`opened ${basename(plan.openWorkspace)}`);
  if (plan.openFiles.length > 0) {
    parts.push(`opened ${plan.openFiles.length} file${plan.openFiles.length === 1 ? '' : 's'}`);
  }
  if (plan.skipped.length > 0) parts.push(`skipped ${plan.skipped.join(', ')}`);
  return parts.length > 0 ? parts.join('; ') : undefined;
}

function basename(path: string): string {
  const parts = path.split(/[/\\]/).filter(Boolean);
  return parts[parts.length - 1] ?? path;
}
