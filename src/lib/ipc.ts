import { invoke } from '@tauri-apps/api/core';
import type {
  AppInfo, BackupInfo, BackupSettings, BackupStatus, DesktopEntry, LogContent, LogKind, LogList, NewsFeed, ProbeResult, ServerList, ServerStatus, SpaceCheck, GameVersion, HookCheck, HookOutcome, HookStatus, HostMetrics, HostSettings, HostStatus, InstallOutcome, InstallRequest, InstallStatus,
  AddedMod, CacheStatus, ModIndex, ModsView, Pruned, SyncPlan, LogoutResult, PlayState, ProfileView, ServerInfo, ServerRelease, Settings, SetupRequest,
  SetupResult, SteamLogin, VerifyReport, ServerUpdateCheck, UpdateOutcome,
} from './types';

/** An error from the backend: always carries a readable message. */
export class IpcError extends Error {}

export function errorText(e: unknown): string {
  if (typeof e === 'string') return e;
  if (e && typeof e === 'object' && 'message' in e && typeof e.message === 'string') return e.message;
  try {
    return JSON.stringify(e);
  } catch {
    return String(e);
  }
}

/** Calls a Tauri command; the backend rejects with a string. Argument keys are camelCase. */
export async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
  } catch (e) {
    throw new IpcError(errorText(e));
  }
}

