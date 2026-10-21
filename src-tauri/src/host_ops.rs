//! The "Host a server" operations of [`Core`]: get a server package (GitHub release or a local
//! file or folder), write its config from the wizard's answers, make the admin account, start
//! and stop it, report its state. The pure parts are in [`crate::host`], the process in
//! [`crate::host_run`].

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
    host::{self, HostDirs, HostSettings, PackageInfo, ServerRelease, config_path, local_web_url},
    host_run::{RunState, StartSpec, run_oneshot_stdin},
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

/// The wizard's answers. The admin password is typed in the UI, handed once to
/// `poe-server account create --password-stdin` and wiped; it is never stored, logged, or put
/// on a command line.
#[derive(Clone, Deserialize)]
pub struct SetupRequest {
    pub settings: HostSettings,
    /// Empty = keep the existing accounts (re-applying changed settings).
    #[serde(default)]
    pub admin_password: String,
}

impl std::fmt::Debug for SetupRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SetupRequest")
            .field("settings", &self.settings)
            .field("admin_password", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SetupResult {
    pub profile: ProfileView,
    /// The admin name was taken already: it was promoted, its password is unchanged.
    pub admin_existed: bool,
    pub config_file: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
pub struct InstallOutcome {
    pub package: PackageInfo,
    /// The archive's SHA-256 (absent when a folder was used).
    pub sha256: Option<String>,
    /// The SHA-256 matched the `.sha256` file that belongs to the archive.
    pub checksum_verified: bool,
    pub source: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct HostStatus {
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
    pub web_url: String,
    pub lan_ip: Option<String>,
    pub log_file: PathBuf,
    pub busy: bool,
    pub hosting_guide_url: &'static str,
}

impl Core {
    fn host_dirs(&self) -> HostDirs {
        HostDirs::new(&self.dirs)
    }

    pub fn host_settings(&self) -> Result<HostSettings> {
        HostSettings::load(self.host_dirs().settings_file())
    }

    fn host_save(&self, s: &HostSettings) -> Result<()> {
        s.save(self.host_dirs().settings_file())
    }

    fn host_begin(&self) -> Result<BusyGuard<'_>> {
        let mut busy = self.host_busy.lock().unwrap();
        if *busy {
            bail!("another server task (download, install or setup) is still running");
        }
        *busy = true;
        Ok(BusyGuard(&self.host_busy))
    }

    fn host_step(&self, step: &str) {
        self.emit("host:setup", json!({"step": step}));
        self.host.note(self.events.as_ref(), step);
    }

    fn host_os(&self) -> &'static str {
        std::env::consts::OS
    }

    /// The package the settings point at, checked.
    fn host_package(&self, s: &HostSettings) -> Result<PackageInfo> {
        let dir = s
            .package_dir
            .as_ref()
            .context("no server package is installed yet: download or pick one first")?;
        host::inspect_package(dir, &self.version.version, self.host_os())
    }

    pub async fn host_status(&self) -> Result<HostStatus> {
        let settings = self.host_settings()?;
        let (package, package_problem) = match &settings.package_dir {
            None => (None, None),
            Some(_) => match self.host_package(&settings) {
                Ok(p) => (Some(p), None),
                Err(e) => (None, Some(format!("{e:#}"))),
            },
        };
        let install_dir = self.install_dir()?;
        let run = self.host.state();
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
        Ok(HostStatus {
            package,
            package_problem,
            platform: self.host_os().to_owned(),
            game_version: self.version.version.clone(),
            game_installed: verify::looks_installed(&install_dir, &self.version),
            install_dir,
            run,
            info,
            web_url,
            lan_ip: host::detect_lan_ip().map(|ip| ip.to_string()),
            log_file: self.host_dirs().log_file(),
            busy: *self.host_busy.lock().unwrap(),
            hosting_guide_url: host::HOSTING_GUIDE_URL,
            settings,
        })
    }

    pub fn host_log_tail(&self, n: usize) -> Vec<String> {
        self.host.tail(n.min(crate::host_run::LOG_LINES))
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
        let releases = host::fetch_releases(api_base).await?;
        host::pick_release(&releases, &self.version.version, self.host_os())
    }

    /// Finds, downloads, verifies (SHA-256 against the release's `.sha256`) and installs the
    /// newest matching server package.
    pub async fn host_install_latest(&self) -> Result<InstallOutcome> {
        self.host_install_latest_from(host::API_BASE).await
    }

    pub async fn host_install_latest_from(&self, api_base: &str) -> Result<InstallOutcome> {
        let release = self.host_find_release_at(api_base).await?;
        let _busy = self.host_begin()?;
        self.ensure_host_stopped()?;
        if let Ok(current) = self.host_package(&self.host_settings()?)
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
        let dirs = self.host_dirs();
        reject_steam_paths(&dirs.packages(), "server folder")?;
        self.host_step(&format!(
            "looking at the checksum of {}",
            release.asset_name
        ));
        let expected = host::parse_sha256_file(
            &host::download_text(&release.sha256_url).await?,
            &release.asset_name,
        )?;
        let archive = dirs
            .downloads()
            .join(format!("{}.part", release.asset_name));
        self.host_step(&format!("downloading {}", release.asset_name));
        let events = self.events.clone();
        let (got, bytes) = host::download(&release.asset_url, &archive, &move |done, total| {
            events.emit(
                "host:progress",
                json!({"phase": "download", "done": done, "total": total}),
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
        self.host_step("checksum matches; unpacking");
        let out = self.host_adopt(&named, Some(got), true, "download").await;
        let _ = tokio::fs::remove_file(&named).await;
        out
    }

    /// Installs a package the user has: a `.tar.gz` / `.zip` (checked against a `.sha256` file
    /// next to it when there is one) or an unpacked package folder.
    pub async fn host_install_local(&self, path: &str) -> Result<InstallOutcome> {
        let source = PathBuf::from(path);
        let _busy = self.host_begin()?;
        self.ensure_host_stopped()?;
        reject_steam_paths(&source, "package")?;
        if source.is_dir() {
            self.host_step("copying the package folder");
            return self.host_adopt(&source, None, false, "folder").await;
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
        self.host_step(if verified {
            "checksum matches; unpacking"
        } else {
            "no .sha256 file next to the package: unpacking without a check"
        });
        self.host_adopt(&source, Some(sha), verified, "file").await
    }

    async fn host_adopt(
        &self,
        source: &Path,
        sha256: Option<String>,
        checksum_verified: bool,
        how: &str,
    ) -> Result<InstallOutcome> {
        let dirs = self.host_dirs();
        let (src, packages, game, os) = (
            source.to_path_buf(),
            dirs.packages(),
            self.version.version.clone(),
            self.host_os(),
        );
        let package =
            tokio::task::spawn_blocking(move || host::install_package(&src, &packages, &game, os))
                .await??;
        let mut s = self.host_settings()?;
        s.package_dir = Some(package.dir.clone());
        self.host_save(&s)?;
        host::prune_packages(&dirs.packages(), &package.dir);
        if s.configured {
            // An update: the package changed, so the config in it is written again from the
            // saved answers (the database lives outside the package and stays).
            self.host_write_configs(&s, &package)
                .context("the new package is installed, but its config could not be written")?;
        }
        self.emit("host:installed", json!({"package": package.name}));
        Ok(InstallOutcome {
            package,
            sha256,
            checksum_verified,
            source: how.to_owned(),
        })
    }

    fn ensure_host_stopped(&self) -> Result<()> {
        if self.host.is_running() {
            bail!("stop the server first");
        }
        Ok(())
    }

    // --- config and setup ----------------------------------------------------------------

    /// Writes `config/server.toml`.
    fn host_write_configs(&self, s: &HostSettings, pkg: &PackageInfo) -> Result<PathBuf> {
        let example = std::fs::read_to_string(pkg.dir.join("config").join("server.example.toml"))
            .context("reading the package's config/server.example.toml")?;
        let game_dir = self.install_dir()?;
        let dirs = self.host_dirs();
        std::fs::create_dir_all(dirs.data())?;
        let db = dirs.database();
        let final_cfg = config_path(&pkg.dir);
        write_atomic(
            &final_cfg,
            host::render_config(&example, s, &game_dir, &db)?.as_bytes(),
        )?;
        Ok(final_cfg)
    }

    fn host_spec(&self, pkg: &PackageInfo, config: &str) -> StartSpec {
        StartSpec {
            exe: pkg.dir.join(host::server_exe_name(self.host_os())),
            cwd: pkg.dir.clone(),
            config: PathBuf::from("config").join(config),
            log_file: self.host_dirs().log_file(),
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

    /// Applies the wizard: writes the config, makes (or promotes) the admin account when a
    /// password is given, and adds the server to Servers. The server must be stopped.
    ///
    /// The account is made with the one-shot commands `poe-server account create <name>
    /// --password-stdin` (the password goes to the child's standard input, which is then closed)
    /// and `poe-server account role <name> admin`, against the package folder and its database.
    /// The server is not started; the password is on no command line.
    pub async fn host_setup(&self, mut req: SetupRequest) -> Result<SetupResult> {
        let password = std::mem::take(&mut req.admin_password);
        let result = self.host_setup_inner(req, &password).await;
        // Drop the plain text copy as early as possible.
        let mut bytes = password.into_bytes();
        for b in bytes.iter_mut() {
            *b = 0;
        }
        result
    }

    async fn host_setup_inner(&self, req: SetupRequest, password: &str) -> Result<SetupResult> {
        let _busy = self.host_begin()?;
        self.ensure_host_stopped()?;
        let stored = self.host_settings()?;
        let mut s = HostSettings {
            package_dir: stored.package_dir.clone(),
            profile_id: stored.profile_id.clone(),
            configured: stored.configured,
            ..req.settings
        };
        s.validate()?;
        let make_admin = !password.is_empty();
        if make_admin {
            if s.admin_name.is_empty() {
                bail!("enter a name for the admin account");
            }
            if password.chars().count() < 8 || password.chars().count() > 256 {
                bail!("the admin password is 8 to 256 characters");
            }
            // It travels as one line on the server's standard input.
            if password.contains(['\n', '\r']) {
                bail!("the admin password cannot contain a line break");
            }
        } else if !s.configured {
            bail!("enter a password for the admin account");
        }
        let pkg = self.host_package(&s)?;
        let install = self.install_dir()?;
        if !verify::looks_installed(&install, &self.version) {
            bail!(
                "the game is not installed in {}: the server reads its game data from there. \
                 Install it first (Install page)",
                install.display()
            );
        }
        let busy = host::busy_ports(&s);
        if !busy.is_empty() {
            bail!(
                "these ports are already in use: {}. Choose other ones",
                busy.join(", ")
            );
        }

        self.host_step("writing the server config");
        let config_file = self.host_write_configs(&s, &pkg)?;
        let mut admin_existed = false;
        if make_admin {
            admin_existed = self.host_make_admin(&s, &pkg, password).await?;
        }
        s.configured = true;
        let profile = self.host_ensure_profile(&mut s)?;
        self.host_save(&s)?;
        self.host_step("done");
        Ok(SetupResult {
            profile,
            admin_existed,
            config_file,
        })
    }

    /// Creates the admin account and gives it the admin role, with one-shot `poe-server`
    /// commands (no server start). Returns whether the account existed already: it is then
    /// only promoted and its password stays.
    async fn host_make_admin(
        &self,
        s: &HostSettings,
        pkg: &PackageInfo,
        password: &str,
    ) -> Result<bool> {
        let spec = self.host_spec(pkg, "server.toml");
        self.host_step(&format!("creating the account {}", s.admin_name));
        // One line on the child's standard input; wiped as soon as it is written.
        let mut line = Vec::with_capacity(password.len() + 1);
        line.extend_from_slice(password.as_bytes());
        line.push(b'\n');
        let created = run_oneshot_stdin(
            &spec.exe,
            &spec.cwd,
            &spec.config,
            &["account", "create", &s.admin_name, "--password-stdin"],
            Some(&line),
        )
        .await;
        line.iter_mut().for_each(|b| *b = 0);
        let existed = match created {
            Ok(_) => false,
            // The server answers with the account error's text.
            Err(e) if e.to_string().contains("that username is taken") => true,
            Err(e) => bail!("creating the admin account failed: {e:#}"),
        };
        self.host_step(&format!("giving {} the admin role", s.admin_name));
        run_oneshot_stdin(
            &spec.exe,
            &spec.cwd,
            &spec.config,
            &["account", "role", &s.admin_name, "admin"],
            None,
        )
        .await?;
        Ok(existed)
    }

    // --- run ------------------------------------------------------------------------------

    pub async fn host_start(&self) -> Result<()> {
        let s = self.host_settings()?;
        if !s.configured {
            bail!("set the server up first");
        }
        if self.host.is_running() {
            bail!("the server is already running");
        }
        let pkg = self.host_package(&s)?;
        let install = self.install_dir()?;
        if !verify::looks_installed(&install, &self.version) {
            bail!(
                "the game is not installed in {}: install it first (Install page)",
                install.display()
            );
        }
        if !config_path(&pkg.dir).is_file() {
            self.host_write_configs(&s, &pkg)?;
        }
        let busy = host::busy_ports(&s);
        if !busy.is_empty() {
            bail!("these ports are already in use: {}", busy.join(", "));
        }
        self.host
            .start(self.events.clone(), &self.host_spec(&pkg, "server.toml"))?;
        Ok(())
    }

    pub async fn host_stop(&self) -> Result<()> {
        self.host.stop(Duration::from_secs(20)).await
    }

    /// Start with the launcher when the user asked for that. Failures go to the server log.
    pub async fn host_autostart(&self) {
        let Ok(s) = self.host_settings() else { return };
        if !(s.autostart && s.configured) {
            return;
        }
        if let Err(e) = self.host_start().await {
            self.host
                .note(self.events.as_ref(), &format!("autostart failed: {e:#}"));
        }
    }

    /// Saves the autostart choice (the only setting that can change while the server runs).
    pub fn host_set_autostart(&self, on: bool) -> Result<HostSettings> {
        let mut s = self.host_settings()?;
        s.autostart = on;
        self.host_save(&s)?;
        Ok(s)
    }
}

fn ensure_file(p: &Path) -> Result<()> {
    if !p.is_file() {
        bail!("{} is not a file or folder", p.display());
    }
    Ok(())
}

/// Used by tests and the app to stop the server on the way out.
pub fn stop_on_exit(core: &Arc<Core>) {
    core.host.stop_blocking();
}
