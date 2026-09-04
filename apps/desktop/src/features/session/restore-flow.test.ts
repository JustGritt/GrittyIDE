import type { Document, DocumentId, SessionRestoreResponse, WorkspaceId } from '@gritty/protocol';
import { CoreError, toCoreError } from '@gritty/protocol';
import { describe, expect, it } from 'vitest';
import { type RestoreDeps, runRestore } from './restore-flow';
import type { SessionAction } from './session-reducer';

const ws = 'ws-1' as WorkspaceId;

const response: SessionRestoreResponse = {
  workspace: { id: ws, root: '/repo', name: 'repo', openedAt: 1 },
  tabs: [
    { path: 'README.md', active: false, viewMode: 'preview', cursor: { line: 2, column: 1 } },
    { path: 'src/main.rs', active: true, cursor: { line: 7, column: 3 }, scrollLine: 5 },
    { path: 'gone.md', active: false },
  ],
  preferences: { version: 1, fontSize: 15 },
  layout: { version: 1, explorer: { showHidden: true } },
  persistence: { available: true, healthy: true },
  notices: [],
};

function document(id: string, path: string, workspaceId: WorkspaceId = ws): Document {
  return { id: id as DocumentId, workspaceId, path } as Document;
}

function fakes(options: {
  restore?: () => Promise<SessionRestoreResponse>;
  live?: readonly Document[];
  rehydrateFails?: boolean;
}) {
  const calls: string[] = [];
  const actions: SessionAction[] = [];
  let nextId = 1;
  const deps: RestoreDeps = {
    restore: options.restore ?? (async () => response),
    rehydrate: async () => {
      calls.push('rehydrate');
      if (options.rehydrateFails) throw new Error('list failed');
      return options.live ?? [];
    },
    openFile: async (_workspaceId, path) => {
      calls.push(`open:${path}`);
      if (path === 'gone.md') {
        return {
          ok: false,
          error: new CoreError({ code: 'NotFound', message: 'gone', retryable: false }),
        };
      }
      return { ok: true, documentId: `doc-${nextId++}` as DocumentId };
    },
    applyPreferences: (value) => calls.push(`prefs:${JSON.stringify(value)}`),
    applyLayout: (value) => calls.push(`layout:${JSON.stringify(value)}`),
    applyOrder: (ids, activeId) => calls.push(`order:${ids.join(',')}:${activeId}`),
    setViewMode: (id, mode) => calls.push(`mode:${id}:${mode}`),
    restoreView: (id, state) => calls.push(`view:${id}:${state.cursor.line}:${state.scrollLine}`),
    dispatch: (action) => actions.push(action),
    toCoreError,
  };
  return { deps, calls, actions };
}

describe('runRestore', () => {
  it('fresh start: applies the blobs, opens the active tab first, skips what is gone, restores order and views', async () => {
    const { deps, calls, actions } = fakes({});
    const outcome = await runRestore(deps);
    expect(calls).toEqual([
      'prefs:{"version":1,"fontSize":15}',
      'layout:{"version":1,"explorer":{"showHidden":true}}',
      'rehydrate',
      'open:src/main.rs',
      'open:README.md',
      'open:gone.md',
      'order:doc-2,doc-1:doc-1',
      'mode:doc-2:preview',
      'view:doc-2:2:2',
      'view:doc-1:7:5',
    ]);
    expect(outcome.opened).toEqual(['src/main.rs', 'README.md']);
    expect(outcome.skipped).toEqual(['gone.md']);
    expect(actions.map((a) => a.type)).toEqual([
      'restoreStarted',
      'restoreFinished',
      'restoreSettled',
    ]);
    expect(actions.at(-1)).toEqual({ type: 'restoreSettled', skipped: ['gone.md'] });
  });

  it('renderer reload: documents the core still holds are not reopened, only ordered', async () => {
    const live = [
      document('live-2', 'src/main.rs'),
      document('live-1', 'README.md'),
      document('extra', 'notes.txt'),
    ];
    const { deps, calls } = fakes({ live });
    await runRestore(deps);
    expect(calls.filter((c) => c.startsWith('open:'))).toEqual(['open:gone.md']);
    expect(calls).toContain('order:live-1,live-2,extra:live-2');
    expect(calls).toContain('mode:live-1:preview');
  });

  it('a failed restore still rehydrates and settles', async () => {
    const { deps, calls, actions } = fakes({
      restore: async () => {
        throw { code: 'Timeout', message: 'no answer', retryable: true };
      },
      live: [document('live-1', 'README.md')],
    });
    const outcome = await runRestore(deps);
    expect(outcome.response).toBeNull();
    expect(calls).toEqual(['rehydrate']);
    expect(actions.map((a) => a.type)).toEqual([
      'restoreStarted',
      'restoreFailed',
      'restoreSettled',
    ]);
  });

  it('a failed rehydration does not stop the restore', async () => {
    const { deps, calls, actions } = fakes({ rehydrateFails: true });
    const outcome = await runRestore(deps);
    expect(outcome.opened).toEqual(['src/main.rs', 'README.md']);
    expect(calls).toContain('rehydrate');
    expect(actions.at(-1)?.type).toBe('restoreSettled');
  });

  it('no workspace: nothing to open, tabs ignored, views untouched', async () => {
    const { deps, calls } = fakes({
      restore: async () => ({ ...response, workspace: undefined }),
    });
    const outcome = await runRestore(deps);
    expect(outcome.opened).toEqual([]);
    expect(calls.some((c) => c.startsWith('open:') || c.startsWith('order:'))).toBe(false);
  });

  it('documents of another workspace are left alone', async () => {
    const live = [document('foreign', 'README.md', 'ws-9' as WorkspaceId)];
    const { deps, calls } = fakes({ live });
    await runRestore(deps);
    expect(calls).toContain('open:README.md');
    expect(calls.find((c) => c.startsWith('order:'))).not.toContain('foreign');
  });
});
