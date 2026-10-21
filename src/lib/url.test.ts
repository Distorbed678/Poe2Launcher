import { describe, expect, it } from 'vitest';
import { normalizeServerUrl } from './url';

describe('normalizeServerUrl', () => {
  it('adds a scheme, trims slashes and a pasted api path', () => {
    expect(normalizeServerUrl('127.0.0.1:8080')).toBe('http://127.0.0.1:8080');
    expect(normalizeServerUrl('  http://play.example/ ')).toBe('http://play.example');
    expect(normalizeServerUrl('HTTPS://play.example:8443/api/v1/')).toBe('https://play.example:8443');
    expect(normalizeServerUrl('http://h/admin')).toBe('http://h/admin');
  });
  it('rejects things that are not addresses', () => {
    for (const bad of ['', '   ', 'ftp://x', 'http://', 'http:///x', 'http://a b', 'http://h?x=1', ':80']) {
      expect(normalizeServerUrl(bad), bad).toBeNull();
    }
  });
});
