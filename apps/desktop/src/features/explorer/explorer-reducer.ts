import type {
  CoreErrorShape,
  DirEntry,
  DirectoryPage,
  EventEnvelope,
  OperationId,
  Workspace,
  WorkspaceClosedReason,
  WorkspaceId,
} from '@gritty/protocol';

/**
 * The explorer tree as the UI knows it: one node per workspace-relative path,
 * loaded one level at a time by `workspace.list`, kept current by
 * `Workspace*` and `Operation*` events. Pure, so it is unit-tested without
 * React or Tauri.
 */
export interface ExplorerNode {
  /** Workspace-relative path; `''` is the root. */
  readonly path: string;
  /** `null` for the root. */
  readonly entry: DirEntry | null;
  readonly expanded: boolean;
  /** A `workspace.list` call, or the operation it handed over to, is in flight. */
  readonly loading: boolean;
  readonly pending: PendingListing | null;
  /** Child paths in server order; `null` until the first page arrived. */
  readonly children: readonly string[] | null;
  /** Entry count of the directory across all pages, once known. */
  readonly total: number | null;
  readonly truncated: boolean;
  readonly cursor: string | null;
  readonly error: CoreErrorShape | null;
}

export interface PendingListing {
  readonly operationId: OperationId;
  /** The page continues the existing children (a "load more"). */
  readonly append: boolean;
}

export interface ClosedNotice {
  readonly root: string;
  readonly reason: WorkspaceClosedReason;
}

export type ListingOutcome =
  | { kind: 'page'; page: DirectoryPage }
  | { kind: 'failed'; error: CoreErrorShape }
  | { kind: 'cancelled' };

export interface ExplorerState {
  readonly workspace: Workspace | null;
  readonly nodes: Readonly<Record<string, ExplorerNode>>;
  readonly showHidden: boolean;
  readonly notice: ClosedNotice | null;
  /**
   * Outcomes of `workspace.list` operations whose `pending` response has not
   * been seen yet: the command response and the completion event are separate
   * IPC messages, and their order is not guaranteed.
   */
  readonly unmatched: Readonly<Record<string, ListingOutcome>>;
}

export const initialExplorerState: ExplorerState = {
  workspace: null,
  nodes: {},
  showHidden: false,
  notice: null,
  unmatched: {},
};

/** Operation kind the core uses for a listing that outlived its budget. */
export const LIST_OPERATION_KIND = 'workspace.list';
const UNMATCHED_LIMIT = 20;

export type ExplorerAction =
  | { type: 'workspace'; workspace: Workspace | null }
  | { type: 'toggle'; path: string }
  | { type: 'listStarted'; path: string }
  | { type: 'listPending'; path: string; operationId: OperationId; append: boolean }
  | { type: 'pageLoaded'; path: string; page: DirectoryPage; append: boolean }
  | { type: 'listFailed'; path: string; error: CoreErrorShape }
  | { type: 'setShowHidden'; showHidden: boolean }
  | { type: 'dismissNotice' }
  | { type: 'event'; event: EventEnvelope };

export function childPath(parent: string, name: string): string {
  return parent === '' ? name : `${parent}/${name}`;
}

/** Directories, and symlinks that resolve to a directory inside the root. */
export function isExpandable(entry: DirEntry | null): boolean {
  if (entry === null) return true;
  if (entry.kind === 'dir') return true;
  return (
    entry.kind === 'symlink' &&
    entry.symlink?.resolvesInsideRoot === true &&
    entry.symlink.targetKind === 'dir'
  );
}

function makeNode(path: string, entry: DirEntry | null, expanded = false): ExplorerNode {
  return {
    path,
    entry,
    expanded,
    loading: false,
    pending: null,
    children: null,
    total: null,
    truncated: false,
    cursor: null,
    error: null,
  };
}

function withNode(
  state: ExplorerState,
  path: string,
  update: (node: ExplorerNode) => ExplorerNode,
): ExplorerState {
  const existing = state.nodes[path];
  if (!existing) return state;
  const updated = update(existing);
  if (updated === existing) return state;
  return { ...state, nodes: { ...state.nodes, [path]: updated } };
}

function openWorkspace(state: ExplorerState, workspace: Workspace): ExplorerState {
  if (state.workspace?.id === workspace.id) return state;
  return {
    ...state,
    workspace,
    nodes: { '': makeNode('', null, true) },
    notice: null,
    unmatched: {},
  };
}

function closeWorkspace(
  state: ExplorerState,
  workspaceId: WorkspaceId | null,
  notice: ClosedNotice | null,
): ExplorerState {
  if (state.workspace === null) return state;
  if (workspaceId !== null && state.workspace.id !== workspaceId) return state;
  return { ...state, workspace: null, nodes: {}, notice, unmatched: {} };
}

function dropSubtree(nodes: Record<string, ExplorerNode>, path: string): void {
  const node = nodes[path];
  if (!node) return;
  for (const child of node.children ?? []) dropSubtree(nodes, child);
  delete nodes[path];
}

function applyPage(
  state: ExplorerState,
  path: string,
  page: DirectoryPage,
  append: boolean,
): ExplorerState {
  const parent = state.nodes[path];
  if (!parent) return state;
  const nodes: Record<string, ExplorerNode> = { ...state.nodes };
  if (!append) {
    for (const child of parent.children ?? []) dropSubtree(nodes, child);
  }
  const added: string[] = [];
  for (const entry of page.entries) {
    const child = childPath(path, entry.name);
    nodes[child] = makeNode(child, entry);
    added.push(child);
  }
  nodes[path] = {
    ...parent,
    loading: false,
    pending: null,
    error: null,
    children: append ? [...(parent.children ?? []), ...added] : added,
    total: page.total,
    truncated: page.truncated,
    cursor: page.cursor ?? null,
  };
  return { ...state, nodes };
}

