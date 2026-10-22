//! Poe2 launcher backend: server profiles and login, game install from Steam depots, hook
//! deploy and launch, mods. `core` is the service layer; `commands` exposes it to the UI.

pub mod api;
pub mod commands;
pub mod core;
pub mod depot;
pub mod desktop;
pub mod host;
pub mod host_ops;
pub mod host_run;
pub mod keystore;
pub mod mods;
pub mod paths;
pub mod play;
pub mod profiles;
pub mod settings;
pub mod steam;
pub mod verify;
pub mod version;

use std::sync::Arc;

use tauri::{Emitter, Manager};

/// What to do with the launcher window when the game's state changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowAction {
    None,
    Minimize,
    Restore,
}

/// "Minimize the launcher while the game is running": minimize when the game starts running
/// (if the setting is on), bring the window back when it ends, but only if the launcher is the
/// one that minimized it.
pub fn window_action(
    play_status: &serde_json::Value,
    minimize_setting: bool,
    minimized_by_us: bool,
) -> WindowAction {
    match play_status.get("state").and_then(|s| s.as_str()) {
        Some("running") if minimize_setting => WindowAction::Minimize,
        Some("exited" | "idle") if minimized_by_us => WindowAction::Restore,
        _ => WindowAction::None,
    }
}

/// Sends events to the UI window, and minimizes or restores it around a game session.
struct TauriEvents {
    app: tauri::AppHandle,
    minimized_by_us: std::sync::atomic::AtomicBool,
}

impl TauriEvents {
    fn new(app: tauri::AppHandle) -> Self {
        Self {
            app,
            minimized_by_us: std::sync::atomic::AtomicBool::new(false),
        }
    }

    fn follow_game(&self, status: &serde_json::Value) {
        use std::sync::atomic::Ordering::SeqCst;
        let setting = self
            .app
            .try_state::<Arc<core::Core>>()
            .and_then(|c| c.settings().ok())
            .is_some_and(|s| s.minimize_while_playing);
        let Some(window) = self.app.get_webview_window("main") else {
            return;
        };
        match window_action(status, setting, self.minimized_by_us.load(SeqCst)) {
            WindowAction::None => {}
            WindowAction::Minimize => {
                if window.minimize().is_ok() {
                    self.minimized_by_us.store(true, SeqCst);
                }
            }
            WindowAction::Restore => {
                self.minimized_by_us.store(false, SeqCst);
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }
    }
}

impl core::Events for TauriEvents {
    fn emit(&self, event: &str, payload: serde_json::Value) {
        if event == "play:status" {
            self.follow_game(&payload);
        }
        let _ = self.app.emit(event, payload);
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // The Wayland app_id (and X11 WM_CLASS name) of the window is GTK's program name, by default
    // the executable name. The compositor takes the window's icon from the .desktop file that
    // matches it (desktop.rs), so it is the app identifier. Before GTK starts.
    #[cfg(target_os = "linux")]
    glib::set_prgname(Some(desktop::APP_ID));
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init());
    // Self-update (plugins.updater in tauri.conf.json) and the restart after it.
    #[cfg(desktop)]
    let builder = builder
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init());
    builder
        .setup(|app| {
            let dirs = paths::Dirs::from_env()?;
            let core = core::Core::new(
                dirs,
                Arc::new(keystore::KeyringStore::new()),
                Arc::new(TauriEvents::new(app.handle().clone())),
            )?;
            // "Start with the launcher" for a hosted server.
            let c = core.clone();
            tauri::async_runtime::spawn(async move { c.host_autostart().await });
            // The window starts hidden (tauri.conf.json): the frame choice and the icon are
            // applied first, so there is no flash of the wrong frame.
            if let Some(window) = app.get_webview_window("main") {
                let custom = core.settings().map(|s| s.custom_frame).unwrap_or(true);
                if !custom {
                    let _ = window.set_decorations(true);
                }
                if let Some(icon) = app.default_window_icon() {
                    let _ = window.set_icon(icon.clone());
                }
                let _ = window.show();
            }
            app.manage(core);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::game_version_set,
            commands::steam_login_status,
            commands::steam_logout,
            commands::desktop_entry_status,
            commands::desktop_entry_install,
            commands::desktop_entry_uninstall,
            commands::profiles_list,
            commands::profile_add,
            commands::profile_update,
            commands::profile_remove,
            commands::profile_confirm_plain_http,
            commands::account_refresh,
            commands::web_admin_open,
            commands::host_status,
            commands::host_find_release,
            commands::host_install_latest,
            commands::host_install_local,
            commands::host_lan_ip,
            commands::host_public_ip,
            commands::host_setup,
            commands::host_start,
            commands::host_stop,
            commands::host_log_tail,
            commands::host_set_autostart,
            commands::host_open_guide,
            commands::server_info,
            commands::account_login,
            commands::account_register,
            commands::account_logout,
            commands::settings_get,
            commands::settings_set,
            commands::install_status,
            commands::install_start,
            commands::install_input,
            commands::install_cancel,
            commands::install_verify,
            commands::mods_list,
            commands::mod_install,
            commands::mod_enable,
            commands::mod_disable,
            commands::mod_move,
            commands::mod_auto_order,
            commands::mod_remove,
            commands::mods_dir,
            commands::play,
            commands::play_state,
            commands::app_info,
        ])
        .build(tauri::generate_context!())
        .expect("error while building the launcher")
        .run(|app, event| {
            // A hosted server lives and dies with the launcher.
            if let tauri::RunEvent::Exit = event
                && let Some(core) = app.try_state::<Arc<core::Core>>()
            {
                host_ops::stop_on_exit(core.inner());
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_window_follows_the_game_only_when_asked() {
        let running = json!({"state": "running", "pid": 5});
        let exited = json!({"state": "exited", "code": 0});
        let preparing = json!({"state": "preparing", "step": "x"});
        // Setting off: never minimizes, and nothing to restore.
        assert_eq!(window_action(&running, false, false), WindowAction::None);
        assert_eq!(window_action(&exited, false, false), WindowAction::None);
        // Setting on: minimize at running, not while preparing.
        assert_eq!(window_action(&preparing, true, false), WindowAction::None);
        assert_eq!(window_action(&running, true, false), WindowAction::Minimize);
        // Restore after our minimize, even when the setting was switched off meanwhile; a game
        // that was not started with the setting on does not pop the window up.
        assert_eq!(window_action(&exited, true, true), WindowAction::Restore);
        assert_eq!(window_action(&exited, false, true), WindowAction::Restore);
        assert_eq!(
            window_action(&json!({"state": "idle"}), true, true),
            WindowAction::Restore
        );
        assert_eq!(window_action(&exited, true, false), WindowAction::None);
        assert_eq!(window_action(&json!({}), true, true), WindowAction::None);
    }
}
