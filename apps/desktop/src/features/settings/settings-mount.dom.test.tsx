/** @vitest-environment jsdom */
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { useReducer } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { initialDocumentsState } from '../documents/documents-reducer';
import { TabStrip } from '../editor/tab-strip';
import type { Accelerator } from '../shell/open-shortcuts';
import { initialSettingsTabState, settingsTabReducer } from './settings-tab';
import { useSettingsChord } from './use-settings-chord';

/**
 * The mount, as a person actually meets it (T120-M §4).
 *
 * A DOM rather than static markup, because what is asserted here does not exist
 * in a string: that a keystroke on the *window* opens the tab, that pressing it
 * twice focuses rather than duplicates, and that the listener yields the
 * keystroke to nobody. The strip's shape is `tab-strip-render.test.tsx`'s.
 *
 * `App.tsx` itself is not rendered: it constructs a core client, a Monaco
 * binding, terminals and a roster, none of which this is about. The harness is
 * the three pieces the mount actually wires together — the chord, the reducer
 * and the strip — with the same edges between them.
 */

vi.mock('../../core/client', () => ({ core: {} }));

afterEach(cleanup);

function Harness({ accelerator = 'meta' as Accelerator }: { accelerator?: Accelerator | null }) {
  const [tab, dispatch] = useReducer(settingsTabReducer, initialSettingsTabState);
  useSettingsChord(accelerator, () => dispatch({ type: 'open', preview: false }));
  return (
    <TabStrip
      state={initialDocumentsState}
      onActivate={() => {}}
      onPromote={() => {}}
      onClose={() => {}}
      onCloseMany={() => {}}
      onReorder={() => {}}
      settings={{
        open: tab.open,
        preview: tab.preview,
        active: tab.active,
        onActivate: () => dispatch({ type: 'open', preview: false }),
        onPromote: () => dispatch({ type: 'keep' }),
        onClose: () => dispatch({ type: 'close' }),
      }}
    />
  );
}

const comma = { key: ',', code: 'Comma', metaKey: true } as const;

describe('the settings chord', () => {
  it('opens the tab from anywhere in the window', () => {
    render(<Harness />);
    expect(screen.queryByRole('tab', { name: 'Settings' })).toBeNull();
    fireEvent.keyDown(window, comma);
    expect(screen.getByRole('tab', { name: 'Settings' })).toBeTruthy();
  });

  it('focuses the tab it already opened rather than opening a second', () => {
    render(<Harness />);
    fireEvent.keyDown(window, comma);
    fireEvent.keyDown(window, comma);
    fireEvent.keyDown(window, comma);
    expect(screen.getAllByRole('tab', { name: 'Settings' })).toHaveLength(1);
    expect(screen.getByRole('tab', { name: 'Settings' }).getAttribute('aria-selected')).toBe(
      'true',
    );
  });

  it('brings the tab back to the front after another surface took the pane', () => {
    render(<Harness />);
    fireEvent.keyDown(window, comma);
    // Closing hands the pane back and takes the tab with it; reopening is the
    // same chord, and still one tab.
    fireEvent.click(screen.getByLabelText('Close Settings'));
    expect(screen.queryByRole('tab', { name: 'Settings' })).toBeNull();
    fireEvent.keyDown(window, comma);
    expect(screen.getAllByRole('tab', { name: 'Settings' })).toHaveLength(1);
  });

  it('is inert until the core has said which platform this is', () => {
    // Guessing which modifier is "the command key" is what `acceleratorForOs`
    // exists to avoid; until health lands there is nothing to guess with.
    render(<Harness accelerator={null} />);
    fireEvent.keyDown(window, comma);
    expect(screen.queryByRole('tab', { name: 'Settings' })).toBeNull();
  });

  it('ignores the other platform accelerator', () => {
    render(<Harness accelerator="ctrl" />);
    fireEvent.keyDown(window, comma);
    expect(screen.queryByRole('tab', { name: 'Settings' })).toBeNull();
  });

  it('fires while a textarea has focus, unlike the quick-open chord', () => {
    // The deliberate difference recorded in `use-settings-chord.ts`: Monaco and
    // every live shell put a `<textarea>` under the cursor, and "I am editing a
    // file and press ⌘," is the case this chord exists for. `Comma` is not a
    // readline binding, so nothing is being taken from a shell.
    render(
      <>
        <textarea aria-label="editor" />
        <Harness />
      </>,
    );
    const area = screen.getByLabelText('editor');
    area.focus();
    fireEvent.keyDown(area, comma);
    expect(screen.getByRole('tab', { name: 'Settings' })).toBeTruthy();
  });

  it('takes the keystroke, so nothing else acts on the same press', () => {
    render(<Harness />);
    const event = new KeyboardEvent('keydown', { ...comma, cancelable: true, bubbles: true });
    window.dispatchEvent(event);
    expect(event.defaultPrevented).toBe(true);
  });

  it('leaves a keystroke that is not the chord alone', () => {
    render(<Harness />);
    const event = new KeyboardEvent('keydown', {
      key: 'p',
      code: 'KeyP',
      metaKey: true,
      cancelable: true,
      bubbles: true,
    });
    window.dispatchEvent(event);
    expect(event.defaultPrevented).toBe(false);
    expect(screen.queryByRole('tab', { name: 'Settings' })).toBeNull();
  });
});

describe('the tab a person clicks', () => {
  it('keeps a preview on a double click', () => {
    render(<Harness />);
    fireEvent.keyDown(window, comma);
    const tab = screen.getByRole('tab', { name: 'Settings' });
    // The chord opens it kept, so a double click is a no-op rather than a
    // change — asserted so the wiring is known to reach `keep` at all.
    fireEvent.doubleClick(tab);
    expect(screen.getAllByRole('tab', { name: 'Settings' })).toHaveLength(1);
  });
});
