//! Server updates: look for a newer server release for the selected game version and install it
//! with one click.
//!
//! The check is quiet and read-only (one request to GitHub, nothing changes). The update itself
//! is the careful kind, because a hosted server holds other people's characters:
//!
//! 1. the new package is downloaded and its SHA-256 checked **while the old server keeps
//!    running**; a bad download changes nothing;
//! 2. the server is stopped gracefully (SIGTERM, the server saves its sessions) when it runs;
//! 3. the database is backed up (kind `pre-update`, the same backup feature as on the Host
//!    page);
//! 4. the new package is unpacked next to the old one (never over it) and checked;
//! 5. its `config/server.toml` is rendered from **its** `server.example.toml`, the saved
//!    answers and what the user changed in the old config ([`host::carry_over_config`]), so
//!    new settings get their defaults and nothing the user set is lost;
//! 6. the settings point at the new package and, if the server ran, it is started again and
//!    watched until its web API answers (or [`UpdateTuning::ready_wait`] has passed with the
//!    process still alive);
//! 7. only then the old package folder is deleted.
//!
//! Any failure after step 2 puts everything back: the settings point at the old package again,
//! the new folder is removed, the database comes back from the step 3 backup when the new
//! server may have touched it, and the old server is started again when it ran before.

use std::{path::Path, time::Duration};

use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::json;

use crate::{
    api::ApiClient,
    backup::{self, BackupInfo},
    core::{Core, now_secs},
    host::{self, HostSettings, PackageInfo, ServerRelease, local_web_url},
    host_ops::PreviousConfig,
    host_run::RunState,
};

/// Knobs that tests turn down.
#[derive(Debug, Clone)]
pub struct UpdateTuning {
    /// How long a restarted server may take to answer on its web API (it reads the game data
    /// first). If the process still lives when this has passed, the update counts as done.
    pub ready_wait: Duration,
}

impl Default for UpdateTuning {
    fn default() -> Self {
        Self {
            ready_wait: Duration::from_secs(90),
        }
    }
}

/// What the Host and Home pages show.
#[derive(Debug, Clone, Serialize)]
pub struct ServerUpdateCheck {
    /// The installed server version (absent: no package, or a package that does not fit).
    pub installed: Option<String>,
    /// The newest compatible release (absent: nothing installed to update, or the lookup failed).
    pub release: Option<ServerRelease>,
    pub update_available: bool,
    /// Why the lookup failed. Quiet: the UI shows it only on the Host page.
    pub error: Option<String>,
    pub checked_at: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateOutcome {
    /// False: the installed server is already the newest one; nothing was touched.
    pub updated: bool,
    pub from: String,
    pub to: String,
    pub package: PackageInfo,
    /// The server was running and was stopped for the update.
    pub was_running: bool,
    /// ... and is running the new version now.
    pub restarted: bool,
    /// The database backup made first (absent: the server had no database yet).
    pub backup: Option<String>,
    pub sha256: Option<String>,
}

impl Core {
    pub async fn host_update_check(&self) -> ServerUpdateCheck {
        self.host_update_check_at(host::API_BASE).await
    }

    /// Asks GitHub whether a newer server release than the installed one exists. Never an
    /// error: a failure is in the `error` field. With no usable package there is nothing to
    /// update and GitHub is not asked.
    pub async fn host_update_check_at(&self, api_base: &str) -> ServerUpdateCheck {
        let mut out = ServerUpdateCheck {
            installed: None,
            release: None,
            update_available: false,
            error: None,
            checked_at: now_secs(),
        };
        if self.host_private_only().is_err() || host::platform_for(self.host_os()).is_err() {
            return out;
        }
        let Ok(settings) = self.host_settings() else {
            return out;
        };
        let Ok(current) = self.host_package(&settings) else {
            return out;
        };
        out.installed = Some(current.server_version.clone());
        match self.host_find_release_at(api_base).await {
            Ok(r) => {
                out.update_available = host::is_newer(&current.server_version, &r.version);
                out.release = Some(r);
            }
            Err(e) => out.error = Some(format!("{e:#}")),
        }
        out
    }

