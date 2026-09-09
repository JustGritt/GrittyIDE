/** @vitest-environment jsdom */
import type { HealthReport, Workspace, WorkspaceId } from '@gritty/protocol';
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { AppBar, type HealthState, type QuickOpenMount } from './app-bar';

/**
 * The bar with quick open in it (T130-M §1, §5, §7).
 *
 * jsdom rather than static markup, because the two things worth asserting are
 * both behaviour: that the chord opens the overlay, and that opening it moves
 * nothing else. The chord's *rules* are `quick-open-chord.test.ts`'s and need
 * no DOM at all.
 */

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

const WORKSPACE: Workspace = {
  id: 'ws-1' as WorkspaceId,
  root: '/tmp/gritty-fixture',
  name: 'gritty-fixture',
  openedAt: 1,
};

const HEALTH: HealthReport = {
  protocolVersion: '2.26.0',
  coreVersion: '0.1.0',
  os: 'macos',
  arch: 'aarch64',
  debug: true,
  shell: { appVersion: '0.1.0', runtime: 'tauri', runtimeVersion: '2.11.5' },
};

const ready: HealthState = { status: 'ready', health: HEALTH };

function mount(overrides: Partial<QuickOpenMount> = {}): QuickOpenMount {
  return {
    accelerator: 'meta',
    deps: {
      workspaceId: WORKSPACE.id,
      hasWorkspace: true,
      document: null,
      terminalVisible: false,
      openFolder: () => {},
      saveDocument: () => {},
      toggleTerminal: () => {},
      openSettings: () => {},
      openFile: () => {},
      revealLine: () => {},
    },
    ...overrides,
  };
}

function pressQuickOpen(options: { shift?: boolean } = {}): void {
  act(() => {
    fireEvent.keyDown(window, {
      code: 'KeyP',
      key: 'p',
      metaKey: true,
      shiftKey: options.shift ?? false,
    });
  });
}

describe('the app bar with quick open', () => {
  afterEach(cleanup);
  beforeEach(() => {
    seam.handlers.clear();
    seam.handlers.set('core_health', () => HEALTH);
  });

  it('keeps its existing slots, and shows no field until the chord', () => {
    const { container } = render(
      <AppBar
        health={ready}
        workspace={WORKSPACE}
        island={<span data-testid="island" />}
        quickOpen={mount()}
      />,
    );
    // The four things that were already in the bar are unmoved.
    expect(screen.getByText('Gritty')).toBeTruthy();
    expect(screen.getByText(WORKSPACE.name)).toBeTruthy();
    expect(screen.getByTestId('island')).toBeTruthy();
    expect(container.querySelector('.appbar__health')).toBeTruthy();
    // And the overlay is not in the tree at all until it is asked for, which
    // is what makes "opening it moves no pixel" true rather than aspirational.
    expect(screen.queryByRole('combobox')).toBeNull();
  });

  it('opens on the chord, into the mode the chord named', () => {
    render(<AppBar health={ready} workspace={WORKSPACE} quickOpen={mount()} />);
    pressQuickOpen();
    const field = screen.getByRole('combobox') as HTMLInputElement;
    expect(field.value).toBe('');

    fireEvent.keyDown(field, { code: 'Escape' });
    expect(screen.queryByRole('combobox')).toBeNull();

    // ⌘⇧P opens straight into command mode, pre-filled with the prefix (§5).
    pressQuickOpen({ shift: true });
    expect((screen.getByRole('combobox') as HTMLInputElement).value).toBe('>');
  });

  it('focuses and selects on a second chord rather than toggling shut', () => {
    render(<AppBar health={ready} workspace={WORKSPACE} quickOpen={mount()} />);
    pressQuickOpen();
    const field = screen.getByRole('combobox') as HTMLInputElement;
    fireEvent.change(field, { target: { value: 'epn' } });
    field.blur();

    pressQuickOpen();
    // Still open — a second ⌘P is "let me retype this", never "close it".
    expect(screen.queryByRole('combobox')).toBe(field);
    expect(window.document.activeElement).toBe(field);
    expect(field.selectionStart).toBe(0);
    expect(field.selectionEnd).toBe('epn'.length);
  });

  it('is inert until the core has said what platform this is', () => {
    // Guessing the modifier would mean stealing the wrong chord on the wrong
    // platform; `useOpenShortcuts` is inert for the same reason.
    render(
      <AppBar
        health={{ status: 'loading' }}
        workspace={WORKSPACE}
        quickOpen={mount({ accelerator: null })}
      />,
    );
    pressQuickOpen();
    expect(screen.queryByRole('combobox')).toBeNull();
  });

  it('opens from Monaco’s own registration, which the window listener cannot see', () => {
    // The case a naive listener gets wrong: the editor puts a <textarea> under
    // the cursor and the window listener yields to those, so quick open's
    // primary case has to arrive by this path.
    let fromEditor: ((commands: boolean) => void) | undefined;
    render(
      <AppBar
        health={ready}
        workspace={WORKSPACE}
        quickOpen={mount({
          subscribeEditorChord: (listener) => {
            fromEditor = listener;
            return () => {
              fromEditor = undefined;
            };
          },
        })}
      />,
    );
    expect(fromEditor).toBeDefined();
    act(() => fromEditor?.(true));
    expect((screen.getByRole('combobox') as HTMLInputElement).value).toBe('>');
  });

  it('dismisses rather than holding stale rows when the workspace closes under it', () => {
    const view = render(<AppBar health={ready} workspace={WORKSPACE} quickOpen={mount()} />);
    pressQuickOpen();
    expect(screen.queryByRole('combobox')).not.toBeNull();

    view.rerender(
      <AppBar
        health={ready}
        workspace={null}
        quickOpen={mount({
          deps: { ...mount().deps, workspaceId: null, hasWorkspace: false },
        })}
      />,
    );
    expect(screen.queryByRole('combobox')).toBeNull();
  });

  it('renders nothing of its own when the shell gives it no quick open', () => {
    render(<AppBar health={ready} workspace={WORKSPACE} />);
    pressQuickOpen();
    expect(screen.queryByRole('combobox')).toBeNull();
    expect(screen.getByText('Gritty')).toBeTruthy();
  });
});
