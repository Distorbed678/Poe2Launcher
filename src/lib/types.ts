// Mirrors the Rust views in src-tauri/src (serde, snake_case).

/** How exposed a server address is: https, or plain http to this computer / a local network / elsewhere. */
export type PlainHttp = 'https' | 'loopback' | 'private' | 'public';

export interface ProfileView {
  id: string;
  name: string;
  url: string;
  logged_in: boolean;
  account: string | null;
  expires_at: number | null;
  /** `player` or `admin`, as of the last login or refresh. */
  role: string | null;
  plain_http: PlainHttp;
  /** The user agreed to send the password and token in clear text to this address. */
  plain_http_confirmed: boolean;
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
  /** Where each game version is installed, by version (`official` included); missing = the default folder. */
  install_dirs: Record<string, string>;
  /** The selected game version, `official`, or null = the newest supported. */
  game_version: string | null;
  /** The official-mode warning was accepted. */
  official_ack: boolean;
  /** Minimize the launcher while the game runs, bring it back when the game ends. */
  minimize_while_playing: boolean;
  /** The launcher's own title bar and frame (off: the system frame). */
  custom_frame: boolean;
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

/** One entry of the version dropdown ("Official (read-only)" first, then the private versions, newest first). */
export interface GameVersion {
  /** `official` or a client version such as `0.5.5e`. */
  version: string;
  app_id: number;
  official: boolean;
  selected: boolean;
  newest: boolean;
  installed: boolean;
}

/** The Steam login DepotDownloader remembers. */
export interface SteamLogin {
  remembered: boolean;
  account: string | null;
  files: string[];
}

/** The Linux `.desktop` entry that gives the window its icon on Wayland. */
export interface DesktopEntry {
  supported: boolean;
  installed: boolean;
  exec: string | null;
  stale: boolean;
  file: string | null;
  current_exec: string | null;
  appimage: boolean;
}

export interface AppInfo {
  launcher_version: string;
  /** The selected game version (`official` for the official mode). */
  client_version: string;
  game_versions: GameVersion[];
  os: string;
  data_dir: string;
  config_dir: string;
}

export type PromptKind = 'password' | 'guard_device' | 'guard_email';

// --- Host a server (src-tauri/src/host.rs, host_ops.rs, host_run.rs) ---

export interface HostSettings {
  server_name: string;
  cluster_id: string;
  login_port: number;
  instance_port: number;
  web_port: number;
  public_host: string;
  web_public: boolean;
  open_registration: boolean;
  admin_name: string;
  autostart: boolean;
  package_dir: string | null;
  configured: boolean;
  profile_id: string | null;
}

export interface PackageInfo {
  dir: string;
  name: string;
  server_version: string;
  game_version: string;
  platform: string;
  commit: string;
}

export type RunState =
  | { state: 'stopped' }
  | { state: 'running'; pid: number }
  | { state: 'stopping'; pid: number }
  | { state: 'exited'; code: number | null };

export interface HostStatus {
  settings: HostSettings;
  package: PackageInfo | null;
  package_problem: string | null;
  platform: string;
  game_version: string;
  game_installed: boolean;
  install_dir: string;
  run: RunState;
  /** The web API answers (the server is ready). */
  info: ServerInfo | null;
  web_url: string;
  lan_ip: string | null;
  log_file: string;
  busy: boolean;
  hosting_guide_url: string;
}

export interface ServerRelease {
  tag: string;
  version: string;
  game_version: string;
  prerelease: boolean;
  asset_name: string;
  asset_url: string;
  size: number;
  sha256_url: string;
}

export interface InstallOutcome {
  package: PackageInfo;
  sha256: string | null;
  checksum_verified: boolean;
  source: 'download' | 'file' | 'folder' | 'current';
}

export interface SetupRequest {
  settings: HostSettings;
  /** Typed in the UI, used once for the admin account, never stored. */
  admin_password: string;
}

export interface SetupResult {
  profile: ProfileView;
  admin_existed: boolean;
  config_file: string;
}
