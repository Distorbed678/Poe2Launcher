import { invoke } from '@tauri-apps/api/core';
import type {
  AccountInfo, AppInfo, InstallRequest, InstallStatus, InstalledMod, InstalledNew, LogoutResult,
  PlayState, ProfileView, ServerInfo, Settings, VerifyReport,
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
  profiles: () => call<ProfileView[]>('profiles_list'),
  profileAdd: (name: string, url: string) => call<ProfileView>('profile_add', { name, url }),
  profileUpdate: (id: string, name: string, url: string) => call<ProfileView>('profile_update', { id, name, url }),
  profileRemove: (id: string) => call<void>('profile_remove', { id }),
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
  play: (profileId: string) => call<void>('play', { profileId }),
  playState: () => call<PlayState>('play_state'),
};
