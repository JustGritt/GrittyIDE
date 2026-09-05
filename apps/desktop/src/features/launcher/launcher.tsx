import type { LastSession, RecentWorkspace, SessionTab } from '@gritty/protocol';
import { type FormEvent, useEffect, useRef, useState } from 'react';
import { ErrorEnvelope } from '../../components/error-envelope';
import type { Accelerator } from '../shell/open-shortcuts';
import {
  describeLastSession,
  formatRelativeTime,
  hasMore,
  parentOf,
  pathToOpen,
  reopenCandidate,
  visibleRecents,
} from './recents';
import { type RecentsController, useRecents } from './use-recents';

/**
 * The startup screen (T049, design artboard 3a).
 *
 * What fills the window while no workspace is open: three ways in, the
 * workspace history, and the drop hint. The shell keeps its own chrome — the
 * top bar, the status bar, the session notices and the terminal dock all stay
 * where they are, so this replaces the sidebar-and-editor body and nothing
 * else.
 *
 * Presentational: every decision it makes lives in `recents.ts`, and the
 * history itself comes from the core.
 *
 * When the core has a last session to offer (T051, artboard 3b) the screen
 * becomes two columns and the rail appears on the right. It only shows up when
 * there is something the recents list does not already say — a workspace that
 * is not open and had tabs in it — so it never repeats the row beside it.
 */
export function Launcher({
  accelerator,
  pathOpen,
  onPathOpenChange,
  onOpen,
  onPickFolder,
  lastSession,
  onResume,
  onDismissLastSession,
}: {
  /** For the shortcut hints. Null until the core has reported its platform. */
  accelerator: Accelerator | null;
  /** Controlled by the shell, because ⌘⇧O opens this field from outside. */
  pathOpen: boolean;
  onPathOpenChange(open: boolean): void;
  /** Open a known root: a recents row, or the path field. */
  onOpen(path: string): void;
  /** Open the native directory picker (the core shows it). */
  onPickFolder(): void;
  /** What the previous window had open, when there is anything worth saying. */
  lastSession: LastSession | null;
  /**
   * Open that workspace and reopen `tabs` in it. The tabs travel with the
   * call rather than being read back from state, so dismissing the rail in
   * the same click cannot race the reopen.
   */
  onResume(root: string, tabs: readonly SessionTab[]): void;
  onDismissLastSession(): void;
}) {
  // Read here rather than in the shell: this component mounts only while no
  // workspace is open, which is exactly when the history is worth a snapshot.
  // Fetching it at startup instead would miss a workspace opened and closed
  // during this run.
  const recents = useRecents();
  const [path, setPath] = useState('');
  const [expanded, setExpanded] = useState(false);
  const pathField = useRef<HTMLInputElement | null>(null);
  const { items } = recents;
  const reopen = reopenCandidate(items);
  const now = Date.now();

  // The field is worth nothing without the caret in it, and ⌘⇧O opens it from
  // outside this component.
  useEffect(() => {
    if (pathOpen) pathField.current?.focus();
  }, [pathOpen]);

  function submitPath(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const candidate = pathToOpen(path);
    if (candidate !== null) onOpen(candidate);
  }

  return (
    <section
      className={`launcher${lastSession ? ' launcher--with-rail' : ''}`}
      aria-labelledby="launcher-title"
    >
      <div className="launcher__panel">
        <div className="launcher__intro">
          {/* h2, not h1: the app bar's wordmark is the document's heading and
              this repeats it. Size comes from the class, not the level. */}
          <h2 className="launcher__title" id="launcher-title">
            Gritty
          </h2>
          <p className="launcher__tagline">
            Agent-native development environment. Open a workspace to edit files, run shells and
            supervise agents.
          </p>
        </div>

        {recents.error && <ErrorEnvelope error={recents.error} onDismiss={recents.clearError} />}

        <div className="launcher__ways">
          <button type="button" className="launcher__way" onClick={onPickFolder}>
            <span className="launcher__glyph launcher__glyph--folder" aria-hidden="true" />
            <span className="launcher__way-name">Open folder…</span>
            <span className="launcher__way-hint">{chordLabel(accelerator, false)}</span>
          </button>

          <button
            type="button"
            className="launcher__way"
            aria-expanded={pathOpen}
            onClick={() => onPathOpenChange(!pathOpen)}
          >
            <span className="launcher__glyph" aria-hidden="true">
              /
            </span>
            <span className="launcher__way-name">Open path…</span>
            <span className="launcher__way-hint">{chordLabel(accelerator, true)}</span>
          </button>

          {/* Only when there is somewhere to go back to. Artboard 3a shows
              "Restore session" here; Gritty reopens the last workspace by
              itself (T060), so what is actually useful in this state is the
              most recent workspace that is still on disk. */}
          {reopen && (
            <button type="button" className="launcher__way" onClick={() => onOpen(reopen.root)}>
              <span className="launcher__glyph launcher__glyph--reopen" aria-hidden="true">
                ↺
              </span>
              <span className="launcher__way-name">Reopen {reopen.name}</span>
              <span className="launcher__way-hint">
                {formatRelativeTime(reopen.lastOpenedAt, now)}
              </span>
            </button>
          )}
        </div>

        {pathOpen && (
          <form className="launcher__path" onSubmit={submitPath}>
            <input
              ref={pathField}
              value={path}
              onChange={(event) => setPath(event.target.value)}
              placeholder="absolute path"
              aria-label="Workspace path"
              spellCheck={false}
              onKeyDown={(event) => {
                if (event.key === 'Escape') onPathOpenChange(false);
              }}
            />
            <button type="submit" className="btn--primary" disabled={pathToOpen(path) === null}>
              Open
            </button>
          </form>
        )}

        {items.length > 0 && (
          <div className="launcher__recents">
            <div className="launcher__recents-head">
              <span className="label">Recent workspaces</span>
              <span className="launcher__spacer" />
              {hasMore(items) && (
                <button
                  type="button"
                  className="btn--bare"
                  onClick={() => setExpanded((open) => !open)}
                >
                  {expanded ? 'Show fewer' : `View all (${items.length})`}
                </button>
              )}
            </div>
            <ul className="launcher__list">
              {visibleRecents(items, expanded).map((item) => (
                <RecentRow
                  key={item.root}
                  item={item}
                  now={now}
                  onOpen={onOpen}
                  recents={recents}
                />
              ))}
            </ul>
          </div>
        )}

        {/* A hint, not a second implementation: the drop gesture is handled by
            the core and `features/drop`, and already works with no workspace. */}
        <p className="launcher__drop">
          <span className="launcher__dot" aria-hidden="true" />
          Drop a folder anywhere in the window to open it as a workspace.
        </p>
      </div>

      {lastSession && (
        <LastSessionRail
          last={lastSession}
          now={now}
          onResume={onResume}
          onDismiss={onDismissLastSession}
        />
      )}
    </section>
  );
}

