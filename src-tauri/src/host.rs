//! "Host a server": everything about getting a `poe-server` release onto the machine and
//! turning the wizard's answers into its `config/server.toml`. No processes here (see
//! [`crate::host_run`]) and no Tauri types, so it is all testable.
//!
//! The release is a package built by the main repository's CI (`.github/scripts/package-server.sh`,
//! `docs/guides/releasing-server.md` there): `poe-server-v<ver>-game-<game version>-<linux|windows>-x86_64`
//! as `.tar.gz` / `.zip` with a `.sha256` next to it. It mirrors the repository layout, so the
//! server runs from its own folder. No game files are inside: `game_dir` points at the
//! launcher's installed game.

use std::{
    net::{IpAddr, TcpListener, UdpSocket},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow, bail, ensure};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use toml_edit::{DocumentMut, Item, Table};

use crate::paths::{Dirs, reject_steam_paths, write_atomic};

/// GitHub repository whose releases (tag `server-v*`) carry the server packages.
pub const REPO: &str = "Distorbed678/Poe2ModdingProject";
pub const TAG_PREFIX: &str = "server-v";
pub const API_BASE: &str = "https://api.github.com";
/// Shown to people who want to forward ports.
pub const HOSTING_GUIDE_URL: &str =
    "https://github.com/Distorbed678/Poe2ModdingProject/blob/main/docs/guides/hosting.md";
/// Plain-text "what is my public IP" service, only asked when the user clicks the button.
pub const PUBLIC_IP_URL: &str = "https://api.ipify.org";

const MAX_ENTRIES: usize = 20_000;
const MAX_UNPACKED_BYTES: u64 = 2 << 30;

// ---------------------------------------------------------------------------------------------
// Settings (host.json in the launcher's config directory; no secrets)

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct HostSettings {
    /// Shown to players (`[gateway] display_name`) and used as the server profile's name.
    pub server_name: String,
    /// `[cluster] id`: login tokens are bound to it.
    pub cluster_id: String,
    pub login_port: u16,
    pub instance_port: u16,
    pub web_port: u16,
    /// The address players type (`[network] public_host`): a LAN address, a public IP or a name.
    pub public_host: String,
    /// Let other computers reach the web API (and the web admin): binds `0.0.0.0` instead of
    /// loopback. Passwords travel in clear text over plain http, so only on a network you trust
    /// or behind a TLS proxy.
    pub web_public: bool,
    /// `[accounts] open_registration`.
    pub open_registration: bool,
    /// The admin account made at setup (the name only; the password is never stored).
    pub admin_name: String,
    /// Start the server when the launcher starts.
    pub autostart: bool,
    /// The extracted package the server runs from.
    pub package_dir: Option<PathBuf>,
    /// A config was written and an admin account made.
    pub configured: bool,
    /// The Servers profile that points at this server.
    pub profile_id: Option<String>,
}

impl Default for HostSettings {
    fn default() -> Self {
        Self {
            server_name: "My server".into(),
            cluster_id: "main".into(),
            login_port: 20481,
            instance_port: 6112,
            web_port: 8080,
            public_host: "127.0.0.1".into(),
            web_public: false,
            open_registration: false,
            admin_name: String::new(),
            autostart: false,
            package_dir: None,
            configured: false,
            profile_id: None,
        }
    }
}

impl HostSettings {
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text)
                .with_context(|| format!("{} is not valid server settings JSON", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        write_atomic(path, serde_json::to_string_pretty(self)?.as_bytes())
    }

    /// The answers a user can give in the wizard.
    pub fn validate(&self) -> Result<()> {
        let name = self.server_name.trim();
        ensure!(
            !name.is_empty() && name.chars().count() <= 64 && !name.chars().any(char::is_control),
            "the server name is 1-64 characters without control characters"
        );
        ensure!(
            valid_cluster_id(&self.cluster_id),
            "the cluster id is 1-32 characters: lower-case letters, digits, - and _"
        );
        for (what, port) in [
            ("login", self.login_port),
            ("instance", self.instance_port),
            ("web", self.web_port),
        ] {
            ensure!(port >= 1024, "the {what} port must be 1024 or higher");
        }
        ensure!(
            self.login_port != self.instance_port
                && self.login_port != self.web_port
                && self.instance_port != self.web_port,
            "the login, instance and web ports must be three different numbers"
        );
        ensure!(
            valid_public_host(&self.public_host),
            "the public host is an IP address or a host name (no spaces, no scheme, no port)"
        );
        if !self.admin_name.is_empty() {
            ensure!(
                valid_account_name(&self.admin_name),
                "the admin account name is 3-64 characters: letters, digits and _ . @ + -"
            );
        }
        Ok(())
    }
}

