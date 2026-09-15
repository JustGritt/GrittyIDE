import type { SearchMatch, SearchScope, WorkspacePath } from '@gritty/protocol';
import { type KeyboardEvent, useEffect, useRef } from 'react';
import { ErrorEnvelope } from '../../components/error-envelope';
import { FileIcon, fileIconKind } from '../../components/file-icon';
import { ChevronIcon } from '../../components/icons';
import { fileName } from '../editor/file-name';
import { fileDirectory } from '../git/git-model';
import { type FilePage, headlineOf, highlightOf, notesOf, pageOf, plural } from './search-model';
import { isActive, type SearchRun } from './search-reducer';
import type { SearchController, SearchToggles } from './use-search';

/** Where a result points: a line and column for a content match, nothing for a name. */
export interface SearchTarget {
  readonly line: number;
  readonly column: number;
}

const SCOPES: readonly { readonly value: SearchScope; readonly label: string }[] = [
  { value: 'both', label: 'Contents and names' },
  { value: 'content', label: 'Contents' },
  { value: 'filenames', label: 'Names' },
];

const TOGGLES: readonly {
  readonly key: Exclude<keyof SearchToggles, 'scope'>;
  readonly label: string;
  readonly glyph: string;
}[] = [
  { key: 'caseSensitive', label: 'Match case', glyph: 'Aa' },
  { key: 'wholeWord', label: 'Match whole word', glyph: 'ab' },
  { key: 'useRegex', label: 'Use regular expression', glyph: '.*' },
];

/** The id the field's inline error is described by. */
const INVALID_ID = 'search-invalid';

/**
 * The **Search** tab (T080-U): find-in-files over `search.query`.
 *
 * A view of the controller the shell owns (`use-search.ts`), so it can be
 * unmounted whenever another sidebar section is showing without losing
 * anything. What each run arm says is `search-model.ts`'s; this file lays it
 * out and routes the keyboard.
 */
export function SearchPanel({
  search,
  onOpen,
}: {
  search: SearchController;
  /** `preview` is a single click, as in the tree; a double click keeps the tab. */
  onOpen: (path: WorkspacePath, target: SearchTarget | null, preview: boolean) => void;
}) {
  const input = useRef<HTMLInputElement | null>(null);
  const results = useRef<HTMLDivElement | null>(null);
  const { run } = search;
  const hasWorkspace = search.workspaceId !== null;

  // Opening the tab is asking to search, so the field takes focus — unless the
  // tab was reached from the keyboard, where the tablist keeps focus so Arrow
  // can carry on past it (the APG's tabs pattern). A click in WebKit does not
  // focus the button, which is what tells the two apart.
  useEffect(() => {
    const active = document.activeElement;
    if (active instanceof HTMLElement && active.closest('[role="tablist"]') !== null) return;
    input.current?.focus();
  }, []);

  const invalid = run !== null && isInvalidInput(run) ? run.error : null;

  function moveFocus(event: KeyboardEvent<HTMLElement>) {
    const container = results.current;
    if (container === null) return;
    const rows = Array.from(container.querySelectorAll<HTMLElement>('[data-search-row]'));
    if (rows.length === 0) return;
    const here = rows.indexOf(document.activeElement as HTMLElement);
    let next: number;
    switch (event.key) {
      case 'ArrowDown':
        next = here === -1 ? 0 : Math.min(rows.length - 1, here + 1);
        break;
      case 'ArrowUp':
        if (here <= 0) {
          // Up from the first row goes back to the field, where the query is.
          if (here === 0) {
            event.preventDefault();
            input.current?.focus();
          }
          return;
        }
        next = here - 1;
        break;
      case 'Home':
        next = 0;
        break;
      case 'End':
        next = rows.length - 1;
        break;
      default:
        return;
    }
    event.preventDefault();
    rows[next]?.focus();
  }

  return (
    <div className="search">
      <form
        className="search__form"
        onSubmit={(event) => {
          event.preventDefault();
          search.submit();
        }}
      >
        <div className="search__field">
          <input
            ref={input}
            className="search__input"
            value={search.query}
            onChange={(event) => search.setQuery(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === 'ArrowDown') moveFocus(event);
            }}
            placeholder={hasWorkspace ? 'Search' : 'No workspace open'}
            aria-label="Search in files"
            aria-invalid={invalid !== null}
            aria-describedby={invalid !== null ? INVALID_ID : undefined}
            spellCheck={false}
            autoComplete="off"
            disabled={!hasWorkspace}
          />
          <span className="search__toggles">
            {TOGGLES.map((toggle) => (
              <button
                key={toggle.key}
                type="button"
                className={`search__toggle${search.toggles[toggle.key] ? ' search__toggle--on' : ''}`}
                aria-pressed={search.toggles[toggle.key]}
                aria-label={toggle.label}
                title={toggle.label}
                disabled={!hasWorkspace}
                onClick={() => search.setToggle(toggle.key, !search.toggles[toggle.key])}
              >
                {toggle.glyph}
              </button>
            ))}
          </span>
        </div>
        <label className="search__scope">
          <span>Look in</span>
          <select
            value={search.toggles.scope}
            disabled={!hasWorkspace}
            onChange={(event) => search.setToggle('scope', event.target.value as SearchScope)}
          >
            {SCOPES.map((scope) => (
              <option key={scope.value} value={scope.value}>
                {scope.label}
              </option>
            ))}
          </select>
        </label>
        {invalid !== null && (
          <p className="search__invalid" id={INVALID_ID}>
            {invalid.message}
          </p>
        )}
      </form>

      {!hasWorkspace ? (
        <p className="search__empty">Open a folder to search it.</p>
      ) : run === null || invalid !== null ? null : (
        <RunView
          run={run}
          search={search}
          onOpen={onOpen}
          results={results}
          onKeyDown={moveFocus}
        />
      )}
    </div>
  );
}

