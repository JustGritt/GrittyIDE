import type { CoreErrorShape } from '@gritty/protocol';
import { describe, expect, it } from 'vitest';
import type { QuickCommand } from './quick-open-commands';
import {
  type ActiveDocument,
  type FileMatch,
  parseLineTarget,
  QUICK_OPEN_MAX_ROWS,
  type QuickOpenEmptyReason,
  type QuickOpenList,
  type QuickOpenRow,
  type QuickOpenState,
  quickOpenList,
  rowIds,
  rowsOf,
} from './quick-open-model';
import { parseQuery } from './quick-open-parser';
import type { OutlineEntry } from './quick-open-types';

/**
 * Every state the list can be in (T130-U §2).
 *
 * The model is the only thing that decides a state, so the test that matters
 * is the total one — every variant named, every empty reason reachable, none
 * left to a default nobody chose. `run-console-model.test.ts` asserts the
 * prompt gate the same way and for the same reason.
 */

const ERROR: CoreErrorShape = {
  code: 'Io',
  message: 'the workspace root went away',
  retryable: false,
};

const COMMANDS: readonly QuickCommand[] = [
  { id: 'workspace.openFolder', title: 'Open Folder…', group: 'File', run: () => {} },
  {
    id: 'file.save',
    title: 'Save',
    group: 'File',
    run: () => {},
    disabled: 'No file is open, so there is nothing to save.',
  },
  { id: 'view.toggleTerminal', title: 'Show Terminal', group: 'View', run: () => {} },
];

const OUTLINE: readonly OutlineEntry[] = [
  { name: 'renderRow', kind: 'function', line: 12, depth: 0 },
  { name: 'QuickOpenBar', kind: 'interface', line: 40, depth: 0 },
];

function files(count: number): readonly FileMatch[] {
  return Array.from({ length: count }, (_, index) => ({
    path: `src/f${index}.ts`,
    name: `f${index}.ts`,
  }));
}

function document(overrides: Partial<ActiveDocument> = {}): ActiveDocument {
  return {
    name: 'editor-pane.tsx',
    lineCount: 200,
    outline: { status: 'ready', entries: OUTLINE, truncated: false },
    ...overrides,
  };
}

/** A state for the raw field text, with the world defaulted to "everything open". */
function at(raw: string, overrides: Partial<QuickOpenState> = {}): QuickOpenState {
  return {
    query: parseQuery(raw),
    hasWorkspace: true,
    files: { status: 'ready', matches: files(3), truncated: false },
    commands: COMMANDS,
    document: document(),
    ...overrides,
  };
}

function kindsOf(list: QuickOpenList): readonly QuickOpenRow['kind'][] {
  return rowsOf(list).map((row) => row.kind);
}

describe('the files mode', () => {
  it('says to open a folder rather than searching nothing', () => {
    const list = quickOpenList(at('editor', { hasWorkspace: false }));
    expect(list).toMatchObject({ kind: 'empty', mode: 'files', reason: 'noWorkspace' });
  });

  it('asks for a query before it asks the core anything', () => {
    // Checked before the query state, so a stale answer to the previous term
    // is not drawn as this empty field's answer.
    const list = quickOpenList(
      at('', { files: { status: 'ready', matches: files(3), truncated: false } }),
    );
    expect(list).toMatchObject({ kind: 'empty', reason: 'noQuery' });
  });

  it('draws the debounce window as a wait, not as a result', () => {
    // A term with nothing asked for it yet is `idle`. Drawing "no match" there
    // would be a lie the user is about to watch being corrected.
    expect(quickOpenList(at('epn', { files: { status: 'idle' } })).kind).toBe('loading');
    expect(quickOpenList(at('epn', { files: { status: 'loading' } })).kind).toBe('loading');
  });

  it('lists what the search returned, in the order it returned it', () => {
    // Source order is the order: ranking is T130-R's, and the model must not
    // quietly impose a second one.
    const list = quickOpenList(at('f'));
    expect(list.kind).toBe('results');
    expect(rowsOf(list).map((row) => (row.kind === 'file' ? row.path : null))).toEqual([
      'src/f0.ts',
      'src/f1.ts',
      'src/f2.ts',
    ]);
  });

  it('names the term it failed to match', () => {
    const list = quickOpenList(
      at('zzz', { files: { status: 'ready', matches: [], truncated: false } }),
    );
    expect(list).toMatchObject({ kind: 'empty', reason: 'noMatch' });
    expect(list.kind === 'empty' && list.message).toContain('zzz');
  });

  it('carries the envelope when the query failed, and stays open', () => {
    const list = quickOpenList(at('epn', { files: { status: 'failed', error: ERROR } }));
    // The whole envelope, not a message: a failure has to stay inspectable.
    expect(list).toEqual({ kind: 'error', mode: 'files', error: ERROR });
  });

  it('says a truncated result is truncated, whichever end cut it', () => {
    // §4: a truncated result must not read as a complete one. The core's own
    // flag and this model's row cap are the same fact to the user — "is this
    // all of them?" — so they produce the same state.
    const fromCore = quickOpenList(
      at('f', { files: { status: 'ready', matches: files(3), truncated: true } }),
    );
    expect(fromCore.kind).toBe('truncated');
    const fromCap = quickOpenList(
      at('f', {
        files: { status: 'ready', matches: files(QUICK_OPEN_MAX_ROWS + 10), truncated: false },
      }),
    );
    expect(fromCap.kind).toBe('truncated');
    expect(rowsOf(fromCap)).toHaveLength(QUICK_OPEN_MAX_ROWS);
    expect(fromCap.kind === 'truncated' && fromCap.notice).not.toBe('');
  });

  it('does not truncate a list that exactly fills the cap', () => {
    // The off-by-one worth pinning: at the boundary nothing was dropped, so
    // claiming otherwise would be a false warning.
    const list = quickOpenList(
      at('f', {
        files: { status: 'ready', matches: files(QUICK_OPEN_MAX_ROWS), truncated: false },
      }),
    );
    expect(list.kind).toBe('results');
  });
});

