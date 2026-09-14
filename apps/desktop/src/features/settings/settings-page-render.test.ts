import type {
  AgentNotificationSettings,
  SettingSpecWire,
  SettingsSchema,
  SettingsSnapshot,
} from '@gritty/protocol';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it, vi } from 'vitest';
import { SettingsPage } from './settings-page';
import type { SettingsController } from './use-settings';

/**
 * What the settings page actually puts on the screen (T120-U §2).
 *
 * Static markup rather than a DOM, matching `run-console-render.test.ts` and
 * `agent-console-render.test.ts`: `apps/desktop`'s vitest project is a node
 * one, and a string is a stronger assertion than a query against a tree. The
 * half that genuinely needs a browser — typing, toggling, resetting, arrowing
 * down the rail — is `settings-page.dom.test.tsx`.
 */

// The page's fields reach `core` for the shell list; nothing here should call
// it, and a module-level import must not need a Tauri window to load.
vi.mock('../../core/client', () => ({
  core: {
    terminal: { profiles: () => Promise.resolve([]) },
    agents: {},
    settings: {},
  },
}));

const refuse = (what: string) => () => {
  throw new Error(`nothing should ${what} while rendering`);
};

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

const EDITOR = spec({
  key: 'editor.wordWrap',
  label: 'Word wrap',
  description: 'Wrap long lines at the width of the editor.',
});

const TERMINAL_SHELL = spec({
  key: 'terminal.shell',
  section: 'terminal',
  label: 'Shell',
  description: 'Absolute path of the shell new terminals start.',
  kind: { type: 'nullablePath' },
  default: null,
  applies: 'NEW_TERMINALS',
});

function schema(settings: SettingSpecWire[], sections = SECTIONS): SettingsSchema {
  return { sections, settings };
}

const SECTIONS = [
  { id: 'editor', label: 'Editor' },
  { id: 'terminal', label: 'Terminal' },
];

function snapshot(overrides: Partial<SettingsSnapshot> = {}): SettingsSnapshot {
  return {
    values: {},
    explicit: [],
    modified: [],
    invalid: [],
    unknown: [],
    diagnostics: [],
    storage: { writable: true, path: '/home/a/.local/share/gritty/settings.json' },
    ...overrides,
  };
}

function controller(overrides: Partial<SettingsController> = {}): SettingsController {
  const held: SettingsController = {
    schema: schema([EDITOR, TERMINAL_SHELL]),
    snapshot: snapshot(),
    loaded: true,
    error: null,
    clearError: () => {},
    busy: false,
    set: refuse('write a setting'),
    reset: refuse('reset a setting'),
    reveal: refuse('reveal the file'),
    notifications: null,
    setNotifications: refuse('touch the platform'),
    ...overrides,
  };
  return held;
}

function markup(overrides: Partial<SettingsController> = {}): string {
  return renderToStaticMarkup(createElement(SettingsPage, { controller: controller(overrides) }));
}

