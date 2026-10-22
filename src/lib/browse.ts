import type { ListedServer, ProbeResult, ProfileView } from './types';

export type SortKey = 'players' | 'ping' | 'name';

export interface BrowseFilter {
  /** Only servers on this client version (the selected game version); null = any. */
  version: string | null;
  search: string;
  /** Only servers that answered. */
  onlineOnly: boolean;
  tag: string | null;
  region: string | null;
  sort: SortKey;
}

export const defaultFilter = (version: string | null): BrowseFilter => ({
  version,
  search: '',
  onlineOnly: false,
  tag: null,
  region: null,
  sort: 'players',
});

/** The client version a server runs: what it said itself, else what the list says. */
export function effectiveVersion(s: ListedServer, p: ProbeResult | undefined): string {
  return p?.ok && p.info ? p.info.version : s.game_version;
}

/** Every word of the search must appear in the name, description, region, tags, address or contact. */
export function matchesSearch(s: ListedServer, search: string): boolean {
  const words = search.toLowerCase().split(/\s+/).filter(Boolean);
  if (words.length === 0) return true;
  const hay = [s.name, s.description, s.region, s.tags.join(' '), s.address, s.contact].join('\n').toLowerCase();
  return words.every((w) => hay.includes(w));
}

export function filterServers(list: ListedServer[], probes: Record<string, ProbeResult>, f: BrowseFilter): ListedServer[] {
  return list.filter((s) => {
    const p = probes[s.id];
    if (f.version && effectiveVersion(s, p) !== f.version) return false;
    if (f.onlineOnly && !p?.ok) return false;
    if (f.tag && !s.tags.includes(f.tag)) return false;
    if (f.region && s.region.toLowerCase() !== f.region.toLowerCase()) return false;
    return matchesSearch(s, f.search);
  });
}

/** Players (most first), ping (lowest first) or name; servers that did not answer go last, ties by name. */
export function sortServers(list: ListedServer[], probes: Record<string, ProbeResult>, key: SortKey): ListedServer[] {
  const by = (s: ListedServer) => probes[s.id];
  const up = (s: ListedServer) => (by(s)?.ok ? 0 : 1);
  return list.slice().sort((a, b) => {
    if (key !== 'name' && up(a) !== up(b)) return up(a) - up(b);
    let d = 0;
    if (key === 'players') d = (by(b)?.info?.online ?? 0) - (by(a)?.info?.online ?? 0);
    else if (key === 'ping') d = (by(a)?.ping_ms ?? Infinity) - (by(b)?.ping_ms ?? Infinity);
    if (!Number.isFinite(d) || d === 0) d = a.name.localeCompare(b.name, undefined, { sensitivity: 'base' });
    return d;
  });
}

export function allTags(list: ListedServer[]): string[] {
  return [...new Set(list.flatMap((s) => s.tags))].sort();
}

export function allRegions(list: ListedServer[]): string[] {
  const seen = new Map<string, string>();
  for (const s of list) if (s.region.trim() && !seen.has(s.region.toLowerCase())) seen.set(s.region.toLowerCase(), s.region);
  return [...seen.values()].sort((a, b) => a.localeCompare(b));
}

/** The profile that already points at this address, if any. */
export function existingProfile(s: ListedServer, profiles: ProfileView[]): ProfileView | undefined {
  const a = s.address.replace(/\/+$/, '').toLowerCase();
  return profiles.find((p) => p.url.replace(/\/+$/, '').toLowerCase() === a);
}

/** A contact line is a link only when it is an https address. */
export function contactLink(contact: string): string | null {
  const c = contact.trim();
  return /^https:\/\/[^\s<>"']+$/i.test(c) ? c : null;
}