pub fn valid_cluster_id(id: &str) -> bool {
    (1..=32).contains(&id.len())
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

pub fn valid_account_name(name: &str) -> bool {
    (3..=64).contains(&name.chars().count())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_.@+-".contains(c))
}

/// An IP address or a plain host name; never a URL, a port or anything with spaces.
pub fn valid_public_host(host: &str) -> bool {
    let h = host.trim();
    if h.is_empty() || h.len() > 253 {
        return false;
    }
    if h.parse::<IpAddr>().is_ok() {
        return true;
    }
    h.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    })
}

// ---------------------------------------------------------------------------------------------
// Folders

/// Where the launcher keeps the server (all below its data directory, never a Steam library).
#[derive(Debug, Clone)]
pub struct HostDirs {
    config_file: PathBuf,
    backup_file: PathBuf,
    root: PathBuf,
}

impl HostDirs {
    pub fn new(dirs: &Dirs) -> Self {
        Self {
            config_file: dirs.config.join("host.json"),
            backup_file: dirs.config.join("backup.json"),
            root: dirs.data.join("server"),
        }
    }
    pub fn settings_file(&self) -> &Path {
        &self.config_file
    }
    /// Extracted packages, one folder per release.
    pub fn packages(&self) -> PathBuf {
        self.root.join("packages")
    }
    pub fn downloads(&self) -> PathBuf {
        self.root.join("downloads")
    }
    /// The database and anything else that must survive an update of the package.
    pub fn data(&self) -> PathBuf {
        self.root.join("data")
    }
    pub fn database(&self) -> PathBuf {
        self.data().join("server.sqlite")
    }
    pub fn log_file(&self) -> PathBuf {
        self.root.join("server.log")
    }
    /// Backups of the database (see `backup.rs`).
    pub fn backups(&self) -> PathBuf {
        self.root.join("backups")
    }
    pub fn backup_settings_file(&self) -> &Path {
        &self.backup_file
    }
}

// ---------------------------------------------------------------------------------------------
// Finding the release

/// `poe-server-v<version>-game-<game>-<platform>-x86_64.<ext>` taken apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetName {
    pub version: String,
    pub game_version: String,
    pub platform: String,
    pub ext: String,
}

pub fn parse_asset_name(name: &str) -> Option<AssetName> {
    let rest = name.strip_prefix("poe-server-v")?;
    let (rest, ext) = if let Some(r) = rest.strip_suffix(".tar.gz") {
        (r, "tar.gz")
    } else {
        (rest.strip_suffix(".zip")?, "zip")
    };
    let rest = rest.strip_suffix("-x86_64")?;
    let (rest, platform) = rest.rsplit_once('-')?;
    let (version, game) = rest.split_once("-game-")?;
    if version.is_empty() || game.is_empty() || !matches!(platform, "linux" | "windows") {
        return None;
    }
    Some(AssetName {
        version: version.into(),
        game_version: game.into(),
        platform: platform.into(),
        ext: ext.into(),
    })
}

/// The platform word of the package names for an OS name (`std::env::consts::OS`).
pub fn platform_for(os: &str) -> Result<&'static str> {
    match os {
        "linux" => Ok("linux"),
        "windows" => Ok("windows"),
        other => bail!(
            "there is no server package for {other}; hosting from the launcher works on Linux and Windows"
        ),
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct GhAsset {
    pub name: String,
    #[serde(default)]
    pub size: u64,
    pub browser_download_url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GhRelease {
    pub tag_name: String,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub assets: Vec<GhAsset>,
}

/// The package picked for this machine.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ServerRelease {
    pub tag: String,
    pub version: String,
    pub game_version: String,
    pub prerelease: bool,
    pub asset_name: String,
    pub asset_url: String,
    pub size: u64,
    /// The `.sha256` file next to the archive.
    pub sha256_url: String,
}

/// The newest stable release (a pre-release only when there is no stable one) whose package
/// is for `game_version` on `os`, with its checksum file.
pub fn pick_release(releases: &[GhRelease], game_version: &str, os: &str) -> Result<ServerRelease> {
    let platform = platform_for(os)?;
    let mut best: Option<(bool, semver::Version, ServerRelease)> = None;
    let mut seen_games: Vec<String> = Vec::new();
    for r in releases.iter().filter(|r| !r.draft) {
        if !r.tag_name.starts_with(TAG_PREFIX) {
            continue;
        }
        for a in &r.assets {
            let Some(n) = parse_asset_name(&a.name) else {
                continue;
            };
            if n.platform != platform {
                continue;
            }
            if n.game_version != game_version {
                if !seen_games.contains(&n.game_version) {
                    seen_games.push(n.game_version.clone());
                }
                continue;
            }
            let sha_name = format!("{}.sha256", a.name.trim_end_matches(&format!(".{}", n.ext)));
            let Some(sha) = r.assets.iter().find(|s| s.name == sha_name) else {
                continue;
            };
            let Ok(v) = semver::Version::parse(&n.version) else {
                continue;
            };
            let candidate = ServerRelease {
                tag: r.tag_name.clone(),
                version: n.version.clone(),
                game_version: n.game_version,
                prerelease: r.prerelease,
                asset_name: a.name.clone(),
                asset_url: a.browser_download_url.clone(),
                size: a.size,
                sha256_url: sha.browser_download_url.clone(),
            };
            // Stable beats pre-release, then the higher version.
            let key = (!r.prerelease, v);
            if best
                .as_ref()
                .is_none_or(|(s, bv, _)| key > (*s, bv.clone()))
            {
                best = Some((key.0, key.1, candidate));
            }
        }
    }
    if let Some((_, _, r)) = best {
        return Ok(r);
    }
    if seen_games.is_empty() {
        bail!(
            "no server release for {platform} was found. Releases are tagged {TAG_PREFIX}<version>; \
             if none exists yet, use \"Use a local package\""
        );
    }
    bail!(
        "no server release matches your game version {game_version} (releases exist for: {}). \
         Update the launcher, or use \"Use a local package\"",
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
                "cannot reach GitHub to look for server releases: {}",
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
            "no server releases can be read from github.com/{REPO}: the repository is still \
             private or has no releases yet (GitHub answers 404 for both). Use \"Use a local \
             package\" with a package you built or were given"
        ),
        403 | 429 => bail!(
            "GitHub refused the request (HTTP {}), probably a rate limit; try again in a few \
             minutes, or use \"Use a local package\"",
            status.as_u16()
        ),
        code => bail!("GitHub answered HTTP {code} when listing the server releases"),
    }
}

