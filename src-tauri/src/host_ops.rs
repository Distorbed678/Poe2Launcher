//! The "Host a server" operations of [`Core`]: create and delete hosted servers (several per
//! computer, each with an id, its own folder and ports, see [`HostDirs`]), get a server package
//! (GitHub release or a local file or folder, shared by the servers), write a server's config
//! from its settings, start and stop it, report its state and usage. The pure parts are in
//! [`crate::host`], the process in [`crate::host_run`].
//!
//! The launcher never handles an account password here: a new server's first admin is created
//! in its web admin (`/setup`, only from this computer), which "Open web admin" leads to.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    api::{ApiClient, ServerInfo},
    core::{Core, ProfileView},
    host::{self, HostDirs, HostSettings, PackageInfo, ServerRelease, local_web_url},
    host_run::{HostRuntime, RunState, StartSpec},
    paths::{reject_steam_paths, write_atomic},
    verify,
};

/// Held while a download, install or setup runs; stops a second one from starting.
pub struct BusyGuard<'a>(&'a std::sync::Mutex<bool>);

impl Drop for BusyGuard<'_> {
    fn drop(&mut self) {
        *self.0.lock().unwrap() = false;
    }
}

/// The config files of the package that is being replaced: its example and the server's
/// config written from it (the user may have changed it in the web admin).
#[derive(Debug, Clone)]
pub struct PreviousConfig {
    pub example: String,
    pub config: String,
}

impl PreviousConfig {
    /// `None` when the old package has no readable `server.example.toml` or the server no config.
    pub fn read(package: &Path, config: &Path) -> Option<Self> {
        Some(Self {
            example: std::fs::read_to_string(package.join("config").join("server.example.toml"))
                .ok()?,
            config: std::fs::read_to_string(config).ok()?,
        })
    }
}

/// The settings to apply to a server.
#[derive(Debug, Clone, Deserialize)]
pub struct SetupRequest {
    pub settings: HostSettings,
}

#[derive(Debug, Clone, Serialize)]
pub struct SetupResult {
    pub profile: ProfileView,
    pub config_file: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
pub struct InstallOutcome {
    pub package: PackageInfo,
    /// The archive's SHA-256 (absent when a folder or an installed package was used).
    pub sha256: Option<String>,
    /// The SHA-256 matched the `.sha256` file that belongs to the archive.
    pub checksum_verified: bool,
    /// `download`, `file`, `folder`, `current` (already installed for this server) or
    /// `shared` (the same release was installed for another server).
    pub source: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct HostStatus {
    pub id: String,
    pub settings: HostSettings,
    pub package: Option<PackageInfo>,
    /// Why the stored package cannot be used (missing, wrong game version, ...).
    pub package_problem: Option<String>,
    pub platform: String,
    pub game_version: String,
    pub game_installed: bool,
    pub install_dir: PathBuf,
    pub run: RunState,
    /// The web API answers.
    pub info: Option<ServerInfo>,
    /// The server has no account yet, so its first admin still has to be made in the web admin
    /// (`GET /setup` from this computer); `None` while it does not answer.
    pub setup_needed: Option<bool>,
    pub web_url: String,
    pub lan_ip: Option<String>,
    pub log_file: PathBuf,
    pub folder: PathBuf,
    pub busy: bool,
    pub hosting_guide_url: &'static str,
}

/// What Manage > Overview shows under "Usage".
#[derive(Debug, Clone, Default, Serialize)]
pub struct HostMetrics {
    /// CPU of the server process in percent of one core (the launcher's own reading on Linux,
    /// else the server's); absent when unknown.
    pub cpu_pct: Option<f32>,
    /// Resident memory of the server process in bytes.
    pub rss_bytes: Option<u64>,
    /// The last readings, oldest first, for the small charts.
    pub cpu_history: Vec<f32>,
    pub rss_history: Vec<u64>,
    /// Simulation frame time of the last sample (mean and longest), milliseconds.
    pub tick_avg_ms: Option<f32>,
    pub tick_max_ms: Option<f32>,
    /// Network traffic of the game connections, bytes per second.
    pub net_in_bps: Option<u64>,
    pub net_out_bps: Option<u64>,
    pub players: Option<u32>,
    pub uptime_secs: Option<u64>,
    /// The server reports no metrics (`[metrics] enabled = false`, or an older server).
    pub server_metrics: bool,
}

/// `GET /setup`: whether the server still needs its first admin.
#[derive(Debug, Clone, Deserialize)]
struct SetupState {
    needed: bool,
}

/// The newest sample of `GET /local/metrics`.
#[derive(Debug, Clone, Deserialize)]
struct LocalMetrics {
    #[serde(default)]
    uptime_secs: u64,
    latest: Option<LocalPoint>,
}

#[derive(Debug, Clone, Deserialize)]
struct LocalPoint {
    players: u32,
    tick_avg_us: u32,
    tick_max_us: u32,
    #[serde(default)]
    rss_bytes: u64,
    cpu_pct: Option<f32>,
    #[serde(default)]
    net_in_bps: u64,
    #[serde(default)]
    net_out_bps: u64,
}

impl Core {
    // --- the servers -------------------------------------------------------------------------