describe('SettingsPage markup', () => {
  it('draws a rail of the registry sections and the first one selected', () => {
    const html = markup();
    expect(html).toContain('role="tablist"');
    expect(html).toContain('>Editor<');
    expect(html).toContain('>Terminal<');
    expect(html).toContain('aria-selected="true"');
  });

  it('draws the rows of the selected section only', () => {
    const html = markup();
    expect(html).toContain('Word wrap');
    expect(html).toContain('editor.wordWrap');
    expect(html).toContain('Wrap long lines at the width of the editor.');
    // The Terminal category is in the rail but its rows are not on screen.
    expect(html).not.toContain('Absolute path of the shell new terminals start.');
  });

  it('names the file and offers to reveal it', () => {
    const html = markup();
    expect(html).toContain('/home/a/.local/share/gritty/settings.json');
    expect(html).toContain('>Reveal<');
  });

  it('says there is no file when there is no data directory', () => {
    const html = markup({
      snapshot: snapshot({ storage: { writable: false, reason: 'No data directory.' } }),
    });
    expect(html).toContain('there is no settings file');
    expect(html).toContain('No data directory.');
  });

  it('marks a modified row and offers a reset for it, and only for it', () => {
    const html = markup({
      schema: schema([EDITOR, spec({ key: 'editor.minimap', label: 'Minimap', default: true })]),
      snapshot: snapshot({ modified: ['editor.wordWrap'], values: { 'editor.wordWrap': true } }),
    });
    expect(html).toContain('settings__modified');
    expect(html).toContain('aria-label="Reset Word wrap to its default"');
    expect(html).not.toContain('aria-label="Reset Minimap to its default"');
  });

  it('renders the applies hint only where a change is not immediate', () => {
    const html = markup({
      schema: schema([TERMINAL_SHELL], [{ id: 'terminal', label: 'Terminal' }]),
    });
    expect(html).toContain('Terminals already open keep the old value.');

    expect(markup()).not.toContain('Terminals already open');
  });

  it('disables every control and says why when the file cannot be written', () => {
    const html = markup({
      snapshot: snapshot({
        storage: {
          writable: false,
          path: '/home/a/settings.json',
          reason: 'settings.json could not be parsed. Fix line 3, then reopen Gritty.',
        },
      }),
    });
    expect(html).toContain('disabled=""');
    expect(html).toContain('Fix line 3, then reopen Gritty.');
  });

  it('says a refused value is not in force, rather than drawing it as a change', () => {
    const html = markup({
      snapshot: snapshot({
        invalid: ['editor.wordWrap'],
        diagnostics: [{ key: 'editor.wordWrap', message: 'expected true or false' }],
      }),
    });
    expect(html).toContain('the default is in force');
    expect(html).toContain('expected true or false');
    expect(html).not.toContain('settings__modified');
  });

  it('puts a parse position on a file-wide diagnostic', () => {
    const html = markup({
      snapshot: snapshot({
        diagnostics: [{ message: 'settings.json could not be parsed', line: 3, column: 5 }],
      }),
    });
    expect(html).toContain('settings.json could not be parsed (line 3:5)');
  });

  it('reports keys it does not declare as kept rather than as an error', () => {
    const html = markup({ snapshot: snapshot({ unknown: ['editor.futureThing'] }) });
    expect(html).toContain('kept exactly as it is');
  });

  it('shows a setting it cannot draw as its stored value rather than dropping it', () => {
    const future = spec({
      key: 'editor.accent',
      label: 'Accent',
      kind: { type: 'colour' } as unknown as SettingSpecWire['kind'],
      default: 'blue',
    });
    const html = markup({
      schema: schema([future], [{ id: 'editor', label: 'Editor' }]),
      snapshot: snapshot({ values: { 'editor.accent': 'teal' } }),
    });
    expect(html).toContain('settings__raw');
    expect(html).toContain('teal');
    expect(html).toContain('does not know how to draw');
  });

  it('draws no notification row at all on a build that cannot raise one', () => {
    const notifications: AgentNotificationSettings = {
      enabled: false,
      supported: false,
      permission: 'DENIED',
    };
    const agents = spec({
      key: 'agents.notifications',
      section: 'agents',
      label: 'Notify when an agent needs you',
    });
    const html = markup({
      schema: schema([EDITOR, agents], [...SECTIONS, { id: 'agents', label: 'Agents' }]),
      notifications,
    });
    // Not disabled, not explained: absent. A greyed-out switch would imply a
    // permission no amount of clicking can obtain.
    expect(html).not.toContain('Notify when an agent needs you');
    expect(html).not.toContain('>Agents<');
  });

  it('puts the integration flow on the Agents category, read by nobody yet', () => {
    const agents = spec({
      key: 'agents.notifications',
      section: 'agents',
      label: 'Notify when an agent needs you',
    });
    const html = markup({
      schema: schema([agents], [{ id: 'agents', label: 'Agents' }]),
      notifications: { enabled: false, supported: true, permission: 'PROMPT' },
    });
    expect(html).toContain('Claude Code integration');
    // ADR-018 §1: nothing is read until a person presses the button. A page
    // section is visible by default, so mounting must not be the trigger.
    expect(html).toContain('Not read yet');
    expect(html).toContain('>Check<');
  });

  it('says it is still reading before the schema lands', () => {
    expect(markup({ schema: null, snapshot: null, loaded: false })).toContain(
      'Reading your settings…',
    );
  });

  it('says what matched nothing, rather than showing an empty rail', () => {
    // The query is component state, so this is the empty-registry half of the
    // same branch; the typed-search half is the DOM test's.
    const html = markup({ schema: schema([], []) });
    expect(html).toContain('This build declares no settings.');
    expect(html).not.toContain('role="tablist"');
  });

  it('puts every string in a text node and never into markup', () => {
    // The house scan: settings values and diagnostics are file content, which
    // is untrusted input (hard rule 13). Attribute values are blanked first,
    // because escaped text inside one is the *safe* outcome.
    const html = markup({
      snapshot: snapshot({
        values: { 'editor.wordWrap': true },
        diagnostics: [{ message: '<img src=x onerror=alert(1)>' }],
        unknown: ['<script>'],
      }),
    });
    expect(html).not.toContain('<img');
    expect(html).not.toContain('<script');
    expect(html).toContain('&lt;img');
  });
});
