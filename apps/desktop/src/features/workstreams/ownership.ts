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

/** The conflicts that are still blocking someone. What the queue draws. */
export function waitingConflicts(map: OwnershipMap): OwnershipConflict[] {
  return map.conflicts.filter((conflict) => conflict.state === 'WAITING');
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
