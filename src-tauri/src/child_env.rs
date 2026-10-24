//! The environment of programs the launcher starts (Proton, the game, a hosted server,
//! DepotDownloader).
//!
//! Started from an AppImage, the launcher's own environment points into the AppImage's mount
//! (`$APPDIR`): `PYTHONHOME`/`PYTHONPATH` (Proton is a Python script and dies with "No module
//! named 'encodings'"), `LD_LIBRARY_PATH` (the bundled libraries), `PATH`, the GTK/GIO/GStreamer
//! module paths. A child inherits all of them, so they are removed or filtered here.

use std::process::Command;

/// Removes the AppImage's variables from what `cmd` inherits: path lists lose the entries under
/// `$APPDIR`, other variables pointing into it are dropped. Variables set on `cmd` itself are
/// kept. Does nothing outside an AppImage.
pub fn clean_appimage_env(cmd: &mut Command) {
    let Some(appdir) = std::env::var_os("APPDIR") else {
        return;
    };
    let appdir = appdir.to_string_lossy().trim_end_matches('/').to_string();
    if appdir.is_empty() {
        return;
    }
    let explicit: Vec<_> = cmd.get_envs().map(|(k, _)| k.to_os_string()).collect();
    for (key, value) in std::env::vars_os() {
        if explicit.contains(&key) {
            continue;
        }
        match cleaned(&key.to_string_lossy(), &value.to_string_lossy(), &appdir) {
            Cleaned::Keep => {}
            Cleaned::Remove => {
                cmd.env_remove(&key);
            }
            Cleaned::Set(v) => {
                cmd.env(&key, v);
            }
        }
    }
}

#[derive(Debug, PartialEq)]
enum Cleaned {
    Keep,
    Remove,
    Set(String),
}

fn cleaned(key: &str, value: &str, appdir: &str) -> Cleaned {
    if matches!(key, "APPDIR" | "APPIMAGE" | "ARGV0" | "OWD") {
        return Cleaned::Remove;
    }
    if !value.contains(appdir) {
        return Cleaned::Keep;
    }
    if value.contains(':') {
        let rest: Vec<&str> = value
            .split(':')
            .filter(|p| !p.is_empty() && !p.starts_with(appdir))
            .collect();
        if rest.is_empty() {
            Cleaned::Remove
        } else {
            Cleaned::Set(rest.join(":"))
        }
    } else {
        Cleaned::Remove
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const APPDIR: &str = "/tmp/.mount_poe2_abc";

    #[test]
    fn path_lists_lose_appimage_entries() {
        assert_eq!(
            cleaned(
                "PATH",
                "/tmp/.mount_poe2_abc/usr/bin/:/usr/local/bin:/usr/bin",
                APPDIR
            ),
            Cleaned::Set("/usr/local/bin:/usr/bin".into())
        );
        assert_eq!(
            cleaned(
                "LD_LIBRARY_PATH",
                "/tmp/.mount_poe2_abc/usr/lib/:/tmp/.mount_poe2_abc/usr/lib32/",
                APPDIR
            ),
            Cleaned::Remove
        );
        assert_eq!(
            cleaned(
                "PYTHONPATH",
                "/tmp/.mount_poe2_abc/usr/share/pyshared/:",
                APPDIR
            ),
            Cleaned::Remove
        );
    }

    #[test]
    fn single_values_inside_the_appimage_are_removed() {
        assert_eq!(
            cleaned("PYTHONHOME", "/tmp/.mount_poe2_abc/usr/", APPDIR),
            Cleaned::Remove
        );
        assert_eq!(
            cleaned("APPIMAGE", "/home/u/poe2.appimage", APPDIR),
            Cleaned::Remove
        );
    }

    #[test]
    fn other_variables_are_kept() {
        assert_eq!(cleaned("HOME", "/home/u", APPDIR), Cleaned::Keep);
        assert_eq!(
            cleaned("PYTHONDONTWRITEBYTECODE", "1", APPDIR),
            Cleaned::Keep
        );
    }

    #[test]
    fn explicit_variables_win() {
        let mut cmd = Command::new("true");
        cmd.env("PATH", "/custom");
        clean_appimage_env(&mut cmd);
        let path = cmd
            .get_envs()
            .find(|(k, _)| *k == "PATH")
            .and_then(|(_, v)| v);
        assert_eq!(path, Some(std::ffi::OsStr::new("/custom")));
    }
}
