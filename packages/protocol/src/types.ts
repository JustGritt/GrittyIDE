// Public protocol types. Everything under ./generated is written by
// `cargo test -p gritty-core` (see crates/gritty-core/src/protocol/export.rs).

import type { CoreEvent } from './generated/CoreEvent';
import type { EventMeta } from './generated/EventMeta';

export type { Actor } from './generated/Actor';
export type { AgentRunId } from './generated/AgentRunId';
export type { CancelOperationRequest } from './generated/CancelOperationRequest';
export type { CandidatesRequest } from './generated/CandidatesRequest';
export type { CloseDocumentRequest } from './generated/CloseDocumentRequest';
export type { CloseWorkspaceRequest } from './generated/CloseWorkspaceRequest';
export type { ContentHash } from './generated/ContentHash';
export type { CoreError as CoreErrorShape } from './generated/CoreError';
export type { CoreErrorCode } from './generated/CoreErrorCode';
export type { CoreEvent } from './generated/CoreEvent';
export type { CorrelationId } from './generated/CorrelationId';
export type { DirEntry } from './generated/DirEntry';
export type { DirEntryKind } from './generated/DirEntryKind';
export type { DirectoryPage } from './generated/DirectoryPage';
export type { DiscardSnapshotRequest } from './generated/DiscardSnapshotRequest';
export type { Document } from './generated/Document';
export type { DocumentChangeCause } from './generated/DocumentChangeCause';
export type { DocumentChangedPayload } from './generated/DocumentChangedPayload';
export type { DocumentClosedPayload } from './generated/DocumentClosedPayload';
export type { DocumentClosedReason } from './generated/DocumentClosedReason';
export type { DocumentContent } from './generated/DocumentContent';
export type { DocumentEncoding } from './generated/DocumentEncoding';
export type { DocumentExternalChangePayload } from './generated/DocumentExternalChangePayload';
export type { DocumentId } from './generated/DocumentId';
export type { DocumentOpenedPayload } from './generated/DocumentOpenedPayload';
export type { DocumentSavedPayload } from './generated/DocumentSavedPayload';
export type { DocumentStatusRequest } from './generated/DocumentStatusRequest';
export type { EventMeta } from './generated/EventMeta';
export type { ExternalModificationState } from './generated/ExternalModificationState';
export type { HealthReport } from './generated/HealthReport';
export type { ListDirectoryRequest } from './generated/ListDirectoryRequest';
export type { ListDirectoryResponse } from './generated/ListDirectoryResponse';
export type { OpenDocumentRequest } from './generated/OpenDocumentRequest';
export type { OpenExternalRequest } from './generated/OpenExternalRequest';
export type { OpenWorkspaceRequest } from './generated/OpenWorkspaceRequest';
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
export type { PersistenceStatus } from './generated/PersistenceStatus';
export type { PersistenceStatusPayload } from './generated/PersistenceStatusPayload';
export type { RecoverSnapshotRequest } from './generated/RecoverSnapshotRequest';
export type { RecoveryCandidate } from './generated/RecoveryCandidate';
export type { RecoveryState } from './generated/RecoveryState';
export type { ReloadDocumentRequest } from './generated/ReloadDocumentRequest';
export type { SaveDocumentRequest } from './generated/SaveDocumentRequest';
export type { SessionNotice } from './generated/SessionNotice';
export type { SessionNoticeKind } from './generated/SessionNoticeKind';
export type { SessionRestoreResponse } from './generated/SessionRestoreResponse';
export type { SessionTab } from './generated/SessionTab';
export type { ShellInfo } from './generated/ShellInfo';
export type { StartDemoRequest } from './generated/StartDemoRequest';
export type { SymlinkInfo } from './generated/SymlinkInfo';
export type { JsonValue } from './generated/serde_json/JsonValue';
export type { TextPosition } from './generated/TextPosition';
export type { Timestamp } from './generated/Timestamp';
export type { UpdateDocumentRequest } from './generated/UpdateDocumentRequest';
export type { UpdateSessionRequest } from './generated/UpdateSessionRequest';
export { CORE_EVENT_CHANNEL, PROTOCOL_VERSION } from './generated/version';
export type { Workspace } from './generated/Workspace';
export type { WorkspaceClosedPayload } from './generated/WorkspaceClosedPayload';
export type { WorkspaceClosedReason } from './generated/WorkspaceClosedReason';
export type { WorkspaceId } from './generated/WorkspaceId';
export type { WorkspaceOpenedPayload } from './generated/WorkspaceOpenedPayload';
export type { WorkspacePath } from './generated/WorkspacePath';

/**
 * What arrives on the event channel: `{ name, seq, workspaceId?, correlationId?, actor, payload }`.
 * Rust flattens `EventMeta` and `CoreEvent` into one object; the intersection mirrors that.
 */
export type EventEnvelope = EventMeta & CoreEvent;

export type EventName = CoreEvent['name'];

/** The envelope narrowed to one event family. */
export type EventOf<N extends EventName> = EventMeta & Extract<CoreEvent, { name: N }>;
