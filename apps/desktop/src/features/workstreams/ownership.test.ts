import type {
  LeaseId,
  OwnershipConflict,
  OwnershipLease,
  OwnershipMap,
  WorkstreamId,
} from '@gritty/protocol';
import { describe, expect, it } from 'vitest';
import {
  buildTarget,
  conflictCancelLabel,
  conflictHeadline,
  conflictHint,
  conflictResolutions,
  conflictSummary,
  heldLeases,
  isViolation,
  leaseStateLabel,
  leasesOf,
  leaseTone,
  ownsLabel,
  TARGET_KINDS,
  targetLabel,
  waitingConflicts,
} from './ownership';

function lease(id: string, patch: Partial<OwnershipLease> = {}): OwnershipLease {
  return {
    id: id as LeaseId,
    workspaceId: 'w-1' as OwnershipLease['workspaceId'],
    workstreamId: 'a' as WorkstreamId,
    target: { type: 'DIRECTORY', path: 'src/auth' },
    mode: 'WRITE',
    acquiredAt: 1,
    state: 'ACTIVE',
    stale: false,
    ...patch,
  };
}

describe('ownership targets', () => {
  it('reads a target back the way it was claimed', () => {
    expect(targetLabel({ type: 'FILE', path: 'src/auth/session.ts' })).toBe('src/auth/session.ts');
    expect(targetLabel({ type: 'DIRECTORY', path: 'src/auth' })).toBe('src/auth/');
    expect(targetLabel({ type: 'GLOB', pattern: 'tests/**/*.ts' })).toBe('tests/**/*.ts');
  });

  it('names the root folder rather than showing an empty string', () => {
    expect(targetLabel({ type: 'DIRECTORY', path: '' })).toBe('the whole workspace');
  });

  it('builds a target from what was typed, and nothing from nothing', () => {
    expect(buildTarget('FILE', ' src/auth/session.ts ')).toEqual({
      type: 'FILE',
      path: 'src/auth/session.ts',
    });
    expect(buildTarget('DIRECTORY', '/src/auth/')).toEqual({ type: 'DIRECTORY', path: 'src/auth' });
    expect(buildTarget('GLOB', ' **/*.rs ')).toEqual({ type: 'GLOB', pattern: '**/*.rs' });
    expect(buildTarget('FILE', '   ')).toBeNull();
    expect(buildTarget('GLOB', '')).toBeNull();
  });

  it('lets a folder be the whole workspace but never a file', () => {
    expect(buildTarget('DIRECTORY', '')).toEqual({ type: 'DIRECTORY', path: '' });
    expect(buildTarget('FILE', '')).toBeNull();
  });

  it('passes a path that escapes the workspace straight to the core', () => {
    // Refusing it here as well would be a second copy of a rule the core
    // already enforces on deserialization, and the two would drift.
    expect(buildTarget('FILE', '../../etc/passwd')).toEqual({
      type: 'FILE',
      path: '../../etc/passwd',
    });
  });

  it('covers every target kind the wire can carry', () => {
    // Adding a kind in Rust fails this compilation rather than silently
    // rendering a blank row.
    expect([...TARGET_KINDS].sort()).toEqual(['DIRECTORY', 'FILE', 'GLOB']);
  });
});