describe('the commands mode', () => {
  it('lists everything on a bare >, which is what the prefix is for', () => {
    const list = quickOpenList(at('>'));
    expect(rowsOf(list)).toHaveLength(COMMANDS.length);
    expect(kindsOf(list)).toEqual(['command', 'command', 'command']);
  });

  it('works with no workspace open, unlike the other three modes', () => {
    // The ticket's failure behaviour: commands still work with nothing open.
    const list = quickOpenList(at('>', { hasWorkspace: false, document: null }));
    expect(list.kind).toBe('results');
  });

  it('matches a subsequence of the group and the title', () => {
    const list = quickOpenList(at('>trm'));
    expect(rowsOf(list).map((row) => (row.kind === 'command' ? row.commandId : null))).toEqual([
      'view.toggleTerminal',
    ]);
  });

  it('carries a disabled command through with its reason, rather than dropping it', () => {
    // Hiding it would mean the user never learns the command exists or what it
    // is waiting for; the row explains itself instead.
    const row = rowsOf(quickOpenList(at('>save')))[0];
    expect(row).toMatchObject({ kind: 'command', commandId: 'file.save' });
    expect(row?.kind === 'command' && row.disabled).toContain('nothing to save');
  });

  it('reports an enabled command as null rather than undefined', () => {
    // A row field, not an optional: the render switches on `disabled !== null`
    // and never has to think about a missing key.
    const row = rowsOf(quickOpenList(at('>open folder')))[0];
    expect(row?.kind === 'command' && row.disabled).toBeNull();
  });

  it('says nothing matched, with the term', () => {
    const list = quickOpenList(at('>zzzz'));
    expect(list).toMatchObject({ kind: 'empty', mode: 'commands', reason: 'noMatch' });
  });

  it('says something different when there are no commands at all', () => {
    // An empty term matches everything, so an empty list here means the array
    // itself is empty — and "narrow the search" would be useless advice.
    const list = quickOpenList(at('>', { commands: [] }));
    expect(list).toMatchObject({ kind: 'empty', reason: 'noMatch' });
    expect(list.kind === 'empty' && list.message).not.toContain('matches');
  });
});