    pub(crate) fn host_dirs(&self, id: &str) -> HostDirs {
        HostDirs::new(&self.dirs, id)
    }

    /// The ids of the hosted servers.
    pub fn host_ids(&self) -> Vec<String> {
        host::server_ids(&self.dirs)
    }

    /// Fails for an id that is not a hosted server.
    fn host_known(&self, id: &str) -> Result<HostDirs> {
        if !host::valid_server_id(id) || !self.host_ids().iter().any(|i| i == id) {
            bail!("there is no hosted server {id}");
        }
        Ok(self.host_dirs(id))
    }

    /// The process of server `id` (made on first use).
    pub fn runtime(&self, id: &str) -> HostRuntime {
        self.hosts
            .lock()
            .unwrap()
            .entry(id.to_owned())
            .or_insert_with(|| HostRuntime::for_server(id))
            .clone()
    }

    /// Every hosted server process that runs (or is stopping).
    pub fn running_hosts(&self) -> Vec<String> {
        self.hosts
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, r)| r.is_running())
            .map(|(id, _)| id.clone())
            .collect()
    }

    pub fn host_settings(&self, id: &str) -> Result<HostSettings> {
        HostSettings::load(self.host_known(id)?.settings_file())
    }

    pub(crate) fn host_save(&self, id: &str, s: &HostSettings) -> Result<()> {
        s.save(self.host_dirs(id).settings_file())
    }

    /// Hosting is for the private game versions; "Official (read-only)" has no server.
    pub(crate) fn host_private_only(&self) -> Result<()> {
        if self.version().is_official() {
            bail!("hosting a server needs a private game version: pick one in the version menu");
        }
        Ok(())
    }

