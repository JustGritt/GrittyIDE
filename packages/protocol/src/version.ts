import { CoreError } from './errors';
import { PROTOCOL_VERSION } from './generated/version';

/** Major component of a `MAJOR.MINOR.PATCH` string, if parseable. */
export function majorVersion(version: string): number | undefined {
  const [major] = version.split('.');
  if (major === undefined || !/^\d+$/.test(major)) return undefined;
  return Number(major);
}

/**
 * The client refuses to talk to a core whose MAJOR differs from its own.
 * Minor and patch differences are additive by contract and accepted.
 */
export function assertCompatible(coreVersion: string, clientVersion = PROTOCOL_VERSION): void {
  const core = majorVersion(coreVersion);
  const client = majorVersion(clientVersion);
  if (core !== undefined && client !== undefined && core === client) return;
  throw new CoreError({
    code: 'ProtocolMismatch',
    message: `core protocol ${coreVersion} is incompatible with client protocol ${clientVersion} (major versions must match)`,
    retryable: false,
    details: { coreVersion, clientVersion },
  });
}
