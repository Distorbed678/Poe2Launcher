//! Mod sets: which cached mods a server profile loads, and writing them into the game.
//!
//! A server profile's set is: the mods the server **requires** (always on for that server, in
//! the version the server pins), plus the mods the player enabled **for that server**, plus the
//! mods the player enabled for **all servers**. The player's other mods are off on that server.
//! Dependencies are added from the cache; an own mod that conflicts with a required one, is
//! incompatible, or lacks a dependency is dropped with a note (a required mod that cannot be
//! satisfied is a problem that blocks Play). The official (read-only) entry has no set at all:
//! the hook loads no mod there, and nothing in this module is called for it.
//!
//! [`apply_to_game`] copies the set from the cache into `<install>/poe-hook-mods/<id>/` and
//! writes `state.toml` (the load order). Mod folders the launcher put there earlier and the set
//! no longer needs are removed; folders it did not put there (a developer's `deploy-mod`) are
//! left alone, never overwritten, and not enabled.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use anyhow::{Context, Result, bail};
use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};

use crate::{
    modcache::{CachedMod, ModCache},
    mods::{Compat, MODS_DIR, Manifest, ModPin, STATE_FILE, compat, resolve_order},
    paths::write_atomic,
};

/// Written into every mod folder the launcher puts into the game.
pub const MARKER_FILE: &str = ".poe2-launcher-mod.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    /// The server requires it for this profile.
    Required,
    /// Enabled for this server profile.
    Profile,
    /// Enabled for all servers.
    Global,
    /// Pulled in by the dependency of another mod.
    Dependency,
}

#[derive(Debug, Clone, Serialize)]
pub struct Member {
    pub id: String,
    pub version: String,
    pub sha256: String,
    pub origin: Origin,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Effective {
    /// The mods to load, in load order.
    pub members: Vec<Member>,
    /// Own mods left out, and why (shown to the player, not an error).
    pub dropped: Vec<String>,
    /// Things that make the set unusable: a required mod that cannot load.
    pub problems: Vec<String>,
}

impl Effective {
    pub fn ids(&self) -> Vec<String> {
        self.members.iter().map(|m| m.id.clone()).collect()
    }
}

/// What the server requires for a profile, as exact versions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Requirement {
    pub id: String,
    pub version: String,
}

struct Ctx<'a> {
    game_version: &'a str,
    hook_abi: u32,
}

fn pick<'a>(
    by_id: &'a BTreeMap<String, Vec<&'a CachedMod>>,
    id: &str,
    version: &str,
) -> Option<&'a CachedMod> {
    by_id
        .get(id)?
        .iter()
        .copied()
        .find(|m| m.meta.version == version)
}

