/**
 * The quick-open controller (T130-V): everything the bar needs that a pure
 * module cannot be.
 *
 * It owns the field text, the files query and the focused row, and it assembles
 * the `QuickOpenState` that `quickOpenList` turns into rows. It does **not** own
 * whether the bar is open — `AppBar` does, the way it already owns the health
 * popover's (T130-M §1) — so this hook is mounted whether or not the overlay is
 * showing and simply has nothing to do when the field is empty.
 *
 * Nothing here decides a list state. Every "is there anything to show, and if
 * not why not" question is `quick-open-model.ts`'s.
 */

import type { CoreErrorShape, FoundFiles, OperationId, WorkspaceId } from '@gritty/protocol';
import { CoreError } from '@gritty/protocol';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { core } from '../../core/client';
import { emptySelection, focusAt, moveFocus, type Selection } from '../explorer/explorer-selection';
import { hasOutlineRules, outlineOf } from './outline';
import {
  type QuickCommandDeps,
  type QuickCommandDocument,
  quickOpenCommands,
} from './quick-open-commands';
import { type QuickOpenChord, quickOpenIntentFor } from './quick-open-keymap';
import {
  type ActiveDocument,
  type FileMatch,
  type FileQueryState,
  type OutlineState,
  QUICK_OPEN_MAX_ROWS,
  type QuickOpenList,
  quickOpenList,
  rowIds,
  rowsOf,
} from './quick-open-model';
import { parseQuery } from './quick-open-parser';

/**
 * How long the field waits before asking the core.
 *
 * A ranked walk is a whole-tree walk — there is no index, deliberately (T130-R)
 * — so a query per keystroke would start and cancel one walk per character.
 * Long enough that typing a five-letter name costs one walk, short enough that
 * a pause reads as instant.
 */
export const QUICK_OPEN_DEBOUNCE_MS = 120;

/** The document the `:` and `@` modes act on, and that commands ask about. */
export interface QuickOpenDocument extends QuickCommandDocument {
  /** Decides which outline rules apply, or that none do. */
  readonly languageId: string;
  /** The buffer. The outline is extracted here, with no round trip. */
  readonly text: string;
  readonly lineCount: number;
}

/** What the mount supplies. Everything the controller cannot know by itself. */
export interface QuickOpenDeps extends Omit<QuickCommandDeps, 'document'> {
  /** `null` with no workspace open: files and symbols then say so. */
  readonly workspaceId: WorkspaceId | null;
  readonly document: QuickOpenDocument | null;
  /** Enter opens a preview tab; ⌘Enter a permanent one (T130-U §5). */
  readonly openFile: (path: string, options: { preview: boolean }) => void;
  /** `@` and `:` move the cursor and open nothing. */
  readonly revealLine: (line: number, column: number) => void;
  /** Escape, an accepted row, and a command that ran all close the bar. */
  readonly onDismiss: () => void;
}

export interface QuickOpenController {
  /** The raw field text, prefix and all. */
  readonly raw: string;
  readonly list: QuickOpenList;
  /** The `aria-activedescendant` target, or `null` before anything is focused. */
  readonly activeId: string | null;
  readonly setRaw: (raw: string) => void;
  /** Returns `true` when the chord was claimed and the event should not reach the input. */
  readonly handleKey: (chord: QuickOpenChord) => boolean;
  /** Pointer selection: focus a row without accepting it. */
  readonly focusRow: (id: string) => void;
  readonly accept: (id: string, options?: { preview?: boolean }) => void;
  /** Called by the mount when the bar opens, so a stale query is not shown. */
  readonly reset: (raw?: string) => void;
}

