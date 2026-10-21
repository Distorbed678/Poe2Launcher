//! Checks an install against the version file: file count and size floors, key files with exact
//! sizes, and the SHA-256 of the client executable. Reads only the install directory.

use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::version::VersionFile;

/// Directories DepotDownloader and the launcher add that are not game files.
const SKIP_DIRS: &[&str] = &[".DepotDownloader", "poe-hook-mods", "logs", "poe-hook-data"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Pass,
    /// Reported, does not fail the verification.
    Warn,
    Fail,
    /// Could not be checked (no reference value).
    Skipped,
}

#[derive(Debug, Clone, Serialize)]
pub struct Check {
    pub name: String,
    pub status: Status,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct VerifyReport {
    /// No check failed.
    pub ok: bool,
    pub files: u64,
    pub bytes: u64,
    pub exe_sha256: Option<String>,
    pub checks: Vec<Check>,
}

/// Counts files and bytes below `dir`, skipping launcher/DepotDownloader bookkeeping.
pub fn scan(dir: &Path) -> Result<(u64, u64)> {
    let (mut files, mut bytes) = (0u64, 0u64);
    let mut stack: Vec<PathBuf> = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d)? {
            let entry = entry?;
            let ty = entry.file_type()?;
            if ty.is_dir() {
                let at_root = d == dir;
                if at_root && SKIP_DIRS.iter().any(|s| entry.file_name() == **s) {
                    continue;
                }
                stack.push(entry.path());
            } else if ty.is_file() {
                files += 1;
                bytes += entry.metadata()?.len();
            }
        }
    }
    Ok((files, bytes))
}

pub fn sha256_file(path: &Path) -> Result<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

/// Blocking (walks tens of thousands of files and hashes the exe): call from `spawn_blocking`.
pub fn verify(dir: &Path, version: &VersionFile) -> VerifyReport {
    let spec = &version.verify;
    let mut checks = Vec::new();
    let mut push = |name: &str, status: Status, detail: String| {
        checks.push(Check {
            name: name.into(),
            status,
            detail,
        })
    };
    if !dir.is_dir() {
        push(
            "install directory",
            Status::Fail,
            format!("{} does not exist", dir.display()),
        );
        return VerifyReport {
            ok: false,
            files: 0,
            bytes: 0,
            exe_sha256: None,
            checks,
        };
    }
    let (files, bytes) = match scan(dir) {
        Ok(v) => v,
        Err(e) => {
            push(
                "scan",
                Status::Fail,
                format!("cannot read the install: {e}"),
            );
            return VerifyReport {
                ok: false,
                files: 0,
                bytes: 0,
                exe_sha256: None,
                checks,
            };
        }
    };
    if spec.min_files > 0 {
        let ok = files >= spec.min_files;
        push(
            "file count",
            if ok { Status::Pass } else { Status::Fail },
            format!("{files} files (at least {} expected)", spec.min_files),
        );
    }
    if spec.min_bytes > 0 {
        let ok = bytes >= spec.min_bytes;
        push(
            "total size",
            if ok { Status::Pass } else { Status::Fail },
            format!(
                "{} (at least {} expected)",
                human(bytes),
                human(spec.min_bytes)
            ),
        );
    }
    for f in &spec.files {
        let p = dir.join(&f.path);
        match std::fs::metadata(&p) {
            Ok(m) if m.is_file() && m.len() == f.size => {
                push(&f.path, Status::Pass, format!("{} bytes", f.size))
            }
            Ok(m) if m.is_file() => push(
                &f.path,
                Status::Fail,
                format!("{} bytes, expected {}", m.len(), f.size),
            ),
            _ => push(&f.path, Status::Fail, "missing".into()),
        }
    }
    let mut exe_sha256 = None;
    if !spec.exe.is_empty() {
        let p = dir.join(&spec.exe);
        match (spec.exe_sha256_hash(), sha256_file(&p)) {
            (_, Err(_)) => push(
                &spec.exe,
                Status::Fail,
                "the client executable is missing".into(),
            ),
            (Ok(None), Ok(got)) => {
                push(
                    "client checksum",
                    Status::Skipped,
                    format!(
                        "no checksum recorded in versions/{}.toml (TODO); this install has {got}",
                        version.version
                    ),
                );
                exe_sha256 = Some(got);
            }
            (Ok(Some(want)), Ok(got)) if want == got => {
                push("client checksum", Status::Pass, got.clone());
                exe_sha256 = Some(got);
            }
            (Ok(Some(want)), Ok(got)) => {
                let status = if spec.exe_sha256_enforce {
                    Status::Fail
                } else {
                    Status::Warn
                };
                push(
                    "client checksum",
                    status,
                    format!(
                        "{got} differs from the recorded {want}{}",
                        if spec.exe_sha256_enforce {
                            ""
                        } else {
                            " (recorded value is unconfirmed against the depot; not fatal)"
                        }
                    ),
                );
                exe_sha256 = Some(got);
            }
            (Err(e), Ok(_)) => push(
                "client checksum",
                Status::Fail,
                format!("version file: {e}"),
            ),
        }
    }
    let ok = checks.iter().all(|c| c.status != Status::Fail);
    VerifyReport {
        ok,
        files,
        bytes,
        exe_sha256,
        checks,
    }
}

