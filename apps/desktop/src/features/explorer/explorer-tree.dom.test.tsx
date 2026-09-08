/** @vitest-environment jsdom */
import type { DirEntry, Workspace, WorkspaceId } from '@gritty/protocol';
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ExplorerTree } from './explorer-tree';
import { useExplorer } from './use-explorer';

/**
 * The tree, driven the way a person drives it (T042).
 *
 * The pure modules cover the algebra; this covers the half they cannot — that
 * the roles and the roving `tabIndex` are actually on the DOM, that a keystroke
 * reaches the right command, and that the inline input commits. It is the
 * closest thing to the manual pass that a machine with no screen can run.
 */

const WORKSPACE: Workspace = {
  id: 'ws-1' as WorkspaceId,
  root: '/tmp/gritty-fixture',
  name: 'gritty-fixture',
  openedAt: 1,
};

function dir(name: string): DirEntry {
  return { name, kind: 'dir', hidden: false, ignoredHint: false };
}

function file(name: string): DirEntry {
  return { name, kind: 'file', hidden: false, ignoredHint: false };
}

const seam = vi.hoisted(() => {
  const handlers = new Map<string, (args?: Record<string, unknown>) => unknown>();
  const calls: { command: string; args?: Record<string, unknown> }[] = [];
  return {
    handlers,
    calls,
    transport: {
      async invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
        calls.push({ command, args });
        const handler = handlers.get(command);
        if (!handler) {
          throw { code: 'Internal', message: `unstubbed command ${command}`, retryable: false };
        }
        return (await handler(args)) as T;
      },
      async listen(channel: string, handler: (payload: unknown) => void) {
        void channel;
        void handler;
        return () => undefined;
      },
    },
  };
});

vi.mock('../../core/client', async () => {
  const { createCoreClient } = await import('@gritty/protocol');
  return { core: createCoreClient(seam.transport) };
});

const ROOT_PAGE = {
  path: '',
  entries: [dir('src'), file('README.md'), file('notes.txt')],
  total: 3,
  truncated: false,
};

beforeEach(() => {
  seam.handlers.clear();
  seam.calls.length = 0;
  seam.handlers.set('core_health', () => ({
    protocolVersion: '2.20.0',
    coreVersion: '0.1.0',
    os: 'macos',
    arch: 'aarch64',
    debug: true,
    shell: { appVersion: '0.1.0', runtime: 'tauri', runtimeVersion: '2.11.5' },
  }));
  seam.handlers.set('workspace_current', () => WORKSPACE);
  seam.handlers.set('workspace_list', () => ({ status: 'ready', page: ROOT_PAGE }));
  seam.handlers.set('files_rename', () => ({ changed: [''], moved: [], undo: 'Rename notes.txt' }));
  seam.handlers.set('files_create', () => ({ changed: [''], moved: [] }));
  seam.handlers.set('files_delete', () => ({
    status: 'ready',
    result: { changed: [''], moved: [] },
  }));
  seam.handlers.set('files_move', () => ({
    status: 'ready',
    result: { changed: [''], moved: [] },
  }));
});

afterEach(cleanup);

/**
 * One component, not `renderHook` plus `render`: those are two React trees, so
 * the tree would hold a controller snapshot that never updates and every
 * keystroke would appear to do nothing.
 */
function Harness() {
  const explorer = useExplorer();
  return <ExplorerTree explorer={explorer} />;
}

/** A mounted tree over a loaded root. */
async function mountTree() {
  const view = render(<Harness />);
  // Let the workspace snapshot and the root listing settle.
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();
  });
  return { view };
}

function rows(container: HTMLElement): HTMLElement[] {
  return Array.from(container.querySelectorAll<HTMLElement>('[role="treeitem"]'));
}

function calledWith(command: string): Record<string, unknown> | undefined {
  const call = seam.calls.find((entry) => entry.command === command);
  return call?.args?.request as Record<string, unknown> | undefined;
}

