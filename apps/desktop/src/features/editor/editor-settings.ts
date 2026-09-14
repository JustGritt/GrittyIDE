import {
  DEFAULT_EDITOR_PREFERENCES,
  type EditorPreferences,
  parseEditorPreferences,
} from '@gritty/editor';
import type { SettingsSnapshot } from '@gritty/protocol';

/**
 * What Monaco is configured from: the `editor.*` settings in force (T120-A,
 * ADR-031).
 *
 * `settings.json` is the source of truth for these four, and the settings page
 * is the one place they are edited. The session's `preferences` blob is no
 * longer read for them; it is left as it was, because it is what the one-time
 * seed (`settings-seed.ts`) moves across on the first launch that has the file.
 *
 * Every read goes through `parseEditorPreferences`, which already knows the
 * editor's bounds. The core validates each value against the registry before it
 * reaches a snapshot, so this is belt and braces against a registry and an
 * editor that disagree — the editor falls back to its default rather than
 * being handed a size it cannot draw.
 *
 * `null` until the first snapshot lands, which is the editor's defaults: those
 * are also the registry's, so a settings file that sets nothing does not change
 * anything on screen when it arrives.
 */
export function editorPreferencesFromSettings(
  snapshot: SettingsSnapshot | null,
): EditorPreferences {
  if (snapshot === null) return DEFAULT_EDITOR_PREFERENCES;
  const { values } = snapshot;
  const tabSize = values['editor.tabSize'];
  return parseEditorPreferences({
    fontSize: values['editor.fontSize'],
    // The registry models tab size as a choice of strings ("2", "4", "8"), the
    // editor as a number; `settings-seed.ts` translates the other way.
    tabSize: typeof tabSize === 'string' ? Number(tabSize) : tabSize,
    wordWrap: values['editor.wordWrap'],
    minimap: values['editor.minimap'],
  });
}
