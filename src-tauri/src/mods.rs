//! Mod management following the `mod.toml` spec (`modules/client-hook/docs/mod-manifest.md`
//! in Poe2ModdingProject).
//!
//! The launcher only manages files: mods live in `<install>/poe-hook-mods/<id>/` (never in game
//! files) and `<install>/poe-hook-mods/state.toml` holds `enabled = ["id", ...]` in load order,
//! which is the contract the DLL loader reads. Nothing here loads a mod.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::{Component, Path, PathBuf},
};

use anyhow::{Context, Result, anyhow, bail};
use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};

/// Directory next to the client executable.
pub const MODS_DIR: &str = "poe-hook-mods";
pub const STATE_FILE: &str = "state.toml";
pub const MANIFEST_FILE: &str = "mod.toml";
/// The `manifest_version` this launcher understands.
pub const MANIFEST_VERSION: u32 = 1;
/// The hook API major this launcher's hook build offers to mods.
pub const HOOK_API: u32 = 1;

const MAX_ZIP_ENTRIES: usize = 20_000;
const MAX_UNPACKED_BYTES: u64 = 4 << 30;

// ---------------------------------------------------------------------------------------------
// Manifest

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub authors: Vec<String>,
    pub license: String,
    pub homepage: String,
    /// `[game] versions`; empty = not stated.
    pub game_versions: Vec<String>,
    pub hook_api: Option<u32>,
    pub after: Vec<String>,
    pub before: Vec<String>,
    pub conflicts: Vec<String>,
    /// id -> semver requirement text.
    pub dependencies: BTreeMap<String, String>,
    pub entry_dll: Option<String>,
    pub data_tables: Vec<String>,
    pub data_opcodes: Vec<String>,
    pub config_defaults: Option<String>,
}

/// Known keys per section, for the "unknown keys are ignored (and logged)" rule.
const KNOWN_TOP: &[&str] = &[
    "manifest_version",
    "id",
    "name",
    "version",
    "description",
    "authors",
    "license",
    "homepage",
    "game",
    "load",
    "dependencies",
    "entry",
    "data",
];
const KNOWN_SECTIONS: &[(&str, &[&str])] = &[
    ("game", &["versions", "hook_api"]),
    ("load", &["after", "before", "conflicts"]),
    ("entry", &["dll"]),
    ("data", &["tables", "opcodes", "config_defaults"]),
];

/// Parsed manifest plus non-fatal findings.
#[derive(Debug, Clone)]
pub struct Parsed {
    pub manifest: Manifest,
    pub unknown_keys: Vec<String>,
}

/// Parses and validates `mod.toml` text; returns every problem found, not just the first.
pub fn parse_manifest(text: &str) -> Result<Parsed, Vec<String>> {
    let table: toml::Table = text
        .parse()
        .map_err(|e| vec![format!("mod.toml is not valid TOML: {e}")])?;
    let mut errors: Vec<String> = Vec::new();
    let mut unknown = Vec::new();
    for k in table.keys() {
        if !KNOWN_TOP.contains(&k.as_str()) {
            unknown.push(k.clone());
        }
    }
    for (sec, keys) in KNOWN_SECTIONS {
        if let Some(t) = table.get(*sec).and_then(toml::Value::as_table) {
            for k in t.keys().filter(|k| !keys.contains(&k.as_str())) {
                unknown.push(format!("{sec}.{k}"));
            }
        }
    }

    let mut m = Manifest::default();
    match table.get("manifest_version") {
        None => errors.push("manifest_version is required".into()),
        Some(v) => match v.as_integer() {
            Some(n) if n == i64::from(MANIFEST_VERSION) => {}
            Some(n) => errors.push(format!(
                "manifest_version {n} is not supported (this launcher reads {MANIFEST_VERSION})"
            )),
            None => errors.push("manifest_version must be an integer".into()),
        },
    }
    let string = |key: &str, required: bool, errors: &mut Vec<String>| -> String {
        match table.get(key) {
            Some(toml::Value::String(s)) => s.clone(),
            Some(_) => {
                errors.push(format!("{key} must be a string"));
                String::new()
            }
            None => {
                if required {
                    errors.push(format!("{key} is required"));
                }
                String::new()
            }
        }
    };
    m.id = string("id", true, &mut errors);
    m.name = string("name", true, &mut errors);
    m.version = string("version", true, &mut errors);
    m.description = string("description", false, &mut errors);
    m.license = string("license", false, &mut errors);
    m.homepage = string("homepage", false, &mut errors);
    m.authors = string_list(&table, None, "authors", &mut errors);

    if table.contains_key("id") && !valid_id(&m.id) {
        errors.push(format!(
            "id {:?} must match [a-z0-9][a-z0-9_-]{{1,63}}",
            m.id
        ));
    }
    if table.contains_key("name") && m.name.trim().is_empty() {
        errors.push("name is empty".into());
    }
    if table.contains_key("version") && Version::parse(&m.version).is_err() {
        errors.push(format!("version {:?} is not a semantic version", m.version));
    }

    if let Some(g) = sub(&table, "game", &mut errors) {
        m.game_versions = string_list(g, Some("game"), "versions", &mut errors);
        match g.get("hook_api") {
            None => {}
            Some(v) => match v.as_integer().and_then(|n| u32::try_from(n).ok()) {
                Some(n) => m.hook_api = Some(n),
                None => errors.push("game.hook_api must be a non-negative integer".into()),
            },
        }
    }
    if let Some(l) = sub(&table, "load", &mut errors) {
        for (key, slot) in [
            ("after", &mut m.after),
            ("before", &mut m.before),
            ("conflicts", &mut m.conflicts),
        ] {
            *slot = string_list(l, Some("load"), key, &mut errors);
            for id in slot.iter() {
                if !valid_id(id) {
                    errors.push(format!("load.{key}: {id:?} is not a valid mod id"));
                } else if *id == m.id {
                    errors.push(format!("load.{key}: a mod cannot refer to itself"));
                }
            }
        }
    }
    if let Some(d) = sub(&table, "dependencies", &mut errors) {
        for (id, req) in d {
            if !valid_id(id) {
                errors.push(format!("dependencies: {id:?} is not a valid mod id"));
                continue;
            }
            if *id == m.id {
                errors.push("dependencies: a mod cannot depend on itself".into());
            }
            match req.as_str() {
                Some(r) if VersionReq::parse(r).is_ok() => {
                    m.dependencies.insert(id.clone(), r.to_owned());
                }
                _ => errors.push(format!(
                    "dependencies.{id}: {req} is not a semver requirement"
                )),
            }
        }
    }
    if let Some(e) = sub(&table, "entry", &mut errors) {
        match e.get("dll") {
            None => {}
            Some(toml::Value::String(s)) => {
                check_rel_path("entry.dll", s, &mut errors);
                m.entry_dll = Some(s.clone());
            }
            Some(_) => errors.push("entry.dll must be a string".into()),
        }
    }
    if let Some(d) = sub(&table, "data", &mut errors) {
        m.data_tables = string_list(d, Some("data"), "tables", &mut errors);
        m.data_opcodes = string_list(d, Some("data"), "opcodes", &mut errors);
        match d.get("config_defaults") {
            None => {}
            Some(toml::Value::String(s)) => {
                check_rel_path("data.config_defaults", s, &mut errors);
                m.config_defaults = Some(s.clone());
            }
            Some(_) => errors.push("data.config_defaults must be a string".into()),
        }
        for p in m.data_tables.iter().chain(&m.data_opcodes) {
            check_rel_path("data path", p, &mut errors);
        }
    }
    if m.entry_dll.is_none()
        && m.data_tables.is_empty()
        && m.data_opcodes.is_empty()
        && m.config_defaults.is_none()
    {
        errors.push("a mod needs at least one of [entry] dll or [data] entries".into());
    }
    if errors.is_empty() {
        Ok(Parsed {
            manifest: m,
            unknown_keys: unknown,
        })
    } else {
        Err(errors)
    }
}

