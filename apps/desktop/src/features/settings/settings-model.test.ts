import type {
  AgentNotificationSettings,
  SettingSpecWire,
  SettingsSchema,
  SettingsSnapshot,
  SettingsStorage,
} from '@gritty/protocol';
import { describe, expect, it } from 'vitest';
import {
  appliesHint,
  availabilityOf,
  matchesQuery,
  type SettingsEnvironment,
  settingsView,
  widgetFor,
} from './settings-model';

/**
 * The settings page as data (T120-U §1).
 *
 * Pure throughout, so every branch is reachable without React, a core or a
 * DOM: which control a kind produces, what is marked as changed, what a search
 * keeps, and — the one that matters most — that every variant of the
 * availability union is reachable and carries a sentence.
 */

const WRITABLE: SettingsStorage = { writable: true, path: '/home/a/settings.json' };

function spec(overrides: Partial<SettingSpecWire> = {}): SettingSpecWire {
  return {
    key: 'editor.wordWrap',
    section: 'editor',
    label: 'Word wrap',
    description: 'Wrap long lines at the width of the editor.',
    kind: { type: 'bool' },
    default: false,
    scope: 'USER',
    applies: 'LIVE',
    ...overrides,
  };
}

function schema(settings: SettingSpecWire[]): SettingsSchema {
  return {
    sections: [
      { id: 'editor', label: 'Editor' },
      { id: 'terminal', label: 'Terminal' },
    ],
    settings,
  };
}

function snapshot(overrides: Partial<SettingsSnapshot> = {}): SettingsSnapshot {
  return {
    values: {},
    explicit: [],
    modified: [],
    invalid: [],
    unknown: [],
    diagnostics: [],
    storage: WRITABLE,
    ...overrides,
  };
}

function env(overrides: Partial<SettingsEnvironment> = {}): SettingsEnvironment {
  return { storage: WRITABLE, notifications: null, ...overrides };
}

const supported: AgentNotificationSettings = {
  enabled: false,
  supported: true,
  permission: 'PROMPT',
};

describe('widgetFor', () => {
  it('maps every kind the registry can declare to a control', () => {
    expect(widgetFor(spec({ kind: { type: 'bool' } }))).toEqual({ kind: 'toggle' });
    expect(widgetFor(spec({ kind: { type: 'integer', min: 8, max: 32 } }))).toEqual({
      kind: 'number',
      min: 8,
      max: 32,
    });
    expect(
      widgetFor(spec({ kind: { type: 'enum', choices: [{ value: '2', label: '2 spaces' }] } })),
    ).toEqual({ kind: 'select', choices: [{ value: '2', label: '2 spaces' }] });
    expect(widgetFor(spec({ kind: { type: 'nullableString', max_len: 200 } }))).toEqual({
      kind: 'text',
      maxLength: 200,
      path: false,
    });
    expect(widgetFor(spec({ kind: { type: 'nullablePath' } }))).toEqual({
      kind: 'text',
      maxLength: null,
      path: true,
    });
    expect(widgetFor(spec({ kind: { type: 'pathList', max_len: 16 } }))).toEqual({
      kind: 'list',
      maxLength: 16,
    });
  });

  it('takes the bounds off the wire rather than from a constant', () => {
    // The point of the whole exercise: a copy that disagrees with Rust by one
    // lets the page offer a value the core refuses, and `settings.set` is
    // atomic, so that refusal fails the whole patch.
    const widget = widgetFor(spec({ kind: { type: 'integer', min: 1, max: 3 } }));
    expect(widget).toEqual({ kind: 'number', min: 1, max: 3 });
  });

  it('gives a key with a bespoke control the custom widget, not its kind', () => {
    // `agents.notifications` is a bool and would otherwise be a toggle that
    // writes the file and arms nothing until the next launch.
    expect(widgetFor(spec({ key: 'agents.notifications', kind: { type: 'bool' } }))).toEqual({
      kind: 'custom',
      control: 'notifications',
    });
  });

  it('falls back to raw for a kind from a newer core', () => {
    const future = spec({ kind: { type: 'colour' } as unknown as SettingSpecWire['kind'] });
    expect(widgetFor(future)).toEqual({ kind: 'raw' });
  });
});

