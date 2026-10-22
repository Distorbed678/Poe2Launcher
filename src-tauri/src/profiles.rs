//! Server profiles: a name and the cluster's web API base URL. The login token of a profile is
//! kept in the keyring ([`crate::keystore`]), keyed by the profile id.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::{keystore::TokenStore, mods::ModPin, paths::write_atomic};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Profile {
    /// Stable key (`p1`, `p2`, ...); also the keyring user name.
    pub id: String,
    pub name: String,
    /// Web API base without `/api/v1`, e.g. `http://127.0.0.1:8080`.
    pub url: String,
    /// The user confirmed sending the password and the token over plain `http://` to this
    /// (non-loopback) address. Reset when the address changes.
    #[serde(default)]
    pub plain_http_ok: bool,
    /// The mods the player enabled for this server (the server's required mods are added on
    /// top at Play and are not stored here). Exact versions from the mod cache, in load order.
    #[serde(default)]
    pub mods: Vec<ModPin>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct File {
    profiles: Vec<Profile>,
}

/// `profiles.json`. Holds no secrets.
pub struct ProfileStore {
    path: PathBuf,
}

impl ProfileStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn list(&self) -> Result<Vec<Profile>> {
        match std::fs::read_to_string(&self.path) {
            Ok(text) => Ok(serde_json::from_str::<File>(&text)
                .with_context(|| format!("{} is not valid profiles JSON", self.path.display()))?
                .profiles),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(e).with_context(|| format!("reading {}", self.path.display())),
        }
    }

    pub fn get(&self, id: &str) -> Result<Profile> {
        self.list()?
            .into_iter()
            .find(|p| p.id == id)
            .with_context(|| format!("no server profile {id:?}"))
    }

    fn save(&self, profiles: Vec<Profile>) -> Result<()> {
        write_atomic(
            &self.path,
            serde_json::to_string_pretty(&File { profiles })?.as_bytes(),
        )
    }

    pub fn add(&self, name: &str, url: &str) -> Result<Profile> {
        let name = clean_name(name)?;
        let url = normalize_base_url(url)?;
        let mut all = self.list()?;
        if all.iter().any(|p| p.name.eq_ignore_ascii_case(&name)) {
            bail!("a server profile named {name:?} already exists");
        }
        let next = all
            .iter()
            .filter_map(|p| p.id.strip_prefix('p')?.parse::<u32>().ok())
            .max()
            .unwrap_or(0)
            + 1;
        let profile = Profile {
            id: format!("p{next}"),
            name,
            url,
            plain_http_ok: false,
            mods: Vec::new(),
        };
        all.push(profile.clone());
        self.save(all)?;
        Ok(profile)
    }

    pub fn update(&self, id: &str, name: &str, url: &str) -> Result<Profile> {
        let name = clean_name(name)?;
        let url = normalize_base_url(url)?;
        let mut all = self.list()?;
        if all
            .iter()
            .any(|p| p.id != id && p.name.eq_ignore_ascii_case(&name))
        {
            bail!("a server profile named {name:?} already exists");
        }
        let profile = all
            .iter_mut()
            .find(|p| p.id == id)
            .with_context(|| format!("no server profile {id:?}"))?;
        profile.name = name;
        if profile.url != url {
            // A confirmation is for one address.
            profile.plain_http_ok = false;
        }
        profile.url = url;
        let out = profile.clone();
        self.save(all)?;
        Ok(out)
    }

    /// Replaces the mods enabled for this profile.
    pub fn set_mods(&self, id: &str, mods: Vec<ModPin>) -> Result<Profile> {
        let mut all = self.list()?;
        let profile = all
            .iter_mut()
            .find(|p| p.id == id)
            .with_context(|| format!("no server profile {id:?}"))?;
        profile.mods = mods;
        let out = profile.clone();
        self.save(all)?;
        Ok(out)
    }

    /// Records (or withdraws) the user's confirmation for plain `http://`.
    pub fn set_plain_http_ok(&self, id: &str, ok: bool) -> Result<Profile> {
        let mut all = self.list()?;
        let profile = all
            .iter_mut()
            .find(|p| p.id == id)
            .with_context(|| format!("no server profile {id:?}"))?;
        profile.plain_http_ok = ok;
        let out = profile.clone();
        self.save(all)?;
        Ok(out)
    }

    /// Removes the profile and its stored login (the token is only forgotten here; call the
    /// server's logout first to revoke it).
    pub fn remove(&self, id: &str, tokens: &dyn TokenStore) -> Result<()> {
        let mut all = self.list()?;
        let before = all.len();
        all.retain(|p| p.id != id);
        if all.len() == before {
            bail!("no server profile {id:?}");
        }
        tokens.delete(id)?;
        self.save(all)
    }
}

fn clean_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 64 || name.chars().any(char::is_control) {
        bail!("a profile name is 1-64 characters without control characters");
    }
    Ok(name.to_owned())
}

