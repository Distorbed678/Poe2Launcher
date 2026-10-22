//! Backups of the hosted server's database (`server.sqlite`).
//!
//! A backup is a standalone SQLite file `server-<UTC date>-<time>-<kind>.sqlite` in
//! `<data>/server/backups/`. It is made with SQLite's online backup API, so it is consistent
//! while the server is running; when that API fails and the server is stopped, a plain file copy
//! is the fallback. Restoring needs a stopped server and first backs up the current database
//! (kind `pre-restore`), so a restore can be undone.
//!
//! Kinds: `manual` (the button), `auto` (before a server start, if switched on; only these are
//! pruned to the last N), `pre-restore`.

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, bail, ensure};
use rusqlite::{Connection, OpenFlags, backup::Backup};
use serde::{Deserialize, Serialize};

use crate::paths::write_atomic;

pub const DEFAULT_KEEP: u32 = 5;
pub const MAX_KEEP: u32 = 100;
const PREFIX: &str = "server-";
const EXT: &str = ".sqlite";

/// The backup choices (`backup.json` in the launcher's config folder).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct BackupSettings {
    /// Back up the database each time the server is started from the launcher.
    pub auto: bool,
    /// How many automatic backups are kept (older ones are deleted).
    pub keep: u32,
}

impl Default for BackupSettings {
    fn default() -> Self {
        Self {
            auto: false,
            keep: DEFAULT_KEEP,
        }
    }
}

impl BackupSettings {
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text)
                .with_context(|| format!("{} is not valid backup settings JSON", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        write_atomic(path, serde_json::to_string_pretty(self)?.as_bytes())
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(
            (1..=MAX_KEEP).contains(&self.keep),
            "keep between 1 and {MAX_KEEP} automatic backups"
        );
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct BackupInfo {
    /// The file name (the id the UI passes back).
    pub name: String,
    /// `manual`, `auto`, `pre-restore`, or `other` for a file somebody put there.
    pub kind: String,
    /// Modification time, Unix seconds.
    pub created: i64,
    pub size: u64,
}

/// `20261022-134501` for a Unix time (UTC).
pub fn stamp(unix: i64) -> String {
    let days = unix.div_euclid(86_400);
    let rem = unix.rem_euclid(86_400);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}{m:02}{d:02}-{:02}{:02}{:02}",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// The Unix time in a name `server-20261022-134501-<kind>[-N].sqlite`.
pub fn unix_of_name(name: &str) -> Option<i64> {
    let s = name.strip_prefix(PREFIX)?;
    let (date, rest) = s.split_once('-')?;
    let time = rest.get(..6)?;
    if date.len() != 8 || !date.bytes().chain(time.bytes()).all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n = |t: &str| t.parse::<i64>().ok();
    let (y, m, d) = (n(&date[..4])?, n(&date[4..6])?, n(&date[6..])?);
    let (hh, mm, ss) = (n(&time[..2])?, n(&time[2..4])?, n(&time[4..])?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) || hh > 23 || mm > 59 || ss > 60 {
        return None;
    }
    // Days from civil (Howard Hinnant).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + hh * 3600 + mm * 60 + ss)
}

fn kind_of(name: &str) -> &'static str {
    let stem = name.strip_suffix(EXT).unwrap_or(name);
    // A second backup in the same second got a counter: `...-auto-2`.
    let stem = match stem.rsplit_once('-') {
        Some((head, tail))
            if tail.bytes().all(|b| b.is_ascii_digit()) && !head.ends_with(char::is_numeric) =>
        {
            head
        }
        _ => stem,
    };
    if stem.ends_with("-manual") {
        "manual"
    } else if stem.ends_with("-auto") {
        "auto"
    } else if stem.ends_with("-pre-restore") {
        "pre-restore"
    } else {
        "other"
    }
}

/// A name the UI may pass back: one of our backup files, never a path.
pub fn valid_backup_name(name: &str) -> bool {
    name.starts_with(PREFIX)
        && name.ends_with(EXT)
        && name.len() <= 80
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && !name.contains("..")
}

fn backup_path(dir: &Path, name: &str) -> Result<PathBuf> {
    ensure!(valid_backup_name(name), "{name} is not a backup file name");
    let p = dir.join(name);
    ensure!(p.is_file(), "the backup {name} does not exist");
    Ok(p)
}

/// The backups in `dir`, newest first.
pub fn list(dir: &Path) -> Result<Vec<BackupInfo>> {
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(e).with_context(|| format!("reading {}", dir.display())),
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !valid_backup_name(&name) {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        // The name carries the time of the backup; the file's own time is the fallback.
        let created = unix_of_name(&name).unwrap_or_else(|| {
            meta.modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_secs() as i64)
        });
        out.push(BackupInfo {
            kind: kind_of(&name).to_owned(),
            name,
            created,
            size: meta.len(),
        });
    }
    // The name starts with the UTC stamp, so it breaks ties of equal modification times.
    out.sort_by(|a, b| (b.created, &b.name).cmp(&(a.created, &a.name)));
    Ok(out)
}

