import type {
  JsonValue,
  SettingSpecWire,
  SettingsSchema,
  SettingsSnapshot,
} from '@gritty/protocol';

/**
 * The one-time move of today's values into `settings.json` (ADR-031 §8).
 *
 * It happens in the WebView, not the core. `preferences` and `layout` are
 * UI-owned blobs the core stores opaquely (ADR-014), so having the core open a
 * session row and interpret one would invert exactly the ownership ADR-031 is
 * drawing. The UI owns those blobs, so the UI is what is entitled to translate
 * them.
 *
 * A **planner**, pure and total: it reads the blobs and the snapshot and says
 * which keys to write. The caller (`use-settings.ts`) sends the result as one
 * `settings.set`, which is one atomic file write — so the seeding either lands
 * whole or not at all, and the blobs are left intact either way. That is what
 * makes retrying on the next launch safe.
 *
 * There is **no migration flag**, and that is deliberate rather than lazy: a
 * key the file already names is reported back in the snapshot, so it is never
 * seeded twice. A flag would be a second source of truth about a question the
 * snapshot already answers.
 */

/** The blobs exactly as the session restore returned them. */
export interface SeedSource {
  /**
   * The `preferences` blob, or `null`/`undefined` when the restore returned
   * none.
   *
   * Raw rather than parsed, and that distinction is the point: a parser fills
   * every missing field with its default, so a parsed blob cannot be told
   * apart from an absent one — and seeding defaults over an absent blob is
   * exactly what pins a user who never chose a font size to today's value
   * forever.
   */
  readonly preferences: unknown;
  /** The `layout` blob, on the same terms. */
  readonly layout: unknown;
}

/** One old field, and the setting it becomes. */
interface Candidate {
  readonly key: string;
  /** The old value, or `undefined` when the blob does not carry a usable one. */
  readonly value: JsonValue | undefined;
}

/**
 * Which keys to write, or an empty object when there is nothing to do.
 *
 * A candidate is dropped when any of these holds, and each drop is a decision
 * ADR-031 §8 states:
 *
 * - **The file already names the key.** Both `explicit` (accepted) and
 *   `invalid` (present and refused) count: overwriting a value a user typed by
 *   hand, even a wrong one, would delete what they were in the middle of
 *   fixing.
 * - **The old value equals the schema default.** Writing it would freeze
 *   today's default into their file, where a later build could never change it.
 * - **The old value is not one the core would accept.** The whole patch is
 *   refused as a unit, so one bad field would cost every good one. A blob can
 *   hold a value from an older UI whose bounds were wider.
 * - **The blob is absent**, which yields no candidates at all.
 */
export function planSeed(
  schema: SettingsSchema,
  snapshot: SettingsSnapshot,
  source: SeedSource,
): Record<string, JsonValue> {
  const already = new Set([...snapshot.explicit, ...snapshot.invalid]);
  const specs = new Map(schema.settings.map((spec) => [spec.key, spec]));
  const patch: Record<string, JsonValue> = {};
  for (const candidate of candidates(source)) {
    if (candidate.value === undefined) continue;
    if (already.has(candidate.key)) continue;
    const spec = specs.get(candidate.key);
    // A build that no longer declares the key. Nothing to seed it into.
    if (!spec) continue;
    if (sameJson(candidate.value, spec.default)) continue;
    if (!acceptable(spec, candidate.value)) continue;
    patch[candidate.key] = candidate.value;
  }
  return patch;
}

/**
 * The old fields, in registry order, with `undefined` for anything the blob
 * does not carry in a usable form.
 *
 * Every read is defensive because a blob is whatever was on disk: a version
 * this build has never seen, half-written, or hand-edited.
 */
function candidates(source: SeedSource): readonly Candidate[] {
  const prefs = objectOf(source.preferences);
  const layout = objectOf(source.layout);
  const terminal = layout === null ? null : objectOf(layout.terminal);
  return [
    { key: 'editor.fontSize', value: integerOf(prefs?.fontSize) },
    // The registry models tab size as an enum of strings ("2", "4", "8") while
    // the blob held a number, so this is the one field that is translated
    // rather than copied. A width the enum does not offer — a hand-edited 3 —
    // becomes a string that is not a choice, and is dropped by `acceptable`.
    { key: 'editor.tabSize', value: stringifyNumber(prefs?.tabSize) },
    { key: 'editor.wordWrap', value: booleanOf(prefs?.wordWrap) },
    { key: 'editor.minimap', value: booleanOf(prefs?.minimap) },
    { key: 'terminal.shell', value: nullableStringOf(terminal?.profilePath) },
    { key: 'terminal.customShells', value: stringArrayOf(terminal?.customShells) },
    { key: 'terminal.fontFamily', value: nullableStringOf(terminal?.fontFamily) },
    { key: 'terminal.fontSize', value: integerOf(terminal?.fontSize) },
    // `workbench.theme` has no predecessor: the theme keyed off the operating
    // system and was never stored (ADR-016). `agents.notifications` has none
    // either — T223 decision 6 kept it in memory precisely because there was
    // nowhere to put it, which is the gap ADR-031 closes.
  ];
}

