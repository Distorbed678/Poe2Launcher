//! Mod sync: what a server says its players need (`mods` in `GET /api/v1/server`), compared with
//! the launcher's mod cache, and the download that fills the cache.
//!
//! The advertisement is not trusted. Every entry is checked one by one (a broken one is skipped
//! and reported); a download is only ever kept when its SHA-256 equals the advertised one **and**
//! `poe_hook_mods::verify_package` accepts the package; a mismatch or any failed check leaves
//! nothing behind. Plain `http://` follows the launcher's rule for everything it sends to a
//! server: not to another machine unless the player confirmed that profile; an extra download
//! URL must be `https://` (or loopback).
//!
//! Wire format (the server's `mods` field; an array, or an object with `client_mods`):
//!
//! ```json
//! { "id": "faster-rolls", "version": "1.0.0", "side": "both", "sha256": "<64 hex>", "size": 48210,
//!   "required": true, "hook_api": 1, "game_versions": ["0.5.5e.hotfix1"],
//!   "sources": [ { "type": "server", "path": "/api/v1/mods/faster-rolls/1.0.0.zip" },
//!                { "type": "url", "url": "https://example.org/faster-rolls-1.0.0.zip" } ],
//!   "name": "Faster rolls", "authors": ["..."], "description": "..." }
//! ```

use std::{path::Path, time::Duration};

use anyhow::{Context, Result, anyhow, bail};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::AsyncWriteExt;

use crate::{
    modcache::{Added, ModCache},
    mods::{Compat, describe_compat, game_compat, valid_id},
    news::truncate,
    profiles::{Profile, is_loopback_host, needs_plain_http_confirmation},
};

/// The package size limit of `poe_hook_mods::Limits::default()`.
pub const MAX_PACKAGE_BYTES: u64 = 512 << 20;
const MAX_ENTRIES: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ModSource {
    /// A path on the server itself (`/api/v1/mods/{id}/{version}.zip`).
    Server { path: String },
    /// An extra address (https) that serves the same package.
    Url { url: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdvertisedMod {
    pub id: String,
    pub version: String,
    /// `client` or `both`.
    pub side: String,
    pub sha256: String,
    pub size: u64,
    /// The player must have it to play here. Absent means required.
    pub required: bool,
    pub hook_api: Option<u32>,
    pub game_versions: Vec<String>,
    pub sources: Vec<ModSource>,
    pub name: String,
    pub authors: Vec<String>,
    pub description: String,
    /// The package contains native code (a DLL in the game process). Taken from the server's
    /// `code` field when it sends one, else a mod that states a `hook_api` has a DLL.
    pub code: bool,
}

/// The `mods` field of `GET /server`: the readable entries and the reasons others were skipped.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Advertisement {
    pub mods: Vec<AdvertisedMod>,
    pub skipped: Vec<String>,
}

impl<'de> Deserialize<'de> for Advertisement {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        // Never fail the whole status over this field: a server that sends something odd is
        // treated like one that advertises nothing.
        Ok(match Option::<Value>::deserialize(d) {
            Ok(Some(v)) => parse_advertisement(&v),
            _ => Advertisement::default(),
        })
    }
}

fn clean(s: &str, max: usize) -> String {
    let s: String = s
        .replace("\r\n", "\n")
        .chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .collect();
    truncate(s.trim(), max)
}

