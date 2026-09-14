import type { AgentNotificationSettings, JsonValue, TerminalProfile } from '@gritty/protocol';
import { type KeyboardEvent, useEffect, useState } from 'react';
import { core } from '../../core/client';
import { addCustomShell } from '../terminal/terminal-profiles';

/**
 * The controls that need more than a value (T120-U §4, §9).
 *
 * Everything the schema fully describes — a toggle, a select — is drawn inline
 * in `settings-page.tsx` from the widget alone. These three are here because
 * each needs something the schema cannot say:
 *
 * - a **draft**, because a fully controlled input the core may refuse cannot
 *   be typed into;
 * - the **shell list's** add/remove editing and the reasons a path does not
 *   work, which come from `terminal.profiles`;
 * - the **notification switch's** trip to the platform, which is the only
 *   thing in Gritty that can raise a permission prompt.
 */

/**
 * A text or number box that commits on blur or Enter, never per keystroke.
 *
 * Two reasons, and the second is the one that makes it necessary rather than
 * merely nicer. Per-keystroke writes would send `1` on the way to `13`, and
 * `settings.set` is one atomic file write each. And a fully controlled input
 * bound to a value the core refuses simply cannot be typed into: every
 * keystroke would be reverted by the answer to the one before it.
 *
 * So the draft is local while it is being edited, and the stored value is put
 * back when a commit is refused — the one piece of state on this page that is
 * not the snapshot, and it is scoped to a single control being typed into.
 */
export function DraftField({
  id,
  value,
  type,
  min,
  max,
  maxLength,
  placeholder,
  disabled,
  describedBy,
  onCommit,
}: {
  id: string;
  /** The stored value, as text. The draft returns to it on a refusal. */
  value: string;
  type: 'text' | 'number';
  min?: number;
  max?: number;
  maxLength?: number;
  placeholder?: string;
  disabled: boolean;
  describedBy: string;
  /** Resolves `false` when the core refused, which puts the draft back. */
  onCommit: (raw: string) => Promise<boolean>;
}) {
  const [draft, setDraft] = useState(value);
  const [dirty, setDirty] = useState(false);

  // Follow the store while the user is not mid-edit. An external edit, a reset
  // or another window's write all arrive as a new `value`, and the box has to
  // show it; a draft in progress is the user's and is left alone.
  useEffect(() => {
    if (!dirty) setDraft(value);
  }, [value, dirty]);

  const commit = async () => {
    if (!dirty) return;
    const raw = draft;
    // Still dirty while the write is in flight, deliberately. Clearing it here
    // would let the sync effect above put the *stored* value back for the one
    // frame between asking and being answered, so an accepted change visibly
    // flickers to the old value and back.
    const accepted = await onCommit(raw);
    setDirty(false);
    // Nothing optimistic: on a refusal the stored value is what is true, and
    // the envelope above the list says why.
    if (!accepted) setDraft(value);
  };

  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'Enter') {
      event.preventDefault();
      void commit();
      return;
    }
    if (event.key === 'Escape') {
      event.preventDefault();
      setDirty(false);
      setDraft(value);
    }
  };

  return (
    <input
      id={id}
      className="settings__input"
      type={type}
      value={draft}
      min={min}
      max={max}
      maxLength={maxLength}
      placeholder={placeholder}
      disabled={disabled}
      spellCheck={false}
      aria-describedby={describedBy}
      onChange={(event) => {
        setDirty(true);
        setDraft(event.target.value);
      }}
      onBlur={() => void commit()}
      onKeyDown={onKeyDown}
    />
  );
}

/**
 * `terminal.customShells`: the paths discovery cannot find.
 *
 * The editing rules are `addCustomShell`'s, unchanged and still the only copy —
 * it is pure and already tested, and re-deriving "is this absolute, is it
 * already here, is the list full" beside it is how the two drift. The bound it
 * is given comes off the wire rather than from the UI constant the popover
 * used, because the core enforces its own and a copy that disagrees by one
 * fails the whole patch.
 *
 * `terminal.profiles` is asked which of them actually work. An entry the user
 * typed that does not resolve is *reported* rather than dropped: they typed
 * it, so they are owed the reason.
 */
