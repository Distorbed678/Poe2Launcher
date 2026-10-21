//! Where login tokens live: the operating system's keyring, never a plain file.
//!
//! [`KeyringStore`] uses the `keyring` crate (Secret Service on Linux, Credential Manager on
//! Windows, Keychain on macOS). [`MemoryStore`] is the same interface for tests.

use std::{collections::HashMap, sync::Mutex};

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

/// Service name in the keyring.
pub const KEYRING_SERVICE: &str = "poe2-launcher";

/// What is kept for a signed-in profile: the launcher token and who it belongs to.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredSession {
    /// 64 hex characters. A secret.
    pub token: String,
    pub account: String,
    /// Unix seconds.
    pub expires_at: i64,
    /// `player` or `admin`, from the login and refreshed with `GET /me`. Old entries have none.
    #[serde(default)]
    pub role: Option<String>,
}

impl std::fmt::Debug for StoredSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoredSession")
            .field("token", &"<redacted>")
            .field("account", &self.account)
            .field("expires_at", &self.expires_at)
            .field("role", &self.role)
            .finish()
    }
}

pub trait TokenStore: Send + Sync {
    fn get(&self, profile_id: &str) -> Result<Option<StoredSession>>;
    fn set(&self, profile_id: &str, session: &StoredSession) -> Result<()>;
    /// Deleting something that is not there is fine.
    fn delete(&self, profile_id: &str) -> Result<()>;
}

/// The OS keyring.
pub struct KeyringStore {
    service: String,
}

impl KeyringStore {
    pub fn new() -> Self {
        Self {
            service: KEYRING_SERVICE.into(),
        }
    }

    pub fn with_service(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    fn entry(&self, profile_id: &str) -> Result<keyring::Entry> {
        keyring::Entry::new(&self.service, profile_id).map_err(explain)
    }
}

impl Default for KeyringStore {
    fn default() -> Self {
        Self::new()
    }
}

fn explain(e: keyring::Error) -> anyhow::Error {
    match e {
        keyring::Error::NoStorageAccess(_) | keyring::Error::PlatformFailure(_) => anyhow!(
            "the system keyring is not available ({e}). The launcher keeps login tokens only \
             in the OS keyring (Secret Service on Linux: GNOME Keyring or KWallet), never in \
             plain files. Start a Secret Service provider and unlock it (GNOME Keyring; on KDE \
             enable the Secret Service interface of KWallet in System Settings), then log in again"
        ),
        other => anyhow!("keyring: {other}"),
    }
}

impl TokenStore for KeyringStore {
    fn get(&self, profile_id: &str) -> Result<Option<StoredSession>> {
        match self.entry(profile_id)?.get_password() {
            Ok(text) => Ok(serde_json::from_str(&text).ok()),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(explain(e)),
        }
    }

    fn set(&self, profile_id: &str, session: &StoredSession) -> Result<()> {
        let text = serde_json::to_string(session)?;
        self.entry(profile_id)?.set_password(&text).map_err(explain)
    }

    fn delete(&self, profile_id: &str) -> Result<()> {
        match self.entry(profile_id)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(explain(e)),
        }
    }
}

/// A keyring stand-in that keeps everything in memory.
#[derive(Default)]
pub struct MemoryStore {
    map: Mutex<HashMap<String, StoredSession>>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.map.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl TokenStore for MemoryStore {
    fn get(&self, profile_id: &str) -> Result<Option<StoredSession>> {
        Ok(self.map.lock().unwrap().get(profile_id).cloned())
    }
    fn set(&self, profile_id: &str, session: &StoredSession) -> Result<()> {
        self.map
            .lock()
            .unwrap()
            .insert(profile_id.into(), session.clone());
        Ok(())
    }
    fn delete(&self, profile_id: &str) -> Result<()> {
        self.map.lock().unwrap().remove(profile_id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(t: &str) -> StoredSession {
        StoredSession {
            token: t.repeat(64),
            account: "alice".into(),
            expires_at: 1_900_000_000,
            role: None,
        }
    }

    #[test]
    fn memory_store_roundtrip() {
        let s = MemoryStore::new();
        assert_eq!(s.get("p1").unwrap(), None);
        s.set("p1", &session("a")).unwrap();
        s.set("p2", &session("b")).unwrap();
        assert_eq!(s.get("p1").unwrap().unwrap().token, "a".repeat(64));
        s.delete("p1").unwrap();
        s.delete("p1").unwrap();
        assert_eq!(s.get("p1").unwrap(), None);
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn debug_output_hides_the_token() {
        let text = format!("{:?}", session("c"));
        assert!(!text.contains(&"c".repeat(64)));
        assert!(text.contains("redacted"));
    }
}
