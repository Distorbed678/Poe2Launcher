/**
 * Development preview without the Tauri backend: `npm run dev`, then open `/?mock` in a browser.
 * It fakes the IPC (profiles, install state, settings, ...) so every page can be looked at and
 * screenshotted. Only loaded in development builds; `?versions=2` pretends the project supports a
 * second game version, `?official` starts in the official mode, `?empty` has no servers.
 * More switches: `?installing` (a running download with speed), `?paused`, `?lowspace`, `?offline`
 * (no news, no server list), `?nonews`, `?motd`, `?nolist`, `?nobackups`, `?nologs`, `?skipped`.
 */
import { mockIPC, mockWindows } from '@tauri-apps/api/mocks';
import { emit } from '@tauri-apps/api/event';
import type { AppInfo, BackupInfo, GameVersion, InstallStatus, ListedServer, LogFile, NewsItem, ProbeResult, ProfileView, ServerInfo, Settings } from './types';

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
    instance_port: 6112, gateway_region: 'Local',
    uptime_secs: id === 'p1' ? 7300 : 190000,
    motd: id === 'p1' && params.has('motd')
      ? 'Welcome to the **test realm**!\n\n- Maintenance on Saturday 18:00 UTC\n- New: the hideout is open, see [the changelog](https://example.org/changes)\n\n<script>alert(1)</script> stays text.'
      : null,
  });
  const pingOf = (id: string) => (id === 'p1' ? 4 : id === 'p2' ? 12 : 88);
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
      { id: 2694491, manifest: current().official ? '' : '7366357142270402355', role: 'content', est_bytes: 150_000_000_000 },
      { id: 2694492, manifest: current().official ? '' : '246779027001700092', role: 'exe (Windows)', est_bytes: 78_000_000 },
    ],
    required_bytes: 150_078_000_000,
    paused: params.has('paused') ? { version: current().version, steam_username: 'steam_user', remember_login: true } : null,
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
  // --- news, server list, logs, backups ---
  const now = Math.floor(Date.now() / 1000);
  const news: NewsItem[] = params.has('nonews') || params.has('offline') ? [] : [
    { id: 'welcome', source: 'project', date: '2026-10-22', title: 'Welcome to the launcher', body: 'This is the **project news**: announcements from the maintainers, shown here and nowhere else.\n\n- The server list is in *Servers > Browse*\n- Logs and backups are new\n\nSee [the guide](https://example.org/guide).', link: 'https://example.org/news/welcome' },
    { id: 'server-server-v0.9.0', source: 'server', version: '0.9.0', date: '2026-10-21T10:00:00Z', title: 'Server 0.9.0', body: '- Admin stop and restart from the web admin\n- The client pathfinder runs on the server\n- Fixes for the instance exit', link: 'https://example.org/releases/server-v0.9.0' },
    { id: 'launcher-v0.2.0', source: 'launcher', version: '0.2.0', date: '2026-10-20T10:00:00Z', title: 'Launcher 0.2.0', body: 'New look, game version dropdown, Official (read-only).', link: 'https://example.org/releases/v0.2.0' },
    { id: 'hook-hook-v0.2.0', source: 'hook', version: '0.2.0', date: '2026-10-19T10:00:00Z', title: 'Client hook 0.2.0', body: '', link: null },
  ];
  const listed: ListedServer[] = params.has('nolist') || params.has('offline') ? [] : [
    { id: 'aurora-pve', name: 'Aurora PvE', address: 'https://aurora.example.org', description: 'Relaxed co-op realm with weekly events. Beginners welcome.', region: 'EU', game_version: '0.5.5e', tags: ['pve', 'friendly'], contact: 'https://discord.gg/example', mods: [], plain_http: false },
    { id: 'ironforge', name: 'Ironforge Hardcore', address: 'https://ironforge.example.org', description: 'One life. Wipes each season.', region: 'NA', game_version: '0.5.5e', tags: ['hardcore', 'seasonal'], contact: 'ironforge@example.org', mods: [{ id: 'core-lib', version: '^1.2' }], plain_http: false },
    { id: 'night-owl', name: 'Night Owl', address: 'http://203.0.113.9:8080', description: 'Small friends realm, plain http.', region: 'Asia', game_version: '0.5.5e', tags: ['pvp'], contact: '', mods: [], plain_http: true },
    { id: 'old-build', name: 'Old Build Museum', address: 'https://old.example.org', description: 'Kept on an older client.', region: 'EU', game_version: '0.4.0', tags: [], contact: '', mods: [], plain_http: false },
    { id: 'gone', name: 'Gone Fishing', address: 'https://gone.example.org', description: 'Not answering.', region: 'EU', game_version: '0.5.5e', tags: ['pve'], contact: '', mods: [], plain_http: false },
  ];
  const probeOf = (s: ListedServer): ProbeResult => {
    if (s.id === 'gone') return { id: s.id, ok: false, ping_ms: null, info: null, error: 'cannot reach the server: timed out' };
    const online = ({ 'aurora-pve': 27, ironforge: 9, 'night-owl': 3, 'old-build': 1 } as Record<string, number>)[s.id] ?? 0;
    const ping = ({ 'aurora-pve': 38, ironforge: 112, 'night-owl': 241, 'old-build': 55 } as Record<string, number>)[s.id] ?? 100;
    return { id: s.id, ok: true, ping_ms: ping, info: { ...serverInfo('p3'), name: s.name, online, version: s.game_version, open_registration: s.id !== 'ironforge', motd: null }, error: null };
  };
  const logFiles: LogFile[] = params.has('nologs') ? [] : [
    { id: 'game:game-1761100000.log', kind: 'game', name: 'game-1761100000.log', size: 18_432, modified: now - 3600, text: true },
    { id: 'game:game-1761000000.log', kind: 'game', name: 'game-1761000000.log', size: 9_120, modified: now - 90_000, text: true },
    { id: 'server:server.log', kind: 'server', name: 'server.log', size: 4_800, modified: now - 600, text: true },
    { id: 'crash:crash-2026-10-20.dmp', kind: 'crash', name: 'crash-2026-10-20.dmp', size: 2_457_600, modified: now - 172_800, text: false },
  ];
  const logText = (id: string): string[] => {
    const out: string[] = [];
    for (let i = 0; i < 60; i++) out.push(`[${String(10 + (i >> 4)).padStart(2, '0')}:${String((i * 7) % 60).padStart(2, '0')}] ${id.startsWith('server') ? 'instance' : 'client'} tick ${i} ok`);
    out.splice(12, 0, '[10:03] WARN  slow frame: 48 ms');
    out.splice(30, 0, '[10:09] ERROR failed to load tile 0x2f81 (retrying)');
    out.splice(31, 0, '[10:09] login token for <account> accepted');
    return out;
  };
  let backups: BackupInfo[] = params.has('nobackups') ? [] : [
    { name: 'server-20261022-090000-auto.sqlite', kind: 'auto', created: now - 7200, size: 3_145_728 },
    { name: 'server-20261021-224500-manual.sqlite', kind: 'manual', created: now - 40_000, size: 3_092_480 },
    { name: 'server-20261020-180000-auto.sqlite', kind: 'auto', created: now - 150_000, size: 2_986_000 },
  ];
  let backupSettings = { auto: true, keep: 5 };

  // `?installing`: a download that shows speed and time left; the numbers come from events.
  if (params.has('installing')) {
    let pct = 4;
    setTimeout(() => {
      void emit('install:state', { state: 'fetching_tool' });
      void emit('install:state', { state: 'downloading' });
      void emit('install:log', { text: 'Downloading depot 2694491', stream: 'stdout' });
      setInterval(() => {
        pct = Math.min(99, pct + 0.9);
        void emit('install:progress', { phase: 'depot', depot: 2694491, percent: pct });
      }, 600);
    }, 900);
  }

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
        case 'server_status': return { info: serverInfo(a.id as string), ping_ms: pingOf(a.id as string) };
        case 'news_get': return { items: news, fetched_at: params.has('offline') ? 0 : now - 120, offline: params.has('offline') };
        case 'open_link': return null;
        case 'server_list_get': return { servers: listed, skipped: params.has('skipped') ? ['entry 3: id must be 3-48 characters of a-z, 0-9 and -'] : [], fetched_at: params.has('offline') ? 0 : now - 300, offline: params.has('offline'), error: params.has('offline') ? 'cannot reach the server list: connection refused or host not found' : '' };
        case 'server_list_probe': return listed.filter((x) => (a.ids as string[]).includes(x.id)).map(probeOf);
        case 'profile_add': profiles.push({ id: `p${profiles.length + 1}`, name: a.name as string, url: a.url as string, logged_in: false, account: null, expires_at: null, role: null, plain_http: String(a.url).startsWith('http://') ? 'public' : 'https', plain_http_confirmed: !!a.confirmPlainHttp, keyring_error: null }); return profiles.at(-1);
        case 'account_login': { const p = profiles.find((x) => x.id === a.id)!; p.logged_in = true; p.account = a.username as string; p.role = 'player'; p.expires_at = now + 6 * 86400; return p; }
        case 'account_logout': { const p = profiles.find((x) => x.id === a.id)!; p.logged_in = false; return { revoked: true, message: 'logged out; the token was revoked' }; }
        case 'install_pause': void emit('install:state', { state: 'paused' }); return null;
        case 'install_resume': void emit('install:state', { state: 'fetching_tool' }); return null;
        case 'install_space': return params.has('lowspace')
          ? { path: status().install_dir, required: 174_000_000_000, present: 0, missing: 174_000_000_000, free: 62_000_000_000, ok: false, message: `Not enough free space on the disk of ${status().install_dir}: about 174.0 GB still have to be downloaded, 62.0 GB are free. Free up at least 112.0 GB or choose another install folder in Settings.` }
          : { path: status().install_dir, required: 174_000_000_000, present: 0, missing: 174_000_000_000, free: 640_000_000_000, ok: true, message: `About 174.0 GB to download, 640.0 GB free on the disk of ${status().install_dir}.` };
        case 'log_list': return { files: logFiles, game_dir: '/home/user/.local/share/poe2-launcher/logs', server_dir: '/home/user/.local/share/poe2-launcher/server', crash_dir: '/home/user/.local/share/poe2-launcher/crash-dumps' };
        case 'log_read': { const f = logFiles.find((x) => x.id === a.id); return { id: a.id, lines: f?.text ? logText(a.id as string) : [], truncated: false, size: f?.size ?? 0, binary: !f?.text }; }
        case 'log_report': return `Poe2 Launcher bug report\nlauncher: 0.2.0\nos: linux x86_64\ngame version: ${current().version} (installed)\nclient hook: 0.2.0 (mod ABI 1, commit abc1234)\nserver package: none\n\n--- ${a.id ?? 'no log'}: last 5 lines ---\n${logText(String(a.id)).slice(-5).join('\n')}`;
        case 'log_open_folder': return null;
        case 'backup_status': return { dir: '/home/user/.local/share/poe2-launcher/server/backups', backups, settings: backupSettings, database_size: 3_145_728, server_running: false };
        case 'backup_create': { const b: BackupInfo = { name: `server-${now}-manual.sqlite`, kind: 'manual', created: now, size: 3_150_000 }; backups = [b, ...backups]; return b; }
        case 'backup_restore': return { name: 'server-pre.sqlite', kind: 'pre-restore', created: now, size: 3_150_000 };
        case 'backup_delete': backups = backups.filter((b) => b.name !== a.name); return null;
        case 'backup_set_settings': backupSettings = { auto: a.auto as boolean, keep: a.keep as number }; return backupSettings;
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
