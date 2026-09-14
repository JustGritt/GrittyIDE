import type { JsonValue, SettingDiagnostic } from '@gritty/protocol';
import { type KeyboardEvent, useMemo, useState } from 'react';
import { ErrorEnvelope } from '../../components/error-envelope';
import { AgentIntegrationSection } from './agent-integration-section';
import { CustomShellsField, DraftField, NotificationField, rawText } from './settings-fields';
import {
  appliesHint,
  boolOf,
  choiceOf,
  numberOf,
  pathListOf,
  type SettingRow,
  type SettingsSectionView,
  settingsView,
  textOf,
} from './settings-model';
import type { SettingsController } from './use-settings';

/**
 * The settings page (T120-U, ADR-031).
 *
 * A category rail on the left, the settings on the right, a search box over
 * key, label and description, and a modified mark with a reset on every row
 * that differs from its default. It draws whatever `settings.describe`
 * declares: **there is no per-setting UI code in this file**, which is the
 * property that makes ADR-031 §3's registry worth having — a setting added to
 * the Rust table arrives here with a working control.
 *
 * It renders as a sibling inside `.editor__surfaces`, leaving Monaco `hidden`
 * rather than unmounted, exactly as the consoles do (ADR-026). The mounting
 * itself is T120-M's.
 *
 * Nothing here is optimistic. Every control shows the snapshot, every write
 * converges on the snapshot the core answered with, and a refusal is rendered
 * through `ErrorEnvelope` rather than swallowed. The one exception is the draft
 * inside `DraftField`, which exists because a controlled input the core may
 * refuse cannot otherwise be typed into.
 */
