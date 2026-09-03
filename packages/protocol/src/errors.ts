import type { CoreErrorCode, CoreErrorShape, CorrelationId, JsonValue } from './types';

const CODES: ReadonlySet<string> = new Set<CoreErrorCode>([
  'InvalidInput',
  'NotFound',
  'PermissionDenied',
  'Conflict',
  'Cancelled',
  'Timeout',
  'Io',
  'Internal',
  'ProtocolMismatch',
]);

/**
 * The error every client call rejects with. Carries the core's envelope fields
 * verbatim; anything that is not an envelope (a transport failure, an unknown
 * command, a thrown `Error`) is wrapped as `Internal` with the raw value in
 * `details.raw` so nothing is ever swallowed.
 */
export class CoreError extends Error implements CoreErrorShape {
  readonly code: CoreErrorCode;
  readonly details?: JsonValue;
  readonly retryable: boolean;
  readonly correlationId?: CorrelationId;

  constructor(shape: CoreErrorShape) {
    super(shape.message);
    this.name = 'CoreError';
    this.code = shape.code;
    this.retryable = shape.retryable;
    if (shape.details !== undefined) this.details = shape.details;
    if (shape.correlationId !== undefined) this.correlationId = shape.correlationId;
  }

  /** The envelope as a plain object (what the core would have sent). */
  toJSON(): CoreErrorShape {
    const shape: CoreErrorShape = {
      code: this.code,
      message: this.message,
      retryable: this.retryable,
    };
    if (this.details !== undefined) shape.details = this.details;
    if (this.correlationId !== undefined) shape.correlationId = this.correlationId;
    return shape;
  }

  static isShape(value: unknown): value is CoreErrorShape {
    if (typeof value !== 'object' || value === null) return false;
    const candidate = value as Record<string, unknown>;
    return (
      typeof candidate.code === 'string' &&
      CODES.has(candidate.code) &&
      typeof candidate.message === 'string' &&
      typeof candidate.retryable === 'boolean'
    );
  }
}

/** Normalise whatever a transport rejected with into a `CoreError`. */
export function toCoreError(raw: unknown): CoreError {
  if (raw instanceof CoreError) return raw;
  if (CoreError.isShape(raw)) return new CoreError(raw);
  if (raw instanceof Error) {
    return new CoreError({
      code: 'Internal',
      message: raw.message,
      retryable: false,
      details: { raw: `${raw.name}: ${raw.message}` },
    });
  }
  const message = typeof raw === 'string' ? raw : 'unexpected error from the core transport';
  return new CoreError({
    code: 'Internal',
    message,
    retryable: false,
    details: { raw: toJsonValue(raw) },
  });
}

function toJsonValue(value: unknown): JsonValue {
  if (value === undefined) return null;
  try {
    return JSON.parse(JSON.stringify(value)) as JsonValue;
  } catch {
    return String(value);
  }
}
