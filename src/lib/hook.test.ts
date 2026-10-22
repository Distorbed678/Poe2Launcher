import { describe, expect, it } from 'vitest';
import { currentStep, hookAction, hookReady, hookSummary, setupNeeded, setupSteps } from './hook';
import type { HookCheck, HookStatus, InstalledHook } from './types';

const installed: InstalledHook = { dir: '/h/0.2.0', name: 'n', version: '0.2.0', game_versions: ['0.5.5e'], mod_abi: 1, commit: 'abc' };

function status(over: Partial<HookStatus> = {}): HookStatus {
  return {
    game_version: '0.5.5e', repo: 'x/y', source: { kind: 'missing' }, dll_dir: '/d', manual_dir: null,
    installed: null, other_installed: [], busy: false, ...over,
  };
}

const check = (update: boolean): HookCheck => ({
  update_available: update,
  release: { tag: 'hook-v0.3.0', version: '0.3.0', game_versions: ['0.5.5e'], prerelease: false, asset_name: 'a', asset_url: 'u', size: 1, sha256_url: 's' },
});

describe('hook summary', () => {
  it('says what is missing, and what is in use', () => {
    expect(hookSummary(null, null).label).toBe('checking');
    expect(hookReady(status())).toBe(false);
    expect(hookSummary(status(), null)).toMatchObject({ tone: 'bad', label: 'not installed' });
    const ok = status({ source: { kind: 'downloaded', version: '0.2.0' }, installed });
    expect(hookReady(ok)).toBe(true);
    expect(hookSummary(ok, check(false))).toMatchObject({ tone: 'ok', label: '0.2.0 installed' });
    expect(hookSummary(ok, check(true))).toMatchObject({ tone: 'warn', label: '0.2.0, update available' });
    expect(hookSummary(status({ source: { kind: 'manual' }, dll_dir: '/mine', installed }), null).label).toBe('your own build');
    expect(hookReady(status({ source: { kind: 'legacy' } }))).toBe(true);
  });

  it('offers the right button', () => {
    expect(hookAction(status(), null)).toEqual({ label: 'Download client hook', primary: true });
    const ok = status({ source: { kind: 'downloaded', version: '0.2.0' }, installed });
    expect(hookAction(ok, null)).toEqual({ label: 'Check for updates', primary: false });
    expect(hookAction(ok, check(true))).toEqual({ label: 'Update', primary: true });
    expect(hookAction(ok, check(false))).toEqual({ label: 'Check again', primary: false });
  });
});

describe('setup checklist', () => {
  it('walks through game, hook, server', () => {
    let steps = setupSteps({ gameInstalled: false, hookReady: false, profiles: 0 });
    expect(steps.map((s) => s.done)).toEqual([false, false, false]);
    expect(currentStep(steps)).toBe('game');
    steps = setupSteps({ gameInstalled: true, hookReady: false, profiles: 0 });
    expect(currentStep(steps)).toBe('hook');
    steps = setupSteps({ gameInstalled: true, hookReady: true, profiles: 0 });
    expect(currentStep(steps)).toBe('server');
    expect(setupNeeded(steps)).toBe(true);
    steps = setupSteps({ gameInstalled: true, hookReady: true, profiles: 1 });
    expect(setupNeeded(steps)).toBe(false);
    expect(currentStep(steps)).toBeNull();
  });
});
