//! The mod index: an `index.json` in the GitHub repository `Distorbed678/Poe2Mods` (anyone adds
//! a mod by pull request; the template with the schema, a validation script and the CI is in
//! `docs/mod-index-template/` of Poe2ModdingProject). The launcher's mod browser reads it, checks
//! every entry, caches it on disk and installs a version into the mod cache: the package is
//! downloaded from the entry's `url` (https) and only kept when its SHA-256 equals the index's
//! and `verify_package` accepts it.
//!
//! The file is not trusted. A broken mod entry is skipped and reported, not the whole index; a
//! version with an unusable hash or address is skipped; nothing is fetched from anywhere but the
//! address in an entry's version. Offline (or rate limited) the saved copy is shown quietly.

use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    core::{Core, now_secs},
    modcache::Added,
    mods::{Compat, Side, compat_for_listing, valid_id},
    modsync::{AdvertisedMod, MAX_PACKAGE_BYTES, ModSource},
    news::{load_cache_as, save_cache_as, truncate},
    profiles::is_loopback_host,
};

pub const INDEX_URL: &str =
    "https://raw.githubusercontent.com/Distorbed678/Poe2Mods/main/index.json";
pub const CACHE_TTL: i64 = 10 * 60;
pub const FORCE_GAP: i64 = 20;
/// The index file schema this launcher reads.
pub const SCHEMA_VERSION: u64 = 1;
const MAX_FILE: usize = 4 << 20;
const MAX_MODS: usize = 5000;
const MAX_VERSIONS: usize = 200;
const MAX_TAGS: usize = 12;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexVersion {
    pub version: String,
    /// `YYYY-MM-DD`, free text for display.
    #[serde(default)]
    pub released: String,
    /// Client builds it was made for; empty = not stated.
    pub game_versions: Vec<String>,
    /// The client mod ABI it needs (a DLL mod).
    pub hook_api: Option<u32>,
    /// The server API it needs (a mod with a server library).
    pub server_api: Option<u32>,
    pub sha256: String,
    pub size: u64,
    /// `https://` address of `<id>-<version>.zip`.
    pub url: String,
    /// Withdrawn: shown struck out, never installed by default.
    #[serde(default)]
    pub yanked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexMod {
    pub id: String,
    pub name: String,
    pub description: String,
    pub author: String,
    /// `client`, `server` or `both`.
    pub side: String,
    pub homepage: String,
    pub licence: String,
    pub tags: Vec<String>,
    /// Newest first.
    pub versions: Vec<IndexVersion>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IndexCache {
    pub attempted_at: i64,
    pub fetched_at: i64,
    pub updated: String,
    pub mods: Vec<IndexMod>,
    pub skipped: Vec<String>,
}

/// What the Browse tab gets.
#[derive(Debug, Clone, Serialize)]
pub struct ModIndexView {
    pub mods: Vec<IndexModView>,
    pub skipped: Vec<String>,
    pub updated: String,
    pub fetched_at: i64,
    /// The saved copy is shown because the index could not be reached.
    pub offline: bool,
    pub error: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct IndexVersionView {
    #[serde(flatten)]
    pub v: IndexVersion,
    /// Against the selected game version and the installed hook.
    pub compat: Compat,
    /// Already in the mod cache with this hash.
    pub cached: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct IndexModView {
    #[serde(flatten)]
    pub m: IndexMod,
    pub versions: Vec<IndexVersionView>,
}

fn no_controls(s: &str) -> bool {
    !s.chars().any(char::is_control)
}

fn text(s: &str, max: usize) -> String {
    let s: String = s
        .replace("\r\n", "\n")
        .chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .collect();
    truncate(s.trim(), max)
}

fn valid_tag(s: &str) -> bool {
    (1..=24).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn is_hex64(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// `https://` (or http on this computer when `allow_local`), no credentials.
fn good_url(url: &str, allow_local: bool) -> bool {
    let Ok(u) = reqwest::Url::parse(url) else {
        return false;
    };
    let local = u.host_str().is_some_and(is_loopback_host);
    (u.scheme() == "https" || (allow_local && u.scheme() == "http" && local))
        && u.username().is_empty()
        && u.password().is_none()
        && no_controls(url)
        && url.len() <= 500
}

fn check_version(id: &str, v: &Value, allow_local: bool) -> Result<IndexVersion, String> {
    let o = v.as_object().ok_or("not an object")?;
    let s = |k: &str| o.get(k).and_then(Value::as_str);
    let version = s("version").ok_or("no version")?;
    if Version::parse(version).is_err() || version.contains(['/', '\\']) {
        return Err(format!("{version:?} is not a semantic version"));
    }
    let sha = s("sha256").ok_or("no sha256")?;
    if !is_hex64(sha) {
        return Err("sha256 is not 64 hex digits".into());
    }
    let size = o
        .get("size")
        .and_then(Value::as_u64)
        .filter(|n| (1..=MAX_PACKAGE_BYTES).contains(n))
        .ok_or("size must be between 1 and 512 MiB")?;
    let url = s("url").ok_or("no url")?;
    if !good_url(url, allow_local) {
        return Err("url must be an https:// address".into());
    }
    let num = |k: &str| -> Result<Option<u32>, String> {
        match o.get(k) {
            None | Some(Value::Null) => Ok(None),
            Some(x) => x
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .map(Some)
                .ok_or(format!("{k} must be a number")),
        }
    };
    let _ = id;
    Ok(IndexVersion {
        version: version.to_owned(),
        released: s("released").map(|r| text(r, 24)).unwrap_or_default(),
        game_versions: o
            .get("game_versions")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .take(16)
                    .map(|g| text(g, 40))
                    .collect()
            })
            .unwrap_or_default(),
        hook_api: num("hook_api")?,
        server_api: num("server_api")?,
        sha256: sha.to_ascii_lowercase(),
        size,
        url: url.to_owned(),
        yanked: o.get("yanked").and_then(Value::as_bool).unwrap_or(false),
    })
}

fn check_mod(v: &Value, allow_local: bool) -> Result<IndexMod, String> {
    let o = v.as_object().ok_or("not an object")?;
    let s = |k: &str| o.get(k).and_then(Value::as_str);
    let id = s("id").ok_or("no id")?;
    if !valid_id(id) {
        return Err(format!("{id:?} is not a valid mod id"));
    }
    let name = s("name")
        .map(|n| text(n, 80))
        .filter(|n| !n.is_empty())
        .ok_or_else(|| format!("{id}: no name"))?;
    let side = s("side").ok_or_else(|| format!("{id}: no side"))?;
    if Side::parse(side).is_none() {
        return Err(format!("{id}: side must be client, server or both"));
    }
    let homepage = s("homepage").unwrap_or("");
    if !homepage.is_empty() && !good_url(homepage, false) {
        return Err(format!("{id}: homepage must be an https:// address"));
    }
    let tags: Vec<String> = o
        .get("tags")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .filter(|t| valid_tag(t))
                .take(MAX_TAGS)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let list = o
        .get("versions")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{id}: no versions"))?;
    let mut versions = Vec::new();
    let mut problems = Vec::new();
    for (i, item) in list.iter().enumerate().take(MAX_VERSIONS) {
        match check_version(id, item, allow_local) {
            Ok(v) => {
                if versions
                    .iter()
                    .any(|x: &IndexVersion| x.version == v.version)
                {
                    problems.push(format!("version {} is listed twice", v.version));
                } else {
                    versions.push(v);
                }
            }
            Err(why) => problems.push(format!("versions[{i}]: {why}")),
        }
    }
    if versions.is_empty() {
        return Err(format!(
            "{id}: no usable version{}",
            problems
                .first()
                .map(|p| format!(" ({p})"))
                .unwrap_or_default()
        ));
    }
    versions.sort_by(|a, b| {
        Version::parse(&b.version)
            .unwrap()
            .cmp(&Version::parse(&a.version).unwrap())
    });
    Ok(IndexMod {
        id: id.to_owned(),
        name,
        description: s("description").map(|d| text(d, 600)).unwrap_or_default(),
        author: s("author").map(|a| text(a, 80)).unwrap_or_default(),
        side: side.to_owned(),
        homepage: homepage.to_owned(),
        licence: o
            .get("licence")
            .or_else(|| o.get("license"))
            .and_then(Value::as_str)
            .map(|l| text(l, 60))
            .unwrap_or_default(),
        tags,
        versions,
    })
}

/// Parses `index.json`; returns the valid mods and the reasons others were skipped.
pub fn parse_index(text_in: &str) -> Result<(String, Vec<IndexMod>, Vec<String>)> {
    parse_index_with(text_in, false)
}

pub fn parse_index_with(
    text_in: &str,
    allow_local: bool,
) -> Result<(String, Vec<IndexMod>, Vec<String>)> {
    let v: Value =
        serde_json::from_str(text_in).map_err(|e| anyhow!("index.json is not valid JSON: {e}"))?;
    let o = v
        .as_object()
        .ok_or_else(|| anyhow!("index.json must be an object"))?;
    match o.get("schema").and_then(Value::as_u64) {
        Some(n) if n == SCHEMA_VERSION => {}
        Some(n) => bail!(
            "index.json has schema {n}; this launcher reads {SCHEMA_VERSION} (update the launcher)"
        ),
        None => bail!("index.json has no schema"),
    }
    let list = o
        .get("mods")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("index.json has no mods list"))?;
    let mut out: Vec<IndexMod> = Vec::new();
    let mut skipped = Vec::new();
    for (i, item) in list.iter().enumerate().take(MAX_MODS) {
        match check_mod(item, allow_local) {
            Ok(m) => {
                if out.iter().any(|x| x.id == m.id) {
                    skipped.push(format!("entry {i}: {} is listed twice", m.id));
                } else {
                    out.push(m);
                }
            }
            Err(why) => skipped.push(truncate(&format!("entry {i}: {why}"), 200)),
        }
    }
    out.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then(a.id.cmp(&b.id))
    });
    let updated = o
        .get("updated")
        .and_then(Value::as_str)
        .map(|u| text(u, 40))
        .unwrap_or_default();
    Ok((updated, out, skipped))
}

async fn fetch_index(url: &str, allow_local: bool) -> Result<(String, Vec<IndexMod>, Vec<String>)> {
    let c = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(6))
        .timeout(Duration::from_secs(15))
        .user_agent(concat!("poe2-launcher/", env!("CARGO_PKG_VERSION")))
        .build()?;
    let res = c
        .get(url)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| anyhow!("cannot reach the mod index: {}", e.without_url()))?;
    let status = res.status();
    match status.as_u16() {
        200..=299 => {}
        404 => bail!("the mod index does not exist yet (HTTP 404)"),
        403 | 429 => bail!(
            "the mod index is rate limited (HTTP {}); try again later",
            status.as_u16()
        ),
        code => bail!("the mod index answered HTTP {code}"),
    }
    let bytes = res.bytes().await?;
    if bytes.len() > MAX_FILE {
        bail!("index.json is larger than {} KB", MAX_FILE / 1024);
    }
    parse_index_with(&String::from_utf8_lossy(&bytes), allow_local)
}

