/** @vitest-environment jsdom */
import { CoreError } from '@gritty/protocol';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ExplorerPanel } from './explorer-panel';
import { initialExplorerState } from './explorer-reducer';
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
