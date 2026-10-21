//! The commands the UI invokes (`@tauri-apps/api` `invoke`). Each is a thin wrapper over
//! [`crate::core::Core`]; errors reach the UI as readable strings.

use std::sync::Arc;

use serde::Serialize;
use tauri::State;

use crate::{
    api::{AccountInfo, ServerInfo},
    core::{Core, InstallRequest, InstallStatus, LogoutResult, PlayState, ProfileView},
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
pub fn profile_add(app: App<'_>, name: String, url: String) -> Res<ProfileView> {
    app.add_profile(&name, &url).map_err(err)
}

#[tauri::command]
pub fn profile_update(app: App<'_>, id: String, name: String, url: String) -> Res<ProfileView> {
    app.update_profile(&id, &name, &url).map_err(err)
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
