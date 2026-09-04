import { DEFAULT_EDITOR_PREFERENCES, type EditorPreferences } from '@gritty/editor';
import { useCallback, useState } from 'react';

/** Editor preferences; the session (T060) restores them at startup and persists changes. */
export function useEditorPreferences() {
  const [prefs, setPrefs] = useState<EditorPreferences>(DEFAULT_EDITOR_PREFERENCES);
  const update = useCallback(
    (patch: Partial<EditorPreferences>) => setPrefs((previous) => ({ ...previous, ...patch })),
    [],
  );
  const replace = useCallback((next: EditorPreferences) => setPrefs(next), []);
  return { prefs, update, replace };
}
