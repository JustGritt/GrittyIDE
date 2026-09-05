import type { AgentSession, AgentTimeline, AgentTimelineEntry } from '@gritty/protocol';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import { AgentTimelinePanel } from './agent-timeline';

/**
 * What the rail actually puts on the screen, rendered (T232 §7, hard rule 13).
 *
 * The model tests above prove the mapping keeps every field a string. This one
 * proves the component keeps it a *text node*: agent output and repository
 * content are untrusted input, and the failure this guards against is the one
 * that does not show up in a view-model assertion — a string that reaches the
 * DOM as markup instead of as text.
 *
 * Static markup rather than a DOM: `apps/desktop`'s vitest project is a node
 * one, and rendering to a string is a stronger assertion anyway. Escaped output
 * is escaped output whatever parses it next.
 */

/** Everything an attacker-controlled transcript might try, in one entry. */
const HOSTILE = '<script>alert(1)</script><img src=x onerror=alert(2)>';
/** A path that climbs out of anything it is joined to. */
const TRAVERSAL = '../../etc/passwd';

const session: AgentSession = {
  id: 'session-1',
  provider: 'CLAUDE_CODE',
  activity: 'WORKING',
  evidence: 'BOTH',
  inWorkspace: true,
  title: `${HOSTILE} ${TRAVERSAL}`,
};

/** Every real tag in the output. An escaped `&lt;img …&gt;` is not one. */
function tagsOf(markup: string): readonly string[] {
  return markup.match(/<[^>]*>/g) ?? [];
}

let seq = 0;
function entry(kind: AgentTimelineEntry['kind'], sidechain = false): AgentTimelineEntry {
  return { seq: seq++, kind, sidechain };
}

function render(entries: readonly AgentTimelineEntry[], overrides: Partial<AgentTimeline> = {}) {
  const timeline: AgentTimeline = {
    sessionId: session.id,
    entries: [...entries],
    dropped: 0,
    restarted: false,
    transcriptEvidence: true,
    ...overrides,
  };
  return renderToStaticMarkup(
    createElement(AgentTimelinePanel, {
      session,
      timeline,
      openingPath: null,
      canOpenFiles: true,
      onOpenFile: () => {
        throw new Error('nothing should open while rendering');
      },
    }),
  );
}

describe('the timeline renders agent output as text', () => {
  const markup = render([
    entry({ type: 'USER_PROMPT', text: HOSTILE }),
    entry({ type: 'ASSISTANT_TEXT', text: `writing to ${TRAVERSAL}` }),
    entry({ type: 'THINKING', text: HOSTILE }),
    entry({ type: 'TOOL_CALL', toolUseId: 'u1', name: HOSTILE, detail: `cat ${TRAVERSAL}` }),
    entry({ type: 'TOOL_RESULT', toolUseId: 'u1', ok: false, detail: HOSTILE }),
    entry({ type: 'NOTICE', level: 'ERROR', text: HOSTILE }),
    entry({ type: 'PERMISSION_PENDING', tool: HOSTILE, detail: TRAVERSAL }),
    entry({ type: 'ASSISTANT_TEXT', text: HOSTILE }, true),
    // The core sends no path for a file outside the workspace, which is what
    // makes this row text rather than a button.
    entry({ type: 'FILE_TOUCHED', path: null, display: TRAVERSAL }),
  ]);

  it('produces no markup from a transcript that contains some', () => {
    // `&lt;` is a character, not a tag: everything hostile survives into the
    // output as something the reader can see and the browser cannot run.
    expect(markup).toContain('&lt;script&gt;alert(1)&lt;/script&gt;');
    expect(markup).toContain('&lt;img src=x onerror=alert(2)&gt;');
    // And nothing the agent wrote became an element. Scanned as tags rather
    // than as substrings, because `onerror` appearing inside an escaped text
    // node is exactly the safe outcome this is asserting.
    for (const tag of tagsOf(markup)) {
      expect(tag).not.toMatch(/^<\/?(script|img|iframe|object|embed|style|a)[\s/>]/i);
    }
  });

  it('never renders anything that navigates or runs', () => {
    // No link the view resolved, no `src` it fetched, no handler it attached.
    for (const tag of tagsOf(markup)) {
      expect(tag).not.toMatch(/\shref/i);
      expect(tag).not.toMatch(/\ssrc=/i);
      expect(tag).not.toMatch(/\son[a-z]+=/i);
    }
  });

  it('offers no path to open for a file outside the workspace', () => {
    // The traversal string is shown — it is what the agent wrote, and hiding it
    // would be worse — but there is no control that opens it. The only buttons
    // this panel draws are openable files, and this timeline has none.
    expect(markup).toContain(TRAVERSAL);
    expect(markup).not.toContain('<button');
  });

  it('escapes the same content in a gap and in the session title', () => {
    const withGaps = render([entry({ type: 'ASSISTANT_TEXT', text: HOSTILE })], {
      dropped: 4,
      restarted: true,
    });
    expect(withGaps).not.toContain('<script');
    expect(withGaps).toContain('4 earlier entries dropped');
    expect(withGaps).toContain('reset');
  });

  it('draws an openable file as a button holding the path the core sent', () => {
    const inside = render([
      entry({ type: 'FILE_TOUCHED', path: 'src/main.rs', display: 'src/main.rs' }),
    ]);
    expect(inside).toContain('<button');
    expect(inside).toContain('src/main.rs');
    // The button carries no path in its markup: the handler closes over the
    // `WorkspacePath` the core sent, so nothing is ever built from a string in
    // the DOM.
    expect(inside).not.toContain('href');
  });

  it('shows an entry kind this build does not know as text', () => {
    const future = render([
      entry({ type: 'FROM_A_NEWER_CORE', text: HOSTILE } as unknown as AgentTimelineEntry['kind']),
    ]);
    expect(future).toContain('Unrecognised');
    expect(future).not.toContain('<script');
    expect(future).toContain('alert(1)');
  });
});
