import type {
  AgentPermissionVerdict,
  AgentSession,
  AgentTimeline,
  WorkspacePath,
} from '@gritty/protocol';
import { useCallback, useEffect, useMemo, useRef } from 'react';
import { AgentStatusBadge } from './agent-status-badge';
import {
  isPinnedToBottom,
  pendingPinKey,
  type TimelineRow,
  type TouchedFile,
  timelineRows,
  touchedFiles,
} from './agent-timeline-model';
import { taskLine } from './agents-reducer';
import { PermissionPrompt } from './permission-prompt';

/**
 * The selected session's timeline (T232): what it has been doing, and the files
 * it changed while doing it.
 *
 * The second half is the point of the whole rail. A multiplexer can show you an
 * agent's output; only an IDE can put you in the file it just edited — so the
 * files-touched list sits above the feed, where a long conversation cannot push
 * it off the screen.
 *
 * Everything drawn here came out of an agent's transcript, which is untrusted
 * input (hard rule 13). The core bounded every field before it crossed the
 * protocol; this component's whole contribution to that is to put each string
 * in a text node and never anywhere else. There is no `dangerouslySetInnerHTML`
 * in this file, no Markdown, no link the view resolves, and no path anything
 * opens by concatenation: the one openable value is the `WorkspacePath` the
 * core itself decided was inside the workspace.
 */
export function AgentTimelinePanel({
  session,
  timeline,
  openingPath,
  canOpenFiles,
  onOpenFile,
  answering,
  onAnswerPermission,
}: {
  session: AgentSession;
  /** `null` only in the instant between selecting a row and the core answering. */
  timeline: AgentTimeline | null;
  /** The file being opened right now, if any. */
  openingPath: string | null;
  /** False with no workspace open: there is nothing for a path to be inside. */
  canOpenFiles: boolean;
  onOpenFile(path: WorkspacePath): void;
  /** The ask being answered right now, if any (T335). */
  answering: string | null;
  onAnswerPermission(askId: string, verdict: AgentPermissionVerdict): void;
}) {
  const pending = session.pending ?? null;
  const rows = useMemo(() => {
    if (!timeline) return [];
    const all = timelineRows(timeline);
    // The card below says everything the feed's own permission row said, and
    // says it in a form that can be answered. Two of them would be the same
    // prompt twice (T335).
    return pending ? all.filter((row) => row.tone !== 'permission') : all;
  }, [timeline, pending]);
  const files = useMemo(() => (timeline ? touchedFiles(timeline.entries) : []), [timeline]);
  const pinKey = pendingPinKey(timeline);
  const title = taskLine(session);
  const ended = session.activity === 'ENDED';

  return (
    <section className="agent-console__timeline" aria-label="Timeline">
      <div className="agent-console__timeline-head">
        <span className="agent-console__timeline-title">{title ?? 'Selected session'}</span>
        <span className="agent-console__spacer" />
        <AgentStatusBadge activity={session.activity} />
      </div>

      {files.length > 0 && (
        <TouchedFileList
          files={files}
          openingPath={openingPath}
          canOpen={canOpenFiles}
          onOpen={onOpenFile}
        />
      )}

      {timeline === null ? (
        <p className="agent-console__placeholder">Opening…</p>
      ) : rows.length === 0 ? (
        // Three states that are not the same thing (§6): nothing to read,
        // nothing said yet, and a session that finished without saying
        // anything. Collapsing them would make an unreadable transcript look
        // like a quiet agent.
        !timeline.transcriptEvidence ? (
          <p className="agent-console__note">
            No transcript could be read for this session, so there is nothing to follow. Gritty
            knows it is running, not what it is doing.
          </p>
        ) : ended ? (
          <p className="agent-console__placeholder">
            This session ended with nothing in its transcript to show.
          </p>
        ) : (
          <p className="agent-console__placeholder">
            Nothing in this session's transcript yet. New entries appear here as the agent works.
          </p>
        )
      ) : (
        <Feed key={session.id} rows={rows} pinKey={pinKey} />
      )}

      {/* The ask lands where the work is — framed, at the foot of the feed,
          never a modal over the whole IDE (artboard 12a). */}
      {pending && (
        <PermissionPrompt
          request={pending}
          sessionLabel={title ?? session.id}
          now={Date.now()}
          busy={answering === pending.askId}
          onAnswer={(verdict) => onAnswerPermission(pending.askId, verdict)}
        />
      )}

      {/* The feed stopped growing; nothing was removed (§6, and the failure
          table's "the session ends mid-render"). */}
      {ended && rows.length > 0 && (
        <p className="agent-console__placeholder">
          This session has ended. The feed above is what it left behind.
        </p>
      )}
    </section>
  );
}