/// Turns what the user typed into the API base URL: adds a scheme to a bare `host:port`
/// (`http://` for this computer and private networks, `https://` for everything else), drops
/// a trailing slash and a pasted `/api/v1`, and refuses anything but http(s) with a host.
pub fn normalize_base_url(input: &str) -> Result<String> {
    let mut s = input.trim().to_owned();
    if s.is_empty() {
        bail!("the server URL is empty");
    }
    if !s.contains("://") {
        let host = url_host(&format!("x://{s}")).unwrap_or_default();
        let scheme = if is_private_host(&host) {
            "http"
        } else {
            "https"
        };
        s = format!("{scheme}://{s}");
    }
    let (scheme, rest) = s.split_once("://").unwrap();
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        bail!("the server URL must start with http:// or https://");
    }
    let rest = rest.trim_end_matches('/');
    let rest = rest.strip_suffix("/api/v1").unwrap_or(rest);
    let rest = rest.trim_end_matches('/');
    let host_part = rest.split('/').next().unwrap_or("");
    if host_part.is_empty() || host_part.contains(char::is_whitespace) || host_part.starts_with(':')
    {
        bail!("the server URL has no host");
    }
    if rest.contains(['?', '#']) {
        bail!("the server URL must not carry a query or fragment");
    }
    Ok(format!("{scheme}://{rest}"))
}

/// Host part (without port) of an http(s) base URL.
pub fn url_host(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let authority = rest.split('/').next()?;
    let authority = authority.rsplit('@').next()?;
    if let Some(v6) = authority.strip_prefix('[') {
        return Some(v6.split(']').next()?.to_owned());
    }
    Some(authority.split(':').next()?.to_owned())
}

/// How exposed a plain `http://` address is (`Https` is the value for an address that is not
/// plain http at all).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PlainHttp {
    /// An https:// address.
    Https,
    /// This computer: nothing travels on a network.
    Loopback,
    /// A private or local-network address: readable by anyone on that network.
    Private,
    /// Anywhere else: readable on the way across the internet.
    Public,
}

/// This computer (`localhost`, `127.0.0.0/8`, `::1`).
pub fn is_loopback_host(host: &str) -> bool {
    let h = host.trim().trim_end_matches('.').to_ascii_lowercase();
    if h == "localhost" || h.ends_with(".localhost") {
        return true;
    }
    match h.parse::<std::net::IpAddr>() {
        Ok(ip) => ip.is_loopback(),
        Err(_) => false,
    }
}

/// Loopback, or an address on a local network: RFC 1918, link-local, unique-local IPv6,
/// `*.local` / `*.lan` names and single-label names.
pub fn is_private_host(host: &str) -> bool {
    if is_loopback_host(host) {
        return true;
    }
    let h = host.trim().trim_end_matches('.').to_ascii_lowercase();
    match h.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(ip)) => ip.is_private() || ip.is_link_local(),
        Ok(std::net::IpAddr::V6(ip)) => {
            let seg = ip.segments()[0];
            (seg & 0xfe00) == 0xfc00 || (seg & 0xffc0) == 0xfe80
        }
        Err(_) => {
            !h.contains('.')
                || h.ends_with(".local")
                || h.ends_with(".lan")
                || h.ends_with(".home.arpa")
        }
    }
}

/// Whether sending a password or token to `url` would be in clear text, and how exposed it is.
/// `None` for https.
pub fn plain_http(url: &str) -> Option<PlainHttp> {
    if !url.to_ascii_lowercase().starts_with("http://") {
        return None;
    }
    let host = url_host(url)?;
    Some(if is_loopback_host(&host) {
        PlainHttp::Loopback
    } else if is_private_host(&host) {
        PlainHttp::Private
    } else {
        PlainHttp::Public
    })
}