/// Whether `dir` holds an install of the client at all (the executable exists).
pub fn looks_installed(dir: &Path, version: &VersionFile) -> bool {
    !version.verify.exe.is_empty() && dir.join(&version.verify.exe).is_file()
}

pub fn human(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1000.0 && i < UNITS.len() - 1 {
        v /= 1000.0;
        i += 1;
    }
    if i == 0 {
        format!("{bytes} B")
    } else {
        format!("{v:.1} {}", UNITS[i])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A version file with `exe` lines under `[exe]` and `verify` lines under `[verify]`.
    fn version(exe: &str, verify: &str) -> VersionFile {
        VersionFile::parse(&format!(
            "version = \"t\"\napp_id = 1\n[[depots]]\nid = 2\nmanifest = \"3\"\n[exe]\nname = \"game.exe\"\n{exe}\n[verify]\n{verify}"
        ))
        .unwrap()
    }

    fn install(dir: &Path) {
        std::fs::create_dir_all(dir.join("Bundles2")).unwrap();
        std::fs::create_dir_all(dir.join(".DepotDownloader")).unwrap();
        std::fs::write(dir.join("game.exe"), b"exe-bytes").unwrap();
        std::fs::write(dir.join("Bundles2/a.bin"), vec![0u8; 100]).unwrap();
        std::fs::write(dir.join(".DepotDownloader/state"), vec![0u8; 5000]).unwrap();
    }

    #[test]
    fn scan_skips_bookkeeping() {
        let tmp = tempfile::tempdir().unwrap();
        install(tmp.path());
        assert_eq!(scan(tmp.path()).unwrap(), (2, 109));
    }

    #[test]
    fn a_good_install_passes_and_the_checksum_is_checked() {
        let tmp = tempfile::tempdir().unwrap();
        install(tmp.path());
        let sha = hex::encode(Sha256::digest(b"exe-bytes"));
        let v = version(
            &format!("sha256 = \"{sha}\"\nsha256_enforce = true\n"),
            "min_files = 2\nmin_bytes = 100\n[[verify.files]]\npath = \"Bundles2/a.bin\"\nsize = 100\n",
        );
        let r = verify(tmp.path(), &v);
        assert!(r.ok, "{:?}", r.checks);
        assert_eq!(r.exe_sha256.as_deref(), Some(sha.as_str()));
        assert!(r.checks.iter().all(|c| c.status == Status::Pass));
        assert!(looks_installed(tmp.path(), &v));
    }

    #[test]
    fn missing_files_wrong_sizes_and_floors_fail() {
        let tmp = tempfile::tempdir().unwrap();
        install(tmp.path());
        let v = version(
            "",
            "min_files = 10\n[[verify.files]]\npath = \"Bundles2/a.bin\"\nsize = 99\n[[verify.files]]\npath = \"nope.dll\"\nsize = 1\n",
        );
        let r = verify(tmp.path(), &v);
        assert!(!r.ok);
        let failed: Vec<_> = r
            .checks
            .iter()
            .filter(|c| c.status == Status::Fail)
            .map(|c| c.name.as_str())
            .collect();
        assert_eq!(failed, ["file count", "Bundles2/a.bin", "nope.dll"]);
        assert!(!verify(&tmp.path().join("absent"), &v).ok);
    }

    #[test]
    fn checksum_mismatch_is_a_warning_unless_enforced_and_unknown_is_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        install(tmp.path());
        let wrong = "00".repeat(32);
        let warn = verify(tmp.path(), &version(&format!("sha256 = \"{wrong}\"\n"), ""));
        assert!(warn.ok);
        assert_eq!(warn.checks.last().unwrap().status, Status::Warn);
        let hard = verify(
            tmp.path(),
            &version(&format!("sha256 = \"{wrong}\"\nsha256_enforce = true\n"), ""),
        );
        assert!(!hard.ok);
        let todo = verify(tmp.path(), &version("sha256 = \"TODO\"\n", ""));
        assert!(todo.ok);
        let last = todo.checks.last().unwrap();
        assert_eq!(last.status, Status::Skipped);
        assert!(last.detail.contains("TODO"));
    }

    #[test]
    fn missing_exe_fails() {
        let tmp = tempfile::tempdir().unwrap();
        install(tmp.path());
        std::fs::remove_file(tmp.path().join("game.exe")).unwrap();
        let v = version("", "");
        assert!(!verify(tmp.path(), &v).ok);
        assert!(!looks_installed(tmp.path(), &v));
    }

    #[test]
    fn sizes_are_readable() {
        assert_eq!(human(999), "999 B");
        assert_eq!(human(150_000_000_000), "150.0 GB");
    }
}
