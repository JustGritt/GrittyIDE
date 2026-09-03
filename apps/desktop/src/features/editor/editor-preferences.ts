import { DEFAULT_EDITOR_PREFERENCES, type EditorPreferences } from '@gritty/editor';
import { useCallback, useState } from 'react';

/** In-memory editor preferences; T060 persists them. */
export function useEditorPreferences() {
  const [prefs, setPrefs] = useState<EditorPreferences>(DEFAULT_EDITOR_PREFERENCES);
  const update = useCallback(
    (patch: Partial<EditorPreferences>) => setPrefs((previous) => ({ ...previous, ...patch })),
    [],
  );
  return { prefs, update };
}