fn sub<'a>(t: &'a toml::Table, key: &str, errors: &mut Vec<String>) -> Option<&'a toml::Table> {
    match t.get(key) {
        None => None,
        Some(v) => match v.as_table() {
            Some(t) => Some(t),
            None => {
                errors.push(format!("[{key}] must be a table"));
                None
            }
        },
    }
}

fn string_list(
    t: &toml::Table,
    section: Option<&str>,
    key: &str,
    errors: &mut Vec<String>,
) -> Vec<String> {
    let label = section.map_or(key.to_owned(), |s| format!("{s}.{key}"));
    match t.get(key) {
        None => Vec::new(),
        Some(toml::Value::Array(a)) => {
            let mut out = Vec::new();
            for v in a {
                match v.as_str() {
                    Some(s) => out.push(s.to_owned()),
                    None => errors.push(format!("{label} must be a list of strings")),
                }
            }
            out
        }
        Some(_) => {
            errors.push(format!("{label} must be a list of strings"));
            Vec::new()
        }
    }
}

/// `[a-z0-9][a-z0-9_-]{1,63}`
pub fn valid_id(id: &str) -> bool {
    let b = id.as_bytes();
    (2..=64).contains(&b.len())
        && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
        && b[1..]
            .iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'_' || *c == b'-')
}

fn check_rel_path(what: &str, p: &str, errors: &mut Vec<String>) {
    let path = Path::new(p);
    let ok = !p.is_empty()
        && !p.contains('\\')
        && !path.is_absolute()
        && path.components().all(|c| matches!(c, Component::Normal(_)));
    if !ok {
        errors.push(format!(
            "{what} {p:?} must be a relative path inside the mod folder"
        ));
    }
}

impl Manifest {
    /// Files the manifest names must exist inside `dir`.
    pub fn check_files(&self, dir: &Path) -> Vec<String> {
        let mut missing = Vec::new();
        if let Some(d) = &self.entry_dll
            && !dir.join(d).is_file()
        {
            missing.push(format!("entry.dll {d:?} is missing"));
        }
        for p in self.data_tables.iter().chain(&self.data_opcodes) {
            if !dir.join(p).exists() {
                missing.push(format!("data path {p:?} is missing"));
            }
        }
        if let Some(c) = &self.config_defaults
            && !dir.join(c).is_file()
        {
            missing.push(format!("data.config_defaults {c:?} is missing"));
        }
        missing
    }
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
    HookApiTooNew {
        needs: u32,
        have: u32,
    },
}

impl Compat {
    /// Whether enabling needs the user's override.
    pub fn blocks(&self) -> bool {
        matches!(
            self,
            Compat::WrongGame { .. } | Compat::HookApiTooNew { .. }
        )
    }
}