describe('availabilityOf', () => {
  it('is ready when the file can be written', () => {
    const target = spec();
    expect(availabilityOf(target, widgetFor(target), env())).toEqual({ kind: 'ready' });
  });

  it('is read-only with the core reason when storage refuses writes', () => {
    const target = spec();
    const storage: SettingsStorage = {
      writable: false,
      path: '/home/a/settings.json',
      reason: 'settings.json could not be parsed. Fix line 3 and reopen Gritty.',
    };
    expect(availabilityOf(target, widgetFor(target), env({ storage }))).toEqual({
      kind: 'readonly',
      reason: storage.reason,
    });
  });

  it('still names a reason when the core sends none', () => {
    // Would be a core bug. Saying something true beats a disabled control with
    // no explanation, which is the default nobody chose.
    const target = spec();
    const availability = availabilityOf(
      target,
      widgetFor(target),
      env({ storage: { writable: false } }),
    );
    expect(availability.kind).toBe('readonly');
    expect(availability.kind === 'readonly' && availability.reason.length).toBeGreaterThan(0);
  });

  it('is read-only for a kind it cannot draw, naming the kind', () => {
    const future = spec({ kind: { type: 'colour' } as unknown as SettingSpecWire['kind'] });
    const availability = availabilityOf(future, widgetFor(future), env());
    expect(availability.kind).toBe('readonly');
    expect(availability.kind === 'readonly' && availability.reason).toContain('colour');
  });

  it('is absent when the platform cannot raise notifications at all', () => {
    // The protocol defines `supported: false` as the setting being absent, not
    // broken. A greyed-out switch would imply a permission that no amount of
    // clicking can obtain.
    const target = spec({ key: 'agents.notifications' });
    const notifications = { ...supported, supported: false, permission: 'DENIED' as const };
    expect(availabilityOf(target, widgetFor(target), env({ notifications }))).toEqual({
      kind: 'absent',
    });
  });

  it('says it is still asking, rather than guessing, before the platform answers', () => {
    const target = spec({ key: 'agents.notifications' });
    const availability = availabilityOf(target, widgetFor(target), env({ notifications: null }));
    expect(availability.kind).toBe('readonly');
  });

  it('is ready once the platform says it can', () => {
    const target = spec({ key: 'agents.notifications' });
    expect(availabilityOf(target, widgetFor(target), env({ notifications: supported }))).toEqual({
      kind: 'ready',
    });
  });

  it('lets storage beat everything, including a control that could not be drawn', () => {
    const future = spec({ kind: { type: 'colour' } as unknown as SettingSpecWire['kind'] });
    const availability = availabilityOf(
      future,
      widgetFor(future),
      env({ storage: { writable: false, reason: 'No application data directory.' } }),
    );
    expect(availability).toEqual({ kind: 'readonly', reason: 'No application data directory.' });
  });
});

describe('matchesQuery', () => {
  const target = spec({
    key: 'terminal.fontFamily',
    label: 'Font family',
    description: 'Font stack for the terminal.',
  });

  it('matches nothing typed', () => {
    expect(matchesQuery(target, '')).toBe(true);
    expect(matchesQuery(target, '   ')).toBe(true);
  });

  it('matches on the key, the label and the description', () => {
    expect(matchesQuery(target, 'fontFamily')).toBe(true);
    expect(matchesQuery(target, 'family')).toBe(true);
    expect(matchesQuery(target, 'stack')).toBe(true);
  });

  it('ignores case', () => {
    expect(matchesQuery(target, 'FONT')).toBe(true);
  });

  it('narrows on every term, so two words are an AND', () => {
    expect(matchesQuery(target, 'terminal font')).toBe(true);
    expect(matchesQuery(target, 'terminal minimap')).toBe(false);
  });
});