/// Computes the set. `pins` are the profile's, then the global ones; `required` the server's.
pub fn effective(
    cache: &ModCache,
    required: &[Requirement],
    profile_pins: &[ModPin],
    global_pins: &[ModPin],
    game_version: &str,
    hook_abi: u32,
) -> Effective {
    let ctx = Ctx {
        game_version,
        hook_abi,
    };
    let all = cache.list();
    let usable: Vec<&CachedMod> = all
        .iter()
        .filter(|m| m.errors.is_empty() && m.manifest.is_some())
        .collect();
    let mut by_id: BTreeMap<String, Vec<&CachedMod>> = BTreeMap::new();
    for m in &usable {
        by_id.entry(m.meta.id.clone()).or_default().push(m);
    }
    let mut out = Effective::default();
    let mut chosen: BTreeMap<String, (&CachedMod, Origin)> = BTreeMap::new();

    // 1. The server's mods: all or nothing, they win over the player's own.
    for r in required {
        match pick(&by_id, &r.id, &r.version) {
            None => out.problems.push(format!(
                "the server requires {} {}, which is not in the mod cache",
                r.id, r.version
            )),
            Some(m) => {
                let c = compat(m.manifest.as_ref().unwrap(), ctx.game_version, ctx.hook_abi);
                if c.blocks() && !matches!(c, Compat::WrongGame { .. }) {
                    out.problems.push(format!(
                        "the server requires {} {}, which cannot load: {}",
                        r.id,
                        r.version,
                        crate::mods::describe_compat(&c)
                    ));
                    continue;
                }
                chosen.insert(r.id.clone(), (m, Origin::Required));
            }
        }
    }

    // 2. The player's mods, profile first.
    let mut own: Vec<(&ModPin, Origin)> = Vec::new();
    own.extend(profile_pins.iter().map(|p| (p, Origin::Profile)));
    own.extend(global_pins.iter().map(|p| (p, Origin::Global)));
    for (pin, origin) in own {
        if let Some((cur, cur_origin)) = chosen.get(&pin.id) {
            // The server's mod, or the profile's pin of this id, already won.
            if *cur_origin == Origin::Required && cur.meta.version != pin.version {
                out.dropped.push(format!(
                    "{} {} is not loaded: the server requires {} {}",
                    pin.id, pin.version, pin.id, cur.meta.version
                ));
            }
            continue;
        }
        let Some(m) = pick(&by_id, &pin.id, &pin.version) else {
            out.dropped.push(format!(
                "{} {} is enabled but not in the mod cache",
                pin.id, pin.version
            ));
            continue;
        };
        let c = compat(m.manifest.as_ref().unwrap(), ctx.game_version, ctx.hook_abi);
        if c.blocks() && !(pin.force && c.overridable()) {
            out.dropped.push(format!(
                "{} {} is not loaded: {}",
                pin.id,
                pin.version,
                crate::mods::describe_compat(&c)
            ));
            continue;
        }
        chosen.insert(pin.id.clone(), (m, origin));
    }

    // 3. Dependencies, from the cache (the newest version that satisfies the requirement).
    let mut queue: Vec<String> = chosen.keys().cloned().collect();
    let mut guard = 0;
    while let Some(id) = queue.pop() {
        guard += 1;
        if guard > 4096 {
            out.problems.push("the dependency chain is too long".into());
            break;
        }
        let Some((m, origin)) = chosen.get(&id).map(|(m, o)| (*m, *o)) else {
            continue; // dropped meanwhile
        };
        let manifest = m.manifest.as_ref().unwrap();
        let mut failed: Option<String> = None;
        let mut add: Vec<(&CachedMod, String)> = Vec::new();
        for (dep, req) in &manifest.dependencies {
            let Ok(req) = VersionReq::parse(req) else {
                failed = Some(format!("{dep}: {req} is not a version requirement"));
                break;
            };
            if let Some((have, _)) = chosen.get(dep) {
                let ok = Version::parse(&have.meta.version).is_ok_and(|v| req.matches(&v));
                if !ok {
                    failed = Some(format!(
                        "needs {dep} {req}, but {} {} is in the set",
                        dep, have.meta.version
                    ));
                    break;
                }
                continue;
            }
            // Newest first: `by_id` lists are in cache order (newest first).
            let found = by_id.get(dep).and_then(|vs| {
                vs.iter().copied().find(|c| {
                    Version::parse(&c.meta.version).is_ok_and(|v| req.matches(&v))
                        && !compat(c.manifest.as_ref().unwrap(), ctx.game_version, ctx.hook_abi)
                            .blocks()
                })
            });
            match found {
                Some(d) => add.push((d, dep.clone())),
                None => {
                    failed = Some(format!("needs {dep} {req}, which is not in the mod cache"));
                    break;
                }
            }
        }
        if let Some(why) = failed {
            if origin == Origin::Required {
                out.problems.push(format!("the required {id} {why}"));
            } else {
                out.dropped.push(format!("{id} is not loaded: it {why}"));
                chosen.remove(&id);
                // Whatever was pulled in only for it stays only if something else needs it;
                // the clean-up pass below removes the rest.
            }
            continue;
        }
        for (d, dep) in add {
            chosen.insert(dep.clone(), (d, Origin::Dependency));
            queue.push(dep);
        }
    }

    // 4. Conflicts: a required mod beats an own one; among own mods the earlier wins.
    let order_of: Vec<String> = chosen.keys().cloned().collect();
    let mut remove: Vec<String> = Vec::new();
    let mut sorted: Vec<&String> = order_of.iter().collect();
    sorted.sort_by_key(|id| {
        (
            match chosen[*id].1 {
                Origin::Required => 0,
                Origin::Profile => 1,
                Origin::Global => 2,
                Origin::Dependency => 3,
            },
            (*id).clone(),
        )
    });
    let mut kept: Vec<&String> = Vec::new();
    for id in sorted {
        let m = chosen[id].0.manifest.as_ref().unwrap();
        let clash = kept.iter().find(|k| {
            let other = chosen[**k].0.manifest.as_ref().unwrap();
            m.conflicts.contains(*k) || other.conflicts.contains(id)
        });
        match clash {
            Some(k) if chosen[id].1 == Origin::Required => {
                out.problems
                    .push(format!("the required {id} conflicts with {k}"));
                kept.push(id);
            }
            Some(k) => {
                out.dropped
                    .push(format!("{id} is not loaded: it conflicts with {k}"));
                remove.push(id.clone());
            }
            None => kept.push(id),
        }
    }
    for id in remove {
        chosen.remove(&id);
    }

    // 5. A dependency nobody needs any more (its dependent was dropped) goes too.
    loop {
        let needed: BTreeSet<&str> = chosen
            .values()
            .flat_map(|(m, _)| {
                m.manifest
                    .as_ref()
                    .unwrap()
                    .dependencies
                    .keys()
                    .map(String::as_str)
            })
            .collect();
        let stale: Vec<String> = chosen
            .iter()
            .filter(|(id, (_, o))| *o == Origin::Dependency && !needed.contains(id.as_str()))
            .map(|(id, _)| id.clone())
            .collect();
        if stale.is_empty() {
            break;
        }
        for id in stale {
            chosen.remove(&id);
        }
    }

    // 6. Load order: constraints, then required, profile, global as the player arranged them.
    let manifests: BTreeMap<String, &Manifest> = chosen
        .iter()
        .map(|(id, (m, _))| (id.clone(), m.manifest.as_ref().unwrap()))
        .collect();
    let ids: BTreeSet<String> = chosen.keys().cloned().collect();
    let mut preferred: Vec<String> = Vec::new();
    preferred.extend(required.iter().map(|r| r.id.clone()));
    preferred.extend(profile_pins.iter().map(|p| p.id.clone()));
    preferred.extend(global_pins.iter().map(|p| p.id.clone()));
    match resolve_order(&manifests, &ids, &preferred) {
        Ok(order) => {
            for id in order {
                let (m, origin) = &chosen[&id];
                out.members.push(Member {
                    id,
                    version: m.meta.version.clone(),
                    sha256: m.meta.sha256.clone(),
                    origin: *origin,
                });
            }
        }
        Err(cycle) => out
            .problems
            .push(format!("load order cycle between: {}", cycle.join(", "))),
    }
    out
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Marker {
    id: String,
    version: String,
    sha256: String,
}

/// What [`apply_to_game`] did.
#[derive(Debug, Default, Serialize)]
pub struct Applied {
    pub enabled: Vec<String>,
    pub copied: Vec<String>,
    pub removed: Vec<String>,
    pub kept_foreign: Vec<String>,
}

/// Puts `set` into `<install_dir>/poe-hook-mods/` and writes `state.toml`. Fails (touching
/// nothing it could not undo by running again) when a folder of an effective mod is not the
/// launcher's own.
pub fn apply_to_game(install_dir: &Path, set: &Effective, cache: &ModCache) -> Result<Applied> {
    let dir = install_dir.join(MODS_DIR);
    let mut done = Applied::default();
    let mut existing: BTreeMap<String, Option<Marker>> = BTreeMap::new();
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            if !e.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                // A leftover scratch folder of an interrupted copy.
                if name.starts_with(".incoming-") {
                    let _ = std::fs::remove_dir_all(e.path());
                }
                continue;
            }
            let marker = std::fs::read_to_string(e.path().join(MARKER_FILE))
                .ok()
                .and_then(|t| serde_json::from_str::<Marker>(&t).ok());
            existing.insert(name, marker);
        }
    }
    // Check everything before touching anything.
    for m in &set.members {
        if let Some(None) = existing.get(&m.id) {
            bail!(
                "{} already has a folder for {} that the launcher did not create (a hand-made or \
                 developer install); move it away or remove it, then Play again",
                dir.display(),
                m.id
            );
        }
        let c = cache
            .get(&m.id, &m.version)
            .with_context(|| format!("{} {} is not in the mod cache", m.id, m.version))?;
        if !c.errors.is_empty() {
            bail!(
                "the cached copy of {} {} is damaged: {}",
                m.id,
                m.version,
                c.errors.join("; ")
            );
        }
    }
    if set.members.is_empty() && !dir.exists() {
        return Ok(done);
    }
    std::fs::create_dir_all(&dir)?;
    // Remove what the launcher put there and the set no longer wants (or wants in another build).
    for (name, marker) in &existing {
        let Some(marker) = marker else {
            done.kept_foreign.push(name.clone());
            continue;
        };
        let wanted = set
            .members
            .iter()
            .any(|m| &m.id == name && m.version == marker.version && m.sha256 == marker.sha256);
        if !wanted {
            std::fs::remove_dir_all(dir.join(name))?;
            done.removed.push(name.clone());
        }
    }
    for m in &set.members {
        let target = dir.join(&m.id);
        if target.join(MARKER_FILE).is_file() {
            continue; // the same build, kept
        }
        let scratch = dir.join(format!(".incoming-{}", m.id));
        let _ = std::fs::remove_dir_all(&scratch);
        let r = (|| -> Result<()> {
            cache.materialise(&m.id, &m.version, &scratch)?;
            let marker = Marker {
                id: m.id.clone(),
                version: m.version.clone(),
                sha256: m.sha256.clone(),
            };
            std::fs::write(scratch.join(MARKER_FILE), serde_json::to_vec(&marker)?)?;
            std::fs::rename(&scratch, &target)
                .with_context(|| format!("moving {} into place", m.id))?;
            Ok(())
        })();
        if r.is_err() {
            let _ = std::fs::remove_dir_all(&scratch);
        }
        r?;
        done.copied.push(m.id.clone());
    }
    let ids = set.ids();
    let text = format!(
        "# Written by the Poe2 launcher at Play for the selected server. `enabled` is the load order.\n{}",
        poe_hook_mods::state_text(&ids)
    );
    write_atomic(&dir.join(STATE_FILE), text.as_bytes())?;
    done.enabled = ids;
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modcache::testutil::*;

    struct Env {
        t: tempfile::TempDir,
        cache: ModCache,
    }

    fn env() -> Env {
        let t = tempfile::tempdir().unwrap();
        let cache = ModCache::new(t.path().join("cache"));
        Env { t, cache }
    }

    /// Adds `id version` with a manifest body extra (`[load]`, `[dependencies]`, ...).
    fn add(e: &Env, id: &str, version: &str, extra: &str) {
        let text = format!(
            "manifest_version = 1\nid = \"{id}\"\nname = \"{id}\"\nversion = \"{version}\"\n{extra}\n[game]\nversions = [\"0.5.5e.hotfix1\"]\nhook_api = 1\n[entry]\ndll = \"{id}.dll\"\n"
        );
        // `[load]`/`[dependencies]` sections must come before [game]: build the order by hand.
        let dir = e.t.path().join(format!("pk-{id}-{version}"));
        std::fs::create_dir_all(&dir).unwrap();
        let z = package(
            &dir,
            id,
            version,
            &[
                ("mod.toml", text.as_bytes()),
                (&format!("{id}.dll"), version.as_bytes()),
            ],
        );
        e.cache.add_package(&z, None, "zip", false).unwrap();
    }

    fn req(id: &str, v: &str) -> Requirement {
        Requirement {
            id: id.into(),
            version: v.into(),
        }
    }
    fn pin(id: &str, v: &str) -> ModPin {
        ModPin::new(id, v)
    }
    fn ids(s: &Effective) -> Vec<String> {
        s.ids()
    }

    #[test]
    fn required_mods_are_on_and_the_players_own_are_off_unless_enabled_for_the_server() {
        let e = env();
        add(&e, "req-mod", "1.0.0", "");
        add(&e, "own-mod", "1.0.0", "");
        add(&e, "other-mod", "1.0.0", "");
        let s = effective(
            &e.cache,
            &[req("req-mod", "1.0.0")],
            &[pin("own-mod", "1.0.0")],
            &[],
            "0.5.5e.hotfix1",
            1,
        );
        assert_eq!(ids(&s), ["req-mod", "own-mod"]);
        assert!(s.problems.is_empty() && s.dropped.is_empty());
        assert_eq!(
            s.members.iter().find(|m| m.id == "req-mod").unwrap().origin,
            Origin::Required
        );
        // other-mod is cached but not enabled anywhere: not loaded.
        assert!(!ids(&s).contains(&"other-mod".to_string()));
        // On another server (no requirement, no pin) nothing is loaded.
        let none = effective(&e.cache, &[], &[], &[], "0.5.5e.hotfix1", 1);
        assert!(none.members.is_empty());
        // Global pins apply to every server.
        let g = effective(
            &e.cache,
            &[],
            &[],
            &[pin("other-mod", "1.0.0")],
            "0.5.5e.hotfix1",
            1,
        );
        assert_eq!(ids(&g), ["other-mod"]);
    }

    #[test]
    fn the_servers_version_wins_over_the_players_pin_of_the_same_mod() {
        let e = env();
        add(&e, "req-mod", "1.0.0", "");
        add(&e, "req-mod", "2.0.0", "");
        let s = effective(
            &e.cache,
            &[req("req-mod", "1.0.0")],
            &[pin("req-mod", "2.0.0")],
            &[],
            "0.5.5e.hotfix1",
            1,
        );
        assert_eq!(s.members.len(), 1);
        assert_eq!(s.members[0].version, "1.0.0");
        assert_eq!(s.dropped.len(), 1);
        // Another server pins the other version.
        let s2 = effective(
            &e.cache,
            &[req("req-mod", "2.0.0")],
            &[],
            &[],
            "0.5.5e.hotfix1",
            1,
        );
        assert_eq!(s2.members[0].version, "2.0.0");
    }

    #[test]
    fn a_missing_required_mod_is_a_problem_but_a_missing_own_mod_is_only_a_note() {
        let e = env();
        add(&e, "own-mod", "1.0.0", "");
        let s = effective(
            &e.cache,
            &[req("gone-mod", "1.0.0")],
            &[pin("own-mod", "9.9.9")],
            &[],
            "0.5.5e.hotfix1",
            1,
        );
        assert_eq!(s.problems.len(), 1);
        assert!(s.problems[0].contains("gone-mod"));
        assert_eq!(s.dropped.len(), 1);
    }

    #[test]
    fn dependencies_are_pulled_in_and_ordered() {
        let e = env();
        add(&e, "lib-mod", "1.0.0", "");
        add(&e, "lib-mod", "1.5.0", "");
        add(&e, "lib-mod", "2.0.0", "");
        add(
            &e,
            "user-mod",
            "1.0.0",
            "[dependencies]\nlib-mod = \"^1\"\n",
        );
        let s = effective(
            &e.cache,
            &[],
            &[pin("user-mod", "1.0.0")],
            &[],
            "0.5.5e.hotfix1",
            1,
        );
        assert_eq!(ids(&s), ["lib-mod", "user-mod"]);
        assert_eq!(
            s.members[0].version, "1.5.0",
            "the newest that satisfies the requirement"
        );
        assert_eq!(s.members[0].origin, Origin::Dependency);
    }

    #[test]
    fn an_own_mod_without_its_dependency_or_in_conflict_is_dropped_with_a_reason() {
        let e = env();
        add(
            &e,
            "needs-mod",
            "1.0.0",
            "[dependencies]\nabsent-mod = \"*\"\n",
        );
        add(&e, "req-mod", "1.0.0", "");
        add(
            &e,
            "clash-mod",
            "1.0.0",
            "[load]\nconflicts = [\"req-mod\"]\n",
        );
        let s = effective(
            &e.cache,
            &[req("req-mod", "1.0.0")],
            &[pin("needs-mod", "1.0.0"), pin("clash-mod", "1.0.0")],
            &[],
            "0.5.5e.hotfix1",
            1,
        );
        assert_eq!(ids(&s), ["req-mod"]);
        assert!(s.problems.is_empty());
        assert!(
            s.dropped
                .iter()
                .any(|d| d.contains("needs-mod") && d.contains("absent-mod")),
            "{:?}",
            s.dropped
        );
        assert!(
            s.dropped
                .iter()
                .any(|d| d.contains("clash-mod") && d.contains("conflicts")),
            "{:?}",
            s.dropped
        );
    }

    #[test]
    fn incompatible_own_mods_are_left_out_unless_forced_for_the_game_version() {
        let e = env();
        add(&e, "old-mod", "1.0.0", "");
        let s = effective(&e.cache, &[], &[pin("old-mod", "1.0.0")], &[], "0.4.0", 1);
        assert!(s.members.is_empty() && s.dropped.len() == 1);
        let forced = ModPin {
            force: true,
            ..pin("old-mod", "1.0.0")
        };
        assert_eq!(
            ids(&effective(&e.cache, &[], &[forced], &[], "0.4.0", 1)),
            ["old-mod"]
        );
        // An ABI mismatch cannot be forced: the loader would refuse it.
        let forced = ModPin {
            force: true,
            ..pin("old-mod", "1.0.0")
        };
        let s = effective(&e.cache, &[], &[forced], &[], "0.5.5e.hotfix1", 2);
        assert!(s.members.is_empty());
    }

    #[test]
    fn apply_copies_the_set_writes_state_and_cleans_up_only_its_own() {
        let e = env();
        add(&e, "aa-mod", "1.0.0", "");
        add(&e, "bb-mod", "1.0.0", "");
        let game = e.t.path().join("game");
        let mods = game.join(MODS_DIR);
        // A developer's folder (no marker) stays and is not enabled.
        std::fs::create_dir_all(mods.join("dev-mod")).unwrap();
        std::fs::write(mods.join("dev-mod/mod.toml"), "x").unwrap();
        std::fs::write(mods.join(STATE_FILE), "enabled = [\"dev-mod\"]\n").unwrap();

        let s1 = effective(
            &e.cache,
            &[req("aa-mod", "1.0.0")],
            &[pin("bb-mod", "1.0.0")],
            &[],
            "0.5.5e.hotfix1",
            1,
        );
        let r = apply_to_game(&game, &s1, &e.cache).unwrap();
        assert_eq!(r.copied.len(), 2);
        assert_eq!(r.kept_foreign, ["dev-mod"]);
        assert!(
            mods.join("aa-mod/aa-mod.dll").is_file()
                && mods.join("aa-mod").join(MARKER_FILE).is_file()
        );
        let state =
            poe_hook_mods::parse_state(&std::fs::read_to_string(mods.join(STATE_FILE)).unwrap())
                .unwrap();
        assert_eq!(state.enabled, ["aa-mod", "bb-mod"]);
        // Same set again: nothing is copied.
        assert!(
            apply_to_game(&game, &s1, &e.cache)
                .unwrap()
                .copied
                .is_empty()
        );

        // Switching to a server that needs only aa-mod removes bb-mod from the game folder.
        let s2 = effective(
            &e.cache,
            &[req("aa-mod", "1.0.0")],
            &[],
            &[],
            "0.5.5e.hotfix1",
            1,
        );
        let r = apply_to_game(&game, &s2, &e.cache).unwrap();
        assert_eq!(r.removed, ["bb-mod"]);
        assert!(!mods.join("bb-mod").exists() && mods.join("dev-mod/mod.toml").is_file());
        // A server with no mods at all: state.toml is emptied, the files go.
        let r = apply_to_game(
            &game,
            &effective(&e.cache, &[], &[], &[], "0.5.5e.hotfix1", 1),
            &e.cache,
        )
        .unwrap();
        assert_eq!(r.removed, ["aa-mod"]);
        assert!(
            std::fs::read_to_string(mods.join(STATE_FILE))
                .unwrap()
                .contains("enabled = []")
        );
    }

    #[test]
    fn switching_between_two_servers_with_different_versions_of_one_mod() {
        let e = env();
        add(&e, "shared-mod", "1.0.0", "");
        add(&e, "shared-mod", "2.0.0", "");
        let game = e.t.path().join("game");
        let mods = game.join(MODS_DIR);
        let a = effective(
            &e.cache,
            &[req("shared-mod", "1.0.0")],
            &[],
            &[],
            "0.5.5e.hotfix1",
            1,
        );
        apply_to_game(&game, &a, &e.cache).unwrap();
        assert_eq!(
            std::fs::read(mods.join("shared-mod/shared-mod.dll")).unwrap(),
            b"1.0.0"
        );
        let b = effective(
            &e.cache,
            &[req("shared-mod", "2.0.0")],
            &[],
            &[],
            "0.5.5e.hotfix1",
            1,
        );
        let r = apply_to_game(&game, &b, &e.cache).unwrap();
        assert_eq!(r.removed, ["shared-mod"]);
        assert_eq!(r.copied, ["shared-mod"]);
        assert_eq!(
            std::fs::read(mods.join("shared-mod/shared-mod.dll")).unwrap(),
            b"2.0.0"
        );
        // And back; the cache still holds both.
        apply_to_game(&game, &a, &e.cache).unwrap();
        assert_eq!(
            std::fs::read(mods.join("shared-mod/shared-mod.dll")).unwrap(),
            b"1.0.0"
        );
        assert_eq!(e.cache.list().len(), 2);
    }

    #[test]
    fn a_foreign_folder_with_the_same_id_stops_the_apply_before_anything_changes() {
        let e = env();
        add(&e, "aa-mod", "1.0.0", "");
        add(&e, "bb-mod", "1.0.0", "");
        let game = e.t.path().join("game");
        let mods = game.join(MODS_DIR);
        std::fs::create_dir_all(mods.join("bb-mod")).unwrap();
        let s = effective(
            &e.cache,
            &[],
            &[pin("aa-mod", "1.0.0"), pin("bb-mod", "1.0.0")],
            &[],
            "0.5.5e.hotfix1",
            1,
        );
        let err = apply_to_game(&game, &s, &e.cache).unwrap_err().to_string();
        assert!(
            err.contains("bb-mod") && err.contains("did not create"),
            "{err}"
        );
        assert!(
            !mods.join("aa-mod").exists(),
            "nothing may be copied before the check passes"
        );
        assert!(!mods.join(STATE_FILE).exists());
    }

    #[test]
    fn an_empty_set_creates_nothing_in_a_game_without_a_mods_folder() {
        let e = env();
        let game = e.t.path().join("game");
        std::fs::create_dir_all(&game).unwrap();
        apply_to_game(&game, &Effective::default(), &e.cache).unwrap();
        assert!(!game.join(MODS_DIR).exists());
    }
}