export function CustomShellsField({
  id,
  paths,
  maxLength,
  disabled,
  describedBy,
  onCommit,
}: {
  id: string;
  paths: readonly string[];
  maxLength: number;
  disabled: boolean;
  describedBy: string;
  onCommit: (next: readonly string[]) => Promise<boolean>;
}) {
  const [draft, setDraft] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [profiles, setProfiles] = useState<readonly TerminalProfile[]>([]);

  // Only to say *why* an entry is broken. A failure here is not worth an
  // envelope: the list still edits, the reasons are simply absent, and
  // "unchecked" is exactly what an empty `profiles` renders as.
  //
  // Keyed on the JSON of the list rather than a joined string: a shell path
  // may contain a space, and splitting one back apart would ask the core about
  // two paths that do not exist.
  const custom = JSON.stringify(paths);
  useEffect(() => {
    let active = true;
    core.terminal.profiles({ custom: JSON.parse(custom) as string[] }).then(
      (answer) => {
        if (active) setProfiles(answer);
      },
      () => {},
    );
    return () => {
      active = false;
    };
  }, [custom]);

  const add = async () => {
    const result = addCustomShell(paths, draft, maxLength);
    if ('error' in result) {
      setError(result.error);
      return;
    }
    setError(null);
    if (await onCommit(result.shells)) setDraft('');
  };

  const remove = async (path: string) => {
    setError(null);
    await onCommit(paths.filter((shell) => shell !== path));
  };

  return (
    <div className="settings__shells">
      {paths.length > 0 && (
        <ul className="settings__shell-list">
          {paths.map((path) => {
            const profile = profiles.find((candidate) => candidate.path === path);
            // Absent means the list has not been fetched back yet, which is not
            // the same as broken; only say so once we know.
            const broken = profile !== undefined && !profile.available;
            return (
              <li key={path} className="settings__shell">
                <code className={broken ? 'settings__shell-broken' : undefined}>{path}</code>
                {broken && (
                  <span className="settings__shell-reason">{profile.unavailableReason}</span>
                )}
                <button
                  type="button"
                  className="settings__ghost"
                  disabled={disabled}
                  aria-label={`Remove ${path}`}
                  onClick={() => void remove(path)}
                >
                  Remove
                </button>
              </li>
            );
          })}
        </ul>
      )}
      <div className="settings__shell-add">
        <input
          id={id}
          className="settings__input"
          type="text"
          value={draft}
          spellCheck={false}
          disabled={disabled}
          placeholder="/opt/homebrew/bin/fish"
          aria-label="Path of a shell to add"
          aria-describedby={describedBy}
          onChange={(event) => {
            setDraft(event.target.value);
            setError(null);
          }}
          onKeyDown={(event) => {
            if (event.key === 'Enter') {
              event.preventDefault();
              void add();
            }
          }}
        />
        <button type="button" disabled={disabled} onClick={() => void add()}>
          Add
        </button>
      </div>
      {error && (
        <p className="settings__field-error" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}

/**
 * `agents.notifications`: the platform, then the file (T223 §2, ADR-031).
 *
 * A plain schema toggle would write `true` and arm nothing until the next
 * launch, which would make the registry's `LIVE` a lie. So this goes through
 * `agents.setNotifications` first — the one call in Gritty that can raise a
 * notification permission prompt — and the controller persists whatever the
 * platform actually did.
 *
 * A build that cannot raise notifications never reaches this component: the
 * protocol defines `supported: false` as the setting being *absent*, and the
 * model drops the row rather than offering a switch that would do nothing.
 */
export function NotificationField({
  id,
  settings,
  disabled,
  describedBy,
  onToggle,
}: {
  id: string;
  /** `null` until the platform has answered. */
  settings: AgentNotificationSettings | null;
  disabled: boolean;
  describedBy: string;
  onToggle: (enabled: boolean) => Promise<boolean>;
}) {
  return (
    <div className="settings__notifications">
      <input
        id={id}
        type="checkbox"
        checked={settings?.enabled ?? false}
        disabled={disabled || settings === null}
        aria-describedby={describedBy}
        onChange={(event) => void onToggle(event.target.checked)}
      />
      {settings?.permission === 'DENIED' && (
        <p className="settings__note">
          The system refused permission, so nothing can be shown. Grant it to Gritty in your
          notification settings; Gritty will not ask again on its own.
        </p>
      )}
    </div>
  );
}

/** The value a `raw` row shows: what is stored, as it is stored. */
export function rawText(value: JsonValue): string {
  return JSON.stringify(value, null, 2) ?? 'null';
}
