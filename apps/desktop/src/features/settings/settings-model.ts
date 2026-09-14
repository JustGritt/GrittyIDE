import type {
  AgentNotificationSettings,
  JsonValue,
  SettingChoice,
  SettingDiagnostic,
  SettingSection,
  SettingSpecWire,
  SettingsSchema,
  SettingsSnapshot,
  SettingsStorage,
} from '@gritty/protocol';

/**
 * The settings page, as data (T120-U, ADR-031 §3).
 *
 * The registry in `gritty-settings` is the single source of truth, so nothing
 * here knows which settings exist: this turns a `SettingsSchema` and a
 * `SettingsSnapshot` into rows, and `settings-page.tsx` draws whatever it is
 * handed. A setting added to the Rust table gets a working control without a
 * line changing in this file — which is the whole reason ADR-031 §3 accepted a
 * registry at all.
 *
 * Pure, so the widget mapping, the modified marks, the search and every reason
 * a control is unavailable are tested without React, a core or a DOM.
 *
 * **No bound is written here.** `min`, `max` and `max_len` come off the wire,
 * because a copy that disagrees with Rust by one lets the page offer a value
 * the core refuses — and `settings.set` is atomic, so that refusal fails the
 * whole patch with an error naming a field the user did not touch.
 */

/**
 * The control a row draws.
 *
 * Five of the six are a mechanical reading of `SettingKindWire`. `custom` is
 * the escape hatch for a setting whose value alone does not describe what
 * changing it *does* — today only `agents.notifications`, which has to reach
 * the platform as well as the file.
 *
 * `raw` is not one of the six: it is the failure case, a setting described by
 * a kind this build has never heard of. It is rendered read-only with its
 * value rather than dropped, the way `run-console-model.ts` renders a feed
 * variant from a newer core — a control that is missing is indistinguishable
 * from a setting that does not exist.
 */
export type SettingWidget =
  | { readonly kind: 'toggle' }
  | { readonly kind: 'number'; readonly min: number; readonly max: number }
  | { readonly kind: 'select'; readonly choices: readonly SettingChoice[] }
  /** A single line. `nullable` is what makes an empty box mean "unset". */
  | { readonly kind: 'text'; readonly maxLength: number | null; readonly path: boolean }
  | { readonly kind: 'list'; readonly maxLength: number }
  | { readonly kind: 'custom'; readonly control: CustomControl }
  | { readonly kind: 'raw' };

/** A setting the page draws itself. Exhaustive on purpose: the list is the contract. */
export type CustomControl = 'notifications';

/** Which keys the page has a bespoke control for, and what that control is. */
const CUSTOM_CONTROLS: Readonly<Record<string, CustomControl>> = {
  // Turning this on is the only thing in Gritty that can raise a notification
  // permission prompt (T223 §2), and the prompt must come from the control the
  // user touched. A `settings.set` alone would write `true` to the file and arm
  // nothing until the next launch, which would make the schema's `LIVE` a lie.
  'agents.notifications': 'notifications',
};

/**
 * Why a control cannot be operated, in `run-console-model.ts`'s `PromptGate`
 * shape and for its reason: a disabled control implies a permission the user
 * could acquire, so the case where none exists has to be a different variant
 * from the case where one does.
 *
 * Every `readonly` carries a sentence written for this situation. None of them
 * is a default nobody chose.
 */
export type SettingAvailability =
  /** Operable. */
  | { readonly kind: 'ready' }
  /**
   * Shown, with its value, and not operable — because something the user can
   * go and fix is in the way. The reason says what to do.
   */
  | { readonly kind: 'readonly'; readonly reason: string }
  /**
   * Not drawn at all, not even disabled.
   *
   * One case: a build that cannot raise notifications reports
   * `supported: false`, which the protocol defines as the setting being
   * *absent* rather than broken. A greyed-out switch there would offer a
   * permission no waiting and no clicking can obtain.
   */
  | { readonly kind: 'absent' };

/** Everything outside the snapshot that decides whether a control can be used. */
export interface SettingsEnvironment {
  readonly storage: SettingsStorage;
  /**
   * The platform's answer for `agents.notifications`, or `null` before it has
   * been read. Reading never prompts, so this is fetched on mount.
   */
  readonly notifications: AgentNotificationSettings | null;
}

