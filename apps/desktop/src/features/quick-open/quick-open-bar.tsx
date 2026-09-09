/**
 * The quick-open bar (T130-V): a field and the list under it.
 *
 * It computes nothing. Every "is there anything to show, and if not why not"
 * question was answered by `quickOpenList`, and this switches on the arm it
 * returned. A state drawn here that the model does not name is a bug in one of
 * the two, and only one of them is testable without a DOM.
 *
 * A **combobox**, not a menu: the input keeps focus and owns the keyboard, and
 * the list is addressed by `aria-activedescendant`. That is why the model's row
 * ids are positional — `id="quick-open-file:0"` — since HTML5 forbids
 * whitespace in an id and both a path and a Markdown heading routinely have one.
 *
 * Every path and symbol name here came from the workspace or a buffer, so it is
 * untrusted text (hard rule 13). All of it reaches the DOM as a text node;
 * nothing is interpolated into markup and nothing is logged.
 */

import { type KeyboardEvent, useEffect, useId, useRef } from 'react';
import {
  type HighlightRange,
  highlightSegments,
  type QuickOpenList,
  type QuickOpenRow,
  rowsOf,
} from './quick-open-model';
import { QUICK_OPEN_PREFIX } from './quick-open-parser';
import type { QuickOpenMode } from './quick-open-types';

export interface QuickOpenBarProps {
  readonly raw: string;
  readonly list: QuickOpenList;
  readonly activeId: string | null;
  readonly onRawChange: (raw: string) => void;
  /** `true` when the chord was claimed; the component then suppresses default. */
  readonly onKey: (chord: QuickOpenChordLike) => boolean;
  readonly onFocusRow: (id: string) => void;
  readonly onAccept: (id: string, options?: { preview?: boolean }) => void;
  readonly onDismiss: () => void;
  /** Focus the field on mount. The mount opens the bar, so this is per-open. */
  readonly autoFocus?: boolean;
  /**
   * Bump to focus and select the field again (T130-M §5).
   *
   * ⌘P while the bar is already open focuses and selects rather than toggling
   * it shut, so a second press is "let me retype this" and never "close it".
   * A signal rather than an imperative handle, so the mount stays declarative
   * and the behaviour is assertable without a ref.
   */
  readonly focusSignal?: number;
}

/** The structural half of a `KeyboardEvent` the keymap reads. */
export interface QuickOpenChordLike {
  readonly code: string;
  readonly key: string;
  readonly metaKey: boolean;
  readonly ctrlKey: boolean;
  readonly altKey: boolean;
  readonly shiftKey: boolean;
}

/** What the field says it is looking at, per mode. */
const PLACEHOLDER: Readonly<Record<QuickOpenMode, string>> = {
  files: 'Search files by name',
  commands: 'Run a command',
  line: 'Go to line, or line:column',
  symbols: 'Go to a symbol in this file',
};

