//! The launcher's view of a mod. The manifest itself is parsed by `poe-hook-mods` (manifest v1
//! and v2, the same crate the hook's loader, the server and the SDK's packer use); this module
//! adds what only the launcher needs: compatibility against the selected game version and the
//! installed hook, the load-order resolver, the pin a profile stores, and the JSON view of a
//! manifest for the UI.
//!
//! Where mods live is in [`crate::modcache`] (verified packages, several versions per id) and
//! [`crate::modsets`] (which of them a server profile loads, written into the game's
//! `poe-hook-mods/` at Play). Nothing here loads a mod.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::{Deserialize, Serialize};

pub use poe_hook_mods::{
    HOOK_API, MANIFEST_FILE, MANIFEST_VERSION, MODS_DIR, Manifest, STATE_FILE, Side,
    parse_manifest, valid_id,
};

/// One enabled mod of a mod set: an exact version (several can be cached side by side).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModPin {
    pub id: String,
    pub version: String,
    /// The user chose to load it although it is made for another game version.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub force: bool,
}

impl ModPin {
    pub fn new(id: &str, version: &str) -> Self {
        Self {
            id: id.into(),
            version: version.into(),
            force: false,
        }
    }
}

/// The manifest as the UI sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModManifestView {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub authors: Vec<String>,
    pub license: String,
    pub homepage: String,
    /// `client`, `server` or `both`.
    pub side: String,
    pub game_versions: Vec<String>,
    pub hook_api: Option<u32>,
    pub server_api: Option<u32>,
    pub after: Vec<String>,
    pub before: Vec<String>,
    pub conflicts: Vec<String>,
    /// id -> semver requirement text.
    pub dependencies: BTreeMap<String, String>,
    /// The client DLL: a mod with one runs native code inside the game process.
    pub entry_dll: Option<String>,
    pub has_code: bool,
}

impl From<&Manifest> for ModManifestView {
    fn from(m: &Manifest) -> Self {
        Self {
            id: m.id.clone(),
            name: m.name.clone(),
            version: m.version.clone(),
            description: m.description.clone(),
            authors: m.authors.clone(),
            license: m.license.clone(),
            homepage: m.homepage.clone(),
            side: m.side.as_str().into(),
            game_versions: m.game_versions.clone(),
            hook_api: m.hook_api,
            server_api: m.server_api,
            after: m.after.clone(),
            before: m.before.clone(),
            conflicts: m.conflicts.clone(),
            dependencies: m.dependencies.clone(),
            entry_dll: m.entry_dll.clone(),
            has_code: has_code(m),
        }
    }
}

/// Whether the client half runs native code (a DLL in the game process).
pub fn has_code(m: &Manifest) -> bool {
    m.entry_dll.is_some()
}

/// The message for a mod that has nothing for the game. The launcher's mod store holds the
/// client half only.
pub fn server_only_message(id: &str) -> String {
    format!(
        "{id} is a server-only mod (side = \"server\"): it has no client half, so it does not \
         belong in the launcher's mod store. Install it into a server's mods folder instead \
         (poe-server mod install)"
    )
}

/// Refuses a mod without a client half.
pub fn ensure_client_half(m: &Manifest) -> Result<(), String> {
    if m.side.includes_client() {
        Ok(())
    } else {
        Err(server_only_message(&m.id))
    }
}

/// Paths (files or folders, relative to the mod folder) that belong to the server half of a
/// mod. Only a `both` mod has any: a client mod's files are all client files. The conventional
/// `server/` folder is left out too, whatever the manifest names.
pub fn server_half_paths(m: &Manifest) -> Vec<String> {
    if m.side != Side::Both {
        return Vec::new();
    }
    let mut v: Vec<String> = Vec::new();
    v.extend(m.server_linux.iter().cloned());
    v.extend(m.server_windows.iter().cloned());
    v.extend(m.server_tables.iter().cloned());
    v.extend(m.server_opcodes.iter().cloned());
    v.extend(m.server_config_defaults.iter().cloned());
    v.push("server".into());
    v.sort();
    v.dedup();
    v
}

/// Whether `rel` (a `/`-separated path inside the mod) is part of the server half.
pub fn is_server_path(server_paths: &[String], rel: &str) -> bool {
    server_paths.iter().any(|s| {
        rel == s
            || rel
                .strip_prefix(s.as_str())
                .is_some_and(|r| r.starts_with('/'))
    })
}

