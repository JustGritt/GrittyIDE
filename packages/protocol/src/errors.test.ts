import { describe, expect, it } from 'vitest';
import { CoreError, toCoreError } from './errors';
import type { CorrelationId } from './types';

describe('toCoreError', () => {
  it('keeps every envelope field from the core', () => {
    const error = toCoreError({
      code: 'InvalidInput',
      message: 'steps: must be between 1 and 10000',
      details: { field: 'steps', reason: 'must be between 1 and 10000' },
      retryable: false,
      correlationId: 'corr-1',
    });
    expect(error).toBeInstanceOf(CoreError);
    expect(error).toBeInstanceOf(Error);
    expect(error.name).toBe('CoreError');
    expect(error.code).toBe('InvalidInput');
    expect(error.message).toBe('steps: must be between 1 and 10000');
    expect(error.details).toEqual({ field: 'steps', reason: 'must be between 1 and 10000' });
    expect(error.retryable).toBe(false);
    expect(error.correlationId).toBe('corr-1');
    expect(error.toJSON()).toEqual({
      code: 'InvalidInput',
      message: 'steps: must be between 1 and 10000',
      details: { field: 'steps', reason: 'must be between 1 and 10000' },
      retryable: false,
      correlationId: 'corr-1',
    });
  });

  it('omits absent optional fields from toJSON', () => {
    const error = new CoreError({ code: 'NotFound', message: 'gone', retryable: false });
    expect(error.toJSON()).toEqual({ code: 'NotFound', message: 'gone', retryable: false });
    expect('details' in error.toJSON()).toBe(false);
  });

  it('wraps a string rejection (unknown command) as Internal with the raw value', () => {
    const error = toCoreError('Command nope not found');
    expect(error.code).toBe('Internal');
    expect(error.message).toBe('Command nope not found');
    expect(error.retryable).toBe(false);
    expect(error.details).toEqual({ raw: 'Command nope not found' });
  });

  it('wraps thrown Errors and unknown objects as Internal', () => {
    const fromError = toCoreError(new TypeError('boom'));
    expect(fromError.code).toBe('Internal');
    expect(fromError.message).toBe('boom');
    expect(fromError.details).toEqual({ raw: 'TypeError: boom' });

    const fromObject = toCoreError({ code: 'NotACode', message: 'x', retryable: true });
    expect(fromObject.code).toBe('Internal');
    expect(fromObject.details).toEqual({
      raw: { code: 'NotACode', message: 'x', retryable: true },
    });

    expect(toCoreError(undefined).details).toEqual({ raw: null });
  });

  it('passes CoreError instances through untouched', () => {
    const original = new CoreError({
      code: 'Cancelled',
      message: 'stop',
      retryable: false,
      correlationId: 'c' as CorrelationId,
    });
    expect(toCoreError(original)).toBe(original);
  });
});
