//! "Host a server": everything about getting a `poe-server` release onto the machine and
//! turning the wizard's answers into its `config/server.toml`. No processes here (see
//! [`crate::host_run`]) and no Tauri types, so it is all testable.
//!
//! The release is a package built by the CI of Poe2Server (`.github/scripts/package-server.sh`,
//! `docs/releasing.md` there): `poe-server-v<ver>-game-<game version>-<linux|windows>-x86_64`
//! as `.tar.gz` / `.zip` with a `.sha256` next to it. It mirrors the repository layout, so the
//! server runs from its own folder. No game files are inside: `game_dir` points at the
//! launcher's installed game.
//!
//! Releases are looked up in [`REPO`] first and, when nothing there fits, in [`FALLBACK_REPO`]
//! (the two releases made before the server moved to its own repository). Packages come in two
//! layouts: the version file is `modules/protocol/versions/<game>.toml` (Poe2Server) or
//! `versions/<game>.toml` (the two old releases).

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
pub const REPO: &str = "Distorbed678/Poe2Server";
/// Asked when [`REPO`] has nothing for this game version and platform: it still holds the
/// releases `server-v0.1.0` and `server-v0.2.0` made before the server had its own repository.
pub const FALLBACK_REPO: &str = "Distorbed678/Poe2ModdingProject";
pub const TAG_PREFIX: &str = "server-v";
pub const API_BASE: &str = "https://api.github.com";
/// Shown to people who want to forward ports.
pub const HOSTING_GUIDE_URL: &str =
    "https://github.com/Distorbed678/Poe2Server/blob/main/docs/hosting.md";
/// Plain-text "what is my public IP" service, only asked when the user clicks the button.
pub const PUBLIC_IP_URL: &str = "https://api.ipify.org";

const MAX_ENTRIES: usize = 20_000;
const MAX_UNPACKED_BYTES: u64 = 2 << 30;

// ---------------------------------------------------------------------------------------------
// Settings (one `servers/<id>.json` per hosted server in the launcher's config directory; no secrets)

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
        Ok(())
    }

    /// The three ports, named.
    pub fn ports(&self) -> [(&'static str, u16); 3] {
        [
            ("login", self.login_port),
            ("instance", self.instance_port),
            ("web", self.web_port),
        ]
    }

    /// The ports this server shares with `other` (another hosted server).
    pub fn port_clashes(&self, other: &HostSettings) -> Vec<u16> {
        let theirs: Vec<u16> = other.ports().iter().map(|p| p.1).collect();
        self.ports()
            .iter()
            .map(|p| p.1)
            .filter(|p| theirs.contains(p))
            .collect()
    }
}

/// Ports for a new server: the defaults moved by [`PORT_STEP`] until none of them is taken by
/// another hosted server or `in_use` (a probe of the system, e.g. [`busy_ports`]).
pub fn next_free_ports(
    others: &[HostSettings],
    in_use: &dyn Fn(&HostSettings) -> bool,
) -> Result<(u16, u16, u16)> {
    let base = HostSettings::default();
    for k in 0..200u16 {
        let mut s = base.clone();
        s.login_port = base.login_port + k * PORT_STEP;
        s.instance_port = base.instance_port + k * PORT_STEP;
        s.web_port = base.web_port + k * PORT_STEP;
        if others.iter().all(|o| s.port_clashes(o).is_empty()) && !in_use(&s) {
            return Ok((s.login_port, s.instance_port, s.web_port));
        }
    }
    bail!("no free set of ports found; set them by hand in the server's Network settings")
}

/// How far the ports of each further hosted server move (20481/6112/8080, 20491/6122/8090, ...).
pub const PORT_STEP: u16 = 10;

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

/// Where the launcher keeps a hosted server (all below its data directory, never a Steam
/// library). Each server has an id (`s1`, `s2`, ...): its settings are
/// `<config>/servers/<id>.json` (+ `<id>.backup.json`), its files `<data>/servers/<id>/`
/// (database, config, backups, log, mods). The downloaded packages are shared by all servers
/// (`<data>/server/packages`): the config lives in the server's folder, not in the package.
#[derive(Debug, Clone)]
pub struct HostDirs {
    id: String,
    config_file: PathBuf,
    backup_file: PathBuf,
    root: PathBuf,
    shared: PathBuf,
}

