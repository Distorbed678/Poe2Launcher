//! "Client hook": getting the hook DLLs (`poe_hook.dll`, `poe_hook_core.dll`) from a release of
//! Poe2ClientHook onto the machine, so that Play can deploy them without a build environment.
//! No processes here and no Tauri types, so it is all testable.
//!
//! The release is a package built by that repository's CI (`.github/scripts/package-hook.py`,
//! `docs/releasing.md` there): `poe-hook-v<ver>-game-<game versions>-windows-x86_64.zip`
//! (game versions joined with `_`) with a `.sha256` next to it, tag `hook-v<ver>`. The zip has
//! one top folder with the two DLLs, the signature tables and a `release.json` that lists the
//! hook version, the game versions the hook supports, the mod ABI and the SHA-256 of every
//! other file. The launcher checks the archive against the `.sha256`, then every file against
//! `release.json`, before the package becomes usable, and again right before a deploy.
//!
//! The DLLs are Windows binaries whatever the host OS is (under Linux the game runs in Proton),
//! so unlike the server package there is no platform in the name to choose by.

use std::{
    collections::HashSet,
    path::{Component, Path, PathBuf},
};

use anyhow::{Context, Result, anyhow, bail, ensure};
use poe_hook_deploy::{CORE_DLL, LOADER_BUILD_NAME};
use serde::{Deserialize, Serialize};

use crate::{
    host::{GhRelease, extract_archive, find_package_root, http_client},
    paths::{Dirs, reject_steam_paths},
    verify::sha256_file,
};

/// GitHub repository whose releases (tag `hook-v*`) carry the hook packages.
pub const REPO: &str = "Distorbed678/Poe2ClientHook";
pub const TAG_PREFIX: &str = "hook-v";
const ASSET_PREFIX: &str = "poe-hook-v";
/// Files a usable package must list: what the deploy copies.
pub const REQUIRED_FILES: [&str; 2] = [LOADER_BUILD_NAME, CORE_DLL];
const MAX_FILES: usize = 200;

/// Where the launcher keeps the hook packages (below its data directory).
#[derive(Debug, Clone)]
pub struct HookDirs {
    root: PathBuf,
}

impl HookDirs {
    pub fn new(dirs: &Dirs) -> Self {
        Self {
            root: dirs.data.join("hook"),
        }
    }
    /// Unpacked packages, one folder per hook version.
    pub fn packages(&self) -> PathBuf {
        self.root.join("packages")
    }
    pub fn downloads(&self) -> PathBuf {
        self.root.join("downloads")
    }
}

// ---------------------------------------------------------------------------------------------
// Finding the release

/// `poe-hook-v<version>-game-<games>-windows-x86_64.zip` taken apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetName {
    pub version: String,
    pub game_versions: Vec<String>,
}

pub fn parse_asset_name(name: &str) -> Option<AssetName> {
    let rest = name
        .strip_prefix(ASSET_PREFIX)?
        .strip_suffix(".zip")?
        .strip_suffix("-windows-x86_64")?;
    let (version, games) = rest.split_once("-game-")?;
    semver::Version::parse(version).ok()?;
    let game_versions: Vec<String> = games.split('_').map(str::to_owned).collect();
    if game_versions.iter().any(|g| !valid_game_version(g)) {
        return None;
    }
    Some(AssetName {
        version: version.into(),
        game_versions,
    })
}

/// Game versions look like `0.5.5e`; they end up in file names and are compared as text.
pub fn valid_game_version(g: &str) -> bool {
    !g.is_empty()
        && g.len() <= 32
        && g.starts_with(|c: char| c.is_ascii_digit())
        && g.chars().all(|c| c.is_ascii_alphanumeric() || c == '.')
}

/// The release picked for a game version.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct HookRelease {
    pub tag: String,
    pub version: String,
    pub game_versions: Vec<String>,
    pub prerelease: bool,
    pub asset_name: String,
    pub asset_url: String,
    pub size: u64,
    /// The `.sha256` file next to the archive.
    pub sha256_url: String,
}

