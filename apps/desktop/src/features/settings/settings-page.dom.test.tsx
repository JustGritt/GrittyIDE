/** @vitest-environment jsdom */
import type {
  JsonValue,
  SettingSpecWire,
  SettingsSchema,
  SettingsSnapshot,
} from '@gritty/protocol';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { useState } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { SettingsPage } from './settings-page';
import type { SettingsController } from './use-settings';

/**
 * The settings page as a person actually meets it (T120-U §9).
 *
 * A DOM rather than static markup, because what is asserted here does not exist
 * in a string: *when* a number box commits, whether a refusal puts the old
 * value back, what typing in the search does to the rail, and where the arrow
 * keys go. The shape of the markup is `settings-page-render.test.ts`'s.
 */

vi.mock('../../core/client', () => ({
  core: {
    terminal: { profiles: () => Promise.resolve([]) },
    agents: {},
    settings: {},
  },
}));

afterEach(cleanup);

function spec(overrides: Partial<SettingSpecWire> & { key: string }): SettingSpecWire {
  return {
    section: overrides.key.split('.')[0] ?? 'editor',
    label: overrides.key,
    description: 'A setting.',
    kind: { type: 'bool' },
    default: false,
    scope: 'USER',
    applies: 'LIVE',
    ...overrides,
  };
}

const WORD_WRAP = spec({
  key: 'editor.wordWrap',
  label: 'Word wrap',
  description: 'Wrap long lines at the width of the editor.',
});
const FONT_SIZE = spec({
  key: 'editor.fontSize',
  label: 'Font size',
  description: 'Font size in the editor, in pixels.',
  kind: { type: 'integer', min: 8, max: 32 },
  default: 13,
});
const SHELL = spec({
  key: 'terminal.shell',
  section: 'terminal',
  label: 'Shell',
  description: 'Absolute path of the shell new terminals start.',
  kind: { type: 'nullablePath' },
  default: null,
});

const SCHEMA: SettingsSchema = {
  sections: [
    { id: 'editor', label: 'Editor' },
    { id: 'terminal', label: 'Terminal' },
  ],
  settings: [WORD_WRAP, FONT_SIZE, SHELL],
};

function snapshot(overrides: Partial<SettingsSnapshot> = {}): SettingsSnapshot {
  return {
    values: { 'editor.wordWrap': false, 'editor.fontSize': 13, 'terminal.shell': null },
    explicit: [],
    modified: [],
    invalid: [],
    unknown: [],
    diagnostics: [],
    storage: { writable: true, path: '/home/a/settings.json' },
    ...overrides,
  };
}

function controller(overrides: Partial<SettingsController> = {}): SettingsController {
  return {
    schema: SCHEMA,
    snapshot: snapshot(),
    loaded: true,
    error: null,
    clearError: () => {},
    busy: false,
    set: async () => true,
    reset: async () => true,
    reveal: async () => {},
    notifications: null,
    setNotifications: async () => true,
    ...overrides,
  };
}

function show(overrides: Partial<SettingsController> = {}) {
  render(<SettingsPage controller={controller(overrides)} />);
}

/**
 * A controller that converges the way the real one does: a write is answered
 * with a snapshot carrying it, and the page follows the snapshot.
 */
function LiveHarness() {
  const [held, setHeld] = useState(snapshot());
  return (
    <SettingsPage
      controller={controller({
        snapshot: held,
        set: async (key, value) => {
          setHeld((current) => ({
            ...current,
            values: { ...current.values, [key]: value },
            modified: [...new Set([...current.modified, key])],
          }));
          return true;
        },
      })}
    />
  );
}

describe('toggling', () => {
  it('writes the new state of the checkbox', async () => {
    const writes: [string, JsonValue][] = [];
    show({
      set: async (key, value) => {
        writes.push([key, value]);
        return true;
      },
    });
    fireEvent.click(screen.getByLabelText('Word wrap'));
    await waitFor(() => expect(writes).toEqual([['editor.wordWrap', true]]));
  });
});

describe('reset', () => {
  it('asks for this key alone, not for every modified one', () => {
    const resets: (readonly string[] | undefined)[] = [];
    show({
      snapshot: snapshot({ modified: ['editor.wordWrap', 'editor.fontSize'] }),
      reset: async (keys) => {
        resets.push(keys);
        return true;
      },
    });
    fireEvent.click(screen.getByLabelText('Reset Word wrap to its default'));
    expect(resets).toEqual([['editor.wordWrap']]);
  });
});

