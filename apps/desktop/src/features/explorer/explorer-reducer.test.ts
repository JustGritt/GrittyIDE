import type {
  DirEntry,
  DirectoryPage,
  EventEnvelope,
  Operation,
  OperationId,
  Workspace,
  WorkspaceId,
} from '@gritty/protocol';
import { describe, expect, it } from 'vitest';
import {
  childPath,
  type ExplorerState,
  explorerReducer,
  initialExplorerState,
  isExpandable,
  selectRows,
} from './explorer-reducer';

const WS: Workspace = { id: 'ws-1' as WorkspaceId, root: '/ws', name: 'ws', openedAt: 1 };
const OP_ID = 'op-1' as OperationId;

function entry(name: string, overrides: Partial<DirEntry> = {}): DirEntry {
  return { name, kind: 'file', hidden: name.startsWith('.'), ignoredHint: false, ...overrides };
}

function page(
  path: string,
  entries: DirEntry[],
  extra: Partial<DirectoryPage> = {},
): DirectoryPage {
  return { path, entries, total: entries.length, truncated: false, ...extra };
}

function opened(): ExplorerState {
  return explorerReducer(initialExplorerState, { type: 'workspace', workspace: WS });
}

function listOperation(status: Operation['status']): Operation {
  return {
    id: OP_ID,
    kind: 'workspace.list',
    actor: { type: 'USER' },
    workspaceId: WS.id,
    status,
    createdAt: 1,
    cancellationRequested: false,
    correlationId: 'corr' as Operation['correlationId'],
  };
}

function event(
  payload: EventEnvelope['name'] extends never ? never : Omit<EventEnvelope, 'seq' | 'actor'>,
): EventEnvelope {
  return { seq: 1, actor: { type: 'USER' }, ...payload } as EventEnvelope;
}

const ROOT_ENTRIES = [
  entry('src', { kind: 'dir' }),
  entry('node_modules', { kind: 'dir', ignoredHint: true }),
  entry('.env'),
  entry('README.md', { size: 12 }),
];

describe('workspace lifecycle', () => {
  it('opening a workspace creates an expanded root and clears stale state', () => {
    const state = opened();
    expect(state.workspace).toEqual(WS);
    expect(state.nodes['']).toMatchObject({ path: '', expanded: true, children: null });
    const reopened = explorerReducer(state, { type: 'workspace', workspace: WS });
    expect(reopened).toBe(state);
  });

  it('closing through an event clears the tree and leaves a notice', () => {
    let state = opened();
    state = explorerReducer(state, {
      type: 'pageLoaded',
      path: '',
      page: page('', ROOT_ENTRIES),
      append: false,
    });
    const other = explorerReducer(state, {
      type: 'event',
      event: event({
        name: 'WorkspaceClosed',
        payload: { workspaceId: 'ws-2' as WorkspaceId, root: '/other', reason: 'USER' },
      }),
    });
    expect(other).toBe(state);
    const closed = explorerReducer(state, {
      type: 'event',
      event: event({
        name: 'WorkspaceClosed',
        payload: { workspaceId: WS.id, root: '/ws', reason: 'ROOT_MISSING' },
      }),
    });
    expect(closed.workspace).toBeNull();
    expect(closed.nodes).toEqual({});
    expect(closed.notice).toEqual({ root: '/ws', reason: 'ROOT_MISSING' });
    expect(explorerReducer(closed, { type: 'dismissNotice' }).notice).toBeNull();
  });

  it('a WorkspaceOpened event for a new workspace resets the tree', () => {
    const state = opened();
    const next: Workspace = { ...WS, id: 'ws-2' as WorkspaceId };
    const replaced = explorerReducer(state, {
      type: 'event',
      event: event({ name: 'WorkspaceOpened', payload: { workspace: next } }),
    });
    expect(replaced.workspace).toEqual(next);
    expect(Object.keys(replaced.nodes)).toEqual(['']);
  });
});