/**
 * The feed itself, oldest entry last, in its own scroll container.
 *
 * It scrolls rather than the rail, for two reasons. A long tool result must not
 * push the files-touched list out of reach, and the pin below needs one element
 * whose bottom means "the newest entry" — which the rail body, with the session
 * list above it, does not have.
 */
function Feed({
  rows,
  pinKey,
}: {
  rows: readonly TimelineRow[];
  /**
   * The permission prompt the session is stopped at, or `null` for none. A
   * change to it re-pins the feed once (§2).
   */
  pinKey: string | null;
}) {
  const feed = useRef<HTMLDivElement | null>(null);
  /**
   * Whether new entries may scroll the view (§1).
   *
   * A ref, not state: it is read by an effect and written by a scroll handler,
   * and re-rendering the feed on every scroll frame to store a boolean nothing
   * draws would be the opposite of what this is for.
   *
   * It starts pinned and is reset by the `key` the panel gives this component:
   * a different session is a different conversation, so it is remounted rather
   * than told to forget where the last one had been scrolled.
   */
  const pinned = useRef(true);

  // A permission prompt is the one row the user is meant to act on, so it may
  // not arrive off-screen for someone who had scrolled up to read something.
  // Re-pinning here rather than scrolling directly leaves the work to the
  // effect below — which is declared after this one, so it runs in the same
  // commit and sees the pin already set.
  //
  // Once, on the change. Afterwards the ordinary rule applies again and the
  // user can scroll away from a standing prompt without being dragged back.
  useEffect(() => {
    if (pinKey !== null) pinned.current = true;
  }, [pinKey]);

  // Follow new entries down, but only from the bottom. Someone who scrolled up
  // to read something is left exactly where they are — including through a
  // `restarted` reset, which redraws the list without touching this.
  //
  // Deliberately on every render rather than keyed to the rows: the pin means
  // "stay at the bottom", and the row array is not the only thing that can
  // change what the bottom is.
  useEffect(() => {
    const element = feed.current;
    if (element && pinned.current) element.scrollTop = element.scrollHeight;
  });

  const onScroll = useCallback(() => {
    const element = feed.current;
    if (element) pinned.current = isPinnedToBottom(element);
  }, []);

  return (
    <div className="agent-feed" ref={feed} onScroll={onScroll}>
      <ol className="agent-feed__list">
        {rows.map((row) => (
          <FeedRow key={row.key} row={row} />
        ))}
      </ol>
    </div>
  );
}