describe('ownership map', () => {
  const map: OwnershipMap = {
    leases: [
      lease('held'),
      lease('stale', { stale: true }),
      lease('expired', { state: 'EXPIRED' }),
      lease('theirs', { workstreamId: 'b' as WorkstreamId }),
    ],
    conflicts: [],
  };

  it('separates what still holds from what is only listed', () => {
    expect(heldLeases(map).map((entry) => entry.id)).toEqual(['held', 'stale', 'theirs']);
  });

  it('groups by holder', () => {
    expect(leasesOf(map, 'a' as WorkstreamId).map((entry) => entry.id)).toEqual([
      'held',
      'stale',
      'expired',
    ]);
    expect(ownsLabel(map, 'a' as WorkstreamId)).toBe('owns 2');
    expect(ownsLabel(map, 'nobody' as WorkstreamId)).toBeNull();
  });

  it('says what a lease is and why, without repairing it', () => {
    expect(leaseTone(lease('x'))).toBe('active');
    expect(leaseTone(lease('x', { stale: true }))).toBe('stale');
    expect(leaseTone(lease('x', { state: 'EXPIRED' }))).toBe('expired');
    // A stale lease still holds: it says so rather than reading as gone.
    expect(leaseStateLabel(lease('x', { stale: true }))).toBe('Held by finished work');
    expect(leaseStateLabel(lease('x'))).toBe('Held');
    expect(leaseStateLabel(lease('x', { state: 'EXPIRED' }))).toBe('Expired');
  });

  it('keeps only the conflicts that are still blocking someone', () => {
    const conflicts: OwnershipMap['conflicts'] = [
      {
        id: 'c1' as OwnershipMap['conflicts'][number]['id'],
        workspaceId: 'w-1' as OwnershipLease['workspaceId'],
        requestedBy: 'b' as WorkstreamId,
        ownedBy: 'a' as WorkstreamId,
        holdingLeaseId: 'held' as LeaseId,
        target: { type: 'FILE', path: 'src/auth/session.ts' },
        createdAt: 2,
        state: 'WAITING',
      },
      {
        id: 'c2' as OwnershipMap['conflicts'][number]['id'],
        workspaceId: 'w-1' as OwnershipLease['workspaceId'],
        requestedBy: 'b' as WorkstreamId,
        ownedBy: 'a' as WorkstreamId,
        holdingLeaseId: 'held' as LeaseId,
        target: { type: 'FILE', path: 'src/auth/types.ts' },
        createdAt: 3,
        state: 'RESOLVED',
      },
    ];
    expect(waitingConflicts({ leases: [], conflicts }).map((entry) => entry.id)).toEqual(['c1']);
  });
});

describe('an observed violation (T326)', () => {
  function violation(patch: Partial<OwnershipConflict> = {}): OwnershipConflict {
    return {
      id: 'v1' as OwnershipConflict['id'],
      workspaceId: 'w-1' as OwnershipLease['workspaceId'],
      requestedBy: 'b' as WorkstreamId,
      ownedBy: 'a' as WorkstreamId,
      holdingLeaseId: 'held' as LeaseId,
      target: { type: 'FILE', path: 'src/auth/session.ts' },
      createdAt: 4,
      state: 'OBSERVED',
      ...patch,
    };
  }

  const refusal = violation({
    id: 'r1' as OwnershipConflict['id'],
    state: 'WAITING',
    target: { type: 'DIRECTORY', path: 'src/auth' },
  });

  it('reaches the queue beside an ordinary refusal', () => {
    const conflicts = [
      refusal,
      violation(),
      violation({ id: 'done' as OwnershipConflict['id'], state: 'CANCELLED' }),
    ];
    expect(waitingConflicts({ leases: [], conflicts }).map((entry) => entry.id)).toEqual([
      'r1',
      'v1',
    ]);
  });

  it('is told apart from a refusal by state, not by shape', () => {
    expect(isViolation(violation())).toBe(true);
    expect(isViolation(refusal)).toBe(false);
  });

  it('says a write happened where a refusal says a request was made', () => {
    expect(conflictHeadline(violation(), 'Payments', 'Authentication')).toBe(
      'Payments wrote src/auth/session.ts while Authentication held it.',
    );
    expect(conflictHeadline(refusal, 'Payments', 'Authentication')).toBe(
      'Payments asked for src/auth/, which Authentication already holds.',
    );
    expect(conflictSummary(violation(), 'Authentication')).toBe(
      'Wrote src/auth/session.ts, owned by Authentication.',
    );
    expect(conflictSummary(refusal, 'Authentication')).toBe(
      'Wants src/auth/, owned by Authentication.',
    );
  });

  it('never calls either one locked or blocked', () => {
    // Gritty observes ownership; it does not enforce it, and the words next to
    // a claim are where that promise is either kept or quietly broken.
    const words = [
      conflictHeadline(violation(), 'Payments', 'Authentication'),
      conflictHeadline(refusal, 'Payments', 'Authentication'),
      conflictSummary(violation(), 'Authentication'),
      conflictHint(violation(), 'Payments'),
      conflictHint(refusal, 'Payments'),
    ].join(' ');
    expect(words).not.toMatch(/locked|blocked/i);
  });

  it('offers OVERRIDE only where there is something to unblock', () => {
    // The core answers InvalidInput for an override on a violation, so the
    // refusal is rendered as an absent button rather than a failed click.
    expect(conflictResolutions(violation())).toEqual(['CANCEL']);
    expect(conflictResolutions(refusal)).toEqual(['CANCEL', 'OVERRIDE']);
    expect(conflictCancelLabel(violation())).toBe('Acknowledge');
    expect(conflictCancelLabel(refusal)).toBe('Withdraw the request');
  });
});
