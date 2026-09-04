import type { SessionNoticeKind } from '@gritty/protocol';
import type { SessionController } from './use-session';

const KIND_LABELS: Record<SessionNoticeKind, string> = {
  DATABASE_MOVED_ASIDE: 'Session database replaced',
  PERSISTENCE_UNAVAILABLE: 'Session persistence unavailable',
  WORKSPACE_ROOT_MISSING: 'Last workspace not reopened',
  OPERATIONS_INTERRUPTED: 'Operations interrupted',
};

/**
 * What the session layer wants the user to know: a damaged or unusable
 * database, a workspace that could not come back, writes that keep failing,
 * tabs that could not be reopened. Dismissible; nothing here blocks editing.
 */
export function SessionNotices({ session }: { session: SessionController }) {
  const { state } = session;
  const { persistence, notices, skipped } = state;
  const degraded = persistence?.available === true && !persistence.healthy;
  if (notices.length === 0 && skipped.length === 0 && !degraded) return null;

  return (
    <div className="session-notices" aria-live="polite">
      {degraded && (
        <p className="session-notice session-notice--degraded" role="status">
          <strong>Session persistence is failing.</strong> Unsaved changes stay in memory and the
          core keeps retrying{persistence?.message ? `: ${persistence.message}` : '.'}
        </p>
      )}
      {notices.map((notice, index) => (
        <p key={`${notice.kind}:${notice.message}`} className="session-notice" role="status">
          <strong>{KIND_LABELS[notice.kind]}.</strong> {notice.message}
          {notice.path && <code className="session-notice__path">{notice.path}</code>}
          <button type="button" onClick={() => session.dismissNotice(index)}>
            Dismiss
          </button>
        </p>
      ))}
      {skipped.length > 0 && (
        <p className="session-notice" role="status">
          <strong>Some tabs could not be reopened.</strong>{' '}
          {skipped.map((path) => (
            <code key={path} className="session-notice__path">
              {path}
            </code>
          ))}
          <button type="button" onClick={session.dismissSkipped}>
            Dismiss
          </button>
        </p>
      )}
    </div>
  );
}
