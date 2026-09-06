import type {
  OwnershipConflict,
  OwnershipLease,
  OwnershipMap,
  OwnershipTarget,
  WorkstreamId,
} from '@gritty/protocol';

/**
 * Presenting the ownership map (T325).
 *
 * Pure, and deliberately thin: it formats, groups and counts. **It does not
 * decide whether two targets overlap.** That lives in the core, for the same
 * reason `Workstream.allowedTransitions` is published on the wire — a second
 * copy of the rule would drift from the first, and this one would drift
 * towards saying "no conflict", which is the expensive direction.
 */

export type TargetKind = OwnershipTarget['type'];

export const TARGET_KINDS: readonly TargetKind[] = ['FILE', 'DIRECTORY', 'GLOB'];

/** What the user types for each kind, on the label of the field. */
export const TARGET_KIND_LABEL: Record<TargetKind, string> = {
  FILE: 'File',
  DIRECTORY: 'Folder',
  GLOB: 'Pattern',
};

export const TARGET_KIND_PLACEHOLDER: Record<TargetKind, string> = {
  FILE: 'src/auth/session.ts',
  DIRECTORY: 'src/auth',
  GLOB: 'tests/auth/**/*.ts',
};

/** The scope as it reads in a row. The root folder is the whole workspace. */
export function targetLabel(target: OwnershipTarget): string {
  switch (target.type) {
    case 'FILE':
      return target.path;
    case 'DIRECTORY':
      return target.path === '' ? 'the whole workspace' : `${target.path}/`;
    case 'GLOB':
      return target.pattern;
  }
}

/**
 * Turn what the user typed into a target, or `null` when there is nothing to
 * claim. Everything else the core validates — a path that escapes the
 * workspace is refused there, on the way in, not talked out of here.
 */
export function buildTarget(kind: TargetKind, value: string): OwnershipTarget | null {
  const trimmed = value.trim();
  if (kind === 'GLOB') return trimmed === '' ? null : { type: 'GLOB', pattern: trimmed };
  // A folder may be the root, which is the empty path; a file may not.
  const path = trimmed.replace(/^\/+|\/+$/g, '');
  if (kind === 'FILE') return path === '' ? null : { type: 'FILE', path };
  return { type: 'DIRECTORY', path };
}

/** Leases still keeping others out. */
export function heldLeases(map: OwnershipMap): OwnershipLease[] {
  return map.leases.filter((lease) => lease.state === 'ACTIVE');
}

export function leasesOf(map: OwnershipMap, workstreamId: WorkstreamId): OwnershipLease[] {
  return map.leases.filter((lease) => lease.workstreamId === workstreamId);
}

/**
 * The conflicts the user has still to deal with. What the queue draws.
 *
 * Two states, not one (T326). A `WAITING` row is a request that was refused
 * before the work started; an `OBSERVED` row is a write that already happened
 * on somebody else's ground. They are the same shape — someone is in someone
 * else's way — so they share a row, a filter and a queue, and differ only in
 * what the words say. There is deliberately no fifth `AttentionFilter`: the
 * user is looking for ownership trouble, not for a taxonomy of it.
 */
export function waitingConflicts(map: OwnershipMap): OwnershipConflict[] {
  return map.conflicts.filter(
    (conflict) => conflict.state === 'WAITING' || conflict.state === 'OBSERVED',
  );
}

/** Whether this row is a write that already landed, rather than a refusal. */
export function isViolation(conflict: OwnershipConflict): boolean {
  return conflict.state === 'OBSERVED';
}

/**
 * The one line a row leads with, in the two tenses that separate them.
 *
 * A refusal is about a request: "Payments asked for src/auth/, which
 * Authentication already holds." A violation is about a write that is already
 * on disk: "Payments wrote src/auth/session.ts while Authentication held it."
 * Neither says *locked* or *blocked*, because nothing was: Gritty observes
 * ownership, it does not enforce it (`docs/architecture/ownership.md` 17-26).
 */
export function conflictHeadline(
  conflict: OwnershipConflict,
  requester: string,
  owner: string,
): string {
  const target = targetLabel(conflict.target);
  return isViolation(conflict)
    ? `${requester} wrote ${target} while ${owner} held it.`
    : `${requester} asked for ${target}, which ${owner} already holds.`;
}

/** The same distinction, at row length, under the name in the queue. */
export function conflictSummary(conflict: OwnershipConflict, owner: string): string {
  const target = targetLabel(conflict.target);
  return isViolation(conflict)
    ? `Wrote ${target}, owned by ${owner}.`
    : `Wants ${target}, owned by ${owner}.`;
}

/**
 * Which of `ownership.resolve`'s two verbs this row accepts.
 *
 * `OVERRIDE` releases the holder's lease so a *pending* request can be made
 * again (ADR-019 §8). A violation has no pending request — the toucher already
 * wrote — so the core answers `InvalidInput`, and the button is not offered
 * rather than offered and refused.
 */
export function conflictResolutions(
  conflict: OwnershipConflict,
): readonly ('CANCEL' | 'OVERRIDE')[] {
  return isViolation(conflict) ? ['CANCEL'] : ['CANCEL', 'OVERRIDE'];
}

/** What `CANCEL` means here: withdrawing a request, or acknowledging a write. */
export function conflictCancelLabel(conflict: OwnershipConflict): string {
  return isViolation(conflict) ? 'Acknowledge' : 'Withdraw the request';
}

/** Why the row is here at all, under the actions. */
export function conflictHint(conflict: OwnershipConflict, requester: string): string {
  return isViolation(conflict)
    ? `Gritty noticed this after the write. Nothing was stopped, and nothing is undone — release the claim, or acknowledge it and ${requester} keeps going.`
    : `Releasing it does not hand the ground over: ${requester} has to claim it again.`;
}

/** The dot's colour role, from the tokens rather than a palette of its own. */
export type LeaseTone = 'active' | 'stale' | 'expired';

export function leaseTone(lease: OwnershipLease): LeaseTone {
  if (lease.state === 'EXPIRED') return 'expired';
  return lease.stale ? 'stale' : 'active';
}

/**
 * What the row says about a lease, and why. A stale one is reported rather
 * than repaired: it still holds, and only the user takes it away.
 */
export function leaseStateLabel(lease: OwnershipLease): string {
  if (lease.state === 'EXPIRED') return 'Expired';
  if (lease.state === 'RELEASED') return 'Released';
  return lease.stale ? 'Held by finished work' : 'Held';
}

export function leaseStateHint(lease: OwnershipLease): string {
  if (lease.state === 'EXPIRED') return 'Its deadline passed, so it no longer keeps anyone out.';
  if (lease.stale) {
    return 'The workstream holding it is finished or abandoned. It still keeps others out until you release it.';
  }
  return 'No other workstream may claim this ground.';
}

/** How the count reads on the roster row: "owns 4". */
export function ownsLabel(map: OwnershipMap, workstreamId: WorkstreamId): string | null {
  const held = leasesOf(map, workstreamId).filter((lease) => lease.state === 'ACTIVE').length;
  return held === 0 ? null : `owns ${held}`;
}
