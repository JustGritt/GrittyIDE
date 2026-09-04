import type { DroppedPath, WorkspaceId } from '@gritty/protocol';
import { describe, expect, it } from 'vitest';
import { describePlan, MAX_FILES_PER_DROP, planDrop } from './drop-policy';

const WS = 'ws-1' as WorkspaceId;

function file(name: string, path = `src/${name}`): DroppedPath {
  return {
    path: `/ws/${path}`,
    name,
    kind: 'FILE_IN_WORKSPACE',
    workspaceId: WS,
    workspacePath: path,
  };
}

function outside(name: string, parent = '/elsewhere'): DroppedPath {
  return { path: `${parent}/${name}`, name, kind: 'FILE_OUTSIDE_WORKSPACE', parent };
}

function directory(name: string, path = `/repos/${name}`): DroppedPath {
  return { path, name, kind: 'DIRECTORY' };
}

describe('planDrop', () => {
  it('opens files that the workspace already contains', () => {
    const plan = planDrop([file('a.ts'), file('b.ts')]);
    expect(plan.openFiles).toEqual([
      { workspaceId: WS, path: 'src/a.ts' },
      { workspaceId: WS, path: 'src/b.ts' },
    ]);
    expect(plan.openWorkspace).toBeUndefined();
    expect(plan.skipped).toEqual([]);
  });

  it('opens a dropped folder as the workspace', () => {
    const plan = planDrop([directory('gritty')]);
    expect(plan.openWorkspace).toBe('/repos/gritty');
    expect(plan.openFiles).toEqual([]);
  });

  it('takes one folder and says what it left', () => {
    const plan = planDrop([directory('one'), directory('two')]);
    expect(plan.openWorkspace).toBe('/repos/one');
    expect(plan.skipped).toEqual(['two (one folder at a time)']);
  });

  it('lets a folder win over files in the same gesture', () => {
    // The files' identity is relative to a root that is about to change.
    const plan = planDrop([file('a.ts'), directory('other')]);
    expect(plan.openWorkspace).toBe('/repos/other');
    expect(plan.openFiles).toEqual([]);
    expect(plan.skipped).toContain('a.ts (dropped with a folder)');
  });

  it('offers the folder once for files outside the workspace', () => {
    const plan = planDrop([outside('notes.md'), outside('todo.md')]);
    expect(plan.openFiles).toEqual([]);
    expect(plan.offerFolder).toEqual({ path: '/elsewhere', files: ['notes.md', 'todo.md'] });
  });

  it('never silently swaps the workspace for an outside file', () => {
    const plan = planDrop([outside('notes.md')]);
    expect(plan.openWorkspace).toBeUndefined();
    expect(plan.offerFolder?.path).toBe('/elsewhere');
  });

  it('reports paths that no longer resolve instead of ignoring them', () => {
    const plan = planDrop([
      { path: '/ws/gone.ts', name: 'gone.ts', kind: 'UNRESOLVABLE' },
      { path: '/ws/sock', name: 'sock', kind: 'UNSUPPORTED' },
    ]);
    expect(plan.skipped).toEqual(['gone.ts no longer exists', 'sock is not a file or folder']);
    expect(plan.openFiles).toEqual([]);
  });

  it('caps how many tabs one gesture can open', () => {
    const many = Array.from({ length: MAX_FILES_PER_DROP + 3 }, (_, i) => file(`f${i}.ts`));
    const plan = planDrop(many);
    expect(plan.openFiles).toHaveLength(MAX_FILES_PER_DROP);
    expect(plan.skipped).toEqual(['3 more files']);
  });

  it('does nothing with an empty drop', () => {
    const plan = planDrop([]);
    expect(plan).toEqual({ openFiles: [], skipped: [] });
    expect(describePlan(plan)).toBeUndefined();
  });

  it('describes what happened in one sentence', () => {
    expect(describePlan(planDrop([directory('gritty')]))).toBe('opened gritty');
    expect(describePlan(planDrop([file('a.ts'), file('b.ts')]))).toBe('opened 2 files');
  });
});
