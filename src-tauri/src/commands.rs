//! The commands the UI invokes (`@tauri-apps/api` `invoke`). Each is a thin wrapper over
//! [`crate::core::Core`]; errors reach the UI as readable strings.

use std::sync::Arc;

use serde::Serialize;
use tauri::State;
use tauri_plugin_opener::OpenerExt;

use crate::{
    api::{AccountInfo, ServerInfo, ServerStatus},
    backup::{BackupInfo, BackupSettings},
    backup_ops::BackupStatus,
    core::{
        Core, GameVersionView, InstallRequest, InstallStatus, LogoutResult, PlayState, ProfileView,
        UrlOpener,
    },
    desktop::DesktopEntry,
    hook_ops::{HookCheck, HookOutcome, HookStatus},
    host::{HostSettings, ServerRelease},
    host_ops::{HostStatus, InstallOutcome, SetupRequest, SetupResult},
    logs::{LogContent, LogList},
    mod_ops::{CacheStatus, ModsView, Pruned},
    modcache::Added,
    modindex::ModIndexView,
    modsync::SyncPlan,
    news::NewsFeed,
    serverlist::{ProbeResult, ServerList},
    settings::Settings,
    space::SpaceCheck,
    steam::SteamLogin,
    verify::VerifyReport,
};

type App<'a> = State<'a, Arc<Core>>;
type Res<T> = Result<T, String>;

fn err(e: anyhow::Error) -> String {
    format!("{e:#}")
}

#[derive(Serialize)]
pub struct AppInfo {
    pub launcher_version: &'static str,
    /// The selected game version.
    pub client_version: String,
    /// Every game version this build supports (the version dropdown), oldest first.
    pub game_versions: Vec<GameVersionView>,
    pub os: &'static str,
    pub data_dir: String,
    pub config_dir: String,
}

#[tauri::command]
pub fn app_info(app: App<'_>) -> AppInfo {
    AppInfo {
        launcher_version: env!("CARGO_PKG_VERSION"),
        client_version: app.version().version,
        game_versions: app.game_versions().unwrap_or_default(),
        os: std::env::consts::OS,
        data_dir: app.dirs.data.display().to_string(),
        config_dir: app.dirs.config.display().to_string(),
    }
}

/// Picks the game version everything version-dependent follows; returns the new list.
#[tauri::command]
pub fn game_version_set(app: App<'_>, version: String) -> Res<Vec<GameVersionView>> {
    app.set_game_version(&version).map_err(err)
}

#[tauri::command]
pub fn profiles_list(app: App<'_>) -> Res<Vec<ProfileView>> {
    app.profiles().map_err(err)
}

#[tauri::command]
pub fn profile_add(
    app: App<'_>,
    name: String,
    url: String,
    confirm_plain_http: bool,
) -> Res<ProfileView> {
    app.add_profile(&name, &url, confirm_plain_http)
        .map_err(err)
}

#[tauri::command]
pub fn profile_update(
    app: App<'_>,
    id: String,
    name: String,
    url: String,
    confirm_plain_http: bool,
) -> Res<ProfileView> {
    app.update_profile(&id, &name, &url, confirm_plain_http)
        .map_err(err)
}

#[tauri::command]
pub fn profile_confirm_plain_http(app: App<'_>, id: String, ok: bool) -> Res<ProfileView> {
    app.set_plain_http_confirmed(&id, ok).map_err(err)
}

#[tauri::command]
pub async fn profile_remove(app: App<'_>, id: String) -> Res<()> {
    app.remove_profile(&id).await.map_err(err)
}

#[tauri::command]
pub async fn server_info(app: App<'_>, id: String) -> Res<ServerInfo> {
    app.server_info(&id).await.map_err(err)
}

#[tauri::command]
pub async fn account_login(
    app: App<'_>,
    id: String,
    username: String,
    password: String,
) -> Res<ProfileView> {
    app.login(&id, &username, &password).await.map_err(err)
}

#[tauri::command]
pub async fn account_register(
    app: App<'_>,
    id: String,
    username: String,
    password: String,
) -> Res<AccountInfo> {
    app.register(&id, &username, &password).await.map_err(err)
}

#[tauri::command]
pub async fn account_logout(app: App<'_>, id: String) -> Res<LogoutResult> {
    app.logout(&id).await.map_err(err)
}

#[tauri::command]
pub async fn account_refresh(app: App<'_>, id: String) -> Res<ProfileView> {
    app.refresh_account(&id).await.map_err(err)
}

