/** @vitest-environment jsdom */
import type { WorkspaceId } from '@gritty/protocol';
import { act, cleanup, fireEvent, render, renderHook, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { SearchPanel } from './search-panel';
import { type SearchController, useSearch } from './use-search';
import { useSearchChord } from './use-search-chord';

/**
 * ⌘⇧F through a real DOM (SEARCH-CHORD): the listener, and the panel taking
 * focus when the real controller is asked to give it.
 *
 * Focus and `defaultPrevented` do not exist in static markup, which is why this
 * file is a DOM test. What it cannot prove is that Monaco lets the keystroke
 * bubble; that rests on Monaco binding nothing on ⌘⇧F, and on the manual check.
 */

// The real controller, over a core that never answers: a query typed here
// stays `starting`, so nothing settles after a test has finished.
vi.mock('../../core/client', async () => {
  const { createCoreClient } = await import('@gritty/protocol');
  return {
    core: createCoreClient({
      invoke: () => new Promise(() => {}),
      listen: async () => () => {},
    }),
  };
});

afterEach(() => {
  cleanup();
});

function press(target: EventTarget, init: KeyboardEventInit): KeyboardEvent {
  const event = new KeyboardEvent('keydown', { bubbles: true, cancelable: true, ...init });
  target.dispatchEvent(event);
  return event;
}

describe('useSearchChord', () => {
  it('fires from inside a text area, and claims the keystroke', () => {
    const onOpen = vi.fn();
    renderHook(() => useSearchChord('meta', true, onOpen));
    const textarea = document.createElement('textarea');
    document.body.append(textarea);
    const event = press(textarea, { code: 'KeyF', key: 'f', metaKey: true, shiftKey: true });
    expect(onOpen).toHaveBeenCalledTimes(1);
    expect(event.defaultPrevented).toBe(true);
    textarea.remove();
  });

  it('leaves ⌘F alone', () => {
    const onOpen = vi.fn();
    renderHook(() => useSearchChord('meta', true, onOpen));
    const event = press(window, { code: 'KeyF', key: 'f', metaKey: true });
    expect(onOpen).not.toHaveBeenCalled();
    expect(event.defaultPrevented).toBe(false);
  });

  it('is inert before the platform is known, and with no workspace', () => {
    const onOpen = vi.fn();
    const { rerender } = renderHook(
      ({ accelerator, enabled }) => useSearchChord(accelerator, enabled, onOpen),
      { initialProps: { accelerator: null as 'meta' | null, enabled: true } },
    );
    const chord = { code: 'KeyF', key: 'f', metaKey: true, shiftKey: true };
    expect(press(window, chord).defaultPrevented).toBe(false);
    rerender({ accelerator: 'meta', enabled: false });
    expect(press(window, chord).defaultPrevented).toBe(false);
    expect(onOpen).not.toHaveBeenCalled();
    rerender({ accelerator: 'meta', enabled: true });
    press(window, chord);
    expect(onOpen).toHaveBeenCalledTimes(1);
  });

  it('calls the latest callback without re-registering', () => {
    const first = vi.fn();
    const second = vi.fn();
    const { rerender } = renderHook(({ onOpen }) => useSearchChord('ctrl', true, onOpen), {
      initialProps: { onOpen: first },
    });
    rerender({ onOpen: second });
    press(window, { code: 'KeyF', key: 'F', ctrlKey: true, shiftKey: true });
    expect(first).not.toHaveBeenCalled();
    expect(second).toHaveBeenCalledTimes(1);
  });
});

let controller: SearchController | null = null;

function Shell({ showPanel }: { showPanel: boolean }) {
  controller = useSearch('ws-1' as WorkspaceId);
  return (
    <>
      <div role="tablist">
        <button type="button" role="tab">
          Search
        </button>
      </div>
      <button type="button">elsewhere</button>
      {showPanel && <SearchPanel search={controller} onOpen={() => {}} />}
    </>
  );
}

function field(): HTMLInputElement {
  return screen.getByRole('textbox', { name: 'Search in files' }) as HTMLInputElement;
}

describe('the panel taking focus', () => {
  it('focuses and selects the query when asked while it is already showing', () => {
    render(<Shell showPanel />);
    act(() => controller?.setQuery('needle'));
    screen.getByRole('button', { name: 'elsewhere' }).focus();
    expect(document.activeElement).not.toBe(field());
    act(() => controller?.requestFocus());
    expect(document.activeElement).toBe(field());
    expect([field().selectionStart, field().selectionEnd]).toEqual([0, 'needle'.length]);
  });

  it('takes focus from the tablist when the chord mounts it, which a keyboard switch does not', () => {
    const { rerender } = render(<Shell showPanel={false} />);
    screen.getByRole('tab').focus();
    // A switch from the keyboard: the tablist keeps focus.
    rerender(<Shell showPanel />);
    expect(document.activeElement).toBe(screen.getByRole('tab'));
    rerender(<Shell showPanel={false} />);
    screen.getByRole('tab').focus();
    // The chord: the request and the mount land together, and the request wins.
    act(() => controller?.requestFocus());
    rerender(<Shell showPanel />);
    expect(document.activeElement).toBe(field());
  });

  it('does not replay an old request when the panel mounts again later', () => {
    const { rerender } = render(<Shell showPanel />);
    act(() => controller?.requestFocus());
    expect(document.activeElement).toBe(field());
    rerender(<Shell showPanel={false} />);
    screen.getByRole('tab').focus();
    rerender(<Shell showPanel />);
    expect(document.activeElement).toBe(screen.getByRole('tab'));
  });

  it('still focuses on an ordinary mount, as a click on the tab does', () => {
    const { rerender } = render(<Shell showPanel={false} />);
    fireEvent.click(screen.getByRole('button', { name: 'elsewhere' }));
    rerender(<Shell showPanel />);
    expect(document.activeElement).toBe(field());
  });
});
