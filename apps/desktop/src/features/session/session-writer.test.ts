import type { UpdateSessionRequest, WorkspaceId } from '@gritty/protocol';
import { describe, expect, it } from 'vitest';
import { SETTINGS_DELAY_MS, SessionWriter, TABS_DELAY_MS, VIEW_DELAY_MS } from './session-writer';

interface Timer {
  callback: () => void;
  at: number;
}

/** Deterministic timers driven by an explicit clock. */
function fakeClock() {
  let now = 0;
  const timers = new Map<number, Timer>();
  let next = 1;
  return {
    now: () => now,
    timers: {
      set(callback: () => void, ms: number) {
        const id = next++;
        timers.set(id, { callback, at: now + ms });
        return id;
      },
      clear(handle: unknown) {
        timers.delete(handle as number);
      },
    },
    async advance(ms: number) {
      const target = now + ms;
      for (;;) {
        const due = [...timers.entries()]
          .filter(([, timer]) => timer.at <= target)
          .sort((a, b) => a[1].at - b[1].at)[0];
        if (!due) break;
        now = due[1].at;
        timers.delete(due[0]);
        due[1].callback();
        await Promise.resolve();
        await Promise.resolve();
      }
      now = target;
    },
    pending: () => timers.size,
  };
}

const ws = 'ws-1' as WorkspaceId;

function setup(snapshot: () => UpdateSessionRequest) {
  const clock = fakeClock();
  const sent: UpdateSessionRequest[] = [];
  const errors: unknown[] = [];
  let fail = false;
  const writer = new SessionWriter(
    {
      snapshot,
      send: async (request) => {
        if (fail) throw new Error('core down');
        sent.push(request);
      },
      onError: (error) => errors.push(error),
      timers: clock.timers,
    },
    clock.now,
  );
  return {
    clock,
    sent,
    errors,
    writer,
    setFail: (value: boolean) => {
      fail = value;
    },
  };
}

const full = (): UpdateSessionRequest => ({
  workspaceId: ws,
  tabs: [{ path: 'a.md', active: true }],
  preferences: { version: 1, fontSize: 14 },
  layout: { version: 1 },
});

describe('SessionWriter', () => {
  it('does nothing until enabled', async () => {
    const { clock, sent, writer } = setup(full);
    writer.schedule('tabs', TABS_DELAY_MS);
    await clock.advance(10_000);
    expect(sent).toEqual([]);
    expect(clock.pending()).toBe(0);
  });

  it('sends only the scheduled fields, after the earliest pending delay', async () => {
    const { clock, sent, writer } = setup(full);
    writer.enable();
    writer.schedule('tabs', VIEW_DELAY_MS);
    await clock.advance(500);
    expect(sent).toEqual([]);
    writer.schedule('preferences', SETTINGS_DELAY_MS);
    await clock.advance(499);
    expect(sent).toEqual([]);
    await clock.advance(1);
    expect(sent).toEqual([
      {
        workspaceId: ws,
        tabs: [{ path: 'a.md', active: true }],
        preferences: { version: 1, fontSize: 14 },
      },
    ]);
    expect(clock.pending()).toBe(0);
  });

  it('a shorter delay pulls a pending write forward, a longer one does not push it back', async () => {
    const { clock, sent, writer } = setup(full);
    writer.enable();
    writer.schedule('tabs', TABS_DELAY_MS);
    writer.schedule('layout', SETTINGS_DELAY_MS);
    await clock.advance(TABS_DELAY_MS);
    expect(sent).toHaveLength(1);
    expect(sent[0]).toMatchObject({ tabs: expect.any(Array), layout: { version: 1 } });
    writer.schedule('layout', SETTINGS_DELAY_MS);
    writer.schedule('tabs', TABS_DELAY_MS);
    await clock.advance(TABS_DELAY_MS);
    expect(sent).toHaveLength(2);
  });

  it('skips tabs without a workspace and skips empty requests', async () => {
    const { clock, sent, writer } = setup(() => ({ tabs: [] }));
    writer.enable();
    writer.schedule('tabs', TABS_DELAY_MS);
    writer.schedule('layout', TABS_DELAY_MS);
    await clock.advance(TABS_DELAY_MS);
    expect(sent).toEqual([]);
  });

  it('flush sends what is pending now and serializes with the request in flight', async () => {
    const { clock, sent, writer } = setup(full);
    writer.enable();
    writer.schedule('layout', SETTINGS_DELAY_MS);
    await writer.flush();
    expect(sent).toEqual([{ layout: { version: 1 } }]);
    expect(clock.pending()).toBe(0);
    await writer.flush();
    expect(sent).toHaveLength(1);
  });

  it('reports send failures through onError and keeps working', async () => {
    const { clock, sent, errors, writer, setFail } = setup(full);
    writer.enable();
    setFail(true);
    writer.schedule('layout', SETTINGS_DELAY_MS);
    await clock.advance(SETTINGS_DELAY_MS);
    expect(sent).toEqual([]);
    expect(errors).toHaveLength(1);
    setFail(false);
    writer.schedule('layout', SETTINGS_DELAY_MS);
    await clock.advance(SETTINGS_DELAY_MS);
    expect(sent).toHaveLength(1);
  });

  it('disable drops pending work', async () => {
    const { clock, sent, writer } = setup(full);
    writer.enable();
    writer.schedule('tabs', TABS_DELAY_MS);
    writer.disable();
    await clock.advance(TABS_DELAY_MS);
    expect(sent).toEqual([]);
    expect(writer.enabled).toBe(false);
  });
});