    pub async fn host_update_apply(&self) -> Result<UpdateOutcome> {
        self.host_update_apply_with(host::API_BASE, &UpdateTuning::default())
            .await
    }

    /// The one-click update (see the module comment). Fails with the old server restored.
    pub async fn host_update_apply_with(
        &self,
        api_base: &str,
        tuning: &UpdateTuning,
    ) -> Result<UpdateOutcome> {
        self.host_private_only()?;
        let settings = self.host_settings()?;
        let current = self
            .host_package(&settings)
            .context("there is no server to update: download or pick a package first")?;
        let release = self.host_find_release_at(api_base).await?;
        if !host::is_newer(&current.server_version, &release.version) {
            return Ok(UpdateOutcome {
                updated: false,
                from: current.server_version.clone(),
                to: current.server_version.clone(),
                package: current,
                was_running: false,
                restarted: false,
                backup: None,
                sha256: None,
            });
        }
        let _busy = self.host_begin()?;
        // 1. Download and check while the old server keeps serving.
        let (archive, sha) = self.host_download_verified(&release).await?;
        let result = self
            .update_swap(&settings, &current, &archive, sha, tuning)
            .await;
        let _ = tokio::fs::remove_file(&archive).await;
        match &result {
            Ok(o) => {
                self.host_step(&format!("updated to {}", o.to));
                self.emit("host:installed", json!({"package": o.package.name}));
            }
            Err(e) => self.host_step(&format!("the update failed: {e:#}")),
        }
        result
    }

    async fn update_swap(
        &self,
        settings: &HostSettings,
        current: &PackageInfo,
        archive: &Path,
        sha256: String,
        tuning: &UpdateTuning,
    ) -> Result<UpdateOutcome> {
        let dirs = self.host_dirs();
        let was_running = self.host.is_running();

        // 2. Stop.
        if was_running {
            self.host_step("stopping the server");
            self.host_stop()
                .await
                .context("the server did not stop, so nothing was changed")?;
        }

        // 3. Back up.
        let db = dirs.database();
        let backup = if db.is_file() {
            self.host_step("backing up the database");
            match backup::create(&db, &dirs.backups(), "pre-update", now_secs(), true) {
                Ok(b) => Some(b),
                Err(e) => {
                    let msg = format!(
                        "the database could not be backed up, so nothing was changed: {e:#}"
                    );
                    return Err(self.update_give_up(settings, current, was_running, msg));
                }
            }
        } else {
            None
        };

        // 4. The new package, next to the old one.
        self.host_step("unpacking the new package");
        let previous = settings
            .configured
            .then(|| PreviousConfig::read(&current.dir))
            .flatten();
        let (src, packages, game, os, keep) = (
            archive.to_path_buf(),
            dirs.packages(),
            self.version().version,
            self.host_os(),
            current.dir.clone(),
        );
        let installed = match tokio::task::spawn_blocking(move || {
            host::install_package_keeping(&src, &packages, &game, os, Some(&keep))
        })
        .await
        {
            Ok(Ok(p)) => p,
            Ok(Err(e)) => {
                let msg = format!(
                    "the new package could not be installed, so nothing was changed: {e:#}"
                );
                return Err(self.update_give_up(settings, current, was_running, msg));
            }
            Err(e) => bail!("installing the package failed: {e}"),
        };

        // 5. Its config, 6. switch.
        let mut next = settings.clone();
        next.package_dir = Some(installed.dir.clone());
        let switched = (|| -> Result<()> {
            if settings.configured {
                self.host_step("writing the config of the new version");
                self.host_write_configs_carrying(settings, &installed, previous.as_ref())
                    .context("the config of the new version could not be written")?;
            }
            self.host_save(&next)
        })();
        if let Err(e) = switched {
            let _ = std::fs::remove_dir_all(&installed.dir);
            let msg = format!("the old version was kept: {e:#}");
            return Err(self.update_give_up(settings, current, was_running, msg));
        }

        // 6b. Start it again and watch it come up.
        if was_running {
            self.host_step("starting the new version");
            if let Err(e) = self.update_start_and_wait(&installed, tuning).await {
                let msg = self
                    .update_rollback(
                        settings,
                        current,
                        &installed,
                        backup.as_ref(),
                        was_running,
                        &e,
                    )
                    .await;
                bail!("{msg}");
            }
        }

        // 7. The old package goes.
        host::prune_packages(&dirs.packages(), &installed.dir);
        Ok(UpdateOutcome {
            updated: true,
            from: current.server_version.clone(),
            to: installed.server_version.clone(),
            package: installed,
            was_running,
            restarted: was_running,
            backup: backup.map(|b| b.name),
            sha256: Some(sha256),
        })
    }

