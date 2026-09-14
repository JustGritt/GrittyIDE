import type { AgentSession } from '@gritty/protocol';
import { ErrorEnvelope } from '../../components/error-envelope';
import { activitySourceLabel, agentStatus, providerLabel } from './agent-status';
import type { ConsoleKey } from './agent-tabs-reducer';
import { AgentTimelinePanel } from './agent-timeline';
import { taskLine } from './agents-reducer';
import type { RosterRow } from './roster-model';
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
 * prompt, which the feed carries and ADR-020 licenses. In particular there is
 * **no prompt box** here and no disabled one either (T310-U §9): there is
 * nothing to enable, and a greyed-out control would imply a permission the user
 * could acquire. Speaking to an agent is `RunConsole`'s, in the other lane.
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
  const status = agentStatus(session);
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
 * A view model rather than a session or a run, so the tab strip never learns
 * what either is: it is handed a key, a label, a tone and a preview flag,
 * exactly the shape it already derives for a document.
 */
export interface AgentTabItem {
  /** The console this tab activates — `run:<id>` or `session:<id>`. */
  readonly id: ConsoleKey;
  /**
   * What the sidebar row calls the same thing: a session's folder, a run's
   * short id. Taken from the row rather than re-derived, so the list a user
   * clicks in and the tab it opens cannot end up calling one agent two names.
   */
  readonly label: string;
  readonly tone: string;
  readonly hint: string;
  readonly preview: boolean;
}

/**
 * The open consoles, in tab order, resolved against the one shared row list.
 *
 * `RosterRow` rather than `AgentSession`, because a tab is now either lane's
 * (T310-U) and that projection is the one place the two are already drawn side
 * by side. Nothing about the domains is merged by this: a row is what a row
 * *says*, its `key` is the console key, and the tab strip gets strings.
 *
 * A tab whose row has gone is dropped here as well as in the reducer: the row
 * is what a label and a status come from, and a tab that could only be drawn as
 * a bare id is worse than one that is gone.
 */
export function agentTabItems(
  openTabs: readonly ConsoleKey[],
  previewId: ConsoleKey | null,
  rows: readonly RosterRow[],
): AgentTabItem[] {
  const byKey = new Map(rows.map((row) => [row.key, row]));
  const items: AgentTabItem[] = [];
  for (const key of openTabs) {
    const row = byKey.get(key);
    if (!row) continue;
    items.push({
      id: key,
      label: row.id,
      tone: row.status.tone,
      hint: `${row.status.label} — ${row.status.hint}`,
      preview: key === previewId,
    });
  }
  return items;
}
