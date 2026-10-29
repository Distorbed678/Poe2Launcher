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
    pub async fn host_update_check(&self, id: &str) -> ServerUpdateCheck {
        self.host_update_check_at(id, host::API_BASE).await
    }

    /// Asks GitHub whether a newer server release than the installed one exists. Never an
    /// error: a failure is in the `error` field. With no usable package there is nothing to
    /// update and GitHub is not asked.
    pub async fn host_update_check_at(&self, id: &str, api_base: &str) -> ServerUpdateCheck {
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
        let Ok(settings) = self.host_settings(id) else {
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

    pub async fn host_update_apply(&self, id: &str) -> Result<UpdateOutcome> {
        self.host_update_apply_with(id, host::API_BASE, &UpdateTuning::default())
            .await
    }

    /// The one-click update (see the module comment). Fails with the old server restored.
    pub async fn host_update_apply_with(
        &self,
        id: &str,
        api_base: &str,
        tuning: &UpdateTuning,
    ) -> Result<UpdateOutcome> {
        self.host_private_only()?;
        let settings = self.host_settings(id)?;
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
        let (archive, sha) = self.host_download_verified(id, &release).await?;
        let result = self
            .update_swap(id, &settings, &current, &archive, sha, tuning)
            .await;
        let _ = tokio::fs::remove_file(&archive).await;
        match &result {
            Ok(o) => {
                self.host_step(id, &format!("updated to {}", o.to));
                self.emit("host:installed", json!({"id": id, "package": o.package.name}));
            }
            Err(e) => self.host_step(id, &format!("the update failed: {e:#}")),
        }
        result
    }

    async fn update_swap(
        &self,
        id: &str,
        settings: &HostSettings,
        current: &PackageInfo,
        archive: &Path,
        sha256: String,
        tuning: &UpdateTuning,
    ) -> Result<UpdateOutcome> {
        let dirs = self.host_dirs(id);
        let was_running = self.runtime(id).is_running();

        // 2. Stop.
        if was_running {
            self.host_step(id, "stopping the server");
            self.host_stop(id)
                .await
                .context("the server did not stop, so nothing was changed")?;
        }

        // 3. Back up.
        let db = dirs.database();
        let backup = if db.is_file() {
            self.host_step(id, "backing up the database");
            match backup::create(&db, &dirs.backups(), "pre-update", now_secs(), true) {
                Ok(b) => Some(b),
                Err(e) => {
                    let msg = format!(
                        "the database could not be backed up, so nothing was changed: {e:#}"
                    );
                    return Err(self.update_give_up(id, settings, current, was_running, msg));
                }
            }
        } else {
            None
        };

        // 4. The new package, next to the old one.
        self.host_step(id, "unpacking the new package");
        let previous = settings
            .configured
            .then(|| PreviousConfig::read(&current.dir, &dirs.config()))
            .flatten();
        let in_use = self.host_packages_in_use();
        let (src, packages, game, os, keep) = (
            archive.to_path_buf(),
            dirs.packages(),
            self.version().version,
            self.host_os(),
            current.dir.clone(),
        );
        let installed = match tokio::task::spawn_blocking(move || {
            host::install_package_keeping(&src, &packages, &game, os, Some(&keep), &in_use)
        })
        .await
        {
            Ok(Ok(p)) => p,
            Ok(Err(e)) => {
                let msg = format!(
                    "the new package could not be installed, so nothing was changed: {e:#}"
                );
                return Err(self.update_give_up(id, settings, current, was_running, msg));
            }
            Err(e) => bail!("installing the package failed: {e}"),
        };

        // 5. Its config, 6. switch.
        let mut next = settings.clone();
        next.package_dir = Some(installed.dir.clone());
        let switched = (|| -> Result<()> {
            if settings.configured {
                self.host_step(id, "writing the config of the new version");
                self.host_write_configs_carrying(id, settings, &installed, previous.as_ref())
                    .context("the config of the new version could not be written")?;
            }
            self.host_save(id, &next)
        })();
        if let Err(e) = switched {
            let _ = std::fs::remove_dir_all(&installed.dir);
            let msg = format!("the old version was kept: {e:#}");
            if let Some(p) = &previous {
                let _ = crate::paths::write_atomic(&dirs.config(), p.config.as_bytes());
            }
            return Err(self.update_give_up(id, settings, current, was_running, msg));
        }

        // 6b. Start it again and watch it come up.
        if was_running {
            self.host_step(id, "starting the new version");
            if let Err(e) = self.update_start_and_wait(id, &installed, tuning).await {
                let msg = self
                    .update_rollback(
                        id,
                        settings,
                        previous.as_ref(),
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
        self.host_prune_packages(id);
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
        id: &str,
        settings: &HostSettings,
        current: &PackageInfo,
        was_running: bool,
        msg: String,
    ) -> anyhow::Error {
        if was_running {
            match self.update_start_old(id, settings, current) {
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
    #[allow(clippy::too_many_arguments)]
    async fn update_rollback(
        &self,
        id: &str,
        settings: &HostSettings,
        previous: Option<&PreviousConfig>,
        current: &PackageInfo,
        installed: &PackageInfo,
        backup: Option<&BackupInfo>,
        was_running: bool,
        cause: &anyhow::Error,
    ) -> String {
        let dirs = self.host_dirs(id);
        let mut notes = vec![format!(
            "the update to {} failed ({cause:#}); the old version {} was put back",
            installed.server_version, current.server_version
        )];
        // Whatever of the new server is still alive goes first.
        if self.runtime(id).is_running() {
            let _ = self.host_stop(id).await;
        }
        if let Err(e) = self.host_save(id, settings) {
            notes.push(format!("the settings could not be reset: {e:#}"));
        }
        if let Some(p) = previous
            && let Err(e) = crate::paths::write_atomic(&dirs.config(), p.config.as_bytes())
        {
            notes.push(format!("the old config could not be put back: {e:#}"));
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
            match self.update_start_old(id, settings, current) {
                Ok(()) => notes.push("the old server runs again".into()),
                Err(e) => notes.push(format!("the old server could not be started again: {e:#}")),
            }
        }
        notes.join("; ")
    }

    fn update_start_old(&self, id: &str, settings: &HostSettings, current: &PackageInfo) -> Result<()> {
        if !self.host_dirs(id).config().is_file() && settings.configured {
            self.host_write_configs(id, settings, current)?;
        }
        self.runtime(id)
            .start(self.events.clone(), &self.host_spec(id, current))?;
        Ok(())
    }

    /// Starts the package and waits until its web API answers. Fails when the process ends
    /// first; when it still lives after `ready_wait` the server is taken to be loading.
    async fn update_start_and_wait(
        &self,
        id: &str,
        pkg: &PackageInfo,
        tuning: &UpdateTuning,
    ) -> Result<()> {
        let settings = self.host_settings(id)?;
        let rt = self.runtime(id);
        rt.start(self.events.clone(), &self.host_spec(id, pkg))?;
        let api = ApiClient::new(&local_web_url(&settings));
        let t0 = std::time::Instant::now();
        loop {
            if let RunState::Exited { code } = rt.state() {
                bail!(
                    "the new server ended right after starting (exit code {})",
                    code.map_or("none".to_owned(), |c| c.to_string())
                );
            }
            if !rt.is_running() {
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
