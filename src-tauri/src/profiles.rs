//! Server profiles: a name and the cluster's web API base URL. The login token of a profile is
//! kept in the keyring ([`crate::keystore`]), keyed by the profile id.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::{keystore::TokenStore, paths::write_atomic};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Profile {
    /// Stable key (`p1`, `p2`, ...); also the keyring user name.
    pub id: String,
    pub name: String,
    /// Web API base without `/api/v1`, e.g. `http://127.0.0.1:8080`.
    pub url: String,
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
        profile.url = url;
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

/// Turns what the user typed into the API base URL: adds `http://` to a bare `host:port`,
/// drops a trailing slash and a pasted `/api/v1`, and refuses anything but http(s) with a host.
pub fn normalize_base_url(input: &str) -> Result<String> {
    let mut s = input.trim().to_owned();
    if s.is_empty() {
        bail!("the server URL is empty");
    }
    if !s.contains("://") {
        s = format!("http://{s}");
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keystore::{MemoryStore, StoredSession};

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