fn is_hex64(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// A host-relative path the server may serve a package from.
fn valid_server_path(p: &str) -> bool {
    p.starts_with('/')
        && !p.starts_with("//")
        && p.len() <= 300
        && !p.contains(['\\', '?', '#', '\0'])
        && !p.split('/').any(|s| s == ".." || s == ".")
        && p.chars().all(|c| !c.is_control())
}

pub fn parse_advertisement(v: &Value) -> Advertisement {
    let list = match v {
        Value::Array(a) => Some(a),
        Value::Object(o) => o.get("client_mods").and_then(Value::as_array),
        _ => None,
    };
    let mut out = Advertisement::default();
    let Some(list) = list else {
        if !v.is_null() {
            out.skipped.push("the mods field is not a list".into());
        }
        return out;
    };
    for (i, item) in list.iter().enumerate().take(MAX_ENTRIES) {
        match parse_one(item) {
            Ok(Some(m)) => {
                if out.mods.iter().any(|x| x.id == m.id) {
                    out.skipped
                        .push(format!("entry {i}: {} is listed twice", m.id));
                } else {
                    out.mods.push(m);
                }
            }
            Ok(None) => {}
            Err(why) => out
                .skipped
                .push(truncate(&format!("entry {i}: {why}"), 200)),
        }
    }
    if list.len() > MAX_ENTRIES {
        out.skipped.push(format!(
            "more than {MAX_ENTRIES} mods: the rest are ignored"
        ));
    }
    out
}

/// `Ok(None)`: a mod that is not for the client (server side), not an error.
fn parse_one(v: &Value) -> Result<Option<AdvertisedMod>, String> {
    let o = v.as_object().ok_or("not an object")?;
    let s = |k: &str| o.get(k).and_then(Value::as_str);
    let id = s("id").ok_or("no id")?;
    if !valid_id(id) {
        return Err(format!("{id:?} is not a valid mod id"));
    }
    let version = s("version").ok_or("no version")?;
    if semver::Version::parse(version).is_err() || version.contains(['/', '\\']) {
        return Err(format!("{id}: {version:?} is not a semantic version"));
    }
    let side = s("side").unwrap_or("client");
    match side {
        "server" => return Ok(None),
        "client" | "both" => {}
        other => return Err(format!("{id}: unknown side {other:?}")),
    }
    let sha256 = s("sha256").ok_or_else(|| format!("{id}: no sha256"))?;
    if !is_hex64(sha256) {
        return Err(format!("{id}: sha256 is not 64 hex digits"));
    }
    let size = o
        .get("size")
        .and_then(Value::as_u64)
        .filter(|n| *n > 0)
        .ok_or_else(|| format!("{id}: no size"))?;
    if size > MAX_PACKAGE_BYTES {
        return Err(format!(
            "{id}: {size} bytes is more than the limit of {MAX_PACKAGE_BYTES}"
        ));
    }
    let hook_api = match o.get("hook_api") {
        None | Some(Value::Null) => None,
        Some(x) => Some(
            x.as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .ok_or_else(|| format!("{id}: hook_api must be a number"))?,
        ),
    };
    let strings = |k: &str, max_items: usize, max_len: usize| -> Vec<String> {
        o.get(k)
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .take(max_items)
                    .map(|x| clean(x, max_len))
                    .filter(|x| !x.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut sources = Vec::new();
    if let Some(list) = o.get("sources").and_then(Value::as_array) {
        for src in list.iter().take(8) {
            match serde_json::from_value::<ModSource>(src.clone()) {
                Ok(ModSource::Server { path }) if valid_server_path(&path) => {
                    sources.push(ModSource::Server { path });
                }
                Ok(ModSource::Url { url }) if url.len() <= 500 => {
                    sources.push(ModSource::Url { url });
                }
                _ => {} // an unusable source is not worth refusing the mod for
            }
        }
    }
    let code = o
        .get("code")
        .and_then(Value::as_bool)
        .unwrap_or(hook_api.is_some());
    Ok(Some(AdvertisedMod {
        id: id.to_owned(),
        version: version.to_owned(),
        side: side.to_owned(),
        sha256: sha256.to_ascii_lowercase(),
        size,
        required: o.get("required").and_then(Value::as_bool).unwrap_or(true),
        hook_api,
        game_versions: strings("game_versions", 16, 40),
        sources,
        name: s("name")
            .map(|n| clean(n, 80))
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| id.to_owned()),
        authors: strings("authors", 8, 60),
        description: s("description").map(|d| clean(d, 400)).unwrap_or_default(),
        code,
    }))
}

// ---------------------------------------------------------------------------------------------
// Sync modes and the plan

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SyncMode {
    Off,
    #[default]
    Ask,
    Auto,
}

impl SyncMode {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "off" => Some(Self::Off),
            "ask" => Some(Self::Ask),
            "auto" => Some(Self::Auto),
            _ => None,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Ask => "ask",
            Self::Auto => "auto",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncState {
    /// Cached with the advertised hash.
    Ready,
    /// Not in the cache.
    Missing,
    /// Cached, but this version has other content.
    Changed,
    /// Made for another game version or mod ABI: cannot be used here.
    Incompatible,
    /// There is nowhere the launcher may download it from.
    NoSource,
}

#[derive(Debug, Clone, Serialize)]
pub struct SyncMod {
    pub id: String,
    pub version: String,
    pub name: String,
    pub side: String,
    pub size: u64,
    pub sha256: String,
    pub required: bool,
    /// Contains native code that would run inside the game process.
    pub code: bool,
    pub authors: Vec<String>,
    pub description: String,
    pub state: SyncState,
    pub detail: String,
}

impl SyncMod {
    pub fn needs_download(&self) -> bool {
        matches!(self.state, SyncState::Missing | SyncState::Changed)
    }
}

/// Compares an advertisement with the cache. `game_version` is the selected game version;
/// `hook_abi` the mod ABI the installed hook offers.
pub fn compare(
    ads: &[AdvertisedMod],
    cache: &ModCache,
    profile: &Profile,
    game_version: &str,
    hook_abi: u32,
) -> Vec<SyncMod> {
    ads.iter()
        .map(|a| {
            let (state, detail) = state_of(a, cache, profile, game_version, hook_abi);
            SyncMod {
                id: a.id.clone(),
                version: a.version.clone(),
                name: a.name.clone(),
                side: a.side.clone(),
                size: a.size,
                sha256: a.sha256.clone(),
                required: a.required,
                code: a.code,
                authors: a.authors.clone(),
                description: a.description.clone(),
                state,
                detail,
            }
        })
        .collect()
}

fn state_of(
    a: &AdvertisedMod,
    cache: &ModCache,
    profile: &Profile,
    game_version: &str,
    hook_abi: u32,
) -> (SyncState, String) {
    if let Compat::WrongGame { supported } = game_compat(&a.game_versions, game_version) {
        return (
            SyncState::Incompatible,
            format!("made for {}, you play {game_version}", supported.join(", ")),
        );
    }
    if let Some(need) = a.hook_api
        && need != hook_abi
    {
        let c = if need > hook_abi {
            Compat::HookApiTooNew {
                needs: need,
                have: hook_abi,
            }
        } else {
            Compat::HookApiOld {
                needs: need,
                have: hook_abi,
            }
        };
        return (SyncState::Incompatible, describe_compat(&c));
    }
    match cache.get(&a.id, &a.version) {
        Some(c) if c.errors.is_empty() && c.meta.sha256 == a.sha256 => {
            return (SyncState::Ready, "downloaded and verified".into());
        }
        Some(c) if c.errors.is_empty() => {
            if source_urls(a, profile).is_empty() {
                return (SyncState::NoSource, no_source_reason(a, profile));
            }
            return (
                SyncState::Changed,
                "a copy with other content is cached; the server's build replaces it".into(),
            );
        }
        _ => {}
    }
    if source_urls(a, profile).is_empty() {
        return (SyncState::NoSource, no_source_reason(a, profile));
    }
    (SyncState::Missing, "not downloaded yet".into())
}

fn no_source_reason(a: &AdvertisedMod, profile: &Profile) -> String {
    if needs_plain_http_confirmation(&profile.url) && !profile.plain_http_ok {
        format!(
            "{} is reached over plain http:// and you did not confirm that (Servers); the download would travel unencrypted",
            profile.name
        )
    } else {
        format!(
            "no usable download address for {} (extra addresses must be https://)",
            a.id
        )
    }
}

/// The addresses a mod may be downloaded from, in order: the server's own first.
pub fn source_urls(a: &AdvertisedMod, profile: &Profile) -> Vec<String> {
    let base = profile.url.trim_end_matches('/');
    let server_ok = !needs_plain_http_confirmation(&profile.url) || profile.plain_http_ok;
    let server_default = format!("{base}/api/v1/mods/{}/{}.zip", a.id, a.version);
    let mut out: Vec<String> = Vec::new();
    let mut has_server = false;
    for s in &a.sources {
        match s {
            ModSource::Server { path } => {
                has_server = true;
                if server_ok {
                    out.push(format!("{base}{path}"));
                }
            }
            ModSource::Url { url } => {
                if let Ok(u) = reqwest::Url::parse(url) {
                    let local = u.host_str().is_some_and(is_loopback_host);
                    let https = u.scheme() == "https";
                    if (https || (u.scheme() == "http" && local)) && u.username().is_empty() {
                        out.push(u.to_string());
                    }
                }
            }
        }
    }
    if !has_server && server_ok {
        out.insert(0, server_default);
    }
    // The server's address first, whatever order the list had.
    let server_prefix = format!("{base}/");
    out.sort_by_key(|u| !u.starts_with(&server_prefix));
    out
}

/// What the player sees and the flow acts on.
#[derive(Debug, Clone, Serialize)]
pub struct SyncPlan {
    pub profile_id: String,
    pub profile_name: String,
    pub mode: SyncMode,
    /// `ok`, `ask`, `auto`, `off` or `blocked`.
    pub status: &'static str,
    pub mods: Vec<SyncMod>,
    /// Why Play cannot go on, when it cannot.
    pub blocker: Option<String>,
    /// Bytes still to download for the required mods.
    pub download_bytes: u64,
    /// Entries of the advertisement the launcher could not read.
    pub skipped: Vec<String>,
    /// The server could not be asked; this is the last advertisement it gave.
    pub stale: bool,
    /// Mods this call downloaded and verified (Auto, or the Download button).
    pub applied: Vec<String>,
    /// Downloads that failed, with the reason.
    pub errors: Vec<String>,
}

/// The status and blocker of a comparison for `mode`.
pub fn assess(
    mods: &[SyncMod],
    mode: SyncMode,
    profile_name: &str,
) -> (&'static str, Option<String>, u64) {
    let required: Vec<&SyncMod> = mods.iter().filter(|m| m.required).collect();
    let bytes: u64 = required
        .iter()
        .filter(|m| m.needs_download())
        .map(|m| m.size)
        .sum();
    let bad: Vec<String> = required
        .iter()
        .filter(|m| matches!(m.state, SyncState::Incompatible | SyncState::NoSource))
        .map(|m| format!("{} {} ({})", m.name, m.version, m.detail))
        .collect();
    if !bad.is_empty() {
        return (
            "blocked",
            Some(format!(
                "{profile_name} requires mods this setup cannot use: {}",
                bad.join("; ")
            )),
            bytes,
        );
    }
    let missing: Vec<String> = required
        .iter()
        .filter(|m| m.needs_download())
        .map(|m| format!("{} {}", m.name, m.version))
        .collect();
    if missing.is_empty() {
        return ("ok", None, 0);
    }
    match mode {
        SyncMode::Off => (
            "off",
            Some(format!(
                "{profile_name} requires mods you do not have: {}. Mod sync is Off, so nothing is downloaded; \
                 switch Mod sync to Ask or Auto (Settings) or get them yourself",
                missing.join(", ")
            )),
            bytes,
        ),
        SyncMode::Ask => (
            "ask",
            Some(format!(
                "{profile_name} requires mods that are not downloaded yet: {}",
                missing.join(", ")
            )),
            bytes,
        ),
        SyncMode::Auto => ("auto", None, bytes),
    }
}

// ---------------------------------------------------------------------------------------------
// Download

fn http() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(900))
        .user_agent(concat!("poe2-launcher/", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            // Never follow a redirect to plain http (except to this computer).
            let local = attempt.url().host_str().is_some_and(is_loopback_host);
            if attempt.previous().len() >= 5 {
                attempt.error("too many redirects")
            } else if attempt.url().scheme() == "https" || local {
                attempt.follow()
            } else {
                attempt.error("redirected to plain http")
            }
        }))
        .build()
        .map_err(Into::into)
}