impl Core {
    fn mod_index_cache_file(&self) -> std::path::PathBuf {
        self.dirs.data.join("cache").join("modindex.json")
    }

    fn index_view(&self, c: IndexCache, offline: bool, error: String) -> ModIndexView {
        let game = self.version().version;
        let abi = self.hook_abi();
        let cache = self.mod_cache();
        let mods = c
            .mods
            .into_iter()
            .map(|m| {
                let versions = m
                    .versions
                    .iter()
                    .map(|v| IndexVersionView {
                        compat: compat_for_listing(&v.game_versions, v.hook_api, &game, abi),
                        cached: cache
                            .get(&m.id, &v.version)
                            .is_some_and(|c| c.errors.is_empty() && c.meta.sha256 == v.sha256),
                        v: v.clone(),
                    })
                    .collect();
                IndexModView { versions, m }
            })
            .collect();
        ModIndexView {
            mods,
            skipped: c.skipped,
            updated: c.updated,
            fetched_at: c.fetched_at,
            offline,
            error,
        }
    }

    /// The mod index (cached; `force`: the player pressed refresh). Never an error: an
    /// unreachable index gives the saved copy and a note.
    pub async fn mod_index_get(&self, force: bool) -> ModIndexView {
        self.mod_index_get_from(INDEX_URL, force, false).await
    }