describe('the tree as a widget', () => {
  it('is a tree of treeitems, with levels and a selection', async () => {
    const { view } = await mountTree();
    const tree = view.container.querySelector('[role="tree"]');
    expect(tree).not.toBeNull();
    expect(tree?.getAttribute('aria-multiselectable')).toBe('true');

    const items = rows(view.container);
    expect(items.map((row) => row.querySelector('.tree__name')?.textContent)).toEqual([
      'src',
      'README.md',
      'notes.txt',
    ]);
    for (const item of items) {
      expect(item.getAttribute('aria-level')).toBe('1');
      expect(item.getAttribute('aria-selected')).toBe('false');
    }
  });

  it('moves the roving tabIndex with the arrow keys', async () => {
    const { view } = await mountTree();
    const tree = view.container.querySelector('[role="tree"]') as HTMLElement;

    fireEvent.keyDown(tree, { code: 'ArrowDown' });
    let items = rows(view.container);
    expect(items[0]?.getAttribute('aria-selected')).toBe('true');
    expect(items[0]?.tabIndex).toBe(0);
    expect(items[1]?.tabIndex).toBe(-1);

    fireEvent.keyDown(tree, { code: 'ArrowDown' });
    items = rows(view.container);
    expect(items[1]?.getAttribute('aria-selected')).toBe('true');
    expect(items[0]?.getAttribute('aria-selected')).toBe('false');
  });

  it('Shift+Down extends the selection rather than replacing it', async () => {
    const { view } = await mountTree();
    const tree = view.container.querySelector('[role="tree"]') as HTMLElement;
    fireEvent.keyDown(tree, { code: 'ArrowDown' });
    fireEvent.keyDown(tree, { code: 'ArrowDown', shiftKey: true });

    const selected = rows(view.container).filter(
      (row) => row.getAttribute('aria-selected') === 'true',
    );
    expect(selected).toHaveLength(2);
  });
});