export const ipc = {
  appInfo: () => call<AppInfo>('app_info'),
  /** Picks the game version (or `official`); everything version-dependent follows. Refused while busy. */
  setGameVersion: (version: string) => call<GameVersion[]>('game_version_set', { version }),
  profiles: () => call<ProfileView[]>('profiles_list'),
  profileAdd: (name: string, url: string, confirmPlainHttp = false) =>
    call<ProfileView>('profile_add', { name, url, confirmPlainHttp }),
  profileUpdate: (id: string, name: string, url: string, confirmPlainHttp = false) =>
    call<ProfileView>('profile_update', { id, name, url, confirmPlainHttp }),
  profileRemove: (id: string) => call<void>('profile_remove', { id }),
  /** Agree (or withdraw) to send the password and token over plain http to this profile's address. */
  confirmPlainHttp: (id: string, ok: boolean) => call<ProfileView>('profile_confirm_plain_http', { id, ok }),
  /** Re-reads the account's role from the server (`GET /me`). */
  accountRefresh: (id: string) => call<ProfileView>('account_refresh', { id }),
  /** Admin only: opens the server's web admin in the system browser, already signed in. */
  webAdminOpen: (id: string) => call<void>('web_admin_open', { id }),
  registerPageOpen: (id: string) => call<void>('register_page_open', { id }),
  hostWebAdminOpen: (id: string) => call<void>('host_web_admin_open', { id }),
  serverInfo: (id: string) => call<ServerInfo>('server_info', { id }),
  /** `GET /server` plus the round-trip time measured by the launcher. */
  serverStatus: (id: string) => call<ServerStatus>('server_status', { id }),
  /** Release notes of the project's repositories and `news.json`, cached; never fails. */
  newsGet: (force = false) => call<NewsFeed>('news_get', { force }),
  /** The public server list (cached). */
  serverListGet: (force = false) => call<ServerList>('server_list_get', { force }),
  /** Asks the listed servers with these ids for their live state, in parallel. */
  serverListProbe: (ids: string[]) => call<ProbeResult[]>('server_list_probe', { ids }),
  /** Opens an https link in the system browser. */
  openLink: (url: string) => call<void>('open_link', { url }),
  logList: () => call<LogList>('log_list'),
  logRead: (id: string, lines = 2000) => call<LogContent>('log_read', { id, lines }),
  /** The redacted text for a bug report (system information + the end of one log). */
  logReport: (id: string | null, lines = 200) => call<string>('log_report', { id, lines }),
  logOpenFolder: (kind: LogKind) => call<void>('log_open_folder', { kind }),
  backupStatus: (id: string) => call<BackupStatus>('backup_status', { id }),
  backupCreate: (id: string) => call<BackupInfo>('backup_create', { id }),
  backupRestore: (id: string, name: string) => call<BackupInfo | null>('backup_restore', { id, name }),
  backupDelete: (id: string, name: string) => call<void>('backup_delete', { id, name }),
  backupSetSettings: (id: string, auto: boolean, keep: number) => call<BackupSettings>('backup_set_settings', { id, auto, keep }),
  login: (id: string, username: string, password: string) => call<ProfileView>('account_login', { id, username, password }),
  logout: (id: string) => call<LogoutResult>('account_logout', { id }),
  settings: () => call<Settings>('settings_get'),
  setSettings: (settings: Settings) => call<Settings>('settings_set', { settings }),
  installStatus: () => call<InstallStatus>('install_status'),
  installStart: (request: InstallRequest) => call<void>('install_start', { request }),
  installInput: (text: string) => call<void>('install_input', { text }),
  installCancel: () => call<void>('install_cancel'),
  installPause: () => call<void>('install_pause'),
  /** Continues a paused download; the password is only needed when no Steam login is remembered. */
  installResume: (steamPassword: string) => call<void>('install_resume', { steamPassword }),
  installSpace: () => call<SpaceCheck>('install_space'),
  installVerify: () => call<VerifyReport>('install_verify'),
  /** The mod cache with its state in `scope` (`all` or a server profile id). */
  mods: (scope: string) => call<ModsView>('mods_list', { scope }),
  /** Adds a local package zip or mod folder to the cache; with `scope`, enables it there. */
  modInstall: (path: string, scope: string | null, replace: boolean) => call<AddedMod>('mod_install', { path, scope, replace }),
  /** Enables (one version per mod) or disables a cached mod in a scope; the answer lists own mods the set leaves out. */
  modSetEnabled: (scope: string, id: string, version: string, enabled: boolean, force = false) =>
    call<string[]>('mod_set_enabled', { scope, id, version, enabled, force }),
  modMove: (scope: string, id: string, position: number) => call<string[]>('mod_move', { scope, id, position }),
  modAutoOrder: (scope: string) => call<string[]>('mod_auto_order', { scope }),
  /** Deletes a cached version and takes it out of every mod set. */
  modRemove: (id: string, version: string) => call<void>('mod_remove', { id, version }),
  modsDir: () => call<string>('mods_dir'),
  modCacheStatus: () => call<CacheStatus>('mod_cache_status'),
  /** "Clear unused cached mods". */
  modCachePrune: () => call<Pruned>('mod_cache_prune'),
  /** Asks the server what it requires and compares it with the cache (the Auto mode also downloads). */
  modSyncCheck: (profileId: string) => call<SyncPlan>('mod_sync_check', { profileId }),
  /** The Download button: fetches the required mods (or just `ids`), verified; nothing is kept on a mismatch. */
  modSyncApply: (profileId: string, ids: string[] | null = null) => call<SyncPlan>('mod_sync_apply', { profileId, ids }),
  /** The mod browser's index (cached; offline gives the saved copy). */
  modIndexGet: (force = false) => call<ModIndex>('mod_index_get', { force }),
  modIndexInstall: (id: string, version: string, scope: string | null) => call<AddedMod>('mod_index_install', { id, version, scope }),
  /** Private versions: the server profile; the official mode takes none (the game's own Steam login). */
  play: (profileId: string | null) => call<void>('play', { profileId }),
  steamLogin: () => call<SteamLogin>('steam_login_status'),
  steamLogout: () => call<SteamLogin>('steam_logout'),
  desktopEntry: () => call<DesktopEntry>('desktop_entry_status'),
  desktopEntryInstall: () => call<DesktopEntry>('desktop_entry_install'),
  desktopEntryUninstall: () => call<DesktopEntry>('desktop_entry_uninstall'),
  playState: () => call<PlayState>('play_state'),
  hookStatus: () => call<HookStatus>('hook_status'),
  /** Asks GitHub for the newest hook release for the selected game version. */
  hookCheck: () => call<HookCheck>('hook_check'),
  hookInstallLatest: () => call<HookOutcome>('hook_install_latest'),
  hostList: () => call<HostStatus[]>('host_list'),
  hostStatus: (id: string) => call<HostStatus>('host_status', { id }),
  hostCreate: (name: string) => call<string>('host_create', { name }),
  hostDelete: (id: string) => call<void>('host_delete', { id }),
  hostMetrics: (id: string) => call<HostMetrics>('host_metrics', { id }),
  hostFindRelease: () => call<ServerRelease>('host_find_release'),
  hostInstallLatest: (id: string) => call<InstallOutcome>('host_install_latest', { id }),
  hostInstallLocal: (id: string, path: string) => call<InstallOutcome>('host_install_local', { id, path }),
  hostLanIp: () => call<string | null>('host_lan_ip'),
  hostPublicIp: () => call<string>('host_public_ip'),
  hostSetup: (id: string, request: SetupRequest) => call<SetupResult>('host_setup', { id, request }),
  hostStart: (id: string) => call<void>('host_start', { id }),
  hostStop: (id: string) => call<void>('host_stop', { id }),
  hostLogTail: (id: string, lines: number) => call<string[]>('host_log_tail', { id, lines }),
  hostOpenGuide: () => call<void>('host_open_guide'),
  hostSetAutostart: (id: string, on: boolean) => call<HostSettings>('host_set_autostart', { id, on }),
  /** Is there a newer server release than the installed one? Quiet: a failed lookup is in `error`. */
  hostUpdateCheck: (id: string) => call<ServerUpdateCheck>('host_update_check', { id }),
  /** Stop (if running), back up the database, install the new package, keep the config, start again; the old server is put back on a failure. */
  hostUpdateApply: (id: string) => call<UpdateOutcome>('host_update_apply', { id }),
};
