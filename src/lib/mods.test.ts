import { describe, expect, it } from 'vitest';
import { blocksEnable, compatLabel, dependentsOf, isValid, isZip, sortMods, stepTarget } from './mods';
import type { InstalledMod } from './types';

function mod(id: string, o: Partial<InstalledMod> & { deps?: string[] } = {}): InstalledMod {
  return {
    id,
    dir_name: id,
    errors: [],
    unknown_keys: [],
    compat: { kind: 'ok' },
    enabled: false,
    order: null,
    manifest: {
      id, name: id, version: '1.0.0', description: '', authors: [], license: '', homepage: '',
      game_versions: [], hook_api: null, after: [], before: [], conflicts: [],
      dependencies: Object.fromEntries((o.deps ?? []).map((d) => [d, '*'])), entry_dll: null,
    },
    ...o,
  };
}

describe('mods helpers', () => {
  it('labels compatibility', () => {
    expect(compatLabel({ kind: 'ok' }).tone).toBe('ok');
    expect(compatLabel({ kind: 'unspecified' }).tone).toBe('warn');
    expect(compatLabel({ kind: 'wrong_game', supported: ['0.4.0'] }).text).toBe('made for 0.4.0');
    expect(compatLabel({ kind: 'hook_api_too_new', needs: 2, have: 1 }).text).toContain('hook API 2');
    expect(compatLabel(null).tone).toBe('bad');
    expect(blocksEnable({ kind: 'wrong_game', supported: [] })).toBe(true);
    expect(blocksEnable({ kind: 'unspecified' })).toBe(false);
    expect(blocksEnable(null)).toBe(false);
  });

  it('lists enabled mods in load order first', () => {
    const list = [mod('zeta'), mod('b', { enabled: true, order: 1 }), mod('alpha'), mod('a', { enabled: true, order: 0 })];
    expect(sortMods(list).map((m) => m.id)).toEqual(['a', 'b', 'alpha', 'zeta']);
  });

  it('computes one-step moves within the enabled mods', () => {
    const list = [mod('a', { enabled: true, order: 0 }), mod('b', { enabled: true, order: 1 }), mod('c')];
    expect(stepTarget(list, 'b', -1)).toBe(0);
    expect(stepTarget(list, 'a', -1)).toBeNull();
    expect(stepTarget(list, 'b', 1)).toBeNull();
    expect(stepTarget(list, 'c', 1)).toBeNull();
  });

  it('finds enabled dependents and validity', () => {
    const list = [mod('lib', { enabled: true, order: 0 }), mod('user', { enabled: true, order: 1, deps: ['lib'] }), mod('off', { deps: ['lib'] })];
    expect(dependentsOf(list, 'lib')).toEqual(['user']);
    expect(dependentsOf(list, 'user')).toEqual([]);
    expect(isValid(list[0])).toBe(true);
    expect(isValid(mod('bad', { errors: ['x'] }))).toBe(false);
    expect(isValid({ ...mod('nomanifest'), manifest: null })).toBe(false);
  });

  it('recognises zip files', () => {
    expect(isZip('/a/b/mod.ZIP')).toBe(true);
    expect(isZip('/a/b/mod')).toBe(false);
  });
});
