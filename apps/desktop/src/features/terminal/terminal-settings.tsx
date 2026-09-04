import type { TerminalProfile } from '@gritty/protocol';
import { useState } from 'react';
import {
  MAX_CUSTOM_SHELLS,
  TERMINAL_FONT_SIZE_RANGE,
  type TerminalLayout,
} from '../session/persisted-layout';
import { addCustomShell } from './terminal-profiles';
import { DEFAULT_TERMINAL_FONT_SIZE } from './terminal-view';

/**
 * Terminal settings (T095): shells discovery cannot find, and the font.
 *
 * Discovery is deliberately conservative — it reports only what `$SHELL` and
 * `/etc/shells` name, because guessing by file name offered a package
 * manager's helper as a shell. This is the escape hatch for everything that is
 * genuinely installed but not registered, and the place to fix a prompt whose
 * glyphs need a patched font.
 *
 * Mirrors the editor's preferences popover: a `<details>`, persisted with the
 * rest of the layout.
 */
export function TerminalSettings({
  settings,
  profiles,
  onChange,
}: {
  settings: TerminalLayout;
  /** Every profile, so a custom entry can be shown with why it does not work. */
  profiles: readonly TerminalProfile[];
  onChange: (patch: Partial<TerminalLayout>) => void;
}) {
  const [draft, setDraft] = useState('');
  const [error, setError] = useState<string | null>(null);

  const add = () => {
    const result = addCustomShell(settings.customShells, draft, MAX_CUSTOM_SHELLS);
    if ('error' in result) {
      setError(result.error);
      return;
    }
    onChange({ customShells: result.shells });
    setDraft('');
    setError(null);
  };

  const remove = (path: string) => {
    onChange({ customShells: settings.customShells.filter((shell) => shell !== path) });
    setError(null);
  };

  return (
    <details className="prefs">
      <summary>Terminal settings</summary>
      <div className="prefs__form">
        <fieldset className="terminal-settings__shells">
          <legend>Shells</legend>
          <p className="terminal-settings__hint">
            Gritty lists the shells your system registers, in <code>/etc/shells</code>. Add the full
            path of anything else you have installed.
          </p>
          {settings.customShells.length > 0 && (
            <ul className="terminal-settings__list">
              {settings.customShells.map((path) => {
                const profile = profiles.find((candidate) => candidate.path === path);
                // Absent means the list has not been fetched back yet, which
                // is not the same as broken; only say so once we know.
                const broken = profile && !profile.available;
                return (
                  <li key={path}>
                    <span className={broken ? 'terminal-settings__broken' : undefined}>{path}</span>
                    {broken && (
                      <span className="terminal-settings__reason">{profile.unavailableReason}</span>
                    )}
                    <button
                      type="button"
                      onClick={() => remove(path)}
                      aria-label={`Remove ${path}`}
                    >
                      Remove
                    </button>
                  </li>
                );
              })}
            </ul>
          )}
          <div className="terminal-settings__add">
            <input
              type="text"
              value={draft}
              spellCheck={false}
              placeholder="/opt/homebrew/bin/fish"
              aria-label="Path of a shell to add"
              onChange={(event) => {
                setDraft(event.target.value);
                setError(null);
              }}
              onKeyDown={(event) => {
                if (event.key === 'Enter') {
                  event.preventDefault();
                  add();
                }
              }}
            />
            <button type="button" onClick={add}>
              Add
            </button>
          </div>
          {error && <p className="terminal-settings__error">{error}</p>}
        </fieldset>

        <label>
          Font family
          <input
            type="text"
            spellCheck={false}
            value={settings.fontFamily ?? ''}
            // A prompt drawn with powerline glyphs needs a patched font; the
            // default stack names the common ones, and this is for the rest.
            placeholder="Default (Nerd Font, then monospace)"
            onChange={(event) => onChange({ fontFamily: event.target.value.trim() || null })}
          />
        </label>

        <label>
          Font size
          <input
            type="number"
            min={TERMINAL_FONT_SIZE_RANGE.min}
            max={TERMINAL_FONT_SIZE_RANGE.max}
            value={settings.fontSize ?? DEFAULT_TERMINAL_FONT_SIZE}
            onChange={(event) => {
              const fontSize = Number(event.target.value);
              const { min, max } = TERMINAL_FONT_SIZE_RANGE;
              if (Number.isFinite(fontSize) && fontSize >= min && fontSize <= max) {
                onChange({ fontSize });
              }
            }}
          />
        </label>
      </div>
    </details>
  );
}
