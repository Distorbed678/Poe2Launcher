import { describe, expect, it } from 'vitest';
import {
  blocksEnable, bestVersion, compatLabel, defaultScope, dialogHasCode, dialogMods, filterIndex, groupMods, indexTags,
  isValid, isZip, needsDialog, overridable, scopeOptions, sideLabel, stepTarget, syncBlocker, totalBytes,
} from './mods';
import type { IndexMod, IndexVersion, ModEntry, ProfileView, SyncMod, SyncPlan } from './types';

function entry(id: string, version: string, o: Partial<ModEntry> = {}): ModEntry {
  return {
    id, version, side: 'client', errors: [], unknown_keys: [], compat: { kind: 'ok' }, compat_text: 'compatible', sha256: 'a'.repeat(64),
    size: 100, source: 'zip', added: 0, has_code: true, server_half_left_out: false, enabled: false, enabled_all: false, required: false,
    in_effect: false, origin: null, order: null,
    manifest: {
      id, name: id, version, description: '', authors: [], license: '', homepage: '', side: 'client', game_versions: [], hook_api: 1, server_api: null,
      after: [], before: [], conflicts: [], dependencies: {}, entry_dll: `${id}.dll`, has_code: true,
    },
    ...o,
  };
}

function sync(id: string, o: Partial<SyncMod> = {}): SyncMod {
  return { id, version: '1.0.0', name: id, side: 'both', size: 1000, sha256: '', required: true, code: false, authors: [], description: '', state: 'missing', detail: '', ...o };
}

function plan(o: Partial<SyncPlan> = {}): SyncPlan {
  return { profile_id: 'p1', profile_name: 'Realm', mode: 'ask', status: 'ask', mods: [sync('aa')], blocker: null, download_bytes: 1000, skipped: [], stale: false, applied: [], errors: [], ...o };
}

function version(v: string, o: Partial<IndexVersion> = {}): IndexVersion {
  return { version: v, released: '', game_versions: ['0.5.5e'], hook_api: 1, server_api: null, sha256: 'b'.repeat(64), size: 1, url: 'https://x', yanked: false, compat: { kind: 'ok' }, cached: false, ...o };
}

function imod(id: string, o: Partial<IndexMod> = {}): IndexMod {
  return { id, name: id.toUpperCase(), description: `the ${id} mod`, author: 'someone', side: 'client', homepage: '', licence: 'MIT', tags: [], versions: [version('1.0.0')], ...o };
}

describe('mods helpers', () => {
  it('labels compatibility, including the mod ABI cases', () => {
    expect(compatLabel({ kind: 'ok' }).tone).toBe('ok');
    expect(compatLabel({ kind: 'unspecified' }).tone).toBe('warn');
    expect(compatLabel({ kind: 'wrong_game', supported: ['0.4.0'] }).text).toBe('made for 0.4.0');
    expect(compatLabel({ kind: 'hook_api_too_new', needs: 2, have: 1 }).text).toContain('ABI 2');
    expect(compatLabel({ kind: 'hook_api_old', needs: 1, have: 2 }).text).toContain('ABI 1');
    expect(compatLabel({ kind: 'hook_api_missing', have: 1 }).tone).toBe('bad');
    expect(compatLabel(null).tone).toBe('bad');
    expect(blocksEnable({ kind: 'wrong_game', supported: [] })).toBe(true);
    expect(blocksEnable({ kind: 'hook_api_old', needs: 1, have: 2 })).toBe(true);
    expect(blocksEnable({ kind: 'unspecified' })).toBe(false);
    expect(blocksEnable(null)).toBe(false);
    expect(overridable({ kind: 'wrong_game', supported: [] })).toBe(true);
    expect(overridable({ kind: 'hook_api_too_new', needs: 2, have: 1 })).toBe(false);
  });

  it('gives each side its badge', () => {
    expect(sideLabel('client').text).toBe('client');
    expect(sideLabel('both').tone).toBe('accent');
    expect(sideLabel('server').title).toContain('does not belong');
  });

  it('groups versions per mod, newest first, with the loading mods in load order on top', () => {
    const list = [
      entry('zeta', '1.0.0'),
      entry('bb', '2.0.0', { enabled: true, in_effect: true, order: 1 }),
      entry('bb', '1.0.0'),
      entry('aa', '1.0.0', { enabled: true, in_effect: true, order: 0 }),
      entry('cc', '1.0.0', { enabled: true }), // enabled but left out
    ];
    const g = groupMods(list);
    expect(g.map((x) => x.id)).toEqual(['aa', 'bb', 'cc', 'zeta']);
    expect(g[1].versions.map((v) => v.version)).toEqual(['2.0.0', '1.0.0']);
    expect(g[1].enabled?.version).toBe('2.0.0');
    expect(g[2].inEffect).toBeNull();
  });

  it('computes one-step moves within the scope\'s enabled mods', () => {
    const list = [entry('a', '1.0.0', { enabled: true }), entry('b', '1.0.0', { enabled: true }), entry('c', '1.0.0')];
    expect(stepTarget(list, 'b', -1)).toBe(0);
    expect(stepTarget(list, 'a', -1)).toBeNull();
    expect(stepTarget(list, 'b', 1)).toBeNull();
    expect(stepTarget(list, 'c', 1)).toBeNull();
  });

  it('knows valid entries and zip files', () => {
    expect(isValid(entry('a', '1.0.0'))).toBe(true);
    expect(isValid(entry('a', '1.0.0', { errors: ['x'] }))).toBe(false);
    expect(isValid({ ...entry('a', '1.0.0'), manifest: null })).toBe(false);
    expect(isZip('/a/b/mod.ZIP')).toBe(true);
    expect(isZip('/a/b/mod')).toBe(false);
  });
});

