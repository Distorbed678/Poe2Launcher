//! Poe2 launcher backend: server profiles and login, game install from Steam depots, hook
//! deploy and launch, mods. `core` is the service layer; `commands` exposes it to the UI.

pub mod api;
pub mod commands;
pub mod core;
pub mod depot;
pub mod keystore;
pub mod mods;
pub mod paths;
pub mod play;
pub mod profiles;
pub mod settings;
pub mod verify;
pub mod version;

use std::sync::Arc;

use tauri::{Emitter, Manager};

/// Sends events to the UI window.
struct TauriEvents(tauri::AppHandle);

impl core::Events for TauriEvents {
    fn emit(&self, event: &str, payload: serde_json::Value) {
        let _ = self.0.emit(event, payload);
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default().plugin(tauri_plugin_dialog::init());
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
                Arc::new(TauriEvents(app.handle().clone())),
            )?;
            app.manage(core);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::profiles_list,
            commands::profile_add,
            commands::profile_update,
            commands::profile_remove,
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
        .run(tauri::generate_context!())
        .expect("error while running the launcher");
}
