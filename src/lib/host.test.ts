import { describe, expect, it } from 'vitest';
import {
  formFromSettings, isUp, logLevel, playerAddress, portsToForward, pushLog, runLabel, settingsFromForm,
  validateHostForm, validPublicHost, updateCheckDue, updateHeadline, updatePlan, UPDATE_CHECK_TTL, type HostForm,
} from './host';
import type { HostSettings, ServerUpdateCheck } from './types';

const base: HostSettings = {
  server_name: 'My server',
  cluster_id: 'main',
  login_port: 20481,
  instance_port: 6112,
  web_port: 8080,
  public_host: '127.0.0.1',
  web_public: false,
  open_registration: false,
  admin_name: '',
  autostart: false,
  package_dir: null,
  configured: false,
  profile_id: null,
};

function form(over: Partial<HostForm> = {}): HostForm {
  return { ...formFromSettings(base, null), admin_name: 'boss', admin_password: 'long-enough', admin_password2: 'long-enough', ...over };
}

describe('the setup form', () => {
  it('offers the LAN address for a fresh setup and keeps a saved one', () => {
    expect(formFromSettings(base, '192.168.1.20').public_host).toBe('192.168.1.20');
    expect(formFromSettings(base, null).public_host).toBe('127.0.0.1');
    expect(formFromSettings({ ...base, configured: true }, '192.168.1.20').public_host).toBe('127.0.0.1');
    expect(formFromSettings({ ...base, public_host: 'play.example' }, '192.168.1.20').public_host).toBe('play.example');
    // The password never comes from saved settings.
    expect(formFromSettings(base, null).admin_password).toBe('');
  });

  it('accepts a good form', () => {
    expect(validateHostForm(form(), true)).toEqual([]);
  });

  it('lists what is wrong', () => {
    const bad = (over: Partial<HostForm>, text: string, needPw = true) =>
      expect(validateHostForm(form(over), needPw).join(' '), text).toContain(text);
    bad({ server_name: ' ' }, 'server name');
    bad({ cluster_id: 'Has Space' }, 'cluster id');
    bad({ login_port: 80 }, 'login port');
    bad({ web_port: 70000 }, 'web port');
    bad({ web_port: 20481 }, 'three different');
    bad({ public_host: 'http://x' }, 'public host');
    bad({ admin_name: 'ab' }, 'admin account name');
    bad({ admin_password: 'short', admin_password2: 'short' }, 'at least 8');
    bad({ admin_password2: 'different-pw' }, 'differ');
  });

  it('does not ask for a password when only re-applying settings', () => {
    expect(validateHostForm(form({ admin_name: '', admin_password: '', admin_password2: '' }), false)).toEqual([]);
    // But a typed password is still checked.
    expect(validateHostForm(form({ admin_password: 'short', admin_password2: 'short' }), false)).not.toEqual([]);
  });

  it('turns the form back into settings without the password', () => {
    const s = settingsFromForm(base, form({ server_name: '  Friends  ', public_host: ' 10.0.0.2 ' }));
    expect(s.server_name).toBe('Friends');
    expect(s.public_host).toBe('10.0.0.2');
    expect(JSON.stringify(s)).not.toContain('long-enough');
    expect(s.package_dir).toBeNull();
  });

  it('checks public hosts', () => {
    for (const ok of ['127.0.0.1', 'play.example', 'my-server', '2001:db8::1']) expect(validPublicHost(ok), ok).toBe(true);
    for (const bad of ['', ' ', 'a b', 'http://x', 'x:80', '-x', 'x..y']) expect(validPublicHost(bad), bad).toBe(false);
  });
});