/// Downloads `url` into `dest`, at most `max` bytes (and not more than `expect` when given).
pub async fn download_file(
    url: &str,
    dest: &Path,
    expect: u64,
    on_progress: &(dyn Fn(u64, u64) + Send + Sync),
) -> Result<u64> {
    let res = http()?
        .get(url)
        .send()
        .await
        .map_err(|e| anyhow!("cannot reach {}: {}", display_url(url), e.without_url()))?;
    let status = res.status();
    if !status.is_success() {
        bail!("{} answered HTTP {}", display_url(url), status.as_u16());
    }
    if let Some(len) = res.content_length()
        && len > expect
    {
        bail!(
            "{} sends {len} bytes, more than the {expect} the server advertised",
            display_url(url)
        );
    }
    let mut file = tokio::fs::File::create(dest).await?;
    let mut done = 0u64;
    let mut stream = res.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| anyhow!("the download broke off: {}", e.without_url()))?;
        done += chunk.len() as u64;
        if done > expect {
            bail!(
                "{} sends more than the {expect} bytes the server advertised",
                display_url(url)
            );
        }
        file.write_all(&chunk).await?;
        on_progress(done, expect);
    }
    file.flush().await?;
    Ok(done)
}

/// Host and path only (no query, no credentials) for messages.
fn display_url(url: &str) -> String {
    reqwest::Url::parse(url)
        .map(|u| format!("{}{}", u.host_str().unwrap_or("?"), u.path()))
        .unwrap_or_else(|_| "the download address".into())
}

