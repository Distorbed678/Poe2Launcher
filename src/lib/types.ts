// Mirrors the Rust views in src-tauri/src (serde, snake_case).

export interface ProfileView {
  id: string;
  name: string;
  url: string;
  logged_in: boolean;
  account: string | null;
  expires_at: number | null;
  keyring_error: string | null;
}

export interface ServerInfo {
  name: string;
  version: string;
  cluster_id: string;
  online: number;
  open_registration: boolean;
  login_host: string;
  login_port: number;
  instance_port: number | null;
  gateway_region: string | null;
  uptime_secs: number;
}

export interface AccountInfo {
  id: number;
  username: string;
  role: string;
}

export interface LogoutResult {
  revoked: boolean;
  message: string;
}

export interface Settings {
  install_dir: string | null;
  dll_dir: string | null;
  steam_root: string | null;
  proton: string;
  selected_profile: string | null;
  steam_username: string;
  remember_steam_login: boolean;
  prefer_code_entry: boolean;
  theme: 'dark' | 'light';
}

export interface DepotView {
  id: number;
  manifest: string;
  role: string;
}

export interface InstallStatus {
  version: string;
  app_id: number;
  install_dir: string;
  installed: boolean;
  running: boolean;
  tool_version: string;
  tool_ready: boolean;
  depots: DepotView[];
}

export interface InstallRequest {
  steam_username: string;
  steam_password: string;
  remember_login: boolean;
  validate: boolean;
}

export type CheckStatus = 'pass' | 'warn' | 'fail' | 'skipped';

export interface Check {
  name: string;
  status: CheckStatus;
  detail: string;
}

export interface VerifyReport {
  ok: boolean;
  files: number;
  bytes: number;
  exe_sha256: string | null;
  checks: Check[];
}

export type Compat =
  | { kind: 'ok' }
  | { kind: 'unspecified' }
  | { kind: 'wrong_game'; supported: string[] }
  | { kind: 'hook_api_too_new'; needs: number; have: number };

export interface ModManifest {
  id: string;
  name: string;
  version: string;
  description: string;
  authors: string[];
  license: string;
  homepage: string;
  game_versions: string[];
  hook_api: number | null;
  after: string[];
  before: string[];
  conflicts: string[];
  dependencies: Record<string, string>;
  entry_dll: string | null;
}

export interface InstalledMod {
  id: string;
  dir_name: string;
  manifest: ModManifest | null;
  errors: string[];
  unknown_keys: string[];
  compat: Compat | null;
  enabled: boolean;
  order: number | null;
}

export interface InstalledNew {
  id: string;
  compat: Compat;
  unknown_keys: string[];
}

export type PlayState =
  | { state: 'idle' }
  | { state: 'preparing'; step: string }
  | { state: 'running'; pid: number | null }
  | { state: 'exited'; code: number | null };

export interface AppInfo {
  launcher_version: string;
  client_version: string;
  os: string;
  data_dir: string;
  config_dir: string;
}

export type PromptKind = 'password' | 'guard_device' | 'guard_email';
