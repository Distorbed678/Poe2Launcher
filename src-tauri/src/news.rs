//! News for the Home page: the newest release notes of the project's three repositories (the
//! launcher, the server `server-v*`, the client hook `hook-v*`) from the GitHub releases API,
//! and `news.json` from the main repository (hand-written announcements).
//!
//! Everything is optional and quiet: the result is cached on disk (`<data>/cache/news.json`),
//! an offline machine or a GitHub rate limit shows the saved copy, a source that fails keeps its
//! older items. Nothing here is trusted: sizes are capped, links must be https, and the text is
//! shown by the UI as text (a small markdown subset, never raw HTML).
//!
//! `news.json` schema (version 1), also documented in `docs/guides/launcher.md`:
//!
//! ```json
//! { "version": 1, "items": [
//!   { "id": "welcome", "date": "2026-10-22", "title": "Welcome", "body": "Markdown subset", "link": "https://..." } ] }
//! ```

use std::{path::Path, time::Duration};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};

use crate::{
    core::{Core, now_secs},
    paths::write_atomic,
};

pub const NEWS_URL: &str =
    "https://raw.githubusercontent.com/Distorbed678/Poe2ModdingProject/main/news.json";
pub const API_BASE: &str = "https://api.github.com";
/// A fresh cache is used as it is for this long (seconds).
pub const CACHE_TTL: i64 = 30 * 60;
/// After a forced refresh, another one is ignored for this long.
pub const FORCE_GAP: i64 = 60;
const RELEASES_PER_REPO: usize = 3;
const MAX_ITEMS: usize = 12;
const MAX_BODY: usize = 4000;
const MAX_RELEASE_BODY: usize = 1500;
const MAX_TITLE: usize = 120;
const MAX_NEWS_FILE: usize = 256 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NewsItem {
    pub id: String,
    /// `launcher`, `server`, `hook` (release notes) or `project` (news.json).
    pub source: String,
    /// `YYYY-MM-DD` or an RFC 3339 time.
    pub date: String,
    pub title: String,
    /// A markdown subset; the UI renders it as text.
    pub body: String,
    pub link: Option<String>,
    /// The release version for release notes.
    #[serde(default)]
    pub version: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RepoSource {
    pub repo: &'static str,
    /// Tag prefix that selects the releases of this product (`hook-v`, `server-v`, `v`).
    pub prefix: &'static str,
    pub source: &'static str,
    pub label: &'static str,
}

pub const REPOS: [RepoSource; 3] = [
    RepoSource {
        repo: "Distorbed678/Poe2Launcher",
        prefix: "v",
        source: "launcher",
        label: "Launcher",
    },
    RepoSource {
        repo: "Distorbed678/Poe2Server",
        prefix: "server-v",
        source: "server",
        label: "Server",
    },
    RepoSource {
        repo: "Distorbed678/Poe2ClientHook",
        prefix: "hook-v",
        source: "hook",
        label: "Client hook",
    },
];

/// Where the news come from (parameters for tests).
#[derive(Debug, Clone)]
pub struct NewsSources {
    pub api_base: String,
    pub news_url: String,
}

impl Default for NewsSources {
    fn default() -> Self {
        Self {
            api_base: API_BASE.into(),
            news_url: NEWS_URL.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NewsCache {
    /// The last time a fetch was tried (also when it failed), Unix seconds.
    pub attempted_at: i64,
    /// The last time at least one source answered.
    pub fetched_at: i64,
    pub items: Vec<NewsItem>,
}

/// What the Home page gets.
#[derive(Debug, Clone, Serialize)]
pub struct NewsFeed {
    pub items: Vec<NewsItem>,
    pub fetched_at: i64,
    /// Nothing could be fetched just now: the items are the saved copy (or none).
    pub offline: bool,
}

// ---------------------------------------------------------------------------------------------
// Parsing

#[derive(Debug, Deserialize)]
struct GhRelease {
    tag_name: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    published_at: Option<String>,
    #[serde(default)]
    html_url: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
}

/// Cuts `s` to at most `max` characters (at a character boundary), with an ellipsis.
pub fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_owned();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

/// Control characters other than newline and tab are dropped; CRLF becomes LF.
pub(crate) fn clean_text(s: &str) -> String {
    s.replace("\r\n", "\n")
        .chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .collect()
}

pub fn is_https_link(s: &str) -> bool {
    s.starts_with("https://")
        && s.len() <= 300
        && !s.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// `YYYY-MM-DD` with a plausible month and day, optionally followed by `T...`.
pub fn valid_date(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() < 10 || !s.is_char_boundary(10) {
        return false;
    }
    let (d, rest) = s.split_at(10);
    let ok = d.char_indices().all(|(i, c)| match i {
        4 | 7 => c == '-',
        _ => c.is_ascii_digit(),
    });
    if !ok || !(rest.is_empty() || rest.starts_with('T')) {
        return false;
    }
    let month: u32 = d[5..7].parse().unwrap_or(0);
    let day: u32 = d[8..10].parse().unwrap_or(0);
    (1..=12).contains(&month) && (1..=31).contains(&day)
}

fn valid_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

#[derive(Debug, Deserialize)]
struct RawItem {
    #[serde(default)]
    id: String,
    #[serde(default)]
    date: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    link: Option<String>,
}

/// Reads `news.json`. A broken item is skipped (and named in the second value), never the file.
pub fn parse_news_json(text: &str) -> Result<(Vec<NewsItem>, Vec<String>)> {
    if text.len() > MAX_NEWS_FILE {
        bail!("news.json is larger than {} KB", MAX_NEWS_FILE / 1024);
    }
    let v: serde_json::Value = serde_json::from_str(text).context("news.json is not valid JSON")?;
    let items = v
        .get("items")
        .and_then(|i| i.as_array())
        .ok_or_else(|| anyhow!("news.json has no \"items\" list"))?;
    if let Some(ver) = v.get("version").and_then(|x| x.as_u64())
        && ver != 1
    {
        bail!("news.json is version {ver}; this launcher reads version 1");
    }
    let (mut out, mut skipped) = (Vec::new(), Vec::new());
    for (i, raw) in items.iter().enumerate() {
        let r: RawItem = match serde_json::from_value(raw.clone()) {
            Ok(r) => r,
            Err(e) => {
                skipped.push(format!("item {i}: {e}"));
                continue;
            }
        };
        let title = clean_text(&r.title).trim().to_owned();
        if !valid_id(&r.id) {
            skipped.push(format!("item {i}: bad id"));
        } else if !valid_date(&r.date) {
            skipped.push(format!("{}: bad date (YYYY-MM-DD)", r.id));
        } else if title.is_empty() || title.chars().count() > MAX_TITLE {
            skipped.push(format!(
                "{}: title missing or longer than {MAX_TITLE}",
                r.id
            ));
        } else if out.iter().any(|o: &NewsItem| o.id == r.id) {
            skipped.push(format!("{}: duplicate id", r.id));
        } else {
            out.push(NewsItem {
                id: r.id,
                source: "project".into(),
                date: r.date,
                title,
                body: truncate(clean_text(&r.body).trim(), MAX_BODY),
                link: r.link.filter(|l| is_https_link(l)),
                version: None,
            });
        }
    }
    Ok((out, skipped))
}

/// The newest stable releases whose tag is `<prefix><digit>...`.
fn releases_to_items(src: &RepoSource, releases: Vec<GhRelease>) -> Vec<NewsItem> {
    let mut out = Vec::new();
    for r in releases.into_iter().filter(|r| !r.draft && !r.prerelease) {
        let Some(version) = r.tag_name.strip_prefix(src.prefix) else {
            continue;
        };
        if !version.starts_with(|c: char| c.is_ascii_digit()) {
            continue;
        }
        let Some(date) = r.published_at.filter(|d| valid_date(d)) else {
            continue;
        };
        let title = match r.name.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
            Some(n) => clean_text(n),
            None => format!("{} {version}", src.label),
        };
        out.push(NewsItem {
            id: format!("{}-{}", src.source, r.tag_name),
            source: src.source.to_owned(),
            date,
            title: truncate(&title, MAX_TITLE),
            body: truncate(
                clean_text(r.body.as_deref().unwrap_or("")).trim(),
                MAX_RELEASE_BODY,
            ),
            link: r.html_url.filter(|l| is_https_link(l)),
            version: Some(version.to_owned()),
        });
        if out.len() >= RELEASES_PER_REPO {
            break;
        }
    }
    out
}

/// Newest first; releases and announcements of the same day keep a stable order.
pub fn sort_items(items: &mut [NewsItem]) {
    items.sort_by(|a, b| (&b.date, &b.id).cmp(&(&a.date, &a.id)));
}

// ---------------------------------------------------------------------------------------------
// Fetching

fn client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(6))
        .timeout(Duration::from_secs(12))
        .user_agent(concat!("poe2-launcher/", env!("CARGO_PKG_VERSION")))
        .build()?)
}

async fn get_text(c: &reqwest::Client, url: &str, accept: &str, max: usize) -> Result<String> {
    let res = c
        .get(url)
        .header("Accept", accept)
        .send()
        .await
        .map_err(|e| anyhow!("cannot reach {}", e.without_url()))?;
    let status = res.status();
    if !status.is_success() {
        match status.as_u16() {
            403 | 429 => bail!("rate limited (HTTP {})", status.as_u16()),
            code => bail!("HTTP {code}"),
        }
    }
    if res.content_length().is_some_and(|l| l as usize > max) {
        bail!("the answer is larger than {} KB", max / 1024);
    }
    let bytes = res.bytes().await?;
    if bytes.len() > max {
        bail!("the answer is larger than {} KB", max / 1024);
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

async fn fetch_repo(
    c: &reqwest::Client,
    api_base: &str,
    src: &RepoSource,
) -> Result<Vec<NewsItem>> {
    let url = format!(
        "{}/repos/{}/releases?per_page=20",
        api_base.trim_end_matches('/'),
        src.repo
    );
    let text = get_text(c, &url, "application/vnd.github+json", 2 << 20).await?;
    let releases: Vec<GhRelease> = serde_json::from_str(&text).context("not a release list")?;
    Ok(releases_to_items(src, releases))
}

/// One fetch of every source. Returns the items of the sources that answered and the names of
/// the ones that failed.
pub async fn fetch_all(s: &NewsSources) -> (Vec<NewsItem>, Vec<&'static str>) {
    let Ok(c) = client() else {
        return (Vec::new(), vec!["launcher", "server", "hook", "project"]);
    };
    let (a, b, h, n) = tokio::join!(
        fetch_repo(&c, &s.api_base, &REPOS[0]),
        fetch_repo(&c, &s.api_base, &REPOS[1]),
        fetch_repo(&c, &s.api_base, &REPOS[2]),
        async {
            let t = get_text(&c, &s.news_url, "application/json", MAX_NEWS_FILE).await?;
            parse_news_json(&t).map(|(items, _)| items)
        }
    );
    let mut items = Vec::new();
    let mut failed = Vec::new();
    for (name, r) in [("launcher", a), ("server", b), ("hook", h), ("project", n)] {
        match r {
            Ok(mut v) => items.append(&mut v),
            Err(_) => failed.push(name),
        }
    }
    (items, failed)
}

// ---------------------------------------------------------------------------------------------
// Cache

pub fn load_cache(path: &Path) -> NewsCache {
    load_cache_as(path)
}

pub fn save_cache(path: &Path, cache: &NewsCache) {
    save_cache_as(path, cache)
}

/// A cache file as `T`; a missing or unreadable one is the default (a cache is only a copy).
pub fn load_cache_as<T: serde::de::DeserializeOwned + Default>(path: &Path) -> T {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn save_cache_as<T: Serialize>(path: &Path, cache: &T) {
    if let Ok(text) = serde_json::to_string(cache) {
        let _ = write_atomic(path, text.as_bytes());
    }
}

/// Folds a fetch into the cache: fresh items replace the old ones of the same source; a source
/// that failed keeps its saved items.
pub fn merge(cache: &NewsCache, fresh: Vec<NewsItem>, failed: &[&str], now: i64) -> NewsCache {
    let mut items = fresh;
    for old in &cache.items {
        if failed.contains(&old.source.as_str()) {
            items.push(old.clone());
        }
    }
    sort_items(&mut items);
    items.dedup_by(|a, b| a.id == b.id);
    items.truncate(MAX_ITEMS);
    let answered = failed.len() < 4;
    NewsCache {
        attempted_at: now,
        fetched_at: if answered { now } else { cache.fetched_at },
        items,
    }
}

impl Core {
    fn news_cache_file(&self) -> std::path::PathBuf {
        self.dirs.data.join("cache").join("news.json")
    }

    /// The news for Home. A fresh cache is returned as it is; otherwise the sources are asked
    /// (`force`: the user pressed refresh, but not more than once a minute). Never an error.
    pub async fn news_get(&self, force: bool) -> NewsFeed {
        self.news_get_from(&NewsSources::default(), force).await
    }

    pub async fn news_get_from(&self, sources: &NewsSources, force: bool) -> NewsFeed {
        let file = self.news_cache_file();
        let cache = load_cache(&file);
        let now = now_secs();
        let age = now - cache.attempted_at;
        let fresh = if force {
            age < FORCE_GAP
        } else {
            age < CACHE_TTL
        };
        if fresh && !cache.items.is_empty() {
            return NewsFeed {
                items: cache.items,
                fetched_at: cache.fetched_at,
                offline: cache.fetched_at < cache.attempted_at,
            };
        }
        let (items, failed) = fetch_all(sources).await;
        let merged = merge(&cache, items, &failed, now);
        save_cache(&file, &merged);
        NewsFeed {
            offline: failed.len() == 4,
            fetched_at: merged.fetched_at,
            items: merged.items,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gh(tag: &str, date: &str, pre: bool, draft: bool) -> GhRelease {
        GhRelease {
            tag_name: tag.into(),
            name: Some(format!("Release {tag}")),
            body: Some("- fixed things\r\n- **new** stuff".into()),
            published_at: Some(date.into()),
            html_url: Some(format!("https://github.com/x/y/releases/tag/{tag}")),
            draft,
            prerelease: pre,
        }
    }

    #[test]
    fn dates_are_checked() {
        assert!(valid_date("2026-10-22"));
        assert!(valid_date("2026-10-22T12:00:00Z"));
        assert!(!valid_date("2026-13-22"));
        assert!(!valid_date("22.10.2026"));
        assert!(!valid_date("2026-10-22 12:00"));
        assert!(!valid_date(""));
        assert!(!valid_date("2026-1é-22")); // multibyte near the cut
    }

    #[test]
    fn releases_become_items_by_tag_prefix() {
        let hook = &REPOS[2];
        let items = releases_to_items(
            hook,
            vec![
                gh("hook-v0.3.0", "2026-10-21T10:00:00Z", true, false), // pre-release
                gh("hook-v0.2.0", "2026-10-20T10:00:00Z", false, false),
                gh("server-v1.0.0", "2026-10-19T10:00:00Z", false, false), // other product
                gh("hook-vnext", "2026-10-19T10:00:00Z", false, false),    // not a version
                gh("hook-v0.1.0", "2026-10-18T10:00:00Z", false, true),    // draft
                gh("hook-v0.0.9", "2026-10-17T10:00:00Z", false, false),
            ],
        );
        let ids: Vec<_> = items.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, ["hook-hook-v0.2.0", "hook-hook-v0.0.9"]);
        assert_eq!(items[0].version.as_deref(), Some("0.2.0"));
        assert_eq!(items[0].source, "hook");
        assert!(
            items[0].body.contains("- fixed things\n- **new** stuff"),
            "{:?}",
            items[0].body
        );
        // The launcher's own tags are plain `v<version>`.
        let l = releases_to_items(
            &REPOS[0],
            vec![
                gh("v0.2.0", "2026-10-20", false, false),
                gh("hook-v1.0.0", "2026-10-20", false, false),
            ],
        );
        assert_eq!(l.len(), 1);
    }

    #[test]
    fn only_the_newest_few_releases_are_kept() {
        let rels = (0..10)
            .map(|i| gh(&format!("v1.0.{i}"), "2026-10-20", false, false))
            .collect();
        assert_eq!(releases_to_items(&REPOS[0], rels).len(), RELEASES_PER_REPO);
    }

    #[test]
    fn news_json_is_validated_item_by_item() {
        let (items, skipped) = parse_news_json(
            r#"{"version":1,"items":[
              {"id":"welcome","date":"2026-10-22","title":"Welcome","body":"Hi **there**","link":"https://example.org/a"},
              {"id":"bad link","date":"2026-10-22","title":"Spaces in the id"},
              {"id":"js","date":"2026-10-21","title":"Link kinds","link":"javascript:alert(1)"},
              {"id":"nodate","title":"No date"},
              {"id":"welcome","date":"2026-10-20","title":"Duplicate"},
              {"id":"long","date":"2026-10-20","title":"x","body":"BODYBODY"}
            ]}"#,
        )
        .unwrap();
        let ids: Vec<_> = items.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, ["welcome", "js", "long"]);
        assert_eq!(items[0].link.as_deref(), Some("https://example.org/a"));
        assert_eq!(items[1].link, None, "only https links survive");
        assert_eq!(items[0].source, "project");
        assert_eq!(skipped.len(), 3, "{skipped:?}");
    }

    #[test]
    fn broken_news_files_are_errors() {
        assert!(parse_news_json("not json").is_err());
        assert!(parse_news_json(r#"{"items": 3}"#).is_err());
        assert!(parse_news_json(r#"{"version":2,"items":[]}"#).is_err());
        assert!(
            parse_news_json(&format!(
                r#"{{"items":[],"pad":"{}"}}"#,
                "x".repeat(MAX_NEWS_FILE)
            ))
            .is_err()
        );
    }

    #[test]
    fn long_bodies_are_cut_and_control_characters_dropped() {
        let long = "ä".repeat(MAX_BODY + 50);
        let text = format!(
            r#"{{"items":[{{"id":"a","date":"2026-10-22","title":"T\u0007itle","body":"{long}"}}]}}"#
        );
        let (items, _) = parse_news_json(&text).unwrap();
        assert_eq!(items[0].title, "Title");
        assert_eq!(items[0].body.chars().count(), MAX_BODY + 1);
        assert!(items[0].body.ends_with('…'));
    }

    #[test]
    fn a_failed_source_keeps_its_saved_items() {
        let item = |id: &str, source: &str, date: &str| NewsItem {
            id: id.into(),
            source: source.into(),
            date: date.into(),
            title: id.into(),
            body: String::new(),
            link: None,
            version: None,
        };
        let cache = NewsCache {
            attempted_at: 10,
            fetched_at: 10,
            items: vec![
                item("old-hook", "hook", "2026-10-01"),
                item("old-server", "server", "2026-10-02"),
            ],
        };
        let m = merge(
            &cache,
            vec![item("new-server", "server", "2026-10-20")],
            &["hook", "launcher", "project"],
            99,
        );
        let ids: Vec<_> = m.items.iter().map(|i| i.id.as_str()).collect();
        // The server answered (its old item is gone), the hook failed (its old item stays).
        assert_eq!(ids, ["new-server", "old-hook"]);
        assert_eq!((m.attempted_at, m.fetched_at), (99, 99));
        // Everything failed: the old copy is all there is, and "fetched" does not move.
        let all_failed = merge(
            &cache,
            vec![],
            &["launcher", "server", "hook", "project"],
            99,
        );
        assert_eq!(all_failed.items.len(), 2);
        assert_eq!((all_failed.attempted_at, all_failed.fetched_at), (99, 10));
    }
}