/// Downloads one mod (advertised, or from the index) from `urls` in order and adds it to the
/// cache. The package is only kept when its hash is the pinned one and `verify_package_bytes`
/// accepts it; the cache is untouched otherwise. `source` is recorded in the cache entry.
pub async fn fetch_into_cache(
    a: &AdvertisedMod,
    urls: &[String],
    source: &str,
    cache: &ModCache,
    scratch_root: &Path,
    on_progress: &(dyn Fn(u64, u64) + Send + Sync),
) -> Result<Added> {
    if urls.is_empty() {
        bail!("no download address for {}", a.id);
    }
    std::fs::create_dir_all(scratch_root)?;
    let dir = tempfile::Builder::new()
        .prefix("dl-")
        .tempdir_in(scratch_root)
        .context("creating a download folder")?;
    let file = dir
        .path()
        .join(poe_hook_mods::package_file_name(&a.id, &a.version));
    let mut last = String::new();
    for url in urls {
        let _ = std::fs::remove_file(&file);
        match download_file(url, &file, a.size, on_progress).await {
            Ok(n) if n != a.size => {
                last = format!(
                    "{} sent {n} bytes, the server advertised {}",
                    display_url(url),
                    a.size
                );
            }
            Ok(_) => match cache.add_package(&file, Some(&a.sha256), source, true) {
                Ok(added) => {
                    if added.id != a.id || added.version != a.version {
                        // Cannot happen (the name rule), but never trust one check alone.
                        let _ = cache.remove(&added.id, &added.version);
                        last = format!("{} holds {} {}", display_url(url), added.id, added.version);
                        continue;
                    }
                    return Ok(added);
                }
                Err(e) => last = format!("{} was refused: {e:#}", display_url(url)),
            },
            Err(e) => last = format!("{e:#}"),
        }
    }
    bail!("{} {}: {last}", a.name, a.version)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::modcache::testutil::*;

    fn entry(id: &str, extra: Value) -> Value {
        let mut v = json!({"id": id, "version": "1.0.0", "side": "both", "sha256": "a".repeat(64), "size": 100});
        v.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        v
    }

    fn profile(url: &str, ok: bool) -> Profile {
        Profile {
            id: "p1".into(),
            name: "Realm".into(),
            url: url.into(),
            plain_http_ok: ok,
            mods: Vec::new(),
        }
    }

    #[test]
    fn the_advertisement_is_read_from_an_array_or_the_analysis_object() {
        let a = json!([entry(
            "aa-mod",
            json!({"required": false, "hook_api": 1, "name": "Aa", "game_versions": ["0.5.5e.hotfix1"]})
        )]);
        let ad = parse_advertisement(&a);
        assert_eq!(ad.mods.len(), 1);
        assert!(!ad.mods[0].required && ad.mods[0].code && ad.mods[0].name == "Aa");
        let obj = json!({"server_api": 1, "client_mods": [entry("bb-mod", json!({}))], "server_only_count": 3});
        let ad = parse_advertisement(&obj);
        assert_eq!(ad.mods.len(), 1);
        assert!(ad.mods[0].required, "absent means required");
        assert!(!ad.mods[0].code, "no hook_api, no code field: data only");
        assert!(parse_advertisement(&Value::Null).mods.is_empty());
        assert_eq!(parse_advertisement(&json!("x")).skipped.len(), 1);
    }

    #[test]
    fn broken_entries_are_skipped_one_by_one() {
        let ad = parse_advertisement(&json!([
            entry("good-mod", json!({})),
            entry("Bad Id", json!({})),
            entry("bad-hash", json!({"sha256": "xyz"})),
            entry("big-mod", json!({"size": MAX_PACKAGE_BYTES + 1})),
            entry("nosize-mod", json!({"size": 0})),
            entry("ver-mod", json!({"version": "../1"})),
            entry("side-mod", json!({"side": "weird"})),
            entry("srv-mod", json!({"side": "server"})),
            entry("good-mod", json!({})),
            "text"
        ]));
        assert_eq!(ad.mods.len(), 1);
        assert_eq!(ad.skipped.len(), 8, "{:?}", ad.skipped); // the server-only entry is silently left out
    }

    #[test]
    fn an_old_server_without_mods_or_with_junk_does_not_break_the_status() {
        let info: crate::api::ServerInfo = serde_json::from_str(
            r#"{"name":"n","version":"0.5.5e.hotfix1","cluster_id":"c","online":0,"open_registration":false,
                "login_host":"h","login_port":20481,"uptime_secs":3}"#,
        )
        .unwrap();
        assert!(info.mods.mods.is_empty());
        let info: crate::api::ServerInfo = serde_json::from_str(
            r#"{"name":"n","version":"0.5.5e.hotfix1","cluster_id":"c","online":0,"open_registration":false,
                "login_host":"h","login_port":20481,"uptime_secs":3,"mods":42}"#,
        )
        .unwrap();
        assert!(info.mods.mods.is_empty() && info.mods.skipped.len() == 1);
    }

    #[test]
    fn sources_follow_the_http_rules() {
        let a = parse_advertisement(&json!([entry(
            "aa-mod",
            json!({"sources": [
                {"type": "url", "url": "https://cdn.example.org/aa-mod-1.0.0.zip"},
                {"type": "url", "url": "http://cdn.example.org/aa-mod-1.0.0.zip"},
                {"type": "server", "path": "/api/v1/mods/aa-mod/1.0.0.zip"},
                {"type": "server", "path": "/../etc/passwd"},
                {"type": "url", "url": "https://user:pw@cdn.example.org/x.zip"},
            ]})
        )]))
        .mods
        .remove(0);
        // https server: its own path first, then the https URL; the http one is dropped.
        let urls = source_urls(&a, &profile("https://play.example.org", false));
        assert_eq!(
            urls,
            [
                "https://play.example.org/api/v1/mods/aa-mod/1.0.0.zip",
                "https://cdn.example.org/aa-mod-1.0.0.zip"
            ]
        );
        // Plain http to another machine, not confirmed: only the https URL remains.
        let urls = source_urls(&a, &profile("http://192.168.1.20:8080", false));
        assert_eq!(urls, ["https://cdn.example.org/aa-mod-1.0.0.zip"]);
        // Confirmed: the server's path is usable again.
        assert_eq!(
            source_urls(&a, &profile("http://192.168.1.20:8080", true)).len(),
            2
        );
        // Loopback needs no confirmation.
        assert_eq!(
            source_urls(&a, &profile("http://127.0.0.1:8080", false)).len(),
            2
        );
        // No sources at all: the default path on the server.
        let b = parse_advertisement(&json!([entry("bb-mod", json!({}))]))
            .mods
            .remove(0);
        assert_eq!(
            source_urls(&b, &profile("http://127.0.0.1:8080", false)),
            ["http://127.0.0.1:8080/api/v1/mods/bb-mod/1.0.0.zip"]
        );
        assert!(source_urls(&b, &profile("http://192.168.1.20:8080", false)).is_empty());
    }

    #[test]
    fn the_comparison_names_each_state() {
        let t = tempfile::tempdir().unwrap();
        let cache = ModCache::new(t.path().join("cache"));
        let zip = package(
            t.path(),
            "have-mod",
            "1.0.0",
            &[
                ("mod.toml", v1_manifest("have-mod", "1.0.0").as_bytes()),
                ("have-mod.dll", b"MZ"),
            ],
        );
        cache.add_package(&zip, None, "zip", false).unwrap();
        let good = hash_of(&zip);
        let ads = parse_advertisement(&json!([
            entry("have-mod", json!({"sha256": good, "hook_api": 1})),
            entry("new-mod", json!({})),
            entry("have-mod", json!({})), // duplicate id: skipped by the parser
        ]))
        .mods;
        let mut ads = ads;
        ads.push(AdvertisedMod {
            id: "chg-mod".into(),
            ..ads[1].clone()
        });
        // chg-mod is cached with another hash.
        let z2 = package(
            t.path(),
            "chg-mod",
            "1.0.0",
            &[
                ("mod.toml", v1_manifest("chg-mod", "1.0.0").as_bytes()),
                ("chg-mod.dll", b"MZ"),
            ],
        );
        cache.add_package(&z2, None, "zip", false).unwrap();
        ads.push(AdvertisedMod {
            id: "old-mod".into(),
            game_versions: vec!["0.4.0".into()],
            ..ads[1].clone()
        });
        ads.push(AdvertisedMod {
            id: "abi-mod".into(),
            hook_api: Some(2),
            ..ads[1].clone()
        });
        let p = profile("http://127.0.0.1:8080", false);
        let by: std::collections::BTreeMap<String, SyncState> =
            compare(&ads, &cache, &p, "0.5.5e.hotfix1", 1)
                .into_iter()
                .map(|m| (m.id, m.state))
                .collect();
        assert_eq!(by["have-mod"], SyncState::Ready);
        assert_eq!(by["new-mod"], SyncState::Missing);
        assert_eq!(by["chg-mod"], SyncState::Changed);
        assert_eq!(by["old-mod"], SyncState::Incompatible);
        assert_eq!(by["abi-mod"], SyncState::Incompatible);
    }

    fn sm(id: &str, state: SyncState, required: bool) -> SyncMod {
        SyncMod {
            id: id.into(),
            version: "1.0.0".into(),
            name: id.into(),
            side: "both".into(),
            size: 10,
            sha256: String::new(),
            required,
            code: false,
            authors: vec![],
            description: String::new(),
            state,
            detail: "why".into(),
        }
    }

    #[test]
    fn the_plan_follows_the_sync_mode() {
        let missing = [
            sm("aa-mod", SyncState::Missing, true),
            sm("bb-mod", SyncState::Ready, true),
            sm("opt-mod", SyncState::Missing, false),
        ];
        let (st, why, bytes) = assess(&missing, SyncMode::Off, "Realm");
        assert_eq!(st, "off");
        assert!(why.unwrap().contains("Mod sync is Off"));
        assert_eq!(bytes, 10, "optional mods do not count");
        assert_eq!(assess(&missing, SyncMode::Ask, "Realm").0, "ask");
        assert_eq!(assess(&missing, SyncMode::Auto, "Realm").0, "auto");
        assert_eq!(
            assess(
                &[sm("a-mod", SyncState::Ready, true)],
                SyncMode::Off,
                "Realm"
            )
            .0,
            "ok"
        );
        // An incompatible required mod blocks in every mode, and an optional one never does.
        let bad = [sm("aa-mod", SyncState::Incompatible, true)];
        for m in [SyncMode::Off, SyncMode::Ask, SyncMode::Auto] {
            assert_eq!(assess(&bad, m, "Realm").0, "blocked");
        }
        assert_eq!(
            assess(
                &[sm("aa-mod", SyncState::Incompatible, false)],
                SyncMode::Ask,
                "Realm"
            )
            .0,
            "ok"
        );
        assert_eq!(SyncMode::parse("auto"), Some(SyncMode::Auto));
        assert_eq!(SyncMode::default(), SyncMode::Ask);
    }
}