    /// `url` and `allow_local` are parameters for tests (a mock index names packages on 127.0.0.1).
    pub async fn mod_index_get_from(
        &self,
        url: &str,
        force: bool,
        allow_local: bool,
    ) -> ModIndexView {
        let file = self.mod_index_cache_file();
        let cache: IndexCache = load_cache_as(&file);
        let now = now_secs();
        let age = now - cache.attempted_at;
        if cache.attempted_at > 0
            && (if force {
                age < FORCE_GAP
            } else {
                age < CACHE_TTL
            })
        {
            let offline = cache.fetched_at < cache.attempted_at;
            return self.index_view(cache, offline, String::new());
        }
        match fetch_index(url, allow_local).await {
            Ok((updated, mods, skipped)) => {
                let c = IndexCache {
                    attempted_at: now,
                    fetched_at: now,
                    updated,
                    mods,
                    skipped,
                };
                save_cache_as(&file, &c);
                self.index_view(c, false, String::new())
            }
            Err(e) => {
                let c = IndexCache {
                    attempted_at: now,
                    ..cache
                };
                save_cache_as(&file, &c);
                let fetched = c.fetched_at;
                let mut v = self.index_view(c, true, format!("{e:#}"));
                v.fetched_at = fetched;
                v
            }
        }
    }

