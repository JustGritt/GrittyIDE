import type { SettingSpecWire, SettingsSchema, SettingsSnapshot } from '@gritty/protocol';
import { describe, expect, it } from 'vitest';
import { planSeed, type SeedSource } from './settings-seed';

/**
 * The one-time move of today's values into `settings.json` (T120-U §7,
 * ADR-031 §8).
 *
 * Pure, and worth testing hard: it runs exactly once per user, unattended, and
 * every one of its skip rules exists because the obvious implementation of it
 * loses something — a value the user typed by hand, a default they never chose
 * but would be pinned to, or the whole patch to one stale field.
 */

/** The registry's v1 set, near enough for the fields the seeder translates. */
const SCHEMA: SettingsSchema = {
  sections: [
    { id: 'editor', label: 'Editor' },
    { id: 'terminal', label: 'Terminal' },
  ],
  settings: [
    wire({ key: 'editor.fontSize', kind: { type: 'integer', min: 8, max: 32 }, default: 13 }),
    wire({
      key: 'editor.tabSize',
      kind: {
        type: 'enum',
        choices: [
          { value: '2', label: '2 spaces' },
          { value: '4', label: '4 spaces' },
          { value: '8', label: '8 spaces' },
        ],
      },
      default: '4',
    }),
    wire({ key: 'editor.wordWrap', kind: { type: 'bool' }, default: false }),
    wire({ key: 'editor.minimap', kind: { type: 'bool' }, default: true }),
    wire({ key: 'terminal.shell', kind: { type: 'nullablePath' }, default: null }),
    wire({ key: 'terminal.customShells', kind: { type: 'pathList', max_len: 16 }, default: [] }),
    wire({
      key: 'terminal.fontFamily',
      kind: { type: 'nullableString', max_len: 200 },
      default: null,
    }),
    wire({ key: 'terminal.fontSize', kind: { type: 'integer', min: 8, max: 32 }, default: 13 }),
  ],
};

function wire(overrides: Partial<SettingSpecWire> & { key: string }): SettingSpecWire {
  return {
    section: overrides.key.split('.')[0] ?? 'editor',
    label: overrides.key,
    description: '',
    kind: { type: 'bool' },
    default: false,
    scope: 'USER',
    applies: 'LIVE',
    ...overrides,
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
    storage: { writable: true, path: '/home/a/settings.json' },
    ...overrides,
  };
}

/** A blob shaped like `serializeEditorPreferences` / `serializeLayout` write it. */
function source(overrides: Partial<SeedSource> = {}): SeedSource {
  return {
    preferences: { version: 1, fontSize: 16, tabSize: 2, wordWrap: true, minimap: false },
    layout: {
      version: 6,
      explorer: { showHidden: false },
      terminal: {
        profilePath: '/opt/homebrew/bin/fish',
        customShells: ['/opt/homebrew/bin/fish'],
        fontFamily: 'Berkeley Mono',
        fontSize: 15,
      },
      agents: { open: true },
      panes: { sidebarWidth: 300, terminalHeight: 280 },
    },
    ...overrides,
  };
}

