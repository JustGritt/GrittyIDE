import type { DocumentId } from '@gritty/protocol';
import { describe, expect, it } from 'vitest';
import { type SchedulerTimers, UpdateScheduler } from './update-scheduler';

const A = 'a' as DocumentId;
const BIG = 'big' as DocumentId;

function fakeTimers() {
  let now = 0;
  let sequence = 0;
  const queue: Array<{ id: number; at: number; callback: () => void }> = [];
  const timers: SchedulerTimers = {
    set(callback, ms) {
      sequence += 1;
      queue.push({ id: sequence, at: now + ms, callback });
      return sequence;
    },
    clear(handle) {
      const index = queue.findIndex((entry) => entry.id === handle);
      if (index !== -1) queue.splice(index, 1);
    },
  };
  return {
    timers,
    advance(ms: number) {
      now += ms;
      for (;;) {
        const index = queue.findIndex((entry) => entry.at <= now);
        if (index === -1) return;
        const [entry] = queue.splice(index, 1);
        entry?.callback();
      }
    },
    pending: () => queue.length,
  };
}

function deferred<T>() {
  let resolve: (value: T) => void = () => undefined;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

/** Let chained promises run. */
const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

function harness() {
  const clock = fakeTimers();
  const flushes: DocumentId[] = [];
  const results: Array<ReturnType<typeof deferred<boolean>>> = [];
  const scheduler = new UpdateScheduler<boolean>({
    debounceMs: (documentId) => (documentId === BIG ? 1000 : 300),
    flush(documentId) {
      flushes.push(documentId);
      const result = deferred<boolean>();
      results.push(result);
      return result.promise;
    },
    timers: clock.timers,
  });
  return { clock, flushes, results, scheduler };
}

describe('UpdateScheduler', () => {
  it('coalesces touches into one flush after the debounce', async () => {
    const { clock, flushes, scheduler } = harness();
    scheduler.touch(A);
    clock.advance(200);
    scheduler.touch(A);
    clock.advance(299);
    await tick();
    expect(flushes).toEqual([]);
    expect(scheduler.isScheduled(A)).toBe(true);
    clock.advance(1);
    await tick();
    expect(flushes).toEqual([A]);
  });

  it('uses the per-document debounce', async () => {
    const { clock, flushes, scheduler } = harness();
    scheduler.touch(BIG);
    scheduler.touch(A);
    clock.advance(300);
    await tick();
    expect(flushes).toEqual([A]);
    clock.advance(700);
    await tick();
    expect(flushes).toEqual([A, BIG]);
  });

  it('keeps one update in flight per document and runs the next after it', async () => {
    const { clock, flushes, results, scheduler } = harness();
    scheduler.touch(A);
    clock.advance(300);
    await tick();
    expect(flushes).toEqual([A]);

    scheduler.touch(A);
    clock.advance(300);
    await tick();
    expect(flushes).toEqual([A]); // still waiting for the first update
    expect(scheduler.isScheduled(A)).toBe(true);

    results[0]?.resolve(true);
    await tick();
    expect(flushes).toEqual([A, A]);
    results[1]?.resolve(true);
    await tick();
    expect(scheduler.isScheduled(A)).toBe(false);
  });

  it('settle waits for the update in flight, flushes again and reports that result', async () => {
    const { clock, flushes, results, scheduler } = harness();
    scheduler.touch(A);
    clock.advance(300);
    await tick();
    scheduler.touch(A);
    const settled = scheduler.settle(A);
    expect(clock.pending()).toBe(0); // the debounce timer was dropped
    await tick();
    expect(flushes).toEqual([A]);
    results[0]?.resolve(true);
    await tick();
    expect(flushes).toEqual([A, A]);
    results[1]?.resolve(false);
    await expect(settled).resolves.toBe(false);
  });

  it('cancel drops the pending flush; cancelAll drops every timer', async () => {
    const { clock, flushes, scheduler } = harness();
    scheduler.touch(A);
    scheduler.cancel(A);
    clock.advance(300);
    await tick();
    expect(flushes).toEqual([]);
    expect(scheduler.isScheduled(A)).toBe(false);

    scheduler.touch(A);
    scheduler.touch(BIG);
    scheduler.cancelAll();
    clock.advance(1000);
    await tick();
    expect(flushes).toEqual([]);
    expect(clock.pending()).toBe(0);
  });
});