export interface SettingRow {
  readonly spec: SettingSpecWire;
  readonly widget: SettingWidget;
  /** The effective value: the file overlaid on the default. */
  readonly value: JsonValue;
  /** Differs from the default, so it gets a mark and a reset. */
  readonly modified: boolean;
  /** The file sets this and the core refused it; the default is in force. */
  readonly invalid: boolean;
  readonly availability: SettingAvailability;
  /** What the core said about this key specifically. */
  readonly diagnostics: readonly SettingDiagnostic[];
}

export interface SettingsSectionView {
  readonly section: SettingSection;
  readonly rows: readonly SettingRow[];
}

export interface SettingsView {
  /** Only sections with at least one row to draw; empty when a search matches nothing. */
  readonly sections: readonly SettingsSectionView[];
  /** Rows drawn, across every section. What "no results" is decided on. */
  readonly total: number;
  /** Modified keys the page can actually reset — drawn rows only. */
  readonly modified: readonly string[];
  /** File-wide diagnostics: the ones naming no key. */
  readonly fileDiagnostics: readonly SettingDiagnostic[];
}

/**
 * The control for one setting.
 *
 * A bespoke control wins over the kind: `agents.notifications` is a `bool` and
 * would otherwise map to a toggle that writes the file and arms nothing.
 */
export function widgetFor(spec: SettingSpecWire): SettingWidget {
  const custom = CUSTOM_CONTROLS[spec.key];
  if (custom) return { kind: 'custom', control: custom };
  switch (spec.kind.type) {
    case 'bool':
      return { kind: 'toggle' };
    case 'integer':
      return { kind: 'number', min: spec.kind.min, max: spec.kind.max };
    case 'enum':
      return { kind: 'select', choices: spec.kind.choices };
    case 'nullableString':
      return { kind: 'text', maxLength: spec.kind.max_len, path: false };
    case 'nullablePath':
      return { kind: 'text', maxLength: null, path: true };
    case 'pathList':
      return { kind: 'list', maxLength: spec.kind.max_len };
    default:
      // A kind from a newer core. Read-only with the raw value, never dropped.
      return { kind: 'raw' };
  }
}

/**
 * Whether this row can be operated, and what to say when it cannot.
 *
 * Storage comes first because it beats everything: a page that lets you change
 * a value it cannot save is the worst version of this screen — it appears to
 * take and it is gone next launch.
 */
export function availabilityOf(
  spec: SettingSpecWire,
  widget: SettingWidget,
  env: SettingsEnvironment,
): SettingAvailability {
  if (!env.storage.writable) {
    return {
      kind: 'readonly',
      // The core writes this to say what to do, not only what is wrong. Its
      // absence would be a core bug; say something true rather than nothing.
      reason: env.storage.reason ?? 'Settings cannot be saved right now, and nothing says why.',
    };
  }
  if (widget.kind === 'raw') {
    return {
      kind: 'readonly',
      reason: `This build does not know how to draw a "${spec.kind.type}" setting. Its value is shown as stored; edit settings.json to change it.`,
    };
  }
  if (widget.kind === 'custom' && widget.control === 'notifications') {
    // Not yet read is not the same as unsupported: say so rather than drawing a
    // switch whose position is a guess.
    if (env.notifications === null) {
      return { kind: 'readonly', reason: 'Asking the system whether it can show notifications…' };
    }
    if (!env.notifications.supported) return { kind: 'absent' };
  }
  return { kind: 'ready' };
}

/**
 * Does this setting match what was typed?
 *
 * Key, label and description, case-insensitively, and **every** whitespace-
 * separated term has to match somewhere. Two terms narrowing rather than
 * widening is what makes "terminal font" useful; it is also what a user who
 * has typed two words already expects.
 */
export function matchesQuery(spec: SettingSpecWire, query: string): boolean {
  const terms = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (terms.length === 0) return true;
  const haystack = `${spec.key} ${spec.label} ${spec.description}`.toLowerCase();
  return terms.every((term) => haystack.includes(term));
}