pub(crate) fn http_client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(15))
        .timeout(std::time::Duration::from_secs(600))
        .user_agent(concat!("poe2-launcher/", env!("CARGO_PKG_VERSION")))
        .build()?)
}

// ---------------------------------------------------------------------------------------------
// Download and checksum

/// The hash in a `.sha256` file (`sha256sum` format: `<hex>  <file name>`; the name is optional
/// but must match when present).
pub fn parse_sha256_file(text: &str, archive_name: &str) -> Result<String> {
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .context("the checksum file is empty")?;
    let mut parts = line.split_whitespace();
    let hash = parts.next().unwrap_or_default().to_ascii_lowercase();
    ensure!(
        hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit()),
        "the checksum file does not start with a SHA-256 (64 hex digits)"
    );
    if let Some(name) = parts.next() {
        let name = name.trim_start_matches('*');
        ensure!(
            name == archive_name,
            "the checksum file is for {name}, not {archive_name}"
        );
    }
    Ok(hash)
}

/// Streams `url` into `dest` and returns the SHA-256 and the byte count.
/// `progress(done, total)` (total 0 = unknown).
pub async fn download(
    url: &str,
    dest: &Path,
    progress: &(dyn Fn(u64, u64) + Send + Sync),
) -> Result<(String, u64)> {
    let res = http_client()?
        .get(url)
        .send()
        .await
        .map_err(|e| anyhow!("download failed: {}", e.without_url()))?;
    let status = res.status();
    ensure!(
        status.is_success(),
        "the download answered HTTP {} (the file may have been removed, or the repository is private)",
        status.as_u16()
    );
    let total = res.content_length().unwrap_or(0);
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let mut file = tokio::fs::File::create(dest).await?;
    let mut hasher = Sha256::new();
    let mut done = 0u64;
    let mut stream = res.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("download interrupted")?;
        hasher.update(&chunk);
        file.write_all(&chunk).await?;
        done += chunk.len() as u64;
        progress(done, total);
    }
    file.flush().await?;
    Ok((hex::encode(hasher.finalize()), done))
}

pub async fn download_text(url: &str) -> Result<String> {
    let res = http_client()?
        .get(url)
        .send()
        .await
        .map_err(|e| anyhow!("download failed: {}", e.without_url()))?;
    ensure!(
        res.status().is_success(),
        "the checksum file answered HTTP {}",
        res.status().as_u16()
    );
    let text = res.text().await?;
    ensure!(text.len() < 4096, "the checksum file is too large");
    Ok(text)
}

pub fn sha256_of_file(path: &Path) -> Result<String> {
    crate::verify::sha256_file(path)
}

/// A `.sha256` file that sits next to a local archive (`X.tar.gz.sha256` or `X.sha256`).
pub fn sibling_checksum_file(archive: &Path) -> Option<PathBuf> {
    let name = archive.file_name()?.to_string_lossy().into_owned();
    let stem = parse_asset_name(&name)
        .map(|n| name.trim_end_matches(&format!(".{}", n.ext)).to_owned())
        .unwrap_or_else(|| name.clone());
    [format!("{name}.sha256"), format!("{stem}.sha256")]
        .into_iter()
        .map(|n| archive.with_file_name(n))
        .find(|p| p.is_file())
}