/// The files of the client half that the manifest names and must exist (the check that a
/// cached copy is whole).
pub fn client_files_missing(m: &Manifest, exists: impl Fn(&str) -> bool) -> Vec<String> {
    let mut missing = Vec::new();
    let mut check = |what: &str, p: &str| {
        if !exists(p) {
            missing.push(format!("{what} {p:?} is missing"));
        }
    };
    if let Some(d) = &m.entry_dll {
        check("entry dll", d);
    }
    for p in m.data_tables.iter().chain(&m.data_opcodes) {
        check("data path", p);
    }
    if let Some(c) = &m.config_defaults {
        check("config_defaults", c);
    }
    missing
}

// ---------------------------------------------------------------------------------------------
// Compatibility

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Compat {
    Ok,
    /// The manifest names no client versions: assumed fine.
    Unspecified,
    WrongGame {
        supported: Vec<String>,
    },
    /// Built for a newer mod ABI than the installed hook offers.
    HookApiTooNew {
        needs: u32,
        have: u32,
    },
    /// Built for an older ABI: the loader offers exactly one, so it has to be rebuilt.
    HookApiOld {
        needs: u32,
        have: u32,
    },
    /// A DLL that does not say which ABI it was built against.
    HookApiMissing {
        have: u32,
    },
}

impl Compat {
    /// Whether enabling needs the user's override. ABI problems cannot be overridden: the
    /// loader refuses such a mod anyway.
    pub fn blocks(&self) -> bool {
        !matches!(self, Compat::Ok | Compat::Unspecified)
    }

    /// Whether the user's override helps (a game version mismatch is a warning in the loader).
    pub fn overridable(&self) -> bool {
        matches!(self, Compat::WrongGame { .. })
    }
}

/// `hook_abi` is the mod ABI the installed hook offers (from its `release.json`; the ABI this
/// launcher was built against when no hook is known).
pub fn compat(m: &Manifest, game_version: &str, hook_abi: u32) -> Compat {
    match m.hook_api {
        Some(need) if need > hook_abi => {
            return Compat::HookApiTooNew {
                needs: need,
                have: hook_abi,
            };
        }
        Some(need) if need < hook_abi => {
            return Compat::HookApiOld {
                needs: need,
                have: hook_abi,
            };
        }
        None if has_code(m) => return Compat::HookApiMissing { have: hook_abi },
        _ => {}
    }
    game_compat(&m.game_versions, game_version)
}

/// The game-version half of [`compat`], for a version list of an advertisement or the index.
pub fn game_compat(game_versions: &[String], game_version: &str) -> Compat {
    if game_versions.is_empty() {
        Compat::Unspecified
    } else if game_versions.iter().any(|v| v == game_version) {
        Compat::Ok
    } else {
        Compat::WrongGame {
            supported: game_versions.to_vec(),
        }
    }
}

/// [`compat`] for a listing that only knows the version lists and the ABI a release states
/// (the index): no manifest to read.
pub fn compat_for_listing(
    game_versions: &[String],
    hook_api: Option<u32>,
    game_version: &str,
    hook_abi: u32,
) -> Compat {
    match hook_api {
        Some(need) if need > hook_abi => Compat::HookApiTooNew {
            needs: need,
            have: hook_abi,
        },
        Some(need) if need < hook_abi => Compat::HookApiOld {
            needs: need,
            have: hook_abi,
        },
        _ => game_compat(game_versions, game_version),
    }
}

