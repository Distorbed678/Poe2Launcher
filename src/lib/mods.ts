import type { Compat, InstalledMod } from './types';

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
      return { text: `needs hook API ${c.needs} (have ${c.have})`, tone: 'bad' };
  }
}

export function blocksEnable(c: Compat | null): boolean {
  return c !== null && (c.kind === 'wrong_game' || c.kind === 'hook_api_too_new');
}

/** Enabled mods in load order, then the others by id. */
export function sortMods(mods: InstalledMod[]): InstalledMod[] {
  return mods.slice().sort((a, b) => {
    if (a.enabled !== b.enabled) return a.enabled ? -1 : 1;
    if (a.enabled && b.enabled) return (a.order ?? 0) - (b.order ?? 0);
    return a.id.localeCompare(b.id);
  });
}

export function isValid(m: InstalledMod): boolean {
  return m.manifest !== null && m.errors.length === 0;
}

/** Where the mod ends up when moved one step; null at the ends. */
export function stepTarget(mods: InstalledMod[], id: string, delta: -1 | 1): number | null {
  const enabled = sortMods(mods).filter((m) => m.enabled);
  const i = enabled.findIndex((m) => m.id === id);
  if (i < 0) return null;
  const j = i + delta;
  return j < 0 || j >= enabled.length ? null : j;
}

/** Mods that need `id` and are enabled: disabling or removing it affects them. */
export function dependentsOf(mods: InstalledMod[], id: string): string[] {
  return mods
    .filter((m) => m.enabled && m.id !== id && m.manifest && id in m.manifest.dependencies)
    .map((m) => m.id);
}

export function isZip(path: string): boolean {
  return /\.zip$/i.test(path);
}
