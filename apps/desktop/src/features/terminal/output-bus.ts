import type { TerminalId } from '@gritty/protocol';

/**
 * Routes terminal output straight from the event stream into an xterm
 * instance, deliberately bypassing React state.
 *
 * `TerminalOutput` arrives up to ~60 times a second per session. Putting that
 * through `useState` would re-render the tree at the same rate and undo the
 * batching the core does; xterm owns its own DOM and only needs the bytes. So
 * React state holds session *lifecycle* (which terminals exist, which ended)
 * and the bus carries the bytes.
 *
 * Output can arrive before the view has mounted and subscribed: `terminal.spawn`
 * resolves, React renders, and the shell has already printed its prompt by
 * then. A small per-session buffer holds that first output until a listener
 * appears, so the prompt is never lost. The buffer is bounded like everything
 * else in this path: a session nobody ever subscribes to cannot grow without
 * limit.
 */
export interface OutputChunk {
  readonly data: string;
  /** Bytes the core dropped before this chunk because it fell behind. */
  readonly truncatedBytes: number;
}

export type OutputListener = (chunk: OutputChunk) => void;

/** Chunks held per session before its first listener subscribes. */
export const MAX_BUFFERED_CHUNKS = 256;

export interface OutputBus {
  /** Deliver a chunk, buffering it when nothing is listening yet. */
  deliver(id: TerminalId, chunk: OutputChunk): void;
  /** Listen for one session; replays whatever was buffered, then streams. */
  subscribe(id: TerminalId, listener: OutputListener): () => void;
  /** Drop everything held for a session that is gone. */
  forget(id: TerminalId): void;
  /** Chunks currently buffered for a session (tests and diagnostics). */
  buffered(id: TerminalId): number;
}

export function createOutputBus(): OutputBus {
  const listeners = new Map<TerminalId, OutputListener>();
  const buffers = new Map<TerminalId, OutputChunk[]>();

  return {
    deliver(id, chunk) {
      const listener = listeners.get(id);
      if (listener) {
        listener(chunk);
        return;
      }
      const buffered = buffers.get(id) ?? [];
      buffered.push(chunk);
      // Drop the oldest, mirroring the core: the newest output is the part
      // worth keeping, and the count of dropped bytes travels with it.
      if (buffered.length > MAX_BUFFERED_CHUNKS) {
        const dropped = buffered.shift();
        const next = buffered[0];
        if (dropped && next) {
          // What was dropped is carried into the oldest surviving chunk, so
          // the count reaches the view even though the bytes did not.
          buffered[0] = {
            data: next.data,
            truncatedBytes: next.truncatedBytes + dropped.data.length + dropped.truncatedBytes,
          };
        }
      }
      buffers.set(id, buffered);
    },

    subscribe(id, listener) {
      // One view per session; a second subscription replaces the first rather
      // than silently splitting the stream between two xterm instances.
      listeners.set(id, listener);
      const buffered = buffers.get(id);
      if (buffered) {
        buffers.delete(id);
        for (const chunk of buffered) listener(chunk);
      }
      return () => {
        if (listeners.get(id) === listener) listeners.delete(id);
      };
    },

    forget(id) {
      listeners.delete(id);
      buffers.delete(id);
    },

    buffered(id) {
      return buffers.get(id)?.length ?? 0;
    },
  };
}
