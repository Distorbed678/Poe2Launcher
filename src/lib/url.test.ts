import { describe, expect, it } from 'vitest';
import { needsPlainHttpConfirmation, normalizeServerUrl, plainHttpRisk, plainHttpWarning } from './url';

describe('normalizeServerUrl', () => {
  it('adds a scheme, trims slashes and a pasted api path', () => {
    expect(normalizeServerUrl('127.0.0.1:8080')).toBe('http://127.0.0.1:8080');
    expect(normalizeServerUrl('192.168.1.9:8080')).toBe('http://192.168.1.9:8080');
    expect(normalizeServerUrl('play.example')).toBe('https://play.example');
    expect(normalizeServerUrl('203.0.113.7:8080')).toBe('https://203.0.113.7:8080');
    expect(normalizeServerUrl('http://203.0.113.7:8080')).toBe('http://203.0.113.7:8080');
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

describe('plain http risk (audit S-03; mirrors the backend)', () => {
  it('grades addresses', () => {
    for (const u of ['https://play.example', 'HTTPS://192.168.1.2']) expect(plainHttpRisk(u)).toBe('https');
    for (const u of ['http://127.0.0.1:8080', 'http://localhost:8080', 'http://[::1]:8080', 'http://127.5.5.5']) {
      expect(plainHttpRisk(u), u).toBe('loopback');
      expect(needsPlainHttpConfirmation(u)).toBe(false);
    }
    for (const u of ['http://192.168.1.5:8080', 'http://10.0.0.2', 'http://172.16.3.4', 'http://169.254.1.1', 'http://nas.local', 'http://myserver', 'http://[fd00::1]']) {
      expect(plainHttpRisk(u), u).toBe('private');
      expect(needsPlainHttpConfirmation(u)).toBe(true);
    }
    for (const u of ['http://203.0.113.7:8080', 'http://play.example', 'http://172.32.0.1', 'http://8.8.8.8']) {
      expect(plainHttpRisk(u), u).toBe('public');
      expect(needsPlainHttpConfirmation(u)).toBe(true);
    }
  });
  it('warns only where there is something to warn about', () => {
    expect(plainHttpWarning('https')).toBeNull();
    expect(plainHttpWarning('loopback')).toBeNull();
    expect(plainHttpWarning('private')).toContain('unencrypted');
    expect(plainHttpWarning('public')).toContain('https://');
  });
});

describe('plain http block', () => {
  it('blocks private and public plain http until confirmed', async () => {
    const { isPlainHttpBlocked } = await import('./url');
    expect(isPlainHttpBlocked({ plain_http: 'public', plain_http_confirmed: false })).toBe(true);
    expect(isPlainHttpBlocked({ plain_http: 'private', plain_http_confirmed: true })).toBe(false);
    expect(isPlainHttpBlocked({ plain_http: 'loopback', plain_http_confirmed: false })).toBe(false);
    expect(isPlainHttpBlocked({ plain_http: 'https', plain_http_confirmed: false })).toBe(false);
  });
});