describe('the line mode', () => {
  it('reads a line, and a line with a column', () => {
    expect(parseLineTarget('120')).toEqual({ line: 120, column: 1 });
    expect(parseLineTarget('120:8')).toEqual({ line: 120, column: 8 });
  });

  it('trims its own term, because a line number never contains a space', () => {
    // The files mode keeps trailing whitespace (a name may end in one); this
    // mode cannot possibly want it.
    expect(parseLineTarget(' 120 ')).toEqual({ line: 120, column: 1 });
  });

  it('refuses anything that is not a line number', () => {
    for (const term of ['abc', '12a', '1:2:3', '-4', '1.5', '', ':8']) {
      expect(parseLineTarget(term)).toBeNull();
    }
  });

  it('offers one row that already carries the resolved target', () => {
    const list = quickOpenList(at(':120:8'));
    expect(rowsOf(list)).toHaveLength(1);
    expect(rowsOf(list)[0]).toMatchObject({ kind: 'line', line: 120, column: 8, clamped: false });
  });

  it('clamps past the end of the file rather than failing', () => {
    // §7: out of range clamps to the last line. A jump that refuses because
    // the file is shorter than you guessed is worse than one that lands.
    const row = rowsOf(quickOpenList(at(':9999', { document: document({ lineCount: 200 }) })))[0];
    expect(row).toMatchObject({ kind: 'line', line: 200, clamped: true });
    // And says so, so the user is not left thinking 9999 exists.
    expect(row?.kind === 'line' && row.label).toContain('200');
  });

  it('clamps a zero or a zero column up to the first line', () => {
    // Lines and columns are 1-based (OutlineEntry.line says so); 0 is not a
    // place in a buffer.
    const zero = rowsOf(quickOpenList(at(':0')))[0];
    expect(zero).toMatchObject({ kind: 'line', line: 1, clamped: true });
    const column = rowsOf(quickOpenList(at(':10:0')))[0];
    expect(column).toMatchObject({ kind: 'line', line: 10, column: 1 });
  });

  it('survives a line number larger than a float can hold', () => {
    const row = rowsOf(quickOpenList(at(`:${'9'.repeat(400)}`)))[0];
    expect(row).toMatchObject({ kind: 'line', line: 200, clamped: true });
  });

  it('says there is nothing to jump in with no editor open', () => {
    expect(quickOpenList(at(':120', { document: null }))).toMatchObject({
      kind: 'empty',
      mode: 'line',
      reason: 'noDocument',
    });
  });

  it('asks for a number on a bare colon', () => {
    expect(quickOpenList(at(':'))).toMatchObject({ kind: 'empty', reason: 'noQuery' });
  });

  it('says a non-number is not a line number, rather than going blank', () => {
    const list = quickOpenList(at(':abc'));
    expect(list).toMatchObject({ kind: 'empty', reason: 'notALine' });
    expect(list.kind === 'empty' && list.message).toContain('abc');
  });
});

describe('the symbols mode has two empties, and they are not the same fact', () => {
  it('says the file kind has no outline rules when the rules never ran', () => {
    const list = quickOpenList(
      at('@', { document: document({ outline: { status: 'unsupported' } }) }),
    );
    expect(list).toMatchObject({ kind: 'empty', reason: 'noOutlineRules' });
  });

  it('says nothing matched when the rules ran and produced nothing for the term', () => {
    const list = quickOpenList(at('@zzzz'));
    expect(list).toMatchObject({ kind: 'empty', reason: 'noOutlineMatch' });
  });

  it('keeps the two apart by reason, not only by prose', () => {
    // The distinction the ticket asks for: "Gritty has no rules for this kind
    // of file" claims nothing about the file, while "nothing matched" claims
    // the rules ran. Collapsing them would tell a Rust user their file has no
    // functions in it because nobody wrote the rules yet.
    const noRules = quickOpenList(
      at('@', { document: document({ outline: { status: 'unsupported' } }) }),
    );
    const noEntries = quickOpenList(
      at('@', {
        document: document({ outline: { status: 'ready', entries: [], truncated: false } }),
      }),
    );
    expect(noRules.kind === 'empty' && noRules.reason).toBe('noOutlineRules');
    expect(noEntries.kind === 'empty' && noEntries.reason).toBe('noOutlineMatch');

    // An outline cut short at its own bound is incomplete even when the rows
    // fit in the list, and §4 is not scoped to the files mode: a truncated
    // list must never read as a complete one, whichever mode produced it.
    const cutShort = quickOpenList(
      at('@', {
        document: document({ outline: { status: 'ready', entries: OUTLINE, truncated: true } }),
      }),
    );
    expect(cutShort.kind).toBe('truncated');
    const whole = quickOpenList(at('@'));
    expect(whole.kind).toBe('results');
    expect(noRules.kind === 'empty' && noRules.message).not.toBe(
      noEntries.kind === 'empty' && noEntries.message,
    );
  });

  it('lists the outline it was given, keeping the line and the depth', () => {
    const list = quickOpenList(at('@'));
    expect(rowsOf(list)[0]).toMatchObject({
      kind: 'symbol',
      name: 'renderRow',
      symbol: 'function',
      line: 12,
      depth: 0,
    });
  });

  it('filters the outline by a subsequence of the name', () => {
    const list = quickOpenList(at('@qob'));
    expect(rowsOf(list).map((row) => (row.kind === 'symbol' ? row.name : null))).toEqual([
      'QuickOpenBar',
    ]);
  });

  it('needs a workspace and an editor, and says which is missing', () => {
    expect(quickOpenList(at('@', { hasWorkspace: false, document: null }))).toMatchObject({
      reason: 'noWorkspace',
    });
    expect(quickOpenList(at('@', { document: null }))).toMatchObject({ reason: 'noDocument' });
  });
});