export function useQuickOpen(deps: QuickOpenDeps): QuickOpenController {
  const [raw, setRawText] = useState('');
  const [files, setFiles] = useState<FileQueryState>({ status: 'idle' });
  const [focused, setFocused] = useState<string | null>(null);

  const query = useMemo(() => parseQuery(raw), [raw]);
  const { workspaceId, document, openFile, revealLine, onDismiss } = deps;

  // The outline is derived from the buffer, so it is recomputed when the buffer
  // changes and not on every keystroke in the field. `outlineOf` is bounded
  // (OUTLINE_LIMIT, and a byte cap on markdown) but it is not free.
  const outline = useMemo((): OutlineState => {
    if (document === null || !hasOutlineRules(document.languageId)) {
      return { status: 'unsupported' };
    }
    const result = outlineOf(document.languageId, document.text);
    if (!result.supported) return { status: 'unsupported' };
    return { status: 'ready', entries: result.entries, truncated: result.truncated };
  }, [document]);

  const commands = useMemo(
    () =>
      quickOpenCommands({
        hasWorkspace: deps.hasWorkspace,
        document: document === null ? null : { name: document.name, dirty: document.dirty },
        terminalVisible: deps.terminalVisible,
        openFolder: deps.openFolder,
        saveDocument: deps.saveDocument,
        toggleTerminal: deps.toggleTerminal,
        openSettings: deps.openSettings,
      }),
    [
      deps.hasWorkspace,
      document,
      deps.terminalVisible,
      deps.openFolder,
      deps.saveDocument,
      deps.toggleTerminal,
      deps.openSettings,
    ],
  );

  const activeDocument = useMemo((): ActiveDocument | null => {
    if (document === null) return null;
    return { name: document.name, lineCount: document.lineCount, outline };
  }, [document, outline]);

  const list = useMemo(
    () =>
      quickOpenList({
        query,
        hasWorkspace: deps.hasWorkspace,
        files,
        commands,
        document: activeDocument,
      }),
    [query, deps.hasWorkspace, files, commands, activeDocument],
  );

  // One files query in flight, superseded rather than raced (T130-U §3).
  //
  // The sequence number is what makes a late answer harmless: the abort below
  // stops the *walk*, but a response already in flight over the IPC cannot be
  // recalled, and rendering it would show the previous term's rows under this
  // term's text.
  const sequence = useRef(0);
  const isFiles = query.mode === 'files';
  const term = query.term;

  useEffect(() => {
    if (!isFiles || term === '' || workspaceId === null) {
      setFiles({ status: 'idle' });
      return;
    }
    const mine = ++sequence.current;
    const abort = new AbortController();
    const timer = setTimeout(() => {
      setFiles({ status: 'loading' });
      findFilesAwaitingOperation(workspaceId, term, abort.signal).then(
        (found) => {
          if (mine !== sequence.current) return;
          setFiles({
            status: 'ready',
            matches: found.matches.map(toFileMatch),
            truncated: found.truncated,
          });
        },
        (error: unknown) => {
          if (mine !== sequence.current) return;
          // Cancellation is not a failure and must not change the rows: the
          // user superseded this query by typing, and an error row under their
          // new term would be a lie about the new term (T130-U failure
          // behaviour).
          if (abort.signal.aborted || isCancellation(error)) return;
          setFiles({ status: 'failed', error: toShape(error) });
        },
      );
    }, QUICK_OPEN_DEBOUNCE_MS);
    return () => {
      clearTimeout(timer);
      abort.abort();
    };
  }, [isFiles, term, workspaceId]);

  // Focus follows the rows: a row that scrolled away must not keep the
  // keyboard, and ids are positional so the *position* is what survives a
  // debounced list settling under the cursor.
  const ids = useMemo(() => rowIds(list), [list]);
  useEffect(() => {
    setFocused((current) => (current !== null && ids.includes(current) ? current : null));
  }, [ids]);

  const accept = useCallback(
    (id: string, options?: { preview?: boolean }) => {
      const row = rowsOf(list).find((candidate) => candidate.id === id);
      if (row === undefined) return;
      switch (row.kind) {
        case 'file':
          openFile(row.path, { preview: options?.preview ?? true });
          onDismiss();
          return;
        case 'command': {
          // A disabled row explains itself and does nothing; running it anyway
          // would be the one case the reason string exists to prevent.
          if (row.disabled !== null) return;
          const command = commands.find((candidate) => candidate.id === row.commandId);
          onDismiss();
          command?.run();
          return;
        }
        case 'line':
        case 'symbol':
          revealLine(row.line, row.kind === 'line' ? row.column : 1);
          onDismiss();
          return;
      }
    },
    [list, commands, openFile, revealLine, onDismiss],
  );

  const handleKey = useCallback(
    (chord: QuickOpenChord): boolean => {
      const intent = quickOpenIntentFor(chord);
      // Everything unclaimed reaches the input, which is what makes typing work
      // (T130-U §8).
      if (intent === null) return false;
      switch (intent.kind) {
        case 'dismiss':
          onDismiss();
          return true;
        case 'move':
          setFocused((current) => moveFocusId(current, ids, intent.delta));
          return true;
        case 'edge':
          setFocused(() => edgeId(ids, intent.edge));
          return true;
        case 'accept': {
          // Nothing focused yet accepts the first row, which is what a
          // quick-open does when you type and press Enter.
          const target = focused ?? ids[0];
          if (target === undefined) return true;
          accept(target, { preview: intent.preview });
          return true;
        }
      }
    },
    [ids, focused, accept, onDismiss],
  );

  const reset = useCallback((next = '') => {
    // A new sequence, so an answer to the query that was in flight when the bar
    // closed cannot land in the reopened one.
    sequence.current += 1;
    setRawText(next);
    setFiles({ status: 'idle' });
    setFocused(null);
  }, []);

  return {
    raw,
    list,
    activeId: focused,
    setRaw: setRawText,
    handleKey,
    focusRow: setFocused,
    accept,
    reset,
  };
}

