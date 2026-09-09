/** @vitest-environment jsdom */
import type { FindFilesResponse, HealthReport, WorkspaceId } from '@gritty/protocol';
import { PROTOCOL_VERSION } from '@gritty/protocol';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { QuickOpenBar } from './quick-open-bar';
import { type QuickOpenDeps, useQuickOpen } from './use-quick-open';

/**
 * The bar, driven the way a person drives it (T130-V).
 *
 * Only what a string cannot say: that the field actually holds the focus, that
 * an arrow moves `aria-activedescendant` on the real DOM, that Enter reaches
 * `openFile` with a preview, that a click outside dismisses, and that the caret
 * goes back where it came from. `quick-open-render.test.ts` covers the markup
 * and every list arm.
 *
 * **The bar, not `App`.** Rendering `App` would make this the second consumer
 * of the four jsdom shims inlined in `shell-restore.dom.test.tsx`, and
 * JSDOM-VITEST says that is the moment they must move to a shared setup file.
 */

const WS = 'ws-1' as WorkspaceId;

const seam = vi.hoisted(() => {
  const handlers = new Map<string, (args?: Record<string, unknown>) => unknown>();
  return {
    handlers,
    transport: {
      async invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
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

function Harness(props: Partial<QuickOpenDeps> & { readonly autoFocus?: boolean }) {
  const { autoFocus = true, ...overrides } = props;
  const controller = useQuickOpen({
    workspaceId: WS,
    hasWorkspace: true,
    document: null,
    terminalVisible: false,
    openFolder: () => {},
    saveDocument: () => {},
    toggleTerminal: () => {},
    openSettings: () => {},
    openFile: () => {},
    revealLine: () => {},
    onDismiss: () => {},
    ...overrides,
  });
  return (
    <QuickOpenBar
      raw={controller.raw}
      list={controller.list}
      activeId={controller.activeId}
      onRawChange={controller.setRaw}
      onKey={controller.handleKey}
      onFocusRow={controller.focusRow}
      onAccept={controller.accept}
      onDismiss={overrides.onDismiss ?? (() => {})}
      autoFocus={autoFocus}
    />
  );
}

function field(): HTMLInputElement {
  return screen.getByRole('combobox') as HTMLInputElement;
}

/** Type into the field and let the debounced query answer. */
async function search(term: string): Promise<void> {
  fireEvent.change(field(), { target: { value: term } });
  await waitFor(() => expect(screen.getAllByRole('option').length).toBeGreaterThan(0));
}

function activeDescendant(): string | null {
  return field().getAttribute('aria-activedescendant');
}

const chord = (code: string, extra: Record<string, unknown> = {}) => ({ code, ...extra });

describe('the quick-open bar', () => {
  afterEach(cleanup);

  beforeEach(() => {
    seam.handlers.clear();
    seam.handlers.set(
      'core_health',
      (): HealthReport => ({
        protocolVersion: PROTOCOL_VERSION,
        coreVersion: '0.1.0',
        os: 'macos',
        arch: 'aarch64',
        debug: true,
        shell: { appVersion: '0.1.0', runtime: 'tauri', runtimeVersion: '2.11.5' },
      }),
    );
    seam.handlers.set(
      'workspace_find_files',
      (): FindFilesResponse => ({
        status: 'ready',
        matches: [
          { path: 'src/editor-pane.tsx', score: 158, ranges: [[4, 1]] },
          { path: 'src/expand.rs', score: 81, ranges: [[4, 1]] },
        ],
        truncated: false,
      }),
    );
    seam.handlers.set('operations_cancel', () => ({}));
  });

  it('takes the focus when it opens', () => {
    render(<Harness />);
    expect(window.document.activeElement).toBe(field());
  });

  it('moves aria-activedescendant with the arrows, and clamps at the ends', async () => {
    render(<Harness />);
    await search('e');
    // Nothing focused yet: Down lands on the first row, which is what a
    // quick-open does. The explorer's `moveFocus` already decides this.
    expect(activeDescendant()).toBeNull();

    act(() => {
      fireEvent.keyDown(field(), chord('ArrowDown'));
    });
    const first = activeDescendant();
    expect(first).toMatch(/file:0$/);

    act(() => {
      fireEvent.keyDown(field(), chord('ArrowDown'));
    });
    expect(activeDescendant()).toMatch(/file:1$/);

    // Clamps rather than wrapping, as the explorer tree does.
    act(() => {
      fireEvent.keyDown(field(), chord('ArrowDown'));
    });
    expect(activeDescendant()).toMatch(/file:1$/);

    act(() => {
      fireEvent.keyDown(field(), chord('Home'));
    });
    expect(activeDescendant()).toBe(first);
  });

  it('marks the active row as selected, for a reader that is not looking at colour', async () => {
    render(<Harness />);
    await search('e');
    act(() => {
      fireEvent.keyDown(field(), chord('ArrowDown'));
    });
    const selected = screen.getAllByRole('option').filter((row) => row.ariaSelected === 'true');
    expect(selected).toHaveLength(1);
  });

  it('opens the first row on Enter as a preview, before anything is focused', async () => {
    const openFile = vi.fn();
    render(<Harness openFile={openFile} />);
    await search('e');
    act(() => {
      fireEvent.keyDown(field(), chord('Enter'));
    });
    // §5: Enter is a *look*, matching the explorer's single click.
    expect(openFile).toHaveBeenCalledWith('src/editor-pane.tsx', { preview: true });
  });

  it('opens a permanent tab on Cmd-Enter', async () => {
    const openFile = vi.fn();
    render(<Harness openFile={openFile} />);
    await search('e');
    act(() => {
      fireEvent.keyDown(field(), chord('Enter', { metaKey: true }));
    });
    expect(openFile).toHaveBeenCalledWith('src/editor-pane.tsx', { preview: false });
  });

  it('lets an unclaimed key through to the input, which is what makes typing work', async () => {
    render(<Harness />);
    await search('e');
    const event = new KeyboardEvent('keydown', {
      code: 'KeyX',
      bubbles: true,
      cancelable: true,
    });
    act(() => {
      field().dispatchEvent(event);
    });
    // §8: everything the keymap does not claim reaches the field.
    expect(event.defaultPrevented).toBe(false);
  });

  it('dismisses on Escape', async () => {
    const onDismiss = vi.fn();
    render(<Harness onDismiss={onDismiss} />);
    await search('e');
    act(() => {
      fireEvent.keyDown(field(), chord('Escape'));
    });
    expect(onDismiss).toHaveBeenCalled();
  });

  it('dismisses on a click outside, and not on one inside the bar', async () => {
    const onDismiss = vi.fn();
    render(<Harness onDismiss={onDismiss} />);
    await search('e');

    // Clicking the field itself is not "outside". (Clicking a *row* does close
    // the bar, but by accepting it — that is the row test above, not this one.)
    act(() => {
      fireEvent.mouseDown(field());
    });
    expect(onDismiss).not.toHaveBeenCalled();

    act(() => {
      fireEvent.mouseDown(window.document.body);
    });
    expect(onDismiss).toHaveBeenCalled();
  });

  it('opens a row the pointer accepted, without accepting one it merely passed over', async () => {
    const openFile = vi.fn();
    render(<Harness openFile={openFile} />);
    await search('e');
    const rows = screen.getAllByRole('option');

    // Hovering moves the focus and nothing else. Accepting on hover is how a
    // palette opens the wrong file.
    act(() => {
      fireEvent.mouseMove(rows[1] as HTMLElement);
    });
    expect(openFile).not.toHaveBeenCalled();
    expect(activeDescendant()).toMatch(/file:1$/);

    act(() => {
      fireEvent.mouseDown(rows[1] as HTMLElement);
    });
    expect(openFile).toHaveBeenCalledWith('src/expand.rs', { preview: true });
  });

  it('gives the caret back to where it came from when it closes', async () => {
    const before = window.document.createElement('textarea');
    window.document.body.appendChild(before);
    before.focus();
    expect(window.document.activeElement).toBe(before);

    const view = render(<Harness />);
    expect(window.document.activeElement).toBe(field());
    view.unmount();
    // The bar stole the caret from a buffer; giving it back is its own job.
    expect(window.document.activeElement).toBe(before);
    before.remove();
  });
});