/**
 * The whole page, as rows.
 *
 * Sections keep the registry's order, and so do the settings inside them: the
 * order is a decision taken once in Rust rather than re-taken here.
 *
 * A section a search empties is dropped, so the rail never offers a category
 * with nothing behind it. A setting the schema declares under a section that
 * does not exist is dropped too — that is a registry bug, and a Rust test
 * already fails on it; inventing a section for it here would hide the bug.
 */
export function settingsView(
  schema: SettingsSchema,
  snapshot: SettingsSnapshot,
  env: SettingsEnvironment,
  query = '',
): SettingsView {
  const modified = new Set(snapshot.modified);
  const invalid = new Set(snapshot.invalid);
  const byKey = new Map<string, SettingDiagnostic[]>();
  const fileDiagnostics: SettingDiagnostic[] = [];
  for (const diagnostic of snapshot.diagnostics) {
    if (diagnostic.key === undefined || diagnostic.key === null) {
      fileDiagnostics.push(diagnostic);
      continue;
    }
    const existing = byKey.get(diagnostic.key);
    if (existing) existing.push(diagnostic);
    else byKey.set(diagnostic.key, [diagnostic]);
  }

  const rowsBySection = new Map<string, SettingRow[]>();
  for (const spec of schema.settings) {
    if (!matchesQuery(spec, query)) continue;
    const widget = widgetFor(spec);
    const availability = availabilityOf(spec, widget, env);
    if (availability.kind === 'absent') continue;
    // `values` is the effective set, so a key missing from it is a core bug
    // rather than "unset". The default is the honest fallback either way, and
    // `undefined` is the only absence: `null` is a value several settings take.
    const stored = snapshot.values[spec.key];
    const row: SettingRow = {
      spec,
      widget,
      value: stored === undefined ? spec.default : stored,
      modified: modified.has(spec.key),
      invalid: invalid.has(spec.key),
      availability,
      diagnostics: byKey.get(spec.key) ?? [],
    };
    const existing = rowsBySection.get(spec.section);
    if (existing) existing.push(row);
    else rowsBySection.set(spec.section, [row]);
  }

  const sections: SettingsSectionView[] = [];
  for (const section of schema.sections) {
    const rows = rowsBySection.get(section.id);
    if (rows && rows.length > 0) sections.push({ section, rows });
  }
  // Counted off `sections` rather than off the row pass, so a row whose section
  // was dropped is not claimed to be on screen. The two differ only for a
  // registry bug, and that is exactly when a count that lies is worst.
  const drawn = sections.flatMap((entry) => entry.rows);
  return {
    sections,
    total: drawn.length,
    modified: drawn.filter((row) => row.modified).map((row) => row.spec.key),
    fileDiagnostics,
  };
}

/**
 * The hint beside a control, or `null` when a change is simply in force.
 *
 * Saying "this reaches the next terminal" is cheaper and more honest than
 * pretending every setting is live, and it is the difference between a user
 * thinking the setting is broken and knowing what to do about it.
 */
export function appliesHint(spec: SettingSpecWire): string | null {
  switch (spec.applies) {
    case 'LIVE':
      return null;
    case 'NEW_TERMINALS':
      return 'Terminals already open keep the old value.';
    default:
      // A newer core's variant. Say that it is not immediate rather than
      // claiming it is.
      return 'This does not take effect immediately.';
  }
}

/**
 * The strings the list widget edits, from whatever the value actually is.
 *
 * Defensive rather than expected: the core validates a `pathList` into an
 * array of strings, and a value that is not one would otherwise throw inside
 * a render.
 */
export function pathListOf(value: JsonValue): readonly string[] {
  if (!Array.isArray(value)) return [];
  return value.filter((entry): entry is string => typeof entry === 'string');
}

/** The text a nullable string or path control shows; `null` and absent are both empty. */
export function textOf(value: JsonValue): string {
  return typeof value === 'string' ? value : '';
}

/** The number a bounded control shows, falling back to the spec's default. */
export function numberOf(value: JsonValue, spec: SettingSpecWire): number {
  if (typeof value === 'number' && Number.isFinite(value)) return value;
  return typeof spec.default === 'number' ? spec.default : 0;
}

/** The string a select shows. Enum values are strings on the wire, always. */
export function choiceOf(value: JsonValue): string {
  return typeof value === 'string' ? value : '';
}

/** The state a toggle shows. */
export function boolOf(value: JsonValue): boolean {
  return value === true;
}
