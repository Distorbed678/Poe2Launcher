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
  /** Message of the day: optional, only present when the server sends one. Plain text / markdown subset. */
  motd?: string | null;
  /** The client mods the server requires or offers (servers without mod support send nothing). */
  mods?: { mods: unknown[]; skipped: string[] };
}

/** `GET /server` with the round-trip time measured by the launcher. */
export interface ServerStatus {
  info: ServerInfo;
  ping_ms: number;
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
  /** What to do when a server requires mods that are not downloaded: `off`, `ask` (default) or `auto`. */
  mod_sync: SyncModeName;
}

export interface DepotView {
  id: number;
  manifest: string;
  role: string;
  /** What the depot weighs in the overall progress (estimate). */
  est_bytes: number;
}

/** A download that was paused and can be resumed (the password is not kept). */
export interface PausedInstall {
  version: string;
  steam_username: string;
  remember_login: boolean;
}

/** Free disk space against what the install needs. */
export interface SpaceCheck {
  path: string;
  required: number;
  present: number;
  missing: number;
  free: number | null;
  ok: boolean;
  message: string;
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
  /** Estimated size of a whole install (0 = unknown). */
  required_bytes: number;
  paused: PausedInstall | null;
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
  | { kind: 'hook_api_too_new'; needs: number; have: number }
  | { kind: 'hook_api_old'; needs: number; have: number }
  | { kind: 'hook_api_missing'; have: number };

/** Where a mod runs: `client` (the hook), `server`, or `both` (one package, two halves). */
export type ModSide = 'client' | 'server' | 'both';

export interface ModManifest {
  id: string;
  name: string;
  version: string;
  description: string;
  authors: string[];
  license: string;
  homepage: string;
  side: ModSide;
  game_versions: string[];
  hook_api: number | null;
  server_api: number | null;
  after: string[];
  before: string[];
  conflicts: string[];
  dependencies: Record<string, string>;
  entry_dll: string | null;
  /** The client half runs native code inside the game process. */
  has_code: boolean;
}

export type ModOrigin = 'required' | 'profile' | 'global' | 'dependency';

/** One cached version of a mod, with its state in the scope being shown. */
export interface ModEntry {
  id: string;
  version: string;
  side: ModSide;
  manifest: ModManifest | null;
  errors: string[];
  unknown_keys: string[];
  compat: Compat | null;
  compat_text: string;
  sha256: string;
  size: number;
  /** `server`, `index`, `zip`, `folder`. */
  source: string;
  added: number;
  has_code: boolean;
  server_half_left_out: boolean;
  /** Enabled in the scope being shown. */
  enabled: boolean;
  /** Enabled for all servers. */
  enabled_all: boolean;
  /** The scope's server requires exactly this version. */
  required: boolean;
  /** Part of what Play would load. */
  in_effect: boolean;
  origin: ModOrigin | null;
  order: number | null;
}

export type SyncState = 'ready' | 'missing' | 'changed' | 'incompatible' | 'no_source';

export interface SyncMod {
  id: string;
  version: string;
  name: string;
  side: ModSide;
  size: number;
  sha256: string;
  required: boolean;
  /** Native code that would run inside the game process. */
  code: boolean;
  authors: string[];
  description: string;
  state: SyncState;
  detail: string;
}

export type SyncModeName = 'off' | 'ask' | 'auto';

/** `ok`, `ask` (needs the player's go), `auto` (will download), `off` (blocked by the setting), `blocked`. */
export type SyncStatus = 'ok' | 'ask' | 'auto' | 'off' | 'blocked';

export interface SyncPlan {
  profile_id: string;
  profile_name: string;
  mode: SyncModeName;
  status: SyncStatus;
  mods: SyncMod[];
  blocker: string | null;
  download_bytes: number;
  skipped: string[];
  stale: boolean;
  applied: string[];
  errors: string[];
}

export interface ModsView {
  entries: ModEntry[];
  scope: string;
  scope_name: string;
  required: SyncMod[];
  dropped: string[];
  problems: string[];
  cache_dir: string;
  cache_bytes: number;
  game_version: string;
  hook_abi: number;
  mod_sync: SyncModeName;
}

export interface CacheStatus {
  dir: string;
  entries: number;
  bytes: number;
  unused_entries: number;
  unused_bytes: number;
}

export interface Pruned {
  removed: number;
  bytes: number;
}

export interface AddedMod {
  id: string;
  version: string;
  side: ModSide;
  sha256: string;
  size: number;
  already_cached: boolean;
  has_code: boolean;
  server_half_left_out: boolean;
  unknown_keys: string[];
  warnings: string[];
}

export interface IndexVersion {
  version: string;
  released: string;
  game_versions: string[];
  hook_api: number | null;
  server_api: number | null;
  sha256: string;
  size: number;
  url: string;
  yanked: boolean;
  compat: Compat;
  /** Already in the mod cache with this hash. */
  cached: boolean;
}

export interface IndexMod {
  id: string;
  name: string;
  description: string;
  author: string;
  side: ModSide;
  homepage: string;
  licence: string;
  tags: string[];
  /** Newest first. */
  versions: IndexVersion[];
}

export interface ModIndex {
  mods: IndexMod[];
  skipped: string[];
  updated: string;
  fetched_at: number;
  offline: boolean;
  error: string;
}

export type PlayState =
  | { state: 'idle' }
  | { state: 'preparing'; step: string }
  | { state: 'running'; pid: number | null }
  | { state: 'exited'; code: number | null };

/** One entry of the version dropdown ("Official (read-only)" first, then the private versions, newest first). */
export interface GameVersion {
  /** `official` or a client version such as `0.5.5e.hotfix1`. */
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
  /** The hosted server's id (`s1`, `s2`, ...). */
  id: string;
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
  /** The server's own folder (database, config, backups, log). */
  folder: string;
  busy: boolean;
  hosting_guide_url: string;
}

/** Manage > Overview: usage of a running server (`host_metrics`). */
export interface HostMetrics {
  cpu_pct: number | null;
  rss_bytes: number | null;
  cpu_history: number[];
  rss_history: number[];
  tick_avg_ms: number | null;
  tick_max_ms: number | null;
  net_in_bps: number | null;
  net_out_bps: number | null;
  players: number | null;
  uptime_secs: number | null;
  /** The server reports metrics (`[metrics] enabled`). */
  server_metrics: boolean;
}

export interface ServerRelease {
  /** The GitHub repository the release is in (Poe2Server, or Poe2ModdingProject for the two oldest). */
  repo: string;
  tag: string;
  version: string;
  game_version: string;
  prerelease: boolean;
  asset_name: string;
  asset_url: string;
  size: number;
  sha256_url: string;
  /** Release notes (a markdown subset, rendered as text). */
  notes: string;
  html_url: string | null;
}

/** `host_update_check`: never an error, a failed lookup is in `error`. */
export interface ServerUpdateCheck {
  installed: string | null;
  release: ServerRelease | null;
  update_available: boolean;
  error: string | null;
  /** Unix seconds. */
  checked_at: number;
}

export interface UpdateOutcome {
  updated: boolean;
  from: string;
  to: string;
  package: PackageInfo;
  was_running: boolean;
  restarted: boolean;
  backup: string | null;
  sha256: string | null;
}

export interface InstallOutcome {
  package: PackageInfo;
  sha256: string | null;
  checksum_verified: boolean;
  source: 'download' | 'file' | 'folder' | 'current' | 'shared';
}

export interface SetupRequest {
  settings: HostSettings;
}

export interface SetupResult {
  profile: ProfileView;
  config_file: string;
}

// --- Client hook -------------------------------------------------------------------------------

/** Where Play takes the hook DLLs from. */
export type DllSource =
  | { kind: 'manual' }
  | { kind: 'downloaded'; version: string }
  | { kind: 'legacy' }
  | { kind: 'missing' };

export interface InstalledHook {
  dir: string;
  name: string;
  version: string;
  game_versions: string[];
  mod_abi: number;
  commit: string;
}

export interface HookStatus {
  /** The game version the hook is matched against (Official uses the newest private version). */
  game_version: string;
  repo: string;
  source: DllSource;
  dll_dir: string;
  manual_dir: string | null;
  installed: InstalledHook | null;
  other_installed: InstalledHook[];
  busy: boolean;
}

export interface HookRelease {
  tag: string;
  version: string;
  game_versions: string[];
  prerelease: boolean;
  asset_name: string;
  asset_url: string;
  size: number;
  sha256_url: string;
}

export interface HookCheck {
  release: HookRelease;
  update_available: boolean;
}

export interface HookOutcome {
  hook: InstalledHook;
  sha256: string | null;
  source: 'download' | 'current';
}

// --- Home: news ----------------------------------------------------------------------------

export type NewsSource = 'launcher' | 'server' | 'hook' | 'project';

export interface NewsItem {
  id: string;
  source: NewsSource;
  /** `YYYY-MM-DD` or an RFC 3339 time. */
  date: string;
  title: string;
  /** A markdown subset, shown as text (never HTML). */
  body: string;
  link: string | null;
  version?: string | null;
}

export interface NewsFeed {
  items: NewsItem[];
  fetched_at: number;
  /** Nothing could be fetched just now: these are the saved items. */
  offline: boolean;
}

// --- Server browser ------------------------------------------------------------------------

export interface RequiredMod {
  id: string;
  version?: string | null;
  url?: string | null;
}

export interface ListedServer {
  id: string;
  name: string;
  address: string;
  description: string;
  region: string;
  game_version: string;
  tags: string[];
  contact: string;
  mods: RequiredMod[];
  plain_http: boolean;
}

export interface ServerList {
  servers: ListedServer[];
  skipped: string[];
  fetched_at: number;
  offline: boolean;
  error: string;
}

export interface ProbeResult {
  id: string;
  ok: boolean;
  ping_ms: number | null;
  info: ServerInfo | null;
  error: string | null;
}

// --- Logs ----------------------------------------------------------------------------------

export type LogKind = 'game' | 'server' | 'crash';

export interface LogFile {
  id: string;
  kind: LogKind;
  name: string;
  size: number;
  modified: number;
  text: boolean;
}

export interface LogList {
  files: LogFile[];
  game_dir: string;
  server_dir: string;
  crash_dir: string;
}

export interface LogContent {
  id: string;
  lines: string[];
  truncated: boolean;
  size: number;
  binary: boolean;
}

// --- Server backups ------------------------------------------------------------------------

export interface BackupInfo {
  name: string;
  kind: 'manual' | 'auto' | 'pre-restore' | 'pre-update' | 'other';
  /** Unix seconds. */
  created: number;
  size: number;
}

export interface BackupSettings {
  auto: boolean;
  keep: number;
}

export interface BackupStatus {
  dir: string;
  backups: BackupInfo[];
  settings: BackupSettings;
  database_size: number | null;
  server_running: boolean;
}