pub fn describe_compat(c: &Compat) -> String {
    match c {
        Compat::Ok => "compatible".into(),
        Compat::Unspecified => "no client version stated".into(),
        Compat::WrongGame { supported } => format!("made for {}", supported.join(", ")),
        Compat::HookApiTooNew { needs, have } => {
            format!("needs mod ABI {needs}, the installed hook offers {have} (update the hook)")
        }
        Compat::HookApiOld { needs, have } => {
            format!("built for mod ABI {needs}, the installed hook offers {have} (needs a rebuild)")
        }
        Compat::HookApiMissing { have } => {
            format!("a DLL without game.hook_api (the hook offers ABI {have})")
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Load order

/// Topological order of `ids` by `dependencies`, `load.after` and `load.before` (only among
/// the given mods). Among mods that are free to go, the one earliest in `preferred` goes first
/// (mods not in `preferred` sort by id after those). A cycle is an error naming the mods in it.
pub fn resolve_order(
    mods: &BTreeMap<String, &Manifest>,
    ids: &BTreeSet<String>,
    preferred: &[String],
) -> Result<Vec<String>, Vec<String>> {
    let rank = |id: &str| -> (usize, String) {
        (
            preferred.iter().position(|p| p == id).unwrap_or(usize::MAX),
            id.to_owned(),
        )
    };
    // before[x] = mods that must load before x
    let mut before: HashMap<&str, BTreeSet<&str>> =
        ids.iter().map(|i| (i.as_str(), BTreeSet::new())).collect();
    for id in ids {
        let Some(m) = mods.get(id) else { continue };
        let me = id.as_str();
        for dep in m.dependencies.keys().chain(&m.after) {
            if let Some((k, _)) = before.get_key_value(dep.as_str()) {
                let k = *k;
                before.get_mut(me).unwrap().insert(k);
            }
        }
        for later in &m.before {
            if let Some(set) = before.get_mut(later.as_str()) {
                set.insert(me);
            }
        }
    }
    let mut done: Vec<String> = Vec::new();
    let mut remaining: BTreeSet<&str> = ids.iter().map(String::as_str).collect();
    while !remaining.is_empty() {
        let next = remaining
            .iter()
            .filter(|id| before[*id].iter().all(|b| !remaining.contains(b)))
            .min_by_key(|id| rank(id))
            .copied();
        match next {
            Some(id) => {
                remaining.remove(id);
                done.push(id.to_owned());
            }
            None => return Err(remaining.iter().map(|s| (*s).to_owned()).collect()),
        }
    }
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn manifest_text(id: &str, version: &str, extra: &str) -> String {
        format!(
            "manifest_version = 1\nid = \"{id}\"\nname = \"{id}\"\nversion = \"{version}\"\n{extra}\n[data]\ntables = [\"data\"]\n"
        )
    }

    fn parse(text: &str) -> Manifest {
        parse_manifest(text).unwrap().manifest
    }

    #[test]
    fn the_shared_parser_reads_v1_and_v2() {
        let v1 = parse(&manifest_text(
            "a-mod",
            "1.0.0",
            "[game]\nversions=[\"0.5.5e.hotfix1\"]\nhook_api=1\n[entry]\ndll=\"a.dll\"\n",
        ));
        assert_eq!(v1.side, Side::Client);
        let v2 = parse(
            "manifest_version = 2\nid=\"b-mod\"\nname=\"b\"\nversion=\"1.0.0\"\nside=\"both\"\n[client]\ndll=\"client/b.dll\"\nhook_api=1\n[server]\nserver_api=1\n[server.library]\nlinux=\"server/libb.so\"\n",
        );
        assert_eq!(v2.side, Side::Both);
        assert!(ensure_client_half(&v2).is_ok());
        let only = parse(
            "manifest_version = 2\nid=\"c-mod\"\nname=\"c\"\nversion=\"1.0.0\"\nside=\"server\"\n[server]\nserver_api=1\n[server.library]\nlinux=\"server/libc.so\"\n",
        );
        let why = ensure_client_half(&only).unwrap_err();
        assert!(why.contains("server-only") && why.contains("poe-server mod install"));
        // A reader that only knows v1 would have refused it: the shared crate does not.
        assert!(
            parse_manifest("manifest_version = 3\nid=\"x1\"\nname=\"x\"\nversion=\"1.0.0\"\n")
                .is_err()
        );
    }

    #[test]
    fn the_server_half_of_a_both_mod_is_identified() {
        let m = parse(
            "manifest_version = 2\nid=\"b-mod\"\nname=\"b\"\nversion=\"1.0.0\"\nside=\"both\"\n[client]\ndll=\"client/b.dll\"\nhook_api=1\ntables=[\"data/c\"]\n[server]\nserver_api=1\ntables=[\"srv-data\"]\n[server.library]\nlinux=\"lib/libb.so\"\n",
        );
        let sp = server_half_paths(&m);
        assert!(is_server_path(&sp, "lib/libb.so"));
        assert!(is_server_path(&sp, "srv-data/t.dat"));
        assert!(is_server_path(&sp, "server/anything"));
        assert!(!is_server_path(&sp, "client/b.dll"));
        assert!(!is_server_path(&sp, "data/c/t.dat"));
        assert!(
            !is_server_path(&sp, "servers.txt"),
            "a prefix of the name is not the folder"
        );
        let client = parse(&manifest_text("a-mod", "1.0.0", ""));
        assert!(server_half_paths(&client).is_empty());
    }

    #[test]
    fn compatibility_follows_game_version_and_the_hooks_abi() {
        let m = parse(&manifest_text(
            "a-mod",
            "1.0.0",
            "[game]\nversions=[\"0.5.5e.hotfix1\"]\nhook_api=1\n[entry]\ndll=\"a.dll\"\n",
        ));
        assert_eq!(compat(&m, "0.5.5e.hotfix1", 1), Compat::Ok);
        assert_eq!(
            compat(&m, "0.4.0", 1),
            Compat::WrongGame {
                supported: vec!["0.5.5e.hotfix1".into()]
            }
        );
        assert_eq!(
            compat(&m, "0.5.5e.hotfix1", 2),
            Compat::HookApiOld { needs: 1, have: 2 }
        );
        let newer = parse(&manifest_text(
            "n-mod",
            "1.0.0",
            "[game]\nhook_api=3\n[entry]\ndll=\"a.dll\"\n",
        ));
        assert_eq!(
            compat(&newer, "0.5.5e.hotfix1", 1),
            Compat::HookApiTooNew { needs: 3, have: 1 }
        );
        // v1 leniency is gone: the loader refuses a DLL without an ABI, so the launcher does.
        let none = parse(&manifest_text("d-mod", "1.0.0", "[entry]\ndll=\"a.dll\"\n"));
        assert_eq!(
            compat(&none, "0.5.5e.hotfix1", 1),
            Compat::HookApiMissing { have: 1 }
        );
        let data = parse(&manifest_text("e-mod", "1.0.0", ""));
        assert_eq!(compat(&data, "0.5.5e.hotfix1", 1), Compat::Unspecified);
        assert!(compat(&m, "0.4.0", 1).overridable());
        assert!(!Compat::HookApiOld { needs: 1, have: 2 }.overridable());
        assert!(
            describe_compat(&Compat::HookApiTooNew { needs: 3, have: 1 })
                .contains("update the hook")
        );
    }

    fn mk(id: &str, deps: &[&str], after: &[&str], before: &[&str]) -> Manifest {
        let mut m = parse(&manifest_text(id, "1.0.0", ""));
        m.dependencies = deps.iter().map(|d| ((*d).into(), "*".into())).collect();
        m.after = after.iter().map(|s| (*s).into()).collect();
        m.before = before.iter().map(|s| (*s).into()).collect();
        m
    }

    fn order(ms: &[Manifest], preferred: &[&str]) -> Result<Vec<String>, Vec<String>> {
        let by: BTreeMap<String, &Manifest> = ms.iter().map(|m| (m.id.clone(), m)).collect();
        let ids: BTreeSet<String> = by.keys().cloned().collect();
        let pref: Vec<String> = preferred.iter().map(|s| (*s).to_owned()).collect();
        resolve_order(&by, &ids, &pref)
    }

    #[test]
    fn load_order_is_topological_with_the_users_choice_for_free_mods() {
        let ms = [
            mk("aa", &[], &[], &[]),
            mk("bb", &["cc"], &[], &[]),
            mk("cc", &[], &[], &[]),
            mk("dd", &[], &["aa"], &[]),
        ];
        assert_eq!(order(&ms, &[]).unwrap(), ["aa", "cc", "bb", "dd"]);
        assert_eq!(order(&ms, &["dd"]).unwrap(), ["aa", "dd", "cc", "bb"]);
        assert_eq!(
            order(&[mk("zz", &[], &[], &["yy"]), mk("yy", &[], &[], &[])], &[]).unwrap(),
            ["zz", "yy"]
        );
        let cyc = [mk("pp", &["qq"], &[], &[]), mk("qq", &["pp"], &[], &[])];
        assert_eq!(order(&cyc, &[]).unwrap_err(), ["pp", "qq"]);
        // Hints about absent mods are ignored.
        assert_eq!(
            order(&[mk("aa", &[], &["missing"], &[])], &[]).unwrap(),
            ["aa"]
        );
    }
}
