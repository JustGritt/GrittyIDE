import type { CoreErrorShape } from '@gritty/protocol';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import { QuickOpenBar, type QuickOpenBarProps } from './quick-open-bar';
import type { QuickCommand } from './quick-open-commands';
import {
  type FileMatch,
  QUICK_OPEN_MAX_ROWS,
  type QuickOpenList,
  type QuickOpenState,
  quickOpenList,
} from './quick-open-model';
import { parseQuery } from './quick-open-parser';
import type { OutlineEntry } from './quick-open-types';

/**
 * What the bar actually puts on the screen (T130-V).
 *
 * Static markup rather than a DOM, matching `run-console-render.test.ts`:
 * `apps/desktop`'s vitest project is a node one, and a string is a stronger
 * assertion than a query against a tree. The half that genuinely needs a
 * browser — focus, arrow keys, outside-click — is `quick-open.dom.test.tsx`.
 *
 * Every callback here is a `refuse()` that throws, so a render that reaches the
 * controller fails loudly rather than silently doing work during a render.
 */

/** The tags, with their attribute *values* blanked. See `run-console-render.test.ts`. */
function tagsOf(markup: string): readonly string[] {
  return (markup.match(/<[^>]*>/g) ?? []).map((tag) => tag.replace(/"[^"]*"/g, '""'));
}

const refuse = (what: string) => () => {
  throw new Error(`nothing should ${what} while rendering`);
};

const COMMANDS: readonly QuickCommand[] = [
  { id: 'workspace.openFolder', title: 'Open Folder…', group: 'File', run: refuse('run') },
  {
    id: 'file.save',
    title: 'Save',
    group: 'File',
    run: refuse('run'),
    disabled: 'No file is open, so there is nothing to save.',
  },
];

const OUTLINE: readonly OutlineEntry[] = [
  { name: 'renderRow', kind: 'function', line: 12, depth: 0 },
  { name: 'A heading', kind: 'heading', line: 3, depth: 2 },
];

function match(path: string, ranges: FileMatch['ranges'] = []): FileMatch {
  const separator = path.lastIndexOf('/');
  return { path, name: separator === -1 ? path : path.slice(separator + 1), ranges };
}

function state(raw: string, overrides: Partial<QuickOpenState> = {}): QuickOpenState {
  return {
    query: parseQuery(raw),
    hasWorkspace: true,
    files: {
      status: 'ready',
      matches: [match('src/editor-pane.tsx'), match('src/expand.rs')],
      truncated: false,
    },
    commands: COMMANDS,
    document: {
      name: 'editor-pane.tsx',
      lineCount: 200,
      outline: { status: 'ready', entries: OUTLINE, truncated: false },
    },
    ...overrides,
  };
}

function render(list: QuickOpenList, overrides: Partial<QuickOpenBarProps> = {}): string {
  return renderToStaticMarkup(
    createElement(QuickOpenBar, {
      raw: '',
      list,
      activeId: null,
      onRawChange: refuse('change the query'),
      onKey: refuse('handle a key') as unknown as QuickOpenBarProps['onKey'],
      onFocusRow: refuse('focus a row'),
      onAccept: refuse('accept a row'),
      onDismiss: refuse('dismiss'),
      autoFocus: false,
      ...overrides,
    }),
  );
}

describe('the field', () => {
  it('is a combobox that owns the keyboard and points at its list', () => {
    const markup = render(quickOpenList(state('epn')));
    // §10: a combobox, not a menu. The list is addressed, not focused.
    expect(markup).toContain('role="combobox"');
    expect(markup).toContain('aria-expanded="true"');
    expect(markup).toContain('role="listbox"');
    expect(markup).toMatch(/aria-controls="[^"]+"/);
  });

  it('names the active row, and only when there is one', () => {
    expect(render(quickOpenList(state('epn')))).not.toContain('aria-activedescendant');
    const focused = render(quickOpenList(state('epn')), { activeId: 'file:1' });
    expect(focused).toMatch(/aria-activedescendant="[^"]*file:1"/);
    expect(focused).toContain('aria-selected="true"');
  });

  it('says which mode it is in, so a blank list is never mysterious', () => {
    // §2: a bar that goes quiet without saying what it was doing is the
    // failure the mode-on-every-variant rule exists for.
    expect(render(quickOpenList(state('epn')))).toContain('Search files by name');
    expect(render(quickOpenList(state('>')))).toContain('Run a command');
    expect(render(quickOpenList(state(':12')))).toContain('Go to line');
    expect(render(quickOpenList(state('@ren')))).toContain('symbol');
  });
});

describe('every arm the model can return is drawn', () => {
  it('draws results in the order the model gave them', () => {
    const markup = render(quickOpenList(state('epn')));
    expect(markup.indexOf('editor-pane.tsx')).toBeGreaterThan(-1);
    expect(markup.indexOf('editor-pane.tsx')).toBeLessThan(markup.indexOf('expand.rs'));
  });

  it('draws the debounce window as a wait', () => {
    expect(render(quickOpenList(state('epn', { files: { status: 'loading' } })))).toContain(
      'Searching…',
    );
  });

  it('shows the truncated notice, and not a silently short list', () => {
    // §4: a truncated result must never read as a complete one.
    const many = Array.from({ length: QUICK_OPEN_MAX_ROWS + 5 }, (_, index) =>
      match(`src/f${index}.ts`),
    );
    const markup = render(
      quickOpenList(state('f', { files: { status: 'ready', matches: many, truncated: true } })),
    );
    expect(markup).toContain('quick-open-truncated');
    expect(markup).toContain('Narrow the search');
  });

  it('names the empty reason on the row, so two empties are not one', () => {
    const markup = render(quickOpenList(state('epn', { hasWorkspace: false })));
    expect(markup).toContain('data-reason="noWorkspace"');
    expect(markup).toContain('Open a folder');
  });

  it('shows a failure with its code, and keeps the field', () => {
    const error: CoreErrorShape = { code: 'Io', message: 'the root went away', retryable: false };
    const markup = render(quickOpenList(state('epn', { files: { status: 'failed', error } })));
    expect(markup).toContain('role="alert"');
    expect(markup).toContain('Io');
    expect(markup).toContain('the root went away');
    // A failed query never closes the bar.
    expect(markup).toContain('role="combobox"');
  });

  it('carries a disabled command through with its reason', () => {
    const markup = render(quickOpenList(state('>save')));
    expect(markup).toContain('aria-disabled="true"');
    expect(markup).toContain('nothing to save');
  });
});

describe('the highlight comes from the contract, not from a second matcher', () => {
  it('marks the spans the scorer reported', () => {
    const markup = render(
      quickOpenList(
        state('epn', {
          files: {
            status: 'ready',
            // e-p-n in `editor-pane.tsx`, offset by `src/`.
            matches: [
              match('src/editor-pane.tsx', [
                [4, 1],
                [11, 1],
                [13, 1],
              ]),
            ],
            truncated: false,
          },
        }),
      ),
    );
    expect(markup).toContain('<mark class="quick-open-match">e</mark>');
    // And the unmatched runs are still there: the path is not summarised.
    expect(markup).toContain('ditor-');
  });

  it('indexes by code point, so a path past the BMP highlights the right run', () => {
    const markup = render(
      quickOpenList(
        state('t', {
          files: {
            status: 'ready',
            matches: [match('🎨/theme.ts', [[2, 1]])],
            truncated: false,
          },
        }),
      ),
    );
    expect(markup).toContain('<mark class="quick-open-match">t</mark>');
    // The counter-example: a UTF-16 slice would have taken `/` here, and
    // splitting inside the surrogate pair would have produced a replacement
    // character somewhere in the markup.
    expect(markup).not.toContain('�');
  });
});

describe('everything from the workspace is text and nothing else (hard rule 13)', () => {
  const hostile = 'src/<script>alert(1)</script>/"onload="alert(1)".ts';
  const markup = render(
    quickOpenList(
      state('x', {
        files: { status: 'ready', matches: [match(hostile)], truncated: false },
        document: {
          name: 'x',
          lineCount: 10,
          outline: {
            status: 'ready',
            entries: [{ name: '<img src=x onerror=alert(1)>', kind: 'heading', line: 1, depth: 1 }],
            truncated: false,
          },
        },
      }),
    ),
  );

  it('puts nothing from a path into a tag', () => {
    for (const tag of tagsOf(markup)) {
      expect(tag).not.toMatch(/<script/i);
      expect(tag).not.toMatch(/\shref/i);
      expect(tag).not.toMatch(/\ssrc=/i);
      expect(tag).not.toMatch(/\son[a-z]+=/i);
    }
  });

  it('still shows the path, escaped', () => {
    expect(markup).toContain('&lt;script&gt;');
  });

  it('escapes a symbol name the same way', () => {
    const symbols = render(
      quickOpenList(
        state('@img', {
          document: {
            name: 'x',
            lineCount: 10,
            outline: {
              status: 'ready',
              entries: [
                { name: '<img src=x onerror=alert(1)>', kind: 'heading', line: 1, depth: 1 },
              ],
              truncated: false,
            },
          },
        }),
      ),
    );
    for (const tag of tagsOf(symbols)) {
      expect(tag).not.toMatch(/\ssrc=/i);
      expect(tag).not.toMatch(/\son[a-z]+=/i);
    }
    expect(symbols).toContain('&lt;img');
  });
});