// ---------------------------------------------------------------------------------------------
// The package

/// `release.json` of a package, plus where it is.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PackageInfo {
    pub dir: PathBuf,
    pub name: String,
    pub server_version: String,
    pub game_version: String,
    pub platform: String,
    pub commit: String,
}

#[derive(Deserialize)]
struct ReleaseJson {
    #[serde(default)]
    name: String,
    server_version: String,
    game_version: String,
    platform: String,
    #[serde(default)]
    commit: String,
}

pub fn server_exe_name(os: &str) -> &'static str {
    if os == "windows" {
        "poe-server.exe"
    } else {
        "poe-server"
    }
}

/// Reads and checks an extracted package: it must be for this game version and OS and hold
/// what the server needs from its own folder.
pub fn inspect_package(dir: &Path, game_version: &str, os: &str) -> Result<PackageInfo> {
    let platform = platform_for(os)?;
    let text = std::fs::read_to_string(dir.join("release.json")).with_context(|| {
        format!(
            "{} has no release.json: this is not a server package",
            dir.display()
        )
    })?;
    let r: ReleaseJson = serde_json::from_str(&text).context("release.json is not valid")?;
    ensure!(
        r.platform == platform,
        "this package is for {}, this computer needs the {platform} package",
        r.platform
    );
    ensure!(
        r.game_version == game_version,
        "this package serves game version {}, your launcher installs {game_version}",
        r.game_version
    );
    let exe = server_exe_name(os);
    for needed in [
        exe.to_owned(),
        "config/server.example.toml".to_owned(),
        format!("versions/{game_version}.toml"),
        "third_party/dat-schema/schema-poe2.min.json".to_owned(),
    ] {
        ensure!(
            dir.join(&needed).is_file(),
            "the package is incomplete: {needed} is missing"
        );
    }
    ensure!(
        dir.join("modules/protocol/opcodes").is_dir(),
        "the package is incomplete: modules/protocol/opcodes is missing"
    );
    Ok(PackageInfo {
        dir: dir.to_path_buf(),
        name: if r.name.is_empty() {
            dir.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        } else {
            r.name
        },
        server_version: r.server_version,
        game_version: r.game_version,
        platform: r.platform,
        commit: r.commit,
    })
}

/// Unpacks `archive` (`.tar.gz` or `.zip`) into `dest`: no path may leave `dest`, no links,
/// bounded entry count and size.
pub fn extract_archive(archive: &Path, dest: &Path) -> Result<()> {
    let name = archive
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    std::fs::create_dir_all(dest)?;
    if name.ends_with(".zip") {
        let mut z = zip::ZipArchive::new(std::fs::File::open(archive)?)
            .context("the file is not a readable zip")?;
        ensure!(z.len() <= MAX_ENTRIES, "the archive has too many entries");
        let mut total = 0u64;
        for i in 0..z.len() {
            total += z.by_index(i)?.size();
        }
        ensure!(
            total <= MAX_UNPACKED_BYTES,
            "the archive unpacks to too much data"
        );
        crate::depot::extract_zip(archive, dest)
    } else if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        let gz = flate2::read::GzDecoder::new(std::fs::File::open(archive)?);
        let mut ar = tar::Archive::new(gz);
        let (mut count, mut total) = (0usize, 0u64);
        for entry in ar.entries().context("the file is not a readable .tar.gz")? {
            let mut e = entry.context("the archive is damaged")?;
            let kind = e.header().entry_type();
            ensure!(
                kind.is_file() || kind.is_dir(),
                "the archive contains a link or special file ({:?}), which is refused",
                e.path().unwrap_or_default()
            );
            count += 1;
            total += e.size();
            ensure!(count <= MAX_ENTRIES, "the archive has too many entries");
            ensure!(
                total <= MAX_UNPACKED_BYTES,
                "the archive unpacks to too much data"
            );
            let shown = e
                .path()
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            ensure!(
                e.unpack_in(dest).context("unpacking the archive")?,
                "unsafe path in the archive: {shown}"
            );
        }
        Ok(())
    } else {
        bail!("a server package is a .tar.gz or a .zip file")
    }
}