/// The newest stable release (a pre-release only when there is no stable one) whose package
/// supports `game_version`, with its checksum file.
pub fn pick_release(releases: &[GhRelease], game_version: &str) -> Result<HookRelease> {
    let mut best: Option<((bool, semver::Version), HookRelease)> = None;
    let mut seen_games: Vec<String> = Vec::new();
    for r in releases.iter().filter(|r| !r.draft) {
        if !r.tag_name.starts_with(TAG_PREFIX) {
            continue;
        }
        for a in &r.assets {
            let Some(n) = parse_asset_name(&a.name) else {
                continue;
            };
            if !n.game_versions.iter().any(|g| g == game_version) {
                for g in &n.game_versions {
                    if !seen_games.contains(g) {
                        seen_games.push(g.clone());
                    }
                }
                continue;
            }
            let sha_name = format!("{}.sha256", a.name.trim_end_matches(".zip"));
            let Some(sha) = r.assets.iter().find(|s| s.name == sha_name) else {
                continue;
            };
            let Ok(v) = semver::Version::parse(&n.version) else {
                continue;
            };
            let key = (!r.prerelease, v);
            if best.as_ref().is_none_or(|(k, _)| key > *k) {
                best = Some((
                    key,
                    HookRelease {
                        tag: r.tag_name.clone(),
                        version: n.version,
                        game_versions: n.game_versions,
                        prerelease: r.prerelease,
                        asset_name: a.name.clone(),
                        asset_url: a.browser_download_url.clone(),
                        size: a.size,
                        sha256_url: sha.browser_download_url.clone(),
                    },
                ));
            }
        }
    }
    if let Some((_, r)) = best {
        return Ok(r);
    }
    if seen_games.is_empty() {
        bail!(
            "no client hook release was found. Releases are tagged {TAG_PREFIX}<version>; if \
             none exists yet, build the hook yourself and set the Hook DLL directory in Settings"
        );
    }
    seen_games.sort();
    bail!(
        "no client hook release supports game version {game_version} (releases exist for: {}). \
         Update the launcher, or build the hook yourself and set the Hook DLL directory in Settings",
        seen_games.join(", ")
    )
}

/// Lists the GitHub releases of [`REPO`]. `api_base` is a parameter for tests.
pub async fn fetch_releases(api_base: &str) -> Result<Vec<GhRelease>> {
    let url = format!(
        "{}/repos/{REPO}/releases?per_page=30",
        api_base.trim_end_matches('/')
    );
    let res = http_client()?
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| {
            anyhow!(
                "cannot reach GitHub to look for client hook releases: {}",
                e.without_url()
            )
        })?;
    let status = res.status();
    if status.is_success() {
        return res
            .json::<Vec<GhRelease>>()
            .await
            .context("GitHub's release list is not what was expected");
    }
    match status.as_u16() {
        404 => bail!(
            "no client hook releases can be read from github.com/{REPO}: the repository has no \
             releases yet or is not public (GitHub answers 404 for both)"
        ),
        403 | 429 => bail!(
            "GitHub refused the request (HTTP {}), probably a rate limit; try again in a few minutes",
            status.as_u16()
        ),
        code => bail!("GitHub answered HTTP {code} when listing the client hook releases"),
    }
}

// ---------------------------------------------------------------------------------------------
// The package

/// One file of `release.json`.
#[derive(Debug, Clone, Deserialize)]
struct ListedFile {
    path: String,
    size: u64,
    sha256: String,
}

#[derive(Debug, Deserialize)]
struct ReleaseJson {
    #[serde(default)]
    schema: u32,
    #[serde(default)]
    name: String,
    hook_version: String,
    game_versions: Vec<String>,
    #[serde(default)]
    mod_abi: u32,
    #[serde(default)]
    platform: String,
    #[serde(default)]
    arch: String,
    #[serde(default)]
    commit: String,
    files: Vec<ListedFile>,
}

/// An unpacked, checked package.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct InstalledHook {
    pub dir: PathBuf,
    pub name: String,
    pub version: String,
    pub game_versions: Vec<String>,
    pub mod_abi: u32,
    pub commit: String,
}

/// A path from `release.json`: relative, `/`-separated, no `..`, no drive or root.
fn safe_listed_path(p: &str) -> bool {
    !p.is_empty()
        && !p.contains(['\\', ':'])
        && Path::new(p)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}