/** One line of the feed. Every string it draws is a text node. */
function FeedRow({ row }: { row: TimelineRow }) {
  const at = row.at === null ? null : clockTime(row.at);
  const tool = row.tone === 'tool';
  /**
   * How the call went, as a class on the row itself and not only as a badge.
   *
   * Artboard 12a draws a command block with its outcome down its left edge, and
   * that is the channel that survives a feed being skimmed: a failed call is
   * findable without reading a badge on every row. `null` while it is still
   * outstanding is the honest state, and reads as *running* rather than as ok.
   */
  const outcome = !tool ? null : row.outcome === null ? 'running' : row.outcome.ok ? 'ok' : 'fail';
  return (
    <li
      className={`agent-feed__row agent-feed__row--${row.tone}${
        outcome === null ? '' : ` agent-feed__row--${outcome}`
      }${row.sidechain ? ' agent-feed__row--sidechain' : ''}`}
    >
      <span className="agent-feed__head">
        <span className="agent-feed__label">{row.label}</span>
        {/* For a tool the name qualifies the label rather than filling the row:
            "TOOL · Bash" reads as one thing, and it leaves the body for the
            command — which is what the reader came for. Drawn before the badge
            so the head reads left to right as what · which · how it went. */}
        {tool && <span className="agent-feed__tool">{row.text}</span>}
        {/* Marked and subordinate, never hidden: T220 dropped subagent work and
            listed that as a known limitation (§4). */}
        {row.sidechain && (
          <span className="agent-feed__badge" title="Work done by a subagent of this session">
            subagent
          </span>
        )}
        {tool && (
          <span className={`agent-feed__badge agent-feed__badge--${outcome}`}>
            {outcome === 'fail' ? 'failed' : outcome}
          </span>
        )}
        <span className="agent-console__spacer" />
        {at !== null && <span className="agent-feed__at">{at}</span>}
      </span>
      {/* A tool's name is already in the head; drawing it again here is the
          redundancy that made every call look alike. */}
      {!tool && <p className="agent-feed__text">{row.text}</p>}
      {row.detail !== null && (
        // A tool's `detail` is its *input* — the command. It is the row's
        // subject, so it is drawn as the row's subject and not as an aside.
        <p className={`agent-feed__detail${tool ? ' agent-feed__detail--input' : ''}`}>
          {row.detail}
        </p>
      )}
      {/* The result's own preview, under the call it belongs to. Only for a
          paired tool row: an unpaired result already drew its detail above. */}
      {tool && row.outcome?.detail != null && (
        <p className="agent-feed__detail agent-feed__detail--result">{row.outcome.detail}</p>
      )}
    </li>
  );
}

/**
 * The files this session wrote, most recent first (§9).
 *
 * A file inside the open workspace is a button. A file outside it is text, and
 * not because the renderer chose to be careful: the core sent no path for it,
 * so there is nothing here that could open it (§10, ADR-010).
 */
function TouchedFileList({
  files,
  openingPath,
  canOpen,
  onOpen,
}: {
  files: readonly TouchedFile[];
  openingPath: string | null;
  canOpen: boolean;
  onOpen(path: WorkspacePath): void;
}) {
  return (
    <section className="agent-files" aria-label="Files touched">
      <h3 className="agent-files__head">Files touched</h3>
      <ul className="agent-files__list">
        {files.map((file) => {
          // Bound to a const so the narrowing survives into the handler.
          const openPath = file.openPath;
          const at = file.at === null ? null : clockTime(file.at);
          if (openPath === null || !canOpen) {
            return (
              <li key={file.key}>
                <span className="agent-files__row agent-files__row--outside">
                  <span className="agent-files__path">{file.display}</span>
                  <span className="agent-files__outside">elsewhere</span>
                  {at !== null && <span className="agent-files__at">{at}</span>}
                </span>
              </li>
            );
          }
          return (
            <li key={file.key}>
              <button
                type="button"
                className="agent-files__row agent-files__row--open"
                // One open at a time: a second click would race a second
                // `documents.status` for nothing.
                disabled={openingPath !== null}
                onClick={() => onOpen(openPath)}
                title="Open in the editor"
              >
                <span className="agent-files__path">{file.display}</span>
                {openingPath === openPath && <span className="agent-files__at">opening…</span>}
                {openingPath !== openPath && at !== null && (
                  <span className="agent-files__at">{at}</span>
                )}
              </button>
            </li>
          );
        })}
      </ul>
    </section>
  );
}

/**
 * The wall-clock time of a record, or `null` when the number cannot be one.
 *
 * `at` came from a transcript the agent wrote, so a value `Date` cannot make
 * sense of is possible. A row with no time is the honest outcome; throwing
 * inside a render over a bad timestamp would take the whole feed with it.
 */
function clockTime(millis: number): string | null {
  const date = new Date(millis);
  return Number.isNaN(date.getTime()) ? null : date.toLocaleTimeString();
}
