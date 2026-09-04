import type { TerminalId } from '@gritty/protocol';
import { describe, expect, it, vi } from 'vitest';
import { createOutputBus, MAX_BUFFERED_CHUNKS, type OutputChunk } from './output-bus';

const id = 'term-1' as TerminalId;
const other = 'term-2' as TerminalId;

function chunk(data: string, truncatedBytes = 0): OutputChunk {
  return { data, truncatedBytes };
}

describe('terminal output bus', () => {
  it('streams to the subscriber without touching React state', () => {
    const bus = createOutputBus();
    const listener = vi.fn();
    bus.subscribe(id, listener);

    bus.deliver(id, chunk('hello'));
    bus.deliver(id, chunk('world'));

    expect(listener.mock.calls.map(([c]) => (c as OutputChunk).data)).toEqual(['hello', 'world']);
    expect(bus.buffered(id)).toBe(0);
  });

  it('replays output that arrived before the view mounted', () => {
    // The shell prints its prompt between `terminal.spawn` resolving and the
    // xterm instance existing; that output must not be lost.
    const bus = createOutputBus();
    bus.deliver(id, chunk('$ '));
    expect(bus.buffered(id)).toBe(1);

    const listener = vi.fn();
    bus.subscribe(id, listener);
    expect(listener).toHaveBeenCalledWith(chunk('$ '));
    expect(bus.buffered(id)).toBe(0);
  });

  it('keeps sessions apart', () => {
    const bus = createOutputBus();
    const first = vi.fn();
    const second = vi.fn();
    bus.subscribe(id, first);
    bus.subscribe(other, second);

    bus.deliver(id, chunk('a'));
    bus.deliver(other, chunk('b'));

    expect(first).toHaveBeenCalledTimes(1);
    expect(first).toHaveBeenCalledWith(chunk('a'));
    expect(second).toHaveBeenCalledWith(chunk('b'));
  });

  it('stops delivering after unsubscribe and buffers again', () => {
    const bus = createOutputBus();
    const listener = vi.fn();
    const unsubscribe = bus.subscribe(id, listener);
    unsubscribe();

    bus.deliver(id, chunk('later'));
    expect(listener).not.toHaveBeenCalled();
    expect(bus.buffered(id)).toBe(1);
  });

  it('a second subscription replaces the first rather than splitting the stream', () => {
    const bus = createOutputBus();
    const first = vi.fn();
    const second = vi.fn();
    const unsubscribeFirst = bus.subscribe(id, first);
    bus.subscribe(id, second);

    bus.deliver(id, chunk('x'));
    expect(first).not.toHaveBeenCalled();
    expect(second).toHaveBeenCalledTimes(1);

    // The stale unsubscribe must not detach the live listener.
    unsubscribeFirst();
    bus.deliver(id, chunk('y'));
    expect(second).toHaveBeenCalledTimes(2);
  });

  it('bounds the buffer and folds what it dropped into the truncation count', () => {
    const bus = createOutputBus();
    for (let index = 0; index < MAX_BUFFERED_CHUNKS + 2; index += 1) {
      bus.deliver(id, chunk('ab'));
    }
    expect(bus.buffered(id)).toBe(MAX_BUFFERED_CHUNKS);

    const listener = vi.fn();
    bus.subscribe(id, listener);
    const firstChunk = listener.mock.calls[0]?.[0] as OutputChunk | undefined;
    expect(firstChunk?.truncatedBytes).toBe(4);
  });

  it('forget drops the buffer and the listener', () => {
    const bus = createOutputBus();
    const listener = vi.fn();
    bus.subscribe(id, listener);
    bus.deliver(id, chunk('a'));
    bus.forget(id);

    bus.deliver(id, chunk('b'));
    expect(listener).toHaveBeenCalledTimes(1);
    expect(bus.buffered(id)).toBe(1);
  });
});
