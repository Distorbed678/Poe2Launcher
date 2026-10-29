import { describe, expect, it } from 'vitest';
import { allRegions, allTags, contactLink, defaultFilter, effectiveVersion, existingProfile, filterServers, matchesSearch, sortServers } from './browse';
import type { ListedServer, ProbeResult, ProfileView, ServerInfo } from './types';

const srv = (id: string, over: Partial<ListedServer> = {}): ListedServer => ({
  id, name: id, address: `https://${id}.example.org`, description: '', region: 'EU', game_version: '0.5.5e.hotfix1', tags: [], contact: '', mods: [], plain_http: false, ...over,
});
const info = (online: number, version = '0.5.5e.hotfix1'): ServerInfo => ({
  name: 'n', version, cluster_id: 'c', online, open_registration: true, login_host: 'h', login_port: 1, instance_port: null, gateway_region: null, uptime_secs: 1,
});
const up = (id: string, online: number, ping: number, version = '0.5.5e.hotfix1'): [string, ProbeResult] => [id, { id, ok: true, ping_ms: ping, info: info(online, version), error: null }];
const down = (id: string): [string, ProbeResult] => [id, { id, ok: false, ping_ms: null, info: null, error: 'x' }];

const list = [srv('alpha', { tags: ['pve'], region: 'EU' }), srv('bravo', { tags: ['pvp', 'hardcore'], region: 'NA', description: 'Fast leveling' }), srv('charlie', { game_version: '0.6.0' }), srv('delta')];
const probes = Object.fromEntries([up('alpha', 3, 80), up('bravo', 12, 30), down('charlie'), up('delta', 3, 20, '0.6.0')]);

describe('filter', () => {
  it('filters by the live client version, falling back to the listed one', () => {
    const f = defaultFilter('0.5.5e.hotfix1');
    // delta says 0.5.5e.hotfix1 in the list but runs 0.6.0 (it told us); charlie did not answer and is listed as 0.6.0.
    expect(filterServers(list, probes, f).map((s) => s.id)).toEqual(['alpha', 'bravo']);
    expect(filterServers(list, probes, defaultFilter('0.6.0')).map((s) => s.id)).toEqual(['charlie', 'delta']);
    expect(filterServers(list, probes, defaultFilter(null))).toHaveLength(4);
    expect(effectiveVersion(list[3]!, probes.delta)).toBe('0.6.0');
    expect(effectiveVersion(list[2]!, probes.charlie)).toBe('0.6.0');
  });

  it('searches every word in name, description, tags, region and address', () => {
    expect(matchesSearch(list[1]!, 'fast bravo')).toBe(true);
    expect(matchesSearch(list[1]!, 'fast alpha')).toBe(false);
    expect(matchesSearch(list[1]!, 'HARDCORE')).toBe(true);
    expect(matchesSearch(list[0]!, 'alpha.example')).toBe(true);
    expect(matchesSearch(list[0]!, '   ')).toBe(true);
    const f = { ...defaultFilter(null), search: 'na' };
    expect(filterServers(list, probes, f).map((s) => s.id)).toContain('bravo');
  });

  it('online only, tag and region', () => {
    const base = defaultFilter(null);
    expect(filterServers(list, probes, { ...base, onlineOnly: true }).map((s) => s.id)).toEqual(['alpha', 'bravo', 'delta']);
    expect(filterServers(list, probes, { ...base, tag: 'pvp' }).map((s) => s.id)).toEqual(['bravo']);
    expect(filterServers(list, probes, { ...base, region: 'na' }).map((s) => s.id)).toEqual(['bravo']);
  });
});

describe('sort', () => {
  it('players: most first, offline last, ties by name', () => {
    expect(sortServers(list, probes, 'players').map((s) => s.id)).toEqual(['bravo', 'alpha', 'delta', 'charlie']);
  });
  it('ping: lowest first, offline last', () => {
    expect(sortServers(list, probes, 'ping').map((s) => s.id)).toEqual(['delta', 'bravo', 'alpha', 'charlie']);
  });
  it('name: plain order whether they answered or not', () => {
    expect(sortServers(list, probes, 'name').map((s) => s.id)).toEqual(['alpha', 'bravo', 'charlie', 'delta']);
  });
  it('servers that were not asked yet sort like offline ones and nothing breaks', () => {
    expect(sortServers(list, {}, 'ping').map((s) => s.id)).toEqual(['alpha', 'bravo', 'charlie', 'delta']);
  });
});

describe('helpers', () => {
  it('collects tags and regions', () => {
    expect(allTags(list)).toEqual(['hardcore', 'pve', 'pvp']);
    expect(allRegions([...list, srv('x', { region: 'eu' }), srv('y', { region: '' })])).toEqual(['EU', 'NA']);
  });
  it('finds a profile with the same address', () => {
    const p = { id: 'p', url: 'https://ALPHA.example.org/' } as ProfileView;
    expect(existingProfile(list[0]!, [p])).toBe(p);
    expect(existingProfile(list[1]!, [p])).toBeUndefined();
  });
  it('links a contact only if it is https', () => {
    expect(contactLink('https://discord.gg/abc')).toBe('https://discord.gg/abc');
    expect(contactLink('discord.gg/abc')).toBeNull();
    expect(contactLink('http://x.org')).toBeNull();
    expect(contactLink('me@example.org')).toBeNull();
    expect(contactLink('https://x.org and more')).toBeNull();
  });
});