describe('planSeed', () => {
  it('translates both blobs into one patch', () => {
    expect(planSeed(SCHEMA, snapshot(), source())).toEqual({
      'editor.fontSize': 16,
      // The blob held a number; the registry models tab size as an enum of
      // strings. This is the one field that is translated rather than copied.
      'editor.tabSize': '2',
      'editor.wordWrap': true,
      'editor.minimap': false,
      'terminal.shell': '/opt/homebrew/bin/fish',
      'terminal.customShells': ['/opt/homebrew/bin/fish'],
      'terminal.fontFamily': 'Berkeley Mono',
      'terminal.fontSize': 15,
    });
  });

  it('seeds nothing when the restore returned no blob', () => {
    // Not the same as a blob full of defaults, which is why the planner reads
    // the raw value rather than a parser's output: a parser cannot tell the
    // two apart, and seeding defaults is what pins a user forever.
    expect(planSeed(SCHEMA, snapshot(), { preferences: null, layout: undefined })).toEqual({});
  });

  it('skips a key the file already sets', () => {
    const patch = planSeed(
      SCHEMA,
      snapshot({ explicit: ['editor.fontSize', 'terminal.fontSize'] }),
      source(),
    );
    expect(patch).not.toHaveProperty('editor.fontSize');
    expect(patch).not.toHaveProperty('terminal.fontSize');
    expect(patch).toHaveProperty('editor.tabSize');
  });

  it('skips a key the file sets and the build refused', () => {
    // `invalid` means the file names it. Overwriting it would delete what the
    // user was in the middle of fixing, which is worse than leaving it wrong.
    const patch = planSeed(SCHEMA, snapshot({ invalid: ['editor.fontSize'] }), source());
    expect(patch).not.toHaveProperty('editor.fontSize');
  });

  it('skips a field whose old value equals the schema default', () => {
    // Writing it would freeze today's default into their file, where a later
    // build could never change it.
    const patch = planSeed(
      SCHEMA,
      snapshot(),
      source({
        preferences: { version: 1, fontSize: 13, tabSize: 4, wordWrap: false, minimap: true },
      }),
    );
    expect(patch).not.toHaveProperty('editor.fontSize');
    expect(patch).not.toHaveProperty('editor.tabSize');
    expect(patch).not.toHaveProperty('editor.wordWrap');
    expect(patch).not.toHaveProperty('editor.minimap');
  });

  it('skips a default-valued nullable and a default-valued empty list', () => {
    const patch = planSeed(
      SCHEMA,
      snapshot(),
      source({
        preferences: null,
        layout: {
          terminal: { profilePath: null, customShells: [], fontFamily: null, fontSize: null },
        },
      }),
    );
    expect(patch).toEqual({});
  });

  it('skips an old value the core would refuse, keeping the rest of the patch', () => {
    // The whole point of validating here at all: `settings.set` is atomic, so
    // one stale field would otherwise cost every good one.
    const patch = planSeed(
      SCHEMA,
      snapshot(),
      source({
        // 99 is outside the schema's 8..32, and a tab width of 3 is not one of
        // the enum's choices. Both are reachable from an older UI.
        preferences: { version: 1, fontSize: 99, tabSize: 3, wordWrap: true, minimap: false },
      }),
    );
    expect(patch).not.toHaveProperty('editor.fontSize');
    expect(patch).not.toHaveProperty('editor.tabSize');
    expect(patch['editor.wordWrap']).toBe(true);
    expect(patch['editor.minimap']).toBe(false);
  });

  it('skips a relative path, which the core requires to be absolute', () => {
    const patch = planSeed(
      SCHEMA,
      snapshot(),
      source({
        preferences: null,
        layout: {
          terminal: {
            profilePath: 'bin/fish',
            customShells: ['/bin/zsh', 'fish'],
            fontFamily: null,
            fontSize: null,
          },
        },
      }),
    );
    expect(patch).not.toHaveProperty('terminal.shell');
    // The list is all-or-nothing, exactly as the core validates it: one bad
    // entry refuses the value, and silently dropping it would seed a list the
    // user never had.
    expect(patch).not.toHaveProperty('terminal.customShells');
  });

  it('skips a blob field of the wrong type instead of throwing', () => {
    const patch = planSeed(
      SCHEMA,
      snapshot(),
      source({
        preferences: { fontSize: '16', tabSize: null, wordWrap: 'yes', minimap: false },
        layout: null,
      }),
    );
    expect(patch).toEqual({ 'editor.minimap': false });
  });

  it('survives a blob that is not an object at all', () => {
    expect(planSeed(SCHEMA, snapshot(), { preferences: 7, layout: ['nope'] })).toEqual({});
  });

  it('ignores a field this build no longer declares', () => {
    const trimmed: SettingsSchema = {
      ...SCHEMA,
      settings: SCHEMA.settings.filter((spec) => spec.key !== 'editor.fontSize'),
    };
    expect(planSeed(trimmed, snapshot(), source())).not.toHaveProperty('editor.fontSize');
  });

  it('never seeds a setting with no predecessor', () => {
    // `workbench.theme` keyed off the operating system and was never stored;
    // `agents.notifications` lived in memory. Neither has an old value, and
    // inventing one would be choosing for the user.
    const withTheme: SettingsSchema = {
      sections: [...SCHEMA.sections, { id: 'workbench', label: 'Workbench' }],
      settings: [
        ...SCHEMA.settings,
        wire({
          key: 'workbench.theme',
          section: 'workbench',
          kind: { type: 'enum', choices: [{ value: 'system', label: 'Follow the system' }] },
          default: 'system',
        }),
      ],
    };
    expect(planSeed(withTheme, snapshot(), source())).not.toHaveProperty('workbench.theme');
  });
});
