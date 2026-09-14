import type { Document, DocumentId } from '@gritty/protocol';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import type { AgentTabItem } from '../agents/agent-console';
import type { DocumentsState, DocumentTab } from '../documents/documents-reducer';
import { TabStrip } from './tab-strip';

/**
 * The strip's three groups and the rules between them (T120-M §3).
 *
 * Static markup, matching `agent-console-render.test.ts` and
 * `run-console-render.test.ts`: `apps/desktop`'s vitest project is a node one,
 * and a string is a stronger assertion than a query against a tree. Order,
 * grouping and which tab is marked active are exactly the things a string says
 * well.
 *
 * The settings tab is the second kind of non-document tab (ADR-026's stated
 * "Revisit when"), so the load-bearing assertions here are the ones the first
 * kind already had to satisfy: never a fake `DocumentTab`, never interleaved,
 * and never active at the same time as a document.
 */

const refuse = (what: string) => () => {
  throw new Error(`nothing should ${what} while rendering`);
};

function tab(id: string, path: string): DocumentTab {
  const document: Document = {
    id: id as DocumentId,
    workspaceId: 'ws' as Document['workspaceId'],
    path: path as Document['path'],
    encoding: 'utf-8',
    languageId: 'typescript',
    savedRevision: 1,
    savedHash: 'h' as Document['savedHash'],
    currentVersion: 1,
    dirty: false,
    readOnly: false,
    binary: false,
    largeFileMode: false,
    externalModificationState: 'UNCHANGED',
    size: 10,
  };
  // `as DocumentTab` below and not a literal: the tab carries fields this test
  // has no opinion about, and spelling them out would make it fail the day one
  // moves. The `Document` above *is* a literal, because its shape is the wire's.
  return {
    document,
    text: '',
    draft: null,
    pending: false,
    bufferGeneration: 1,
    syncing: false,
    saving: false,
    error: null,
    acknowledged: null,
  } as DocumentTab;
}

function documents(overrides: Partial<DocumentsState> = {}): DocumentsState {
  return {
    tabs: { d1: tab('d1', 'src/a.ts'), d2: tab('d2', 'src/b.ts') },
    order: ['d1', 'd2'] as DocumentId[],
    activeId: 'd1' as DocumentId,
    previewId: null,
    orphans: [],
    nextOrphanId: 1,
    ...overrides,
  };
}

const console1: AgentTabItem = {
  id: 'session:s1',
  label: 'gritty',
  tone: 'busy',
  hint: 'Working',
  preview: false,
};

type Props = Parameters<typeof TabStrip>[0];

function markup(overrides: Partial<Props> = {}): string {
  return renderToStaticMarkup(
    createElement(TabStrip, {
      state: documents(),
      onActivate: refuse('activate a document'),
      onPromote: refuse('keep a document'),
      onClose: refuse('close a document'),
      onCloseMany: refuse('close several documents'),
      onReorder: refuse('reorder'),
      ...overrides,
    } as Props),
  );
}

/**
 * The tab labels in the order the strip draws them.
 *
 * Tags are stripped rather than skipped: an agent tab wraps a status mark in
 * nested spans before its label, and a pattern that assumed one would read that
 * tab as empty and quietly pass the ordering assertion.
 */
function labels(html: string): string[] {
  return [...html.matchAll(/class="tabs__tab[^"]*"[^>]*>([\s\S]*?)<\/button>/g)].map(([, inner]) =>
    (inner ?? '').replace(/<[^>]*>/g, '').trim(),
  );
}

