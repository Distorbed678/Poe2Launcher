//! User settings (`settings.json`): paths and choices. Nothing secret lives here.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::paths::{Dirs, reject_steam_paths, write_atomic};

pub const DEFAULT_PROTON: &str = "Proton - Experimental";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Settings {
    /// Where the game is installed; empty = `<data>/game/<version>`.
    pub install_dir: Option<PathBuf>,
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
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            install_dir: None,
            dll_dir: None,
            steam_root: None,
            proton: DEFAULT_PROTON.into(),
            selected_profile: None,
            steam_username: String::new(),
            remember_steam_login: false,
            prefer_code_entry: true,
            theme: "dark".into(),
        }
    }
}

impl Settings {
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text)
                .with_context(|| format!("{} is not valid settings JSON", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        write_atomic(path, serde_json::to_string_pretty(self)?.as_bytes())
    }

    /// Refuses directories inside a Steam library and unknown themes.
    pub fn validate(&self) -> Result<()> {
        for (what, dir) in [
            ("install directory", &self.install_dir),
            ("hook DLL directory", &self.dll_dir),
        ] {
            if let Some(d) = dir {
                reject_steam_paths(d, what)?;
            }
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
        self.install_dir
            .clone()
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
            s.install_dir_or_default(&dirs, "0.5.5e"),
            Path::new("/r/data/game/0.5.5e")
        );
        assert_eq!(s.dll_dir_or_default(&dirs), Path::new("/r/data/hook-dlls"));
    }
}
