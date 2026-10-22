//! Linux desktop integration for an AppImage (or any binary that is not installed by a package):
//! a `.desktop` entry and the icon in the user's XDG folders, so the compositor can show the
//! icon and the name.
//!
//! On Wayland the compositor takes a window's icon from the `.desktop` file that matches its
//! `app_id`; the launcher's `app_id` is its identifier, [`APP_ID`]. The entry is named
//! `<APP_ID>.desktop` and says `StartupWMClass=<APP_ID>`, the icon is `<APP_ID>` in the hicolor
//! theme. Written by the "Install desktop entry" action in Settings; removed by its
//! counterpart. Windows and macOS have their own installers and report "unsupported".

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;

pub const APP_ID: &str = "org.poe2modding.launcher";
const NAME: &str = "Poe2 Launcher";

const SVG: &[u8] = include_bytes!("../../assets/icon.svg");
const PNGS: &[(&str, &[u8])] = &[
    ("32x32", include_bytes!("../icons/32x32.png")),
    ("128x128", include_bytes!("../icons/128x128.png")),
    ("256x256", include_bytes!("../icons/128x128@2x.png")),
    ("512x512", include_bytes!("../icons/icon.png")),
];

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DesktopEntry {
    /// Linux only; elsewhere the action is hidden.
    pub supported: bool,
    /// The `.desktop` file exists.
    pub installed: bool,
    /// What it starts (`Exec`), when installed.
    pub exec: Option<String>,
    /// The entry starts another program than the one that is running now (the AppImage moved).
    pub stale: bool,
    /// Where the entry is (or would be) written.
    pub file: Option<PathBuf>,
    /// The program an install would point at now.
    pub current_exec: Option<String>,
    /// This process runs from an AppImage.
    pub appimage: bool,
}

/// The XDG data folder (`~/.local/share`).
fn data_home() -> Option<PathBuf> {
    dirs::data_dir()
}

fn desktop_file(data: &Path) -> PathBuf {
    data.join("applications").join(format!("{APP_ID}.desktop"))
}

fn icon_files(data: &Path) -> Vec<PathBuf> {
    let base = data.join("icons/hicolor");
    let mut v: Vec<PathBuf> = PNGS
        .iter()
        .map(|(size, _)| base.join(size).join("apps").join(format!("{APP_ID}.png")))
        .collect();
    v.push(base.join("scalable/apps").join(format!("{APP_ID}.svg")));
    v
}

/// The program a launcher entry should start: the AppImage file when running from one
/// (`$APPIMAGE`), otherwise this executable.
pub fn current_exec() -> Option<(PathBuf, bool)> {
    if let Some(ai) = std::env::var_os("APPIMAGE").filter(|v| !v.is_empty()) {
        return Some((PathBuf::from(ai), true));
    }
    std::env::current_exe().ok().map(|p| (p, false))
}