describe('pages and rows', () => {
  it('builds child nodes in server order and filters hidden entries until asked', () => {
    let state = opened();
    state = explorerReducer(state, { type: 'listStarted', path: '' });
    expect(selectRows(state)).toEqual([{ kind: 'status', node: state.nodes[''], depth: 0 }]);
    state = explorerReducer(state, {
      type: 'pageLoaded',
      path: '',
      page: page('', ROOT_ENTRIES),
      append: false,
    });
    expect(state.nodes['']?.children).toEqual(['src', 'node_modules', '.env', 'README.md']);
    expect(state.nodes['']?.loading).toBe(false);
    const visible = selectRows(state).map((row) =>
      row.kind === 'entry' ? row.entry.name : row.kind,
    );
    expect(visible).toEqual(['src', 'node_modules', 'README.md']);
    state = explorerReducer(state, { type: 'setShowHidden', showHidden: true });
    const all = selectRows(state).map((row) => (row.kind === 'entry' ? row.entry.name : row.kind));
    expect(all).toEqual(['src', 'node_modules', '.env', 'README.md']);
  });

  it('expands one level per toggle and nests rows by depth', () => {
    let state = opened();
    state = explorerReducer(state, {
      type: 'pageLoaded',
      path: '',
      page: page('', ROOT_ENTRIES),
      append: false,
    });
    state = explorerReducer(state, { type: 'toggle', path: 'src' });
    expect(state.nodes.src?.expanded).toBe(true);
    state = explorerReducer(state, { type: 'listStarted', path: 'src' });
    state = explorerReducer(state, {
      type: 'pageLoaded',
      path: 'src',
      page: page('src', [entry('lib', { kind: 'dir' }), entry('main.rs')]),
      append: false,
    });
    const rows = selectRows(state).map((row) =>
      row.kind === 'entry' ? `${row.depth}:${row.entry.name}` : `${row.depth}:${row.kind}`,
    );
    expect(rows).toEqual(['0:src', '1:lib', '1:main.rs', '0:node_modules', '0:README.md']);
    expect(childPath('src', 'lib')).toBe('src/lib');
    expect(state.nodes['src/lib']?.entry?.kind).toBe('dir');
    const collapsed = explorerReducer(state, { type: 'toggle', path: 'src' });
    expect(selectRows(collapsed)).toHaveLength(3);
  });

  it('reloading a directory keeps what survived it, expanded and loaded (T042)', () => {
    let state = opened();
    state = explorerReducer(state, {
      type: 'pageLoaded',
      path: '',
      page: page('', ROOT_ENTRIES),
      append: false,
    });
    state = explorerReducer(state, { type: 'toggle', path: 'src' });
    state = explorerReducer(state, {
      type: 'pageLoaded',
      path: 'src',
      page: page('src', [entry('old.rs')]),
      append: false,
    });
    expect(state.nodes.src?.expanded).toBe(true);

    // The kind of re-list every file operation now causes.
    state = explorerReducer(state, {
      type: 'pageLoaded',
      path: '',
      page: page('', [entry('src', { kind: 'dir' })]),
      append: false,
    });
    expect(state.nodes.src?.expanded, 'a survivor stays open').toBe(true);
    expect(state.nodes.src?.children, 'and keeps its loaded children').toEqual(['src/old.rs']);
    expect(state.nodes['src/old.rs']).toBeDefined();
  });

  it('a re-list drops selected rows that are no longer there (T042)', () => {
    let state = opened();
    state = explorerReducer(state, {
      type: 'pageLoaded',
      path: '',
      page: page('', [entry('a.ts'), entry('b.ts'), entry('c.ts')]),
      append: false,
    });
    state = explorerReducer(state, {
      type: 'selection',
      selection: { paths: ['a.ts', 'b.ts'], anchor: 'a.ts', focused: 'b.ts' },
    });

    // `b.ts` was renamed or deleted by someone; the re-list says so.
    state = explorerReducer(state, {
      type: 'pageLoaded',
      path: '',
      page: page('', [entry('a.ts'), entry('c.ts')]),
      append: false,
    });
    expect(state.selection.paths).toEqual(['a.ts']);
    expect(
      state.selection.focused,
      'the focus moves to a survivor so the keyboard still has somewhere to be',
    ).toBe('a.ts');
  });

  it('reloading still drops a subtree whose directory is gone', () => {
    let state = opened();
    state = explorerReducer(state, {
      type: 'pageLoaded',
      path: '',
      page: page('', ROOT_ENTRIES),
      append: false,
    });
    state = explorerReducer(state, {
      type: 'pageLoaded',
      path: 'src',
      page: page('src', [entry('old.rs')]),
      append: false,
    });
    state = explorerReducer(state, {
      type: 'pageLoaded',
      path: '',
      page: page('', [entry('README.md')]),
      append: false,
    });
    expect(state.nodes.src).toBeUndefined();
    expect(state.nodes['src/old.rs']).toBeUndefined();
  });

  it('truncated pages show a "more" row and appended pages extend the children', () => {
    let state = opened();
    const first = page('', [entry('a'), entry('b')], { total: 5, truncated: true, cursor: 'fb' });
    state = explorerReducer(state, { type: 'pageLoaded', path: '', page: first, append: false });
    const more = selectRows(state).at(-1);
    expect(more).toMatchObject({ kind: 'more', depth: 0, remaining: 3 });
    state = explorerReducer(state, { type: 'listStarted', path: '' });
    expect(selectRows(state).map((r) => r.kind)).toEqual(['entry', 'entry', 'status']);
    const second = page('', [entry('c'), entry('d'), entry('e')], { total: 5 });
    state = explorerReducer(state, { type: 'pageLoaded', path: '', page: second, append: true });
    expect(state.nodes['']?.children).toEqual(['a', 'b', 'c', 'd', 'e']);
    expect(state.nodes['']?.truncated).toBe(false);
    expect(selectRows(state).map((r) => r.kind)).toEqual([
      'entry',
      'entry',
      'entry',
      'entry',
      'entry',
    ]);
  });

  it('records a failure on the node and clears it on the next attempt', () => {
    let state = opened();
    const error = { code: 'NotFound', message: 'gone', retryable: false } as const;
    state = explorerReducer(state, { type: 'listFailed', path: '', error });
    expect(state.nodes['']?.error).toEqual(error);
    expect(selectRows(state)).toEqual([{ kind: 'status', node: state.nodes[''], depth: 0 }]);
    state = explorerReducer(state, { type: 'listStarted', path: '' });
    expect(state.nodes['']?.error).toBeNull();
    expect(state.nodes['']?.loading).toBe(true);
  });
});

