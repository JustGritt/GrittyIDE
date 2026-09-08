import { describe, expect, it } from 'vitest';
import { createCoreClient } from './client';
import { COMMANDS } from './commands';
import { createFakeTransport } from './testing/fake-transport';
import type {
  HealthReport,
  Operation,
  SearchCompletion,
  SearchMatch,
  SearchMatchesFoundPayload,
  Workspace,
} from './types';
import { CORE_EVENT_CHANNEL, PROTOCOL_VERSION } from './types';

// PROTO-W3: the `search.query` wire. What is worth a test here is not that a
// command name exists but the shape of the contract around it — one call
// returns an operation and nothing else, matches arrive as batches that a
// consumer folds, a batch that dropped matches says so, and a reload gets the
// operation back without its results.

const health: HealthReport = {
  protocolVersion: PROTOCOL_VERSION,
  coreVersion: '0.1.0',
  os: 'macos',
  arch: 'aarch64',
  debug: true,
  shell: { appVersion: '0.1.0', runtime: 'tauri', runtimeVersion: '2.11.5' },
};

const workspaceId = 'w-1' as Workspace['id'];
const operationId = 'op-1' as Operation['id'];

const running: Operation = {
  id: operationId,
  kind: 'search.query',
  actor: { type: 'USER' },
  workspaceId,
  status: 'RUNNING',
  createdAt: 1_800_000_000_000,
  startedAt: 1_800_000_000_000,
  cancellationRequested: false,
  correlationId: 'corr-1' as Operation['correlationId'],
};

function contentMatch(line: number): SearchMatch {
  return {
    path: 'src/main.rs' as SearchMatch['path'],
    kind: 'content',
    line,
    column: 5,
    snippet: `    // needle ${line}`,
    truncated: false,
    matchStart: 7,
    matchLength: 6,
  };
}

const filenameMatch: SearchMatch = {
  path: 'src/needle.ts' as SearchMatch['path'],
  kind: 'filename',
  snippet: 'src/needle.ts',
  truncated: false,
  matchStart: 4,
  matchLength: 6,
};

function client(overrides: Record<string, () => unknown> = {}) {
  const transport = createFakeTransport();
  transport.handlers.set('core_health', () => health);
  transport.handlers.set('search_query', () => running);
  transport.handlers.set('operations_list', () => [running]);
  transport.handlers.set('operations_cancel', () => ({
    ...running,
    cancellationRequested: true,
  }));
  for (const [command, handler] of Object.entries(overrides)) {
    transport.handlers.set(command, handler);
  }
  return { transport, core: createCoreClient(transport) };
}

function batch(payload: SearchMatchesFoundPayload, seq: number) {
  return {
    name: 'SearchMatchesFound',
    seq,
    workspaceId,
    correlationId: 'corr-1',
    actor: { type: 'USER' },
    payload,
  };
}

