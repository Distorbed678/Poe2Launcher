//! The backup operations of [`Core`]: status, back up now, restore, delete, and the automatic
//! backup before a server start. The file work is in [`crate::backup`].

use anyhow::{Result, bail};
use serde::Serialize;

use crate::{
    backup::{self, BackupInfo, BackupSettings},
    core::{Core, now_secs},
    paths::reject_steam_paths,
};

#[derive(Debug, Clone, Serialize)]
pub struct BackupStatus {
    pub dir: std::path::PathBuf,
    pub backups: Vec<BackupInfo>,
    pub settings: BackupSettings,
    /// Size of the live database, absent when the server never ran.
    pub database_size: Option<u64>,
    /// The server runs: backing up is fine (online), restoring is not.
    pub server_running: bool,
}

impl Core {
    pub fn backup_settings(&self, id: &str) -> Result<BackupSettings> {
        BackupSettings::load(self.host_dirs(id).backup_settings_file())
    }

    pub fn backup_status(&self, id: &str) -> Result<BackupStatus> {
        let dirs = self.host_dirs(id);
        let dir = dirs.backups();
        Ok(BackupStatus {
            backups: backup::list(&dir)?,
            settings: self.backup_settings(id)?,
            database_size: std::fs::metadata(dirs.database()).ok().map(|m| m.len()),
            server_running: self.runtime(id).is_running(),
            dir,
        })
    }

    /// Saves the automatic-backup choices.
    pub fn backup_set_settings(&self, id: &str, auto: bool, keep: u32) -> Result<BackupSettings> {
        let s = BackupSettings { auto, keep };
        s.validate()?;
        s.save(self.host_dirs(id).backup_settings_file())?;
        Ok(s)
    }

    /// A backup now; works while the server runs.
    pub fn backup_create(&self, id: &str) -> Result<BackupInfo> {
        let dirs = self.host_dirs(id);
        reject_steam_paths(&dirs.backups(), "backup folder")?;
        backup::create(
            &dirs.database(),
            &dirs.backups(),
            "manual",
            now_secs(),
            !self.runtime(id).is_running(),
        )
    }

    /// Restores a backup. The server must be stopped; the current database is backed up first
    /// (kind `pre-restore`) and returned.
    pub fn backup_restore(&self, id: &str, name: &str) -> Result<Option<BackupInfo>> {
        if self.runtime(id).is_running() {
            bail!("stop the server before restoring a backup");
        }
        let busy = self.host_busy.lock().unwrap();
        if *busy {
            bail!("another server task (download, install or setup) is still running");
        }
        let dirs = self.host_dirs(id);
        backup::restore(&dirs.database(), &dirs.backups(), name, now_secs())
    }

    pub fn backup_delete(&self, id: &str, name: &str) -> Result<()> {
        backup::delete(&self.host_dirs(id).backups(), name)
    }

    /// Before a start: back up (kind `auto`) and keep the last N, when the user switched that
    /// on and there is a database. Never stops the start; a failure goes to the server log.
    pub fn backup_auto(&self, id: &str) {
        let Ok(s) = self.backup_settings(id) else {
            return;
        };
        if !s.auto {
            return;
        }
        let dirs = self.host_dirs(id);
        if !dirs.database().is_file() {
            return;
        }
        let rt = self.runtime(id);
        let note = |t: &str| rt.note(self.events.as_ref(), t);
        match backup::create(&dirs.database(), &dirs.backups(), "auto", now_secs(), true) {
            Ok(b) => {
                note(&format!(
                    "backup before start: {} ({})",
                    b.name,
                    crate::verify::human(b.size)
                ));
                if let Err(e) = backup::prune_auto(&dirs.backups(), s.keep) {
                    note(&format!("could not delete old backups: {e:#}"));
                }
            }
            Err(e) => note(&format!("backup before start failed: {e:#}")),
        }
    }
}