describe('a number box', () => {
  it('commits on blur, once, rather than on every keystroke', async () => {
    const writes: [string, JsonValue][] = [];
    show({
      set: async (key, value) => {
        writes.push([key, value]);
        return true;
      },
    });
    const box = screen.getByLabelText('Font size');
    fireEvent.change(box, { target: { value: '2' } });
    fireEvent.change(box, { target: { value: '20' } });
    // Nothing yet: `2` on the way to `20` is a value the core would have
    // accepted and written, and each write is an atomic file write.
    expect(writes).toEqual([]);
    fireEvent.blur(box);
    await waitFor(() => expect(writes).toEqual([['editor.fontSize', 20]]));
  });

  it('commits on Enter', async () => {
    const writes: [string, JsonValue][] = [];
    show({
      set: async (key, value) => {
        writes.push([key, value]);
        return true;
      },
    });
    const box = screen.getByLabelText('Font size');
    fireEvent.change(box, { target: { value: '18' } });
    fireEvent.keyDown(box, { key: 'Enter' });
    await waitFor(() => expect(writes).toEqual([['editor.fontSize', 18]]));
  });

  it('can be typed into at all', () => {
    // The regression this component exists for: a fully controlled input bound
    // to a value the core has not accepted yet reverts every keystroke, so the
    // box simply cannot be used.
    show();
    const box = screen.getByLabelText('Font size') as HTMLInputElement;
    fireEvent.change(box, { target: { value: '9' } });
    expect(box.value).toBe('9');
  });

  it('puts the stored value back when the core refuses', async () => {
    show({ set: async () => false });
    const box = screen.getByLabelText('Font size') as HTMLInputElement;
    fireEvent.change(box, { target: { value: '99' } });
    fireEvent.blur(box);
    // Nothing optimistic survives a refusal: 99 is not in force, so it is not
    // on screen either.
    await waitFor(() => expect(box.value).toBe('13'));
  });

  it('holds the draft while the write is in flight, rather than flashing the old value', async () => {
    // The frame between asking and being answered. Clearing the draft on the
    // way *into* the write puts the stored value back for that frame, so an
    // accepted change visibly reverts and then comes back.
    let settle: (accepted: boolean) => void = () => {};
    const pending = new Promise<boolean>((resolve) => {
      settle = resolve;
    });
    show({ set: () => pending });
    const box = screen.getByLabelText('Font size') as HTMLInputElement;
    fireEvent.change(box, { target: { value: '20' } });
    fireEvent.blur(box);
    expect(box.value).toBe('20');
    settle(true);
    await pending;
  });

  it('keeps an accepted value on screen once the snapshot carries it', async () => {
    // A live controller, because this is the handover the page actually does:
    // the draft is let go only when the value it wrote is the stored one.
    render(<LiveHarness />);
    const box = screen.getByLabelText('Font size') as HTMLInputElement;
    fireEvent.change(box, { target: { value: '20' } });
    fireEvent.blur(box);
    await waitFor(() => expect(box.value).toBe('20'));
  });

  it('abandons the draft on Escape without writing', () => {
    const writes: unknown[] = [];
    show({
      set: async (key, value) => {
        writes.push([key, value]);
        return true;
      },
    });
    const box = screen.getByLabelText('Font size') as HTMLInputElement;
    fireEvent.change(box, { target: { value: '31' } });
    fireEvent.keyDown(box, { key: 'Escape' });
    expect(box.value).toBe('13');
    expect(writes).toEqual([]);
  });

  it('writes nothing for text that is not a number', async () => {
    const writes: unknown[] = [];
    show({
      set: async (key, value) => {
        writes.push([key, value]);
        return true;
      },
    });
    const box = screen.getByLabelText('Font size') as HTMLInputElement;
    fireEvent.change(box, { target: { value: '' } });
    fireEvent.blur(box);
    await waitFor(() => expect(box.value).toBe('13'));
    expect(writes).toEqual([]);
  });
});