/// Reads `release.json` and checks its form (not the files).
fn read_release_json(dir: &Path) -> Result<ReleaseJson> {
    let text = std::fs::read_to_string(dir.join("release.json")).with_context(|| {
        format!(
            "{} has no release.json: this is not a client hook package",
            dir.display()
        )
    })?;
    let r: ReleaseJson = serde_json::from_str(&text).context("release.json is not valid")?;
    ensure!(
        r.schema == 1,
        "release.json has schema {}, this launcher reads schema 1; update the launcher",
        r.schema
    );
    semver::Version::parse(&r.hook_version)
        .map_err(|_| anyhow!("release.json: {:?} is not a version", r.hook_version))?;
    ensure!(
        !r.game_versions.is_empty() && r.game_versions.iter().all(|g| valid_game_version(g)),
        "release.json lists no usable game versions"
    );
    ensure!(
        r.platform == "windows" && r.arch == "x86_64",
        "this hook package is for {} {}; the client is a Windows x86_64 program",
        r.platform,
        r.arch
    );
    ensure!(
        !r.files.is_empty() && r.files.len() <= MAX_FILES,
        "release.json lists no files, or too many"
    );
    let mut seen = HashSet::new();
    for f in &r.files {
        ensure!(
            safe_listed_path(&f.path),
            "release.json lists an unsafe path: {:?}",
            f.path
        );
        ensure!(
            seen.insert(f.path.as_str()),
            "release.json lists {} twice",
            f.path
        );
        ensure!(
            f.sha256.len() == 64 && f.sha256.chars().all(|c| c.is_ascii_hexdigit()),
            "release.json: the checksum of {} is not a SHA-256",
            f.path
        );
    }
    for needed in REQUIRED_FILES {
        ensure!(
            r.files.iter().any(|f| f.path == needed),
            "release.json does not list {needed}"
        );
    }
    Ok(r)
}

fn installed_from(dir: &Path, r: ReleaseJson) -> InstalledHook {
    InstalledHook {
        dir: dir.to_path_buf(),
        name: if r.name.is_empty() {
            dir.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        } else {
            r.name
        },
        version: r.hook_version,
        game_versions: r.game_versions,
        mod_abi: r.mod_abi,
        commit: r.commit,
    }
}