/// The folder of the hosted servers' settings files.
pub fn servers_config_dir(dirs: &Dirs) -> PathBuf {
    dirs.config.join("servers")
}

/// A server id: `s` and digits.
pub fn valid_server_id(id: &str) -> bool {
    id.len() >= 2
        && id.len() <= 8
        && id.starts_with('s')
        && id[1..].chars().all(|c| c.is_ascii_digit())
}

/// The ids of the hosted servers, in number order.
pub fn server_ids(dirs: &Dirs) -> Vec<String> {
    let mut ids: Vec<String> = std::fs::read_dir(servers_config_dir(dirs))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let id = name.strip_suffix(".json")?;
            valid_server_id(id).then(|| id.to_owned())
        })
        .collect();
    ids.sort_by_key(|id| id[1..].parse::<u32>().unwrap_or(u32::MAX));
    ids
}

/// The id for a new server: the smallest free `s<n>`.
pub fn new_server_id(existing: &[String]) -> String {
    (1..)
        .map(|n| format!("s{n}"))
        .find(|id| !existing.contains(id))
        .expect("an unused id")
}

impl HostDirs {
    pub fn new(dirs: &Dirs, id: &str) -> Self {
        let cfg = servers_config_dir(dirs);
        Self {
            id: id.to_owned(),
            config_file: cfg.join(format!("{id}.json")),
            backup_file: cfg.join(format!("{id}.backup.json")),
            root: dirs.data.join("servers").join(id),
            shared: dirs.data.join("server"),
        }
    }
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn settings_file(&self) -> &Path {
        &self.config_file
    }
    /// The server's own folder.
    pub fn root(&self) -> &Path {
        &self.root
    }
    /// Extracted packages, one folder per release, shared by every hosted server.
    pub fn packages(&self) -> PathBuf {
        self.shared.join("packages")
    }
    pub fn downloads(&self) -> PathBuf {
        self.shared.join("downloads")
    }
    /// The database and anything else that must survive an update of the package.
    pub fn data(&self) -> PathBuf {
        self.root.join("data")
    }
    pub fn database(&self) -> PathBuf {
        self.data().join("server.sqlite")
    }
    /// The server's config (`POE_CONFIG`), written from the package's example and the settings.
    pub fn config(&self) -> PathBuf {
        self.root.join("server.toml")
    }
    /// The server's mods (`[mods] dir`).
    pub fn mods(&self) -> PathBuf {
        self.root.join("mods")
    }
    /// Where the web admin's config editor keeps the previous files (`[web] config_backup_dir`).
    pub fn config_backups(&self) -> PathBuf {
        self.root.join("config-backups")
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

/// Moves the single hosted server of launchers before 0.6.0 (`<config>/host.json`,
/// `<data>/server/{data,backups,server.log}`, the config inside its package) to the server id
/// `s1`. Does nothing when there is no old server or `s1` exists. Returns whether it moved one.
pub fn migrate_single_server(dirs: &Dirs) -> Result<bool> {
    let old_settings = dirs.config.join("host.json");
    if !old_settings.is_file() {
        return Ok(false);
    }
    let new = HostDirs::new(dirs, "s1");
    if new.settings_file().exists() {
        return Ok(false);
    }
    std::fs::create_dir_all(servers_config_dir(dirs))?;
    std::fs::create_dir_all(new.root())?;
    let old_root = dirs.data.join("server");
    for (from, to) in [
        (old_root.join("data"), new.data()),
        (old_root.join("backups"), new.backups()),
        (old_root.join("server.log"), new.log_file()),
        (
            dirs.config.join("backup.json"),
            new.backup_settings_file().to_path_buf(),
        ),
    ] {
        if from.exists() {
            std::fs::rename(&from, &to)
                .with_context(|| format!("moving {} to {}", from.display(), to.display()))?;
        }
    }
    let settings = HostSettings::load(&old_settings)?;
    // The config the user may have edited in the web admin lives in the package: keep it next to
    // the server so the next config write carries the edits over.
    if let Some(pkg) = &settings.package_dir {
        let old_cfg = pkg.join("config").join("server.toml");
        if old_cfg.is_file() && !new.config().exists() {
            std::fs::copy(&old_cfg, new.config())?;
        }
    }
    settings.save(new.settings_file())?;
    std::fs::remove_file(&old_settings)?;
    Ok(true)
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

/// The folder name a package archive unpacks to: the asset name without its extension.
pub fn package_name_of_asset(asset: &str) -> Option<&str> {
    parse_asset_name(asset)?;
    asset
        .strip_suffix(".tar.gz")
        .or_else(|| asset.strip_suffix(".zip"))
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
    pub body: Option<String>,
    #[serde(default)]
    pub html_url: Option<String>,
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
    /// The repository the release is in ([`REPO`] or [`FALLBACK_REPO`]).
    pub repo: String,
    pub tag: String,
    pub version: String,
    pub game_version: String,
    pub prerelease: bool,
    pub asset_name: String,
    pub asset_url: String,
    pub size: u64,
    /// The `.sha256` file next to the archive.
    pub sha256_url: String,
    /// The release notes (a markdown subset, cleaned and cut; shown as text by the UI).
    pub notes: String,
    /// The release's page on GitHub (https only).
    pub html_url: Option<String>,
}

const MAX_NOTES: usize = 4000;

/// The newest stable release (a pre-release only when there is no stable one) whose package
/// is for `game_version` on `os`, with its checksum file. `repo` of the result is left empty:
/// the caller knows which repository the list came from ([`find_release`]).
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
                repo: String::new(),
                tag: r.tag_name.clone(),
                version: n.version.clone(),
                game_version: n.game_version,
                prerelease: r.prerelease,
                asset_name: a.name.clone(),
                asset_url: a.browser_download_url.clone(),
                size: a.size,
                sha256_url: sha.browser_download_url.clone(),
                notes: crate::news::truncate(
                    crate::news::clean_text(r.body.as_deref().unwrap_or("")).trim(),
                    MAX_NOTES,
                ),
                html_url: r.html_url.clone().filter(|u| crate::news::is_https_link(u)),
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

/// Lists the GitHub releases of `repo`. `api_base` is a parameter for tests.
pub async fn fetch_releases(api_base: &str, repo: &str) -> Result<Vec<GhRelease>> {
    let url = format!(
        "{}/repos/{repo}/releases?per_page=30",
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
            "no server releases can be read from github.com/{repo}: the repository is still \
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

/// The newest server release for `game_version` on `os`: from [`REPO`], and only when that has
/// none that fits (or cannot be read) from [`FALLBACK_REPO`]. The error is the first
/// repository's, because that is the one a user should look at.
pub async fn find_release(api_base: &str, game_version: &str, os: &str) -> Result<ServerRelease> {
    platform_for(os)?;
    let first = async {
        let list = fetch_releases(api_base, REPO).await?;
        pick_release(&list, game_version, os)
    }
    .await;
    let err = match first {
        Ok(mut r) => {
            r.repo = REPO.into();
            return Ok(r);
        }
        Err(e) => e,
    };
    let second = async {
        let list = fetch_releases(api_base, FALLBACK_REPO).await?;
        pick_release(&list, game_version, os)
    }
    .await;
    match second {
        Ok(mut r) => {
            r.repo = FALLBACK_REPO.into();
            Ok(r)
        }
        Err(_) => Err(err),
    }
}

/// Whether `candidate` is a newer server version than `installed` (semver; two versions that
/// are not semver differ = newer, which is the safe answer for "offer the update").
pub fn is_newer(installed: &str, candidate: &str) -> bool {
    match (
        semver::Version::parse(installed),
        semver::Version::parse(candidate),
    ) {
        (Ok(a), Ok(b)) => b > a,
        _ => installed != candidate,
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

/// Where a package keeps the version file of `game_version`: the current layout first.
pub fn version_file_candidates(game_version: &str) -> [String; 2] {
    [
        format!("modules/protocol/versions/{game_version}.toml"),
        format!("versions/{game_version}.toml"),
    ]
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
        "third_party/dat-schema/schema-poe2.min.json".to_owned(),
    ] {
        ensure!(
            dir.join(&needed).is_file(),
            "the package is incomplete: {needed} is missing"
        );
    }
    // The version file is in the protocol module (Poe2Server packages) or at the top (the two
    // packages made in Poe2ModdingProject).
    ensure!(
        version_file_candidates(game_version)
            .iter()
            .any(|f| dir.join(f).is_file()),
        "the package is incomplete: modules/protocol/versions/{game_version}.toml is missing"
    );
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
    install_package_keeping(source, packages_dir, game_version, os, None, &[])
}

/// Like [`install_package`], but refuses (before anything is replaced) to overwrite the folder
/// `keep`: an update keeps the old package until the new one works. A folder in `in_use`
/// (another hosted server runs from it) is never replaced either: the same release is already
/// there, so it is used as it is.
pub fn install_package_keeping(
    source: &Path,
    packages_dir: &Path,
    game_version: &str,
    os: &str,
    keep: Option<&Path>,
    in_use: &[PathBuf],
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
        ensure!(
            keep != Some(target.as_path()),
            "the new package would be installed in the folder of the current one ({}); \
             nothing was changed",
            target.display()
        );
        if target.exists() && in_use.iter().any(|p| p == &target) {
            let existing = inspect_package(&target, game_version, os)?;
            return Ok(PackageInfo {
                dir: target,
                ..existing
            });
        }
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

/// Removes every package folder that no hosted server uses (`keep`).
pub fn prune_packages(packages_dir: &Path, keep: &[PathBuf]) {
    if let Ok(rd) = std::fs::read_dir(packages_dir) {
        for e in rd.flatten() {
            if !keep.contains(&e.path()) && e.path().is_dir() {
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The server config

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
    dirs: &HostDirs,
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
    set_str(
        &mut doc,
        &["database"],
        "path",
        &dirs.database().to_string_lossy(),
    );
    // Everything the server writes goes to its own folder: servers share the package folder.
    set_str(&mut doc, &["mods"], "dir", &dirs.mods().to_string_lossy());
    set_str(
        &mut doc,
        &["web"],
        "config_backup_dir",
        &dirs.config_backups().to_string_lossy(),
    );
    // An admin's "restart" ends the process with code 75 and the launcher starts it again (on
    // every platform: host_run.rs). Older servers ignore the key (an unknown key is only warned
    // about by newer ones).
    set_str(&mut doc, &["server"], "restart_mode", "exit");
    Ok(doc.to_string())
}

/// A value without the comments and spacing around it, to compare two of them.
fn bare(v: &toml_edit::Value) -> String {
    let mut v = v.clone();
    *v.decor_mut() = toml_edit::Decor::default();
    v.to_string()
}

/// Every setting of `table` as (path, text): plain values and arrays of tables as a whole.
fn flatten(table: &Table, path: &mut Vec<String>, out: &mut Vec<(Vec<String>, String)>) {
    for (key, item) in table.iter() {
        path.push(key.to_owned());
        match item {
            Item::Table(t) => flatten(t, path, out),
            Item::Value(v) => out.push((path.clone(), bare(v))),
            Item::ArrayOfTables(a) => out.push((
                path.clone(),
                a.iter()
                    .map(|t| t.to_string())
                    .collect::<Vec<_>>()
                    .join("\n--\n"),
            )),
            Item::None => {}
        }
        path.pop();
    }
}

/// The new package's example config with what the user changed in the old package's config:
/// every setting of `old_config` whose value differs from `old_example` (or that the old
/// example does not have at all) is copied over. Settings the user never touched take the new
/// package's defaults, so keys that are new in this release appear with their defaults and
/// changed defaults apply. The result is for [`render_config`], which then puts the wizard's
/// answers on top. A broken `old_config` is an error; the caller then uses the plain example.
pub fn carry_over_config(old_example: &str, old_config: &str, new_example: &str) -> Result<String> {
    let old_example: DocumentMut = old_example
        .parse()
        .context("the old package's server.example.toml is not valid TOML")?;
    let old_config: DocumentMut = old_config
        .parse()
        .context("the old config/server.toml is not valid TOML")?;
    let mut doc: DocumentMut = new_example
        .parse()
        .context("the new package's server.example.toml is not valid TOML")?;
    let (mut base, mut mine) = (Vec::new(), Vec::new());
    flatten(old_example.as_table(), &mut Vec::new(), &mut base);
    flatten(old_config.as_table(), &mut Vec::new(), &mut mine);
    for (path, text) in mine {
        if base.iter().any(|(p, t)| *p == path && *t == text) {
            continue; // as shipped: take the new default
        }
        let Some((key, parents)) = path.split_last() else {
            continue;
        };
        let parents: Vec<&str> = parents.iter().map(String::as_str).collect();
        // Look the setting up in the old config again for its real item.
        let mut cur = old_config.as_item();
        for k in &path {
            match cur.get(k.as_str()) {
                Some(next) => cur = next,
                None => break,
            }
        }
        match cur {
            Item::Value(v) => set_value(&mut doc, &parents, key, v.clone()),
            Item::ArrayOfTables(a) => {
                table_at(&mut doc, &parents)[key.as_str()] = Item::ArrayOfTables(a.clone());
            }
            _ => {}
        }
    }
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

    /// Dirs of server s1 under a made-up root (nothing is created).
    fn test_dirs() -> HostDirs {
        HostDirs::new(&Dirs::under(Path::new("/r")), "s1")
    }

    #[test]
    fn server_ids_and_folders() {
        assert!(valid_server_id("s1") && valid_server_id("s12"));
        assert!(!valid_server_id("s") && !valid_server_id("x1") && !valid_server_id("s1/.."));
        assert_eq!(new_server_id(&[]), "s1");
        assert_eq!(new_server_id(&["s1".into(), "s3".into()]), "s2");
        let d = test_dirs();
        assert_eq!(d.settings_file(), Path::new("/r/config/servers/s1.json"));
        assert_eq!(d.config(), Path::new("/r/data/servers/s1/server.toml"));
        assert_eq!(d.packages(), Path::new("/r/data/server/packages"), "shared");
        let tmp = tempfile::tempdir().unwrap();
        let dirs = Dirs::under(tmp.path());
        std::fs::create_dir_all(servers_config_dir(&dirs)).unwrap();
        for f in [
            "s2.json",
            "s10.json",
            "s1.json",
            "s1.backup.json",
            "notes.txt",
        ] {
            std::fs::write(servers_config_dir(&dirs).join(f), "{}").unwrap();
        }
        assert_eq!(server_ids(&dirs), ["s1", "s2", "s10"]);
    }

    #[test]
    fn new_servers_get_the_next_free_ports() {
        let first = HostSettings::default();
        assert_eq!(
            next_free_ports(&[], &|_| false).unwrap(),
            (20481, 6112, 8080)
        );
        assert_eq!(
            next_free_ports(std::slice::from_ref(&first), &|_| false).unwrap(),
            (20491, 6122, 8090)
        );
        // A port taken by another program moves the new server on too.
        let busy = |s: &HostSettings| s.web_port == 8090;
        assert_eq!(
            next_free_ports(std::slice::from_ref(&first), &busy).unwrap(),
            (20501, 6132, 8100)
        );
        let mut other = first.clone();
        (other.login_port, other.instance_port, other.web_port) = (30000, 30001, 20491);
        assert_eq!(first.port_clashes(&other), Vec::<u16>::new());
        assert_eq!(
            next_free_ports(&[first, other], &|_| false).unwrap(),
            (20501, 6132, 8100)
        );
    }

    #[test]
    fn the_single_server_of_older_launchers_becomes_s1() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = Dirs::under(tmp.path());
        assert!(!migrate_single_server(&dirs).unwrap(), "nothing to move");
        let pkg = dirs.data.join("server/packages/p");
        std::fs::create_dir_all(pkg.join("config")).unwrap();
        std::fs::write(pkg.join("config/server.toml"), "edited = true\n").unwrap();
        std::fs::create_dir_all(dirs.data.join("server/data")).unwrap();
        std::fs::write(dirs.data.join("server/data/server.sqlite"), "db").unwrap();
        std::fs::write(dirs.data.join("server/server.log"), "log").unwrap();
        std::fs::create_dir_all(&dirs.config).unwrap();
        std::fs::write(dirs.config.join("backup.json"), r#"{"auto":true,"keep":3}"#).unwrap();
        let old = HostSettings {
            server_name: "Old".into(),
            package_dir: Some(pkg.clone()),
            configured: true,
            ..Default::default()
        };
        old.save(&dirs.config.join("host.json")).unwrap();
        assert!(migrate_single_server(&dirs).unwrap());
        let d = HostDirs::new(&dirs, "s1");
        assert_eq!(HostSettings::load(d.settings_file()).unwrap(), old);
        assert_eq!(std::fs::read_to_string(d.database()).unwrap(), "db");
        assert_eq!(std::fs::read_to_string(d.log_file()).unwrap(), "log");
        assert_eq!(
            std::fs::read_to_string(d.config()).unwrap(),
            "edited = true\n"
        );
        assert!(d.backup_settings_file().is_file());
        assert!(!dirs.config.join("host.json").exists());
        assert!(!migrate_single_server(&dirs).unwrap(), "done once");
    }

    #[test]
    fn package_folder_names_come_from_the_asset() {
        assert_eq!(
            package_name_of_asset("poe-server-v0.4.0-game-0.5.5e.hotfix1-linux-x86_64.tar.gz"),
            Some("poe-server-v0.4.0-game-0.5.5e.hotfix1-linux-x86_64")
        );
        assert_eq!(package_name_of_asset("other.zip"), None);
    }

    fn rel(tag: &str, pre: bool, names: &[&str]) -> GhRelease {
        GhRelease {
            tag_name: tag.into(),
            body: None,
            html_url: None,
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
        let n =
            parse_asset_name("poe-server-v0.1.0-game-0.5.5e.hotfix1-linux-x86_64.tar.gz").unwrap();
        assert_eq!(
            n,
            AssetName {
                version: "0.1.0".into(),
                game_version: "0.5.5e.hotfix1".into(),
                platform: "linux".into(),
                ext: "tar.gz".into()
            }
        );
        let w = parse_asset_name("poe-server-v0.2.0-rc1-game-0.5.5e.hotfix1-windows-x86_64.zip")
            .unwrap();
        assert_eq!(
            (w.version.as_str(), w.platform.as_str(), w.ext.as_str()),
            ("0.2.0-rc1", "windows", "zip")
        );
        for bad in [
            "poe-server-v0.1.0-game-0.5.5e.hotfix1-macos-x86_64.tar.gz",
            "poe-server-v0.1.0-game-0.5.5e.hotfix1-linux-x86_64.sha256",
            "poe-server-v0.1.0-linux-x86_64.zip",
            "other.tar.gz",
        ] {
            assert!(parse_asset_name(bad).is_none(), "{bad}");
        }
    }

    const LIN: &str = "poe-server-v0.1.0-game-0.5.5e.hotfix1-linux-x86_64.tar.gz";
    const LIN_SHA: &str = "poe-server-v0.1.0-game-0.5.5e.hotfix1-linux-x86_64.sha256";
    const LIN2: &str = "poe-server-v0.2.0-game-0.5.5e.hotfix1-linux-x86_64.tar.gz";
    const LIN2_SHA: &str = "poe-server-v0.2.0-game-0.5.5e.hotfix1-linux-x86_64.sha256";

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
                    "poe-server-v0.2.0-game-0.5.5e.hotfix1-windows-x86_64.zip",
                ],
            ),
            rel(
                "server-v0.3.0-rc1",
                true,
                &[
                    "poe-server-v0.3.0-rc1-game-0.5.5e.hotfix1-linux-x86_64.tar.gz",
                    "poe-server-v0.3.0-rc1-game-0.5.5e.hotfix1-linux-x86_64.sha256",
                ],
            ),
            rel("v9.9.9", false, &["unrelated"]),
        ];
        let r = pick_release(&all, "0.5.5e.hotfix1", "linux").unwrap();
        assert_eq!((r.version.as_str(), r.asset_name.as_str()), ("0.2.0", LIN2));
        assert!(r.sha256_url.ends_with(LIN2_SHA));
        // A pre-release is taken only when nothing stable fits.
        let only_pre = vec![all[2].clone()];
        assert!(
            pick_release(&only_pre, "0.5.5e.hotfix1", "linux")
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
        assert!(e.contains("0.6.0") && e.contains("0.5.5e.hotfix1"), "{e}");
        let e = pick_release(&all, "0.5.5e.hotfix1", "windows")
            .unwrap_err()
            .to_string();
        assert!(e.contains("windows"), "{e}");
        // No checksum file next to the archive: not usable.
        let nosha = vec![rel("server-v0.1.0", false, &[LIN])];
        assert!(pick_release(&nosha, "0.5.5e.hotfix1", "linux").is_err());
        assert!(pick_release(&[], "0.5.5e.hotfix1", "linux").is_err());
        assert!(pick_release(&all, "0.5.5e.hotfix1", "macos").is_err());
        // Drafts are ignored.
        let mut draft = rel("server-v0.1.0", false, &[LIN, LIN_SHA]);
        draft.draft = true;
        assert!(pick_release(&[draft], "0.5.5e.hotfix1", "linux").is_err());
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
        let mut s = HostSettings::default();
        for h in ["192.168.1.5", "play.example.com", "::1", "my-server"] {
            s.public_host = h.into();
            assert!(s.validate().is_ok(), "{h}");
        }
    }

    const EXAMPLE: &str = r#"# header comment
[game]
version = "0.5.5e.hotfix1"
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
        let dirs = test_dirs();
        let out = render_config(EXAMPLE, &s, Path::new("/games/poe2"), &dirs).unwrap();
        let doc: toml_edit::DocumentMut = out.parse().unwrap();
        assert_eq!(doc["game"]["game_dir"].as_str(), Some("/games/poe2"));
        assert_eq!(doc["game"]["version"].as_str(), Some("0.5.5e.hotfix1"));
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
        assert_eq!(
            doc["database"]["path"].as_str(),
            Some(dirs.database().to_str().unwrap())
        );
        // What the server writes goes to its own folder (servers share the package folder).
        assert_eq!(
            doc["mods"]["dir"].as_str(),
            Some(dirs.mods().to_str().unwrap())
        );
        assert_eq!(
            doc["web"]["config_backup_dir"].as_str(),
            Some(dirs.config_backups().to_str().unwrap())
        );
        assert_eq!(doc["server"]["restart_mode"].as_str(), Some("exit"));
        assert!(out.contains("# header comment") && out.contains("# trailing comment"));

        // The web API stays on loopback unless asked.
        let s2 = HostSettings {
            web_public: false,
            ..s.clone()
        };
        let out = render_config(EXAMPLE, &s2, Path::new("/g"), &dirs).unwrap();
        assert!(out.contains("127.0.0.1:30080"));
    }

    #[test]
    fn a_config_without_the_tables_still_gets_them() {
        let out =
            render_config("", &HostSettings::default(), Path::new("/g"), &test_dirs()).unwrap();
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

    #[test]
    fn newer_versions_are_compared_as_semver() {
        assert!(is_newer("0.2.0", "0.10.0"));
        assert!(is_newer("0.2.0-rc1", "0.2.0"));
        assert!(!is_newer("0.2.1", "0.2.1"));
        assert!(!is_newer("0.3.0", "0.2.9"));
        // Not semver: a difference is offered as an update.
        assert!(is_newer("dev-abc", "0.2.0"));
        assert!(!is_newer("dev-abc", "dev-abc"));
    }

    #[test]
    fn both_package_layouts_are_accepted() {
        let make = |version_file: Option<&str>| {
            let d = tempfile::tempdir().unwrap();
            let r = d.path();
            let put = |p: &str| {
                let f = r.join(p);
                std::fs::create_dir_all(f.parent().unwrap()).unwrap();
                std::fs::write(f, b"x").unwrap();
            };
            std::fs::write(
                r.join("release.json"),
                format!(
                    r#"{{"name":"n","server_version":"0.2.1","game_version":"0.5.5e.hotfix1","platform":"{}"}}"#,
                    platform_for(std::env::consts::OS).unwrap()
                ),
            )
            .unwrap();
            put(server_exe_name(std::env::consts::OS));
            put("config/server.example.toml");
            put("third_party/dat-schema/schema-poe2.min.json");
            put("modules/protocol/opcodes/0.5.5e.hotfix1/a.toml");
            if let Some(v) = version_file {
                put(v);
            }
            d
        };
        let check = |d: &tempfile::TempDir| {
            inspect_package(d.path(), "0.5.5e.hotfix1", std::env::consts::OS)
                .map(|p| p.server_version)
        };
        assert_eq!(
            check(&make(Some("modules/protocol/versions/0.5.5e.hotfix1.toml"))).unwrap(),
            "0.2.1"
        );
        assert_eq!(
            check(&make(Some("versions/0.5.5e.hotfix1.toml"))).unwrap(),
            "0.2.1"
        );
        let e = check(&make(None)).unwrap_err().to_string();
        assert!(
            e.contains("modules/protocol/versions/0.5.5e.hotfix1.toml"),
            "{e}"
        );
        // The version file of another game version does not count.
        assert!(check(&make(Some("versions/0.6.0.toml"))).is_err());
    }

    const OLD_EXAMPLE: &str = r#"# old
[game]
version = "0.5.5e.hotfix1"
game_dir = "GameFiles"

[network]
public_host = "127.0.0.1"

[web]
enabled = false
rate_limit = 60 # per minute

[[leagues]]
id = "a"

[[leagues]]
id = "b"
"#;

    const NEW_EXAMPLE: &str = r#"# new
[game]
version = "0.5.5e.hotfix1"
game_dir = "GameFiles"

[network]
public_host = "127.0.0.1"

[web]
enabled = false
rate_limit = 120 # per minute, raised in the new release
cors = "none" # new key

[mods]
require_client_mods = false # a new section

[[leagues]]
id = "a"
"#;

    #[test]
    fn a_changed_config_keeps_the_users_values_and_takes_new_defaults() {
        // The user changed one value, the wizard changed others, and left rate_limit alone.
        let old_cfg = OLD_EXAMPLE
            .replace("public_host = \"127.0.0.1\"", "public_host = \"10.0.0.5\"")
            .replace("enabled = false", "enabled = true")
            .replace("id = \"b\"", "id = \"c\"")
            + "\n[extra]\nnote = \"mine\"\n";
        let out = carry_over_config(OLD_EXAMPLE, &old_cfg, NEW_EXAMPLE).unwrap();
        let doc: DocumentMut = out.parse().unwrap();
        assert_eq!(doc["network"]["public_host"].as_str(), Some("10.0.0.5"));
        assert_eq!(doc["web"]["enabled"].as_bool(), Some(true));
        // Untouched by the user: the new release's default wins.
        assert_eq!(doc["web"]["rate_limit"].as_integer(), Some(120));
        assert_eq!(doc["web"]["cors"].as_str(), Some("none"));
        assert_eq!(doc["mods"]["require_client_mods"].as_bool(), Some(false));
        // The users's own section and the changed league list come along as a whole.
        assert_eq!(doc["extra"]["note"].as_str(), Some("mine"));
        let leagues = doc["leagues"].as_array_of_tables().unwrap();
        let ids: Vec<_> = leagues.iter().map(|t| t["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["a", "c"]);
        assert!(out.contains("# new") && out.contains("# per minute, raised"));

        // A config as shipped changes nothing: the new example comes out.
        let same = carry_over_config(OLD_EXAMPLE, OLD_EXAMPLE, NEW_EXAMPLE).unwrap();
        assert_eq!(same, NEW_EXAMPLE);
        // And the wizard's answers still go on top.
        let s = HostSettings {
            public_host: "203.0.113.9".into(),
            ..Default::default()
        };
        let rendered = render_config(&out, &s, Path::new("/g"), &test_dirs()).unwrap();
        let doc: DocumentMut = rendered.parse().unwrap();
        assert_eq!(doc["network"]["public_host"].as_str(), Some("203.0.113.9"));
        assert_eq!(doc["web"]["rate_limit"].as_integer(), Some(120));
        // A broken old config is an error for the caller to handle.
        assert!(carry_over_config(OLD_EXAMPLE, "[broken", NEW_EXAMPLE).is_err());
    }
}
