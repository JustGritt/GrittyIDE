import type { RecentWorkspace } from '@gritty/protocol';
import { type FormEvent, useEffect, useRef, useState } from 'react';
import { ErrorEnvelope } from '../../components/error-envelope';
import type { Accelerator } from '../shell/open-shortcuts';
import {
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
 */
export function Launcher({
  accelerator,
  pathOpen,
  onPathOpenChange,
  onOpen,
  onPickFolder,
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
    <section className="launcher" aria-labelledby="launcher-title">
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
    </section>
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