export function SettingsPage({
  controller,
  initialSection,
}: {
  controller: SettingsController;
  /**
   * The section id to open on, e.g. `editor` from the editor bar (T120-A).
   * Read once, when the page mounts; an id the schema does not declare falls
   * back to the first section like any other unresolvable selection.
   */
  initialSection?: string;
}) {
  const [query, setQuery] = useState('');
  const [selected, setSelected] = useState<string | null>(initialSection ?? null);
  const { schema, snapshot, notifications } = controller;

  const view = useMemo(
    () =>
      schema && snapshot
        ? settingsView(schema, snapshot, { storage: snapshot.storage, notifications }, query)
        : null,
    [schema, snapshot, notifications, query],
  );

  // The rail's selection, resolved against what is actually on screen. A
  // search that empties the selected category moves to the first one that has
  // something rather than showing an empty panel with a highlighted rail.
  const sections = view?.sections ?? [];
  const active = sections.find((entry) => entry.section.id === selected) ?? sections[0] ?? null;

  const onRailKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const index = sections.findIndex((entry) => entry.section.id === active?.section.id);
    if (index < 0) return;
    const move = (to: number) => {
      event.preventDefault();
      const next = sections[Math.max(0, Math.min(sections.length - 1, to))];
      if (next) setSelected(next.section.id);
    };
    if (event.key === 'ArrowDown') move(index + 1);
    else if (event.key === 'ArrowUp') move(index - 1);
    else if (event.key === 'Home') move(0);
    else if (event.key === 'End') move(sections.length - 1);
  };

  return (
    <section className="settings" aria-label="Settings">
      <header className="settings__bar">
        <h2 className="settings__title">Settings</h2>
        <input
          className="settings__search"
          type="search"
          value={query}
          spellCheck={false}
          placeholder="Search settings"
          aria-label="Search settings"
          onChange={(event) => setQuery(event.target.value)}
        />
      </header>

      {/* Where the file is, and how to get to it. Selectable text rather than
          only a button: the path is the answer to "what do I open in another
          editor", and on a machine with no file manager the button is the half
          that does not work. */}
      <p className="settings__location">
        {controller.snapshot?.storage.path ? (
          <>
            <code className="settings__path">{controller.snapshot.storage.path}</code>
            <button
              type="button"
              className="settings__ghost"
              onClick={() => void controller.reveal()}
            >
              Reveal
            </button>
          </>
        ) : (
          <span className="settings__placeholder">
            There is no application data directory, so there is no settings file.
          </span>
        )}
      </p>

      {snapshot && !snapshot.storage.writable && (
        <p className="settings__warning" role="status">
          {snapshot.storage.reason ??
            'Settings cannot be saved right now, and nothing says why. The values below are the defaults.'}
        </p>
      )}

      {view?.fileDiagnostics.map((diagnostic) => (
        <p className="settings__warning" role="status" key={diagnostic.message}>
          {diagnosticText(diagnostic)}
        </p>
      ))}

      {snapshot && snapshot.unknown.length > 0 && (
        <p className="settings__note">
          {snapshot.unknown.length === 1
            ? `${snapshot.unknown.length} setting in the file is not one this build declares. It is kept exactly as it is.`
            : `${snapshot.unknown.length} settings in the file are not ones this build declares. They are kept exactly as they are.`}
        </p>
      )}

      {controller.error && (
        <div className="settings__error">
          <ErrorEnvelope error={controller.error} onDismiss={controller.clearError} />
        </div>
      )}

      {!controller.loaded ? (
        <p className="settings__placeholder">Reading your settings…</p>
      ) : sections.length === 0 ? (
        <p className="settings__placeholder">
          {query === '' ? 'This build declares no settings.' : `Nothing matches “${query}”.`}
        </p>
      ) : (
        <div className="settings__body">
          {/* Tabs rather than a nav: one panel is shown at a time, and that is
              what a tab list means. Vertical, with a roving tab stop, so the
              rail is one stop and the arrows move inside it. */}
          <div
            className="settings__rail"
            role="tablist"
            aria-label="Setting categories"
            aria-orientation="vertical"
            onKeyDown={onRailKeyDown}
          >
            {sections.map((entry) => {
              const current = entry.section.id === active?.section.id;
              return (
                <button
                  key={entry.section.id}
                  type="button"
                  role="tab"
                  id={`settings-tab-${entry.section.id}`}
                  className="settings__rail-item"
                  aria-selected={current}
                  aria-controls={`settings-panel-${entry.section.id}`}
                  tabIndex={current ? 0 : -1}
                  onClick={() => setSelected(entry.section.id)}
                >
                  {entry.section.label}
                  {query !== '' && (
                    <span className="settings__rail-count">{entry.rows.length}</span>
                  )}
                </button>
              );
            })}
          </div>

          {active && (
            <div
              className="settings__list"
              role="tabpanel"
              id={`settings-panel-${active.section.id}`}
              aria-labelledby={`settings-tab-${active.section.id}`}
            >
              <Section view={active} controller={controller} />
            </div>
          )}
        </div>
      )}
    </section>
  );
}

function Section({
  view,
  controller,
}: {
  view: SettingsSectionView;
  controller: SettingsController;
}) {
  return (
    <>
      {view.rows.map((row) => (
        <Row key={row.spec.key} row={row} controller={controller} />
      ))}
      {/* The one thing on this page that is not a setting. It lives on the
          Agents category because that is where a person goes looking for it,
          and it reads nothing until a button is pressed (ADR-018 §1). */}
      {view.section.id === 'agents' && <AgentIntegrationSection />}
    </>
  );
}

function Row({ row, controller }: { row: SettingRow; controller: SettingsController }) {
  const { spec, availability } = row;
  const id = `settings-${spec.key}`;
  const describedBy = `${id}-description`;
  const disabled = availability.kind !== 'ready' || controller.busy;
  const hint = appliesHint(spec);

  const commit = (value: JsonValue) => controller.set(spec.key, value);

  return (
    <div className="settings__row" data-modified={row.modified}>
      <div className="settings__row-head">
        <label className="settings__label" htmlFor={id}>
          {spec.label}
        </label>
        {row.modified && (
          <>
            {/* The mark and the reset are one idea, so they sit together: what
                changed, and how to put it back. */}
            <span className="settings__modified" aria-hidden="true" />
            <button
              type="button"
              className="settings__ghost"
              disabled={disabled}
              aria-label={`Reset ${spec.label} to its default`}
              onClick={() => void controller.reset([spec.key])}
            >
              Reset
            </button>
          </>
        )}
        <span className="settings__spacer" />
        <code className="settings__key">{spec.key}</code>
      </div>

      <p className="settings__description" id={describedBy}>
        {spec.description}
      </p>

      <Control
        row={row}
        id={id}
        describedBy={describedBy}
        disabled={disabled}
        commit={commit}
        controller={controller}
      />

      {hint && <p className="settings__applies">{hint}</p>}

      {availability.kind === 'readonly' && (
        <p className="settings__reason" role="status">
          {availability.reason}
        </p>
      )}

      {row.invalid && (
        <p className="settings__warning" role="status">
          The settings file sets this to a value this build refused, so the default is in force.
          Changing it here replaces it.
        </p>
      )}

      {row.diagnostics.map((diagnostic) => (
        <p className="settings__reason" key={diagnostic.message}>
          {diagnosticText(diagnostic)}
        </p>
      ))}
    </div>
  );
}

