//! The launcher's own directories, and the guard that keeps it out of the Steam install.
//!
//! Hard rule: the launcher never reads or writes the Steam install of Path of Exile 2
//! (`.../steamapps/common/Path of Exile 2`) or its Proton prefix (`compatdata/2694490`). Every
//! directory the user can choose goes through [`reject_steam_paths`].

use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};

/// Environment variable that moves every launcher directory under one root (tests, portable use).
pub const HOME_ENV: &str = "POE2_LAUNCHER_HOME";
const APP_DIR: &str = "poe2-launcher";

/// Where the launcher keeps its files.
#[derive(Debug, Clone)]
pub struct Dirs {
    /// `settings.json`, `profiles.json`.
    pub config: PathBuf,
    /// Downloaded tools, the game install, the Proton prefix, logs.
    pub data: PathBuf,
}

impl Dirs {
    pub fn from_env() -> Result<Self> {
        if let Some(home) = std::env::var_os(HOME_ENV) {
            return Ok(Self::under(Path::new(&home)));
        }
        Ok(Self {
            config: dirs::config_dir()
                .context("no configuration directory on this system")?
                .join(APP_DIR),
            data: dirs::data_dir()
                .context("no data directory on this system")?
                .join(APP_DIR),
        })
    }

    /// Both directories below `root` (`root/config`, `root/data`).
    pub fn under(root: &Path) -> Self {
        Self {
            config: root.join("config"),
            data: root.join("data"),
        }
    }

    pub fn settings_file(&self) -> PathBuf {
        self.config.join("settings.json")
    }
    pub fn profiles_file(&self) -> PathBuf {
        self.config.join("profiles.json")
    }
    /// Downloaded helper tools (DepotDownloader).
    pub fn tools_dir(&self) -> PathBuf {
        self.data.join("tools")
    }
    /// The game install for a client version, unless the user chose another directory.
    pub fn default_install_dir(&self, version: &str) -> PathBuf {
        self.data.join("game").join(version)
    }
    /// The Proton prefix for the official servers (Linux): separate from the private-server
    /// prefix, so no private login or realm choice leaks into it.
    pub fn prefix_dir_official(&self) -> PathBuf {
        self.data.join("proton-prefix-official")
    }
    /// DepotDownloader's "local data" folder (`XDG_DATA_HOME` for it on Unix): its remembered
    /// Steam login lives below it.
    pub fn steam_login_home(&self) -> PathBuf {
        self.data.join("steam-login")
    }
    /// The Proton prefix the launcher owns (Linux).
    pub fn prefix_dir(&self) -> PathBuf {
        self.data.join("proton-prefix")
    }
    pub fn crash_dump_dir(&self) -> PathBuf {
        self.data.join("crash-dumps")
    }
    /// Per-launch game output (Proton's stdout and stderr).
    pub fn logs_dir(&self) -> PathBuf {
        self.data.join("logs")
    }
    /// Fallback location for the hook DLLs when no path is configured.
    pub fn hook_dlls_dir(&self) -> PathBuf {
        self.data.join("hook-dlls")
    }
}

/// True when `path` is, or lies inside, the Steam install of PoE2 or its Proton prefix, or
/// anywhere below a Steam library's `steamapps`.
///
/// Purely lexical (case-insensitive, `.`/`..` resolved): it does not touch the file system,
/// so it is safe to call on paths that do not exist yet.
pub fn is_steam_path(path: &Path) -> bool {
    let parts = normalize(path);
    let lower: Vec<String> = parts.iter().map(|p| p.to_lowercase()).collect();
    let has_pair = |a: &str, b: &str| lower.windows(2).any(|w| w[0] == a && w[1] == b);
    lower.iter().any(|p| p == "steamapps")
        || has_pair("compatdata", "2694490")
        || lower
            .iter()
            .any(|p| p == "path of exile 2" && has_pair("common", "path of exile 2"))
}

/// Error for a directory the launcher must not use.
pub fn reject_steam_paths(path: &Path, what: &str) -> Result<()> {
    if is_steam_path(path) {
        bail!(
            "{what} {} is inside a Steam library. The launcher never reads or writes the Steam \
             install; choose a directory of its own",
            path.display()
        );
    }
    Ok(())
}

fn normalize(path: &Path) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for c in path.components() {
        match c {
            // A Windows path typed on another OS arrives as one component: split it too.
            Component::Normal(s) => out.extend(
                s.to_string_lossy()
                    .split('\\')
                    .filter(|p| !p.is_empty())
                    .map(str::to_owned),
            ),
            Component::ParentDir => {
                out.pop();
            }
            _ => {}
        }
    }
    out
}

/// Writes `bytes` to `path` through a temporary file in the same directory.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, bytes).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_steam_install_and_prefix_are_off_limits() {
        for bad in [
            "/home/u/.local/share/Steam/steamapps/common/Path of Exile 2",
            "/home/u/.local/share/Steam/steamapps/common/Path of Exile 2/Bundles2",
            "/home/u/.local/share/Steam/steamapps/compatdata/2694490/pfx",
            "/mnt/games/SteamLibrary/steamapps/anything",
            "C:\\Program Files (x86)\\Steam\\steamapps\\common\\Path of Exile 2",
            "/home/u/STEAMAPPS/common/x",
            "/home/u/data/../.local/share/Steam/steamapps/common",
            "/x/compatdata/2694490",
        ] {
            assert!(is_steam_path(Path::new(bad)), "{bad}");
            assert!(reject_steam_paths(Path::new(bad), "dir").is_err());
        }
        for good in [
            "/home/u/.local/share/poe2-launcher/game/0.5.5e.hotfix1",
            "/home/u/Games/Path of Exile 2",
            "/data/compatdata/12345",
            "relative/dir",
        ] {
            assert!(!is_steam_path(Path::new(good)), "{good}");
        }
        // `..` cannot be used to hide a Steam path or to fake an escape from one.
        assert!(is_steam_path(Path::new("/a/b/../steamapps")));
        assert!(!is_steam_path(Path::new("/a/steamapps/../b")));
    }

    #[test]
    fn dirs_live_under_one_root() {
        let d = Dirs::under(Path::new("/r"));
        assert_eq!(d.settings_file(), Path::new("/r/config/settings.json"));
        assert_eq!(
            d.default_install_dir("0.5.5e.hotfix1"),
            Path::new("/r/data/game/0.5.5e.hotfix1")
        );
        assert!(!is_steam_path(&d.prefix_dir()));
    }

    #[test]
    fn atomic_write_replaces() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("a/b.json");
        write_atomic(&f, b"1").unwrap();
        write_atomic(&f, b"22").unwrap();
        assert_eq!(std::fs::read(&f).unwrap(), b"22");
        assert!(!tmp.path().join("a/b.json.tmp").exists());
    }
}