/// True when the address needs the user's confirmation before a password or token is sent:
/// plain `http://` to anything but this computer.
pub fn needs_plain_http_confirmation(url: &str) -> bool {
    matches!(
        plain_http(url),
        Some(PlainHttp::Private | PlainHttp::Public)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keystore::{MemoryStore, StoredSession};

    #[test]
    fn plain_http_is_graded_by_where_the_address_is() {
        assert_eq!(plain_http("https://play.example"), None);
        assert_eq!(plain_http("HTTPS://192.168.1.2"), None);
        for loopback in [
            "http://127.0.0.1:8080",
            "http://localhost:8080",
            "http://[::1]:8080",
            "http://127.5.5.5",
        ] {
            assert_eq!(
                plain_http(loopback),
                Some(PlainHttp::Loopback),
                "{loopback}"
            );
            assert!(!needs_plain_http_confirmation(loopback));
        }
        for private in [
            "http://192.168.1.5:8080",
            "http://10.0.0.2",
            "http://172.16.3.4",
            "http://169.254.1.1",
            "http://nas.local",
            "http://myserver",
            "http://[fd00::1]",
        ] {
            assert_eq!(plain_http(private), Some(PlainHttp::Private), "{private}");
            assert!(needs_plain_http_confirmation(private));
        }
        for public in [
            "http://203.0.113.7:8080",
            "http://play.example",
            "http://172.32.0.1",
            "http://8.8.8.8",
        ] {
            assert_eq!(plain_http(public), Some(PlainHttp::Public), "{public}");
            assert!(needs_plain_http_confirmation(public));
        }
    }

    #[test]
    fn a_bare_address_gets_https_unless_it_is_local() {
        let ok = |i: &str, o: &str| assert_eq!(normalize_base_url(i).unwrap(), o, "{i}");
        ok("127.0.0.1:8080", "http://127.0.0.1:8080");
        ok("localhost:8080", "http://localhost:8080");
        ok("192.168.1.9:8080", "http://192.168.1.9:8080");
        ok("play.example", "https://play.example");
        ok("203.0.113.7:8080", "https://203.0.113.7:8080");
        // An explicit http:// is kept: the confirmation, not the parser, guards it.
        ok("http://203.0.113.7:8080", "http://203.0.113.7:8080");
    }

    #[test]
    fn a_confirmation_belongs_to_one_address() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ProfileStore::new(tmp.path().join("profiles.json"));
        let p = store.add("Far", "http://203.0.113.7:8080").unwrap();
        assert!(!p.plain_http_ok);
        assert!(store.set_plain_http_ok(&p.id, true).unwrap().plain_http_ok);
        // Renaming keeps it, a new address drops it.
        assert!(
            store
                .update(&p.id, "Far 2", "http://203.0.113.7:8080")
                .unwrap()
                .plain_http_ok
        );
        assert!(
            !store
                .update(&p.id, "Far 2", "http://203.0.113.8:8080")
                .unwrap()
                .plain_http_ok
        );
        assert!(store.set_plain_http_ok("nope", true).is_err());
        // Old files without the field still load.
        std::fs::write(
            store.path(),
            r#"{"profiles":[{"id":"p1","name":"x","url":"http://h:1"}]}"#,
        )
        .unwrap();
        assert!(!store.list().unwrap()[0].plain_http_ok);
    }

    #[test]
    fn urls_are_normalised() {
        let ok = |i: &str, o: &str| assert_eq!(normalize_base_url(i).unwrap(), o, "{i}");
        ok("127.0.0.1:8080", "http://127.0.0.1:8080");
        ok("  http://play.example/ ", "http://play.example");
        ok(
            "HTTPS://play.example:8443/api/v1/",
            "https://play.example:8443",
        );
        ok("http://h/admin", "http://h/admin");
        for bad in [
            "",
            "ftp://x",
            "http://",
            "http:///x",
            "http://a b",
            "http://h?x=1",
        ] {
            assert!(normalize_base_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn hosts_are_extracted() {
        assert_eq!(
            url_host("http://play.example:8080").as_deref(),
            Some("play.example")
        );
        assert_eq!(url_host("https://u:p@h/x").as_deref(), Some("h"));
        assert_eq!(url_host("http://[::1]:8080").as_deref(), Some("::1"));
        assert_eq!(url_host("nonsense"), None);
    }

    #[test]
    fn profiles_are_stored_and_removed_with_their_token() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ProfileStore::new(tmp.path().join("profiles.json"));
        let tokens = MemoryStore::new();
        assert!(store.list().unwrap().is_empty());

        let a = store.add("Home", "127.0.0.1:8080").unwrap();
        let b = store.add("Friends", "https://friends.example").unwrap();
        assert_eq!((a.id.as_str(), b.id.as_str()), ("p1", "p2"));
        assert!(store.add("home", "x:1").is_err(), "names are unique");
        assert!(store.add("Bad", "ftp://x").is_err());

        tokens
            .set(
                &a.id,
                &StoredSession {
                    token: "t".repeat(64),
                    account: "alice".into(),
                    expires_at: 1,
                    role: None,
                },
            )
            .unwrap();
        let renamed = store.update(&a.id, "Home 2", "localhost:9000").unwrap();
        assert_eq!(renamed.url, "http://localhost:9000");
        assert_eq!(store.get("p1").unwrap().name, "Home 2");

        store.remove(&a.id, &tokens).unwrap();
        assert!(
            tokens.get(&a.id).unwrap().is_none(),
            "the token went with the profile"
        );
        assert!(store.remove(&a.id, &tokens).is_err());
        // Ids are not reused while a higher one exists.
        assert_eq!(store.add("Third", "h:1").unwrap().id, "p3");
        // The file holds no token.
        let text = std::fs::read_to_string(store.path()).unwrap();
        assert!(!text.contains("tttt"));
    }
}