function RunView({
  run,
  search,
  onOpen,
  results,
  onKeyDown,
}: {
  run: SearchRun;
  search: SearchController;
  onOpen: (path: WorkspacePath, target: SearchTarget | null, preview: boolean) => void;
  results: React.RefObject<HTMLDivElement | null>;
  onKeyDown: (event: KeyboardEvent<HTMLElement>) => void;
}) {
  const active = isActive(run);
  const headline = headlineOf(run);
  const page = pageOf(run.files, search.limit, search.collapsed);
  return (
    <>
      <div className="search__status">
        {/* Announced when it settles, not on every batch: a count that changes
            twenty times a second is noise to a screen reader. */}
        {headline !== null && (
          <span className="search__headline" aria-live={active ? 'off' : 'polite'}>
            {headline}
          </span>
        )}
        {active && (
          <button type="button" onClick={search.stop} disabled={run.stopRequested}>
            Stop
          </button>
        )}
      </div>
      {notesOf(run).map((note) => (
        <p key={note.text} className={`search__note search__note--${note.tone}`}>
          {note.text}
        </p>
      ))}
      {run.phase === 'failed' && run.error !== null && <ErrorEnvelope error={run.error} />}
      {/* Keyboard routing on the container: the rows are plain buttons, and
          Arrow moves between them the way it moves through the tree. */}
      {/* biome-ignore lint/a11y/noStaticElementInteractions: the handler only routes Arrow keys between the buttons inside */}
      <div className="search__results" ref={results} onKeyDown={onKeyDown}>
        <ul className="search__files">
          {page.files.map((entry) => (
            <FileGroup
              key={entry.file.path}
              entry={entry}
              collapsed={search.collapsed.has(entry.file.path)}
              onToggle={() => search.toggleFile(entry.file.path)}
              onOpen={onOpen}
            />
          ))}
        </ul>
        {page.hidden > 0 && (
          <p className="search__more">
            <span>{plural(page.hidden, 'more result')} not shown.</span>
            <button type="button" onClick={search.showMore}>
              Show more
            </button>
          </p>
        )}
      </div>
    </>
  );
}

function FileGroup({
  entry,
  collapsed,
  onToggle,
  onOpen,
}: {
  entry: FilePage;
  collapsed: boolean;
  onToggle: () => void;
  onOpen: (path: WorkspacePath, target: SearchTarget | null, preview: boolean) => void;
}) {
  const { file, lines, count } = entry;
  const name = fileName(file.path);
  const directory = fileDirectory(file.path);
  const hiddenHere = collapsed ? 0 : count - lines.length - (file.nameMatch === null ? 0 : 1);
  return (
    <li className="search__file">
      <button
        type="button"
        className={`search__file-row${collapsed ? '' : ' search__file-row--open'}`}
        aria-expanded={!collapsed}
        title={file.path}
        data-search-row=""
        onClick={onToggle}
      >
        <span className="search__chevron">
          <ChevronIcon />
        </span>
        <FileIcon kind={fileIconKind(name)} />
        <span className="search__name">{name}</span>
        {directory !== null && <span className="search__dir">{directory}</span>}
        <span className="badge search__count" title={plural(count, 'result')}>
          {count}
        </span>
      </button>
      {!collapsed && (
        <ul className="search__matches" aria-label={`Results in ${file.path}`}>
          {file.nameMatch !== null && (
            <MatchRow match={file.nameMatch} label="name" target={null} onOpen={onOpen} />
          )}
          {lines.map((match) => (
            <MatchRow
              key={`${match.line}:${match.column}`}
              match={match}
              label={String(match.line ?? '')}
              target={
                match.line === undefined ? null : { line: match.line, column: match.column ?? 1 }
              }
              onOpen={onOpen}
            />
          ))}
          {hiddenHere > 0 && (
            <li className="search__cut">
              {plural(hiddenHere, 'more result')} in this file not shown
            </li>
          )}
        </ul>
      )}
    </li>
  );
}

function MatchRow({
  match,
  label,
  target,
  onOpen,
}: {
  match: SearchMatch;
  label: string;
  target: SearchTarget | null;
  onOpen: (path: WorkspacePath, target: SearchTarget | null, preview: boolean) => void;
}) {
  const { before, match: hit, after } = highlightOf(match);
  const where = target === null ? match.path : `${match.path}:${target.line}:${target.column}`;
  return (
    <li>
      <button
        type="button"
        className="search__match"
        data-search-row=""
        title={match.truncated ? `${where} — the line is longer than shown` : where}
        onClick={() => onOpen(match.path, target, true)}
        onDoubleClick={() => onOpen(match.path, target, false)}
      >
        <span className="search__line">{label}</span>
        <span className="search__snippet">
          {before}
          <mark className="search__hit">{hit}</mark>
          {after}
        </span>
      </button>
    </li>
  );
}

/**
 * A refusal before any operation existed, for a query the core could not
 * accept — a pattern that does not compile. Said beside the field rather than
 * as an alert: it is about what was typed, and typing is how it is fixed.
 */
function isInvalidInput(run: SearchRun): boolean {
  return run.phase === 'failed' && run.operationId === null && run.error?.code === 'InvalidInput';
}