    /// A failure before the new package was switched in: start the old server again when it
    /// ran, and return the error to show.
    fn update_give_up(
        &self,
        settings: &HostSettings,
        current: &PackageInfo,
        was_running: bool,
        msg: String,
    ) -> anyhow::Error {
        if was_running {
            match self.update_start_old(settings, current) {
                Ok(()) => anyhow::anyhow!("{msg}; the old server was started again"),
                Err(e) => {
                    anyhow::anyhow!("{msg}; the old server could not be started again: {e:#}")
                }
            }
        } else {
            anyhow::anyhow!("{msg}")
        }
    }

    /// The new server did not come up: back to the old package, database and process.
    async fn update_rollback(
        &self,
        settings: &HostSettings,
        current: &PackageInfo,
        installed: &PackageInfo,
        backup: Option<&BackupInfo>,
        was_running: bool,
        cause: &anyhow::Error,
    ) -> String {
        let dirs = self.host_dirs();
        let mut notes = vec![format!(
            "the update to {} failed ({cause:#}); the old version {} was put back",
            installed.server_version, current.server_version
        )];
        // Whatever of the new server is still alive goes first.
        if self.host.is_running() {
            let _ = self.host_stop().await;
        }
        if let Err(e) = self.host_save(settings) {
            notes.push(format!("the settings could not be reset: {e:#}"));
        }
        let _ = std::fs::remove_dir_all(&installed.dir);
        if let Some(b) = backup {
            // The new server may have changed the database on its way up.
            match backup::restore(&dirs.database(), &dirs.backups(), &b.name, now_secs()) {
                Ok(_) => notes.push("the database was restored from the backup made first".into()),
                Err(e) => notes.push(format!(
                    "the database could not be restored from the backup {}: {e:#}",
                    b.name
                )),
            }
        }
        if was_running {
            match self.update_start_old(settings, current) {
                Ok(()) => notes.push("the old server runs again".into()),
                Err(e) => notes.push(format!("the old server could not be started again: {e:#}")),
            }
        }
        notes.join("; ")
    }

    fn update_start_old(&self, settings: &HostSettings, current: &PackageInfo) -> Result<()> {
        if !crate::host::config_path(&current.dir).is_file() && settings.configured {
            self.host_write_configs(settings, current)?;
        }
        self.host
            .start(self.events.clone(), &self.host_spec(current, "server.toml"))?;
        Ok(())
    }

    /// Starts the package and waits until its web API answers. Fails when the process ends
    /// first; when it still lives after `ready_wait` the server is taken to be loading.
    async fn update_start_and_wait(&self, pkg: &PackageInfo, tuning: &UpdateTuning) -> Result<()> {
        let settings = self.host_settings()?;
        self.host
            .start(self.events.clone(), &self.host_spec(pkg, "server.toml"))?;
        let api = ApiClient::new(&local_web_url(&settings));
        let t0 = std::time::Instant::now();
        loop {
            if let RunState::Exited { code } = self.host.state() {
                bail!(
                    "the new server ended right after starting (exit code {})",
                    code.map_or("none".to_owned(), |c| c.to_string())
                );
            }
            if !self.host.is_running() {
                bail!("the new server ended right after starting");
            }
            if tokio::time::timeout(Duration::from_millis(800), api.server_info())
                .await
                .is_ok_and(|r| r.is_ok())
            {
                return Ok(());
            }
            if t0.elapsed() >= tuning.ready_wait {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }
}