/// The folder inside `staging` that holds `release.json` (the package's own top folder, or
/// `staging` itself).
pub(crate) fn find_package_root(staging: &Path) -> Result<PathBuf> {
    if staging.join("release.json").is_file() {
        return Ok(staging.to_path_buf());
    }
    let dirs: Vec<PathBuf> = std::fs::read_dir(staging)?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    match dirs.as_slice() {
        [one] if one.join("release.json").is_file() => Ok(one.clone()),
        _ => bail!("the archive does not contain a server package (no release.json)"),
    }
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let kind = e.file_type()?;
        let target = to.join(e.file_name());
        if kind.is_symlink() {
            bail!(
                "the folder contains a symbolic link ({}), which is refused",
                e.path().display()
            );
        } else if kind.is_dir() {
            copy_dir(&e.path(), &target)?;
        } else {
            std::fs::copy(e.path(), &target)?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
    Ok(())
}
#[cfg(not(unix))]
fn make_executable(_: &Path) -> Result<()> {
    Ok(())
}

/// Puts a package (an archive or an extracted folder) into `packages_dir/<name>` and returns
/// it. The old folder of the same name is replaced; nothing is left half-installed.
pub fn install_package(
    source: &Path,
    packages_dir: &Path,
    game_version: &str,
    os: &str,
) -> Result<PackageInfo> {
    reject_steam_paths(packages_dir, "server folder")?;
    std::fs::create_dir_all(packages_dir)?;
    let staging = packages_dir.join(".staging");
    if staging.exists() {
        std::fs::remove_dir_all(&staging)?;
    }
    let result = (|| -> Result<PackageInfo> {
        if source.is_dir() {
            copy_dir(source, &staging)?;
        } else if source.is_file() {
            extract_archive(source, &staging)?;
        } else {
            bail!("{} does not exist", source.display());
        }
        let root = find_package_root(&staging)?;
        let info = inspect_package(&root, game_version, os)?;
        make_executable(&root.join(server_exe_name(os)))?;
        ensure!(
            !info.name.is_empty() && !info.name.contains(['/', '\\']) && info.name != "..",
            "the package has no usable name"
        );
        let target = packages_dir.join(&info.name);
        if target.exists() {
            std::fs::remove_dir_all(&target)
                .with_context(|| format!("replacing {}", target.display()))?;
        }
        std::fs::rename(&root, &target)?;
        Ok(PackageInfo {
            dir: target,
            ..info
        })
    })();
    let _ = std::fs::remove_dir_all(&staging);
    result
}

/// Removes every package folder except `keep`.
pub fn prune_packages(packages_dir: &Path, keep: &Path) {
    if let Ok(rd) = std::fs::read_dir(packages_dir) {
        for e in rd.flatten() {
            if e.path() != keep && e.path().is_dir() {
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The server config

/// Where the config is written: `<package>/config/server.toml`.
pub fn config_path(package: &Path) -> PathBuf {
    package.join("config").join("server.toml")
}

fn table_at<'a>(doc: &'a mut DocumentMut, path: &[&str]) -> &'a mut Table {
    let mut t = doc.as_table_mut();
    for key in path {
        let item = t.entry(key).or_insert(Item::Table(Table::new()));
        if !item.is_table() {
            *item = Item::Table(Table::new());
        }
        t = item.as_table_mut().expect("just made a table");
    }
    t
}

/// Replaces a value and keeps the comment after it on the same line.
fn set_value(doc: &mut DocumentMut, path: &[&str], key: &str, v: toml_edit::Value) {
    let table = table_at(doc, path);
    let mut v = v;
    if let Some(old) = table.get(key).and_then(Item::as_value) {
        *v.decor_mut() = old.decor().clone();
    }
    table[key] = Item::Value(v);
}

fn set_str(doc: &mut DocumentMut, path: &[&str], key: &str, v: &str) {
    set_value(doc, path, key, v.into());
}
fn set_bool(doc: &mut DocumentMut, path: &[&str], key: &str, v: bool) {
    set_value(doc, path, key, v.into());
}
fn set_int(doc: &mut DocumentMut, path: &[&str], key: &str, v: i64) {
    set_value(doc, path, key, v.into());
}

/// `server.example.toml` of the package with the wizard's answers in it. Comments and every
/// other key stay as the package shipped them.
pub fn render_config(
    example: &str,
    s: &HostSettings,
    game_dir: &Path,
    database: &Path,
) -> Result<String> {
    s.validate()?;
    let mut doc: DocumentMut = example
        .parse()
        .context("the package's config/server.example.toml is not valid TOML")?;
    let bind_ip = "0.0.0.0";
    let web_ip = if s.web_public { "0.0.0.0" } else { "127.0.0.1" };

    set_str(&mut doc, &["game"], "game_dir", &game_dir.to_string_lossy());
    set_str(&mut doc, &["network"], "public_host", s.public_host.trim());
    set_str(
        &mut doc,
        &["network", "login"],
        "bind",
        &format!("{bind_ip}:{}", s.login_port),
    );
    set_str(
        &mut doc,
        &["network", "instance"],
        "bind",
        &format!("{bind_ip}:{}", s.instance_port),
    );
    set_int(
        &mut doc,
        &["network", "instance"],
        "public_port",
        i64::from(s.instance_port),
    );
    // The login server's public port is the bind port unless it is set: keep it absent.
    table_at(&mut doc, &["network", "login"]).remove("public_port");
    set_str(&mut doc, &["cluster"], "id", &s.cluster_id);
    set_str(&mut doc, &["gateway"], "display_name", s.server_name.trim());
    set_bool(&mut doc, &["web"], "enabled", true);
    set_str(
        &mut doc,
        &["web"],
        "bind",
        &format!("{web_ip}:{}", s.web_port),
    );
    set_str(&mut doc, &["web"], "static_dir", "web-admin");
    set_bool(
        &mut doc,
        &["accounts"],
        "open_registration",
        s.open_registration,
    );
    set_str(&mut doc, &["database"], "path", &database.to_string_lossy());
    // An admin's "restart" ends the process with code 75 and the launcher starts it again (on
    // every platform: host_run.rs). Older servers ignore the key (an unknown key is only warned
    // about by newer ones).
    set_str(&mut doc, &["server"], "restart_mode", "exit");
    Ok(doc.to_string())
}

// ---------------------------------------------------------------------------------------------
// Network helpers

/// The address of this computer on the local network (the interface the default route uses).
/// A UDP "connect" only picks the route: nothing is sent.
pub fn detect_lan_ip() -> Option<IpAddr> {
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("192.0.2.1:9").ok()?;
    let ip = s.local_addr().ok()?.ip();
    (!ip.is_loopback() && !ip.is_unspecified()).then_some(ip)
}

/// Asks a "what is my IP" service. Only called when the user presses the button.
pub async fn lookup_public_ip(url: &str) -> Result<IpAddr> {
    let res = http_client()?
        .get(url)
        .send()
        .await
        .map_err(|e| anyhow!("cannot reach the IP lookup service: {}", e.without_url()))?;
    ensure!(
        res.status().is_success(),
        "the IP lookup service answered HTTP {}",
        res.status().as_u16()
    );
    let text = res.text().await?;
    text.trim()
        .parse::<IpAddr>()
        .map_err(|_| anyhow!("the IP lookup service did not answer with an address"))
}

/// Ports that something already listens on (TCP for all three, UDP for the instance port, the
/// side channel of the DLL).
pub fn busy_ports(s: &HostSettings) -> Vec<String> {
    let mut busy = Vec::new();
    for (what, port) in [
        ("login", s.login_port),
        ("instance", s.instance_port),
        ("web", s.web_port),
    ] {
        if TcpListener::bind(("0.0.0.0", port)).is_err() {
            busy.push(format!("{what} port {port} (TCP)"));
        }
    }
    if UdpSocket::bind(("0.0.0.0", s.instance_port)).is_err() {
        busy.push(format!("instance port {} (UDP)", s.instance_port));
    }
    busy
}

/// `http://127.0.0.1:<web port>`: the address the launcher uses for its own server.
pub fn local_web_url(s: &HostSettings) -> String {
    format!("http://127.0.0.1:{}", s.web_port)
}

/// Removes terminal colour codes from a server log line.
pub fn strip_ansi(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for n in chars.by_ref() {
                    if ('@'..='~').contains(&n) {
                        break;
                    }
                }
            }
        } else if c != '\r' {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

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
                    browser_download_url: format!("https://example.invalid/{tag}/{n}"),
                })
                .collect(),
        }
    }

    #[test]
    fn asset_names_are_parsed() {
        let n = parse_asset_name("poe-server-v0.1.0-game-0.5.5e-linux-x86_64.tar.gz").unwrap();
        assert_eq!(
            n,
            AssetName {
                version: "0.1.0".into(),
                game_version: "0.5.5e".into(),
                platform: "linux".into(),
                ext: "tar.gz".into()
            }
        );
        let w = parse_asset_name("poe-server-v0.2.0-rc1-game-0.5.5e-windows-x86_64.zip").unwrap();
        assert_eq!(
            (w.version.as_str(), w.platform.as_str(), w.ext.as_str()),
            ("0.2.0-rc1", "windows", "zip")
        );
        for bad in [
            "poe-server-v0.1.0-game-0.5.5e-macos-x86_64.tar.gz",
            "poe-server-v0.1.0-game-0.5.5e-linux-x86_64.sha256",
            "poe-server-v0.1.0-linux-x86_64.zip",
            "other.tar.gz",
        ] {
            assert!(parse_asset_name(bad).is_none(), "{bad}");
        }
    }

    const LIN: &str = "poe-server-v0.1.0-game-0.5.5e-linux-x86_64.tar.gz";
    const LIN_SHA: &str = "poe-server-v0.1.0-game-0.5.5e-linux-x86_64.sha256";
    const LIN2: &str = "poe-server-v0.2.0-game-0.5.5e-linux-x86_64.tar.gz";
    const LIN2_SHA: &str = "poe-server-v0.2.0-game-0.5.5e-linux-x86_64.sha256";

    #[test]
    fn the_newest_matching_stable_release_wins() {
        let all = vec![
            rel("server-v0.1.0", false, &[LIN, LIN_SHA]),
            rel(
                "server-v0.2.0",
                false,
                &[
                    LIN2,
                    LIN2_SHA,
                    "poe-server-v0.2.0-game-0.5.5e-windows-x86_64.zip",
                ],
            ),
            rel(
                "server-v0.3.0-rc1",
                true,
                &[
                    "poe-server-v0.3.0-rc1-game-0.5.5e-linux-x86_64.tar.gz",
                    "poe-server-v0.3.0-rc1-game-0.5.5e-linux-x86_64.sha256",
                ],
            ),
            rel("v9.9.9", false, &["unrelated"]),
        ];
        let r = pick_release(&all, "0.5.5e", "linux").unwrap();
        assert_eq!((r.version.as_str(), r.asset_name.as_str()), ("0.2.0", LIN2));
        assert!(r.sha256_url.ends_with(LIN2_SHA));
        // A pre-release is taken only when nothing stable fits.
        let only_pre = vec![all[2].clone()];
        assert!(
            pick_release(&only_pre, "0.5.5e", "linux")
                .unwrap()
                .prerelease
        );
    }

    #[test]
    fn mismatches_are_explained() {
        let all = vec![rel("server-v0.1.0", false, &[LIN, LIN_SHA])];
        let e = pick_release(&all, "0.6.0", "linux")
            .unwrap_err()
            .to_string();
        assert!(e.contains("0.6.0") && e.contains("0.5.5e"), "{e}");
        let e = pick_release(&all, "0.5.5e", "windows")
            .unwrap_err()
            .to_string();
        assert!(e.contains("windows"), "{e}");
        // No checksum file next to the archive: not usable.
        let nosha = vec![rel("server-v0.1.0", false, &[LIN])];
        assert!(pick_release(&nosha, "0.5.5e", "linux").is_err());
        assert!(pick_release(&[], "0.5.5e", "linux").is_err());
        assert!(pick_release(&all, "0.5.5e", "macos").is_err());
        // Drafts are ignored.
        let mut draft = rel("server-v0.1.0", false, &[LIN, LIN_SHA]);
        draft.draft = true;
        assert!(pick_release(&[draft], "0.5.5e", "linux").is_err());
    }

    #[test]
    fn checksum_files_are_read_strictly() {
        let h = "a".repeat(64);
        assert_eq!(parse_sha256_file(&format!("{h}  {LIN}\n"), LIN).unwrap(), h);
        assert_eq!(
            parse_sha256_file(&format!("{}  *{LIN}", h.to_uppercase()), LIN).unwrap(),
            h
        );
        assert_eq!(parse_sha256_file(&h, LIN).unwrap(), h);
        assert!(parse_sha256_file(&format!("{h}  other.tar.gz"), LIN).is_err());
        assert!(parse_sha256_file("nothex  x", LIN).is_err());
        assert!(parse_sha256_file("", LIN).is_err());
    }

    #[test]
    fn settings_are_validated() {
        let ok = HostSettings::default();
        assert!(ok.validate().is_ok());
        let bad = |f: &dyn Fn(&mut HostSettings)| {
            let mut s = HostSettings::default();
            f(&mut s);
            assert!(s.validate().is_err());
        };
        bad(&|s| s.server_name = "  ".into());
        bad(&|s| s.cluster_id = "Has Space".into());
        bad(&|s| s.login_port = 80);
        bad(&|s| s.web_port = s.login_port);
        bad(&|s| s.public_host = "http://x".into());
        bad(&|s| s.public_host = "a b".into());
        bad(&|s| s.public_host = "x:80".into());
        bad(&|s| s.admin_name = "ab".into());
        bad(&|s| s.admin_name = "bad name".into());
        let mut s = HostSettings::default();
        for h in ["192.168.1.5", "play.example.com", "::1", "my-server"] {
            s.public_host = h.into();
            assert!(s.validate().is_ok(), "{h}");
        }
    }

    const EXAMPLE: &str = r#"# header comment