/// Quotes a path for the `Exec` key (Desktop Entry spec: double quotes, with `"`, `` ` ``, `$`
/// and `\` escaped; `%` doubled so it is not a field code).
pub fn quote_exec(path: &str) -> String {
    let mut out = String::from("\"");
    for c in path.chars() {
        match c {
            '"' | '`' | '$' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            '%' => out.push_str("%%"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

pub fn desktop_text(exec: &Path) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name={NAME}\n\
         GenericName=Game launcher\n\
         Comment=Launcher for the Path of Exile 2 private server (unofficial)\n\
         Exec={} %U\n\
         Icon={APP_ID}\n\
         Terminal=false\n\
         Categories=Game;\n\
         StartupNotify=true\n\
         StartupWMClass={APP_ID}\n",
        quote_exec(&exec.display().to_string())
    )
}

/// The `Exec` program of an existing entry (unquoted), if it can be read.
fn read_exec(file: &Path) -> Option<String> {
    let text = std::fs::read_to_string(file).ok()?;
    let line = text.lines().find_map(|l| l.strip_prefix("Exec="))?;
    let line = line.trim();
    let program = if let Some(rest) = line.strip_prefix('"') {
        let mut out = String::new();
        let mut chars = rest.chars();
        while let Some(c) = chars.next() {
            match c {
                '\\' => out.extend(chars.next()),
                '"' => break,
                _ => out.push(c),
            }
        }
        out.replace("%%", "%")
    } else {
        line.split_whitespace().next()?.to_owned()
    };
    Some(program)
}

fn status_at(data: Option<&Path>, exec: Option<(PathBuf, bool)>) -> DesktopEntry {
    let appimage = exec.as_ref().is_some_and(|e| e.1);
    let current = exec.map(|e| e.0.display().to_string());
    let Some(data) = data.filter(|_| cfg!(target_os = "linux")) else {
        return DesktopEntry {
            supported: false,
            installed: false,
            exec: None,
            stale: false,
            file: None,
            current_exec: current,
            appimage,
        };
    };
    let file = desktop_file(data);
    let installed = file.is_file();
    let exec_now = installed.then(|| read_exec(&file)).flatten();
    let stale = installed && exec_now.is_some() && exec_now != current;
    DesktopEntry {
        supported: true,
        installed,
        exec: exec_now,
        stale,
        file: Some(file),
        current_exec: current,
        appimage,
    }
}

pub fn status() -> DesktopEntry {
    status_at(data_home().as_deref(), current_exec())
}

/// Writes the entry and the icons below `data`; returns the `.desktop` path.
pub fn install_at(data: &Path, exec: &Path) -> Result<PathBuf> {
    let file = desktop_file(data);
    std::fs::create_dir_all(file.parent().unwrap())?;
    std::fs::write(&file, desktop_text(exec))
        .with_context(|| format!("writing {}", file.display()))?;
    let files = icon_files(data);
    for (path, bytes) in files
        .iter()
        .zip(PNGS.iter().map(|(_, b)| *b).chain(std::iter::once(SVG)))
    {
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(path, bytes).with_context(|| format!("writing {}", path.display()))?;
    }
    Ok(file)
}

/// Removes what [`install_at`] wrote. Returns how many files were removed.
pub fn uninstall_at(data: &Path) -> Result<usize> {
    let mut n = 0;
    for f in std::iter::once(desktop_file(data)).chain(icon_files(data)) {
        match std::fs::remove_file(&f) {
            Ok(()) => n += 1,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).with_context(|| format!("removing {}", f.display())),
        }
    }
    Ok(n)
}

fn require_linux() -> Result<PathBuf> {
    anyhow::ensure!(
        cfg!(target_os = "linux"),
        "desktop entries are for Linux only"
    );
    data_home().context("no XDG data folder on this system")
}

pub fn install() -> Result<DesktopEntry> {
    let data = require_linux()?;
    let (exec, _) = current_exec().context("cannot tell which program is running")?;
    install_at(&data, &exec)?;
    Ok(status())
}

pub fn uninstall() -> Result<DesktopEntry> {
    uninstall_at(&require_linux()?)?;
    Ok(status())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_is_quoted_and_read_back() {
        assert_eq!(quote_exec("/a b/C.AppImage"), "\"/a b/C.AppImage\"");
        assert_eq!(quote_exec("/x\"y$z`w\\%"), "\"/x\\\"y\\$z\\`w\\\\%%\"");
        let tmp = tempfile::tempdir().unwrap();
        for p in [
            "/home/u/Apps/Poe2 Launcher.AppImage",
            "/opt/we\"ird $dir/100%/x",
        ] {
            let f = install_at(tmp.path(), Path::new(p)).unwrap();
            assert_eq!(read_exec(&f).as_deref(), Some(p));
        }
    }

    #[test]
    fn the_entry_matches_the_wayland_app_id() {
        let text = desktop_text(Path::new("/x/y"));
        assert!(text.starts_with("[Desktop Entry]\n"));
        assert!(text.contains(&format!("Icon={APP_ID}\n")));
        assert!(text.contains(&format!("StartupWMClass={APP_ID}\n")));
        assert!(text.contains("Exec=\"/x/y\" %U\n"));
        assert!(text.contains("Terminal=false") && text.contains("Categories=Game;"));
        // The app id is the Tauri identifier.
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(conf["identifier"], APP_ID);
    }

    #[test]
    fn install_writes_entry_and_icons_and_uninstall_removes_them() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path();
        let exec = PathBuf::from("/apps/Poe2.AppImage");
        let file = install_at(data, &exec).unwrap();
        assert_eq!(
            file,
            data.join("applications/org.poe2modding.launcher.desktop")
        );
        for size in ["32x32", "128x128", "256x256", "512x512"] {
            let p = data.join(format!(
                "icons/hicolor/{size}/apps/org.poe2modding.launcher.png"
            ));
            let bytes = std::fs::read(&p).unwrap();
            assert_eq!(&bytes[1..4], b"PNG", "{size}");
        }
        let svg = std::fs::read_to_string(
            data.join("icons/hicolor/scalable/apps/org.poe2modding.launcher.svg"),
        )
        .unwrap();
        assert!(svg.contains("<svg"));
        // A second install replaces, never fails.
        install_at(data, Path::new("/apps/New.AppImage")).unwrap();
        assert_eq!(read_exec(&file).as_deref(), Some("/apps/New.AppImage"));

        // Unrelated files stay.
        let other = data.join("applications/other.desktop");
        std::fs::write(&other, "x").unwrap();
        assert_eq!(uninstall_at(data).unwrap(), 6);
        assert!(!file.exists() && other.exists());
        assert_eq!(uninstall_at(data).unwrap(), 0, "already gone is fine");
    }

    #[test]
    fn status_reports_stale_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let here = Some((PathBuf::from("/apps/Poe2.AppImage"), true));
        let s = status_at(Some(tmp.path()), here.clone());
        if cfg!(target_os = "linux") {
            assert!(s.supported && !s.installed && !s.stale && s.appimage);
            install_at(tmp.path(), Path::new("/apps/Poe2.AppImage")).unwrap();
            let s = status_at(Some(tmp.path()), here.clone());
            assert!(s.installed && !s.stale);
            assert_eq!(s.exec.as_deref(), Some("/apps/Poe2.AppImage"));
            // The AppImage moved: the entry points at the old place.
            let moved = Some((PathBuf::from("/elsewhere/Poe2.AppImage"), true));
            assert!(status_at(Some(tmp.path()), moved).stale);
        } else {
            assert!(!s.supported);
        }
    }
}