describe('listings that became operations', () => {
  it('applies the page from OperationCompleted to the pending node', () => {
    let state = opened();
    state = explorerReducer(state, { type: 'listStarted', path: '' });
    state = explorerReducer(state, {
      type: 'listPending',
      path: '',
      operationId: OP_ID,
      append: false,
    });
    expect(state.nodes['']?.pending).toEqual({ operationId: OP_ID, append: false });
    state = explorerReducer(state, {
      type: 'event',
      event: event({
        name: 'OperationCompleted',
        payload: { operation: listOperation('COMPLETED'), result: page('', ROOT_ENTRIES) },
      }),
    });
    expect(state.nodes['']?.pending).toBeNull();
    expect(state.nodes['']?.children).toHaveLength(4);
  });

  it('keeps an outcome that arrives before the pending response and applies it then', () => {
    let state = opened();
    state = explorerReducer(state, { type: 'listStarted', path: '' });
    state = explorerReducer(state, {
      type: 'event',
      event: event({
        name: 'OperationCompleted',
        payload: { operation: listOperation('COMPLETED'), result: page('', ROOT_ENTRIES) },
      }),
    });
    expect(state.nodes['']?.children).toBeNull();
    expect(Object.keys(state.unmatched)).toEqual([OP_ID]);
    state = explorerReducer(state, {
      type: 'listPending',
      path: '',
      operationId: OP_ID,
      append: false,
    });
    expect(state.unmatched).toEqual({});
    expect(state.nodes['']?.children).toHaveLength(4);
    expect(state.nodes['']?.loading).toBe(false);
  });

  it('maps failed and cancelled operations onto the node', () => {
    let state = opened();
    state = explorerReducer(state, {
      type: 'listPending',
      path: '',
      operationId: OP_ID,
      append: false,
    });
    const failed = explorerReducer(state, {
      type: 'event',
      event: event({
        name: 'OperationFailed',
        payload: {
          operation: listOperation('FAILED'),
          error: { code: 'Io', message: 'disk', retryable: true },
        },
      }),
    });
    expect(failed.nodes['']?.error?.code).toBe('Io');
    expect(failed.nodes['']?.loading).toBe(false);
    const cancelled = explorerReducer(state, {
      type: 'event',
      event: event({
        name: 'OperationCancelled',
        payload: { operation: listOperation('CANCELLED') },
      }),
    });
    expect(cancelled.nodes['']?.loading).toBe(false);
    expect(cancelled.nodes['']?.pending).toBeNull();
    expect(cancelled.nodes['']?.children).toBeNull();
  });

  it('ignores operations of other kinds', () => {
    const state = explorerReducer(opened(), {
      type: 'listPending',
      path: '',
      operationId: OP_ID,
      append: false,
    });
    const other = explorerReducer(state, {
      type: 'event',
      event: event({
        name: 'OperationCompleted',
        payload: { operation: { ...listOperation('COMPLETED'), kind: 'operations.demo' } },
      }),
    });
    expect(other).toBe(state);
  });
});

