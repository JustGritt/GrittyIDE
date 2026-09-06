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
  activitySource: 'TRANSCRIPT',
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

function render(
  entries: readonly AgentTimelineEntry[],
  overrides: Partial<AgentTimeline> = {},
  onSession: Partial<AgentSession> = {},
) {
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
      session: { ...session, ...onSession },
      timeline,
      openingPath: null,
      canOpenFiles: true,
      onOpenFile: () => {
        throw new Error('nothing should open while rendering');
      },
      answering: null,
      onAnswerPermission: () => {
        throw new Error('nothing should be answered while rendering');
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

  it('draws the permission prompt the core holds beside the ring', () => {
    // The one row the user is meant to act on, and the only one that arrives
    // outside `entries`. It is the last thing in the feed, and its tool name
    // and detail are transcript strings like every other field here.
    const waiting = render([entry({ type: 'ASSISTANT_TEXT', text: 'about to run it' })], {
      pending: {
        seq: Number.MAX_SAFE_INTEGER,
        kind: { type: 'PERMISSION_PENDING', tool: HOSTILE, detail: `rm -rf ${TRAVERSAL}` },
        sidechain: false,
      },
    });
    expect(waiting).toContain('Waiting on you');
    expect(waiting).toContain('agent-feed__row--permission');
    expect(waiting).toContain(`rm -rf ${TRAVERSAL}`);
    expect(waiting).not.toContain('<script');
    // Last: the prompt closes the feed rather than sitting somewhere in it.
    expect(waiting.indexOf('agent-feed__row--permission')).toBeGreaterThan(
      waiting.indexOf('about to run it'),
    );
  });

  it('does not call a session stopped at a prompt an empty one', () => {
    // "Nothing in this session's transcript yet" is the wrong thing to say
    // about a session that is waiting on the user.
    const waiting = render([], {
      pending: {
        seq: Number.MAX_SAFE_INTEGER,
        kind: { type: 'PERMISSION_PENDING', tool: 'Bash', detail: null },
        sidechain: false,
      },
    });
    expect(waiting).toContain('Waiting on you');
    expect(waiting).not.toContain('Nothing in this session');
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

// ---------------------------------------------------------------------------
// The permission card (T335, artboard 12a)
//
// Two things, and the second is the one that matters. The first is the sweep
// above, extended: an ask carries the agent's own tool input, which is the most
// attacker-controlled string in the product. The second is that the card is
// drawn *at all* — the review that sent T210, T221 and T232 back found the same
// defect three times, a correct view model with no caller, and a view-model
// test cannot catch it.
// ---------------------------------------------------------------------------

describe('a permission request is drawn, and drawn as text', () => {
  const pending = {
    askId: 'ask-1',
    sessionId: session.id,
    verb: 'WRITE_FILE',
    target: `${TRAVERSAL}/${HOSTILE}`,
    detail: HOSTILE,
    diff: [`+${HOSTILE}`, `-${TRAVERSAL}`, ' context'] as string[],
    added: 64,
    removed: 11,
    outsideRoot: true,
    sensitive: false,
    askedAt: 0,
    suggestedScope: HOSTILE,
  };

  const markup = render([entry({ type: 'ASSISTANT_TEXT', text: 'working' })], {}, {
    activity: 'BLOCKED',
    pending,
  } as Partial<AgentSession>);

  it('is on the screen, with all three answers', () => {
    expect(markup).toContain('needs permission');
    expect(markup).toContain('Allow once');
    expect(markup).toContain('for this run');
    expect(markup).toContain('Deny');
    // The keys the artboard prints beside them.
    expect(markup).toContain('⌘⏎');
    expect(markup).toContain('esc');
  });

  it('names the verb, the blast radius and the size of the change', () => {
    expect(markup).toContain('write file');
    expect(markup).toContain('outside its worktree');
    expect(markup).toContain('+64 −11 · 1 file');
  });

  it('puts every attacker-controlled field in a text node', () => {
    const tags = tagsOf(markup);
    for (const tag of tags) {
      expect(tag).not.toMatch(/^<\/?(script|img|iframe|object|embed|a)\b/i);
      expect(tag).not.toMatch(/\son\w+=/i);
      expect(tag).not.toMatch(/\ssrc=/i);
      expect(tag).not.toMatch(/\shref=/i);
    }
    expect(markup).not.toContain('<script>');
    // Present as escaped text, which is the point: the user has to be able to
    // read the path they are being asked about, however hostile it looks.
    expect(markup).toContain('&lt;script&gt;');
  });

  it('replaces the feed row rather than saying the same thing twice', () => {
    const withRow = render([entry({ type: 'PERMISSION_PENDING', tool: 'Bash', detail: null })], {
      pending: entry({ type: 'PERMISSION_PENDING', tool: 'Bash', detail: null }),
    });
    expect(withRow).toContain('agent-feed__row--permission');

    const withCard = render(
      [entry({ type: 'ASSISTANT_TEXT', text: 'working' })],
      { pending: entry({ type: 'PERMISSION_PENDING', tool: 'Bash', detail: null }) },
      { activity: 'BLOCKED', pending } as Partial<AgentSession>,
    );
    expect(withCard).toContain('needs permission');
    expect(withCard).not.toContain('agent-feed__row--permission');
  });

  it('offers no middle answer when the core could not name a scope', () => {
    // An unscoped standing grant is a blanket yes; the button is absent rather
    // than present and meaning something narrower than it says.
    const unscoped = render([], {}, {
      activity: 'BLOCKED',
      pending: { ...pending, suggestedScope: undefined, verb: 'OTHER' },
    } as Partial<AgentSession>);
    expect(unscoped).toContain('Allow once');
    expect(unscoped).not.toContain('for this run');
  });

  it('says a redacted file was deliberately not looked at', () => {
    const secret = render([], {}, {
      activity: 'BLOCKED',
      pending: { ...pending, diff: [], sensitive: true, target: '~/p/.env' },
    } as Partial<AgentSession>);
    expect(secret).toContain('~/p/.env');
    expect(secret).toContain('contents are not shown');
  });
});