    /// Downloads `id` `version` from the (cached) index into the mod cache and, when `scope` is
    /// given (`all` or a profile id), enables it there.
    pub async fn mod_index_install(
        &self,
        id: &str,
        version: &str,
        scope: Option<&str>,
    ) -> Result<Added> {
        self.mod_index_install_from(id, version, scope, false).await
    }

    pub async fn mod_index_install_from(
        &self,
        id: &str,
        version: &str,
        scope: Option<&str>,
        allow_local: bool,
    ) -> Result<Added> {
        if self.version().is_official() {
            bail!("the hook's mod loader loads no mods in official (read-only) mode");
        }
        let idx: IndexCache = load_cache_as(&self.mod_index_cache_file());
        let m = idx
            .mods
            .iter()
            .find(|m| m.id == id)
            .ok_or_else(|| anyhow!("{id} is not in the mod index (refresh it first)"))?;
        if Side::parse(&m.side).is_none_or(|s| !s.includes_client()) {
            bail!("{}", crate::mods::server_only_message(id));
        }
        let v = m
            .versions
            .iter()
            .find(|v| v.version == version)
            .ok_or_else(|| anyhow!("{id} {version} is not in the mod index"))?;
        if !good_url(&v.url, allow_local) {
            bail!("the index gives no https address for {id} {version}");
        }
        let ad = AdvertisedMod {
            id: m.id.clone(),
            version: v.version.clone(),
            side: m.side.clone(),
            sha256: v.sha256.clone(),
            size: v.size,
            required: false,
            hook_api: v.hook_api,
            game_versions: v.game_versions.clone(),
            sources: vec![ModSource::Url { url: v.url.clone() }],
            name: m.name.clone(),
            authors: vec![m.author.clone()],
            description: m.description.clone(),
            code: v.hook_api.is_some(),
        };
        let urls = vec![v.url.clone()];
        let added = self.download_to_cache(&ad, &urls, "index").await?;
        if let Some(scope) = scope {
            self.mod_set_enabled(scope, &added.id, &added.version, true, false)
                .with_context(|| {
                    format!(
                        "{} {} was added to the cache but could not be enabled",
                        added.id, added.version
                    )
                })?;
        }
        Ok(added)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn ver(v: &str, extra: Value) -> Value {
        let mut o = json!({"version": v, "game_versions": ["0.5.5e.hotfix1"], "hook_api": 1, "sha256": "b".repeat(64), "size": 1234, "url": format!("https://github.com/x/y/releases/download/v{v}/m-{v}.zip")});
        o.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        o
    }

    fn entry(id: &str, versions: Vec<Value>, extra: Value) -> Value {
        let mut o = json!({"id": id, "name": id.to_uppercase(), "description": "d", "author": "someone", "side": "client", "tags": ["qol", "Bad Tag"], "homepage": "https://example.org/m", "licence": "MIT", "versions": versions});
        o.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        o
    }

    fn file(mods: Vec<Value>) -> String {
        json!({"schema": 1, "updated": "2026-10-22T00:00:00Z", "mods": mods}).to_string()
    }

    #[test]
    fn a_valid_index_is_read_sorted_and_tidied() {
        let (updated, mods, skipped) = parse_index(&file(vec![
            entry(
                "zz-mod",
                vec![
                    ver("1.0.0", json!({})),
                    ver("1.10.0", json!({})),
                    ver("1.2.0", json!({"yanked": true, "released": "2026-10-01"})),
                ],
                json!({}),
            ),
            entry(
                "aa-mod",
                vec![ver("0.1.0", json!({"server_api": 1}))],
                json!({"side": "both"}),
            ),
        ]))
        .unwrap();
        assert!(skipped.is_empty(), "{skipped:?}");
        assert_eq!(updated, "2026-10-22T00:00:00Z");
        assert_eq!(
            mods.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            ["aa-mod", "zz-mod"],
            "sorted by name"
        );
        let z = &mods[1];
        assert_eq!(
            z.versions
                .iter()
                .map(|v| v.version.as_str())
                .collect::<Vec<_>>(),
            ["1.10.0", "1.2.0", "1.0.0"]
        );
        assert!(z.versions[1].yanked && z.versions[1].released == "2026-10-01");
        assert_eq!(z.tags, ["qol"], "an invalid tag is dropped, not the mod");
        assert_eq!(mods[0].versions[0].server_api, Some(1));
    }

    #[test]
    fn broken_entries_and_versions_are_skipped_one_by_one() {
        let (_, mods, skipped) = parse_index(&file(vec![
            entry(
                "good-mod",
                vec![
                    ver("1.0.0", json!({})),
                    ver("nope", json!({})),
                    ver("1.1.0", json!({"sha256": "zz"})),
                    ver("1.2.0", json!({"url": "http://example.org/a.zip"})),
                ],
                json!({}),
            ),
            entry("Bad Id", vec![ver("1.0.0", json!({}))], json!({})),
            entry("no-versions", vec![], json!({})),
            entry(
                "bad-side",
                vec![ver("1.0.0", json!({}))],
                json!({"side": "x"}),
            ),
            entry(
                "bad-home",
                vec![ver("1.0.0", json!({}))],
                json!({"homepage": "javascript:alert(1)"}),
            ),
            entry(
                "big-mod",
                vec![ver("1.0.0", json!({"size": MAX_PACKAGE_BYTES + 1}))],
                json!({}),
            ),
            entry("good-mod", vec![ver("2.0.0", json!({}))], json!({})),
            json!("text"),
        ]))
        .unwrap();
        assert_eq!(mods.len(), 1);
        assert_eq!(
            mods[0].versions.len(),
            1,
            "only the usable version of good-mod is kept"
        );
        assert_eq!(skipped.len(), 7, "{skipped:?}");
    }

    #[test]
    fn the_file_must_have_the_right_schema() {
        assert!(parse_index("not json").is_err());
        assert!(parse_index("[]").is_err());
        assert!(
            parse_index(r#"{"mods": []}"#)
                .unwrap_err()
                .to_string()
                .contains("schema")
        );
        assert!(
            parse_index(r#"{"schema": 2, "mods": []}"#)
                .unwrap_err()
                .to_string()
                .contains("update the launcher")
        );
        assert!(parse_index(r#"{"schema": 1}"#).is_err());
        let (_, mods, _) = parse_index(r#"{"schema": 1, "mods": []}"#).unwrap();
        assert!(mods.is_empty(), "the empty template is valid");
    }

    #[test]
    fn urls_are_https_unless_the_test_allows_this_computer() {
        assert!(good_url("https://github.com/a/b.zip", false));
        assert!(!good_url("http://github.com/a/b.zip", false));
        assert!(!good_url("http://127.0.0.1:1/a.zip", false));
        assert!(good_url("http://127.0.0.1:1/a.zip", true));
        assert!(!good_url("http://evil.example/a.zip", true));
        assert!(!good_url("https://user:pw@github.com/a.zip", false));
        assert!(!good_url("file:///etc/passwd", true));
    }
}