/**
 * Would the core accept this value for this setting?
 *
 * Mirrors `crates/settings/src/document.rs::validate` — deliberately a mirror
 * and not a second opinion. It exists only to keep one stale blob field from
 * failing an otherwise good patch; the core validates every write regardless,
 * and a disagreement between the two costs a skipped seed rather than a bad
 * value.
 */
function acceptable(spec: SettingSpecWire, value: JsonValue): boolean {
  switch (spec.kind.type) {
    case 'bool':
      return typeof value === 'boolean';
    case 'integer':
      return (
        typeof value === 'number' &&
        Number.isInteger(value) &&
        value >= spec.kind.min &&
        value <= spec.kind.max
      );
    case 'enum':
      return typeof value === 'string' && spec.kind.choices.some((c) => c.value === value);
    case 'nullableString':
      return (
        value === null || (typeof value === 'string' && [...value].length <= spec.kind.max_len)
      );
    case 'nullablePath':
      return value === null || (typeof value === 'string' && isAbsolutePath(value.trim()));
    case 'pathList':
      return (
        Array.isArray(value) &&
        value.length <= spec.kind.max_len &&
        value.every((entry) => typeof entry === 'string' && isAbsolutePath(entry.trim()))
      );
    default:
      // A kind this build has never seen. Never seed into it: the core is the
      // only thing that knows what it means.
      return false;
  }
}

/**
 * Absolute in the way `std::path::Path::is_absolute` means it, on either
 * family — a leading `/`, a drive letter, or a UNC root.
 *
 * Rust compiles that check for one target, so only one family is ever really
 * in play; accepting both here is safe because the path being tested came out
 * of this machine's own layout blob, written by this UI on this machine.
 */
function isAbsolutePath(path: string): boolean {
  return path.startsWith('/') || /^[A-Za-z]:[\\/]/.test(path) || path.startsWith('\\\\');
}

/** Structural equality over JSON. Only used against a schema default. */
function sameJson(a: JsonValue, b: JsonValue): boolean {
  if (a === b) return true;
  if (Array.isArray(a) || Array.isArray(b)) {
    if (!Array.isArray(a) || !Array.isArray(b) || a.length !== b.length) return false;
    return a.every((entry, index) => {
      const other = b[index];
      // In range by the length check above; `undefined` is not a JSON value,
      // so treating it as unequal is the only honest reading anyway.
      return other !== undefined && sameJson(entry, other);
    });
  }
  if (typeof a === 'object' && a !== null && typeof b === 'object' && b !== null) {
    const left = Object.keys(a);
    return (
      left.length === Object.keys(b).length &&
      left.every((key) => {
        const other = b[key];
        return other !== undefined && sameJson(a[key] as JsonValue, other);
      })
    );
  }
  return false;
}

function objectOf(value: unknown): Record<string, unknown> | null {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

function integerOf(value: unknown): number | undefined {
  return typeof value === 'number' && Number.isInteger(value) ? value : undefined;
}

function booleanOf(value: unknown): boolean | undefined {
  return typeof value === 'boolean' ? value : undefined;
}

/** A nullable field: `null` is a value worth seeding, absent is not. */
function nullableStringOf(value: unknown): string | null | undefined {
  if (value === null) return null;
  return typeof value === 'string' ? value : undefined;
}

function stringArrayOf(value: unknown): string[] | undefined {
  if (!Array.isArray(value)) return undefined;
  return value.every((entry) => typeof entry === 'string') ? [...(value as string[])] : undefined;
}

/** The number-to-enum translation. Anything else is not a candidate at all. */
function stringifyNumber(value: unknown): string | undefined {
  return typeof value === 'number' && Number.isInteger(value) ? String(value) : undefined;
}
