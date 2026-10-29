//! User settings (`settings.json`): paths and choices. Nothing secret lives here.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::paths::{Dirs, reject_steam_paths, write_atomic};

pub const DEFAULT_PROTON: &str = "Proton - Experimental";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Settings {
    /// Old single install directory (before the game version switch): read from old files and
    /// moved into `install_dirs` for the oldest supported version by [`Settings::normalize`],
    /// never written back.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub install_dir: Option<PathBuf>,
    /// Where each game version is installed, by version; a missing or empty entry means
    /// `<data>/game/<version>`.
    pub install_dirs: BTreeMap<String, PathBuf>,
    /// The game version the launcher works with (install, play, mods, server packages); empty or
    /// unknown = the newest one it supports.
    pub game_version: Option<String>,
    /// The user read the official-mode warning and agreed (asked once, before the first Play on
    /// the official servers).
    pub official_ack: bool,
    /// Minimize the launcher while the game runs and bring it back when the game ends.
    pub minimize_while_playing: bool,
    /// Draw the launcher's own title bar and window frame (default); off = the system frame.
    pub custom_frame: bool,
    /// Build output of the hook DLLs (`poe_hook.dll`, `poe_hook_core.dll`), e.g.
    /// `modules/client-hook/target/x86_64-pc-windows-msvc/release`; empty = `<data>/hook-dlls`.
    pub dll_dir: Option<PathBuf>,
    /// Linux: the Steam root (Proton is read from `<root>/steamapps/common`); empty = `~/.local/share/Steam`.
    pub steam_root: Option<PathBuf>,
    /// Linux: Proton installation, a folder name under `steamapps/common`.
    pub proton: String,
    /// The profile the Play button uses.
    pub selected_profile: Option<String>,
    /// Pre-fills the Steam user name in the install form. Not a secret.
    pub steam_username: String,
    /// Ask DepotDownloader to remember the Steam login (`-remember-password`).
    pub remember_steam_login: bool,
    /// Prefer typing a Steam Guard code over confirming in the Steam mobile app (`-no-mobile`).
    pub prefer_code_entry: bool,
    /// `dark` (default) or `light`.
    pub theme: String,
    /// What to do when a server requires mods that are not in the cache: `off` (show what is
    /// missing and block Play), `ask` (default; a dialog with Download) or `auto` (download,
    /// verify and switch without asking, code mods included).
    pub mod_sync: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            install_dir: None,
            install_dirs: BTreeMap::new(),
            game_version: None,
            custom_frame: true,
            official_ack: false,
            minimize_while_playing: false,
            dll_dir: None,
            steam_root: None,
            proton: DEFAULT_PROTON.into(),
            selected_profile: None,
            steam_username: String::new(),
            remember_steam_login: false,
            prefer_code_entry: true,
            theme: "dark".into(),
            mod_sync: "ask".into(),
        }
    }
}

impl Settings {
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str::<Self>(&text)
                .map(|s| s.normalized().without_dropped_versions())
                .with_context(|| format!("{} is not valid settings JSON", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        write_atomic(path, serde_json::to_string_pretty(self)?.as_bytes())
    }

    /// Moves an old single `install_dir` into `install_dirs` (it belongs to the oldest supported
    /// version, the only one that existed when it was written) and drops empty entries.
    pub fn normalized(mut self) -> Self {
        if let Some(dir) = self.install_dir.take()
            && let Some(oldest) = crate::version::VersionFile::known_versions().first()
        {
            self.install_dirs.entry((*oldest).to_owned()).or_insert(dir);
        }
        self.install_dirs.retain(|_, d| !d.as_os_str().is_empty());
        if self.game_version.as_deref().is_some_and(str::is_empty) {
            self.game_version = None;
        }
        self
    }

    /// Forgets what belongs to game versions this launcher no longer supports (0.5.5e since
    /// launcher 0.6.0): their install folders, and a selected one (back to the newest). Applied
    /// when the file is read, so an old file still loads and saves.
    pub fn without_dropped_versions(mut self) -> Self {
        self.install_dirs
            .retain(|v, _| crate::version::VersionFile::is_selectable(v));
        if self
            .game_version
            .as_deref()
            .is_some_and(|v| !crate::version::VersionFile::is_selectable(v))
        {
            self.game_version = None;
        }
        self
    }

    /// Refuses directories inside a Steam library, unknown themes and unknown game versions.
    pub fn validate(&self) -> Result<()> {
        for (what, dir) in [
            ("install directory", &self.install_dir),
            ("hook DLL directory", &self.dll_dir),
        ] {
            if let Some(d) = dir {
                reject_steam_paths(d, what)?;
            }
        }
        for (version, dir) in &self.install_dirs {
            anyhow::ensure!(
                crate::version::VersionFile::is_selectable(version),
                "install directory for unknown game version {version}"
            );
            reject_steam_paths(dir, "install directory")?;
        }
        if let Some(v) = &self.game_version {
            anyhow::ensure!(
                crate::version::VersionFile::is_selectable(v),
                "unsupported game version {v} (supported: official, {})",
                crate::version::VersionFile::known_versions().join(", ")
            );
        }
        if crate::modsync::SyncMode::parse(&self.mod_sync).is_none() {
            anyhow::bail!("mod sync must be off, ask or auto");
        }
        if !matches!(self.theme.as_str(), "dark" | "light") {
            anyhow::bail!("theme must be dark or light");
        }
        if self.proton.trim().is_empty() || self.proton.contains(['/', '\\']) {
            anyhow::bail!("Proton is a folder name under steamapps/common, not a path");
        }
        Ok(())
    }

    pub fn install_dir_or_default(&self, dirs: &Dirs, version: &str) -> PathBuf {
        self.install_dirs
            .get(version)
            .cloned()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| dirs.default_install_dir(version))
    }