/// Every file under `dir` (relative, `/`-separated); a link or special file is an error.
fn list_files(dir: &Path) -> Result<Vec<String>> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<String>) -> Result<()> {
        for e in std::fs::read_dir(dir)? {
            let e = e?;
            let kind = e.file_type()?;
            ensure!(
                !kind.is_symlink(),
                "the package contains a symbolic link ({}), which is refused",
                e.path().display()
            );
            if kind.is_dir() {
                walk(base, &e.path(), out)?;
            } else {
                let rel = e
                    .path()
                    .strip_prefix(base)?
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push(rel);
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out)?;
    out.sort();
    Ok(out)
}

/// The full check of an unpacked package: `release.json` is well formed, every listed file is
/// there with the listed size and SHA-256, and nothing else is in the folder. Blocking (hashes
/// the DLLs): call from `spawn_blocking`.
pub fn verify_package(dir: &Path) -> Result<InstalledHook> {
    let r = read_release_json(dir)?;
    let present = list_files(dir)?;
    let listed: HashSet<&str> = r.files.iter().map(|f| f.path.as_str()).collect();
    for p in &present {
        ensure!(
            p == "release.json" || listed.contains(p.as_str()),
            "the package contains {p}, which release.json does not list; it was refused"
        );
    }
    for f in &r.files {
        let path = dir.join(&f.path);
        let meta = std::fs::metadata(&path)
            .map_err(|_| anyhow!("the package is incomplete: {} is missing", f.path))?;
        ensure!(
            meta.is_file() && meta.len() == f.size,
            "{} has {} bytes, release.json says {}",
            f.path,
            meta.len(),
            f.size
        );
        let got = sha256_file(&path)?;
        ensure!(
            got == f.sha256.to_ascii_lowercase(),
            "{} does not match the checksum in release.json (expected {}, got {got})",
            f.path,
            f.sha256
        );
    }
    Ok(installed_from(dir, r))
}

/// The cheap check used for status: `release.json` is fine and the listed files exist with the
/// listed size (no hashing).
pub fn inspect_package(dir: &Path) -> Result<InstalledHook> {
    let r = read_release_json(dir)?;
    for f in &r.files {
        let ok =
            std::fs::metadata(dir.join(&f.path)).is_ok_and(|m| m.is_file() && m.len() == f.size);
        ensure!(
            ok,
            "the installed hook is damaged: {} is missing or changed",
            f.path
        );
    }
    Ok(installed_from(dir, r))
}

/// Unpacks `archive` (a zip) into `packages_dir/<hook version>` after checking it against
/// `game_version` (the package must support it). The old folder of the same version is
/// replaced; a package that fails any check leaves nothing behind.
pub fn install_package(
    archive: &Path,
    packages_dir: &Path,
    game_version: &str,
) -> Result<InstalledHook> {
    reject_steam_paths(packages_dir, "client hook folder")?;
    std::fs::create_dir_all(packages_dir)?;
    let staging = packages_dir.join(".staging");
    if staging.exists() {
        std::fs::remove_dir_all(&staging)?;
    }
    let result = (|| -> Result<InstalledHook> {
        ensure!(archive.is_file(), "{} does not exist", archive.display());
        ensure!(
            archive
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("zip")),
            "a client hook package is a .zip file"
        );
        extract_archive(archive, &staging)?;
        let root = find_package_root(&staging).map_err(|_| {
            anyhow!("the archive does not contain a client hook package (no release.json)")
        })?;
        let info = verify_package(&root)?;
        ensure!(
            info.game_versions.iter().any(|g| g == game_version),
            "this hook package supports game version {}, your launcher works with {game_version}",
            info.game_versions.join(", ")
        );
        // The version becomes a folder name: it was parsed as semver, so it is plain text.
        let target = packages_dir.join(&info.version);
        if target.exists() {
            std::fs::remove_dir_all(&target)
                .with_context(|| format!("replacing {}", target.display()))?;
        }
        std::fs::rename(&root, &target)?;
        Ok(InstalledHook {
            dir: target,
            ..info
        })
    })();
    let _ = std::fs::remove_dir_all(&staging);
    result
}

/// Every package folder in `packages_dir` that passes the cheap check, newest first.
pub fn list_installed(packages_dir: &Path) -> Vec<InstalledHook> {
    let Ok(rd) = std::fs::read_dir(packages_dir) else {
        return Vec::new();
    };
    let mut out: Vec<(semver::Version, InstalledHook)> = rd
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| inspect_package(&e.path()).ok())
        .filter_map(|h| semver::Version::parse(&h.version).ok().map(|v| (v, h)))
        .collect();
    out.sort_by(|a, b| b.0.cmp(&a.0));
    out.into_iter().map(|(_, h)| h).collect()
}

/// The newest installed package that supports `game_version`.
pub fn active_for(installed: &[InstalledHook], game_version: &str) -> Option<InstalledHook> {
    installed
        .iter()
        .find(|h| h.game_versions.iter().any(|g| g == game_version))
        .cloned()
}