    pub(crate) fn host_begin(&self) -> Result<BusyGuard<'_>> {
        self.host_private_only()?;
        let mut busy = self.host_busy.lock().unwrap();
        if *busy {
            bail!("another server task (download, install or setup) is still running");
        }
        *busy = true;
        Ok(BusyGuard(&self.host_busy))
    }

    pub(crate) fn host_step(&self, id: &str, step: &str) {
        self.emit("host:setup", json!({"id": id, "step": step}));
        self.runtime(id).note(self.events.as_ref(), step);
    }

    pub(crate) fn host_os(&self) -> &'static str {
        std::env::consts::OS
    }

    /// The package the settings point at, checked.
    pub(crate) fn host_package(&self, s: &HostSettings) -> Result<PackageInfo> {
        let dir = s
            .package_dir
            .as_ref()
            .context("no server package is installed yet: download or pick one first")?;
        host::inspect_package(dir, &self.version().version, self.host_os())
    }

    /// The settings of every hosted server but `except`.
    fn host_others(&self, except: &str) -> Vec<HostSettings> {
        self.host_ids()
            .into_iter()
            .filter(|i| i != except)
            .filter_map(|i| HostSettings::load(self.host_dirs(&i).settings_file()).ok())
            .collect()
    }

    /// The package folders some hosted server uses (they are never pruned or replaced).
    pub(crate) fn host_packages_in_use(&self) -> Vec<PathBuf> {
        self.host_ids()
            .into_iter()
            .filter_map(|i| HostSettings::load(self.host_dirs(&i).settings_file()).ok())
            .filter_map(|s| s.package_dir)
            .collect()
    }

    pub(crate) fn host_prune_packages(&self, id: &str) {
        host::prune_packages(&self.host_dirs(id).packages(), &self.host_packages_in_use());
    }

    /// Creates a hosted server: an id, the next free ports, the name, and the newest installed
    /// package when there is one (else the Program tab gets one). Not set up or started.
    /// Returns the new server's id.
    pub fn host_create(&self, name: &str) -> Result<String> {
        self.host_private_only()?;
        let ids = self.host_ids();
        let id = host::new_server_id(&ids);
        let others = self.host_others(&id);
        let (login, instance, web) =
            host::next_free_ports(&others, &|s| !host::busy_ports(s).is_empty())?;
        let mut s = HostSettings {
            server_name: if name.trim().is_empty() {
                format!("Server {}", &id[1..])
            } else {
                name.trim().to_owned()
            },
            login_port: login,
            instance_port: instance,
            web_port: web,
            public_host: host::detect_lan_ip()
                .map_or_else(|| "127.0.0.1".into(), |ip| ip.to_string()),
            ..HostSettings::default()
        };
        s.validate()?;
        // A release another server runs is used again (no download).
        s.package_dir = others
            .iter()
            .filter_map(|o| o.package_dir.clone())
            .filter(|d| host::inspect_package(d, &self.version().version, self.host_os()).is_ok())
            .max();
        std::fs::create_dir_all(host::servers_config_dir(&self.dirs))?;
        std::fs::create_dir_all(self.host_dirs(&id).root())?;
        self.host_save(&id, &s)?;
        Ok(id)
    }

    /// Deletes a hosted server: stops it, removes it from Servers (its login is revoked), and
    /// deletes its folder (database, config, backups, log) and settings. Packages no other
    /// server uses go too.
    pub async fn host_delete(&self, id: &str) -> Result<()> {
        let dirs = self.host_known(id)?;
        let s = HostSettings::load(dirs.settings_file())?;
        let rt = self.runtime(id);
        if rt.is_running() {
            rt.stop(crate::host_run::STOP_GRACE + Duration::from_secs(10))
                .await
                .context("the server did not stop, so nothing was deleted")?;
        }
        if let Some(p) = &s.profile_id
            && self.profiles()?.iter().any(|v| &v.id == p)
        {
            self.remove_profile(p).await?;
        }
        reject_steam_paths(dirs.root(), "server folder")?;
        if dirs.root().exists() {
            std::fs::remove_dir_all(dirs.root())
                .with_context(|| format!("deleting {}", dirs.root().display()))?;
        }
        std::fs::remove_file(dirs.settings_file())?;
        let _ = std::fs::remove_file(dirs.backup_settings_file());
        self.hosts.lock().unwrap().remove(id);
        self.host_prune_packages(id);
        Ok(())
    }

    pub async fn host_status(&self, id: &str) -> Result<HostStatus> {
        let settings = self.host_settings(id)?;
        let dirs = self.host_dirs(id);
        let (package, package_problem) = match &settings.package_dir {
            None => (None, None),
            Some(_) => match self.host_package(&settings) {
                Ok(p) => (Some(p), None),
                Err(e) => (None, Some(format!("{e:#}"))),
            },
        };
        let install_dir = self.install_dir()?;
        let run = self.runtime(id).state();
        let web_url = local_web_url(&settings);
        let info = if matches!(run, RunState::Running { .. }) {
            tokio::time::timeout(
                Duration::from_millis(1500),
                ApiClient::new(&web_url).server_info(),
            )
            .await
            .ok()
            .and_then(Result::ok)
        } else {
            None
        };
        let setup_needed = if info.is_some() {
            tokio::time::timeout(
                Duration::from_millis(1500),
                ApiClient::new(&web_url).get_json::<SetupState>("/setup"),
            )
            .await
            .ok()
            .and_then(Result::ok)
            .map(|s| s.needed)
        } else {
            None
        };
        Ok(HostStatus {
            setup_needed,
            id: id.to_owned(),
            package,
            package_problem,
            platform: self.host_os().to_owned(),
            game_version: self.version().version,
            game_installed: verify::looks_installed(&install_dir, &self.version()),
            install_dir,
            run,
            info,
            web_url,
            lan_ip: host::detect_lan_ip().map(|ip| ip.to_string()),
            log_file: dirs.log_file(),
            folder: dirs.root().to_path_buf(),
            busy: *self.host_busy.lock().unwrap(),
            hosting_guide_url: host::HOSTING_GUIDE_URL,
            settings,
        })
    }

    /// Every hosted server's status (the Host tab).
    pub async fn host_list(&self) -> Result<Vec<HostStatus>> {
        let ids = self.host_ids();
        let all = futures_util::future::join_all(ids.iter().map(|id| self.host_status(id))).await;
        all.into_iter().collect()
    }

    /// CPU, memory, frame time, network and players of a running server.
    pub async fn host_metrics(&self, id: &str) -> Result<HostMetrics> {
        let s = self.host_settings(id)?;
        let rt = self.runtime(id);
        let mut m = HostMetrics::default();
        let Some(pid) = rt.pid() else {
            return Ok(m);
        };
        if let Some((cpu, rss)) = rt.probe_usage(pid) {
            m.cpu_pct = cpu;
            m.rss_bytes = Some(rss);
        }
        let api = ApiClient::with_timeouts(
            &local_web_url(&s),
            Duration::from_millis(500),
            Duration::from_millis(1200),
        );
        if let Ok(Ok(local)) = tokio::time::timeout(
            Duration::from_millis(1500),
            api.get_json::<LocalMetrics>("/local/metrics"),
        )
        .await
        {
            m.uptime_secs = Some(local.uptime_secs);
            if let Some(p) = local.latest {
                m.server_metrics = true;
                m.players = Some(p.players);
                m.tick_avg_ms = Some(p.tick_avg_us as f32 / 1000.0);
                m.tick_max_ms = Some(p.tick_max_us as f32 / 1000.0);
                m.net_in_bps = Some(p.net_in_bps);
                m.net_out_bps = Some(p.net_out_bps);
                // Where the launcher cannot read the process (Windows), the server's own reading.
                if m.rss_bytes.is_none() && p.rss_bytes > 0 {
                    m.rss_bytes = Some(p.rss_bytes);
                }
                if m.cpu_pct.is_none() {
                    m.cpu_pct = p.cpu_pct;
                }
            }
        }
        rt.record_usage(m.cpu_pct, m.rss_bytes);
        (m.cpu_history, m.rss_history) = rt.usage_history();
        Ok(m)
    }

    pub fn host_log_tail(&self, id: &str, n: usize) -> Vec<String> {
        self.runtime(id).tail(n.min(crate::host_run::LOG_LINES))
    }

    pub fn host_lan_ip(&self) -> Option<String> {
        host::detect_lan_ip().map(|ip| ip.to_string())
    }

    /// Asks an outside service for this network's public address. Only on the user's click.
    pub async fn host_public_ip(&self) -> Result<String> {
        Ok(host::lookup_public_ip(host::PUBLIC_IP_URL)
            .await?
            .to_string())
    }

    // --- getting the package -------------------------------------------------------------

    /// The newest server release for this launcher's game version and this OS.
    pub async fn host_find_release(&self) -> Result<ServerRelease> {
        self.host_find_release_at(host::API_BASE).await
    }

    pub async fn host_find_release_at(&self, api_base: &str) -> Result<ServerRelease> {
        self.host_private_only()?;
        host::find_release(api_base, &self.version().version, self.host_os()).await
    }

    /// Finds, downloads, verifies (SHA-256 against the release's `.sha256`) and installs the
    /// newest matching server package for server `id`. A release another server already has is
    /// used without a download.
    pub async fn host_install_latest(&self, id: &str) -> Result<InstallOutcome> {
        self.host_install_latest_from(id, host::API_BASE).await
    }

    pub async fn host_install_latest_from(
        &self,
        id: &str,
        api_base: &str,
    ) -> Result<InstallOutcome> {
        self.host_known(id)?;
        let release = self.host_find_release_at(api_base).await?;
        let _busy = self.host_begin()?;
        self.ensure_host_stopped(id)?;
        if let Ok(current) = self.host_package(&self.host_settings(id)?)
            && current.server_version == release.version
            && current.game_version == release.game_version
        {
            return Ok(InstallOutcome {
                package: current,
                sha256: None,
                checksum_verified: false,
                source: "current".into(),
            });
        }
        let dirs = self.host_dirs(id);
        if let Some(name) = host::package_name_of_asset(&release.asset_name) {
            let dir = dirs.packages().join(name);
            if let Ok(pkg) = host::inspect_package(&dir, &self.version().version, self.host_os()) {
                self.host_step(
                    id,
                    &format!("{} is installed already (another server uses it)", pkg.name),
                );
                return self.host_use_package(id, pkg, None, false, "shared");
            }
        }
        let (named, got) = self.host_download_verified(id, &release).await?;
        let out = self
            .host_adopt(id, &named, Some(got), true, "download")
            .await;
        let _ = tokio::fs::remove_file(&named).await;
        out
    }

    /// Downloads the release's archive into the downloads folder and checks its SHA-256
    /// against the release's `.sha256` file (and its size). Returns the archive's path and
    /// hash; on a mismatch the download is deleted. The caller holds the busy flag.
    pub(crate) async fn host_download_verified(
        &self,
        id: &str,
        release: &ServerRelease,
    ) -> Result<(PathBuf, String)> {
        let dirs = self.host_dirs(id);
        reject_steam_paths(&dirs.packages(), "server folder")?;
        self.host_step(
            id,
            &format!("looking at the checksum of {}", release.asset_name),
        );
        let expected = host::parse_sha256_file(
            &host::download_text(&release.sha256_url).await?,
            &release.asset_name,
        )?;
        let archive = dirs
            .downloads()
            .join(format!("{}.part", release.asset_name));
        self.host_step(id, &format!("downloading {}", release.asset_name));
        let events = self.events.clone();
        let sid = id.to_owned();
        let (got, bytes) = host::download(&release.asset_url, &archive, &move |done, total| {
            events.emit(
                "host:progress",
                json!({"id": sid, "phase": "download", "done": done, "total": total}),
            );
        })
        .await?;
        if got != expected || (release.size != 0 && bytes != release.size) {
            let _ = tokio::fs::remove_file(&archive).await;
            bail!(
                "the downloaded server package does not match its checksum (expected {expected}, got {got}); \
                 it was deleted and nothing was installed"
            );
        }
        // Keep the real name for the archive so the package check reads naturally.
        let named = dirs.downloads().join(&release.asset_name);
        tokio::fs::rename(&archive, &named).await?;
        self.host_step(id, "checksum matches; unpacking");
        Ok((named, got))
    }

    /// Installs a package the user has for server `id`: a `.tar.gz` / `.zip` (checked against a
    /// `.sha256` file next to it when there is one) or an unpacked package folder.
    pub async fn host_install_local(&self, id: &str, path: &str) -> Result<InstallOutcome> {
        self.host_known(id)?;
        let source = PathBuf::from(path);
        let _busy = self.host_begin()?;
        self.ensure_host_stopped(id)?;
        reject_steam_paths(&source, "package")?;
        if source.is_dir() {
            self.host_step(id, "copying the package folder");
            return self.host_adopt(id, &source, None, false, "folder").await;
        }
        ensure_file(&source)?;
        let src = source.clone();
        let sha = tokio::task::spawn_blocking(move || host::sha256_of_file(&src)).await??;
        let mut verified = false;
        if let Some(sum) = host::sibling_checksum_file(&source) {
            let name = source.file_name().unwrap().to_string_lossy().into_owned();
            let expected = host::parse_sha256_file(&std::fs::read_to_string(&sum)?, &name)?;
            if expected != sha {
                bail!(
                    "{} does not match {} (expected {expected}, got {sha}); nothing was installed",
                    source.display(),
                    sum.display()
                );
            }
            verified = true;
        }
        self.host_step(
            id,
            if verified {
                "checksum matches; unpacking"
            } else {
                "no .sha256 file next to the package: unpacking without a check"
            },
        );
        self.host_adopt(id, &source, Some(sha), verified, "file")
            .await
    }

    async fn host_adopt(
        &self,
        id: &str,
        source: &Path,
        sha256: Option<String>,
        checksum_verified: bool,
        how: &str,
    ) -> Result<InstallOutcome> {
        let dirs = self.host_dirs(id);
        let (src, packages, game, os, in_use) = (
            source.to_path_buf(),
            dirs.packages(),
            self.version().version,
            self.host_os(),
            self.host_packages_in_use(),
        );
        let package = tokio::task::spawn_blocking(move || {
            host::install_package_keeping(&src, &packages, &game, os, None, &in_use)
        })
        .await??;
        self.host_use_package(id, package, sha256, checksum_verified, how)
    }

    /// Points server `id` at `package`, writes its config again when it is set up (the user's
    /// changes to the old config carried over), and prunes the packages nobody uses.
    fn host_use_package(
        &self,
        id: &str,
        package: PackageInfo,
        sha256: Option<String>,
        checksum_verified: bool,
        how: &str,
    ) -> Result<InstallOutcome> {
        let dirs = self.host_dirs(id);
        let mut s = self.host_settings(id)?;
        let previous = s
            .package_dir
            .as_deref()
            .filter(|_| s.configured)
            .and_then(|d| PreviousConfig::read(d, &dirs.config()));
        s.package_dir = Some(package.dir.clone());
        self.host_save(id, &s)?;
        self.host_prune_packages(id);
        // An "autostart failed" note about the old (or missing) package is not true any more.
        self.runtime(id).forget_failed_starts();
        if s.configured {
            // The package changed, so the config is written again from the saved settings and
            // what the user changed in the old one (the database lives outside the package).
            self.host_write_configs_carrying(id, &s, &package, previous.as_ref())
                .context("the new package is installed, but its config could not be written")?;
        }
        self.emit("host:installed", json!({"id": id, "package": package.name}));
        Ok(InstallOutcome {
            package,
            sha256,
            checksum_verified,
            source: how.to_owned(),
        })
    }

    fn ensure_host_stopped(&self, id: &str) -> Result<()> {
        if self.runtime(id).is_running() {
            bail!("stop the server first");
        }
        Ok(())
    }

    // --- config and setup ----------------------------------------------------------------

    /// Writes the server's `server.toml` (in its own folder).
    pub(crate) fn host_write_configs(
        &self,
        id: &str,
        s: &HostSettings,
        pkg: &PackageInfo,
    ) -> Result<PathBuf> {
        let dirs = self.host_dirs(id);
        let previous = PreviousConfig::read(&pkg.dir, &dirs.config());
        self.host_write_configs_carrying(id, s, pkg, previous.as_ref())
    }

    /// Writes the server's `server.toml` from the package's `server.example.toml`, the settings
    /// and, when there is one, what the user changed in the config written before (see
    /// [`host::carry_over_config`]); settings new in this release get their defaults.
    pub(crate) fn host_write_configs_carrying(
        &self,
        id: &str,
        s: &HostSettings,
        pkg: &PackageInfo,
        previous: Option<&PreviousConfig>,
    ) -> Result<PathBuf> {
        let mut example =
            std::fs::read_to_string(pkg.dir.join("config").join("server.example.toml"))
                .context("reading the package's config/server.example.toml")?;
        if let Some(prev) = previous {
            match host::carry_over_config(&prev.example, &prev.config, &example) {
                Ok(merged) => example = merged,
                Err(e) => self.runtime(id).note(
                    self.events.as_ref(),
                    &format!("the old config could not be carried over, using the defaults: {e:#}"),
                ),
            }
        }
        let game_dir = self.install_dir()?;
        let dirs = self.host_dirs(id);
        for d in [dirs.data(), dirs.mods(), dirs.config_backups()] {
            std::fs::create_dir_all(d)?;
        }
        let cfg = dirs.config();
        write_atomic(
            &cfg,
            host::render_config(&example, s, &game_dir, &dirs)?.as_bytes(),
        )?;
        Ok(cfg)
    }

    pub(crate) fn host_spec(&self, id: &str, pkg: &PackageInfo) -> StartSpec {
        let dirs = self.host_dirs(id);
        StartSpec {
            exe: pkg.dir.join(host::server_exe_name(self.host_os())),
            cwd: pkg.dir.clone(),
            config: dirs.config(),
            log_file: dirs.log_file(),
        }
    }

    /// Server profile for this server: created, or updated if it exists.
    fn host_ensure_profile(&self, s: &mut HostSettings) -> Result<ProfileView> {
        let url = local_web_url(s);
        let name = s.server_name.trim().to_owned();
        let known = s
            .profile_id
            .as_deref()
            .and_then(|id| self.profiles().ok()?.into_iter().find(|p| p.id == id));
        let view = match known {
            Some(p) => self.update_profile(&p.id, &name, &url, false)?,
            None => match self.add_profile(&name, &url, false) {
                Ok(v) => v,
                Err(_) => self.add_profile(&format!("{name} (hosted)"), &url, false)?,
            },
        };
        s.profile_id = Some(view.id.clone());
        Ok(view)
    }

    /// Applies the settings: writes the config and adds the server to Servers (My servers).
    /// The server must be stopped. No account is made: the first admin is created in the
    /// server's web admin.
    pub async fn host_setup(&self, id: &str, req: SetupRequest) -> Result<SetupResult> {
        self.host_known(id)?;
        let _busy = self.host_begin()?;
        self.ensure_host_stopped(id)?;
        let stored = self.host_settings(id)?;
        let mut s = HostSettings {
            package_dir: stored.package_dir.clone(),
            profile_id: stored.profile_id.clone(),
            configured: stored.configured,
            ..req.settings
        };
        s.validate()?;
        for other in self.host_others(id) {
            let clash = s.port_clashes(&other);
            if !clash.is_empty() {
                bail!(
                    "{} uses port {} already: pick other ports",
                    other.server_name,
                    clash
                        .iter()
                        .map(u16::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
        }
        let pkg = self.host_package(&s)?;
        let install = self.install_dir()?;
        if !verify::looks_installed(&install, &self.version()) {
            bail!(
                "the game is not installed in {}: the server reads its game data from there. \
                 Install it first (Library)",
                install.display()
            );
        }
        let ports_changed = s.ports() != stored.ports() || !stored.configured;
        if ports_changed {
            let busy = host::busy_ports(&s);
            if !busy.is_empty() {
                bail!(
                    "these ports are already in use: {}. Choose other ones",
                    busy.join(", ")
                );
            }
        }
        self.host_step(id, "writing the server config");
        let config_file = self.host_write_configs(id, &s, &pkg)?;
        s.configured = true;
        let profile = self.host_ensure_profile(&mut s)?;
        self.host_save(id, &s)?;
        self.host_step(id, "settings applied");
        Ok(SetupResult {
            profile,
            config_file,
        })
    }

    // --- run ------------------------------------------------------------------------------

    pub async fn host_start(&self, id: &str) -> Result<()> {
        self.host_private_only()?;
        let s = self.host_settings(id)?;
        if !s.configured {
            bail!("apply the server's settings first");
        }
        let rt = self.runtime(id);
        if rt.is_running() {
            bail!("the server is already running");
        }
        let pkg = self.host_package(&s)?;
        let install = self.install_dir()?;
        if !verify::looks_installed(&install, &self.version()) {
            bail!(
                "the game is not installed in {}: install it first (Library)",
                install.display()
            );
        }
        if !self.host_dirs(id).config().is_file() {
            self.host_write_configs(id, &s, &pkg)?;
        }
        for other_id in self.running_hosts() {
            if other_id != id
                && let Ok(o) = self.host_settings(&other_id)
                && !s.port_clashes(&o).is_empty()
            {
                bail!(
                    "{} runs on the same ports: stop it or change the ports",
                    o.server_name
                );
            }
        }
        let busy = host::busy_ports(&s);
        if !busy.is_empty() {
            bail!("these ports are already in use: {}", busy.join(", "));
        }
        // The database is not in use yet: the best moment for a safe copy.
        self.backup_auto(id);
        rt.start(self.events.clone(), &self.host_spec(id, &pkg))?;
        Ok(())
    }

    pub async fn host_stop(&self, id: &str) -> Result<()> {
        // SIGTERM, up to the grace time for the server's own shutdown, then a hard kill.
        self.runtime(id)
            .stop(crate::host_run::STOP_GRACE + Duration::from_secs(10))
            .await
    }

    /// Starts the servers the user asked to start with the launcher. Failures go to their logs.
    pub async fn host_autostart(&self) {
        for id in self.host_ids() {
            let Ok(s) = self.host_settings(&id) else {
                continue;
            };
            if !(s.autostart && s.configured) {
                continue;
            }
            if let Err(e) = self.host_start(&id).await {
                self.runtime(&id)
                    .note(self.events.as_ref(), &format!("autostart failed: {e:#}"));
            }
        }
    }

    /// Saves the autostart choice (the only setting that can change while the server runs).
    pub fn host_set_autostart(&self, id: &str, on: bool) -> Result<HostSettings> {
        let mut s = self.host_settings(id)?;
        s.autostart = on;
        self.host_save(id, &s)?;
        Ok(s)
    }
}

fn ensure_file(p: &Path) -> Result<()> {
    if !p.is_file() {
        bail!("{} is not a file or folder", p.display());
    }
    Ok(())
}

/// Used by tests and the app to stop every hosted server on the way out.
pub fn stop_on_exit(core: &Arc<Core>) {
    let all: Vec<HostRuntime> = core.hosts.lock().unwrap().values().cloned().collect();
    for rt in all {
        rt.stop_blocking();
    }
}