describe('settingsView', () => {
  const wordWrap = spec();
  const fontSize = spec({
    key: 'editor.fontSize',
    label: 'Font size',
    description: 'Font size in the editor, in pixels.',
    kind: { type: 'integer', min: 8, max: 32 },
    default: 13,
  });
  const shell = spec({
    key: 'terminal.shell',
    section: 'terminal',
    label: 'Shell',
    description: 'Absolute path of the shell new terminals start.',
    kind: { type: 'nullablePath' },
    default: null,
    applies: 'NEW_TERMINALS',
  });

  it('groups rows under the registry sections, in the registry order', () => {
    const view = settingsView(schema([shell, wordWrap, fontSize]), snapshot(), env());
    expect(view.sections.map((entry) => entry.section.id)).toEqual(['editor', 'terminal']);
    expect(view.sections[0]?.rows.map((row) => row.spec.key)).toEqual([
      'editor.wordWrap',
      'editor.fontSize',
    ]);
    expect(view.total).toBe(3);
  });

  it('marks what the core says is modified, and nothing else', () => {
    const view = settingsView(
      schema([wordWrap, fontSize]),
      snapshot({ modified: ['editor.fontSize'], values: { 'editor.fontSize': 16 } }),
      env(),
    );
    const rows = view.sections[0]?.rows ?? [];
    expect(rows.map((row) => [row.spec.key, row.modified])).toEqual([
      ['editor.wordWrap', false],
      ['editor.fontSize', true],
    ]);
    expect(view.modified).toEqual(['editor.fontSize']);
  });

  it('shows the effective value, falling back to the default when the core omits one', () => {
    const view = settingsView(
      schema([wordWrap, fontSize]),
      snapshot({ values: { 'editor.wordWrap': true } }),
      env(),
    );
    const rows = view.sections[0]?.rows ?? [];
    expect(rows[0]?.value).toBe(true);
    expect(rows[1]?.value).toBe(13);
  });

  it('keeps a null value rather than reading it as absent', () => {
    // `terminal.shell` is legitimately null. Falling back to the default for it
    // would be harmless here and wrong the day a default is not null.
    const view = settingsView(
      schema([shell]),
      snapshot({ values: { 'terminal.shell': null } }),
      env(),
    );
    expect(view.sections[0]?.rows[0]?.value).toBeNull();
  });

  it('drops a section a search empties, and reports nothing found', () => {
    const view = settingsView(schema([wordWrap, fontSize, shell]), snapshot(), env(), 'shell');
    expect(view.sections.map((entry) => entry.section.id)).toEqual(['terminal']);
    expect(view.total).toBe(1);

    const nothing = settingsView(schema([wordWrap]), snapshot(), env(), 'zzz');
    expect(nothing.sections).toEqual([]);
    expect(nothing.total).toBe(0);
  });

  it('drops an absent row entirely rather than drawing it disabled', () => {
    const notifications = spec({ key: 'agents.notifications', section: 'editor' });
    const view = settingsView(
      schema([wordWrap, notifications]),
      snapshot(),
      env({ notifications: { ...supported, supported: false } }),
    );
    expect(view.sections[0]?.rows.map((row) => row.spec.key)).toEqual(['editor.wordWrap']);
    expect(view.total).toBe(1);
  });

  it('carries the invalid flag, which is not the same as modified', () => {
    // The file sets it and the build refused it, so the default is in force.
    // Drawing it as changed would say a value is applied when it is not.
    const view = settingsView(
      schema([fontSize]),
      snapshot({ invalid: ['editor.fontSize'], modified: [] }),
      env(),
    );
    const row = view.sections[0]?.rows[0];
    expect(row?.invalid).toBe(true);
    expect(row?.modified).toBe(false);
  });

  it('splits diagnostics into the ones about a key and the ones about the file', () => {
    const view = settingsView(
      schema([fontSize]),
      snapshot({
        diagnostics: [
          { key: 'editor.fontSize', message: 'not a whole number between 8 and 32' },
          { message: 'settings.json could not be parsed', line: 3, column: 5 },
        ],
      }),
      env(),
    );
    expect(view.sections[0]?.rows[0]?.diagnostics).toHaveLength(1);
    expect(view.fileDiagnostics.map((entry) => entry.message)).toEqual([
      'settings.json could not be parsed',
    ]);
  });

  it('drops a setting naming a section that does not exist, rather than inventing one', () => {
    const orphan = spec({ key: 'ghost.thing', section: 'ghost' });
    const view = settingsView(schema([wordWrap, orphan]), snapshot(), env());
    expect(view.sections.map((entry) => entry.section.id)).toEqual(['editor']);
    // And the count agrees with what is drawn. A total that included the
    // dropped row would make the page say it is showing something it is not.
    expect(view.total).toBe(1);
  });
});

describe('appliesHint', () => {
  it('says nothing when a change is simply in force', () => {
    expect(appliesHint(spec({ applies: 'LIVE' }))).toBeNull();
  });

  it('says what a terminal setting does not reach', () => {
    expect(appliesHint(spec({ applies: 'NEW_TERMINALS' }))).toContain('already open');
  });

  it('refuses to claim immediacy for a variant it does not know', () => {
    const future = spec({ applies: 'ON_RESTART' as SettingSpecWire['applies'] });
    expect(appliesHint(future)).not.toBeNull();
  });
});