    pub fn dll_dir_or_default(&self, dirs: &Dirs) -> PathBuf {
        self.dll_dir
            .clone()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| dirs.hook_dlls_dir())
    }

    /// The Steam root; only used to find Proton (read-only) on Linux.
    pub fn steam_root_or_default(&self) -> Result<PathBuf> {
        match self
            .steam_root
            .clone()
            .filter(|p| !p.as_os_str().is_empty())
        {
            Some(p) => Ok(p),
            None => poe_hook_deploy::launch::default_steam_root(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_gives_defaults_and_roundtrips() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("s.json");
        let mut s = Settings::load(&f).unwrap();
        assert_eq!(s, Settings::default());
        assert!(s.prefer_code_entry && !s.remember_steam_login);
        s.dll_dir = Some("/x/dlls".into());
        s.save(&f).unwrap();
        assert_eq!(Settings::load(&f).unwrap(), s);
    }

    #[test]
    fn partial_json_fills_in_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("s.json");
        std::fs::write(&f, r#"{"theme":"light"}"#).unwrap();
        let s = Settings::load(&f).unwrap();
        assert_eq!(s.theme, "light");
        assert_eq!(s.proton, DEFAULT_PROTON);
    }

    #[test]
    fn validation_refuses_steam_dirs_and_bad_values() {
        let mut s = Settings::default();
        assert!(s.validate().is_ok());
        s.install_dir = Some("/home/u/.local/share/Steam/steamapps/common/Path of Exile 2".into());
        assert!(s.validate().is_err());
        s.install_dir = None;
        s.theme = "pink".into();
        assert!(s.validate().is_err());
        s.theme = "dark".into();
        s.proton = "../Proton".into();
        assert!(s.validate().is_err());
    }

    #[test]
    fn defaults_resolve_below_the_data_dir() {
        let dirs = Dirs::under(Path::new("/r"));
        let s = Settings::default();
        assert_eq!(
            s.install_dir_or_default(&dirs, "0.5.5e.hotfix1"),
            Path::new("/r/data/game/0.5.5e.hotfix1")
        );
        assert_eq!(s.dll_dir_or_default(&dirs), Path::new("/r/data/hook-dlls"));
    }

    #[test]
    fn an_old_single_install_dir_moves_to_the_oldest_version() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("s.json");
        std::fs::write(&f, r#"{"install_dir":"/games/poe2"}"#).unwrap();
        let s = Settings::load(&f).unwrap();
        let oldest = crate::version::VersionFile::known_versions()[0];
        assert_eq!(s.install_dir, None);
        assert_eq!(s.install_dirs[oldest], Path::new("/games/poe2"));
        // Written back in the new shape only.
        s.save(&f).unwrap();
        let text = std::fs::read_to_string(&f).unwrap();
        assert!(!text.contains("\"install_dir\""), "{text}");
        assert_eq!(Settings::load(&f).unwrap(), s);
        // An explicit entry for that version wins over the old field.
        let both = Settings {
            install_dir: Some("/old".into()),
            install_dirs: BTreeMap::from([(oldest.to_owned(), PathBuf::from("/new"))]),
            ..Settings::default()
        }
        .normalized();
        assert_eq!(both.install_dirs[oldest], Path::new("/new"));
    }

    #[test]
    fn install_dirs_are_per_version() {
        let dirs = Dirs::under(Path::new("/r"));
        let s = Settings {
            install_dirs: BTreeMap::from([("0.5.5e.hotfix1".to_owned(), PathBuf::from("/games/a"))]),
            ..Settings::default()
        };
        assert_eq!(
            s.install_dir_or_default(&dirs, "0.5.5e.hotfix1"),
            Path::new("/games/a")
        );
        // Another version does not inherit it.
        assert_eq!(
            s.install_dir_or_default(&dirs, "0.6.0"),
            Path::new("/r/data/game/0.6.0")
        );
    }

    #[test]
    fn game_version_and_install_dirs_are_checked() {
        let known = crate::version::VersionFile::known_versions()[0];
        let mut s = Settings {
            game_version: Some(known.to_owned()),
            ..Settings::default()
        };
        assert!(s.validate().is_ok());
        s.game_version = Some("9.9.9".into());
        assert!(s.validate().is_err());
        s.game_version = Some("official".into());
        assert!(s.validate().is_ok());
        s.game_version = Some(String::new());
        assert_eq!(s.normalized().game_version, None, "empty means newest");
        let mut old = Settings {
            game_version: Some("0.5.5e".into()),
            ..Default::default()
        };
        old.install_dirs.insert("0.5.5e".into(), PathBuf::from("/g"));
        old.install_dirs.insert(known.to_owned(), PathBuf::from("/h"));
        let n = old.normalized().without_dropped_versions();
        assert_eq!(n.game_version, None, "a dropped version falls back to the newest");
        assert_eq!(n.install_dirs.keys().collect::<Vec<_>>(), [known]);
        n.validate().unwrap();
        let s = Settings {
            install_dirs: BTreeMap::from([("9.9.9".to_owned(), PathBuf::from("/x"))]),
            ..Settings::default()
        };
        assert!(s.validate().is_err());
        let s = Settings {
            install_dirs: BTreeMap::from([(
                known.to_owned(),
                PathBuf::from("/home/u/.local/share/Steam/steamapps/common/Path of Exile 2"),
            )]),
            ..Settings::default()
        };
        assert!(s.validate().is_err());
    }
}