[game]
version = "0.5.5e"
game_dir = "GameFiles"

[network]
public_host = "127.0.0.1"

[network.login]
bind = "0.0.0.0:20481"
# public_port = 20481
public_port = 20481

[network.instance]
bind = "0.0.0.0:6112"   # trailing comment
public_port = 6112

[web]
enabled = false
bind = "127.0.0.1:8080"
static_dir = ""

[accounts]
open_registration = false
name_prefix = "Exile"

[database]
path = "runtime/server.sqlite"
"#;

    #[test]
    fn the_config_takes_the_answers_and_keeps_everything_else() {
        let s = HostSettings {
            server_name: "Friends \"Only\"".into(),
            cluster_id: "friends".into(),
            login_port: 30481,
            instance_port: 30112,
            web_port: 30080,
            public_host: "203.0.113.7".into(),
            web_public: true,
            open_registration: true,
            ..Default::default()
        };
        let out = render_config(
            EXAMPLE,
            &s,
            Path::new("/games/poe2"),
            Path::new("/d/server.sqlite"),
        )
        .unwrap();
        let doc: toml_edit::DocumentMut = out.parse().unwrap();
        assert_eq!(doc["game"]["game_dir"].as_str(), Some("/games/poe2"));
        assert_eq!(doc["game"]["version"].as_str(), Some("0.5.5e"));
        assert_eq!(doc["network"]["public_host"].as_str(), Some("203.0.113.7"));
        assert_eq!(
            doc["network"]["login"]["bind"].as_str(),
            Some("0.0.0.0:30481")
        );
        assert!(doc["network"]["login"].get("public_port").is_none());
        assert_eq!(
            doc["network"]["instance"]["bind"].as_str(),
            Some("0.0.0.0:30112")
        );
        assert_eq!(
            doc["network"]["instance"]["public_port"].as_integer(),
            Some(30112)
        );
        assert_eq!(doc["cluster"]["id"].as_str(), Some("friends"));
        assert_eq!(
            doc["gateway"]["display_name"].as_str(),
            Some("Friends \"Only\"")
        );
        assert_eq!(doc["web"]["enabled"].as_bool(), Some(true));
        assert_eq!(doc["web"]["bind"].as_str(), Some("0.0.0.0:30080"));
        assert_eq!(doc["web"]["static_dir"].as_str(), Some("web-admin"));
        assert_eq!(doc["accounts"]["open_registration"].as_bool(), Some(true));
        assert_eq!(doc["accounts"]["name_prefix"].as_str(), Some("Exile"));
        assert_eq!(doc["database"]["path"].as_str(), Some("/d/server.sqlite"));
        assert_eq!(doc["server"]["restart_mode"].as_str(), Some("exit"));
        assert!(out.contains("# header comment") && out.contains("# trailing comment"));

        // The web API stays on loopback unless asked.
        let s2 = HostSettings {
            web_public: false,
            ..s.clone()
        };
        let out = render_config(EXAMPLE, &s2, Path::new("/g"), Path::new("/d")).unwrap();
        assert!(out.contains("127.0.0.1:30080"));
    }

    #[test]
    fn a_config_without_the_tables_still_gets_them() {
        let out = render_config(
            "",
            &HostSettings::default(),
            Path::new("/g"),
            Path::new("/d"),
        )
        .unwrap();
        let doc: toml_edit::DocumentMut = out.parse().unwrap();
        assert_eq!(doc["web"]["bind"].as_str(), Some("127.0.0.1:8080"));
        assert_eq!(
            doc["network"]["login"]["bind"].as_str(),
            Some("0.0.0.0:20481")
        );
    }

    #[test]
    fn ansi_codes_are_removed() {
        assert_eq!(
            strip_ansi("\u{1b}[2m2026\u{1b}[0m \u{1b}[32m INFO\u{1b}[0m hello\r"),
            "2026  INFO hello"
        );
        assert_eq!(strip_ansi("plain"), "plain");
    }

    #[test]
    fn public_hosts_are_checked() {
        for good in [
            "127.0.0.1",
            "play.example",
            "a-b.example.com",
            "2001:db8::1",
        ] {
            assert!(valid_public_host(good), "{good}");
        }
        for bad in [
            "",
            " ",
            "a b",
            "http://x",
            "x:1",
            "-x",
            "x..y",
            "x/y",
            "ü.example",
        ] {
            assert!(!valid_public_host(bad), "{bad}");
        }
    }

    #[test]
    fn sibling_checksums_are_found_by_either_name() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join(LIN);
        std::fs::write(&a, b"x").unwrap();
        assert!(sibling_checksum_file(&a).is_none());
        std::fs::write(tmp.path().join(LIN_SHA), b"h").unwrap();
        assert_eq!(sibling_checksum_file(&a).unwrap(), tmp.path().join(LIN_SHA));
        std::fs::write(tmp.path().join(format!("{LIN}.sha256")), b"h").unwrap();
        assert_eq!(
            sibling_checksum_file(&a).unwrap(),
            tmp.path().join(format!("{LIN}.sha256"))
        );
    }
}
