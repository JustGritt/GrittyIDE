import type {
  AgentNoticeLevel,
  AgentTimeline,
  AgentTimelineEntry,
  WorkspacePath,
} from '@gritty/protocol';

/**
 * The selected session's timeline as the rail draws it (T232).
 *
 * Pure, so the whole of what the feed says — the pairing, the gaps, the files
 * touched, the empty states — is tested without React, a core or a DOM. The
 * component below this only turns rows into elements.
 *
 * Two rules hold everything here together:
 *
 * 1. **Every string is text** (§7, hard rule 13). The core bounded each field
 *    before it crossed the protocol; this file's only job is to not undo that,
 *    so nothing here parses, resolves, concatenates or interprets a string. The
 *    one field that is ever *used* rather than shown is `openPath`, and it is
 *    used exactly as it arrived because the core is what decided it may be
 *    opened at all.
 * 2. **A loss the core reported is drawn, never repaired** (§5). `dropped` and
 *    `restarted` become explicit rows. Splicing the entries either side of a
 *    gap into one conversation would be the UI inventing continuity the core
 *    deliberately refused to claim.
 */

/** Which class a row draws with, and how heavily the reader should weigh it. */
export type TimelineTone =
  | 'gap'
  | 'prompt'
  | 'assistant'
  | 'thinking'
  | 'tool'
  | 'result'
  | 'file'
  | 'notice'
  | 'permission'
  | 'unknown';

/** How a tool call ended, once its result has arrived. */
export interface ToolOutcome {
  readonly ok: boolean;
  /** The core's bounded preview of the result. Display only. */
  readonly detail: string | null;
}

/**
 * One drawable line of the feed.
 *
 * Flat rather than a discriminated union because every row draws with the same
 * three slots — a label, a body, and a quieter second line — and the renderer
 * stays a single element instead of ten near-identical ones. The fields that
 * only some kinds carry are `null` everywhere else, and each says which kind
 * fills it.
 */
export interface TimelineRow {
  /** Stable across renders: the entry's `seq`, or a fixed id for a gap. */
  readonly key: string;
  readonly tone: TimelineTone;
  /** The short word in front of the row. Never empty. */
  readonly label: string;
  /** The row's body. Always plain text, always rendered as a text node. */
  readonly text: string;
  /** A quieter second line: a tool's bounded input, a result's detail. */
  readonly detail: string | null;
  /** When the record was written, when it said so. `null` is not "now" (§5). */
  readonly at: number | null;
  /** Subagent work: marked and subordinate, never hidden (§4). */
  readonly sidechain: boolean;
  /** Tool rows: how the call ended, or `null` while it is still outstanding. */
  readonly outcome: ToolOutcome | null;
  /**
   * File rows inside the open workspace: what `documents.open` takes (§10).
   * `null` for a file outside it — the core sent no path, so there is nothing
   * to open and the renderer has nothing to be restrained about.
   */
  readonly openPath: WorkspacePath | null;
  /** Notice rows: how much it matters. */
  readonly level: AgentNoticeLevel | null;
}

/**
 * How many entries the view keeps, mirroring the core's ring
 * (`MAX_TIMELINE_ENTRIES` in `crates/gritty-core/src/agent_session/timeline.rs`).
 *
 * Not a second policy: the core drops from the oldest end at exactly this
 * count and *reports* the loss, so a client that mirrors the ring trims the
 * same entries the core already counted. It is here so that a client which
 * somehow ran ahead of the core still cannot grow without bound.
 */
export const MAX_TIMELINE_ENTRIES = 500;

/**
 * How much of an entry from a newer core is shown.
 *
 * The core bounds every field it defines, but this build cannot know the shape
 * of a kind it has never seen, so the serialized form is clipped here rather
 * than trusted to be small.
 */
const MAX_UNKNOWN_ENTRY = 2000;

const NOTICE_LABELS: Record<AgentNoticeLevel, string> = {
  INFO: 'Note',
  WARNING: 'Warning',
  ERROR: 'Error',
};

function blank(): Pick<TimelineRow, 'detail' | 'outcome' | 'openPath' | 'level'> {
  return { detail: null, outcome: null, openPath: null, level: null };
}