function Control({
  row,
  id,
  describedBy,
  disabled,
  commit,
  controller,
}: {
  row: SettingRow;
  id: string;
  describedBy: string;
  disabled: boolean;
  commit: (value: JsonValue) => Promise<boolean>;
  controller: SettingsController;
}) {
  const { spec, widget, value } = row;
  switch (widget.kind) {
    case 'toggle':
      return (
        <input
          id={id}
          className="settings__toggle"
          type="checkbox"
          checked={boolOf(value)}
          disabled={disabled}
          aria-describedby={describedBy}
          onChange={(event) => void commit(event.target.checked)}
        />
      );

    case 'number':
      return (
        <DraftField
          id={id}
          type="number"
          value={String(numberOf(value, spec))}
          min={widget.min}
          max={widget.max}
          disabled={disabled}
          describedBy={describedBy}
          // Sent as typed, and the bounds are the wire's, so the core is the
          // only thing that decides. A value it refuses comes back as an
          // envelope naming this field, and the box returns to what is stored.
          onCommit={(raw) => {
            const parsed = Number(raw.trim());
            if (raw.trim() === '' || !Number.isFinite(parsed)) return Promise.resolve(false);
            return commit(parsed);
          }}
        />
      );

    case 'select':
      return (
        <select
          id={id}
          className="settings__select"
          value={choiceOf(value)}
          disabled={disabled}
          aria-describedby={describedBy}
          onChange={(event) => void commit(event.target.value)}
        >
          {widget.choices.map((choice) => (
            <option key={choice.value} value={choice.value}>
              {choice.label}
            </option>
          ))}
        </select>
      );

    case 'text':
      return (
        <DraftField
          id={id}
          type="text"
          value={textOf(value)}
          maxLength={widget.maxLength ?? undefined}
          placeholder={
            widget.path ? 'Absolute path, or empty for the default' : 'Empty for the default'
          }
          disabled={disabled}
          describedBy={describedBy}
          // Empty means unset, which is `null` and not `""`: the schema's
          // default applies again, and the key stops being modified.
          onCommit={(raw) => commit(raw.trim() === '' ? null : raw.trim())}
        />
      );

    case 'list':
      return (
        <CustomShellsField
          id={id}
          paths={pathListOf(value)}
          maxLength={widget.maxLength}
          disabled={disabled}
          describedBy={describedBy}
          onCommit={(next) => commit([...next])}
        />
      );

    case 'custom':
      return (
        <NotificationField
          id={id}
          settings={controller.notifications}
          disabled={disabled}
          describedBy={describedBy}
          onToggle={controller.setNotifications}
        />
      );

    case 'raw':
      // A kind from a newer core. Shown as stored rather than dropped: a
      // missing control is indistinguishable from a setting that does not
      // exist, and this way the value is at least inspectable.
      return <pre className="settings__raw">{rawText(value)}</pre>;

    default:
      return null;
  }
}

/** A diagnostic, with the position when the core carried one. */
function diagnosticText(diagnostic: SettingDiagnostic): string {
  if (diagnostic.line === undefined || diagnostic.line === null) return diagnostic.message;
  const column =
    diagnostic.column === undefined || diagnostic.column === null ? '' : `:${diagnostic.column}`;
  return `${diagnostic.message} (line ${diagnostic.line}${column})`;
}