describe('mod sync state', () => {
  it('shows the dialog only in Ask mode with required mods to download', () => {
    expect(needsDialog(plan())).toBe(true);
    expect(needsDialog(plan({ status: 'ok', mods: [sync('aa', { state: 'ready' })] }))).toBe(false);
    expect(needsDialog(plan({ status: 'off' }))).toBe(false);
    expect(needsDialog(plan({ mods: [sync('aa', { required: false })] }))).toBe(false);
    expect(needsDialog(null)).toBe(false);
  });

  it('lists what the dialog downloads and whether any of it is code', () => {
    const p = plan({ mods: [sync('aa', { code: true, size: 10 }), sync('bb', { state: 'changed', size: 20 }), sync('cc', { state: 'ready' }), sync('dd', { required: false })] });
    const rows = dialogMods(p);
    expect(rows.map((m) => m.id)).toEqual(['aa', 'bb']);
    expect(dialogHasCode(rows)).toBe(true);
    expect(dialogHasCode([sync('x')])).toBe(false);
    expect(totalBytes(rows)).toBe(30);
  });

  it('blocks Play with the server\'s reason in Off mode or for an unusable required mod', () => {
    expect(syncBlocker(plan({ status: 'off', blocker: 'Mod sync is Off' }))).toBe('Mod sync is Off');
    expect(syncBlocker(plan({ status: 'blocked', blocker: null }))).toContain('cannot be loaded');
    expect(syncBlocker(plan({ status: 'ask' }))).toBeNull();
    expect(syncBlocker(plan({ status: 'ok' }))).toBeNull();
    expect(syncBlocker(undefined)).toBeNull();
  });
});

describe('the mod browser', () => {
  const list = [
    imod('fast-rolls', { tags: ['qol', 'loot'], side: 'both' }),
    imod('old-ui', { versions: [version('1.0.0', { compat: { kind: 'wrong_game', supported: ['0.4.0'] } })], tags: ['ui'] }),
    imod('srv-tool', { side: 'server', author: 'Ann' }),
  ];

  it('filters by text, side, tag and game version', () => {
    const f = { query: '', side: 'any' as const, gameOnly: false, tag: '' };
    expect(filterIndex(list, f).length).toBe(3);
    expect(filterIndex(list, { ...f, gameOnly: true }).map((m) => m.id)).toEqual(['fast-rolls', 'srv-tool']);
    expect(filterIndex(list, { ...f, side: 'both' }).map((m) => m.id)).toEqual(['fast-rolls']);
    expect(filterIndex(list, { ...f, tag: 'ui' }).map((m) => m.id)).toEqual(['old-ui']);
    expect(filterIndex(list, { ...f, query: 'ann' }).map((m) => m.id)).toEqual(['srv-tool']);
    expect(filterIndex(list, { ...f, query: ' ROLLS ' }).map((m) => m.id)).toEqual(['fast-rolls']);
    expect(filterIndex(list, { ...f, query: 'loot' }).map((m) => m.id)).toEqual(['fast-rolls']);
    expect(indexTags(list)).toEqual(['loot', 'qol', 'ui']);
  });

  it('offers the newest usable version, never a withdrawn one', () => {
    const m = imod('x', { versions: [version('3.0.0', { yanked: true }), version('2.0.0', { compat: { kind: 'wrong_game', supported: [] } }), version('1.0.0')] });
    expect(bestVersion(m)?.version).toBe('1.0.0');
    const none = imod('y', { versions: [version('2.0.0', { compat: { kind: 'wrong_game', supported: [] } }), version('1.0.0', { yanked: true })] });
    expect(bestVersion(none)?.version).toBe('2.0.0');
    expect(bestVersion(imod('z', { versions: [version('1.0.0', { yanked: true })] }))).toBeNull();
  });
});

describe('scopes', () => {
  const profiles = [{ id: 'p1', name: 'Realm' }, { id: 'p2', name: 'LAN' }] as ProfileView[];
  it('lists all servers first and starts in the selected server', () => {
    expect(scopeOptions(profiles).map((o) => o.value)).toEqual(['all', 'p1', 'p2']);
    expect(defaultScope('p2', profiles)).toBe('p2');
    expect(defaultScope('gone', profiles)).toBe('all');
    expect(defaultScope(null, [])).toBe('all');
  });
});