describe('what to forward', () => {
  it('names the three kinds of traffic and the web port only when it is public', () => {
    const r = portsToForward({ login_port: 1, instance_port: 2, web_port: 3, web_public: false });
    expect(r.map((x) => [x.port, x.proto])).toEqual([[1, 'TCP'], [2, 'TCP + UDP']]);
    expect(portsToForward({ login_port: 1, instance_port: 2, web_port: 3, web_public: true }).at(-1)).toMatchObject({ port: 3, proto: 'TCP' });
  });
  it('gives the address players type', () => {
    expect(playerAddress({ public_host: ' play.example ', web_port: 8080 })).toBe('play.example:8080');
    expect(playerAddress({ public_host: '2001:db8::1', web_port: 8080 })).toBe('[2001:db8::1]:8080');
  });
});

describe('run state and log', () => {
  it('words the states', () => {
    expect(runLabel({ state: 'stopped' }, false)).toBe('Stopped');
    expect(runLabel({ state: 'running', pid: 7 }, false)).toContain('Starting');
    expect(runLabel({ state: 'running', pid: 7 }, true)).toContain('Running');
    expect(runLabel({ state: 'exited', code: 3 }, false)).toContain('unexpectedly');
    expect(runLabel({ state: 'exited', code: 0 }, false)).toBe('Ended');
    expect(isUp({ state: 'running', pid: 1 })).toBe(true);
    expect(isUp({ state: 'exited', code: 1 })).toBe(false);
  });
  it('keeps the newest lines', () => {
    let l: string[] = [];
    for (let i = 0; i < 7; i++) l = pushLog(l, `l${i}`, 5);
    expect(l).toEqual(['l2', 'l3', 'l4', 'l5', 'l6']);
  });
  it('classifies lines', () => {
    expect(logLevel('[launcher] started')).toBe('launcher');
    expect(logLevel('2026 ERROR x: y')).toBe('error');
    expect(logLevel('2026  WARN x: y')).toBe('warn');
    expect(logLevel('2026  INFO x: y')).toBe('plain');
  });
});

describe('server updates', () => {
  const check = (over: Partial<ServerUpdateCheck> = {}): ServerUpdateCheck => ({
    installed: '0.2.0',
    release: { repo: 'Distorbed678/Poe2Server', tag: 'server-v0.3.0', version: '0.3.0', game_version: '0.5.5e', prerelease: false, asset_name: 'a', asset_url: 'u', size: 1, sha256_url: 's', notes: '- new', html_url: null },
    update_available: true,
    error: null,
    checked_at: 1000,
    ...over,
  });

  it('says what is on offer, or nothing', () => {
    expect(updateHeadline(check())).toBe('Server update available (v0.3.0)');
    expect(updateHeadline(check({ update_available: false }))).toBeNull();
    expect(updateHeadline(check({ release: null }))).toBeNull();
    expect(updateHeadline(null)).toBeNull();
  });

  it('asks again after the TTL, for another game version, or when forced', () => {
    const last = { at: 1000, game: '0.5.5e' };
    expect(updateCheckDue(null, '0.5.5e', 1001)).toBe(true);
    expect(updateCheckDue(last, '0.5.5e', 1000 + UPDATE_CHECK_TTL - 1)).toBe(false);
    expect(updateCheckDue(last, '0.5.5e', 1000 + UPDATE_CHECK_TTL)).toBe(true);
    expect(updateCheckDue(last, '0.6.0', 1001)).toBe(true);
    expect(updateCheckDue(last, '0.5.5e', 1001, true)).toBe(true);
  });

  it('describes the steps, which depend on the server running', () => {
    const running = updatePlan(true);
    expect(running).toContain('stops the server');
    expect(running).toContain('starts the server again');
    expect(running).toContain('If anything fails, the old version is put back.');
    const stopped = updatePlan(false);
    expect(stopped).not.toContain('stops the server');
    expect(stopped).not.toContain('starts the server again');
    expect(stopped).toContain('backs up its database, installs the new version and keeps your settings (new settings get their defaults).');
    expect(updatePlan(false, false)).toContain('The update installs the new version and keeps your settings');
  });
});
