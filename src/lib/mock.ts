/**
 * Development preview without the Tauri backend: `npm run dev`, then open `/?mock` in a browser.
 * It fakes the IPC (profiles, install state, settings, ...) so every page can be looked at and
 * screenshotted. Only loaded in development builds; `?versions=2` pretends the project supports a
 * second game version, `?official` starts in the official mode, `?empty` has no servers.
 */
import { mockIPC, mockWindows } from '@tauri-apps/api/mocks';
import type { AppInfo, GameVersion, InstallStatus, ProfileView, ServerInfo, Settings } from './types';

export function installMock(params: URLSearchParams): void {
  mockWindows('main');
  const two = params.has('versions');
  const empty = params.has('empty');
  let settings: Settings = {
    install_dirs: {},
    game_version: params.has('official') ? 'official' : null,
    official_ack: params.has('ack'),
    minimize_while_playing: false,
    custom_frame: true,
    dll_dir: null,
    steam_root: null,
    proton: 'Proton - Experimental',
    selected_profile: 'p1',
    steam_username: 'steam_user',
    remember_steam_login: true,
    prefer_code_entry: true,
    theme: params.get('theme') === 'light' ? 'light' : 'dark',
  };
  let remembered = true;
  const installed = new Set<string>(params.has('fresh') ? [] : (params.get('installed') ?? '0.5.5e').split(','));
  const profiles: ProfileView[] = empty
    ? []
    : [
        { id: 'p1', name: 'Local test realm', url: 'http://127.0.0.1:8080', logged_in: true, account: 'Alice', expires_at: Math.floor(Date.now() / 1000) + 6 * 86400, role: 'admin', plain_http: 'loopback', plain_http_confirmed: false, keyring_error: null },
        { id: 'p2', name: 'Friends LAN party', url: 'http://192.168.1.20:8080', logged_in: false, account: null, expires_at: null, role: null, plain_http: 'private', plain_http_confirmed: true, keyring_error: null },
        { id: 'p3', name: 'Next-build realm', url: 'https://play.example.org', logged_in: true, account: 'Alice', expires_at: Math.floor(Date.now() / 1000) + 3 * 86400, role: 'player', plain_http: 'https', plain_http_confirmed: false, keyring_error: null },
      ];
  const serverInfo = (id: string): ServerInfo => ({
    name: id === 'p1' ? 'Local Realm' : id === 'p2' ? 'LAN Realm' : 'Next Realm',
    version: id === 'p3' ? (two ? '0.6.0' : '0.7.0') : '0.5.5e',
    cluster_id: 'c', online: id === 'p1' ? 3 : 12, open_registration: id !== 'p3', login_host: '127.0.0.1', login_port: 20481,
    instance_port: 6112, gateway_region: 'Local', uptime_secs: id === 'p1' ? 7300 : 190000,
  });
  const versions = (): GameVersion[] => {
    const cur = settings.game_version ?? (two ? '0.6.0' : '0.5.5e');
    const priv = two ? ['0.6.0', '0.5.5e'] : ['0.5.5e'];
    return [
      { version: 'official', app_id: 2694490, official: true, selected: cur === 'official', newest: false, installed: installed.has('official') },
      ...priv.map((v, i) => ({ version: v, app_id: 2694490, official: false, selected: cur === v, newest: i === 0, installed: installed.has(v) })),
    ];
  };
  const current = () => versions().find((v) => v.selected)!;
  const status = (): InstallStatus => ({
    version: current().version, app_id: 2694490, install_dir: `/home/user/.local/share/poe2-launcher/game/${current().version}`,
    installed: current().installed, running: false, tool_version: '3.4.0', tool_ready: true,
    depots: [
      { id: 2694491, manifest: current().official ? '' : '7366357142270402355', role: 'content' },
      { id: 2694492, manifest: current().official ? '' : '246779027001700092', role: 'exe (Windows)' },
    ],
  });
  // The client hook: `?nohook` starts without one, `?hookupdate` pretends a newer release exists.
  const hookDir = '/home/user/.local/share/poe2-launcher/hook/packages';
  const hookPkg = (version: string) => ({ dir: `${hookDir}/${version}`, name: `poe-hook-v${version}-game-0.5.5e-windows-x86_64`, version, game_versions: ['0.5.5e'], mod_abi: 1, commit: 'abc1234' });
  let hookVersion: string | null = params.has('nohook') ? null : params.has('hookupdate') ? '0.1.0' : '0.2.0';
  const hookStatus = () => ({
    game_version: current().official ? '0.5.5e' : current().version, repo: 'Distorbed678/Poe2ClientHook',
    source: settings.dll_dir ? { kind: 'manual' } : hookVersion ? { kind: 'downloaded', version: hookVersion } : { kind: 'missing' },
    dll_dir: settings.dll_dir ?? (hookVersion ? `${hookDir}/${hookVersion}` : '/home/user/.local/share/poe2-launcher/hook-dlls'),
    manual_dir: settings.dll_dir, installed: hookVersion ? hookPkg(hookVersion) : null, other_installed: [], busy: false,
  });
  const hookRelease = () => ({ tag: 'hook-v0.2.0', version: '0.2.0', game_versions: ['0.5.5e'], prerelease: false, asset_name: 'poe-hook-v0.2.0-game-0.5.5e-windows-x86_64.zip', asset_url: 'https://example.invalid/a.zip', size: 5400000, sha256_url: 'https://example.invalid/a.sha256' });
  const info = (): AppInfo => ({
    launcher_version: '0.2.0', client_version: current().version, game_versions: versions(), os: 'linux',
    data_dir: '/home/user/.local/share/poe2-launcher', config_dir: '/home/user/.config/poe2-launcher',
  });

  mockIPC(
    (cmd, args) => {
      const a = (args ?? {}) as Record<string, unknown>;
      switch (cmd) {
        case 'app_info': return info();
        case 'game_version_set': settings = { ...settings, game_version: a.version as string }; return versions();
        case 'profiles_list': return profiles;
        case 'settings_get': return settings;
        case 'settings_set': settings = a.settings as Settings; return settings;
        case 'install_status': return status();
        case 'play_state': return { state: 'idle' };
        case 'server_info': return serverInfo(a.id as string);
        case 'account_refresh': return profiles.find((p) => p.id === a.id);
        case 'mods_list':
          return params.has('mods')
            ? [
                { id: 'core-lib', dir_name: 'core-lib', manifest: { id: 'core-lib', name: 'Core library', version: '1.2.0', description: 'Shared helpers', authors: [], license: 'MIT', homepage: '', game_versions: ['0.5.5e'], hook_api: 1, after: [], before: [], conflicts: [], dependencies: {}, entry_dll: null }, errors: [], unknown_keys: [], compat: { kind: 'ok' }, enabled: true, order: 0 },
                { id: 'old-mod', dir_name: 'old-mod', manifest: { id: 'old-mod', name: 'Old mod', version: '0.1.0', description: 'Made for another build', authors: [], license: 'MIT', homepage: '', game_versions: ['0.4.0'], hook_api: 1, after: [], before: [], conflicts: [], dependencies: { 'core-lib': '^1' }, entry_dll: null }, errors: [], unknown_keys: [], compat: { kind: 'wrong_game', supported: ['0.4.0'] }, enabled: false, order: null },
              ]
            : [];
        case 'mods_dir': return '/home/user/.local/share/poe2-launcher/game/0.5.5e/poe-hook-mods';
        case 'steam_login_status': return { remembered, account: remembered ? settings.steam_username : null, files: [] };
        case 'steam_logout': remembered = false; settings = { ...settings, steam_username: '', remember_steam_login: false }; return { remembered, account: null, files: [] };
        case 'desktop_entry_status': return { supported: true, installed: false, exec: null, stale: false, file: '/home/user/.local/share/applications/org.poe2modding.launcher.desktop', current_exec: '/home/user/Apps/Poe2-Launcher.AppImage', appimage: true };
        case 'hook_status': return hookStatus();
        case 'hook_check': return { release: hookRelease(), update_available: hookVersion !== '0.2.0' };
        case 'hook_install_latest': hookVersion = '0.2.0'; return { hook: hookPkg('0.2.0'), sha256: 'f'.repeat(64), source: 'download' };
        case 'host_status': return {
          settings: { server_name: 'My private realm', cluster_id: 'my-cluster', login_port: 20481, instance_port: 6112, web_port: 8080, public_host: '192.168.1.20', web_public: false, open_registration: true, admin_name: 'admin', autostart: false, package_dir: null, configured: false, profile_id: null },
          package: null, package_problem: null, platform: 'linux', game_version: current().version, game_installed: current().installed,
          install_dir: status().install_dir, run: { state: 'stopped' }, info: null, web_url: 'http://127.0.0.1:8080', lan_ip: '192.168.1.20',
          log_file: '/home/user/.local/share/poe2-launcher/server/server.log', busy: false, hosting_guide_url: 'https://example.invalid',
        };
        case 'host_log_tail': return [];
        case 'plugin:updater|check': return null;
        case 'plugin:dialog|open': return null;
        default: return null;
      }
    },
    { shouldMockEvents: true },
  );
}
