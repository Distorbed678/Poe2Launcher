//! The public server list: a `servers.json` in the GitHub repository
//! `Distorbed678/Poe2ServerList` (anyone adds a server by pull request). The launcher fetches
//! it, checks every entry, caches it on disk and asks each listed server's `GET /api/v1/server`
//! itself (players, client version, ping), so the list never claims anything it did not see.
//!
//! The file is not trusted. Entries are checked one by one (a broken one is skipped and
//! reported, not the whole list); an address that is this computer or a local network is
//! refused, so a list cannot make the launcher poke at the player's own network; only
//! `GET /server` is sent (no login, no token). The schema is in `docs/server-list-template/`.

use std::{collections::HashSet, time::Duration};

use anyhow::{Context, Result, anyhow, bail};
use futures_util::{StreamExt, stream};
use serde::{Deserialize, Serialize};

use crate::{
    api::{ApiClient, ServerInfo},
    core::{Core, now_secs},
    news::{load_cache_as, save_cache_as, truncate},
    profiles::{is_private_host, normalize_base_url, plain_http, url_host},
};

pub const LIST_URL: &str =
    "https://raw.githubusercontent.com/Distorbed678/Poe2ServerList/main/servers.json";
pub const CACHE_TTL: i64 = 10 * 60;
pub const FORCE_GAP: i64 = 20;
const MAX_FILE: usize = 1 << 20;
const MAX_SERVERS: usize = 500;
const MAX_TAGS: usize = 8;
const MAX_MODS: usize = 16;
/// How many servers are asked at the same time.
pub const PROBE_PARALLEL: usize = 12;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RequiredMod {
    pub id: String,
    /// A version requirement such as `^1.2`; free text.
    #[serde(default)]
    pub version: Option<String>,
    /// Where to get it (https).
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ListedServer {
    /// Stable key of the entry: `[a-z0-9-]`, 3-48 characters.
    pub id: String,
    pub name: String,
    /// The web API address, normalised (`https://play.example.org`).
    pub address: String,
    pub description: String,
    pub region: String,
    /// The client version the server runs, as the listing owner states it.
    pub game_version: String,
    pub tags: Vec<String>,
    /// Free text: a Discord invite, an e-mail address, a web page.
    pub contact: String,
    pub mods: Vec<RequiredMod>,
    /// The address is plain `http://` (the UI warns and asks before adding).
    #[serde(default)]
    pub plain_http: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ListCache {
    pub attempted_at: i64,
    pub fetched_at: i64,
    pub servers: Vec<ListedServer>,
    pub skipped: Vec<String>,
}

/// What the Browse tab gets.
#[derive(Debug, Clone, Serialize)]
pub struct ServerList {
    pub servers: Vec<ListedServer>,
    /// Entries of the file that were not valid, with the reason.
    pub skipped: Vec<String>,
    pub fetched_at: i64,
    /// The list could not be fetched just now: this is the saved copy (or empty).
    pub offline: bool,
    /// Why, for a short note (empty when fine).
    pub error: String,
}

/// One server asked for its live state.
#[derive(Debug, Clone, Serialize)]
pub struct ProbeResult {
    pub id: String,
    pub ok: bool,
    pub ping_ms: Option<u32>,
    pub info: Option<ServerInfo>,
    pub error: Option<String>,
}

// ---------------------------------------------------------------------------------------------
// Parsing

#[derive(Debug, Deserialize)]
struct RawServer {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    address: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    region: String,
    #[serde(default)]
    game_version: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    contact: String,
    #[serde(default)]
    mods: Vec<RequiredMod>,
}

fn no_controls(s: &str) -> bool {
    !s.chars().any(|c| c.is_control())
}

fn valid_list_id(s: &str) -> bool {
    (3..=48).contains(&s.len())
        && !s.starts_with('-')
        && !s.ends_with('-')
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn valid_tag(s: &str) -> bool {
    (1..=24).contains(&s.len())
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn valid_mod_id(s: &str) -> bool {
    (1..=64).contains(&s.len())
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

fn check_server(r: RawServer, allow_local: bool) -> Result<ListedServer, String> {
    let id = r.id.clone();
    if !valid_list_id(&r.id) {
        return Err("id must be 3-48 characters of a-z, 0-9 and -".into());
    }
    let name = r.name.trim().to_owned();
    if name.is_empty() || name.chars().count() > 64 || !no_controls(&name) {
        return Err(format!("{id}: name is 1-64 characters"));
    }
    let address = normalize_base_url(&r.address).map_err(|e| format!("{id}: address: {e}"))?;
    let host = url_host(&address).unwrap_or_default();
    if !allow_local && is_private_host(&host) {
        return Err(format!(
            "{id}: address {host} is this computer or a local network; a public list cannot name one"
        ));
    }
    let description = r.description.trim();
    if description.chars().count() > 400 || !no_controls(&description.replace('\n', " ")) {
        return Err(format!("{id}: description is at most 400 characters"));
    }
    let region = r.region.trim();
    if region.chars().count() > 24 || !no_controls(region) {
        return Err(format!("{id}: region is at most 24 characters"));
    }
    let game_version = r.game_version.trim();
    if game_version.is_empty()
        || game_version.len() > 16
        || !game_version
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
    {
        return Err(format!(
            "{id}: game_version is required (for example 0.5.5e)"
        ));
    }
    if r.tags.len() > MAX_TAGS || !r.tags.iter().all(|t| valid_tag(t)) {
        return Err(format!(
            "{id}: at most {MAX_TAGS} tags of a-z, 0-9 and - (24 characters each)"
        ));
    }
    let contact = r.contact.trim();
    if contact.chars().count() > 120 || !no_controls(contact) {
        return Err(format!("{id}: contact is at most 120 characters"));
    }
    if r.mods.len() > MAX_MODS {
        return Err(format!("{id}: at most {MAX_MODS} required mods"));
    }
    let mut mods = Vec::new();
    for m in r.mods {
        if !valid_mod_id(&m.id) {
            return Err(format!("{id}: a required mod has no valid id"));
        }
        let version = m
            .version
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty());
        if version
            .as_ref()
            .is_some_and(|v| v.len() > 32 || !no_controls(v))
        {
            return Err(format!(
                "{id}: mod {}: version is at most 32 characters",
                m.id
            ));
        }
        mods.push(RequiredMod {
            id: m.id,
            version,
            url: m.url.filter(|u| {
                u.starts_with("https://") && u.len() <= 300 && !u.contains(char::is_whitespace)
            }),
        });
    }
    Ok(ListedServer {
        plain_http: plain_http(&address).is_some(),
        id: r.id,
        name,
        address,
        description: description.to_owned(),
        region: region.to_owned(),
        game_version: game_version.to_owned(),
        tags: r.tags,
        contact: contact.to_owned(),
        mods,
    })
}

/// Reads `servers.json`: `{"schema": 1, "servers": [...]}`. Returns the valid entries and a
/// reason for every skipped one.
pub fn parse_list(text: &str) -> Result<(Vec<ListedServer>, Vec<String>)> {
    parse_list_with(text, false)
}

/// [`parse_list`]; `allow_local` lets tests list servers on this computer.
pub fn parse_list_with(text: &str, allow_local: bool) -> Result<(Vec<ListedServer>, Vec<String>)> {
    if text.len() > MAX_FILE {
        bail!("servers.json is larger than {} KB", MAX_FILE / 1024);
    }
    let v: serde_json::Value =
        serde_json::from_str(text).context("servers.json is not valid JSON")?;
    if let Some(s) = v.get("schema").and_then(|s| s.as_u64())
        && s != 1
    {
        bail!("servers.json uses schema {s}; this launcher reads schema 1 (update the launcher)");
    }
    let list = v
        .get("servers")
        .and_then(|s| s.as_array())
        .ok_or_else(|| anyhow!("servers.json has no \"servers\" list"))?;
    let (mut out, mut skipped) = (Vec::new(), Vec::new());
    let (mut ids, mut addrs) = (HashSet::new(), HashSet::new());
    for (i, raw) in list.iter().enumerate() {
        if out.len() >= MAX_SERVERS {
            skipped.push(format!(
                "more than {MAX_SERVERS} entries: the rest is ignored"
            ));
            break;
        }
        let r: RawServer = match serde_json::from_value(raw.clone()) {
            Ok(r) => r,
            Err(e) => {
                skipped.push(format!("entry {i}: {e}"));
                continue;
            }
        };
        match check_server(r, allow_local) {
            Ok(s) => {
                if !ids.insert(s.id.clone()) {
                    skipped.push(format!("{}: duplicate id", s.id));
                } else if !addrs.insert(s.address.to_ascii_lowercase()) {
                    skipped.push(format!("{}: same address as an earlier entry", s.id));
                } else {
                    out.push(s);
                }
            }
            Err(why) => skipped.push(truncate(&format!("entry {i}: {why}"), 200)),
        }
    }
    Ok((out, skipped))
}

// ---------------------------------------------------------------------------------------------
// Fetching and probing

async fn fetch_list(url: &str, allow_local: bool) -> Result<(Vec<ListedServer>, Vec<String>)> {
    let c = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(6))
        .timeout(Duration::from_secs(12))
        .user_agent(concat!("poe2-launcher/", env!("CARGO_PKG_VERSION")))
        .build()?;
    let res = c
        .get(url)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| anyhow!("cannot reach the server list: {}", e.without_url()))?;
    let status = res.status();
    match status.as_u16() {
        200..=299 => {}
        404 => bail!("the server list does not exist yet (HTTP 404)"),
        403 | 429 => bail!(
            "the server list is rate limited (HTTP {}); try again later",
            status.as_u16()
        ),
        code => bail!("the server list answered HTTP {code}"),
    }
    let bytes = res.bytes().await?;
    if bytes.len() > MAX_FILE {
        bail!("servers.json is larger than {} KB", MAX_FILE / 1024);
    }
    parse_list_with(&String::from_utf8_lossy(&bytes), allow_local)
}

/// Asks one server (`GET /api/v1/server`) with short timeouts.
pub async fn probe_one(s: &ListedServer) -> ProbeResult {
    let c = ApiClient::with_timeouts(&s.address, Duration::from_secs(3), Duration::from_secs(6));
    match c.server_status().await {
        Ok(st) => ProbeResult {
            id: s.id.clone(),
            ok: true,
            ping_ms: Some(st.ping_ms),
            info: Some(st.info),
            error: None,
        },
        Err(e) => ProbeResult {
            id: s.id.clone(),
            ok: false,
            ping_ms: None,
            info: None,
            error: Some(e.to_string()),
        },
    }
}

/// Asks all of `servers`, `PROBE_PARALLEL` at a time.
pub async fn probe_all(servers: &[ListedServer]) -> Vec<ProbeResult> {
    stream::iter(servers.iter())
        .map(probe_one)
        .buffer_unordered(PROBE_PARALLEL)
        .collect()
        .await
}

impl Core {
    fn server_list_cache_file(&self) -> std::path::PathBuf {
        self.dirs.data.join("cache").join("serverlist.json")
    }

    /// The public server list (cached; `force`: the user pressed refresh). Never an error: an
    /// unreachable list gives the saved copy and a note.
    pub async fn server_list_get(&self, force: bool) -> ServerList {
        self.server_list_get_from(LIST_URL, force, false).await
    }

    /// `url` and `allow_local` are parameters for tests (a mock list names servers on 127.0.0.1).
    pub async fn server_list_get_from(
        &self,
        url: &str,
        force: bool,
        allow_local: bool,
    ) -> ServerList {
        let file = self.server_list_cache_file();
        let cache: ListCache = load_cache_as(&file);
        let now = now_secs();
        let age = now - cache.attempted_at;
        if (if force {
            age < FORCE_GAP
        } else {
            age < CACHE_TTL
        }) && cache.attempted_at > 0
        {
            return ServerList {
                offline: cache.fetched_at < cache.attempted_at,
                servers: cache.servers,
                skipped: cache.skipped,
                fetched_at: cache.fetched_at,
                error: String::new(),
            };
        }
        match fetch_list(url, allow_local).await {
            Ok((servers, skipped)) => {
                let c = ListCache {
                    attempted_at: now,
                    fetched_at: now,
                    servers,
                    skipped,
                };
                save_cache_as(&file, &c);
                ServerList {
                    servers: c.servers,
                    skipped: c.skipped,
                    fetched_at: now,
                    offline: false,
                    error: String::new(),
                }
            }
            Err(e) => {
                let c = ListCache {
                    attempted_at: now,
                    ..cache
                };
                save_cache_as(&file, &c);
                ServerList {
                    servers: c.servers,
                    skipped: c.skipped,
                    fetched_at: c.fetched_at,
                    offline: true,
                    error: format!("{e:#}"),
                }
            }
        }
    }

    /// Asks the listed servers with these ids for their live state.
    pub async fn server_list_probe(&self, ids: &[String]) -> Result<Vec<ProbeResult>> {
        let cache: ListCache = load_cache_as(&self.server_list_cache_file());
        let wanted: HashSet<&str> = ids.iter().map(String::as_str).collect();
        let servers: Vec<ListedServer> = cache
            .servers
            .into_iter()
            .filter(|s| wanted.contains(s.id.as_str()))
            .collect();
        if servers.is_empty() && !ids.is_empty() {
            bail!("those servers are not in the list (refresh it first)");
        }
        Ok(probe_all(&servers).await)
    }

    /// `GET /server` of one of the user's servers, with the ping measured here.
    pub async fn server_status(&self, id: &str) -> Result<crate::api::ServerStatus> {
        let (_, c) = self.client_for(id)?;
        Ok(c.server_status().await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(extra: &str) -> String {
        format!(
            r#"{{"id":"example-realm","name":"Example Realm","address":"https://play.example.org",
                "description":"A realm","region":"EU","game_version":"0.5.5e","tags":["pve"],
                "contact":"discord.gg/x"{extra}}}"#
        )
    }

    fn list(entries: &[String]) -> String {
        format!(r#"{{"schema":1,"servers":[{}]}}"#, entries.join(","))
    }

    #[test]
    fn a_valid_entry_is_read_and_normalised() {
        let (s, skipped) = parse_list(&list(&[entry(
            r#","mods":[{"id":"core-lib","version":"^1","url":"https://example.org/m.zip"}]"#,
        )]))
        .unwrap();
        assert!(skipped.is_empty(), "{skipped:?}");
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].address, "https://play.example.org");
        assert!(!s[0].plain_http);
        assert_eq!(s[0].mods[0].id, "core-lib");
        assert_eq!(
            s[0].mods[0].url.as_deref(),
            Some("https://example.org/m.zip")
        );
    }

    #[test]
    fn plain_http_is_flagged_and_local_addresses_refused() {
        let http = entry("").replace("https://play.example.org", "http://203.0.113.9:8080");
        let (s, _) = parse_list(&list(&[http])).unwrap();
        assert!(s[0].plain_http);
        for local in [
            "http://127.0.0.1:8080",
            "192.168.1.20:8080",
            "localhost",
            "https://10.0.0.5",
            "http://[::1]:8080",
            "box.local",
        ] {
            let e = entry("").replace("https://play.example.org", local);
            let (s, skipped) = parse_list(&list(&[e])).unwrap();
            assert!(s.is_empty(), "{local} must be refused");
            assert!(
                skipped[0].contains("local network") || skipped[0].contains("address"),
                "{local}: {skipped:?}"
            );
        }
        // A bare name gets https and stays public.
        let bare = entry("").replace("https://play.example.org", "play.example.org:8443");
        assert_eq!(
            parse_list(&list(&[bare])).unwrap().0[0].address,
            "https://play.example.org:8443"
        );
    }

    #[test]
    fn bad_entries_are_skipped_one_by_one() {
        let bad_id = entry("").replace("example-realm", "Bad Id");
        let no_version = entry("").replace(r#""game_version":"0.5.5e","#, "");
        let many_tags = entry("").replace(r#"["pve"]"#, r#"["a","b","c","d","e","f","g","h","i"]"#);
        let upper_tag = entry("").replace(r#"["pve"]"#, r#"["PvE"]"#);
        let long_name = entry("").replace("Example Realm", &"n".repeat(65));
        let dup = entry("").replace("play.example.org", "other.example.org"); // same id as the good one
        let same_addr = entry("").replace("example-realm", "second-realm");
        let good = entry("");
        let (s, skipped) = parse_list(&list(&[
            good,
            bad_id,
            no_version,
            many_tags,
            upper_tag,
            long_name,
            dup,
            same_addr,
            "42".into(),
        ]))
        .unwrap();
        assert_eq!(s.len(), 1, "{skipped:?}");
        assert_eq!(skipped.len(), 8, "{skipped:?}");
        assert!(skipped.iter().any(|m| m.contains("duplicate id")));
        assert!(skipped.iter().any(|m| m.contains("same address")));
    }

    #[test]
    fn broken_files_are_errors() {
        assert!(parse_list("[]").is_err());
        assert!(parse_list("nope").is_err());
        assert!(
            parse_list(r#"{"schema":2,"servers":[]}"#)
                .unwrap_err()
                .to_string()
                .contains("schema 2")
        );
        assert!(
            parse_list(&format!(
                r#"{{"servers":[],"x":"{}"}}"#,
                "x".repeat(MAX_FILE)
            ))
            .is_err()
        );
        assert!(
            parse_list(r#"{"schema":1,"servers":[]}"#)
                .unwrap()
                .0
                .is_empty()
        );
    }

    #[test]
    fn mod_urls_must_be_https() {
        let e = entry(r#","mods":[{"id":"m","url":"http://example.org/m.zip"},{"id":"n"}]"#);
        let (s, _) = parse_list(&list(&[e])).unwrap();
        assert_eq!(s[0].mods[0].url, None);
        let e = entry(r#","mods":[{"id":"has space"}]"#);
        assert!(parse_list(&list(&[e])).unwrap().0.is_empty());
    }
}
