import type { RecoveryState } from '@gritty/protocol';
import { type CandidateItem, hasPrompt, pendingChoices } from './session-reducer';
import type { SessionController } from './use-session';

const STATE_LABELS: Record<RecoveryState, string> = {
  SAME_AS_DISK: 'file unchanged since',
  DISK_CHANGED: 'file changed on disk since; opens as conflicted',
  FILE_MISSING: 'file is gone; opens as deleted',
};

function describe(item: CandidateItem): string {
  const when = new Date(item.updatedAt).toLocaleString();
  return `${STATE_LABELS[item.state]} · ${item.size.toLocaleString()} B · saved by Gritty ${when}`;
}

/**
 * Unsaved buffers left by a previous run (T060). Same-as-disk snapshots wait
 * for the user; changed and missing ones are recovered automatically and
 * only show up here while that happens or when it failed. Nothing here ever
 * writes to the disk: a recovered buffer is dirty until the user saves it.
 */
export function RecoveryPrompt({ session }: { session: SessionController }) {
  const { state } = session;
  if (!hasPrompt(state)) return null;
  const choices = pendingChoices(state);
  const items = state.candidates?.items ?? [];

  return (
    <div className="editor__banner recovery" role="alertdialog" aria-labelledby="recovery-title">
      <p id="recovery-title">
        <strong>Unsaved changes from a previous session.</strong> Restoring puts the text back into
        the editor as unsaved changes; nothing is written to disk until you save.
      </p>
      <ul className="recovery__list">
        {items.map((item) => (
          <li key={item.path} className="recovery__item">
            <code>{item.path}</code>
            <span className="recovery__state">{describe(item)}</span>
            {item.status === 'restoring' && <span className="recovery__status">restoring…</span>}
            {item.status === 'failed' && (
              <span className="recovery__error">could not restore: {item.error}</span>
            )}
            {item.status !== 'restoring' && (
              <span className="recovery__actions">
                <button type="button" onClick={() => session.recover(item.path)}>
                  {item.status === 'failed' ? 'Retry' : 'Restore'}
                </button>
                <button type="button" onClick={() => session.discard(item.path)}>
                  Discard
                </button>
              </span>
            )}
          </li>
        ))}
      </ul>
      {choices.length > 1 && (
        <div className="editor__banner-actions">
          {/* biome-ignore lint/a11y/noAutofocus: the default answer is "restore" (task T060, required behavior 4) */}
          <button type="button" autoFocus onClick={session.restoreAll}>
            Restore all ({choices.length})
          </button>
          <button type="button" onClick={session.discardAll}>
            Discard all
          </button>
        </div>
      )}
    </div>
  );
}
