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
    await ipc.modSetEnabled('p1', 'my-mod', '1.0.0', true, true);
    expect(invoke).toHaveBeenCalledWith('mod_set_enabled', { scope: 'p1', id: 'my-mod', version: '1.0.0', enabled: true, force: true });
    await ipc.modSyncApply('p1');
    expect(invoke).toHaveBeenLastCalledWith('mod_sync_apply', { profileId: 'p1', ids: null });
    invoke.mockResolvedValue(undefined);
    await ipc.play('p1');
    expect(invoke).toHaveBeenLastCalledWith('play', { profileId: 'p1' });
    // The official mode plays without a server profile.
    await ipc.play(null);
    expect(invoke).toHaveBeenLastCalledWith('play', { profileId: null });
    await ipc.setGameVersion('official');
    expect(invoke).toHaveBeenLastCalledWith('game_version_set', { version: 'official' });
    await ipc.steamLogout();
    expect(invoke).toHaveBeenLastCalledWith('steam_logout', undefined);
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
