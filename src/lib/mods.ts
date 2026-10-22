import type { Compat, IndexMod, IndexVersion, ModEntry, ModOrigin, ModSide, ProfileView, SyncMod, SyncPlan, SyncState } from './types';

export const SCOPE_ALL = 'all';

export function compatLabel(c: Compat | null): { text: string; tone: 'ok' | 'warn' | 'bad' } {
  if (!c) return { text: 'invalid', tone: 'bad' };
  switch (c.kind) {
    case 'ok':
      return { text: 'compatible', tone: 'ok' };
    case 'unspecified':
      return { text: 'no client version stated', tone: 'warn' };
    case 'wrong_game':
      return { text: `made for ${c.supported.join(', ')}`, tone: 'bad' };
    case 'hook_api_too_new':
      return { text: `needs mod ABI ${c.needs} (hook offers ${c.have})`, tone: 'bad' };
    case 'hook_api_old':
      return { text: `built for mod ABI ${c.needs} (hook offers ${c.have})`, tone: 'bad' };
    case 'hook_api_missing':
      return { text: 'a DLL without a stated mod ABI', tone: 'bad' };
  }
}

/** Whether the mod cannot be enabled without the player's say (or at all). */
export function blocksEnable(c: Compat | null): boolean {
  return c !== null && c.kind !== 'ok' && c.kind !== 'unspecified';
}

/** Only a game-version mismatch can be overridden; the loader refuses a wrong mod ABI anyway. */
export function overridable(c: Compat | null): boolean {
  return c !== null && c.kind === 'wrong_game';
}

export function sideLabel(side: ModSide): { text: string; tone: 'info' | 'accent' | 'neutral'; title: string } {
  switch (side) {
    case 'client':
      return { text: 'client', tone: 'info', title: 'Runs in the game on your computer' };
    case 'server':
      return { text: 'server', tone: 'neutral', title: 'Runs on a server only: it does not belong in the launcher' };
    case 'both':
      return { text: 'both', tone: 'accent', title: 'One package with a client half (installed here) and a server half (stays on the server)' };
  }
}

export function originLabel(o: ModOrigin | null): string {
  switch (o) {
    case 'required':
      return 'required by the server';
    case 'profile':
      return 'enabled for this server';
    case 'global':
      return 'enabled for all servers';
    case 'dependency':
      return 'needed by another mod';
    default:
      return '';
  }
}

export interface ModGroup {
  id: string;
  name: string;
  side: ModSide;
  /** Newest version first. */
  versions: ModEntry[];
  /** The version that is enabled in the scope, if any. */
  enabled: ModEntry | null;
  /** Part of what Play loads. */
  inEffect: ModEntry | null;
}

/** One group per mod id: every cached version, the enabled one, and the one that loads. */
export function groupMods(entries: ModEntry[]): ModGroup[] {
  const by = new Map<string, ModEntry[]>();
  for (const e of entries) by.set(e.id, [...(by.get(e.id) ?? []), e]);
  const groups: ModGroup[] = [];
  for (const [id, versions] of by) {
    const newest = versions[0];
    groups.push({
      id,
      name: newest.manifest?.name ?? id,
      side: newest.side,
      versions,
      enabled: versions.find((v) => v.enabled) ?? null,
      inEffect: versions.find((v) => v.in_effect) ?? null,
    });
  }
  // What loads first (in load order), then the enabled but not loading, then the rest by name.
  const rank = (g: ModGroup) => (g.inEffect ? 0 : g.enabled ? 1 : 2);
  return groups.sort((a, b) => {
    if (rank(a) !== rank(b)) return rank(a) - rank(b);
    if (a.inEffect && b.inEffect) return (a.inEffect.order ?? 0) - (b.inEffect.order ?? 0);
    return a.name.localeCompare(b.name) || a.id.localeCompare(b.id);
  });
}