describe('the settings group', () => {
  it('is absent when settings are not mounted at all', () => {
    expect(markup()).not.toContain('Settings');
  });

  it('is absent while the tab is closed', () => {
    const html = markup({
      settings: {
        open: false,
        preview: false,
        active: false,
        onActivate: refuse('activate settings'),
        onPromote: refuse('keep settings'),
        onClose: refuse('close settings'),
      },
    });
    expect(html).not.toContain('Settings');
  });

  it('comes after the documents and after the consoles, never among them', () => {
    const html = markup({
      agents: {
        items: [console1],
        activeId: null,
        onActivate: refuse('activate a console'),
        onPromote: refuse('keep a console'),
        onClose: refuse('close a console'),
      },
      settings: {
        open: true,
        preview: false,
        active: false,
        onActivate: refuse('activate settings'),
        onPromote: refuse('keep settings'),
        onClose: refuse('close settings'),
      },
    });
    expect(labels(html)).toEqual(['a.ts', 'b.ts', 'gritty', 'Settings']);
  });

  it('offers a way to close itself', () => {
    const html = markup({
      settings: {
        open: true,
        preview: false,
        active: true,
        onActivate: refuse('activate settings'),
        onPromote: refuse('keep settings'),
        onClose: refuse('close settings'),
      },
    });
    expect(html).toContain('aria-label="Close Settings"');
  });

  it('is set in italics while it is a preview', () => {
    const html = markup({
      settings: {
        open: true,
        preview: true,
        active: true,
        onActivate: refuse('activate settings'),
        onPromote: refuse('keep settings'),
        onClose: refuse('close settings'),
      },
    });
    expect(html).toContain('tabs__tab--preview');
    expect(html).toContain('double-click to keep');
  });
});

describe('what is active', () => {
  it('marks the document while nothing else has the pane', () => {
    const html = markup();
    expect(html).toContain('aria-selected="true"');
    // The first tab, and only it.
    expect([...html.matchAll(/aria-selected="true"/g)]).toHaveLength(1);
  });

  it('deactivates every document while the settings tab shows', () => {
    // The rule the console group already had: `documents.state.activeId` still
    // points at the file — that is what session restore persists — so the check
    // lives in the view, and a second kind of non-document tab has to honour it
    // too or two tabs claim to be selected at once.
    const html = markup({
      settings: {
        open: true,
        preview: false,
        active: true,
        onActivate: refuse('activate settings'),
        onPromote: refuse('keep settings'),
        onClose: refuse('close settings'),
      },
    });
    const selected = [...html.matchAll(/aria-selected="true"/g)];
    expect(selected).toHaveLength(1);
    // And it is the settings tab, not the file.
    expect(html).toMatch(/aria-selected="true"[^>]*>Settings/);
  });

  it('leaves a console selected rather than the settings tab it took the pane from', () => {
    const html = markup({
      agents: {
        items: [console1],
        activeId: 'session:s1',
        onActivate: refuse('activate a console'),
        onPromote: refuse('keep a console'),
        onClose: refuse('close a console'),
      },
      settings: {
        open: true,
        preview: false,
        active: false,
        onActivate: refuse('activate settings'),
        onPromote: refuse('keep settings'),
        onClose: refuse('close settings'),
      },
    });
    expect([...html.matchAll(/aria-selected="true"/g)]).toHaveLength(1);
    expect(html).toContain('Settings');
  });
});

describe('the strip itself', () => {
  it('is absent with nothing open at all', () => {
    expect(markup({ state: documents({ tabs: {}, order: [], activeId: null }) })).toBe('');
  });

  it('exists for a settings tab alone, so there is something to close it from', () => {
    // ⌘, with no file and no console open must still produce a strip. The
    // console group needed the same thing for the same reason: a non-document
    // tab outlives the last document.
    const html = markup({
      state: documents({ tabs: {}, order: [], activeId: null }),
      settings: {
        open: true,
        preview: false,
        active: true,
        onActivate: refuse('activate settings'),
        onPromote: refuse('keep settings'),
        onClose: refuse('close settings'),
      },
    });
    expect(html).toContain('Settings');
  });

  it('never gives the settings tab a document identity', () => {
    // ADR-026's load-bearing rule: a non-document tab is a second source, never
    // a fake `DocumentTab`. If it ever reached `documents.state.order`,
    // `buildSessionTabs` would walk it into a `SessionTab` and the strip would
    // need the migration T231 priced this feature on.
    const state = documents({ tabs: {}, order: [], activeId: null });
    markup({
      state,
      settings: {
        open: true,
        preview: false,
        active: true,
        onActivate: refuse('activate settings'),
        onPromote: refuse('keep settings'),
        onClose: refuse('close settings'),
      },
    });
    expect(Object.keys(state.tabs)).toEqual([]);
    expect(state.order).toEqual([]);
  });
});