/**
 * What one entry says, before pairing.
 *
 * Total over `AgentTimelineKind` by exhaustion *and* by fallthrough: the
 * `switch` covers every variant this build knows, and anything else — a kind a
 * newer core added — becomes an `unknown` row showing what arrived. An entry
 * never vanishes for not being understood, which is the same allowance
 * `agent-status.ts` makes for an unrecognised activity.
 */
function describe(
  entry: AgentTimelineEntry,
  outcome: ToolOutcome | null,
): Omit<TimelineRow, 'key' | 'at' | 'sidechain'> {
  const kind = entry.kind;
  switch (kind.type) {
    case 'USER_PROMPT':
      return { tone: 'prompt', label: 'You', text: kind.text, ...blank() };
    case 'ASSISTANT_TEXT':
      return { tone: 'assistant', label: 'Agent', text: kind.text, ...blank() };
    case 'THINKING':
      return { tone: 'thinking', label: 'Thinking', text: kind.text, ...blank() };
    case 'TOOL_CALL':
      return {
        tone: 'tool',
        label: 'Tool',
        text: kind.name,
        ...blank(),
        detail: kind.detail,
        // `null` reads as *still running* in the renderer, which is the honest
        // state of a call whose result has not been written yet.
        outcome,
      };
    case 'TOOL_RESULT':
      // Only reached for a result that could not be attached to a call: one
      // with no `toolUseId`, or one whose call has already fallen out of the
      // ring. Shown rather than dropped, because a result with no visible call
      // is still something that happened.
      return {
        tone: 'result',
        label: 'Result',
        text: kind.ok ? 'Succeeded' : 'Failed',
        ...blank(),
        detail: kind.detail,
        outcome: { ok: kind.ok, detail: kind.detail },
      };
    case 'FILE_TOUCHED':
      return {
        tone: 'file',
        label: 'Wrote',
        text: kind.display,
        ...blank(),
        openPath: kind.path,
      };
    case 'NOTICE':
      return {
        tone: 'notice',
        // Widened for the lookup the way `providerLabel` is: the protocol is
        // additive, so a newer core can send a level this build's union does
        // not contain.
        label: (NOTICE_LABELS as Record<string, string | undefined>)[kind.level] ?? 'Note',
        text: kind.text,
        ...blank(),
        level: kind.level,
      };
    case 'PERMISSION_PENDING':
      return {
        tone: 'permission',
        label: 'Waiting on you',
        text: kind.tool,
        ...blank(),
        detail: kind.detail,
      };
    default:
      return {
        tone: 'unknown',
        label: 'Unrecognised',
        // Shown as it arrived rather than mapped onto a kind this build would
        // be guessing at. It is a string in a text node like every other field
        // here, so an unknown shape costs legibility and nothing else.
        text: serialize(kind),
        ...blank(),
      };
  }
}

/** A kind this build does not know, clipped to something a row can hold. */
function serialize(kind: unknown): string {
  let text: string;
  try {
    text = JSON.stringify(kind) ?? String(kind);
  } catch {
    text = String(kind);
  }
  return text.length > MAX_UNKNOWN_ENTRY ? `${text.slice(0, MAX_UNKNOWN_ENTRY)}…` : text;
}

/**
 * The rows the feed draws, oldest first (§1).
 *
 * The gaps come first because both describe the *head* of the timeline: the
 * entries that are missing are the oldest ones. They are two separate rows
 * rather than one summary, because their order relative to each other is not
 * knowable from what the core sends and merging them would state something
 * neither flag says.
 */