export function QuickOpenBar({
  raw,
  list,
  activeId,
  onRawChange,
  onKey,
  onFocusRow,
  onAccept,
  onDismiss,
  autoFocus = true,
  focusSignal = 0,
}: QuickOpenBarProps) {
  const container = useRef<HTMLDivElement>(null);
  const input = useRef<HTMLInputElement>(null);
  const prefix = useId();
  const listId = `${prefix}-list`;
  const rowId = (id: string) => `${prefix}-${id}`;

  // Take the focus, and give it back on the way out.
  //
  // The bar is a transient overlay: it steals the caret from wherever the user
  // was — usually a Monaco buffer — so returning it is its own responsibility
  // and not the mount's. Restoring on unmount rather than in the Escape
  // handler covers every way the bar closes: Escape, an accepted row, a
  // command that ran, and an outside click.
  const restoreTo = useRef<HTMLElement | null>(null);
  useEffect(() => {
    if (!autoFocus) return;
    const previous = window.document.activeElement;
    restoreTo.current = previous instanceof HTMLElement ? previous : null;
    const element = container.current;
    input.current?.focus();
    return () => {
      // Restore when the focus is still ours, or when it was *orphaned* — the
      // usual case, since removing the focused input drops the focus to
      // `<body>`. What must not happen is stealing it from something that
      // deliberately took it while the bar was open.
      const owner = window.document.activeElement;
      const orphaned = owner === null || owner === window.document.body;
      const stillOurs = element !== null && owner !== null && element.contains(owner);
      if (!orphaned && !stillOurs) return;
      restoreTo.current?.focus();
    };
  }, [autoFocus]);

  // A second ⌘P: take the focus back and select what is there, so the next
  // keystroke replaces the query. Skipped on the first render, where the mount
  // effect above has already focused an empty field.
  const firstSignal = useRef(focusSignal);
  useEffect(() => {
    if (focusSignal === firstSignal.current) return;
    const element = input.current;
    element?.focus();
    element?.select();
  }, [focusSignal]);

  // Dismiss on a click outside, on resize and on scroll — the same three the
  // menu takes, and for the same reason. Capture, so the thing underneath does
  // not also activate on the dismissing click.
  useEffect(() => {
    const onPointerDown = (event: MouseEvent) => {
      const element = container.current;
      if (element && event.target instanceof Node && element.contains(event.target)) return;
      onDismiss();
    };
    window.addEventListener('mousedown', onPointerDown, true);
    window.addEventListener('resize', onDismiss);
    window.addEventListener('scroll', onDismiss, true);
    return () => {
      window.removeEventListener('mousedown', onPointerDown, true);
      window.removeEventListener('resize', onDismiss);
      window.removeEventListener('scroll', onDismiss, true);
    };
  }, [onDismiss]);

  function onKeyDown(event: KeyboardEvent<HTMLInputElement>) {
    if (onKey(event)) event.preventDefault();
  }

  const rows = rowsOf(list);

  return (
    <div className="quick-open" ref={container}>
      <input
        ref={input}
        className="quick-open-field"
        type="text"
        role="combobox"
        aria-expanded="true"
        aria-controls={listId}
        aria-autocomplete="list"
        aria-activedescendant={activeId === null ? undefined : rowId(activeId)}
        aria-label={PLACEHOLDER[list.mode]}
        placeholder={PLACEHOLDER[list.mode]}
        spellCheck={false}
        autoComplete="off"
        value={raw}
        onChange={(event) => onRawChange(event.target.value)}
        onKeyDown={onKeyDown}
      />
      <div className="quick-open-list" id={listId} role="listbox" aria-label="Results">
        {list.kind === 'loading' && (
          <p className="quick-open-note" role="status">
            Searching…
          </p>
        )}
        {list.kind === 'empty' && (
          <p className="quick-open-note" data-reason={list.reason} role="status">
            {list.message}
          </p>
        )}
        {list.kind === 'error' && (
          <p className="quick-open-note quick-open-error" role="alert">
            <span className="quick-open-error-code">{list.error.code}</span>
            {list.error.message}
          </p>
        )}
        {rows.map((row) => (
          <div
            key={row.id}
            id={rowId(row.id)}
            className="quick-open-row"
            role="option"
            // `-1`, not `0`: in a combobox the input keeps focus and the active
            // option is named by `aria-activedescendant`, so an option must be
            // addressable but must never be in the tab order. Tabbing through
            // fifty results to reach whatever is after the bar is the failure
            // this avoids.
            tabIndex={-1}
            aria-selected={row.id === activeId}
            data-kind={row.kind}
            data-active={row.id === activeId ? 'true' : undefined}
            aria-disabled={row.kind === 'command' && row.disabled !== null ? true : undefined}
            // Pointer moves the focus; it does not accept. Accepting on hover
            // is how a palette opens the wrong file.
            onMouseMove={() => onFocusRow(row.id)}
            onMouseDown={(event) => {
              // Before the field can lose focus, so the bar does not dismiss
              // itself through the outside-click listener above.
              event.preventDefault();
              onAccept(row.id, { preview: !(event.metaKey || event.ctrlKey) });
            }}
          >
            <Row row={row} />
          </div>
        ))}
        {list.kind === 'truncated' && (
          <p className="quick-open-note quick-open-truncated" role="status">
            {list.notice}
          </p>
        )}
      </div>
    </div>
  );
}

function Row({ row }: { row: QuickOpenRow }) {
  switch (row.kind) {
    case 'file':
      return (
        <>
          <span className="quick-open-title">{row.name}</span>
          <span className="quick-open-detail">
            <Highlighted text={row.path} ranges={row.ranges} />
          </span>
        </>
      );
    case 'command':
      return (
        <>
          <span className="quick-open-group">{row.group}</span>
          <span className="quick-open-title">{row.title}</span>
          {row.disabled !== null && <span className="quick-open-reason">{row.disabled}</span>}
        </>
      );
    case 'line':
      return (
        <>
          <span className="quick-open-title">{row.label}</span>
          {row.clamped && <span className="quick-open-reason">Clamped to the last line</span>}
        </>
      );
    case 'symbol':
      return (
        <>
          <span className="quick-open-symbol" data-symbol={row.symbol} aria-hidden="true" />
          <span className="quick-open-title" style={indentOf(row.depth)}>
            {row.name}
          </span>
          <span className="quick-open-detail">{`Line ${row.line}`}</span>
        </>
      );
  }
}

/**
 * The path, with the spans the scorer matched marked.
 *
 * The split is `highlightSegments`', which indexes by code point because the
 * contract's ranges are char indices. Doing it here with `slice` would be wrong
 * for any path past the BMP, and untestable without a DOM.
 */
function Highlighted({ text, ranges }: { text: string; ranges: readonly HighlightRange[] }) {
  return (
    <>
      {highlightSegments(text, ranges).map((segment, index) =>
        segment.matched ? (
          // biome-ignore lint/suspicious/noArrayIndexKey: segments are positional
          <mark key={index} className="quick-open-match">
            {segment.text}
          </mark>
        ) : (
          // biome-ignore lint/suspicious/noArrayIndexKey: segments are positional
          <span key={index}>{segment.text}</span>
        ),
      )}
    </>
  );
}

/** Markdown nests; code is flat, so this is a no-op for every code outline. */
function indentOf(depth: number) {
  return depth === 0 ? undefined : { paddingInlineStart: `${Math.min(depth, 6) * 0.75}rem` };
}

/** Re-exported so the mount can label the field without importing the parser. */
export { QUICK_OPEN_PREFIX };