function applyOutcome(state: ExplorerState, path: string, outcome: ListingOutcome): ExplorerState {
  const append = state.nodes[path]?.pending?.append ?? false;
  switch (outcome.kind) {
    case 'page':
      return applyPage(state, path, outcome.page, append);
    case 'failed':
      return withNode(state, path, (node) => ({
        ...node,
        loading: false,
        pending: null,
        error: outcome.error,
      }));
    case 'cancelled':
      return withNode(state, path, (node) => ({ ...node, loading: false, pending: null }));
  }
}

function settle(state: ExplorerState, operationId: OperationId, outcome: ListingOutcome) {
  const path = Object.keys(state.nodes).find(
    (candidate) => state.nodes[candidate]?.pending?.operationId === operationId,
  );
  if (path === undefined) {
    const kept = Object.entries(state.unmatched).slice(-(UNMATCHED_LIMIT - 1));
    return { ...state, unmatched: { ...Object.fromEntries(kept), [operationId]: outcome } };
  }
  return applyOutcome(state, path, outcome);
}

function onEvent(state: ExplorerState, event: EventEnvelope): ExplorerState {
  switch (event.name) {
    case 'WorkspaceOpened':
      return openWorkspace(state, event.payload.workspace);
    case 'WorkspaceClosed':
      return closeWorkspace(state, event.payload.workspaceId, {
        root: event.payload.root,
        reason: event.payload.reason,
      });
    case 'OperationCompleted': {
      if (event.payload.operation.kind !== LIST_OPERATION_KIND) return state;
      const page = event.payload.result as DirectoryPage | undefined;
      const outcome: ListingOutcome = page
        ? { kind: 'page', page }
        : {
            kind: 'failed',
            error: {
              code: 'Internal',
              message: 'listing completed without a page',
              retryable: false,
            },
          };
      return settle(state, event.payload.operation.id, outcome);
    }
    case 'OperationFailed':
      if (event.payload.operation.kind !== LIST_OPERATION_KIND) return state;
      return settle(state, event.payload.operation.id, {
        kind: 'failed',
        error: event.payload.error,
      });
    case 'OperationCancelled':
      if (event.payload.operation.kind !== LIST_OPERATION_KIND) return state;
      return settle(state, event.payload.operation.id, { kind: 'cancelled' });
    default:
      return state;
  }
}

export function explorerReducer(state: ExplorerState, action: ExplorerAction): ExplorerState {
  switch (action.type) {
    case 'workspace':
      return action.workspace === null
        ? closeWorkspace(state, null, null)
        : openWorkspace(state, action.workspace);
    case 'toggle':
      return withNode(state, action.path, (node) => ({ ...node, expanded: !node.expanded }));
    case 'listStarted':
      return withNode(state, action.path, (node) => ({ ...node, loading: true, error: null }));
    case 'listPending': {
      const pending = withNode(state, action.path, (node) => ({
        ...node,
        loading: true,
        error: null,
        pending: { operationId: action.operationId, append: action.append },
      }));
      const outcome = pending.unmatched[action.operationId];
      if (!outcome) return pending;
      const { [action.operationId]: _matched, ...unmatched } = pending.unmatched;
      return applyOutcome({ ...pending, unmatched }, action.path, outcome);
    }
    case 'pageLoaded':
      return applyPage(state, action.path, action.page, action.append);
    case 'listFailed':
      return withNode(state, action.path, (node) => ({
        ...node,
        loading: false,
        pending: null,
        error: action.error,
      }));
    case 'setShowHidden':
      return state.showHidden === action.showHidden
        ? state
        : { ...state, showHidden: action.showHidden };
    case 'dismissNotice':
      return state.notice === null ? state : { ...state, notice: null };
    case 'event':
      return onEvent(state, action.event);
  }
}

export type ExplorerRow =
  | { kind: 'entry'; node: ExplorerNode; entry: DirEntry; depth: number }
  | { kind: 'more'; node: ExplorerNode; depth: number; remaining: number }
  | { kind: 'status'; node: ExplorerNode; depth: number };

/** The rows to render, depth-first over expanded nodes; hidden entries only when asked. */
export function selectRows(state: ExplorerState): ExplorerRow[] {
  const rows: ExplorerRow[] = [];
  const root = state.nodes[''];
  if (!root) return rows;
  const visit = (node: ExplorerNode, depth: number): void => {
    const initial = node.children === null;
    if (initial && (node.loading || node.error)) rows.push({ kind: 'status', node, depth });
    for (const path of node.children ?? []) {
      const child = state.nodes[path];
      if (!child?.entry) continue;
      if (!state.showHidden && child.entry.hidden) continue;
      rows.push({ kind: 'entry', node: child, entry: child.entry, depth });
      if (child.expanded) visit(child, depth + 1);
    }
    if (!initial && (node.loading || node.error)) rows.push({ kind: 'status', node, depth });
    if (node.truncated && !node.loading && !node.error) {
      const remaining = Math.max(0, (node.total ?? 0) - (node.children?.length ?? 0));
      rows.push({ kind: 'more', node, depth, remaining });
    }
  };
  visit(root, 0);
  return rows;
}

/** Hidden entries that `showHidden: false` is currently filtering out. */
export function countHidden(state: ExplorerState): number {
  let hidden = 0;
  for (const node of Object.values(state.nodes)) {
    if (node.entry?.hidden) hidden += 1;
  }
  return hidden;
}
