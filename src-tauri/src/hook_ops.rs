//! The "Client hook" operations of [`Core`]: find the newest hook release for the selected game
//! version, download it, verify it (archive SHA-256 against the release's `.sha256`, then every
//! file against `release.json`), install it below the launcher's data folder and tell Play
//! where the DLLs are. The pure parts are in [`crate::hook`].

use std::path::PathBuf;

use anyhow::{Result, bail};
use serde::Serialize;
use serde_json::json;

use crate::{
    core::Core,
    hook::{self, HookDirs, HookRelease, InstalledHook},
    host,
    paths::reject_steam_paths,
    play,
    settings::Settings,
    version::VersionFile,
};

/// Where the DLLs Play deploys come from.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DllSource {
    /// The "Hook DLL directory" setting (advanced override); wins over everything.
    Manual,
    /// A release downloaded by the launcher.
    Downloaded { version: String },
    /// Files in the launcher's old default folder `<data>/hook-dlls` (copied there by hand).
    Legacy,
    /// Nothing usable yet.
    Missing,
}

#[derive(Debug, Clone, Serialize)]
pub struct HookStatus {
    /// The game version the hook is matched against: the selected one, or for Official
    /// (read-only) the newest private version (the same DLL, in official mode).
    pub game_version: String,
    pub repo: &'static str,
    pub source: DllSource,
    /// Where Play takes the DLLs from.
    pub dll_dir: PathBuf,
    /// The "Hook DLL directory" setting, when set.
    pub manual_dir: Option<PathBuf>,
    /// The downloaded hook that serves `game_version` (also when a manual directory overrides it).
    pub installed: Option<InstalledHook>,
    /// Downloaded hooks that do not serve this game version (kept for the others).
    pub other_installed: Vec<InstalledHook>,
    pub busy: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct HookCheck {
    pub release: HookRelease,
    /// The release is newer than the downloaded hook (or none is downloaded).
    pub update_available: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct HookOutcome {
    pub hook: InstalledHook,
    /// The archive's SHA-256 (absent when the installed hook was already the newest).
    pub sha256: Option<String>,
    /// `download`, or `current` when nothing newer was there.
    pub source: String,
}

/// Held while the hook is downloaded and installed.
struct BusyGuard<'a>(&'a std::sync::Mutex<bool>);
impl Drop for BusyGuard<'_> {
    fn drop(&mut self) {
        *self.0.lock().unwrap() = false;
    }
}

impl Core {
    fn hook_dirs(&self) -> HookDirs {
        HookDirs::new(&self.dirs)
    }

    /// The game version a hook is looked up for: the selected private version; Official
    /// (read-only) uses the same DLLs, matched against the newest version the launcher knows.
    pub fn hook_game_version(&self) -> String {
        let v = self.version();
        if v.is_official() {
            VersionFile::latest().map_or(v.version, |l| l.version)
        } else {
            v.version
        }
    }

    /// Where Play takes the hook DLLs from, and what that is. A manual directory wins; else the
    /// newest downloaded hook for the game version; else the old default folder when both DLLs
    /// are in it; else (nothing) that folder, so the error names a place.
    pub fn hook_dll_dir(&self, s: &Settings) -> (PathBuf, DllSource) {
        if let Some(d) = s.dll_dir.clone().filter(|p| !p.as_os_str().is_empty()) {
            return (d, DllSource::Manual);
        }
        let installed = hook::list_installed(&self.hook_dirs().packages());
        if let Some(h) = hook::active_for(&installed, &self.hook_game_version()) {
            let version = h.version.clone();
            return (h.dir, DllSource::Downloaded { version });
        }
        let legacy = self.dirs.hook_dlls_dir();
        if hook::REQUIRED_FILES
            .iter()
            .all(|n| legacy.join(n).is_file())
        {
            return (legacy, DllSource::Legacy);
        }
        (legacy, DllSource::Missing)
    }

    pub fn hook_status(&self) -> Result<HookStatus> {
        let s = self.settings()?;
        let game = self.hook_game_version();
        let (dll_dir, source) = self.hook_dll_dir(&s);
        let all = hook::list_installed(&self.hook_dirs().packages());
        let installed = hook::active_for(&all, &game);
        let other_installed = all
            .into_iter()
            .filter(|h| !h.game_versions.contains(&game))
            .collect();
        Ok(HookStatus {
            game_version: game,
            repo: hook::REPO,
            source,
            dll_dir,
            manual_dir: s.dll_dir.filter(|p| !p.as_os_str().is_empty()),
            installed,
            other_installed,
            busy: *self.hook_busy.lock().unwrap(),
        })
    }