/** Where an enabled mod ends up when moved one step among the scope's enabled mods; null at the ends. */
export function stepTarget(entries: ModEntry[], id: string, delta: -1 | 1): number | null {
  const enabled = entries.filter((e) => e.enabled);
  const i = enabled.findIndex((e) => e.id === id);
  if (i < 0) return null;
  const j = i + delta;
  return j < 0 || j >= enabled.length ? null : j;
}

export function isValid(m: ModEntry): boolean {
  return m.manifest !== null && m.errors.length === 0;
}

export function isZip(path: string): boolean {
  return /\.zip$/i.test(path);
}

// --- mod sync -------------------------------------------------------------------------------

export function syncStateLabel(s: SyncState): { text: string; tone: 'ok' | 'warn' | 'bad' | 'info' } {
  switch (s) {
    case 'ready':
      return { text: 'ready', tone: 'ok' };
    case 'missing':
      return { text: 'not downloaded', tone: 'warn' };
    case 'changed':
      return { text: 'other build cached', tone: 'warn' };
    case 'incompatible':
      return { text: 'incompatible', tone: 'bad' };
    case 'no_source':
      return { text: 'no download address', tone: 'bad' };
  }
}

/** The mods the sync dialog lists: the required ones that still have to be downloaded. */
export function dialogMods(plan: SyncPlan): SyncMod[] {
  return plan.mods.filter((m) => m.required && (m.state === 'missing' || m.state === 'changed'));
}

/** Whether Ask mode should show its dialog now. */
export function needsDialog(plan: SyncPlan | undefined | null): boolean {
  return !!plan && plan.status === 'ask' && dialogMods(plan).length > 0;
}

/** Why Play cannot go on because of the server's mods (Off with something missing, or an unusable required mod). */
export function syncBlocker(plan: SyncPlan | undefined | null): string | null {
  if (!plan) return null;
  if (plan.status === 'off' || plan.status === 'blocked') return plan.blocker ?? 'The server requires mods that cannot be loaded';
  return null;
}

export function dialogHasCode(mods: SyncMod[]): boolean {
  return mods.some((m) => m.code);
}

export function totalBytes(mods: SyncMod[]): number {
  return mods.reduce((n, m) => n + m.size, 0);
}

// --- the mod browser -------------------------------------------------------------------------

export interface IndexFilter {
  query: string;
  side: 'any' | ModSide;
  /** Only mods that have a version for the selected game version. */
  gameOnly: boolean;
  tag: string;
}

export const emptyFilter: IndexFilter = { query: '', side: 'any', gameOnly: true, tag: '' };

/** A version that works with the selected game version and mod ABI, and is not withdrawn. */
export function usableVersion(v: IndexVersion): boolean {
  return !v.yanked && !blocksEnable(v.compat);
}

/** The version the Install button offers: the newest usable one, else the newest that is not withdrawn. */
export function bestVersion(m: IndexMod): IndexVersion | null {
  return m.versions.find(usableVersion) ?? m.versions.find((v) => !v.yanked) ?? null;
}

export function filterIndex(mods: IndexMod[], f: IndexFilter): IndexMod[] {
  const q = f.query.trim().toLowerCase();
  return mods.filter((m) => {
    if (f.side !== 'any' && m.side !== f.side) return false;
    if (f.tag && !m.tags.includes(f.tag)) return false;
    if (f.gameOnly && !m.versions.some(usableVersion)) return false;
    if (!q) return true;
    return [m.id, m.name, m.description, m.author, ...m.tags].some((t) => t.toLowerCase().includes(q));
  });
}

export function indexTags(mods: IndexMod[]): string[] {
  return [...new Set(mods.flatMap((m) => m.tags))].sort();
}

/** The scope picker: all servers, then each server profile. */
export function scopeOptions(profiles: ProfileView[]): { value: string; label: string }[] {
  return [{ value: SCOPE_ALL, label: 'All servers' }, ...profiles.map((p) => ({ value: p.id, label: p.name }))];
}

/** The scope the page starts in: the selected server, else all servers. */
export function defaultScope(selected: string | null | undefined, profiles: ProfileView[]): string {
  return selected && profiles.some((p) => p.id === selected) ? selected : SCOPE_ALL;
}