describe('the operations', () => {
  it('F2 opens an inline input and Enter renames through the core', async () => {
    const { view } = await mountTree();
    const tree = view.container.querySelector('[role="tree"]') as HTMLElement;

    // Focus the third row (`notes.txt`) and rename it.
    fireEvent.keyDown(tree, { code: 'End' });
    fireEvent.keyDown(tree, { code: 'F2' });

    const input = view.container.querySelector<HTMLInputElement>('.tree__input input');
    expect(input, 'F2 opens the inline input').not.toBeNull();
    expect(input?.value).toBe('notes.txt');

    await act(async () => {
      fireEvent.change(input as HTMLInputElement, { target: { value: 'renamed.txt' } });
      fireEvent.keyDown(input as HTMLInputElement, { key: 'Enter' });
      await Promise.resolve();
    });

    expect(calledWith('files_rename')).toMatchObject({
      path: 'notes.txt',
      newName: 'renamed.txt',
    });
  });

  it('refuses to send a name the policy rejects, and Escape sends nothing', async () => {
    const { view } = await mountTree();
    const tree = view.container.querySelector('[role="tree"]') as HTMLElement;
    fireEvent.keyDown(tree, { code: 'End' });
    fireEvent.keyDown(tree, { code: 'F2' });
    const input = view.container.querySelector<HTMLInputElement>('.tree__input input');

    await act(async () => {
      fireEvent.change(input as HTMLInputElement, { target: { value: 'a/b' } });
      fireEvent.keyDown(input as HTMLInputElement, { key: 'Enter' });
      await Promise.resolve();
    });
    expect(input?.getAttribute('aria-invalid')).toBe('true');
    expect(view.container.querySelector('.tree__problem')?.textContent).toMatch(/"\/"/);
    expect(calledWith('files_rename'), 'a bad name never reaches the core').toBeUndefined();

    await act(async () => {
      fireEvent.keyDown(input as HTMLInputElement, { key: 'Escape' });
      await Promise.resolve();
    });
    expect(view.container.querySelector('.tree__input')).toBeNull();
    expect(calledWith('files_rename')).toBeUndefined();
  });

  it('Delete trashes without a dialog, and Shift+Delete asks first', async () => {
    const { view } = await mountTree();
    const tree = view.container.querySelector('[role="tree"]') as HTMLElement;
    fireEvent.keyDown(tree, { code: 'ArrowDown' });

    await act(async () => {
      fireEvent.keyDown(tree, { code: 'Backspace', shiftKey: true });
      await Promise.resolve();
    });
    // A permanent delete is confirmed, so nothing has been sent yet.
    const dialog = view.container.querySelector('[role="alertdialog"]');
    expect(dialog, 'Shift+Delete asks first').not.toBeNull();
    expect(calledWith('files_delete')).toBeUndefined();

    await act(async () => {
      const confirm = view.container.querySelector('.dialog__danger') as HTMLButtonElement;
      fireEvent.click(confirm);
      await Promise.resolve();
    });
    expect(calledWith('files_delete')).toMatchObject({ paths: ['src'], mode: 'PERMANENT' });
  });

  it('a plain Delete goes straight to the Trash', async () => {
    const { view } = await mountTree();
    const tree = view.container.querySelector('[role="tree"]') as HTMLElement;
    fireEvent.keyDown(tree, { code: 'ArrowDown' });

    await act(async () => {
      fireEvent.keyDown(tree, { code: 'Backspace' });
      await Promise.resolve();
    });
    expect(view.container.querySelector('[role="alertdialog"]')).toBeNull();
    expect(calledWith('files_delete')).toMatchObject({ paths: ['src'], mode: 'TRASH' });
  });

  it('cut then paste moves the selection into the folder under the cursor', async () => {
    const { view } = await mountTree();
    const tree = view.container.querySelector('[role="tree"]') as HTMLElement;

    // Cut `notes.txt`…
    fireEvent.keyDown(tree, { code: 'End' });
    fireEvent.keyDown(tree, { code: 'KeyX', metaKey: true });
    // …then paste onto `src`, which is a directory, so it lands inside it.
    fireEvent.keyDown(tree, { code: 'Home' });
    await act(async () => {
      fireEvent.keyDown(tree, { code: 'KeyV', metaKey: true });
      await Promise.resolve();
    });

    expect(calledWith('files_move')).toMatchObject({
      paths: ['notes.txt'],
      targetDir: 'src',
      mode: 'MOVE',
    });
  });

  it('right-clicking a row opens the menu with the file actions on it', async () => {
    const { view } = await mountTree();
    const first = rows(view.container)[0] as HTMLElement;
    fireEvent.contextMenu(first, { clientX: 10, clientY: 10 });

    const menu = document.querySelector('[role="menu"]');
    expect(menu).not.toBeNull();
    const labels = Array.from(menu?.querySelectorAll('[role="menuitem"]') ?? []).map(
      (item) => item.textContent,
    );
    expect(labels).toEqual(
      expect.arrayContaining([
        'New File…',
        'Rename…',
        'Copy Path',
        'Reveal in Finder',
        'Move to Trash',
      ]),
    );
  });

  it('dragging a file onto a folder moves it there', async () => {
    const { view } = await mountTree();
    const items = rows(view.container);
    const src = items[0] as HTMLElement;
    const notes = items[2] as HTMLElement;
    const dataTransfer = { setData: vi.fn(), effectAllowed: '', dropEffect: '', getData: () => '' };

    fireEvent.dragStart(notes, { dataTransfer });
    fireEvent.dragOver(src, { dataTransfer });
    await act(async () => {
      fireEvent.drop(src, { dataTransfer });
      await Promise.resolve();
    });

    expect(calledWith('files_move')).toMatchObject({
      paths: ['notes.txt'],
      targetDir: 'src',
      mode: 'MOVE',
    });
  });
});
