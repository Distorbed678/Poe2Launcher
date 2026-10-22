import { describe, expect, it } from 'vitest';
import { compatBlocker, isOfficial, privateVersions, serverCompat, versionLabel, versionSub } from './gameversion';
import type { GameVersion } from './types';

const v = (version: string, extra: Partial<GameVersion> = {}): GameVersion => ({
  version,
  app_id: 1,
  official: version === 'official',
  selected: false,
  newest: false,
  installed: false,
  ...extra,
});
const list = [v('official'), v('0.6.0', { newest: true }), v('0.5.5e')];

describe('game versions', () => {
  it('names the official entry and the private ones', () => {
    expect(isOfficial('official')).toBe(true);
    expect(isOfficial('0.5.5e')).toBe(false);
    expect(versionLabel('official')).toBe('Official (read-only)');
    expect(versionLabel(list[1]!)).toBe('0.6.0');
    expect(versionSub(list[0]!)).toMatch(/Official/);
    expect(versionSub(list[1]!)).toMatch(/newest/);
    expect(versionSub(list[2]!)).toBe('Private server');
    expect(privateVersions(list).map((x) => x.version)).toEqual(['0.6.0', '0.5.5e']);
  });

  it('compares the server version with the selection', () => {
    expect(serverCompat('0.5.5e', '0.5.5e', list)).toEqual({ kind: 'ok' });
    expect(serverCompat('0.6.0', '0.5.5e', list)).toEqual({ kind: 'other', version: '0.6.0' });
    expect(serverCompat('0.7.0', '0.5.5e', list)).toEqual({ kind: 'unsupported', version: '0.7.0' });
    expect(serverCompat(undefined, '0.5.5e', list)).toEqual({ kind: 'unknown' });
    // "official" is never a server version.
    expect(serverCompat('official', '0.5.5e', list)).toEqual({ kind: 'unsupported', version: 'official' });
  });

  it('only a known other version blocks Play', () => {
    expect(compatBlocker({ kind: 'other', version: '0.6.0' })).toMatch(/0\.6\.0/);
    expect(compatBlocker({ kind: 'unsupported', version: '0.7.0' })).toBeNull();
    expect(compatBlocker({ kind: 'ok' })).toBeNull();
    expect(compatBlocker({ kind: 'unknown' })).toBeNull();
  });
});