export function timelineRows(timeline: AgentTimeline): readonly TimelineRow[] {
  const rows: TimelineRow[] = [];

  if (timeline.restarted) {
    rows.push(
      gap(
        'gap:restarted',
        'This view was reset: the transcript was replaced, truncated, or grew faster than it could be read. Everything before this point is gone.',
      ),
    );
  }
  if (timeline.dropped > 0) {
    rows.push(
      gap(
        'gap:dropped',
        `${timeline.dropped} earlier ${
          timeline.dropped === 1 ? 'entry' : 'entries'
        } dropped: the timeline outgrew the number the core keeps.`,
      ),
    );
  }

  // A result attaches to the call it belongs to rather than floating below it
  // (§3). Both passes are over the same bounded ring, so this stays linear.
  const calls = new Set<string>();
  const results = new Map<string, ToolOutcome>();
  for (const entry of timeline.entries) {
    if (entry.kind.type === 'TOOL_CALL' && entry.kind.toolUseId !== null) {
      calls.add(entry.kind.toolUseId);
    } else if (entry.kind.type === 'TOOL_RESULT' && entry.kind.toolUseId !== null) {
      // Last one wins: a repeated id is a transcript oddity, and the newer
      // record is the one that describes how the call actually ended.
      results.set(entry.kind.toolUseId, { ok: entry.kind.ok, detail: entry.kind.detail });
    }
  }

  for (const entry of timeline.entries) {
    const kind = entry.kind;
    // Attached above; drawing it again would show the same result twice.
    if (kind.type === 'TOOL_RESULT' && kind.toolUseId !== null && calls.has(kind.toolUseId)) {
      continue;
    }
    const outcome =
      kind.type === 'TOOL_CALL' && kind.toolUseId !== null
        ? (results.get(kind.toolUseId) ?? null)
        : null;
    rows.push({
      key: `entry:${entry.seq}`,
      at: entry.at ?? null,
      sidechain: entry.sidechain,
      ...describe(entry, outcome),
    });
  }

  return rows;
}

function gap(key: string, text: string): TimelineRow {
  return {
    key,
    tone: 'gap',
    label: 'Gap',
    text,
    at: null,
    sidechain: false,
    ...blank(),
  };
}

/** One file the session wrote, as the files-touched list holds it (§9). */
export interface TouchedFile {
  /** Stable per file: the path when there is one, the display string when not. */
  readonly key: string;
  /** Always safe to show, never to resolve. */
  readonly display: string;
  /**
   * What opens it, or `null` for a file outside the open workspace (§10).
   * The core sends no path for those, so this is enforced by the protocol
   * rather than by the renderer choosing not to.
   */
  readonly openPath: WorkspacePath | null;
  /** When it was last touched, when the record said. */
  readonly at: number | null;
  /** The entry it was last seen at; what the list is ordered by. */
  readonly seq: number;
}

/**
 * The files the session touched: deduplicated, most recent first (§9).
 *
 * Ordered by `seq` rather than by `at`, because `at` is absent whenever the
 * record carried no timestamp and sorting by a missing value would reorder the
 * list around nothing. `seq` is the order the core guarantees.
 *
 * Bounded by the ring the entries came from: a file that fell out of the
 * timeline falls out of this list with it. That is history the core no longer
 * holds, not a directory the view is failing to refresh.
 */
export function touchedFiles(entries: readonly AgentTimelineEntry[]): readonly TouchedFile[] {
  const seen = new Map<string, TouchedFile>();
  for (const entry of entries) {
    if (entry.kind.type !== 'FILE_TOUCHED') continue;
    const { path, display } = entry.kind;
    // Prefixed so a display string can never collide with a workspace path.
    const key = path === null ? `outside:${display}` : `in:${path}`;
    // Set unconditionally: entries arrive oldest first, so the last write is
    // the most recent touch, which is the time the row reports.
    seen.set(key, { key, display, openPath: path, at: entry.at ?? null, seq: entry.seq });
  }
  return [...seen.values()].sort((a, b) => b.seq - a.seq);
}

/**
 * How close to the bottom still counts as being at the bottom.
 *
 * Sub-pixel scroll positions and a row that grew by a hair both leave
 * `scrollTop` a fraction short of the end, and a pin that only survives an
 * exact match would unstick itself for no reason the user did anything to
 * cause.
 */
export const PIN_THRESHOLD_PX = 24;

/** What the feed measures to decide whether new entries may scroll it (§1). */
export interface ScrollExtent {
  readonly scrollTop: number;
  readonly scrollHeight: number;
  readonly clientHeight: number;
}

/**
 * Whether the feed should follow new entries down.
 *
 * True while the user is at the bottom, and — importantly — while there is
 * nothing to scroll at all, so a feed that has just started stays pinned. False
 * the moment they scroll up to read something, which is what stops the next
 * batch yanking the view out from under them.
 */
export function isPinnedToBottom(
  extent: ScrollExtent,
  threshold: number = PIN_THRESHOLD_PX,
): boolean {
  return extent.scrollHeight - extent.scrollTop - extent.clientHeight <= threshold;
}
