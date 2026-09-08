import type { AgentPermissionRequest, AgentPermissionVerdict } from '@gritty/protocol';
import { useEffect } from 'react';
import { PERMISSION_CHORD_HINT, permissionVerdictFor } from './permission-chords';
import {
  askedAgo,
  blastRadius,
  changeSummary,
  diffSign,
  diffText,
  scopeLabel,
  verbLabel,
} from './permission-model';

/**
 * The ask, where the work is (T335, artboard 12a).
 *
 * A framed block at the foot of the feed, not a modal and not a row in the
 * transcript. That is the whole design decision of turn 12: *the agent you're
 * watching asks in place; the ones you aren't ask through a queue — never a
 * modal over the whole IDE.* A modal would stop the user doing the very thing
 * they need to do to judge the ask, which is look at their code.
 *
 * Everything drawn here is untrusted input twice over (hard rule 13): the
 * agent's tool input, bounded by the hook, then bounded again by the core. This
 * component's contribution is the same as the feed's — every string goes in a
 * text node and nowhere else. No `dangerouslySetInnerHTML`, no path anything
 * opens, no link the view resolves.
 */
export function PermissionPrompt({
  request,
  sessionLabel,
  now,
  busy,
  onAnswer,
  onReveal,
}: {
  request: AgentPermissionRequest;
  /** How the row above names this session — `t090`, or its id. */
  sessionLabel: string;
  /** Injected so the age is a value a test can fix, not `Date.now()`. */
  now: number;
  /** An answer is in flight. The buttons go quiet rather than double-firing. */
  busy: boolean;
  onAnswer(verdict: AgentPermissionVerdict): void;
  /**
   * Show the terminal this session is running in. The honest escape hatch: the
   * prompt is also live over there, and answering it there works too (ADR-020).
   */
  onReveal?: () => void;
}) {
  const scope = scopeLabel(request);
  const summary = changeSummary(request);
  const radius = blastRadius(request);
  // Absent and empty mean the same thing here — the core omits the field when
  // there is nothing to show — so they are collapsed once rather than guarded
  // at each of the three places that read it.
  const diff = request.diff ?? [];

  // The card owns these keys while it is on screen, and only while it is. They
  // are gated by hand because there is no central shortcut registry — the same
  // apology `agents-section.tsx` makes, and the same shape.
  useEffect(() => {
    if (busy) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (isTypingInField(event.target)) return;
      const verdict = permissionVerdictFor(event, scope !== null);
      if (!verdict) return;
      event.preventDefault();
      onAnswer(verdict);
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [busy, scope, onAnswer]);

  return (
    <section className="permission" aria-label="Permission request">
      <div className="permission__head">
        <span className="permission__dot" aria-hidden="true" />
        <span className="permission__who">{sessionLabel} needs permission</span>
        <span className="permission__verb">{verbLabel(request.verb)}</span>
        {/* No `title`: a native tooltip would put the agent's own tool input
            into an *attribute*, and the render test's tag sweep is right to
            object. React escapes it safely, but a security check that has to be
            exempted stops being one — and an ellipsised path is a small loss
            beside that. */}
        <span className="permission__target">{request.target}</span>
        <span className="permission__aside">
          {radius.length > 0 ? `${radius.join(' · ')} · ` : ''}
          asked {askedAgo(Number(request.askedAt), now)} ago
        </span>
      </div>

      {diff.length > 0 && (
        <div className="permission__diff">
          {diff.map((line, index) => {
            const sign = diffSign(line);
            return (
              <div
                // Diff lines have no id and repeat freely; the index is their
                // identity, and the list is replaced whole on every ask.
                // biome-ignore lint/suspicious/noArrayIndexKey: no stable id exists
                key={index}
                className={`permission__diff-line permission__diff-line--${sign}`}
              >
                <span className="permission__diff-sign" aria-hidden="true">
                  {sign === 'add' ? '+' : sign === 'remove' ? '−' : ' '}
                </span>
                <span className="permission__diff-text">{diffText(line)}</span>
              </div>
            );
          })}
          {summary && <div className="permission__diff-summary">{summary}</div>}
        </div>
      )}

      {diff.length === 0 && request.sensitive && (
        <p className="permission__redacted">
          The contents are not shown. A file with this name usually holds a secret, and a prompt
          that leaked the secret it was asking about would be worse than no prompt.
        </p>
      )}

      <div className="permission__answers">
        <button
          type="button"
          className="btn--primary permission__answer permission__answer--allow"
          disabled={busy}
          onClick={() => onAnswer('ALLOW_ONCE')}
        >
          Allow once <kbd>{PERMISSION_CHORD_HINT.ALLOW_ONCE}</kbd>
        </button>
        {scope && (
          <button
            type="button"
            className="permission__answer"
            disabled={busy}
            onClick={() => onAnswer('ALLOW_FOR_RUN')}
          >
            {scope} <kbd>{PERMISSION_CHORD_HINT.ALLOW_FOR_RUN}</kbd>
          </button>
        )}
        <button
          type="button"
          className="btn--danger permission__answer permission__answer--deny"
          disabled={busy}
          onClick={() => onAnswer('DENY')}
        >
          Deny <kbd>{PERMISSION_CHORD_HINT.DENY}</kbd>
        </button>
        <span className="agent-console__spacer" />
        {onReveal && (
          <button type="button" className="btn--bare permission__aside-action" onClick={onReveal}>
            Answer in its terminal
          </button>
        )}
      </div>
    </section>
  );
}

/**
 * Whether a keystroke belongs to something the user is typing in.
 *
 * The same check `island.tsx` makes, and for the same reason it only names
 * `INPUT`: Monaco's own editing surface is a hidden `textarea`, so excluding
 * textareas wholesale would hand every chord to the editor.
 */
function isTypingInField(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return target.tagName === 'INPUT' || target.isContentEditable;
}