describe('watched entries (T070)', () => {
  /** Root loaded with `src` (loaded and expanded) and `docs` (never opened). */
  function withLoadedTree(): ExplorerState {
    const root = explorerReducer(opened(), {
      type: 'pageLoaded',
      path: '',
      page: page('', [entry('src', { kind: 'dir' }), entry('docs', { kind: 'dir' })]),
      append: false,
    });
    return explorerReducer(root, {
      type: 'pageLoaded',
      path: 'src',
      page: page('src', [entry('main.rs')]),
      append: false,
    });
  }

  function changed(directories: string[], overflowed = false, workspaceId = WS.id): EventEnvelope {
    return event({
      name: 'WorkspaceEntriesChanged',
      payload: { workspaceId, directories, overflowed },
    });
  }

  it('marks a loaded directory stale and leaves an unopened one alone', () => {
    const state = explorerReducer(withLoadedTree(), {
      type: 'event',
      event: changed(['src', 'docs']),
    });
    // `docs` has never been listed: there is nothing on screen to correct.
    expect(state.stale).toEqual(['src']);
    // Nothing else about the node moves until its page comes back.
    expect(state.nodes.src?.loading).toBe(false);
    expect(state.nodes.src?.children).toEqual(['src/main.rs']);
  });

  it('ignores a directory it does not know and a batch for another workspace', () => {
    const loaded = withLoadedTree();
    expect(explorerReducer(loaded, { type: 'event', event: changed(['elsewhere']) })).toBe(loaded);
    const other = changed(['src'], false, 'ws-other' as WorkspaceId);
    expect(explorerReducer(loaded, { type: 'event', event: other })).toBe(loaded);
  });

  it('re-lists every loaded node when the core says the batch overflowed', () => {
    const state = explorerReducer(withLoadedTree(), {
      type: 'event',
      event: changed([], true),
    });
    // The root and `src`, the two that have pages; not `docs`.
    expect([...state.stale].sort()).toEqual(['', 'src']);
  });

  it('accumulates batches and hands each path over exactly once', () => {
    let state = explorerReducer(withLoadedTree(), { type: 'event', event: changed(['src']) });
    state = explorerReducer(state, { type: 'event', event: changed(['', 'src']) });
    expect([...state.stale].sort()).toEqual(['', 'src']);
    // The hook takes what it is about to list; a batch that arrives meanwhile
    // is not swept away with it.
    const taking = explorerReducer(state, { type: 'refreshing', paths: ['src'] });
    expect(taking.stale).toEqual(['']);
    const after = explorerReducer(taking, { type: 'event', event: changed(['src']) });
    expect([...after.stale].sort()).toEqual(['', 'src']);
  });

  it('drops everything pending when the workspace closes', () => {
    const state = explorerReducer(withLoadedTree(), { type: 'event', event: changed(['src']) });
    expect(state.stale).not.toEqual([]);
    const closed = explorerReducer(state, { type: 'workspace', workspace: null });
    expect(closed.stale).toEqual([]);
  });
});