describe('a nullable text box', () => {
  it('sends null for an empty box, so the default applies again', async () => {
    const writes: [string, JsonValue][] = [];
    show({
      snapshot: snapshot({ values: { 'terminal.shell': '/bin/zsh' } }),
      set: async (key, value) => {
        writes.push([key, value]);
        return true;
      },
    });
    fireEvent.click(screen.getByRole('tab', { name: 'Terminal' }));
    const box = screen.getByLabelText('Shell');
    fireEvent.change(box, { target: { value: '  ' } });
    fireEvent.blur(box);
    await waitFor(() => expect(writes).toEqual([['terminal.shell', null]]));
  });
});

describe('search', () => {
  it('narrows the rows and drops a category it empties', () => {
    show();
    expect(screen.getByRole('tab', { name: /Terminal/ })).toBeTruthy();

    fireEvent.change(screen.getByLabelText('Search settings'), { target: { value: 'minimap' } });
    expect(screen.queryByRole('tablist')).toBeNull();
    expect(screen.getByText(/Nothing matches/)).toBeTruthy();
  });

  it('moves the selection to a category that still has something', () => {
    show();
    // Editor is selected; a search only Terminal matches must not leave an
    // empty panel under a highlighted rail.
    fireEvent.change(screen.getByLabelText('Search settings'), { target: { value: 'shell' } });
    expect(screen.getByRole('tab', { name: /Terminal/ }).getAttribute('aria-selected')).toBe(
      'true',
    );
    expect(screen.getByLabelText('Shell')).toBeTruthy();
    expect(screen.queryByLabelText('Word wrap')).toBeNull();
  });

  it('matches on the dotted key as well as the words', () => {
    show();
    fireEvent.change(screen.getByLabelText('Search settings'), {
      target: { value: 'editor.font' },
    });
    expect(screen.getByLabelText('Font size')).toBeTruthy();
    expect(screen.queryByLabelText('Word wrap')).toBeNull();
  });
});

describe('the category rail', () => {
  it('moves with the arrow keys', () => {
    show();
    const editor = screen.getByRole('tab', { name: 'Editor' });
    expect(editor.getAttribute('aria-selected')).toBe('true');

    fireEvent.keyDown(editor, { key: 'ArrowDown' });
    expect(screen.getByRole('tab', { name: 'Terminal' }).getAttribute('aria-selected')).toBe(
      'true',
    );
    expect(screen.getByLabelText('Shell')).toBeTruthy();

    fireEvent.keyDown(screen.getByRole('tab', { name: 'Terminal' }), { key: 'ArrowUp' });
    expect(screen.getByRole('tab', { name: 'Editor' }).getAttribute('aria-selected')).toBe('true');
  });

  it('stops at both ends rather than wrapping', () => {
    show();
    fireEvent.keyDown(screen.getByRole('tab', { name: 'Editor' }), { key: 'ArrowUp' });
    expect(screen.getByRole('tab', { name: 'Editor' }).getAttribute('aria-selected')).toBe('true');

    fireEvent.keyDown(screen.getByRole('tab', { name: 'Editor' }), { key: 'End' });
    expect(screen.getByRole('tab', { name: 'Terminal' }).getAttribute('aria-selected')).toBe(
      'true',
    );
    fireEvent.keyDown(screen.getByRole('tab', { name: 'Terminal' }), { key: 'ArrowDown' });
    expect(screen.getByRole('tab', { name: 'Terminal' }).getAttribute('aria-selected')).toBe(
      'true',
    );

    fireEvent.keyDown(screen.getByRole('tab', { name: 'Terminal' }), { key: 'Home' });
    expect(screen.getByRole('tab', { name: 'Editor' }).getAttribute('aria-selected')).toBe('true');
  });

  it('keeps exactly one tab stop, so the rail is one stop and not five', () => {
    show();
    expect(screen.getByRole('tab', { name: 'Editor' }).getAttribute('tabindex')).toBe('0');
    expect(screen.getByRole('tab', { name: 'Terminal' }).getAttribute('tabindex')).toBe('-1');
  });
});

describe('when the file cannot be written', () => {
  it('disables the controls rather than letting a change appear to take', () => {
    show({
      snapshot: snapshot({
        storage: { writable: false, path: '/home/a/settings.json', reason: 'Could not be parsed.' },
      }),
    });
    expect((screen.getByLabelText('Word wrap') as HTMLInputElement).disabled).toBe(true);
    expect((screen.getByLabelText('Font size') as HTMLInputElement).disabled).toBe(true);
    expect(screen.getAllByText('Could not be parsed.').length).toBeGreaterThan(0);
  });
});