describe('search client', () => {
  it('names the command in the table, under the naming rule', () => {
    expect(COMMANDS['search.query']).toBe('search_query');
  });

  it('sends the query and its options and resolves with the operation', async () => {
    const { transport, core } = client();
    const operation = await core.search.query({
      workspaceId,
      query: 'needle',
      options: { caseSensitive: true, scope: 'filenames', includeIgnored: true },
    });

    expect(operation).toEqual(running);
    const call = transport.calls.find((entry) => entry.command !== 'core_health');
    expect(call).toEqual({
      command: 'search_query',
      args: {
        request: {
          workspaceId,
          query: 'needle',
          options: { caseSensitive: true, scope: 'filenames', includeIgnored: true },
        },
      },
    });
  });

  it('takes a query with no options at all', async () => {
    const { transport, core } = client();
    await core.search.query({ workspaceId, query: 'needle' });
    const call = transport.calls.find((entry) => entry.command !== 'core_health');
    expect(call?.args).toEqual({ request: { workspaceId, query: 'needle' } });
  });

  it('rejects with the error envelope when the query is invalid', async () => {
    const { core } = client({
      search_query: () => {
        throw {
          code: 'InvalidInput',
          message: 'query: must not be empty or only whitespace',
          retryable: false,
          details: { field: 'query' },
        };
      },
    });
    await expect(core.search.query({ workspaceId, query: '  ' })).rejects.toMatchObject({
      code: 'InvalidInput',
      details: { field: 'query' },
    });
  });

  it('folds streamed batches into one growing result list', async () => {
    const { transport, core } = client();
    const found: SearchMatch[] = [];
    let dropped = 0;
    core.subscribe('SearchMatchesFound', (event) => {
      if (event.payload.operationId !== operationId) return;
      found.push(...event.payload.matches);
      dropped += event.payload.truncatedMatches;
    });
    await core.search.query({ workspaceId, query: 'needle' });

    transport.emit(
      batch({ operationId, matches: [contentMatch(2), contentMatch(3)], truncatedMatches: 0 }, 1),
    );
    transport.emit(batch({ operationId, matches: [filenameMatch], truncatedMatches: 7 }, 2));

    expect(found).toHaveLength(3);
    expect(found.map((match) => match.line)).toEqual([2, 3, undefined]);
    expect(found[2]?.kind).toBe('filename');
    expect(dropped).toBe(7);
  });

  it('ignores batches belonging to another search', async () => {
    const { transport, core } = client();
    const found: SearchMatch[] = [];
    core.subscribe('SearchMatchesFound', (event) => {
      if (event.payload.operationId !== operationId) return;
      found.push(...event.payload.matches);
    });
    await core.search.query({ workspaceId, query: 'needle' });

    transport.emit(
      batch(
        {
          operationId: 'op-other' as Operation['id'],
          matches: [contentMatch(9)],
          truncatedMatches: 0,
        },
        1,
      ),
    );
    expect(found).toHaveLength(0);
  });

  it('completes with a summary that carries counts and not matches', async () => {
    const { transport, core } = client();
    let completion: SearchCompletion | undefined;
    core.subscribe('OperationCompleted', (event) => {
      completion = event.payload.result as SearchCompletion;
    });
    await core.search.query({ workspaceId, query: 'needle' });

    const summary: SearchCompletion = {
      summary: {
        filesScanned: 120,
        filesMatched: 2,
        directoriesScanned: 14,
        bytesScanned: 40_000,
        matchesFound: 3,
        snippetsTruncated: 1,
        skipped: {
          binary: 2,
          invalidUtf8: 1,
          tooLarge: 0,
          unreadable: 0,
          outsideRoot: 0,
          ignoredDirectories: 1,
        },
        limitReached: false,
        elapsedMs: 12,
      },
      truncatedMatches: 7,
    };
    transport.emit({
      name: 'OperationCompleted',
      seq: 1,
      workspaceId,
      correlationId: 'corr-1',
      actor: { type: 'USER' },
      payload: { operation: { ...running, status: 'COMPLETED' }, result: summary },
    });

    expect(completion?.summary.matchesFound).toBe(3);
    expect(completion?.truncatedMatches).toBe(7);
    expect(JSON.stringify(completion)).not.toContain('needle');
  });

  /**
   * The non-goal, asserted rather than assumed: `Operation`'s snapshot never
   * carried a domain result stream, so a renderer that reloads mid-search gets
   * the operation back `RUNNING` and none of the matches it already showed.
   */
  it('rehydrates a running search after a reload without replaying its matches', async () => {
    const first = client();
    const before: SearchMatch[] = [];
    first.core.subscribe('SearchMatchesFound', (event) => {
      before.push(...event.payload.matches);
    });
    await first.core.search.query({ workspaceId, query: 'needle' });
    first.transport.emit(
      batch({ operationId, matches: [contentMatch(2)], truncatedMatches: 0 }, 1),
    );
    expect(before).toHaveLength(1);
    first.core.dispose();

    // A reload is a new client over a core that kept running.
    const after = client();
    const replayed: SearchMatch[] = [];
    after.core.subscribe('SearchMatchesFound', (event) => {
      replayed.push(...event.payload.matches);
    });
    const operations = await after.core.operations.list();
    expect(operations).toEqual([running]);
    expect(operations[0]?.status).toBe('RUNNING');
    expect(replayed).toHaveLength(0);
  });

  it('is cancelled through operations.cancel, not a command of its own', async () => {
    const { transport, core } = client();
    const operation = await core.search.query({ workspaceId, query: 'needle' });
    const cancelled = await core.operations.cancel(operation.id);
    expect(cancelled.cancellationRequested).toBe(true);
    expect(transport.calls.map((entry) => entry.command)).toContain('operations_cancel');
    expect(Object.keys(COMMANDS)).not.toContain('search.cancel');
  });

  it('listens on the one core event channel', () => {
    const { transport } = client();
    createCoreClient(transport);
    expect(CORE_EVENT_CHANNEL).toBe('gritty://core/event');
  });
});
