/** @vitest-environment jsdom */
import { CoreError } from '@gritty/protocol';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ExplorerPanel } from './explorer-panel';
import { type ExplorerState, initialExplorerState } from './explorer-reducer';
import type { ExplorerController } from './use-explorer';

/**
 * EXPLORER-DISCARD-NOTICE, at the panel: the "discard them and continue"
 * notice follows the controller's `canDiscardAndRetry`, not `error.code`.
 *
 * The hook's half — when `canDiscardAndRetry` is true — is in
 * `use-explorer-force-replay.dom.test.tsx`. This is the wiring: the panel used
 * to render the notice for every `Conflict`.
 *
 * Rendered on the Changes tab with nothing in it, so the tree (and the core it
 * would call) is never mounted: the notice sits above both tabs.
 */

const TAKEN_NAME = new CoreError({
  code: 'Conflict',
  message: 'src/b.ts already exists',
  retryable: false,
});

function controller(error: CoreError, canDiscardAndRetry: boolean) {
  return {
    state: initialExplorerState,
    error,
    canDiscardAndRetry,
    discardAndRetry: vi.fn(async () => {}),
    clearError: vi.fn(),
    dismissNotice: vi.fn(),
    openFolder: vi.fn(),
  } as unknown as ExplorerController;
}

function renderPanel(explorer: ExplorerController) {
  return render(<ExplorerPanel explorer={explorer} tab="changes" onTabChange={() => {}} />);
}

describe('the discard notice', () => {
  afterEach(cleanup);

  it('is absent for a Conflict that is not a dirty-document refusal', () => {
    renderPanel(controller(TAKEN_NAME, false));
    expect(screen.getByText(/src\/b\.ts already exists/)).toBeTruthy();
    expect(screen.queryByRole('button', { name: 'discard them and continue' })).toBeNull();
  });

  it('replays through the controller when the refusal is the error on screen', () => {
    const explorer = controller(
      new CoreError({
        code: 'Conflict',
        message: '1 document(s) have unsaved changes',
        retryable: false,
        details: { dirtyDocuments: [{ path: 'notes.md' }] },
      }),
      true,
    );
    renderPanel(explorer);
    fireEvent.click(screen.getByRole('button', { name: 'discard them and continue' }));
    expect(explorer.discardAndRetry).toHaveBeenCalledTimes(1);
  });
});

/**
 * WATCH-DEGRADED, at the panel: the notice says which kind of degraded it is,
 * and its buttons reach the controller.
 */
describe('the watch notice', () => {
  afterEach(cleanup);

  function watching(degraded: ExplorerState['watchDegraded'], dismissed = false) {
    return {
      state: {
        ...initialExplorerState,
        watchDegraded: degraded,
        watchNoticeDismissed: dismissed,
      },
      error: null,
      canDiscardAndRetry: false,
      refreshAll: vi.fn(),
      dismissWatchNotice: vi.fn(),
      clearError: vi.fn(),
      dismissNotice: vi.fn(),
      openFolder: vi.fn(),
    } as unknown as ExplorerController;
  }

  it('says some folders are not watched, and refreshes or dismisses', () => {
    const explorer = watching(['DIRECTORY_LIMIT']);
    renderPanel(explorer);
    expect(screen.getByRole('status').textContent).toMatch(/Some folders aren’t watched/);

    fireEvent.click(screen.getByRole('button', { name: 'Refresh tree' }));
    expect(explorer.refreshAll).toHaveBeenCalledTimes(1);
    fireEvent.click(screen.getByRole('button', { name: 'Dismiss' }));
    expect(explorer.dismissWatchNotice).toHaveBeenCalledTimes(1);
  });

  it('says nothing is watched when the backend is unavailable', () => {
    renderPanel(watching(['BACKEND_UNAVAILABLE', 'DIRECTORY_LIMIT']));
    expect(screen.getByRole('status').textContent).toMatch(
      /Changes made outside Gritty aren’t being watched/,
    );
  });

  it('shows nothing for a healthy watch, or once dismissed', () => {
    renderPanel(watching([]));
    expect(screen.queryByRole('button', { name: 'Refresh tree' })).toBeNull();
    cleanup();
    renderPanel(watching(['DIRECTORY_LIMIT'], true));
    expect(screen.queryByRole('button', { name: 'Refresh tree' })).toBeNull();
  });
});
