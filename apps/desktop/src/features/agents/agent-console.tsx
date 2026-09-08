import type { AgentSession } from '@gritty/protocol';
import { ErrorEnvelope } from '../../components/error-envelope';
import { activitySourceLabel, agentStatus, providerLabel } from './agent-status';
import { AgentTimelinePanel } from './agent-timeline';
import { taskLine } from './agents-reducer';
import type { AgentConsoleController } from './use-agent-console';
import type { AgentsController } from './use-agents';

/**
 * One agent session, in the editor area (T233-C, design artboards 11a/12a).
 *
 * What the rail's body was, given the whole pane instead of 320px. It is a tab
 * beside the open files rather than a third column, which is ADR-026 — and the
 * reason the transcript is legible at all: a feed of tool calls, diffs and
 * command output does not fit a column narrow enough to leave an editor beside
 * it, and the rail was taking that width from a split-view markdown file.
 *
 * The header is everything the rail's row said about a session, kept because
 * deleting the rail would otherwise delete the only place any of it appears.
 * `activitySourceLabel` in particular is a *required behaviour*, not a nicety:
 * T221 §24 asks that a row say where its verdict came from, because three
 * sources can disagree and `BLOCKED` — the one status the user is meant to act
 * on — is otherwise a verdict with no provenance.
 *
 * Read-only, still. Every session here is one another terminal owns; nothing
 * here starts or stops one. The single exception is answering a permission
 * prompt, which the feed carries and ADR-020 licenses.
 */
export function AgentConsole({
  session,
  controller,
  agents,
}: {
  /** The session this tab is about, resolved from the one shared roster. */
  session: AgentSession;
  controller: AgentConsoleController;
  agents: AgentsController;
}) {
  const status = agentStatus(session.activity);
  const task = taskLine(session);

  return (
    <section className="agent-console" aria-label={`Agent session ${session.id}`}>
      <header className="agent-console__bar">
        {/* Identity, then status: which agent, and how it is doing. The same
            two-channel encoding the sidebar rows and the terminal tabs use. */}
        <span className="agent-console__identity" aria-hidden="true" />
        <span className={`agent-status agent-status--${status.tone}`} title={status.hint}>
          <span className="agent-status__mark" aria-hidden="true" />
          {status.label}
        </span>
        {/* Where that verdict came from (T221 §24). The rail's row was the only
            place this appeared; it would have gone with the rail. */}
        <span className="agent-console__source">{activitySourceLabel(session.activitySource)}</span>
        <span className="agent-console__spacer" />
        <span className="agent-console__meta">{providerLabel(session.provider)}</span>
        {session.root ? (
          <span className="agent-console__root" title={session.root}>
            {session.root}
          </span>
        ) : (
          <em className="agent-console__unknown">directory unknown</em>
        )}
        {session.gitBranch && <span className="agent-console__meta">{session.gitBranch}</span>}
      </header>

      {/* The task is its own line rather than crammed into the bar: it is the
          agent's own sentence and can be long, and it wraps to one line here
          instead of pushing the status off the end. */}
      <p className="agent-console__task">
        {task ?? <em className="agent-console__unknown">No transcript yet</em>}
      </p>

      {controller.error && (
        <div className="agent-console__error">
          <ErrorEnvelope error={controller.error} onDismiss={controller.clearError} />
        </div>
      )}

      {/* T232's panel, unchanged. The feed's rules — deltas keyed on `seq`,
          losses drawn rather than repaired, a tool result attached to its call,
          every string a text node — are its own and are not re-decided here. */}
      <AgentTimelinePanel
        session={session}
        timeline={controller.timeline}
        openingPath={controller.openingPath}
        canOpenFiles={controller.canOpenFiles}
        onOpenFile={controller.openFile}
        answering={agents.answering}
        onAnswerPermission={(askId, verdict) => {
          void agents.answerPermission(askId, verdict);
        }}
      />
    </section>
  );
}

/**
 * One console's tab, as the strip needs it.
 *
 * A view model rather than an `AgentSession`, so the tab strip never learns
 * what a session is: it is handed a label, a tone and a preview flag, exactly
 * the shape it already derives for a document.
 */
export interface AgentTabItem {
  readonly sessionId: string;
  /**
   * The session's folder rather than its id: an id is a uuid nobody reads, and
   * the folder is what the roster groups by and what the user recognises. A
   * short id stands in when the working directory was never seen — a session
   * found by transcript alone has no `rootName`.
   */
  readonly label: string;
  readonly tone: string;
  readonly hint: string;
  readonly preview: boolean;
}

/**
 * The open consoles, in tab order, resolved against the one shared roster.
 *
 * A tab whose session has left the roster is dropped here as well as in the
 * reducer: the roster is what a label and a status come from, and a tab that
 * could only be drawn as a bare id is worse than one that is gone.
 */
export function agentTabItems(
  openTabs: readonly string[],
  previewId: string | null,
  sessions: readonly AgentSession[],
): AgentTabItem[] {
  const byId = new Map(sessions.map((session) => [session.id, session]));
  const items: AgentTabItem[] = [];
  for (const sessionId of openTabs) {
    const session = byId.get(sessionId);
    if (!session) continue;
    const status = agentStatus(session.activity);
    items.push({
      sessionId,
      label: session.rootName ?? sessionId.slice(0, 7),
      tone: status.tone,
      hint: `${status.label} — ${status.hint}`,
      preview: sessionId === previewId,
    });
  }
  return items;
}
