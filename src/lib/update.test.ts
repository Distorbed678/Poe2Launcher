import { describe, expect, it, vi } from 'vitest';
import { checkForUpdate, installUpdate, type UpdateDeps, type UpdateHandle, type UpdateState } from './update';

function handle(over: Partial<UpdateHandle> = {}): UpdateHandle {
  return { version: '0.2.0', currentVersion: '0.1.0', body: ' Fixes. ', downloadAndInstall: async () => {}, ...over };
}
const deps = (check: UpdateDeps['check'], relaunch = async () => {}): UpdateDeps => ({ check, relaunch });

describe('checkForUpdate', () => {
  it('reports an available update with version and trimmed notes', async () => {
    const h = handle({ date: '2026-10-21' });
    const r = await checkForUpdate(deps(async () => h));
    expect(r.state).toEqual({ kind: 'available', version: '0.2.0', notes: 'Fixes.', date: '2026-10-21' });
    expect(r.handle).toBe(h);
  });

  it('reports current when there is no newer version', async () => {
    expect((await checkForUpdate(deps(async () => null))).state).toEqual({ kind: 'current' });
  });

  it('turns a failed check (404 on a private repo, offline) into the quiet unavailable state', async () => {
    for (const err of ['Could not fetch a valid release JSON from the remote', new Error('network down')]) {
      const r = await checkForUpdate(deps(async () => Promise.reject(err)));
      expect(r.state.kind).toBe('unavailable');
      expect(r.handle).toBeNull();
    }
  });
});

describe('installUpdate', () => {
  it('shows progress, then restarts', async () => {
    const seen: UpdateState[] = [];
    const relaunch = vi.fn(async () => {});
    const h = handle({
      downloadAndInstall: async (on) => {
        on?.({ event: 'Started', data: { contentLength: 100 } });
        on?.({ event: 'Progress', data: { chunkLength: 40 } });
        on?.({ event: 'Progress', data: { chunkLength: 60 } });
        on?.({ event: 'Finished' });
      },
    });
    await installUpdate(h, deps(async () => h, relaunch), (s) => seen.push(s));
    expect(seen).toContainEqual({ kind: 'installing', done: 100, total: 100, phase: 'downloading' });
    expect(seen).toContainEqual({ kind: 'installing', done: 100, total: 100, phase: 'installing' });
    expect(seen.at(-1)).toEqual({ kind: 'restarting' });
    expect(relaunch).toHaveBeenCalledOnce();
  });

  it('a failed download or bad signature is an error state, without a restart', async () => {
    const seen: UpdateState[] = [];
    const relaunch = vi.fn(async () => {});
    const h = handle({ downloadAndInstall: async () => Promise.reject('signature verification failed') });
    await installUpdate(h, deps(async () => h, relaunch), (s) => seen.push(s));
    const last = seen.at(-1)!;
    expect(last.kind).toBe('error');
    expect(last.kind === 'error' && last.message).toContain('signature verification failed');
    expect(relaunch).not.toHaveBeenCalled();
  });

  it('a failed restart says the update is installed', async () => {
    const seen: UpdateState[] = [];
    const h = handle();
    await installUpdate(h, deps(async () => h, async () => Promise.reject('no')), (s) => seen.push(s));
    const last = seen.at(-1)!;
    expect(last.kind === 'error' && last.message).toContain('installed');
  });
});
