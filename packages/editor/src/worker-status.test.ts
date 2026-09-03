import { describe, expect, it, vi } from 'vitest';
import { type WorkerLike, WorkerStatus, type WorkerStatusSnapshot } from './worker-status';

interface FakeWorker extends WorkerLike {
  fire(type: string, event: Event): void;
}

function fakeWorker(): FakeWorker {
  const listeners = new Map<string, Array<(event: Event) => void>>();
  return {
    addEventListener(type: string, listener: EventListenerOrEventListenerObject | null) {
      if (typeof listener !== 'function') return;
      listeners.set(type, [...(listeners.get(type) ?? []), listener as (event: Event) => void]);
    },
    fire(type, event) {
      for (const listener of listeners.get(type) ?? []) listener(event);
    },
  } as FakeWorker;
}

function fakeTarget() {
  const listeners = new Map<string, Set<EventListenerOrEventListenerObject>>();
  return {
    addEventListener(type: string, listener: EventListenerOrEventListenerObject | null) {
      if (listener) listeners.set(type, new Set([...(listeners.get(type) ?? []), listener]));
    },
    removeEventListener(type: string, listener: EventListenerOrEventListenerObject | null) {
      if (listener) listeners.get(type)?.delete(listener);
    },
    fire(type: string, event: Event) {
      for (const listener of listeners.get(type) ?? []) {
        if (typeof listener === 'function') listener(event);
      }
    },
    count(type: string) {
      return listeners.get(type)?.size ?? 0;
    },
  };
}

describe('WorkerStatus', () => {
  it('records started workers and runtime errors', () => {
    const log = vi.fn();
    const status = new WorkerStatus({ log });
    const worker = status.track('json', fakeWorker);
    expect(status.snapshot()).toEqual({ failures: [], started: ['json'] });

    worker.fire('error', {
      message: 'boom',
      filename: 'json.worker.js',
      lineno: 3,
    } as unknown as Event);
    expect(status.snapshot().failures).toEqual([
      { label: 'json', phase: 'runtime', reason: 'boom (json.worker.js:3)' },
    ]);
    expect(log).toHaveBeenCalledWith(
      'monaco worker failure',
      expect.objectContaining({ label: 'json' }),
    );
  });

  it('describes an error event without a message as a blocked script', () => {
    const status = new WorkerStatus({ log: () => {} });
    const worker = status.track('typescript', fakeWorker);
    worker.fire('error', {} as Event);
    expect(status.snapshot().failures[0]?.reason).toMatch(/blocked or failed to load/);
  });

  it('records a construction error and rethrows it for Monaco to handle', () => {
    const status = new WorkerStatus({ log: () => {} });
    expect(() =>
      status.track('editorWorkerService', () => {
        throw new Error('Worker is not defined');
      }),
    ).toThrow('Worker is not defined');
    expect(status.snapshot()).toEqual({
      failures: [
        { label: 'editorWorkerService', phase: 'create', reason: 'Worker is not defined' },
      ],
      started: [],
    });
  });

  it('captures CSP violations with their directive and URI, until disposed', () => {
    const target = fakeTarget();
    const status = new WorkerStatus({ log: () => {}, violationTarget: target });
    const seen: WorkerStatusSnapshot[] = [];
    const unsubscribe = status.subscribe((snapshot) => seen.push(snapshot));

    target.fire('securitypolicyviolation', {
      effectiveDirective: 'worker-src',
      blockedURI: 'blob',
      sourceFile: 'http://localhost:1420/src/main.tsx',
      lineNumber: 12,
    } as unknown as Event);
    expect(seen).toHaveLength(1);
    expect(seen[0]?.failures).toEqual([
      {
        label: 'worker-src',
        phase: 'csp',
        reason: 'CSP worker-src blocked blob at http://localhost:1420/src/main.tsx:12',
      },
    ]);

    unsubscribe();
    target.fire('securitypolicyviolation', {
      effectiveDirective: 'script-src',
    } as unknown as Event);
    expect(seen).toHaveLength(1);
    expect(status.snapshot().failures).toHaveLength(2);

    status.dispose();
    expect(target.count('securitypolicyviolation')).toBe(0);
    target.fire('securitypolicyviolation', { effectiveDirective: 'img-src' } as unknown as Event);
    expect(status.snapshot().failures).toHaveLength(2);
  });
});