describe('file operations that end without a result (FILES-PARTIAL)', () => {
  /** Root loaded with `src` (loaded) and `docs` (never opened). */
  function withLoadedTree(): ExplorerState {
    const root = explorerReducer(opened(), {
      type: 'pageLoaded',
      path: '',
      page: page('', [entry('src', { kind: 'dir' }), entry('docs', { kind: 'dir' })]),
      append: false,
    });
    return explorerReducer(root, {
      type: 'pageLoaded',
      path: 'src',
      page: page('src', [entry('main.rs')]),
      append: false,
    });
  }

  function filesOperation(status: Operation['status'], workspaceId = WS.id): Operation {
    return { ...listOperation(status), kind: 'files.transfer', workspaceId };
  }

  it('re-lists every loaded node when a transfer fails or is cancelled', () => {
    const failed = explorerReducer(withLoadedTree(), {
      type: 'event',
      event: event({
        name: 'OperationFailed',
        payload: {
          operation: filesOperation('FAILED'),
          error: { code: 'NotFound', message: 'missing.txt does not exist', retryable: false },
        },
      }),
    });
    // It may have moved entries before it stopped, and carries no `changed`.
    expect([...failed.stale].sort()).toEqual(['', 'src']);

    const cancelled = explorerReducer(withLoadedTree(), {
      type: 'event',
      event: event({
        name: 'OperationCancelled',
        payload: { operation: filesOperation('CANCELLED') },
      }),
    });
    expect([...cancelled.stale].sort()).toEqual(['', 'src']);
  });

  it("ignores another workspace's file operation", () => {
    const loaded = withLoadedTree();
    const other = explorerReducer(loaded, {
      type: 'event',
      event: event({
        name: 'OperationCancelled',
        payload: { operation: filesOperation('CANCELLED', 'ws-other' as WorkspaceId) },
      }),
    });
    expect(other).toBe(loaded);
  });

  it('invalidates the whole loaded tree on request', () => {
    const state = explorerReducer(withLoadedTree(), { type: 'invalidate', paths: [], all: true });
    expect([...state.stale].sort()).toEqual(['', 'src']);
  });
});

describe('isExpandable', () => {
  it('allows directories and inside-root directory symlinks only', () => {
    expect(isExpandable(null)).toBe(true);
    expect(isExpandable(entry('src', { kind: 'dir' }))).toBe(true);
    expect(isExpandable(entry('a.txt'))).toBe(false);
    expect(
      isExpandable(
        entry('link', {
          kind: 'symlink',
          symlink: { broken: false, resolvesInsideRoot: true, targetKind: 'dir' },
        }),
      ),
    ).toBe(true);
    expect(
      isExpandable(
        entry('out', {
          kind: 'symlink',
          symlink: { broken: false, resolvesInsideRoot: false, targetKind: 'dir' },
        }),
      ),
    ).toBe(false);
    expect(
      isExpandable(
        entry('broken', { kind: 'symlink', symlink: { broken: true, resolvesInsideRoot: false } }),
      ),
    ).toBe(false);
  });
});
