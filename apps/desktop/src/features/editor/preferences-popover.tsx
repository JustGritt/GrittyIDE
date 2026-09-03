import type { EditorPreferences } from '@gritty/editor';

export function PreferencesPopover({
  prefs,
  onChange,
}: {
  prefs: EditorPreferences;
  onChange: (patch: Partial<EditorPreferences>) => void;
}) {
  return (
    <details className="prefs">
      <summary>Editor settings</summary>
      <div className="prefs__form">
        <label>
          Font size
          <input
            type="number"
            min={8}
            max={32}
            value={prefs.fontSize}
            onChange={(e) => {
              const fontSize = Number(e.target.value);
              if (Number.isFinite(fontSize) && fontSize >= 8 && fontSize <= 32) {
                onChange({ fontSize });
              }
            }}
          />
        </label>
        <label>
          Tab size
          <select
            value={prefs.tabSize}
            onChange={(e) => onChange({ tabSize: Number(e.target.value) })}
          >
            <option value={2}>2</option>
            <option value={4}>4</option>
            <option value={8}>8</option>
          </select>
        </label>
        <label className="prefs__check">
          <input
            type="checkbox"
            checked={prefs.wordWrap}
            onChange={(e) => onChange({ wordWrap: e.target.checked })}
          />
          Word wrap
        </label>
        <label className="prefs__check">
          <input
            type="checkbox"
            checked={prefs.minimap}
            onChange={(e) => onChange({ minimap: e.target.checked })}
          />
          Minimap
        </label>
      </div>
    </details>
  );
}