/// Hands the sign-in link to the system browser (Tauri opener).
struct TauriOpener(tauri::AppHandle);

impl UrlOpener for TauriOpener {
    fn open(&self, url: &str) -> anyhow::Result<()> {
        self.0
            .opener()
            .open_url(url, None::<&str>)
            .map_err(|e| anyhow::anyhow!("{e}"))
    }
}

/// "Open web admin": admin check, single-use web token, then the system browser.
#[tauri::command]
pub async fn web_admin_open(app: App<'_>, handle: tauri::AppHandle, id: String) -> Res<()> {
    app.web_admin_open(&id, &TauriOpener(handle))
        .await
        .map_err(err)
}

#[tauri::command]
pub fn settings_get(app: App<'_>) -> Res<Settings> {
    app.settings().map_err(err)
}

#[tauri::command]
pub fn settings_set(app: App<'_>, settings: Settings) -> Res<Settings> {
    app.set_settings(settings).map_err(err)
}

#[tauri::command]
pub fn install_status(app: App<'_>) -> Res<InstallStatus> {
    app.install_status().map_err(err)
}

#[tauri::command]
pub fn install_start(app: App<'_>, request: InstallRequest) -> Res<()> {
    app.inner().install_start(request).map_err(err)
}

#[tauri::command]
pub fn install_input(app: App<'_>, text: String) -> Res<()> {
    app.install_input(&text).map_err(err)
}

/// Pauses the download (DepotDownloader is stopped; what is on disk stays).
#[tauri::command]
pub fn install_pause(app: App<'_>) -> Res<()> {
    app.install_pause().map_err(err)
}

/// Resumes a paused download (the password is only needed when no Steam login is remembered).
#[tauri::command]
pub fn install_resume(app: App<'_>, steam_password: String) -> Res<()> {
    app.inner().install_resume(steam_password).map_err(err)
}

/// Disk space needed for the selected version against the free space of the target disk.
#[tauri::command]
pub async fn install_space(app: App<'_>) -> Res<SpaceCheck> {
    app.install_space().await.map_err(err)
}

#[tauri::command]
pub fn install_cancel(app: App<'_>) -> Res<()> {
    app.install_cancel().map_err(err)
}

#[tauri::command]
pub async fn install_verify(app: App<'_>) -> Res<VerifyReport> {
    app.verify_install().await.map_err(err)
}

#[tauri::command]
pub fn mods_list(app: App<'_>, scope: Option<String>) -> Res<ModsView> {
    app.mods_list(scope.as_deref().unwrap_or("all"))
        .map_err(err)
}

/// Adds a local mod (a package zip or a mod folder) to the cache; with `scope`, enables it there.
#[tauri::command]
pub fn mod_install(app: App<'_>, path: String, scope: Option<String>, replace: bool) -> Res<Added> {
    app.mod_install(&path, scope.as_deref(), replace)
        .map_err(err)
}

#[tauri::command]
pub fn mod_set_enabled(
    app: App<'_>,
    scope: String,
    id: String,
    version: String,
    enabled: bool,
    force: bool,
) -> Res<Vec<String>> {
    app.mod_set_enabled(&scope, &id, &version, enabled, force)
        .map_err(err)
}

#[tauri::command]
pub fn mod_move(app: App<'_>, scope: String, id: String, position: usize) -> Res<Vec<String>> {
    app.mod_move(&scope, &id, position).map_err(err)
}

#[tauri::command]
pub fn mod_auto_order(app: App<'_>, scope: String) -> Res<Vec<String>> {
    app.mod_auto_order(&scope).map_err(err)
}

#[tauri::command]
pub fn mod_remove(app: App<'_>, id: String, version: String) -> Res<()> {
    app.mod_remove(&id, &version).map_err(err)
}

#[tauri::command]
pub fn mods_dir(app: App<'_>) -> Res<String> {
    app.mods_dir().map(|p| p.display().to_string()).map_err(err)
}

#[tauri::command]
pub fn mod_cache_status(app: App<'_>) -> Res<CacheStatus> {
    app.mod_cache_status().map_err(err)
}

#[tauri::command]
pub fn mod_cache_prune(app: App<'_>) -> Res<Pruned> {
    app.mod_cache_prune().map_err(err)
}

/// Asks the server what it requires and compares it with the cache (Auto mode also downloads).
#[tauri::command]
pub async fn mod_sync_check(app: App<'_>, profile_id: String) -> Res<SyncPlan> {
    app.inner().mod_sync_check(&profile_id).await.map_err(err)
}

