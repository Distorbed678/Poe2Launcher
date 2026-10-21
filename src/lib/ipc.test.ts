import { beforeEach, describe, expect, it, vi } from 'vitest';

const invoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invoke(...a) }));

import { call, errorText, ipc, IpcError } from './ipc';

beforeEach(() => {
  invoke.mockReset();
});

describe('ipc', () => {
  it('passes camelCase arguments to the command', async () => {
    invoke.mockResolvedValue(['core']);
    await ipc.modEnable('my-mod', true);
    expect(invoke).toHaveBeenCalledWith('mod_enable', { id: 'my-mod', overrideCompat: true });
    invoke.mockResolvedValue(undefined);
    await ipc.play('p1');
    expect(invoke).toHaveBeenLastCalledWith('play', { profileId: 'p1' });
  });

  it('turns rejections into IpcError with readable text', async () => {
    // The backend rejects with a plain string. (A synchronous throw: vitest reports the
    // derived promise of a rejecting mock as unhandled.)
    invoke.mockImplementation(() => {
      throw 'wrong account name or password';
    });
    let caught: unknown;
    try {
      await call('account_login');
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(IpcError);
    expect((caught as Error).message).toBe('wrong account name or password');
    // Error objects and plain objects are described by their message.
    expect(errorText(new Error('boom'))).toBe('boom');
    expect(errorText({ message: 'boom' })).toBe('boom');
    expect(errorText({ a: 1 })).toBe('{"a":1}');
  });
});
