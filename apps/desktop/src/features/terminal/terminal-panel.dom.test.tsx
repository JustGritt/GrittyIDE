/** @vitest-environment jsdom */
import type { TerminalProfile } from '@gritty/protocol';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { DEFAULT_TERMINAL_LAYOUT, type TerminalLayout } from '../session/persisted-layout';
import { createOutputBus } from './output-bus';
import { TerminalPanel } from './terminal-panel';
import type { TerminalsController } from './use-terminals';

/**
 * The dock's two ways into its settings, now that they are settings (T120-A).
 *
 * The bar used to carry a popover that edited a copy of them. What is asserted
 * here is that it no longer does: the `⋯` asks for the settings page, and the
 * shell picker hands the chosen path to its one owner rather than patching a
 * layout. No session is open, so no xterm is constructed.
 */

vi.mock('../../core/client', () => ({ core: {} }));

afterEach(cleanup);

function profile(path: string, name: string, isDefault = false): TerminalProfile {
  return { path, name, isDefault, isCustom: false, available: true };
}

function controller(profiles: readonly TerminalProfile[]): TerminalsController {
  const noop = async () => null;
  return {
    terminals: [],
    profiles: [...profiles],
    // Not ready, so the dock does not try to start a shell on its own.
    ready: false,
    active: null,
    workspaceId: null,
    bus: createOutputBus(),
    error: null,
    spawn: noop,
    close: async () => {},
    kill: async () => {},
    rename: async () => {},
    recolor: async () => {},
    activate: () => {},
    write: () => {},
    resize: () => {},
    clearError: () => {},
  };
}

function renderDock(settings: TerminalLayout = DEFAULT_TERMINAL_LAYOUT) {
  const onShellChange = vi.fn();
  const onOpenSettings = vi.fn();
  render(
    <TerminalPanel
      terminals={controller([
        profile('/bin/zsh', 'zsh', true),
        profile('/opt/homebrew/bin/fish', 'fish'),
      ])}
      open={false}
      onOpenChange={() => {}}
      workspace={null}
      cwd={undefined}
      chords={false}
      settings={settings}
      onShellChange={onShellChange}
      onOpenSettings={onOpenSettings}
    />,
  );
  return { onShellChange, onOpenSettings };
}

describe('the terminal dock and its settings', () => {
  it('opens the settings page from the bar rather than a popover', () => {
    const { onOpenSettings } = renderDock();
    // The popover's fields are gone from the bar: there is one place to edit.
    expect(screen.queryByLabelText('Path of a shell to add')).toBeNull();
    expect(document.querySelector('details')).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Terminal settings' }));
    expect(onOpenSettings).toHaveBeenCalledTimes(1);
  });

  it('hands the picked shell to its owner as a path', () => {
    const { onShellChange, onOpenSettings } = renderDock();
    fireEvent.click(screen.getByRole('button', { name: 'Shell for new terminals' }));
    fireEvent.click(screen.getByRole('menuitem', { name: 'fish' }));
    expect(onShellChange).toHaveBeenCalledWith('/opt/homebrew/bin/fish');
    expect(onOpenSettings).not.toHaveBeenCalled();
  });

  it('shows the shell the settings name', () => {
    renderDock({ ...DEFAULT_TERMINAL_LAYOUT, profilePath: '/opt/homebrew/bin/fish' });
    expect(screen.getByRole('button', { name: 'Shell for new terminals' }).textContent).toContain(
      'fish',
    );
  });
});
