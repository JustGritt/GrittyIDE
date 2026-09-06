import type { OwnershipLease, Workstream, WorkstreamId } from '@gritty/protocol';
import { type FormEvent, useState } from 'react';
import { stateLabel } from './attention';
import {
  buildTarget,
  leaseStateHint,
  leaseStateLabel,
  leaseTone,
  TARGET_KIND_LABEL,
  TARGET_KIND_PLACEHOLDER,
  TARGET_KINDS,
  type TargetKind,
  targetLabel,
} from './ownership';
import type { WorkstreamsController } from './use-workstreams';

/**
 * Ownership inside the island (T325): which workstream may write where.
 *
 * One section rather than a control on every roster row, because the question
 * it answers — *who owns this file, and why can't the other one touch it* — is
 * about the workspace, not about one piece of work.
 *
 * It says plainly that this is **observed** ownership. Gritty refuses a second
 * claim; it does not stop a write. Presenting a registry as protection it does
 * not provide would be worse than not having one.
 */
export function OwnershipSection({ controller }: { controller: WorkstreamsController }) {
  const { ownership, ownershipLoaded, workstreams } = controller;
  const byId = new Map(workstreams.map((workstream) => [workstream.id as string, workstream]));
  const held = ownership.leases;

  return (
    <section className="ownership" aria-label="Workspace ownership">
      <div className="ownership__head">
        <span className="ownership__title">Ownership</span>
        <span className="island__spacer" />
        <span className="island__meta">
          {held.length === 0 ? 'nothing claimed' : countOf(held)}
        </span>
      </div>

      {!ownershipLoaded ? (
        <p className="island__empty">Reading the ownership map…</p>
      ) : held.length === 0 ? (
        <p className="island__empty">
          Nothing is claimed. A workstream can take the files, folders or patterns it is going to
          change, so a second one is told before it starts rather than after.
        </p>
      ) : (
        <div className="ownership__list">
          {held.map((lease) => (
            <LeaseRow
              key={lease.id}
              lease={lease}
              holder={byId.get(lease.workstreamId as string)}
              controller={controller}
            />
          ))}
        </div>
      )}

      <ClaimForm controller={controller} />

      <p className="ownership__note">
        Gritty refuses a second claim on the same ground. It does not stop a write — an agent that
        edits an owned file anyway will not be blocked by this.
      </p>
    </section>
  );
}

function countOf(leases: readonly OwnershipLease[]): string {
  const stale = leases.filter((lease) => lease.stale || lease.state === 'EXPIRED').length;
  const base = `${leases.length} claim${leases.length === 1 ? '' : 's'}`;
  return stale === 0 ? base : `${base} · ${stale} to clear`;
}

function LeaseRow({
  lease,
  holder,
  controller,
}: {
  lease: OwnershipLease;
  /** Absent only while the roster and the map are a beat apart mid-load. */
  holder: Workstream | undefined;
  controller: WorkstreamsController;
}) {
  return (
    <div className={`ownership__row ownership__row--${leaseTone(lease)}`}>
      <span className={`island__dot island__dot--${dotTone(lease)}`} aria-hidden="true" />
      <div className="ownership__body">
        <div className="ownership__line">
          {/* A target is a path the user typed, rendered as text: nothing here
              opens it, runs it or resolves it. */}
          <span className="ownership__target" title={targetLabel(lease.target)}>
            {targetLabel(lease.target)}
          </span>
          <span className="ownership__state" title={leaseStateHint(lease)}>
            {leaseStateLabel(lease)}
          </span>
        </div>
        <div className="ownership__meta">
          <span>{holder ? holder.title : 'an unknown workstream'}</span>
          {holder && <span>{stateLabel(holder.state)}</span>}
          {lease.note && <span title={lease.note}>{lease.note}</span>}
        </div>
      </div>
      <button
        type="button"
        className="btn--bare"
        disabled={controller.busy}
        onClick={() => void controller.release(lease.id)}
      >
        Release
      </button>
    </div>
  );
}

function dotTone(lease: OwnershipLease): string {
  const tone = leaseTone(lease);
  if (tone === 'stale') return 'blocked';
  return tone === 'expired' ? 'idle' : 'active';
}

/**
 * Claim ground for a workstream. Nothing is optimistic and nothing is
 * validated here beyond "there is something to claim": a path that escapes the
 * workspace is refused by the core, and a refusal that names the holder is
 * more use than a field that quietly greys out.
 */
function ClaimForm({ controller }: { controller: WorkstreamsController }) {
  const claimable = controller.workstreams.filter((workstream) => workstream.state !== 'ABANDONED');
  const [workstreamId, setWorkstreamId] = useState<string>('');
  const [kind, setKind] = useState<TargetKind>('DIRECTORY');
  const [value, setValue] = useState('');
  const [conflict, setConflict] = useState<string | null>(null);

  const selected = claimable.find((workstream) => (workstream.id as string) === workstreamId);
  const target = buildTarget(kind, value);
  // A folder may be the whole workspace, which is the empty path; everything
  // else needs something typed.
  const ready = selected !== undefined && (target !== null || kind === 'DIRECTORY');

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!selected || !target) return;
    setConflict(null);
    const answer = await controller.acquire(selected.id as WorkstreamId, target);
    if (!answer) return;
    if (answer.type === 'CONFLICTED') {
      // The refusal is already a row in the queue; this says so where the
      // click happened, so the user is not left wondering what took.
      setConflict(targetLabel(answer.conflict.target));
      return;
    }
    setValue('');
  }

  if (claimable.length === 0) return null;

  return (
    <form className="ownership__claim" onSubmit={submit}>
      <label className="ownership__field">
        <span className="label">Workstream</span>
        <select value={workstreamId} onChange={(event) => setWorkstreamId(event.target.value)}>
          <option value="">Choose…</option>
          {claimable.map((workstream) => (
            <option key={workstream.id} value={workstream.id as string}>
              {workstream.title}
            </option>
          ))}
        </select>
      </label>
      <label className="ownership__field">
        <span className="label">Claims</span>
        <select value={kind} onChange={(event) => setKind(event.target.value as TargetKind)}>
          {TARGET_KINDS.map((option) => (
            <option key={option} value={option}>
              {TARGET_KIND_LABEL[option]}
            </option>
          ))}
        </select>
      </label>
      <input
        value={value}
        onChange={(event) => setValue(event.target.value)}
        placeholder={TARGET_KIND_PLACEHOLDER[kind]}
        aria-label={`${TARGET_KIND_LABEL[kind]} to claim`}
        spellCheck={false}
        maxLength={240}
      />
      <button type="submit" disabled={!ready || controller.busy}>
        Claim
      </button>
      {conflict && (
        <p className="ownership__refused">
          {conflict} is already owned. The conflict is in the attention queue.
        </p>
      )}
    </form>
  );
}