fn unique_name(dir: &Path, unix: i64, kind: &str) -> String {
    let base = format!("{PREFIX}{}-{kind}", stamp(unix));
    let mut name = format!("{base}{EXT}");
    let mut n = 2;
    while dir.join(&name).exists() {
        name = format!("{base}-{n}{EXT}");
        n += 1;
    }
    name
}

/// Copies the database with the online backup API into `dest` (a new file), then leaves it as a
/// single self-contained file (no WAL).
fn online_backup(db: &Path, dest: &Path) -> Result<()> {
    let src = Connection::open_with_flags(
        db,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .with_context(|| format!("opening {}", db.display()))?;
    src.busy_timeout(Duration::from_secs(10))?;
    let mut dst = Connection::open(dest)?;
    {
        let b = Backup::new(&src, &mut dst)?;
        // Small steps with a pause, so a running server is not held up.
        b.run_to_completion(256, Duration::from_millis(2), None)?;
    }
    dst.pragma_update(None, "journal_mode", "DELETE")?;
    let check: String = dst.query_row("PRAGMA quick_check", [], |r| r.get(0))?;
    ensure!(
        check == "ok",
        "the backup did not pass SQLite's check: {check}"
    );
    Ok(())
}

/// Makes a backup of `db` in `dir`. `allow_copy`: the server is stopped, so a plain copy of the
/// file is acceptable when the backup API fails (a missing WAL is then not a concern).
pub fn create(
    db: &Path,
    dir: &Path,
    kind: &str,
    unix: i64,
    allow_copy: bool,
) -> Result<BackupInfo> {
    ensure!(
        matches!(kind, "manual" | "auto" | "pre-restore"),
        "unknown backup kind {kind}"
    );
    ensure!(
        db.is_file(),
        "the server has no database yet ({} does not exist): start it once first",
        db.display()
    );
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let name = unique_name(dir, unix, kind);
    let part = dir.join(format!("{name}.part"));
    let _ = std::fs::remove_file(&part);
    let fin = dir.join(&name);
    if let Err(e) = online_backup(db, &part) {
        let _ = std::fs::remove_file(&part);
        if !allow_copy {
            return Err(e).context("the online backup of the running server's database failed");
        }
        std::fs::copy(db, &part).with_context(|| format!("copying {}", db.display()))?;
    }
    std::fs::rename(&part, &fin).with_context(|| format!("saving {}", fin.display()))?;
    let size = std::fs::metadata(&fin)?.len();
    Ok(BackupInfo {
        name,
        kind: kind.to_owned(),
        created: unix,
        size,
    })
}

pub fn delete(dir: &Path, name: &str) -> Result<()> {
    let p = backup_path(dir, name)?;
    std::fs::remove_file(&p).with_context(|| format!("deleting {}", p.display()))
}

/// Deletes the oldest `auto` backups beyond `keep`; returns the deleted names.
pub fn prune_auto(dir: &Path, keep: u32) -> Result<Vec<String>> {
    let autos: Vec<BackupInfo> = list(dir)?
        .into_iter()
        .filter(|b| b.kind == "auto")
        .collect();
    let mut gone = Vec::new();
    for b in autos.into_iter().skip(keep as usize) {
        if std::fs::remove_file(dir.join(&b.name)).is_ok() {
            gone.push(b.name);
        }
    }
    Ok(gone)
}

/// Checks that `file` is a healthy SQLite database.
pub fn check_database(file: &Path) -> Result<()> {
    let c = Connection::open_with_flags(file, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("{} is not a SQLite database", file.display()))?;
    let r: String = c
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .with_context(|| format!("{} is not a SQLite database", file.display()))?;
    ensure!(
        r == "ok",
        "{} failed SQLite's integrity check: {r}",
        file.display()
    );
    Ok(())
}

/// Puts the backup `name` in place of `db`. The server must be stopped (the caller checks).
/// The current database is backed up first; returns that backup (None if there was no database).
pub fn restore(db: &Path, dir: &Path, name: &str, unix: i64) -> Result<Option<BackupInfo>> {
    let src = backup_path(dir, name)?;
    check_database(&src).context("this backup cannot be restored")?;
    let pre = if db.is_file() {
        Some(create(db, dir, "pre-restore", unix, true)?)
    } else {
        None
    };
    let parent = db.parent().context("the database has no folder")?;
    std::fs::create_dir_all(parent)?;
    let staging = parent.join("server.sqlite.restoring");
    let _ = std::fs::remove_file(&staging);
    std::fs::copy(&src, &staging).context("copying the backup")?;
    // Leftovers of the old database must not be replayed over the restored one.
    for suffix in ["-wal", "-shm", "-journal"] {
        let mut side = db.as_os_str().to_owned();
        side.push(suffix);
        let _ = std::fs::remove_file(PathBuf::from(side));
    }
    if let Err(e) = std::fs::rename(&staging, db) {
        let _ = std::fs::remove_file(&staging);
        bail!("could not put the backup in place: {e}");
    }
    Ok(pre)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_db(path: &Path, rows: &[&str]) {
        let c = Connection::open(path).unwrap();
        c.pragma_update(None, "journal_mode", "WAL").unwrap();
        c.execute_batch("CREATE TABLE IF NOT EXISTS t (v TEXT)")
            .unwrap();
        for r in rows {
            c.execute("INSERT INTO t (v) VALUES (?1)", [r]).unwrap();
        }
    }

    fn rows(path: &Path) -> Vec<String> {
        let c = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let mut s = c.prepare("SELECT v FROM t ORDER BY rowid").unwrap();
        s.query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    #[test]
    fn stamps_are_utc() {
        assert_eq!(stamp(0), "19700101-000000");
        assert_eq!(stamp(1_700_000_000), "20231114-221320");
        assert_eq!(stamp(1_782_131_100), "20260622-122500");
        // Leap day.
        assert_eq!(stamp(1_709_164_800), "20240229-000000");
    }

    #[test]
    fn names_are_checked() {
        assert!(valid_backup_name("server-20261022-134501-manual.sqlite"));
        assert!(!valid_backup_name("../server-1-manual.sqlite"));
        assert!(!valid_backup_name("server-1/../../x.sqlite"));
        assert!(!valid_backup_name("other.sqlite"));
        assert!(!valid_backup_name("server-1-manual.sqlite.part"));
        assert_eq!(kind_of("server-1-auto.sqlite"), "auto");
        assert_eq!(kind_of("server-1-pre-restore.sqlite"), "pre-restore");
        assert_eq!(kind_of("server-1-2.sqlite"), "other");
        assert_eq!(kind_of("server-20261022-134501-auto-2.sqlite"), "auto");
        assert_eq!(
            kind_of("server-20261022-134501-pre-restore-12.sqlite"),
            "pre-restore"
        );
        assert_eq!(
            unix_of_name("server-20231114-221320-manual.sqlite"),
            Some(1_700_000_000)
        );
        assert_eq!(
            unix_of_name(&format!("server-{}-auto-3.sqlite", stamp(1_709_164_800))),
            Some(1_709_164_800)
        );
        assert_eq!(unix_of_name("server-20231314-221320-manual.sqlite"), None);
        assert_eq!(unix_of_name("server-x-manual.sqlite"), None);
    }

    #[test]
    fn a_backup_taken_from_a_live_wal_database_is_whole() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("data/server.sqlite");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        make_db(&db, &["a", "b"]);
        // A second connection stays open and writes (the "running server"), data still in the WAL.
        let live = Connection::open(&db).unwrap();
        live.execute("INSERT INTO t (v) VALUES ('c')", []).unwrap();
        let dir = tmp.path().join("backups");
        let b = create(&db, &dir, "manual", 1_700_000_000, false).unwrap();
        assert_eq!(b.kind, "manual");
        assert!(b.name.starts_with("server-20231114-221320-manual"));
        assert_eq!(rows(&dir.join(&b.name)), ["a", "b", "c"]);
        // Standalone: no side files next to it.
        assert!(!dir.join(format!("{}-wal", b.name)).exists());
        live.execute("INSERT INTO t (v) VALUES ('d')", []).unwrap();
        // The backup does not change when the source does.
        assert_eq!(rows(&dir.join(&b.name)), ["a", "b", "c"]);
    }

    #[test]
    fn a_missing_database_is_explained() {
        let tmp = tempfile::tempdir().unwrap();
        let e = create(
            &tmp.path().join("nope.sqlite"),
            tmp.path(),
            "manual",
            1,
            true,
        )
        .unwrap_err();
        assert!(format!("{e:#}").contains("no database yet"), "{e:#}");
        assert!(
            create(
                &tmp.path().join("nope.sqlite"),
                tmp.path(),
                "weird",
                1,
                true
            )
            .is_err()
        );
    }

    #[test]
    fn list_orders_newest_first_and_ignores_strangers() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("server.sqlite");
        make_db(&db, &["x"]);
        let dir = tmp.path().join("b");
        create(&db, &dir, "manual", 1_700_000_000, true).unwrap();
        create(&db, &dir, "auto", 1_700_000_100, true).unwrap();
        create(&db, &dir, "auto", 1_700_000_100, true).unwrap(); // same second: a counter
        std::fs::write(dir.join("notes.txt"), "x").unwrap();
        std::fs::write(dir.join("server-1-manual.sqlite.part"), "x").unwrap();
        let l = list(&dir).unwrap();
        assert_eq!(l.len(), 3);
        assert!(l.iter().all(|b| b.size > 0));
        let kinds: Vec<_> = l.iter().map(|b| b.kind.as_str()).collect();
        assert!(kinds.contains(&"manual") && kinds.iter().filter(|k| **k == "auto").count() == 2);
        assert!(list(&tmp.path().join("missing")).unwrap().is_empty());
    }

    #[test]
    fn only_automatic_backups_are_pruned() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("server.sqlite");
        make_db(&db, &["x"]);
        let dir = tmp.path().join("b");
        create(&db, &dir, "manual", 1_700_000_000, true).unwrap();
        for i in 0..5 {
            create(&db, &dir, "auto", 1_700_001_000 + i, true).unwrap();
        }
        let gone = prune_auto(&dir, 2).unwrap();
        assert_eq!(gone.len(), 3);
        let l = list(&dir).unwrap();
        assert_eq!(l.iter().filter(|b| b.kind == "auto").count(), 2);
        assert_eq!(l.iter().filter(|b| b.kind == "manual").count(), 1);
        // The two newest remain.
        assert!(l.iter().any(|b| b.name.contains(&stamp(1_700_001_004))));
        assert!(l.iter().any(|b| b.name.contains(&stamp(1_700_001_003))));
    }

    #[test]
    fn restore_puts_the_old_data_back_and_keeps_the_current_one() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("data/server.sqlite");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        make_db(&db, &["old"]);
        let dir = tmp.path().join("backups");
        let b = create(&db, &dir, "manual", 1_700_000_000, false).unwrap();
        // The database changes after the backup, with WAL leftovers next to it.
        {
            let c = Connection::open(&db).unwrap();
            c.execute("INSERT INTO t (v) VALUES ('new')", []).unwrap();
            std::mem::forget(c); // keeps the -wal/-shm files, like a crashed server
        }
        let pre = restore(&db, &dir, &b.name, 1_700_000_500).unwrap().unwrap();
        assert_eq!(pre.kind, "pre-restore");
        assert_eq!(rows(&db), ["old"]);
        // The state before the restore is itself restorable.
        assert_eq!(rows(&dir.join(&pre.name)), ["old", "new"]);
        // The backup stays.
        assert!(dir.join(&b.name).is_file());
        assert!(!tmp.path().join("data/server.sqlite.restoring").exists());
    }

    #[test]
    fn restore_refuses_damaged_files_and_bad_names() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("server.sqlite");
        make_db(&db, &["keep"]);
        let dir = tmp.path().join("b");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("server-1-manual.sqlite"),
            "this is not sqlite at all, no",
        )
        .unwrap();
        let e = restore(&db, &dir, "server-1-manual.sqlite", 5).unwrap_err();
        assert!(format!("{e:#}").contains("cannot be restored"), "{e:#}");
        assert_eq!(rows(&db), ["keep"], "the database is untouched");
        assert!(restore(&db, &dir, "../server.sqlite", 5).is_err());
        assert!(restore(&db, &dir, "server-9-manual.sqlite", 5).is_err());
        // Only the one failed restore attempt wrote nothing new.
        assert_eq!(list(&dir).unwrap().len(), 1);
    }

    #[test]
    fn restore_without_a_current_database_works() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("orig.sqlite");
        make_db(&src, &["a"]);
        let dir = tmp.path().join("b");
        let b = create(&src, &dir, "manual", 10, true).unwrap();
        let db = tmp.path().join("fresh/data/server.sqlite");
        assert!(restore(&db, &dir, &b.name, 20).unwrap().is_none());
        assert_eq!(rows(&db), ["a"]);
    }

    #[test]
    fn delete_removes_only_backups() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("server.sqlite");
        make_db(&db, &["x"]);
        let dir = tmp.path().join("b");
        let b = create(&db, &dir, "manual", 10, true).unwrap();
        std::fs::write(tmp.path().join("server-keep.sqlite"), "x").unwrap();
        assert!(delete(&dir, "../server-keep.sqlite").is_err());
        delete(&dir, &b.name).unwrap();
        assert!(list(&dir).unwrap().is_empty());
        assert!(delete(&dir, &b.name).is_err());
    }

    #[test]
    fn settings_round_trip_and_validate() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("backup.json");
        assert_eq!(BackupSettings::load(&f).unwrap(), BackupSettings::default());
        let s = BackupSettings {
            auto: true,
            keep: 3,
        };
        s.save(&f).unwrap();
        assert_eq!(BackupSettings::load(&f).unwrap(), s);
        assert!(
            BackupSettings {
                auto: true,
                keep: 0
            }
            .validate()
            .is_err()
        );
        assert!(
            BackupSettings {
                auto: false,
                keep: 101
            }
            .validate()
            .is_err()
        );
        std::fs::write(&f, r#"{"auto": true}"#).unwrap();
        assert_eq!(BackupSettings::load(&f).unwrap().keep, DEFAULT_KEEP);
    }
}