/**
 * The right-hand rail (artboard 3b): what the last window had open, and one
 * click to get it back.
 *
 * The artboard offers "Restore all / Files only / Discard". Shells are not
 * persisted per workspace and agent worktrees do not exist, so "all" and
 * "files only" would be the same button pressed twice — this offers the two
 * choices that actually differ, and Dismiss hides the rail without deleting
 * anything (the tabs and any recovery snapshots stay where they are).
 */
function LastSessionRail({
  last,
  now,
  onResume,
  onDismiss,
}: {
  last: LastSession;
  now: number;
  onResume(root: string, tabs: readonly SessionTab[]): void;
  onDismiss(): void;
}) {
  const { workspace, tabs, unsaved } = last;
  return (
    <aside className="launcher__rail" aria-label="Last session">
      <div className="launcher__rail-head">
        <span className="label">Last session</span>
        <span className="launcher__spacer" />
        <span className="launcher__rail-when">
          {formatRelativeTime(workspace.lastOpenedAt, now)}
        </span>
      </div>

      <div className="launcher__rail-card">
        <div className="launcher__rail-name">{workspace.name}</div>
        <code className="launcher__rail-path" title={workspace.root}>
          {workspace.displayRoot}
        </code>
        <p className="launcher__rail-body">{describeLastSession(tabs.length, unsaved)}</p>
        {workspace.exists ? (
          <div className="launcher__rail-actions">
            <button
              type="button"
              className="btn--primary"
              onClick={() => onResume(workspace.root, tabs)}
            >
              Restore
            </button>
            <button type="button" onClick={() => onResume(workspace.root, [])}>
              Open without them
            </button>
            <button type="button" className="btn--bare" onClick={onDismiss}>
              Dismiss
            </button>
          </div>
        ) : (
          <div className="launcher__rail-actions">
            {/* Nothing to open: say why rather than offering a button that
                would fail. The history row beside it offers Forget. */}
            <span className="launcher__rail-gone">That folder is no longer there.</span>
            <button type="button" className="btn--bare" onClick={onDismiss}>
              Dismiss
            </button>
          </div>
        )}
      </div>

      <div className="launcher__rail-tabs">
        <span className="label">Open tabs</span>
        <ul>
          {tabs.map((tab) => (
            <li key={tab.path}>
              <span className="launcher__rail-tab">{tab.path}</span>
              {tab.active && <span className="launcher__rail-active">active</span>}
            </li>
          ))}
        </ul>
      </div>

      {unsaved > 0 && (
        <p className="launcher__rail-note">
          <span className="launcher__dot launcher__dot--warn" aria-hidden="true" />
          Restoring keeps those buffers dirty; the files on disk are untouched.
        </p>
      )}
    </aside>
  );
}

function RecentRow({
  item,
  now,
  onOpen,
  recents,
}: {
  item: RecentWorkspace;
  now: number;
  onOpen(path: string): void;
  recents: RecentsController;
}) {
  return (
    <li className={`launcher__row${item.exists ? '' : ' launcher__row--missing'}`}>
      <button
        type="button"
        className="launcher__recent"
        onClick={() => onOpen(item.root)}
        disabled={!item.exists}
        title={item.root}
      >
        <span className="launcher__recent-name">{item.name}</span>
        {item.exists ? (
          <span className="launcher__recent-when">
            {formatRelativeTime(item.lastOpenedAt, now)}
          </span>
        ) : (
          <span className="launcher__recent-gone">root missing</span>
        )}
        <span className="launcher__spacer" />
        <span className="launcher__recent-parent">{parentOf(item.displayRoot)}</span>
      </button>
      {/* Offered on every row, not only the broken ones: a list you cannot
          prune is one that only grows. */}
      <button
        type="button"
        className="btn--bare launcher__forget"
        onClick={() => recents.forget(item.root)}
        title={`Forget ${item.name}`}
      >
        Forget
      </button>
    </li>
  );
}

/** "⌘O" on macOS, "Ctrl+O" elsewhere; nothing at all until the core says. */
function chordLabel(accelerator: Accelerator | null, shift: boolean): string {
  if (accelerator === null) return '';
  if (accelerator === 'meta') return shift ? '⌘⇧O' : '⌘O';
  return shift ? 'Ctrl+Shift+O' : 'Ctrl+O';
}