/// Removes the packages `keep` makes obsolete: older versions that support no game version
/// the kept one does not. Packages for other game versions stay.
pub fn prune_packages(packages_dir: &Path, keep: &InstalledHook) {
    let Ok(keep_v) = semver::Version::parse(&keep.version) else {
        return;
    };
    for h in list_installed(packages_dir) {
        let older = semver::Version::parse(&h.version).is_ok_and(|v| v < keep_v);
        let covered = h
            .game_versions
            .iter()
            .all(|g| keep.game_versions.contains(g));
        if older && covered && h.dir != keep.dir {
            let _ = std::fs::remove_dir_all(&h.dir);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::GhAsset;

    fn rel(tag: &str, pre: bool, names: &[&str]) -> GhRelease {
        GhRelease {
            tag_name: tag.into(),
            draft: false,
            prerelease: pre,
            assets: names
                .iter()
                .map(|n| GhAsset {
                    name: (*n).into(),
                    size: 10,
                    browser_download_url: format!("https://x/{n}"),
                })
                .collect(),
        }
    }

    fn pair(v: &str, games: &str) -> [String; 2] {
        let n = format!("poe-hook-v{v}-game-{games}-windows-x86_64");
        [format!("{n}.zip"), format!("{n}.sha256")]
    }

    fn release(tag: &str, pre: bool, v: &str, games: &str) -> GhRelease {
        let [a, b] = pair(v, games);
        rel(tag, pre, &[&a, &b])
    }

    #[test]
    fn asset_names_are_taken_apart() {
        let n = parse_asset_name("poe-hook-v0.2.0-game-0.5.5e-windows-x86_64.zip").unwrap();
        assert_eq!(
            (n.version.as_str(), n.game_versions),
            ("0.2.0", vec!["0.5.5e".to_owned()])
        );
        let n =
            parse_asset_name("poe-hook-v1.0.0-rc.1-game-0.5.5e_0.6.0-windows-x86_64.zip").unwrap();
        assert_eq!(n.version, "1.0.0-rc.1");
        assert_eq!(n.game_versions, ["0.5.5e", "0.6.0"]);
        for bad in [
            "poe-hook-v0.2.0-game-0.5.5e-windows-x86_64.sha256",
            "poe-hook-v0.2.0-game-0.5.5e-linux-x86_64.zip",
            "poe-hook-vx-game-0.5.5e-windows-x86_64.zip",
            "poe-hook-v0.2.0-game--windows-x86_64.zip",
            "poe-hook-v0.2.0-game-../x-windows-x86_64.zip",
            "poe-server-v0.1.0-game-0.5.5e-linux-x86_64.tar.gz",
        ] {
            assert!(parse_asset_name(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn the_newest_stable_release_for_the_game_version_wins() {
        let rs = vec![
            release("hook-v0.2.0", false, "0.2.0", "0.5.5e"),
            release("hook-v0.3.0", false, "0.3.0", "0.5.5e"),
            release("hook-v0.4.0-rc1", true, "0.4.0-rc1", "0.5.5e"),
            release("hook-v0.9.0", false, "0.9.0", "0.6.0"),
            release("server-v9.0.0", false, "9.0.0", "0.5.5e"),
        ];
        let r = pick_release(&rs, "0.5.5e").unwrap();
        assert_eq!(
            (r.tag.as_str(), r.version.as_str()),
            ("hook-v0.3.0", "0.3.0")
        );
        assert!(r.sha256_url.ends_with(".sha256"));
        assert_eq!(pick_release(&rs, "0.6.0").unwrap().version, "0.9.0");
        // A pre-release only when nothing stable fits.
        let only_pre = vec![release("hook-v0.4.0-rc1", true, "0.4.0-rc1", "0.5.5e")];
        assert!(pick_release(&only_pre, "0.5.5e").unwrap().prerelease);
    }

    #[test]
    fn a_wrong_game_version_or_a_missing_checksum_is_a_clear_refusal() {
        let rs = vec![release("hook-v0.2.0", false, "0.2.0", "0.5.5e")];
        let err = pick_release(&rs, "0.7.0").unwrap_err().to_string();
        assert!(err.contains("0.7.0") && err.contains("0.5.5e"), "{err}");
        let no_sum = vec![rel(
            "hook-v0.2.0",
            false,
            &["poe-hook-v0.2.0-game-0.5.5e-windows-x86_64.zip"],
        )];
        assert!(pick_release(&no_sum, "0.5.5e").is_err());
        assert!(
            pick_release(&[], "0.5.5e")
                .unwrap_err()
                .to_string()
                .contains("no client hook release")
        );
    }

    #[test]
    fn listed_paths_stay_inside_the_package() {
        for ok in ["poe_hook.dll", "signatures/0.5.5e.toml"] {
            assert!(safe_listed_path(ok), "{ok}");
        }
        for bad in [
            "",
            "../x",
            "/etc/passwd",
            "a/../../b",
            "C:evil",
            "a\\b",
            "./a",
        ] {
            assert!(!safe_listed_path(bad), "{bad}");
        }
    }
}
