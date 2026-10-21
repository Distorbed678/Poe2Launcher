//! The commands the UI invokes (`@tauri-apps/api` `invoke`). Each is a thin wrapper over
//! [`crate::core::Core`]; errors reach the UI as readable strings.

use std::sync::Arc;

use serde::Serialize;
use tauri::State;
use tauri_plugin_opener::OpenerExt;

use crate::{
    api::{AccountInfo, ServerInfo},
    core::{Core, InstallRequest, InstallStatus, LogoutResult, PlayState, ProfileView, UrlOpener},
    host::{HostSettings, ServerRelease},
    host_ops::{HostStatus, InstallOutcome, SetupRequest, SetupResult},
    mods::{InstalledMod, InstalledNew},
    settings::Settings,
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
    pub client_version: String,
    pub os: &'static str,
    pub data_dir: String,
    pub config_dir: String,
}

#[tauri::command]
pub fn app_info(app: App<'_>) -> AppInfo {
    AppInfo {
        launcher_version: env!("CARGO_PKG_VERSION"),
        client_version: app.version.version.clone(),
        os: std::env::consts::OS,
        data_dir: app.dirs.data.display().to_string(),
        config_dir: app.dirs.config.display().to_string(),
    }
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

#[tauri::command]
pub fn install_cancel(app: App<'_>) -> Res<()> {
    app.install_cancel().map_err(err)
}

#[tauri::command]
pub async fn install_verify(app: App<'_>) -> Res<VerifyReport> {
    app.verify_install().await.map_err(err)
}

#[tauri::command]
pub fn mods_list(app: App<'_>) -> Res<Vec<InstalledMod>> {
    app.mods_list().map_err(err)
}

#[tauri::command]
pub fn mod_install(app: App<'_>, path: String, replace: bool) -> Res<InstalledNew> {
    app.mod_install(&path, replace).map_err(err)
}

#[tauri::command]
pub fn mod_enable(app: App<'_>, id: String, override_compat: bool) -> Res<Vec<String>> {
    app.mod_enable(&id, override_compat).map_err(err)
}

#[tauri::command]
pub fn mod_disable(app: App<'_>, id: String) -> Res<Vec<String>> {
    app.mod_disable(&id).map_err(err)
}

#[tauri::command]
pub fn mod_move(app: App<'_>, id: String, position: usize) -> Res<Vec<String>> {
    app.mod_move(&id, position).map_err(err)
}

#[tauri::command]
pub fn mod_auto_order(app: App<'_>) -> Res<Vec<String>> {
    app.mod_auto_order().map_err(err)
}

#[tauri::command]
pub fn mod_remove(app: App<'_>, id: String) -> Res<()> {
    app.mod_remove(&id).map_err(err)
}

#[tauri::command]
pub fn mods_dir(app: App<'_>) -> Res<String> {
    app.mods_dir().map(|p| p.display().to_string()).map_err(err)
}

#[tauri::command]
pub async fn play(app: App<'_>, profile_id: String) -> Res<()> {
    app.inner().play(&profile_id).await.map_err(err)
}

#[tauri::command]
pub fn play_state(app: App<'_>) -> PlayState {
    app.play_state()
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