/** `FoundFile` → the two-and-a-bit facts a row needs. */
function toFileMatch(found: FoundFiles['matches'][number]): FileMatch {
  const separator = found.path.lastIndexOf('/');
  return {
    path: found.path,
    name: separator === -1 ? found.path : found.path.slice(separator + 1),
    ranges: found.ranges,
  };
}

/**
 * `workspace.findFiles`, with the `pending` arm awaited.
 *
 * On any repository of size `pending` is the normal answer — T080 measured
 * 10,429 files at 94 ms against a 100 ms budget — so a caller that only handled
 * `ready` would work on a fixture and hang on a real project. Both arms end
 * with the same `FoundFiles`, so they converge here and the controller above
 * never learns which one answered.
 *
 * `signal` aborts by cancelling the *operation*, which is the one cancellation
 * surface (`operations.cancel`); there is no `findFiles.cancel`.
 */
export async function findFilesAwaitingOperation(
  workspaceId: WorkspaceId,
  query: string,
  signal: AbortSignal,
  limit = QUICK_OPEN_MAX_ROWS,
): Promise<FoundFiles> {
  const response = await core.workspace.findFiles({ workspaceId, query, limit });
  if (response.status === 'ready') {
    return { matches: response.matches, truncated: response.truncated };
  }
  const id = response.operation.id;
  if (signal.aborted) {
    void core.operations.cancel(id).catch(() => undefined);
    throw new DOMException('superseded', 'AbortError');
  }
  return await awaitOperation(id, signal);
}

/**
 * Resolve when the operation ends, including when it ended before we asked.
 *
 * The race this closes is real and would hang the bar in `loading` for ever: a
 * walk that finishes one millisecond after the budget emits `OperationCompleted`
 * while the `pending` response is still crossing the IPC, so by the time this
 * subscribes the event is gone. ADR-009 orders a command against the *listener
 * registration*, not against events that follow the response. So the terminal
 * events are buffered from module load and checked here first.
 */
