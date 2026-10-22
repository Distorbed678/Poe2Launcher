//! The Steam login that DepotDownloader remembers (`-remember-password`).
//!
//! DepotDownloader 3.4.0 keeps a login *token* (never the password) in `account.config`, a
//! protobuf file in .NET's isolated storage:
//! `<local data>/IsolatedStorage/<id>/<id>/Url.<hash>/AssemFiles/account.config`.
//! On Unix `<local data>` follows `XDG_DATA_HOME`, so the launcher starts the tool with its own
//! (`Dirs::steam_login_home`) and "Log out of Steam" deletes the file(s) below it. On Windows
//! .NET asks the system for the folder (no environment override), so the storage is the user's
//! `%LOCALAPPDATA%\IsolatedStorage` and only files named `account.config` below it, at the
//! isolated-storage depth, are removed. Nothing of the Steam client itself is read or written.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;

use crate::paths::Dirs;

/// DepotDownloader's file name for the remembered login.
pub const ACCOUNT_FILE: &str = "account.config";
const STORAGE_DIR: &str = "IsolatedStorage";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SteamLogin {
    /// A remembered login exists (DepotDownloader will not ask for the password).
    pub remembered: bool,
    /// The account name it was made for, as far as the launcher knows it.
    pub account: Option<String>,
    /// The files that hold it.
    pub files: Vec<PathBuf>,
}

/// Isolated-storage roots that can hold the launcher's DepotDownloader login.
pub fn storage_roots(dirs: &Dirs) -> Vec<PathBuf> {
    let mut roots = vec![dirs.steam_login_home().join(STORAGE_DIR)];
    if cfg!(windows)
        && let Some(local) = dirs::data_local_dir()
    {
        roots.push(local.join(STORAGE_DIR));
    }
    roots
}

/// Every `account.config` below `root` at the isolated-storage depth
/// (`<id>/<id>/<evidence>/<store>/account.config`), found by walking at most that deep.
pub fn find_login_files(root: &Path) -> Vec<PathBuf> {
    let dirs_in = |p: &Path| -> Vec<PathBuf> {
        std::fs::read_dir(p)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .map(|e| e.path())
            .collect()
    };
    let mut found = Vec::new();
    for a in dirs_in(root) {
        for b in dirs_in(&a) {
            for c in dirs_in(&b) {
                for d in dirs_in(&c) {
                    let f = d.join(ACCOUNT_FILE);
                    if f.is_file() {
                        found.push(f);
                    }
                }
            }
        }
    }
    found.sort();
    found
}

/// What is remembered now. `account` is the name the launcher saved in its settings.
pub fn status(dirs: &Dirs, account: &str) -> SteamLogin {
    let files: Vec<PathBuf> = storage_roots(dirs)
        .iter()
        .flat_map(|r| find_login_files(r))
        .collect();
    let account = Some(account.trim().to_owned()).filter(|a| !a.is_empty());
    SteamLogin {
        remembered: !files.is_empty(),
        account,
        files,
    }
}

/// Deletes the remembered login; returns how many files were removed.
pub fn forget(dirs: &Dirs) -> Result<usize> {
    let mut n = 0;
    for root in storage_roots(dirs) {
        for f in find_login_files(&root) {
            std::fs::remove_file(&f).with_context(|| format!("removing {}", f.display()))?;
            n += 1;
        }
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make(dirs: &Dirs, id: &str) -> PathBuf {
        let d = dirs
            .steam_login_home()
            .join(STORAGE_DIR)
            .join("wpyxktyf.23q")
            .join("pcashsiq.gza")
            .join(format!("Url.{id}"))
            .join("AssemFiles");
        std::fs::create_dir_all(&d).unwrap();
        let f = d.join(ACCOUNT_FILE);
        std::fs::write(&f, b"\x0a\x05alice token").unwrap();
        f
    }

    #[test]
    fn finds_and_forgets_only_account_config_in_the_launchers_storage() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = Dirs::under(tmp.path());
        assert!(!status(&dirs, "").remembered);
        assert_eq!(forget(&dirs).unwrap(), 0);

        let f = make(&dirs, "abc");
        // Something else in the same store, and a same-named file at the wrong depth, stay.
        let other = f.parent().unwrap().join("other.dat");
        std::fs::write(&other, b"x").unwrap();
        let shallow = dirs.steam_login_home().join(STORAGE_DIR).join(ACCOUNT_FILE);
        std::fs::create_dir_all(shallow.parent().unwrap()).unwrap();
        std::fs::write(&shallow, b"x").unwrap();
        // The game install and a pretend Steam folder are not looked at.
        let game = dirs.default_install_dir("0.5.5e").join(ACCOUNT_FILE);
        std::fs::create_dir_all(game.parent().unwrap()).unwrap();
        std::fs::write(&game, b"x").unwrap();

        let st = status(&dirs, " alice ");
        assert!(st.remembered);
        assert_eq!(st.account.as_deref(), Some("alice"));
        assert_eq!(st.files, vec![f.clone()]);

        assert_eq!(forget(&dirs).unwrap(), 1);
        assert!(!f.exists() && other.exists() && shallow.exists() && game.exists());
        let st = status(&dirs, "alice");
        assert!(!st.remembered && st.files.is_empty());
        assert_eq!(forget(&dirs).unwrap(), 0, "nothing left, no error");
    }

    #[test]
    fn several_stores_are_all_forgotten() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = Dirs::under(tmp.path());
        let (a, b) = (make(&dirs, "one"), make(&dirs, "two"));
        assert_eq!(status(&dirs, "").files.len(), 2);
        assert_eq!(forget(&dirs).unwrap(), 2);
        assert!(!a.exists() && !b.exists());
    }
}
