import { describe, expect, it } from 'vitest';
import { hrefFor, pageFor, parseHash } from './router.svelte';

describe('router', () => {
  it('parses hashes and builds links', () => {
    expect(parseHash('#/library?tab=logs').segments).toEqual(['library']);
    expect(parseHash('#/library?tab=logs').query.get('tab')).toBe('logs');
    expect(hrefFor('library', { tab: 'mods', x: undefined })).toBe('#/library?tab=mods');
  });

  it('opens on Home and sends merged pages to their new place', () => {
    expect(pageFor(undefined)).toBe('home');
    expect(pageFor('account')).toBe('servers');
    expect(pageFor('install')).toBe('library');
    expect(pageFor('mods')).toBe('library');
    expect(pageFor('host')).toBe('servers');
  });
});