/// The Download button of the sync dialog.
#[tauri::command]
pub async fn mod_sync_apply(
    app: App<'_>,
    profile_id: String,
    ids: Option<Vec<String>>,
) -> Res<SyncPlan> {
    app.inner()
        .mod_sync_apply(&profile_id, ids)
        .await
        .map_err(err)
}

/// The mod browser's index (cached; never fails: offline gives the saved copy).
#[tauri::command]
pub async fn mod_index_get(app: App<'_>, force: bool) -> Res<ModIndexView> {
    Ok(app.inner().mod_index_get(force).await)
}

#[tauri::command]
pub async fn mod_index_install(
    app: App<'_>,
    id: String,
    version: String,
    scope: Option<String>,
) -> Res<Added> {
    app.inner()
        .mod_index_install(&id, &version, scope.as_deref())
        .await
        .map_err(err)
}

/// Starts the game: for a private game version with the given server profile, for "Official
/// (read-only)" without one (the game's own Steam login; the launcher logs in nowhere).
#[tauri::command]
pub async fn play(app: App<'_>, profile_id: Option<String>) -> Res<()> {
    let core = app.inner();
    if core.version().is_official() {
        return core.play_official().await.map_err(err);
    }
    let id = profile_id.ok_or_else(|| "pick a server first".to_owned())?;
    core.play(&id).await.map_err(err)
}

/// Whether DepotDownloader remembers a Steam login.
#[tauri::command]
pub fn steam_login_status(app: App<'_>) -> Res<SteamLogin> {
    app.steam_login().map_err(err)
}

/// "Log out of Steam": forgets the remembered DepotDownloader login.
#[tauri::command]
pub fn steam_logout(app: App<'_>) -> Res<SteamLogin> {
    app.steam_logout().map_err(err)
}

/// Linux: is the `.desktop` entry (for the icon and name on Wayland) installed?
#[tauri::command]
pub fn desktop_entry_status() -> DesktopEntry {
    crate::desktop::status()
}

#[tauri::command]
pub fn desktop_entry_install() -> Res<DesktopEntry> {
    crate::desktop::install().map_err(err)
}

#[tauri::command]
pub fn desktop_entry_uninstall() -> Res<DesktopEntry> {
    crate::desktop::uninstall().map_err(err)
}

#[tauri::command]
pub fn play_state(app: App<'_>) -> PlayState {
    app.play_state()
}

// --- Client hook --------------------------------------------------------------------------

#[tauri::command]
pub fn hook_status(app: App<'_>) -> Res<HookStatus> {
    app.hook_status().map_err(err)
}

#[tauri::command]
pub async fn hook_check(app: App<'_>) -> Res<HookCheck> {
    app.hook_check().await.map_err(err)
}

#[tauri::command]
pub async fn hook_install_latest(app: App<'_>) -> Res<HookOutcome> {
    app.hook_install_latest().await.map_err(err)
}

// --- Host a server ------------------------------------------------------------------------

#[tauri::command]
pub async fn host_status(app: App<'_>) -> Res<HostStatus> {
    app.host_status().await.map_err(err)
}

#[tauri::command]
pub async fn host_find_release(app: App<'_>) -> Res<ServerRelease> {
    app.host_find_release().await.map_err(err)
}

#[tauri::command]
pub async fn host_install_latest(app: App<'_>) -> Res<InstallOutcome> {
    app.host_install_latest().await.map_err(err)
}

#[tauri::command]
pub async fn host_install_local(app: App<'_>, path: String) -> Res<InstallOutcome> {
    app.host_install_local(&path).await.map_err(err)
}

#[tauri::command]
pub fn host_lan_ip(app: App<'_>) -> Option<String> {
    app.host_lan_ip()
}

#[tauri::command]
pub async fn host_public_ip(app: App<'_>) -> Res<String> {
    app.host_public_ip().await.map_err(err)
}

#[tauri::command]
pub async fn host_setup(app: App<'_>, request: SetupRequest) -> Res<SetupResult> {
    app.host_setup(request).await.map_err(err)
}

#[tauri::command]
pub async fn host_start(app: App<'_>) -> Res<()> {
    app.host_start().await.map_err(err)
}

#[tauri::command]
pub async fn host_stop(app: App<'_>) -> Res<()> {
    app.host_stop().await.map_err(err)
}