function awaitOperation(id: OperationId, signal: AbortSignal): Promise<FoundFiles> {
  return new Promise<FoundFiles>((resolve, reject) => {
    const settled = terminalEvents.get(id);
    if (settled !== undefined) {
      terminalEvents.delete(id);
      finish(settled);
      return;
    }
    const waiter = (outcome: TerminalOutcome) => {
      cleanup();
      finish(outcome);
    };
    waiters.set(id, waiter);
    const onAbort = () => {
      cleanup();
      void core.operations.cancel(id).catch(() => undefined);
      reject(new DOMException('superseded', 'AbortError'));
    };
    signal.addEventListener('abort', onAbort, { once: true });

    function cleanup() {
      waiters.delete(id);
      signal.removeEventListener('abort', onAbort);
    }
    function finish(outcome: TerminalOutcome) {
      if (outcome.kind === 'completed') resolve(outcome.found);
      else if (outcome.kind === 'cancelled') reject(new DOMException('cancelled', 'AbortError'));
      else reject(new CoreError(outcome.error));
    }
  });
}

type TerminalOutcome =
  | { kind: 'completed'; found: FoundFiles }
  | { kind: 'failed'; error: CoreErrorShape }
  | { kind: 'cancelled' };

/**
 * Outcomes seen before anyone asked for them, newest last.
 *
 * Bounded (rule 8) and tiny: at most one query is in flight, so this holds an
 * outcome for at most the microtask between the response landing and the
 * `await` above. The cap is what stops a leak if a caller ever vanishes between
 * the two.
 */
const terminalEvents = new Map<OperationId, TerminalOutcome>();
const waiters = new Map<OperationId, (outcome: TerminalOutcome) => void>();
const TERMINAL_BUFFER = 16;

function record(id: OperationId, outcome: TerminalOutcome): void {
  const waiter = waiters.get(id);
  if (waiter !== undefined) {
    waiter(outcome);
    return;
  }
  terminalEvents.set(id, outcome);
  while (terminalEvents.size > TERMINAL_BUFFER) {
    const oldest = terminalEvents.keys().next();
    if (oldest.done === true) break;
    terminalEvents.delete(oldest.value);
  }
}

core.subscribe('OperationCompleted', (event) => {
  if (event.payload.operation.kind !== FIND_FILES_KIND) return;
  record(event.payload.operation.id, {
    kind: 'completed',
    found: event.payload.result as FoundFiles,
  });
});
core.subscribe('OperationFailed', (event) => {
  if (event.payload.operation.kind !== FIND_FILES_KIND) return;
  record(event.payload.operation.id, { kind: 'failed', error: event.payload.error });
});
core.subscribe('OperationCancelled', (event) => {
  if (event.payload.operation.kind !== FIND_FILES_KIND) return;
  record(event.payload.operation.id, { kind: 'cancelled' });
});

/** `FIND_KIND` in `crates/gritty-core/src/workspace/find.rs`. */
const FIND_FILES_KIND = 'workspace.findFiles';

function isCancellation(error: unknown): boolean {
  if (error instanceof DOMException) return error.name === 'AbortError';
  return error instanceof CoreError && error.code === 'Cancelled';
}

function toShape(error: unknown): CoreErrorShape {
  if (error instanceof CoreError) return error;
  return {
    code: 'Internal',
    message: error instanceof Error ? error.message : 'the file search failed',
    retryable: false,
  };
}

/**
 * The explorer's list algebra, over row ids (T130-U §9).
 *
 * Not reimplemented: `moveFocus` already clamps rather than wrapping, and
 * already lands Down on the first row and Up on the last when nothing is
 * focused — both of which the ticket asks for by name. Only the shape differs,
 * and a `Selection` of one row is the whole adaptation.
 */
function selectionOf(current: string | null): Selection {
  return current === null
    ? emptySelection
    : { paths: [current], anchor: current, focused: current };
}

function moveFocusId(current: string | null, ids: readonly string[], delta: number): string | null {
  return moveFocus(selectionOf(current), ids, delta).focused;
}

function edgeId(ids: readonly string[], edge: 'first' | 'last'): string | null {
  return focusAt(emptySelection, ids, edge === 'first' ? 0 : ids.length - 1).focused;
}
