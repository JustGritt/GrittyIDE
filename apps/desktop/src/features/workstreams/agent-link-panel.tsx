import type { AgentProvider, WorkstreamId } from '@gritty/protocol';
import { type FormEvent, useState } from 'react';
import { providerLabel } from '../agents/agent-status';
import { taskLine } from '../agents/agents-reducer';
import type { AgentsController } from '../agents/use-agents';
import {
  linkedSessionLabel,
  linkedSessions,
  linkedStatusHint,
  linkedStatusLabel,
  linkedStatusTone,
  linkedSummary,
  workstreamsOfSession,
} from './agent-links';
import type { WorkstreamsController } from './use-workstreams';

/**
 * Linking agent sessions to workstreams, in the island (T223 §16/§17).
 *
 * This is the half of the task the roster alone cannot do: without it the
 * island shows *what is running* beside *what the work is* and never says
 * which is which. One section rather than a control on every row, following
 * the ownership section right above it — the question it answers is about the
 * workspace, not about one session — and it is the only place a link is made,
 * because the rail is deliberately read-only about sessions it does not own.
 *
 * Linking grants no control. It records that this session is working on this
 * piece of work, and nothing here can prompt, answer or cancel one.
 */
export function AgentLinkSection({
  controller,
  agents,
}: {
  controller: WorkstreamsController;
  agents: AgentsController;
}) {
  const { workstreams, agentLinks, agentLinksLoaded } = controller;
  const byId = new Map(workstreams.map((workstream) => [workstream.id as string, workstream]));
  // A link whose workstream is gone cannot be drawn or removed; the database
  // cascades them away, so this only guards a partially-loaded renderer.
  const drawable = agentLinks.filter((link) => byId.has(link.workstreamId as string));

  return (
    <section className="agent-links" aria-label="Agent sessions linked to work">
      <div className="agent-links__head">
        <span className="agent-links__title">Linked work</span>
        <span className="island__spacer" />
        <span className="island__meta">
          {drawable.length === 0
            ? 'nothing linked'
            : `${drawable.length} link${drawable.length === 1 ? '' : 's'}`}
        </span>
      </div>

      {!agentLinksLoaded ? (
        <p className="island__empty">Reading the links…</p>
      ) : drawable.length === 0 ? (
        <p className="island__empty">
          Nothing is linked. Attaching a session to a workstream is what turns the roster from a
          list of processes into a view of the work — and it is only a record: Gritty still does not
          run, answer or stop these sessions.
        </p>
      ) : (
        <div className="agent-links__list">
          {workstreams
            .filter((workstream) => drawable.some((link) => link.workstreamId === workstream.id))
            .map((workstream) => (
              <div className="agent-links__group" key={workstream.id}>
                <span className="agent-links__work">{workstream.title}</span>
                <LinkedSessions
                  controller={controller}
                  agents={agents}
                  workstreamId={workstream.id}
                  removable
                />
              </div>
            ))}
        </div>
      )}

      <LinkForm controller={controller} agents={agents} />
    </section>
  );
}

/**
 * One workstream's linked sessions, with their live activity when they are in
 * the roster and "not running" when they are not (§12, §17).
 *
 * Never an error and never an invented row: a link outlives the process it
 * names, and saying so is the honest thing — it is what worked on this.
 */
export function LinkedSessions({
  controller,
  agents,
  workstreamId,
  removable = false,
}: {
  controller: WorkstreamsController;
  agents: AgentsController;
  workstreamId: WorkstreamId;
  /** Whether each row offers "Unlink". Off where the row is only a summary. */
  removable?: boolean;
}) {
  const entries = linkedSessions(controller.agentLinks, agents.roster, workstreamId);
  if (entries.length === 0) return null;

  return (
    <ul className="agent-links__sessions">
      {entries.map((entry) => (
        <li className="agent-links__session" key={`${entry.link.provider}:${entry.link.sessionId}`}>
          <span
            className={`island__dot island__dot--${dotTone(linkedStatusTone(entry))}`}
            aria-hidden="true"
          />
          {/* Everything textual here came out of a transcript: rendered as
              text, never as markup and never as a path anything opens. */}
          <span className="agent-links__task">{linkedSessionLabel(entry)}</span>
          <span className="agent-links__status" title={linkedStatusHint(entry)}>
            {linkedStatusLabel(entry)}
          </span>
          {removable && (
            <button
              type="button"
              className="btn--bare"
              disabled={controller.busy}
              onClick={() =>
                void controller.unlinkAgent(workstreamId, entry.link.provider, entry.link.sessionId)
              }
            >
              Unlink
            </button>
          )}
        </li>
      ))}
    </ul>
  );
}

/** The one-line count for a roster row: "2 agents · 1 not running". */
export function linkedRowLabel(
  controller: WorkstreamsController,
  agents: AgentsController,
  workstreamId: WorkstreamId,
): string | null {
  return linkedSummary(linkedSessions(controller.agentLinks, agents.roster, workstreamId));
}

function dotTone(tone: string): string {
  switch (tone) {
    case 'working':
      return 'active';
    case 'blocked':
      return 'blocked';
    case 'done':
      return 'waiting';
    default:
      return 'idle';
  }
}

/**
 * Attach a session to a workstream. Two selects and a button, following the
 * ownership claim form: nothing is validated here beyond "both were chosen",
 * because a refusal from the core that names its reason is more use than a
 * field that quietly greys out.
 *
 * The session list is the roster as it is now. A session that has already left
 * it cannot be linked from here — the *command* allows it, deliberately, but
 * offering a picker of sessions nobody can see would be a worse control than
 * none.
 */
function LinkForm({
  controller,
  agents,
}: {
  controller: WorkstreamsController;
  agents: AgentsController;
}) {
  const linkable = controller.workstreams.filter(
    (workstream) => workstream.state !== 'ABANDONED' && workstream.state !== 'DONE',
  );
  const [workstreamId, setWorkstreamId] = useState<string>('');
  const [sessionId, setSessionId] = useState<string>('');

  const workstream = linkable.find((candidate) => (candidate.id as string) === workstreamId);
  const session = agents.roster.sessions.find((candidate) => candidate.id === sessionId);
  const already =
    session !== undefined &&
    workstream !== undefined &&
    workstreamsOfSession(controller.agentLinks, [workstream], session).length > 0;

  function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!workstream || !session) return;
    void controller.linkAgent(workstream.id, session.provider as AgentProvider, session.id);
    setSessionId('');
  }

  if (linkable.length === 0 || agents.roster.sessions.length === 0) return null;

  return (
    <form className="agent-links__form" onSubmit={submit}>
      <label className="agent-links__field">
        <span className="label">Session</span>
        <select value={sessionId} onChange={(event) => setSessionId(event.target.value)}>
          <option value="">Choose…</option>
          {agents.roster.sessions.map((candidate) => (
            <option key={candidate.id} value={candidate.id}>
              {taskLine(candidate) ?? `${providerLabel(candidate.provider)} · ${candidate.id}`}
            </option>
          ))}
        </select>
      </label>
      <label className="agent-links__field">
        <span className="label">Works on</span>
        <select value={workstreamId} onChange={(event) => setWorkstreamId(event.target.value)}>
          <option value="">Choose…</option>
          {linkable.map((candidate) => (
            <option key={candidate.id} value={candidate.id as string}>
              {candidate.title}
            </option>
          ))}
        </select>
      </label>
      <button type="submit" disabled={!workstream || !session || already || controller.busy}>
        {already ? 'Already linked' : 'Link'}
      </button>
    </form>
  );
}