    /// Before a deploy: the DLLs exist, and a downloaded hook is still what was verified.
    pub async fn check_hook(&self, s: &Settings) -> Result<()> {
        let (dir, source) = self.hook_dll_dir(s);
        if let DllSource::Downloaded { version } = source {
            let d = dir.clone();
            tokio::task::spawn_blocking(move || hook::verify_package(&d))
                .await?
                .map_err(|e| {
                    anyhow::anyhow!(
                        "the downloaded client hook {version} is damaged ({e:#}); download it again (Install page)"
                    )
                })?;
        }
        play::check_dlls(&dir)
    }

    fn hook_begin(&self) -> Result<BusyGuard<'_>> {
        if matches!(
            self.play_state(),
            crate::core::PlayState::Preparing { .. } | crate::core::PlayState::Running { .. }
        ) {
            bail!("the game is starting or running: close it before changing the client hook");
        }
        let mut busy = self.hook_busy.lock().unwrap();
        if *busy {
            bail!("the client hook is already being downloaded");
        }
        *busy = true;
        Ok(BusyGuard(&self.hook_busy))
    }

    fn hook_step(&self, step: &str) {
        self.emit("hook:step", json!({"step": step}));
    }

    /// The newest hook release for the selected game version, and whether it is newer than the
    /// downloaded one.
    pub async fn hook_check(&self) -> Result<HookCheck> {
        self.hook_check_at(host::API_BASE).await
    }

    pub async fn hook_check_at(&self, api_base: &str) -> Result<HookCheck> {
        let releases = hook::fetch_releases(api_base).await?;
        let release = hook::pick_release(&releases, &self.hook_game_version())?;
        let status = self.hook_status()?;
        let update_available = match status.installed {
            None => true,
            Some(h) => match (
                semver::Version::parse(&h.version),
                semver::Version::parse(&release.version),
            ) {
                (Ok(have), Ok(new)) => new > have,
                _ => true,
            },
        };
        Ok(HookCheck {
            release,
            update_available,
        })
    }

    /// Finds, downloads, verifies and installs the newest hook for the selected game version.
    pub async fn hook_install_latest(&self) -> Result<HookOutcome> {
        self.hook_install_latest_from(host::API_BASE).await
    }

    pub async fn hook_install_latest_from(&self, api_base: &str) -> Result<HookOutcome> {
        let check = self.hook_check_at(api_base).await?;
        let release = check.release;
        let _busy = self.hook_begin()?;
        if !check.update_available
            && let Some(current) = self.hook_status()?.installed
        {
            return Ok(HookOutcome {
                hook: current,
                sha256: None,
                source: "current".into(),
            });
        }
        let dirs = self.hook_dirs();
        reject_steam_paths(&dirs.packages(), "client hook folder")?;
        self.hook_step(&format!(
            "looking at the checksum of {}",
            release.asset_name
        ));
        let expected = host::parse_sha256_file(
            &host::download_text(&release.sha256_url).await?,
            &release.asset_name,
        )?;
        let part = dirs
            .downloads()
            .join(format!("{}.part", release.asset_name));
        self.hook_step(&format!("downloading {}", release.asset_name));
        let events = self.events.clone();
        let (got, bytes) = host::download(&release.asset_url, &part, &move |done, total| {
            events.emit(
                "hook:progress",
                json!({"phase": "download", "done": done, "total": total}),
            );
        })
        .await?;
        if got != expected || (release.size != 0 && bytes != release.size) {
            let _ = tokio::fs::remove_file(&part).await;
            bail!(
                "the downloaded client hook does not match its checksum (expected {expected}, got {got}); \
                 it was deleted and nothing was installed"
            );
        }
        let archive = dirs.downloads().join(&release.asset_name);
        tokio::fs::rename(&part, &archive).await?;
        self.hook_step("checksum matches; checking every file and unpacking");
        let (a, packages, game) = (archive.clone(), dirs.packages(), self.hook_game_version());
        let installed =
            tokio::task::spawn_blocking(move || hook::install_package(&a, &packages, &game)).await;
        let _ = tokio::fs::remove_file(&archive).await;
        let installed = installed??;
        if installed.version != release.version {
            // The archive said one thing in its name and another inside: not a release we made.
            let _ = std::fs::remove_dir_all(&installed.dir);
            bail!(
                "the package {} contains hook version {}; it was removed",
                release.asset_name,
                installed.version
            );
        }
        hook::prune_packages(&dirs.packages(), &installed);
        self.emit("hook:installed", json!({"version": installed.version}));
        Ok(HookOutcome {
            hook: installed,
            sha256: Some(got),
            source: "download".into(),
        })
    }
}
