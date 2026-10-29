/**
 * Development preview without the Tauri backend: `npm run dev`, then open `/?mock` in a browser.
 * It fakes the IPC (profiles, install state, settings, ...) so every page can be looked at and
 * screenshotted. Only loaded in development builds; `?versions=2` pretends the project supports a
 * second game version, `?official` starts in the official mode, `?empty` has no servers.
 * More switches: `?installing` (a running download with speed), `?paused`, `?lowspace`, `?offline`
 * (no news, no server list), `?nonews`, `?motd`, `?nolist`, `?nobackups`, `?nologs`, `?skipped`;
 * hosting: `?hostpkg` (a server is installed and set up; `&hosts=N` for N servers), `?serverupdate` (the same, and a newer server release is on offer; `&serverrunning` runs it); mods: `?mods` (a cache with mods), `?modsync[=off|auto]` (server p1 requires two mods; Ask dialog at Play), `?offline` also affects the mod index.
 */
import { mockIPC, mockWindows } from '@tauri-apps/api/mocks';
import { emit } from '@tauri-apps/api/event';
import type { ModEntry, ModIndex, ModManifest, ModsView, SyncMod, SyncPlan, AppInfo, BackupInfo, GameVersion, InstallStatus, ListedServer, LogFile, NewsItem, ProbeResult, ProfileView, ServerInfo, Settings } from './types';

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
    mod_sync: params.get('modsync') === 'off' || params.get('modsync') === 'auto' ? (params.get('modsync') as 'off' | 'auto') : 'ask',
  };
  let remembered = true;
  const installed = new Set<string>(params.has('fresh') ? [] : (params.get('installed') ?? '0.5.5e.hotfix1').split(','));
  const profiles: ProfileView[] = empty
    ? []
    : [
        { id: 'p1', name: 'Local test realm', url: 'http://127.0.0.1:8080', logged_in: true, account: 'Alice', expires_at: Math.floor(Date.now() / 1000) + 6 * 86400, role: 'admin', plain_http: 'loopback', plain_http_confirmed: false, keyring_error: null },
        { id: 'p2', name: 'Friends LAN party', url: 'http://192.168.1.20:8080', logged_in: false, account: null, expires_at: null, role: null, plain_http: 'private', plain_http_confirmed: true, keyring_error: null },
        { id: 'p3', name: 'Next-build realm', url: 'https://play.example.org', logged_in: true, account: 'Alice', expires_at: Math.floor(Date.now() / 1000) + 3 * 86400, role: 'player', plain_http: 'https', plain_http_confirmed: false, keyring_error: null },
      ];
  const serverInfo = (id: string): ServerInfo => ({
    name: id === 'p1' ? 'Local Realm' : id === 'p2' ? 'LAN Realm' : 'Next Realm',
    version: id === 'p3' ? (two ? '0.6.0' : '0.7.0') : '0.5.5e.hotfix1',
    cluster_id: 'c', online: id === 'p1' ? 3 : 12, open_registration: id !== 'p3', login_host: '127.0.0.1', login_port: 20481,
    instance_port: 6112, gateway_region: 'Local',
    uptime_secs: id === 'p1' ? 7300 : 190000,
    motd: id === 'p1' && params.has('motd')
      ? 'Welcome to the **test realm**!\n\n- Maintenance on Saturday 18:00 UTC\n- New: the hideout is open, see [the changelog](https://example.org/changes)\n\n<script>alert(1)</script> stays text.'
      : null,
    mods: id === 'p1' && params.has('modsync') ? { mods: advertised(), skipped: [] } : { mods: [], skipped: [] },
  });
  const pingOf = (id: string) => (id === 'p1' ? 4 : id === 'p2' ? 12 : 88);
  const versions = (): GameVersion[] => {
    const cur = settings.game_version ?? (two ? '0.6.0' : '0.5.5e.hotfix1');
    const priv = two ? ['0.6.0', '0.5.5e.hotfix1'] : ['0.5.5e.hotfix1'];
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
      { id: 2694491, manifest: current().official ? '' : '4712263798014036045', role: 'content', est_bytes: 150_000_000_000 },
      { id: 2694492, manifest: current().official ? '' : '4973272861086296374', role: 'exe (Windows)', est_bytes: 78_000_000 },
    ],
    required_bytes: 150_078_000_000,
    paused: params.has('paused') ? { version: current().version, steam_username: 'steam_user', remember_login: true } : null,
  });
  // The client hook: `?nohook` starts without one, `?hookupdate` pretends a newer release exists.
  const hookDir = '/home/user/.local/share/poe2-launcher/hook/packages';
  const hookPkg = (version: string) => ({ dir: `${hookDir}/${version}`, name: `poe-hook-v${version}-game-0.5.5e.hotfix1-windows-x86_64`, version, game_versions: ['0.5.5e.hotfix1'], mod_abi: 1, commit: 'abc1234' });
  // The hosted server: installed version (null: none) and whether it runs.
  let hostVersion: string | null = params.has('hostpkg') || params.has('serverupdate') ? '0.2.0' : null;
  let hostRunning = params.has('serverrunning');
  // `?hosts=N`: N servers on this computer (default 1 when a package is installed).
  let hostIds: string[] = Array.from({ length: Number(params.get('hosts') ?? (hostVersion ? 1 : 0)) }, (_, i) => `s${i + 1}`);
  const hostNames: Record<string, string> = { s1: 'Weekend realm', s2: 'Test cluster', s3: 'Hardcore' };
  const hostStatus = (id: string) => {
    const n = Number(id.slice(1)) - 1;
    const running = hostRunning && id === 's1';
    const configured = hostVersion !== null && (id === 's1' || id === 's2');
    return {
      id,
      settings: { server_name: hostNames[id] ?? `Server ${n + 1}`, cluster_id: 'main', login_port: 20481 + 10 * n, instance_port: 6112 + 10 * n, web_port: 8080 + 10 * n, public_host: '192.168.1.20', web_public: false, open_registration: true, autostart: id === 's1', package_dir: hostVersion ? hostPkg(hostVersion).dir : null, configured, profile_id: configured && id === 's1' ? 'p1' : null },
      package: hostVersion ? hostPkg(hostVersion) : null, package_problem: null, platform: 'linux', game_version: current().version, game_installed: current().installed,
      install_dir: status().install_dir, run: running ? { state: 'running', pid: 4242 } : { state: 'stopped' },
      info: running ? { name: hostNames[id], version: current().version, cluster_id: 'main', online: 2, open_registration: true, login_host: '192.168.1.20', login_port: 20481, instance_port: 6112, gateway_region: null, uptime_secs: 11_520, motd: null, mods: [] } : null,
      web_url: `http://127.0.0.1:${8080 + 10 * n}`, lan_ip: '192.168.1.20',
      log_file: `/home/user/.local/share/poe2-launcher/servers/${id}/server.log`, folder: `/home/user/.local/share/poe2-launcher/servers/${id}`, busy: false, hosting_guide_url: 'https://example.invalid',
    };
  };
  const hostPkg = (v: string) => ({ dir: `/home/user/.local/share/poe2-launcher/server/packages/poe-server-v${v}-game-0.5.5e.hotfix1-linux-x86_64`, name: `poe-server-v${v}-game-0.5.5e.hotfix1-linux-x86_64`, server_version: v, game_version: '0.5.5e.hotfix1', platform: 'linux', commit: 'abc1234' });
  const hostRelease = () => ({
    repo: 'Distorbed678/Poe2Server', tag: 'server-v0.3.0', version: '0.3.0', game_version: '0.5.5e.hotfix1', prerelease: false,
    asset_name: 'poe-server-v0.3.0-game-0.5.5e.hotfix1-linux-x86_64.tar.gz', asset_url: 'https://example.invalid/a', size: 41_000_000, sha256_url: 'https://example.invalid/a.sha256',
    notes: '## Server 0.3.0\n\n- Terrain for more areas\n- **New:** `[web] cors` setting\n- Fixes a crash when a player leaves during a transfer',
    html_url: 'https://github.com/Distorbed678/Poe2Server/releases/tag/server-v0.3.0',
  });
  let hookVersion: string | null = params.has('nohook') ? null : params.has('hookupdate') ? '0.1.0' : '0.2.0';
  const hookStatus = () => ({
    game_version: current().official ? '0.5.5e.hotfix1' : current().version, repo: 'Distorbed678/Poe2ClientHook',
    source: settings.dll_dir ? { kind: 'manual' } : hookVersion ? { kind: 'downloaded', version: hookVersion } : { kind: 'missing' },
    dll_dir: settings.dll_dir ?? (hookVersion ? `${hookDir}/${hookVersion}` : '/home/user/.local/share/poe2-launcher/hook-dlls'),
    manual_dir: settings.dll_dir, installed: hookVersion ? hookPkg(hookVersion) : null, other_installed: [], busy: false,
  });
  const hookRelease = () => ({ tag: 'hook-v0.2.0', version: '0.2.0', game_versions: ['0.5.5e.hotfix1'], prerelease: false, asset_name: 'poe-hook-v0.2.0-game-0.5.5e.hotfix1-windows-x86_64.zip', asset_url: 'https://example.invalid/a.zip', size: 5400000, sha256_url: 'https://example.invalid/a.sha256' });
  // --- news, server list, logs, backups ---
  const now = Math.floor(Date.now() / 1000);
  const news: NewsItem[] = params.has('nonews') || params.has('offline') ? [] : [
    { id: 'welcome', source: 'project', date: '2026-10-22', title: 'Welcome to the launcher', body: 'This is the **project news**: announcements from the maintainers, shown here and nowhere else.\n\n- The server list is in *Servers > Browse*\n- Logs and backups are new\n\nSee [the guide](https://example.org/guide).', link: 'https://example.org/news/welcome' },
    { id: 'server-server-v0.9.0', source: 'server', version: '0.9.0', date: '2026-10-21T10:00:00Z', title: 'Server 0.9.0', body: '- Admin stop and restart from the web admin\n- The client pathfinder runs on the server\n- Fixes for the instance exit', link: 'https://example.org/releases/server-v0.9.0' },
    { id: 'launcher-v0.2.0', source: 'launcher', version: '0.2.0', date: '2026-10-20T10:00:00Z', title: 'Launcher 0.2.0', body: 'New look, game version dropdown, Official (read-only).', link: 'https://example.org/releases/v0.2.0' },
    { id: 'hook-hook-v0.2.0', source: 'hook', version: '0.2.0', date: '2026-10-19T10:00:00Z', title: 'Client hook 0.2.0', body: '', link: null },
  ];
  const listed: ListedServer[] = params.has('nolist') || params.has('offline') ? [] : [
    { id: 'aurora-pve', name: 'Aurora PvE', address: 'https://aurora.example.org', description: 'Relaxed co-op realm with weekly events. Beginners welcome.', region: 'EU', game_version: '0.5.5e.hotfix1', tags: ['pve', 'friendly'], contact: 'https://discord.gg/example', mods: [], plain_http: false },
    { id: 'ironforge', name: 'Ironforge Hardcore', address: 'https://ironforge.example.org', description: 'One life. Wipes each season.', region: 'NA', game_version: '0.5.5e.hotfix1', tags: ['hardcore', 'seasonal'], contact: 'ironforge@example.org', mods: [{ id: 'core-lib', version: '^1.2' }], plain_http: false },
    { id: 'night-owl', name: 'Night Owl', address: 'http://203.0.113.9:8080', description: 'Small friends realm, plain http.', region: 'Asia', game_version: '0.5.5e.hotfix1', tags: ['pvp'], contact: '', mods: [], plain_http: true },
    { id: 'old-build', name: 'Old Build Museum', address: 'https://old.example.org', description: 'Kept on an older client.', region: 'EU', game_version: '0.4.0', tags: [], contact: '', mods: [], plain_http: false },
    { id: 'gone', name: 'Gone Fishing', address: 'https://gone.example.org', description: 'Not answering.', region: 'EU', game_version: '0.5.5e.hotfix1', tags: ['pve'], contact: '', mods: [], plain_http: false },
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


  // --- mods: a cache, pins per scope, an advertisement for p1 (`?modsync`), the index (`?mods`) ---
  const cache: ModEntry[] = [];
  const pins: Record<string, { id: string; version: string }[]> = {};
  let syncDone = params.get('modsync') === 'auto';
  const manifestOf = (id: string, version: string, side: 'client' | 'both', code: boolean): ModManifest => ({
    id, name: id.split('-').map((w) => w[0].toUpperCase() + w.slice(1)).join(' '), version, description: `The ${id} mod`, authors: ['Ann'], license: 'MIT', homepage: '', side,
    game_versions: ['0.5.5e.hotfix1'], hook_api: code ? 1 : null, server_api: side === 'both' ? 1 : null, after: [], before: [], conflicts: [], dependencies: {}, entry_dll: code ? `${id}.dll` : null, has_code: code,
  });
  function entry(id: string, version: string, o: Partial<ModEntry> = {}): ModEntry {
    const side = o.side === 'both' ? 'both' : 'client';
    const code = o.has_code ?? true;
    return { id, version, side, manifest: manifestOf(id, version, side, code), errors: [], unknown_keys: [], compat: { kind: 'ok' }, compat_text: 'compatible', sha256: 'a'.repeat(64), size: 48_210, source: 'zip', added: now, has_code: code, server_half_left_out: side === 'both', enabled: false, enabled_all: false, required: false, in_effect: false, origin: null, order: null, ...o };
  }
  if (params.has('mods') || params.has('modsync')) {
    cache.push(entry('hud-tweaks', '1.2.0', { source: 'index' }), entry('hud-tweaks', '1.1.0', { source: 'index' }), entry('old-ui', '0.3.0', { compat: { kind: 'wrong_game', supported: ['0.4.0'] }, compat_text: 'made for 0.4.0' }), entry('loot-filter', '2.0.0', { has_code: false, source: 'folder' }));
    pins.all = [{ id: 'loot-filter', version: '2.0.0' }];
    pins.p1 = [{ id: 'hud-tweaks', version: '1.2.0' }];
  }
  const advertised = () => [
    { id: 'faster-rolls', version: '1.0.0', side: 'both', sha256: 'c'.repeat(64), size: 48_210, required: true, hook_api: 1, game_versions: ['0.5.5e.hotfix1'], sources: [{ type: 'server', path: '/api/v1/mods/faster-rolls/1.0.0.zip' }], name: 'Faster rolls', authors: ['Ann'], description: 'Rolls resolve 1.5x faster on this realm.' },
    { id: 'realm-skin', version: '0.4.1', side: 'client', sha256: 'd'.repeat(64), size: 1_240_000, required: true, game_versions: ['0.5.5e.hotfix1'], name: 'Realm skin', authors: ['Bo'], description: 'Replaces the login background.' },
  ];
  function syncPlan(profileId: string): SyncPlan {
    const p = profiles.find((x) => x.id === profileId);
    const mods: SyncMod[] = profileId === 'p1' && params.has('modsync')
      ? advertised().map((m) => ({ id: m.id, version: m.version, name: m.name, side: m.side as 'client' | 'both', size: m.size, sha256: m.sha256, required: true, code: m.hook_api === 1, authors: m.authors, description: m.description, state: syncDone ? 'ready' : 'missing', detail: syncDone ? 'downloaded and verified' : 'not downloaded yet' }))
      : [];
    const missing = mods.some((m) => m.state === 'missing');
    const mode = settings.mod_sync;
    const status = !missing ? 'ok' : mode === 'off' ? 'off' : mode === 'auto' ? 'auto' : 'ask';
    return { profile_id: profileId, profile_name: p?.name ?? '', mode, status, mods, blocker: status === 'off' ? `${p?.name} requires mods you do not have: Faster rolls 1.0.0, Realm skin 0.4.1. Mod sync is Off, so nothing is downloaded` : null, download_bytes: missing ? mods.reduce((n, m) => n + m.size, 0) : 0, skipped: [], stale: false, applied: [], errors: [] };
  }
  function modsView(scope: string): ModsView {
    const sp = pins[scope] ?? [];
    const req = scope === 'p1' ? syncPlan('p1').mods : [];
    const entries = cache.map((e, i) => {
      const enabled = sp.some((x) => x.id === e.id && x.version === e.version);
      const enabledAll = (pins.all ?? []).some((x) => x.id === e.id && x.version === e.version);
      const inEffect = (enabled || enabledAll) && e.compat?.kind === 'ok';
      return { ...e, enabled, enabled_all: enabledAll, in_effect: inEffect, origin: inEffect ? (enabled ? 'profile' : 'global') : null, order: inEffect ? i : null } as ModEntry;
    });
    if (scope === 'p1' && syncDone) {
      for (const [i, m] of req.entries()) entries.unshift(entry(m.id, m.version, { side: m.side as 'both' | 'client', has_code: m.code, source: 'server', required: true, in_effect: true, origin: 'required', order: i, enabled: false }));
    }
    return { entries, scope, scope_name: scope === 'all' ? 'All servers' : (profiles.find((p) => p.id === scope)?.name ?? ''), required: req, dropped: [], problems: [], cache_dir: '/home/user/.local/share/poe2-launcher/mods/cache', cache_bytes: cache.reduce((n, m) => n + m.size, 0), game_version: current().version, hook_abi: 1, mod_sync: settings.mod_sync };
  }
  function modIndex(): ModIndex {
    const v = (version: string, o: object = {}) => ({ version, released: '2026-10-2' + version.length, game_versions: ['0.5.5e.hotfix1'], hook_api: 1, server_api: null, sha256: 'b'.repeat(64), size: 52_000, url: 'https://github.com/example/m/releases/download/x.zip', yanked: false, compat: { kind: 'ok' as const }, cached: false, ...o });
    const m = (id: string, name: string, side: 'client' | 'both' | 'server', description: string, tags: string[], versions: ReturnType<typeof v>[]) => ({ id, name, description, author: 'Ann', side, homepage: 'https://example.org/' + id, licence: 'MIT', tags, versions });
    if (params.has('offline')) return { mods: [m('hud-tweaks', 'HUD tweaks', 'client', 'Moves and resizes parts of the HUD.', ['ui'], [v('1.2.0', { cached: true })])], skipped: [], updated: '', fetched_at: now - 86_400, offline: true, error: 'cannot reach the mod index: connection refused or host not found' };
    return { mods: [
      m('faster-rolls', 'Faster rolls', 'both', 'Rolls resolve 1.5x faster. Needs the server half on the realm.', ['gameplay', 'qol'], [v('1.0.0', { server_api: 1 })]),
      m('hud-tweaks', 'HUD tweaks', 'client', 'Moves and resizes parts of the HUD, with a layout editor.', ['ui'], [v('1.2.0', { cached: true }), v('1.1.0', { cached: true }), v('1.0.0')]),
      m('loot-filter', 'Loot filter', 'client', 'A data-only loot highlight list, no code.', ['qol', 'loot'], [v('2.0.0', { hook_api: null, cached: true })]),
      m('old-ui', 'Old UI', 'client', 'Made for an older client build.', ['ui'], [v('0.3.0', { game_versions: ['0.4.0'], compat: { kind: 'wrong_game', supported: ['0.4.0'] } })]),
      m('admin-tools', 'Admin tools', 'server', 'Server commands for moderators (a server mod).', ['admin'], [v('0.9.0', { hook_api: null, server_api: 1 })]),
    ], skipped: params.has('skipped') ? ['entry 5: id must match [a-z0-9][a-z0-9_-]{1,63}'] : [], updated: '2026-10-22T00:00:00Z', fetched_at: now - 300, offline: false, error: '' };
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
        case 'backup_status': return { dir: '/home/user/.local/share/poe2-launcher/servers/s1/backups', backups, settings: backupSettings, database_size: 3_145_728, server_running: false };
        case 'backup_create': { const b: BackupInfo = { name: `server-${now}-manual.sqlite`, kind: 'manual', created: now, size: 3_150_000 }; backups = [b, ...backups]; return b; }
        case 'backup_restore': return { name: 'server-pre.sqlite', kind: 'pre-restore', created: now, size: 3_150_000 };
        case 'backup_delete': backups = backups.filter((b) => b.name !== a.name); return null;
        case 'backup_set_settings': backupSettings = { auto: a.auto as boolean, keep: a.keep as number }; return backupSettings;
        case 'account_refresh': return profiles.find((p) => p.id === a.id);
        case 'mods_list': return modsView(a.scope as string);
        case 'mod_install': { cache.push(entry('local-mod', '0.1.0', { source: 'zip' })); return { id: 'local-mod', version: '0.1.0', side: 'client', sha256: 'f'.repeat(64), size: 2048, already_cached: false, has_code: true, server_half_left_out: false, unknown_keys: [], warnings: ['no .sha256 file next to the package: its identity is not pinned'] }; }
        case 'mod_set_enabled': {
          const scope = a.scope as string;
          const list = (pins[scope] ??= []).filter((x) => x.id !== a.id);
          if (a.enabled) list.push({ id: a.id as string, version: a.version as string });
          pins[scope] = list;
          return [];
        }
        case 'mod_move': case 'mod_auto_order': return [];
        case 'mod_remove': { const i = cache.findIndex((m) => m.id === a.id && m.version === a.version); if (i >= 0) cache.splice(i, 1); return null; }
        case 'mods_dir': return '/home/user/.local/share/poe2-launcher/mods/cache';
        case 'mod_cache_status': return { dir: '/home/user/.local/share/poe2-launcher/mods/cache', entries: cache.length, bytes: cache.reduce((n, m) => n + m.size, 0), unused_entries: 1, unused_bytes: 48_210 };
        case 'mod_cache_prune': return { removed: 1, bytes: 48_210 };
        case 'mod_sync_check': return syncPlan(a.profileId as string);
        case 'mod_sync_apply': { syncDone = true; const pl = syncPlan(a.profileId as string); pl.applied = ['Faster rolls 1.0.0']; return pl; }
        case 'mod_index_get': return modIndex();
        case 'mod_index_install': { cache.push(entry(a.id as string, a.version as string, { source: 'index' })); return { id: a.id, version: a.version, side: 'client', sha256: 'e'.repeat(64), size: 9000, already_cached: false, has_code: true, server_half_left_out: false, unknown_keys: [], warnings: [] }; }
        case 'steam_login_status': return { remembered, account: remembered ? settings.steam_username : null, files: [] };
        case 'steam_logout': remembered = false; settings = { ...settings, steam_username: '', remember_steam_login: false }; return { remembered, account: null, files: [] };
        case 'desktop_entry_status': return { supported: true, installed: false, exec: null, stale: false, file: '/home/user/.local/share/applications/org.poe2modding.launcher.desktop', current_exec: '/home/user/Apps/Poe2-Launcher.AppImage', appimage: true };
        case 'hook_status': return hookStatus();
        case 'hook_check': return { release: hookRelease(), update_available: hookVersion !== '0.2.0' };
        case 'hook_install_latest': hookVersion = '0.2.0'; return { hook: hookPkg('0.2.0'), sha256: 'f'.repeat(64), source: 'download' };
        case 'host_list': return hostIds.map(hostStatus);
        case 'host_status': return hostStatus(a.id as string);
        case 'host_create': { const id = `s${hostIds.length + 1}`; hostIds = [...hostIds, id]; hostNames[id] = a.name as string; return id; }
        case 'host_delete': hostIds = hostIds.filter((x) => x !== a.id); return null;
        case 'host_metrics': return hostRunning && a.id === 's1'
          ? { cpu_pct: 6.2, rss_bytes: 431_000_000, cpu_history: [3, 4, 3, 6, 5, 8, 7, 6, 9, 6], rss_history: [400e6, 405e6, 410e6, 415e6, 420e6, 431e6], tick_avg_ms: 4.1, tick_max_ms: 9.8, net_in_bps: 12_300, net_out_bps: 26_400, players: 2, uptime_secs: 11_520, server_metrics: true }
          : { cpu_pct: null, rss_bytes: null, cpu_history: [], rss_history: [], tick_avg_ms: null, tick_max_ms: null, net_in_bps: null, net_out_bps: null, players: null, uptime_secs: null, server_metrics: false };
        case 'host_setup': return { profile: profiles[0], config_file: '/home/user/.local/share/poe2-launcher/servers/s1/server.toml' };
        case 'host_web_admin_open': case 'register_page_open': return null;
        case 'host_update_check': return { installed: hostVersion, release: hostVersion ? hostRelease() : null, update_available: hostVersion !== null && hostVersion !== '0.3.0' && params.has('serverupdate'), error: null, checked_at: now };
        case 'host_update_apply': { const from = hostVersion ?? '0.2.0'; hostVersion = '0.3.0'; return { updated: true, from, to: '0.3.0', package: hostPkg('0.3.0'), was_running: hostRunning, restarted: hostRunning, backup: 'server-20261022-134501-pre-update.sqlite', sha256: 'e'.repeat(64) }; }
        case 'host_log_tail': return hostRunning && a.id === 's1' ? ['14:05:02 INFO login server listening addr=0.0.0.0:20481', '14:05:02 INFO instance server listening addr=0.0.0.0:6112', '[launcher] backup before start: server-20261030-140501-auto.sqlite (3.1 MB)', '14:06:40 INFO Distorbed logged in', '14:21:13 WARN slow frame 41 ms (area G1_2)', '14:22:01 INFO generated terrain for G1_3 in 812 ms'] : [];
        case 'plugin:updater|check': return null;
        case 'plugin:dialog|open': return null;
        default: return null;
      }
    },
    { shouldMockEvents: true },
  );
}