#[tauri::command]
pub fn host_log_tail(app: App<'_>, lines: usize) -> Vec<String> {
    app.host_log_tail(lines)
}

#[tauri::command]
pub fn host_set_autostart(app: App<'_>, on: bool) -> Res<HostSettings> {
    app.host_set_autostart(on).map_err(err)
}

/// Opens the hosting guide (a fixed address) in the system browser.
#[tauri::command]
pub fn host_open_guide(handle: tauri::AppHandle) -> Res<()> {
    TauriOpener(handle)
        .open(crate::host::HOSTING_GUIDE_URL)
        .map_err(err)
}

// --- Home: news, server status -----------------------------------------------------------

/// Release notes of the project's repositories and `news.json`, cached; never an error.
#[tauri::command]
pub async fn news_get(app: App<'_>, force: bool) -> Res<NewsFeed> {
    Ok(app.news_get(force).await)
}

/// `GET /server` of one of the user's servers with the ping measured by the launcher.
#[tauri::command]
pub async fn server_status(app: App<'_>, id: String) -> Res<ServerStatus> {
    app.server_status(&id).await.map_err(err)
}

// --- Server browser ------------------------------------------------------------------------

#[tauri::command]
pub async fn server_list_get(app: App<'_>, force: bool) -> Res<ServerList> {
    Ok(app.server_list_get(force).await)
}

/// Asks the listed servers with these ids (`GET /server`, in parallel, short timeouts).
#[tauri::command]
pub async fn server_list_probe(app: App<'_>, ids: Vec<String>) -> Res<Vec<ProbeResult>> {
    app.server_list_probe(&ids).await.map_err(err)
}

// --- Logs and crash dumps ------------------------------------------------------------------

#[tauri::command]
pub fn log_list(app: App<'_>) -> LogList {
    app.log_list()
}

#[tauri::command]
pub fn log_read(app: App<'_>, id: String, lines: usize) -> Res<LogContent> {
    app.log_read(&id, lines).map_err(err)
}

/// The text of "Copy for a bug report" (redacted).
#[tauri::command]
pub fn log_report(app: App<'_>, id: Option<String>, lines: usize) -> Res<String> {
    app.log_report(id.as_deref(), lines).map_err(err)
}

/// Opens the folder of `game`, `server` or `crash` logs in the file manager.
#[tauri::command]
pub fn log_open_folder(app: App<'_>, handle: tauri::AppHandle, kind: String) -> Res<()> {
    let dir = app.log_folder(&kind).map_err(err)?;
    handle
        .opener()
        .open_path(dir.display().to_string(), None::<&str>)
        .map_err(|e| format!("could not open the folder: {e}"))
}

// --- Server backups ------------------------------------------------------------------------

#[tauri::command]
pub fn backup_status(app: App<'_>) -> Res<BackupStatus> {
    app.backup_status().map_err(err)
}

#[tauri::command]
pub async fn backup_create(app: App<'_>) -> Res<BackupInfo> {
    let core = app.inner().clone();
    tokio::task::spawn_blocking(move || core.backup_create())
        .await
        .map_err(|e| e.to_string())?
        .map_err(err)
}

/// Restores a backup; returns the backup made of the database that was replaced.
#[tauri::command]
pub async fn backup_restore(app: App<'_>, name: String) -> Res<Option<BackupInfo>> {
    let core = app.inner().clone();
    tokio::task::spawn_blocking(move || core.backup_restore(&name))
        .await
        .map_err(|e| e.to_string())?
        .map_err(err)
}

#[tauri::command]
pub fn backup_delete(app: App<'_>, name: String) -> Res<()> {
    app.backup_delete(&name).map_err(err)
}

#[tauri::command]
pub fn backup_set_settings(app: App<'_>, auto: bool, keep: u32) -> Res<BackupSettings> {
    app.backup_set_settings(auto, keep).map_err(err)
}

// --- Links -----------------------------------------------------------------------------------

/// Opens an `https://` link from the news, the server list or a server's message in the system
/// browser. Anything else is refused: the text behind these links is not trusted.
#[tauri::command]
pub fn open_link(handle: tauri::AppHandle, url: String) -> Res<()> {
    let url = url.trim();
    if !crate::news::is_https_link(url) {
        return Err("only https:// links are opened".into());
    }
    handle
        .opener()
        .open_url(url, None::<&str>)
        .map_err(|e| format!("could not open the browser: {e}"))
}