describe('the union is total', () => {
  /** One labelled state per variant, so a missing one is a missing row here. */
  const CASES: readonly (readonly [string, QuickOpenState])[] = [
    ['files, loading', at('epn', { files: { status: 'loading' } })],
    ['files, results', at('epn')],
    [
      'files, truncated',
      at('epn', { files: { status: 'ready', matches: files(99), truncated: true } }),
    ],
    ['files, error', at('epn', { files: { status: 'failed', error: ERROR } })],
    ['files, no query', at('')],
    ['files, no workspace', at('epn', { hasWorkspace: false })],
    ['files, no match', at('zz', { files: { status: 'ready', matches: [], truncated: false } })],
    ['commands, results', at('>')],
    ['commands, no match', at('>zzzz')],
    ['line, results', at(':12')],
    ['line, no query', at(':')],
    ['line, not a line', at(':abc')],
    ['line, no document', at(':12', { document: null })],
    ['symbols, results', at('@')],
    [
      'symbols, no outline rules',
      at('@', { document: document({ outline: { status: 'unsupported' } }) }),
    ],
    ['symbols, no outline match', at('@zzzz')],
    ['symbols, no workspace', at('@', { hasWorkspace: false })],
    ['symbols, no document', at('@', { document: null })],
  ];

  it('reaches every list state the type names', () => {
    const expected: readonly QuickOpenList['kind'][] = [
      'loading',
      'results',
      'truncated',
      'empty',
      'error',
    ];
    const seen = new Set(CASES.map(([, state]) => quickOpenList(state).kind));
    expect([...seen].sort()).toEqual([...expected].sort());
  });

  it('reaches every empty reason the type names', () => {
    // The point of enumerating the reasons rather than writing prose at the
    // call site: an unreachable reason is dead code, and a missing one is a
    // state that would have gone blank.
    const expected: readonly QuickOpenEmptyReason[] = [
      'noQuery',
      'noMatch',
      'noWorkspace',
      'noDocument',
      'noOutlineRules',
      'noOutlineMatch',
      'notALine',
    ];
    const seen = new Set(
      CASES.map(([, state]) => quickOpenList(state)).flatMap((list) =>
        list.kind === 'empty' ? [list.reason] : [],
      ),
    );
    expect([...seen].sort()).toEqual([...expected].sort());
  });

  it('reaches every row kind the type names', () => {
    const expected: readonly QuickOpenRow['kind'][] = ['file', 'command', 'line', 'symbol'];
    const seen = new Set(CASES.flatMap(([, state]) => kindsOf(quickOpenList(state))));
    expect([...seen].sort()).toEqual([...expected].sort());
  });

  it('always says which mode it is in, even with nothing to show', () => {
    // A bar that goes blank without saying what it was doing is the failure §2
    // is about.
    for (const [label, state] of CASES) {
      expect(`${label}: ${quickOpenList(state).mode}`).toBe(`${label}: ${state.query.mode}`);
    }
  });

  it('never draws a blank explanation', () => {
    for (const [label, state] of CASES) {
      const list = quickOpenList(state);
      if (list.kind === 'empty') expect(`${label}: ${list.message}`).not.toBe(`${label}: `);
      if (list.kind === 'truncated') expect(`${label}: ${list.notice}`).not.toBe(`${label}: `);
    }
  });

  it('gives every row a unique, whitespace-free id', () => {
    // The id becomes an `aria-activedescendant` target, and HTML5 forbids
    // whitespace in one — which is exactly why the id is positional and the
    // path (which may contain a space) is a separate field.
    for (const [label, state] of CASES) {
      const ids = rowIds(quickOpenList(state));
      expect(`${label}: ${new Set(ids).size}`).toBe(`${label}: ${ids.length}`);
      for (const id of ids) expect(id).not.toMatch(/\s/);
    }
  });

  it('has no rows in the three states that have none', () => {
    for (const [label, state] of CASES) {
      const list = quickOpenList(state);
      if (list.kind === 'results' || list.kind === 'truncated') continue;
      expect(`${label}: ${rowsOf(list).length}`).toBe(`${label}: 0`);
    }
  });
});
