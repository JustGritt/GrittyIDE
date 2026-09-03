// Public protocol types. Everything under ./generated is written by
// `cargo test -p gritty-core` (see crates/gritty-core/src/protocol/export.rs).

import type { CoreEvent } from './generated/CoreEvent';
import type { EventMeta } from './generated/EventMeta';

export type { Actor } from './generated/Actor';
export type { AgentRunId } from './generated/AgentRunId';
export type { CancelOperationRequest } from './generated/CancelOperationRequest';
export type { CoreError as CoreErrorShape } from './generated/CoreError';
export type { CoreErrorCode } from './generated/CoreErrorCode';
export type { CoreEvent } from './generated/CoreEvent';
export type { CorrelationId } from './generated/CorrelationId';
export type { EventMeta } from './generated/EventMeta';
export type { HealthReport } from './generated/HealthReport';
export type { Operation } from './generated/Operation';
export type { OperationCancelledPayload } from './generated/OperationCancelledPayload';
export type { OperationCompletedPayload } from './generated/OperationCompletedPayload';
export type { OperationFailedPayload } from './generated/OperationFailedPayload';
export type { OperationId } from './generated/OperationId';
export type { OperationKind } from './generated/OperationKind';
export type { OperationProgress } from './generated/OperationProgress';
export type { OperationProgressPayload } from './generated/OperationProgressPayload';
export type { OperationStartedPayload } from './generated/OperationStartedPayload';
export type { OperationStatus } from './generated/OperationStatus';
export type { ShellInfo } from './generated/ShellInfo';
export type { StartDemoRequest } from './generated/StartDemoRequest';
export type { JsonValue } from './generated/serde_json/JsonValue';
export type { Timestamp } from './generated/Timestamp';
export { CORE_EVENT_CHANNEL, PROTOCOL_VERSION } from './generated/version';
export type { WorkspaceId } from './generated/WorkspaceId';

/**
 * What arrives on the event channel: `{ name, seq, workspaceId?, correlationId?, actor, payload }`.
 * Rust flattens `EventMeta` and `CoreEvent` into one object; the intersection mirrors that.
 */
export type EventEnvelope = EventMeta & CoreEvent;

export type EventName = CoreEvent['name'];

/** The envelope narrowed to one event family. */
export type EventOf<N extends EventName> = EventMeta & Extract<CoreEvent, { name: N }>;
