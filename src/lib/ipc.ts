import { invoke } from '@tauri-apps/api/core';
import type {
  AccountInfo, AppInfo, DesktopEntry, GameVersion, HostSettings, HostStatus, InstallOutcome, InstallRequest, InstallStatus, InstalledMod,
  InstalledNew, LogoutResult, PlayState, ProfileView, ServerInfo, ServerRelease, Settings, SetupRequest,
  SetupResult, SteamLogin, VerifyReport,
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
  serverInfo: (id: string) => call<ServerInfo>('server_info', { id }),
  login: (id: string, username: string, password: string) => call<ProfileView>('account_login', { id, username, password }),
  register: (id: string, username: string, password: string) => call<AccountInfo>('account_register', { id, username, password }),
  logout: (id: string) => call<LogoutResult>('account_logout', { id }),
  settings: () => call<Settings>('settings_get'),
  setSettings: (settings: Settings) => call<Settings>('settings_set', { settings }),
  installStatus: () => call<InstallStatus>('install_status'),
  installStart: (request: InstallRequest) => call<void>('install_start', { request }),
  installInput: (text: string) => call<void>('install_input', { text }),
  installCancel: () => call<void>('install_cancel'),
  installVerify: () => call<VerifyReport>('install_verify'),
  mods: () => call<InstalledMod[]>('mods_list'),
  modInstall: (path: string, replace: boolean) => call<InstalledNew>('mod_install', { path, replace }),
  modEnable: (id: string, overrideCompat: boolean) => call<string[]>('mod_enable', { id, overrideCompat }),
  modDisable: (id: string) => call<string[]>('mod_disable', { id }),
  modMove: (id: string, position: number) => call<string[]>('mod_move', { id, position }),
  modAutoOrder: () => call<string[]>('mod_auto_order'),
  modRemove: (id: string) => call<void>('mod_remove', { id }),
  modsDir: () => call<string>('mods_dir'),
  /** Private versions: the server profile; the official mode takes none (the game's own Steam login). */
  play: (profileId: string | null) => call<void>('play', { profileId }),
  steamLogin: () => call<SteamLogin>('steam_login_status'),
  steamLogout: () => call<SteamLogin>('steam_logout'),
  desktopEntry: () => call<DesktopEntry>('desktop_entry_status'),
  desktopEntryInstall: () => call<DesktopEntry>('desktop_entry_install'),
  desktopEntryUninstall: () => call<DesktopEntry>('desktop_entry_uninstall'),
  playState: () => call<PlayState>('play_state'),
  hostStatus: () => call<HostStatus>('host_status'),
  hostFindRelease: () => call<ServerRelease>('host_find_release'),
  hostInstallLatest: () => call<InstallOutcome>('host_install_latest'),
  hostInstallLocal: (path: string) => call<InstallOutcome>('host_install_local', { path }),
  hostLanIp: () => call<string | null>('host_lan_ip'),
  hostPublicIp: () => call<string>('host_public_ip'),
  hostSetup: (request: SetupRequest) => call<SetupResult>('host_setup', { request }),
  hostStart: () => call<void>('host_start'),
  hostStop: () => call<void>('host_stop'),
  hostLogTail: (lines: number) => call<string[]>('host_log_tail', { lines }),
  hostOpenGuide: () => call<void>('host_open_guide'),
  hostSetAutostart: (on: boolean) => call<HostSettings>('host_set_autostart', { on }),
};
