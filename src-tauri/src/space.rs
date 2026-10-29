//! Disk-space check before a game install: the target disk must have room for what is still
//! missing. The size comes from the version file (`[verify] min_bytes`, the floor measured on a
//! full install) with a margin, since the floor is deliberately below the real size.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::{verify, version::VersionFile};

/// `min_bytes` is a floor ("at least this much is on disk after a good install"); the full
/// 0.5.5e.hotfix1 install measured 150.8 GB against a floor of 130 GB. The estimate adds this margin.
pub const ESTIMATE_PERCENT: u64 = 116;

/// The install size the check plans for (0 = the version file gives none).
pub fn estimated_install_bytes(v: &VersionFile) -> u64 {
    v.verify.min_bytes.saturating_mul(ESTIMATE_PERCENT) / 100
}

/// How much each depot weighs in the overall progress, for the speed and time-left display:
/// the exe depot is as big as the executable, the others share the rest.
pub fn depot_weights(v: &VersionFile) -> Vec<u64> {
    let total = estimated_install_bytes(v);
    let exe_size = v
        .verify
        .files
        .iter()
        .find(|f| f.path == v.verify.exe)
        .map_or(0, |f| f.size);
    let is_exe = |role: &str| role.to_ascii_lowercase().contains("exe");
    let exe_depots = v.depots.iter().filter(|d| is_exe(&d.role)).count() as u64;
    let others = (v.depots.len() as u64).saturating_sub(exe_depots).max(1);
    let rest = total.saturating_sub(exe_size * exe_depots);
    v.depots
        .iter()
        .map(|d| {
            if is_exe(&d.role) {
                exe_size
            } else {
                rest / others
            }
        })
        .collect()
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SpaceCheck {
    pub path: PathBuf,
    /// What the install needs in total (estimate), 0 = unknown.
    pub required: u64,
    /// Already in the folder (a repair or a resumed download).
    pub present: u64,
    /// Still to come: `required - present`.
    pub missing: u64,
    /// Free space of the disk the folder is on; absent if it could not be read.
    pub free: Option<u64>,
    pub ok: bool,
    /// For the UI, also when `ok`.
    pub message: String,
}

/// The free space of the disk that holds `dir`, even if `dir` does not exist yet.
pub fn free_space(dir: &Path) -> Option<u64> {
    let mut d = dir;
    loop {
        if d.exists() {
            return fs4::available_space(d).ok();
        }
        d = d.parent()?;
    }
}

/// The decision (pure, so it is tested without a disk).
pub fn evaluate(path: &Path, required: u64, present: u64, free: Option<u64>) -> SpaceCheck {
    let missing = required.saturating_sub(present);
    let (ok, message) = match free {
        _ if required == 0 => (
            true,
            "The size of the install is not known; space is not checked.".to_owned(),
        ),
        Some(free) if free < missing => (
            false,
            format!(
                "Not enough free space on the disk of {}: about {} still have to be downloaded, {} are free. \
                 Free up at least {} or choose another install folder in Settings.",
                path.display(),
                verify::human(missing),
                verify::human(free),
                verify::human(missing - free),
            ),
        ),
        Some(free) => (
            true,
            format!(
                "About {} to download, {} free on the disk of {}.",
                verify::human(missing),
                verify::human(free),
                path.display()
            ),
        ),
        None => (
            true,
            "The free space of the disk could not be read; it is not checked.".to_owned(),
        ),
    };
    SpaceCheck {
        path: path.to_path_buf(),
        required,
        present,
        missing,
        free,
        ok,
        message,
    }
}

/// The check for installing `v` into `dir` right now.
pub fn check(dir: &Path, v: &VersionFile) -> SpaceCheck {
    let present = if dir.is_dir() {
        verify::scan(dir).map(|(_, b)| b).unwrap_or(0)
    } else {
        0
    };
    evaluate(dir, estimated_install_bytes(v), present, free_space(dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_enough_space_is_refused_with_a_clear_message() {
        let c = evaluate(
            Path::new("/games/poe2"),
            150_000_000_000,
            0,
            Some(40_000_000_000),
        );
        assert!(!c.ok);
        assert_eq!(c.missing, 150_000_000_000);
        assert!(c.message.contains("Not enough free space"), "{}", c.message);
        assert!(
            c.message.contains("150.0 GB") && c.message.contains("40.0 GB"),
            "{}",
            c.message
        );
        assert!(
            c.message.contains("Free up at least 110.0 GB"),
            "{}",
            c.message
        );
    }

    #[test]
    fn what_is_already_there_does_not_need_room_again() {
        let c = evaluate(
            Path::new("/g"),
            150_000_000_000,
            120_000_000_000,
            Some(40_000_000_000),
        );
        assert!(c.ok, "{}", c.message);
        assert_eq!(c.missing, 30_000_000_000);
        // Exactly enough is enough.
        assert!(evaluate(Path::new("/g"), 100, 0, Some(100)).ok);
        assert!(!evaluate(Path::new("/g"), 100, 0, Some(99)).ok);
    }

    #[test]
    fn unknown_sizes_do_not_block() {
        assert!(evaluate(Path::new("/g"), 0, 0, Some(1)).ok);
        let c = evaluate(Path::new("/g"), 100, 0, None);
        assert!(c.ok && c.message.contains("could not be read"));
        // Present more than required: nothing missing.
        assert_eq!(evaluate(Path::new("/g"), 10, 50, Some(0)).missing, 0);
    }

    #[test]
    fn free_space_is_read_from_the_nearest_existing_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let deep = tmp.path().join("a/b/c");
        let (a, b) = (free_space(&deep).unwrap(), free_space(tmp.path()).unwrap());
        // Same filesystem; tests running in parallel write files between the two reads.
        assert!(a.abs_diff(b) < 1 << 30, "{a} vs {b}");
    }

    #[test]
    fn the_estimate_and_depot_weights_follow_the_version_file() {
        let v = VersionFile::embedded(VersionFile::known_versions()[0]).unwrap();
        let est = estimated_install_bytes(&v);
        assert!(
            est > v.verify.min_bytes,
            "the estimate has a margin over the floor"
        );
        let w = depot_weights(&v);
        assert_eq!(w.len(), v.depots.len());
        // The weights add up to the estimate; the exe depot is the small one.
        assert_eq!(w.iter().sum::<u64>(), est);
        let exe_at = v
            .depots
            .iter()
            .position(|d| d.role.contains("exe"))
            .unwrap();
        assert!(w[exe_at] < est / 100);
    }

    #[test]
    fn a_real_folder_is_checked_against_its_disk() {
        let tmp = tempfile::tempdir().unwrap();
        let mut v = VersionFile::embedded(VersionFile::known_versions()[0]).unwrap();
        v.verify.min_bytes = 10 << 40; // 10 TB: no test machine has that free
        let c = check(&tmp.path().join("game"), &v);
        assert!(!c.ok, "{}", c.message);
        v.verify.min_bytes = 1000;
        assert!(check(tmp.path(), &v).ok);
    }
}