pub fn compat(m: &Manifest, game_version: &str, hook_api: u32) -> Compat {
    if let Some(need) = m.hook_api
        && need > hook_api
    {
        return Compat::HookApiTooNew {
            needs: need,
            have: hook_api,
        };
    }
    if m.game_versions.is_empty() {
        Compat::Unspecified
    } else if m.game_versions.iter().any(|v| v == game_version) {
        Compat::Ok
    } else {
        Compat::WrongGame {
            supported: m.game_versions.clone(),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Load order

/// Topological order of `ids` by `dependencies`, `load.after` and `load.before` (only among
/// the given mods). Among mods that are free to go, the one earliest in `preferred` goes first
/// (mods not in `preferred` sort by id after those). The spec's order is `preferred = []`:
/// ties broken by id. A cycle is an error naming the mods in it.
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

// ---------------------------------------------------------------------------------------------
// State file

#[derive(Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct State {
    #[serde(default)]
    pub enabled: Vec<String>,
}

// ---------------------------------------------------------------------------------------------
// The mod directory

#[derive(Debug, Clone, Serialize)]
pub struct InstalledMod {
    pub id: String,
    pub dir_name: String,
    pub manifest: Option<Manifest>,
    pub errors: Vec<String>,
    pub unknown_keys: Vec<String>,
    pub compat: Option<Compat>,
    pub enabled: bool,
    /// Position in the load order when enabled (0 = first).
    pub order: Option<usize>,
}

pub struct ModStore {
    dir: PathBuf,
    game_version: String,
}

impl ModStore {
    /// `install_dir` is the game directory; mods go to `<install_dir>/poe-hook-mods`.
    pub fn new(install_dir: &Path, game_version: &str) -> Self {
        Self {
            dir: install_dir.join(MODS_DIR),
            game_version: game_version.into(),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn state(&self) -> Result<State> {
        match std::fs::read_to_string(self.dir.join(STATE_FILE)) {
            Ok(t) => toml::from_str(&t).context("state.toml is not valid"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(State::default()),
            Err(e) => Err(e.into()),
        }
    }

    fn save_state(&self, enabled: &[String]) -> Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let text = format!(
            "# Written by the Poe2 launcher. `enabled` is the load order.\n{}",
            toml::to_string(&State {
                enabled: enabled.to_vec()
            })?
        );
        crate::paths::write_atomic(&self.dir.join(STATE_FILE), text.as_bytes())
    }

    /// Every folder with something that looks like a mod, valid or not.
    pub fn list(&self) -> Result<Vec<InstalledMod>> {
        let state = self.state().unwrap_or_default();
        let mut out = Vec::new();
        let Ok(rd) = std::fs::read_dir(&self.dir) else {
            return Ok(out);
        };
        for entry in rd.flatten() {
            if !entry.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            let dir_name = entry.file_name().to_string_lossy().into_owned();
            if dir_name.starts_with('.') {
                continue;
            }
            let mut item = InstalledMod {
                id: dir_name.clone(),
                dir_name: dir_name.clone(),
                manifest: None,
                errors: Vec::new(),
                unknown_keys: Vec::new(),
                compat: None,
                enabled: false,
                order: None,
            };
            match std::fs::read_to_string(entry.path().join(MANIFEST_FILE)) {
                Err(_) => item.errors.push("mod.toml is missing".into()),
                Ok(text) => match parse_manifest(&text) {
                    Err(e) => item.errors = e,
                    Ok(p) => {
                        item.errors = p.manifest.check_files(&entry.path());
                        if p.manifest.id != dir_name {
                            item.errors.push(format!(
                                "the folder is named {dir_name:?} but the manifest id is {:?}",
                                p.manifest.id
                            ));
                        }
                        item.id = p.manifest.id.clone();
                        item.compat = Some(compat(&p.manifest, &self.game_version, HOOK_API));
                        item.unknown_keys = p.unknown_keys;
                        item.manifest = Some(p.manifest);
                    }
                },
            }
            if let Some(pos) = state.enabled.iter().position(|e| *e == dir_name) {
                item.enabled = true;
                item.order = Some(pos);
            }
            out.push(item);
        }
        out.sort_by(|a, b| {
            (a.order.unwrap_or(usize::MAX), &a.id).cmp(&(b.order.unwrap_or(usize::MAX), &b.id))
        });
        Ok(out)
    }

    fn valid_mods(&self) -> Result<Vec<InstalledMod>> {
        Ok(self
            .list()?
            .into_iter()
            .filter(|m| m.manifest.is_some() && m.errors.is_empty())
            .collect())
    }

    /// Enables a mod (and, first, its dependencies); returns the ids newly enabled in the order
    /// they were added. Refuses incompatible mods unless `override_compat`, missing or
    /// out-of-range dependencies, conflicts and ordering cycles; on any error nothing changes.
    pub fn enable(&self, id: &str, override_compat: bool) -> Result<Vec<String>> {
        let installed = self.valid_mods()?;
        let by_id: BTreeMap<String, &InstalledMod> =
            installed.iter().map(|m| (m.id.clone(), m)).collect();
        let mut enabled = self.state()?.enabled;
        let mut added: Vec<String> = Vec::new();
        let mut stack: Vec<String> = Vec::new();
        self.enable_rec(
            id,
            override_compat,
            &by_id,
            &mut enabled,
            &mut added,
            &mut stack,
        )?;

        // Conflicts among everything that ends up enabled.
        for a in &enabled {
            let ma = by_id[a].manifest.as_ref().unwrap();
            for c in &ma.conflicts {
                if enabled.contains(c) {
                    bail!("{a} conflicts with {c}: disable one of them first");
                }
            }
        }
        let manifests: BTreeMap<String, &Manifest> = by_id
            .iter()
            .filter(|(k, _)| enabled.contains(k))
            .map(|(k, m)| (k.clone(), m.manifest.as_ref().unwrap()))
            .collect();
        let ids: BTreeSet<String> = enabled.iter().cloned().collect();
        let order = resolve_order(&manifests, &ids, &enabled)
            .map_err(|cycle| anyhow!("load order cycle between: {}", cycle.join(", ")))?;
        self.save_state(&order)?;
        Ok(added)
    }

    fn enable_rec(
        &self,
        id: &str,
        override_compat: bool,
        by_id: &BTreeMap<String, &InstalledMod>,
        enabled: &mut Vec<String>,
        added: &mut Vec<String>,
        stack: &mut Vec<String>,
    ) -> Result<()> {
        if enabled.iter().any(|e| e == id) {
            return Ok(());
        }
        if stack.iter().any(|s| s == id) {
            bail!("dependency cycle: {} -> {id}", stack.join(" -> "));
        }
        let m = by_id
            .get(id)
            .ok_or_else(|| anyhow!("mod {id:?} is not installed or has errors"))?;
        let manifest = m.manifest.as_ref().unwrap();
        let c = m.compat.clone().unwrap_or(Compat::Ok);
        if c.blocks() && !override_compat {
            bail!(
                "{id} is not compatible with client {}: {}",
                self.game_version,
                describe_compat(&c)
            );
        }
        stack.push(id.to_owned());
        for (dep, req) in &manifest.dependencies {
            let d = by_id
                .get(dep)
                .ok_or_else(|| anyhow!("{id} needs {dep} {req}, which is not installed"))?;
            let have = Version::parse(&d.manifest.as_ref().unwrap().version)?;
            if !VersionReq::parse(req)?.matches(&have) {
                bail!("{id} needs {dep} {req}, but {have} is installed");
            }
            self.enable_rec(dep, override_compat, by_id, enabled, added, stack)?;
        }
        stack.pop();
        enabled.push(id.to_owned());
        added.push(id.to_owned());
        Ok(())
    }

    /// Disables a mod and every enabled mod that depends on it; returns all ids disabled.
    pub fn disable(&self, id: &str) -> Result<Vec<String>> {
        let installed = self.valid_mods()?;
        let mut enabled = self.state()?.enabled;
        if !enabled.iter().any(|e| e == id) {
            return Ok(Vec::new());
        }
        let mut gone = vec![id.to_owned()];
        loop {
            let more: Vec<String> = installed
                .iter()
                .filter(|m| enabled.contains(&m.id) && !gone.contains(&m.id))
                .filter(|m| {
                    m.manifest
                        .as_ref()
                        .unwrap()
                        .dependencies
                        .keys()
                        .any(|d| gone.contains(d))
                })
                .map(|m| m.id.clone())
                .collect();
            if more.is_empty() {
                break;
            }
            gone.extend(more);
        }
        enabled.retain(|e| !gone.contains(e));
        self.save_state(&enabled)?;
        Ok(gone)
    }

    /// Moves an enabled mod to `position` in the load order; the order the constraints
    /// (dependencies, `after`/`before`) allow wins over the request. Returns the new order.
    pub fn move_to(&self, id: &str, position: usize) -> Result<Vec<String>> {
        let mut enabled = self.state()?.enabled;
        let from = enabled
            .iter()
            .position(|e| e == id)
            .ok_or_else(|| anyhow!("{id} is not enabled"))?;
        let item = enabled.remove(from);
        enabled.insert(position.min(enabled.len()), item);
        self.reorder(&enabled)
    }

    /// The spec's order: constraints, ties by id.
    pub fn auto_order(&self) -> Result<Vec<String>> {
        let enabled = self.state()?.enabled;
        self.reorder_with(&enabled, &[])
    }

    fn reorder(&self, preferred: &[String]) -> Result<Vec<String>> {
        self.reorder_with(preferred, preferred)
    }

    fn reorder_with(&self, set: &[String], preferred: &[String]) -> Result<Vec<String>> {
        let installed = self.valid_mods()?;
        let manifests: BTreeMap<String, &Manifest> = installed
            .iter()
            .filter(|m| set.contains(&m.id))
            .map(|m| (m.id.clone(), m.manifest.as_ref().unwrap()))
            .collect();
        let ids: BTreeSet<String> = manifests.keys().cloned().collect();
        let order = resolve_order(&manifests, &ids, preferred)
            .map_err(|cycle| anyhow!("load order cycle between: {}", cycle.join(", ")))?;
        self.save_state(&order)?;
        Ok(order)
    }

    /// Removes an installed mod. Refuses while an enabled mod depends on it.
    pub fn remove(&self, id: &str) -> Result<()> {
        let all = self.list()?;
        let target = all
            .iter()
            .find(|m| m.id == id)
            .ok_or_else(|| anyhow!("mod {id:?} is not installed"))?;
        let dependents: Vec<&str> = all
            .iter()
            .filter(|m| m.enabled && m.id != id)
            .filter(|m| {
                m.manifest
                    .as_ref()
                    .is_some_and(|mf| mf.dependencies.contains_key(id))
            })
            .map(|m| m.id.as_str())
            .collect();
        if !dependents.is_empty() {
            bail!("{id} is needed by enabled mods: {}", dependents.join(", "));
        }
        let mut enabled = self.state()?.enabled;
        if enabled.contains(&target.dir_name) {
            enabled.retain(|e| *e != target.dir_name);
            self.save_state(&enabled)?;
        }
        std::fs::remove_dir_all(self.dir.join(&target.dir_name))?;
        Ok(())
    }

    /// Installs a mod from a folder with a `mod.toml` at its root.
    pub fn install_dir(&self, src: &Path, replace: bool) -> Result<InstalledNew> {
        let src = src
            .canonicalize()
            .with_context(|| format!("{} does not exist", src.display()))?;
        if let Ok(own) = self.dir.canonicalize()
            && src.starts_with(&own)
        {
            bail!("that folder is already inside the mod directory");
        }
        std::fs::create_dir_all(&self.dir)?;
        let tmp = self.dir.join(format!(".incoming-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let result = (|| {
            copy_tree(&src, &tmp, &mut 0)?;
            self.finish_install(&tmp, replace)
        })();
        let _ = std::fs::remove_dir_all(&tmp);
        result
    }

    /// Installs a mod from a zip whose root holds `mod.toml` (or one folder that does).
    pub fn install_zip(&self, zip_path: &Path, replace: bool) -> Result<InstalledNew> {
        std::fs::create_dir_all(&self.dir)?;
        let tmp = self.dir.join(format!(".incoming-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let result = (|| {
            unzip_limited(zip_path, &tmp)?;
            let root = if tmp.join(MANIFEST_FILE).is_file() {
                tmp.clone()
            } else {
                let dirs: Vec<_> = std::fs::read_dir(&tmp)?.flatten().collect();
                match dirs.as_slice() {
                    [one] if one.path().join(MANIFEST_FILE).is_file() => one.path(),
                    _ => bail!("the archive has no mod.toml at its root"),
                }
            };
            self.finish_install(&root, replace)
        })();
        let _ = std::fs::remove_dir_all(&tmp);
        result
    }

    fn finish_install(&self, staged: &Path, replace: bool) -> Result<InstalledNew> {
        let text = std::fs::read_to_string(staged.join(MANIFEST_FILE))
            .context("mod.toml not found in the mod folder")?;
        let parsed =
            parse_manifest(&text).map_err(|e| anyhow!("invalid mod.toml: {}", e.join("; ")))?;
        let m = parsed.manifest;
        let missing = m.check_files(staged);
        if !missing.is_empty() {
            bail!("the mod is incomplete: {}", missing.join("; "));
        }
        let target = self.dir.join(&m.id);
        if target.exists() {
            if !replace {
                bail!("mod {} is already installed (replace it to update)", m.id);
            }
            std::fs::remove_dir_all(&target)?;
        }
        // Rename keeps it atomic; across file systems fall back to a copy.
        if std::fs::rename(staged, &target).is_err() {
            copy_tree(staged, &target, &mut 0)?;
        }
        Ok(InstalledNew {
            compat: compat(&m, &self.game_version, HOOK_API),
            unknown_keys: parsed.unknown_keys,
            id: m.id,
        })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct InstalledNew {
    pub id: String,
    pub compat: Compat,
    pub unknown_keys: Vec<String>,
}

pub fn describe_compat(c: &Compat) -> String {
    match c {
        Compat::Ok => "compatible".into(),
        Compat::Unspecified => "no client version stated".into(),
        Compat::WrongGame { supported } => format!("made for {}", supported.join(", ")),
        Compat::HookApiTooNew { needs, have } => format!("needs hook API {needs}, this has {have}"),
    }
}

/// Copies a directory tree, refusing symbolic links; bounded in size and count.
fn copy_tree(src: &Path, dst: &Path, count: &mut usize) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        *count += 1;
        if *count > MAX_ZIP_ENTRIES {
            bail!("the mod folder has too many files");
        }
        let ty = entry.file_type()?;
        let to = dst.join(entry.file_name());
        if ty.is_symlink() {
            bail!(
                "{} is a symbolic link; mods must not contain links",
                entry.path().display()
            );
        } else if ty.is_dir() {
            copy_tree(&entry.path(), &to, count)?;
        } else {
            std::fs::copy(entry.path(), &to)?;
        }
    }
    Ok(())
}

fn unzip_limited(zip_path: &Path, dest: &Path) -> Result<()> {
    let mut archive = zip::ZipArchive::new(
        std::fs::File::open(zip_path).with_context(|| format!("opening {}", zip_path.display()))?,
    )
    .context("not a zip archive")?;
    if archive.len() > MAX_ZIP_ENTRIES {
        bail!("the archive has too many entries");
    }
    std::fs::create_dir_all(dest)?;
    let mut total = 0u64;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let rel = entry
            .enclosed_name()
            .ok_or_else(|| anyhow!("unsafe path in the archive: {}", entry.name()))?;
        if entry.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000) {
            bail!("the archive contains a symbolic link: {}", entry.name());
        }
        let out = dest.join(&rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out)?;
            continue;
        }
        total = total.saturating_add(entry.size());
        if total > MAX_UNPACKED_BYTES {
            bail!(
                "the archive unpacks to more than {} bytes",
                MAX_UNPACKED_BYTES
            );
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut f = std::fs::File::create(&out)?;
        // `size()` comes from the archive and can lie: bound what is really written.
        let limit = MAX_UNPACKED_BYTES.saturating_sub(total) + 1;
        let copied = std::io::copy(&mut std::io::Read::take(&mut entry, limit), &mut f)?;
        if copied >= limit {
            bail!("the archive unpacks to more than {MAX_UNPACKED_BYTES} bytes");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn manifest_text(id: &str, extra: &str) -> String {
        format!(
            "manifest_version = 1\nid = \"{id}\"\nname = \"{id} name\"\nversion = \"1.0.0\"\n{extra}\n"
        )
    }

    /// A mod folder with a data dir, so `[data]` has something to name.
    fn make_mod(root: &Path, id: &str, extra: &str, sections: &str) -> PathBuf {
        let dir = root.join(id);
        std::fs::create_dir_all(dir.join("data/tables")).unwrap();
        std::fs::write(
            dir.join("mod.toml"),
            format!(
                "{}{sections}\n[data]\ntables = [\"data/tables\"]\n",
                manifest_text(id, extra)
            ),
        )
        .unwrap();
        dir
    }

    #[test]
    fn the_spec_example_parses() {
        let text = r#"
manifest_version = 1
id = "my-mod"
name = "My Mod"
version = "1.2.0"
description = "One sentence."
authors = ["Someone"]
license = "MIT"
homepage = "https://example.invalid/my-mod"
[game]
versions = ["0.5.5e"]
hook_api = 1
[load]
after = ["other-mod"]
before = []
conflicts = []
[dependencies]
core-lib = ">=1.0, <2"
[entry]
dll = "my_mod.dll"
[data]
tables = ["data/tables"]
opcodes = ["data/opcodes"]
config_defaults = "data/defaults.toml"
"#;
        let p = parse_manifest(text).unwrap();
        assert!(p.unknown_keys.is_empty());
        let m = p.manifest;
        assert_eq!(m.id, "my-mod");
        assert_eq!(m.game_versions, ["0.5.5e"]);
        assert_eq!(m.dependencies["core-lib"], ">=1.0, <2");
        assert_eq!(m.entry_dll.as_deref(), Some("my_mod.dll"));
    }

    #[test]
    fn validation_collects_every_problem() {
        let errs = parse_manifest(
            "id = \"A\"\nversion = \"one\"\n[dependencies]\ndep-x = \"nonsense\"\n[entry]\ndll = \"../up.dll\"\n",
        )
        .unwrap_err();
        let all = errs.join("\n");
        for needle in [
            "manifest_version is required",
            "name is required",
            "id \"A\"",
            "semantic version",
            "semver requirement",
            "relative path",
        ] {
            assert!(all.contains(needle), "{needle} not in:\n{all}");
        }
        assert!(parse_manifest("not toml {{").is_err());
        let e = parse_manifest(&manifest_text("good-id", "")).unwrap_err();
        assert!(e.join(" ").contains("at least one of"), "{e:?}");
        let e = parse_manifest(
            &format!("{}[entry]\ndll = \"a.dll\"\n", manifest_text("m2", ""))
                .replace("manifest_version = 1", "manifest_version = 2"),
        )
        .unwrap_err();
        assert!(e.join(" ").contains("not supported"));
    }

    #[test]
    fn ids_follow_the_spec() {
        for ok in ["ab", "my-mod", "a_1", "0x", &"a".repeat(64)] {
            assert!(valid_id(ok), "{ok}");
        }
        for bad in ["a", "A1", "-ab", "_ab", "a b", "a.b", "", &"a".repeat(65)] {
            assert!(!valid_id(bad), "{bad}");
        }
    }

    #[test]
    fn unknown_keys_are_reported_not_fatal() {
        let p = parse_manifest(&format!(
            "{}future = 1\n[entry]\ndll = \"a.dll\"\nflags = 2\n",
            manifest_text("m1", "")
        ))
        .unwrap();
        assert_eq!(p.unknown_keys, ["future", "entry.flags"]);
    }

    #[test]
    fn compatibility_follows_game_version_and_hook_api() {
        let mut m = parse_manifest(&format!(
            "{}[entry]\ndll = \"a.dll\"\n",
            manifest_text("m1", "")
        ))
        .unwrap()
        .manifest;
        assert_eq!(compat(&m, "0.5.5e", 1), Compat::Unspecified);
        m.game_versions = vec!["0.5.5e".into()];
        assert_eq!(compat(&m, "0.5.5e", 1), Compat::Ok);
        assert!(compat(&m, "0.5.6", 1).blocks());
        m.hook_api = Some(2);
        assert!(matches!(
            compat(&m, "0.5.5e", 1),
            Compat::HookApiTooNew { needs: 2, have: 1 }
        ));
        assert!(!compat(&m, "0.5.5e", 2).blocks());
    }

    fn mk(id: &str, deps: &[&str], after: &[&str], before: &[&str]) -> Manifest {
        Manifest {
            id: id.into(),
            dependencies: deps.iter().map(|d| ((*d).into(), "*".into())).collect(),
            after: after.iter().map(|s| (*s).into()).collect(),
            before: before.iter().map(|s| (*s).into()).collect(),
            ..Default::default()
        }
    }

    fn order(ms: &[Manifest], preferred: &[&str]) -> Result<Vec<String>, Vec<String>> {
        let map: BTreeMap<String, &Manifest> = ms.iter().map(|m| (m.id.clone(), m)).collect();
        let ids = map.keys().cloned().collect();
        let pref: Vec<String> = preferred.iter().map(|s| (*s).to_owned()).collect();
        resolve_order(&map, &ids, &pref)
    }

    #[test]
    fn load_order_is_topological_with_id_ties() {
        let ms = [
            mk("c", &["a"], &[], &[]),
            mk("b", &[], &["c"], &[]),
            mk("a", &[], &[], &[]),
            mk("z", &[], &[], &["a"]),
        ];
        // z before a (before-hint), a before c (dependency), c before b (after-hint).
        assert_eq!(order(&ms, &[]).unwrap(), ["z", "a", "c", "b"]);
        // Constraints beat the preferred order; free choices follow it.
        assert_eq!(
            order(&ms, &["b", "c", "a", "z"]).unwrap(),
            ["z", "a", "c", "b"]
        );
        let free = [
            mk("a", &[], &[], &[]),
            mk("b", &[], &[], &[]),
            mk("c", &[], &[], &[]),
        ];
        assert_eq!(order(&free, &["c", "a"]).unwrap(), ["c", "a", "b"]);
        assert_eq!(order(&free, &[]).unwrap(), ["a", "b", "c"]);
    }

    #[test]
    fn hints_about_absent_mods_are_ignored_and_cycles_are_reported() {
        let ms = [
            mk("a", &[], &["ghost"], &["ghost2"]),
            mk("b", &[], &[], &[]),
        ];
        assert_eq!(order(&ms, &[]).unwrap(), ["a", "b"]);
        let cyc = [
            mk("a", &["b"], &[], &[]),
            mk("b", &[], &["a"], &[]),
            mk("c", &[], &[], &[]),
        ];
        let err = order(&cyc, &[]).unwrap_err();
        assert_eq!(err, ["a", "b"]);
    }

    #[test]
    fn enable_disable_order_and_state_file() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ModStore::new(tmp.path(), "0.5.5e");
        let mods = store.dir().to_path_buf();
        make_mod(&mods, "core-lib", "", "");
        make_mod(
            &mods,
            "alpha",
            "",
            "[dependencies]\ncore-lib = \">=1, <2\"\n",
        );
        make_mod(
            &mods,
            "beta",
            "",
            "[load]\nafter = [\"alpha\"]\nconflicts = [\"evil\"]\n",
        );
        make_mod(&mods, "evil", "", "");

        assert_eq!(store.enable("alpha", false).unwrap(), ["core-lib", "alpha"]);
        assert_eq!(store.enable("beta", false).unwrap(), ["beta"]);
        assert_eq!(
            store.state().unwrap().enabled,
            ["core-lib", "alpha", "beta"]
        );
        // The state file is the loader's contract.
        let text = std::fs::read_to_string(mods.join("state.toml")).unwrap();
        assert!(
            text.contains("enabled = [\"core-lib\", \"alpha\", \"beta\"]"),
            "{text}"
        );

        let e = store.enable("evil", false).unwrap_err().to_string();
        assert!(e.contains("conflicts"), "{e}");
        assert_eq!(
            store.state().unwrap().enabled.len(),
            3,
            "a refused enable changes nothing"
        );

        // Moving beta first is overruled by its `after` hint; core-lib/alpha are constrained too.
        assert_eq!(
            store.move_to("beta", 0).unwrap(),
            ["core-lib", "alpha", "beta"]
        );
        let listed = store.list().unwrap();
        assert_eq!(listed.iter().filter(|m| m.enabled).count(), 3);
        assert_eq!(listed[0].id, "core-lib");

        // Disabling a dependency takes its dependents along.
        assert_eq!(store.disable("core-lib").unwrap(), ["core-lib", "alpha"]);
        assert_eq!(store.state().unwrap().enabled, ["beta"]);
        assert!(store.disable("alpha").unwrap().is_empty());
        assert_eq!(store.auto_order().unwrap(), ["beta"]);
    }

    #[test]
    fn dependencies_must_exist_and_match() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ModStore::new(tmp.path(), "0.5.5e");
        let mods = store.dir().to_path_buf();
        make_mod(&mods, "needs-missing", "", "[dependencies]\nnope = \"*\"\n");
        make_mod(&mods, "lib", "", "");
        make_mod(&mods, "needs-v2", "", "[dependencies]\nlib = \">=2\"\n");
        assert!(
            store
                .enable("needs-missing", false)
                .unwrap_err()
                .to_string()
                .contains("not installed")
        );
        assert!(
            store
                .enable("needs-v2", false)
                .unwrap_err()
                .to_string()
                .contains("but 1.0.0 is installed")
        );
        assert!(store.state().unwrap().enabled.is_empty());
    }

    #[test]
    fn incompatible_mods_need_the_override() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ModStore::new(tmp.path(), "0.5.5e");
        make_mod(
            store.dir(),
            "old-mod",
            "",
            "[game]\nversions = [\"0.4.0\"]\n",
        );
        let e = store.enable("old-mod", false).unwrap_err().to_string();
        assert!(e.contains("not compatible") && e.contains("0.4.0"), "{e}");
        assert_eq!(store.enable("old-mod", true).unwrap(), ["old-mod"]);
        let m = &store.list().unwrap()[0];
        assert!(m.enabled && m.compat.as_ref().unwrap().blocks());
    }

    #[test]
    fn broken_mods_are_listed_with_their_errors_and_cannot_be_enabled() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ModStore::new(tmp.path(), "0.5.5e");
        std::fs::create_dir_all(store.dir().join("junk")).unwrap();
        let dir = make_mod(store.dir(), "dll-less", "", "");
        std::fs::write(
            dir.join("mod.toml"),
            manifest_text("dll-less", "[entry]\ndll = \"missing.dll\""),
        )
        .unwrap();
        let list = store.list().unwrap();
        assert_eq!(list.len(), 2);
        assert!(list.iter().all(|m| !m.errors.is_empty()));
        assert!(store.enable("dll-less", false).is_err());
        assert!(store.enable("junk", false).is_err());
    }

    #[test]
    fn a_folder_install_validates_and_copies_without_touching_game_files() {
        let tmp = tempfile::tempdir().unwrap();
        let game = tmp.path().join("game");
        std::fs::create_dir_all(&game).unwrap();
        std::fs::write(game.join("PathOfExileSteam.exe"), b"x").unwrap();
        let store = ModStore::new(&game, "0.5.5e");
        let src = make_mod(
            tmp.path(),
            "my-mod",
            "",
            "[game]\nversions = [\"0.5.5e\"]\n",
        );
        let r = store.install_dir(&src, false).unwrap();
        assert_eq!((r.id.as_str(), &r.compat), ("my-mod", &Compat::Ok));
        assert!(game.join("poe-hook-mods/my-mod/data/tables").is_dir());
        assert!(
            store
                .install_dir(&src, false)
                .unwrap_err()
                .to_string()
                .contains("already installed")
        );
        store.install_dir(&src, true).unwrap();
        // Only the mods directory and the exe exist in the game directory.
        let names: BTreeSet<_> = std::fs::read_dir(&game)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            BTreeSet::from(["PathOfExileSteam.exe".to_owned(), MODS_DIR.to_owned()])
        );
        assert!(!game.join("poe-hook-mods").join(".incoming-0").exists());
        // Bad manifests are refused and leave nothing behind.
        let bad = tmp.path().join("bad");
        std::fs::create_dir_all(&bad).unwrap();
        std::fs::write(bad.join("mod.toml"), "id = \"x\"").unwrap();
        assert!(store.install_dir(&bad, false).is_err());
        assert_eq!(store.list().unwrap().len(), 1);
    }

    fn zip_with(path: &Path, files: &[(&str, &str)]) {
        let mut w = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        for (name, body) in files {
            w.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            w.write_all(body.as_bytes()).unwrap();
        }
        w.finish().unwrap();
    }

    #[test]
    fn zip_install_accepts_root_or_single_folder_and_refuses_escapes() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ModStore::new(&tmp.path().join("game"), "0.5.5e");
        let manifest = format!(
            "{}[data]\ntables = [\"t/x.txt\"]\n",
            manifest_text("zipped", "")
        );

        let flat = tmp.path().join("flat.zip");
        zip_with(&flat, &[("mod.toml", &manifest), ("t/x.txt", "1")]);
        assert_eq!(store.install_zip(&flat, false).unwrap().id, "zipped");
        assert!(store.dir().join("zipped/t/x.txt").is_file());
        store.remove("zipped").unwrap();

        let nested = tmp.path().join("nested.zip");
        zip_with(
            &nested,
            &[
                ("zipped-1.0/mod.toml", &manifest),
                ("zipped-1.0/t/x.txt", "1"),
            ],
        );
        assert_eq!(store.install_zip(&nested, false).unwrap().id, "zipped");

        let evil = tmp.path().join("evil.zip");
        zip_with(&evil, &[("mod.toml", &manifest), ("../outside.txt", "1")]);
        assert!(store.install_zip(&evil, true).is_err());
        assert!(
            !tmp.path().join("game/outside.txt").exists()
                && !store.dir().join("../outside.txt").exists()
        );

        let empty = tmp.path().join("empty.zip");
        zip_with(&empty, &[("readme.txt", "no mod here")]);
        assert!(
            store
                .install_zip(&empty, false)
                .unwrap_err()
                .to_string()
                .contains("no mod.toml")
        );
        assert!(
            store
                .install_zip(&tmp.path().join("missing.zip"), false)
                .is_err()
        );
    }

    #[test]
    fn remove_refuses_while_needed_and_cleans_the_state() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ModStore::new(tmp.path(), "0.5.5e");
        make_mod(store.dir(), "lib", "", "");
        make_mod(store.dir(), "user", "", "[dependencies]\nlib = \"*\"\n");
        store.enable("user", false).unwrap();
        assert!(
            store
                .remove("lib")
                .unwrap_err()
                .to_string()
                .contains("needed by")
        );
        store.remove("user").unwrap();
        assert_eq!(store.state().unwrap().enabled, ["lib"]);
        store.remove("lib").unwrap();
        assert!(store.state().unwrap().enabled.is_empty());
        assert!(store.list().unwrap().is_empty());
        assert!(store.remove("lib").is_err());
    }
}
