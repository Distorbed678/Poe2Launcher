//! The mod operations of [`Core`]: the mod cache, the per-server mod sets, the sync with a
//! server's advertisement, and the preparation at Play. The data model is in [`crate::modcache`]
//! (verified packages, several versions), [`crate::modsets`] (the set a profile loads and
//! writing it into the game), [`crate::modsync`] (advertisement, compare, download) and
//! [`crate::modindex`] (the mod browser's index).
//!
//! A scope is `all` (mods enabled for every server) or a server profile id.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    api::ServerInfo,
    core::{Core, now_secs},
    modcache::{Added, CachedMod, ModCache},
    mods::{Compat, HOOK_API, ModManifestView, ModPin, compat, describe_compat, resolve_order},
    modsets::{self, Applied, Effective, Origin, Requirement},
    modsync::{
        AdvertisedMod, SyncMod, SyncMode, SyncPlan, assess, compare, fetch_into_cache, source_urls,
    },
    news::{load_cache_as, save_cache_as},
    paths::write_atomic,
    profiles::Profile,
    settings::Settings,
};

/// The scope for mods that are enabled on every server.
pub const SCOPE_ALL: &str = "all";

#[derive(Debug, Serialize, Deserialize, Default)]
struct SetsFile {
    #[serde(default)]
    global: Vec<ModPin>,
}

/// The last advertisement a server gave (saved per profile, so the Mods page can show what the
/// server requires while the server is not reachable).
#[derive(Debug, Serialize, Deserialize, Default)]
struct LastAd {
    fetched_at: i64,
    mods: Vec<AdvertisedMod>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModEntry {
    pub id: String,
    pub version: String,
    pub side: String,
    pub manifest: Option<ModManifestView>,
    pub errors: Vec<String>,
    pub unknown_keys: Vec<String>,
    pub compat: Option<Compat>,
    pub compat_text: String,
    pub sha256: String,
    pub size: u64,
    /// `server`, `index`, `zip`, `folder`.
    pub source: String,
    pub added: i64,
    pub has_code: bool,
    pub server_half_left_out: bool,
    /// Enabled in the scope being shown.
    pub enabled: bool,
    /// Enabled for all servers.
    pub enabled_all: bool,
    /// The scope's server requires exactly this version.
    pub required: bool,
    /// Part of the set that Play would load in this scope.
    pub in_effect: bool,
    pub origin: Option<Origin>,
    /// Position in the load order.
    pub order: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModsView {
    pub entries: Vec<ModEntry>,
    pub scope: String,
    pub scope_name: String,
    /// What the scope's server last said it requires, compared with the cache.
    pub required: Vec<SyncMod>,
    /// Own mods that are enabled but left out, and why.
    pub dropped: Vec<String>,
    /// A required mod that cannot load.
    pub problems: Vec<String>,
    pub cache_dir: String,
    pub cache_bytes: u64,
    pub game_version: String,
    pub hook_abi: u32,
    pub mod_sync: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CacheStatus {
    pub dir: String,
    pub entries: usize,
    pub bytes: u64,
    pub unused_entries: usize,
    pub unused_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Pruned {
    pub removed: usize,
    pub bytes: u64,
}

fn scoped(scope: &str) -> &str {
    if scope.is_empty() { SCOPE_ALL } else { scope }
}

impl Core {
    // --- paths and small helpers ----------------------------------------------------------

    pub fn mod_cache(&self) -> ModCache {
        ModCache::new(self.dirs.data.join("mods").join("cache"))
    }

    fn mod_scratch(&self) -> PathBuf {
        self.dirs.data.join("mods").join("tmp")
    }

    fn sets_file(&self) -> PathBuf {
        self.dirs.config.join("modsets.json")
    }

    fn last_ad_file(&self, profile_id: &str) -> PathBuf {
        self.dirs
            .data
            .join("mods")
            .join("advertised")
            .join(format!("{profile_id}.json"))
    }

    /// The mod ABI the installed hook offers (this launcher's own when no hook is known).
    pub fn hook_abi(&self) -> u32 {
        self.hook_status()
            .ok()
            .and_then(|h| h.installed)
            .map(|i| i.mod_abi)
            .filter(|n| *n > 0)
            .unwrap_or(HOOK_API)
    }

    fn ensure_mods_allowed(&self) -> Result<()> {
        if self.version().is_official() {
            bail!("the hook's mod loader loads no mods in official (read-only) mode");
        }
        Ok(())
    }

    fn global_pins(&self) -> Vec<ModPin> {
        load_cache_as::<SetsFile>(&self.sets_file()).global
    }

    fn scope_pins(&self, scope: &str) -> Result<Vec<ModPin>> {
        if scoped(scope) == SCOPE_ALL {
            Ok(self.global_pins())
        } else {
            Ok(self.profile_store().get(scope)?.mods)
        }
    }

    fn save_scope_pins(&self, scope: &str, pins: Vec<ModPin>) -> Result<()> {
        if scoped(scope) == SCOPE_ALL {
            let text = serde_json::to_string_pretty(&SetsFile { global: pins })?;
            write_atomic(&self.sets_file(), text.as_bytes())
        } else {
            self.profile_store().set_mods(scope, pins).map(|_| ())
        }
    }

    fn last_ad(&self, profile_id: &str) -> LastAd {
        load_cache_as(&self.last_ad_file(profile_id))
    }

    fn save_last_ad(&self, profile_id: &str, mods: &[AdvertisedMod]) {
        save_cache_as(
            &self.last_ad_file(profile_id),
            &LastAd {
                fetched_at: now_secs(),
                mods: mods.to_vec(),
            },
        );
    }

    fn requirements_of(ads: &[AdvertisedMod]) -> Vec<Requirement> {
        ads.iter()
            .filter(|a| a.required)
            .map(|a| Requirement {
                id: a.id.clone(),
                version: a.version.clone(),
            })
            .collect()
    }

    // --- the library --------------------------------------------------------------------

    /// Everything in the cache with its state in `scope`.
    pub fn mods_list(&self, scope: &str) -> Result<ModsView> {
        self.ensure_mods_allowed()?;
        let scope = scoped(scope).to_owned();
        let settings = self.settings()?;
        let game = self.version().version;
        let abi = self.hook_abi();
        let cache = self.mod_cache();
        let global = self.global_pins();
        let (profile, scope_pins, ads) = if scope == SCOPE_ALL {
            (None, global.clone(), Vec::new())
        } else {
            let p = self.profile_store().get(&scope)?;
            let ads = self.last_ad(&p.id).mods;
            (Some(p.clone()), p.mods, ads)
        };
        let requirements = Self::requirements_of(&ads);
        let eff = if profile.is_some() {
            modsets::effective(&cache, &requirements, &scope_pins, &global, &game, abi)
        } else {
            modsets::effective(&cache, &[], &[], &global, &game, abi)
        };
        let members: BTreeMap<(&str, &str), (usize, Origin)> = eff
            .members
            .iter()
            .enumerate()
            .map(|(i, m)| ((m.id.as_str(), m.version.as_str()), (i, m.origin)))
            .collect();
        let entries = cache
            .list()
            .into_iter()
            .map(|c| {
                let comp = c.manifest.as_ref().map(|m| compat(m, &game, abi));
                let key = (c.meta.id.as_str(), c.meta.version.as_str());
                let member = members.get(&key).copied();
                let pinned = |pins: &[ModPin]| {
                    pins.iter()
                        .any(|p| p.id == c.meta.id && p.version == c.meta.version)
                };
                ModEntry {
                    id: c.meta.id.clone(),
                    version: c.meta.version.clone(),
                    side: c.meta.side.clone(),
                    manifest: c.manifest.as_ref().map(ModManifestView::from),
                    errors: c.errors.clone(),
                    unknown_keys: c.unknown_keys.clone(),
                    compat_text: comp
                        .as_ref()
                        .map(describe_compat)
                        .unwrap_or_else(|| "invalid".into()),
                    compat: comp,
                    sha256: c.meta.sha256.clone(),
                    size: c.meta.size,
                    source: c.meta.source.clone(),
                    added: c.meta.added,
                    has_code: c.meta.has_code,
                    server_half_left_out: c.meta.server_half_left_out,
                    enabled: pinned(&scope_pins),
                    enabled_all: pinned(&global),
                    required: requirements
                        .iter()
                        .any(|r| r.id == c.meta.id && r.version == c.meta.version),
                    in_effect: member.is_some(),
                    origin: member.map(|m| m.1),
                    order: member.map(|m| m.0),
                }
            })
            .collect();
        let required = match &profile {
            Some(p) => compare(&ads, &cache, p, &game, abi)
                .into_iter()
                .filter(|m| m.required)
                .collect(),
            None => Vec::new(),
        };
        Ok(ModsView {
            entries,
            scope_name: profile.map_or_else(|| "All servers".into(), |p| p.name),
            scope,
            required,
            dropped: eff.dropped,
            problems: eff.problems,
            cache_dir: cache.root().display().to_string(),
            cache_bytes: cache.disk_bytes(),
            game_version: game,
            hook_abi: abi,
            mod_sync: settings.mod_sync,
        })
    }

    /// Adds a local mod (a package zip or a mod folder) to the cache; with `scope`, enables it there.
    pub fn mod_install(&self, path: &str, scope: Option<&str>, replace: bool) -> Result<Added> {
        self.ensure_mods_allowed()?;
        let p = PathBuf::from(path);
        let cache = self.mod_cache();
        let added = if p.is_dir() {
            cache.add_folder(&p, replace)?
        } else {
            cache.add_package(&p, None, "zip", replace)?
        };
        if let Some(scope) = scope {
            self.mod_set_enabled(scope, &added.id, &added.version, true, false)?;
        }
        Ok(added)
    }

    /// Enables (one version per id) or disables a cached mod in a scope. Returns notes about
    /// own mods that the new set leaves out (a conflict, a missing dependency, ...).
    pub fn mod_set_enabled(
        &self,
        scope: &str,
        id: &str,
        version: &str,
        enabled: bool,
        force: bool,
    ) -> Result<Vec<String>> {
        self.ensure_mods_allowed()?;
        let scope = scoped(scope);
        let cache = self.mod_cache();
        let mut pins = self.scope_pins(scope)?;
        pins.retain(|p| p.id != id);
        if enabled {
            let m = cache
                .get(id, version)
                .ok_or_else(|| anyhow!("{id} {version} is not in the mod cache"))?;
            if !m.errors.is_empty() {
                bail!("{id} {version} is damaged: {}", m.errors.join("; "));
            }
            let c = compat(
                m.manifest.as_ref().unwrap(),
                &self.version().version,
                self.hook_abi(),
            );
            if c.blocks() && !(force && c.overridable()) {
                bail!(
                    "{id} {version} is not compatible with this setup: {}",
                    describe_compat(&c)
                );
            }
            pins.push(ModPin {
                id: id.into(),
                version: version.into(),
                force: force && c.overridable(),
            });
        }
        self.save_scope_pins(scope, pins)?;
        Ok(self.mods_list(scope)?.dropped)
    }

    /// Moves an enabled mod to `position` among the scope's enabled mods (the constraints of
    /// dependencies and `after`/`before` win at load time).
    pub fn mod_move(&self, scope: &str, id: &str, position: usize) -> Result<Vec<String>> {
        self.ensure_mods_allowed()?;
        let mut pins = self.scope_pins(scope)?;
        let from = pins
            .iter()
            .position(|p| p.id == id)
            .ok_or_else(|| anyhow!("{id} is not enabled here"))?;
        let item = pins.remove(from);
        pins.insert(position.min(pins.len()), item);
        let ids = pins.iter().map(|p| p.id.clone()).collect();
        self.save_scope_pins(scope, pins)?;
        Ok(ids)
    }

    /// The spec's order for the scope's enabled mods: constraints, ties by id.
    pub fn mod_auto_order(&self, scope: &str) -> Result<Vec<String>> {
        self.ensure_mods_allowed()?;
        let cache = self.mod_cache();
        let pins = self.scope_pins(scope)?;
        let all = cache.list();
        let mut manifests = BTreeMap::new();
        for p in &pins {
            if let Some(c) = all
                .iter()
                .find(|c| c.meta.id == p.id && c.meta.version == p.version)
                && let Some(m) = &c.manifest
            {
                manifests.insert(p.id.clone(), m);
            }
        }
        let ids: BTreeSet<String> = manifests.keys().cloned().collect();
        let order = resolve_order(&manifests, &ids, &[])
            .map_err(|cycle| anyhow!("load order cycle between: {}", cycle.join(", ")))?;
        let mut sorted: Vec<ModPin> = Vec::new();
        for id in &order {
            if let Some(p) = pins.iter().find(|p| &p.id == id) {
                sorted.push(p.clone());
            }
        }
        // Pins whose mod is not usable keep their place at the end.
        sorted.extend(pins.iter().filter(|p| !order.contains(&p.id)).cloned());
        self.save_scope_pins(scope, sorted)?;
        Ok(order)
    }

    /// Deletes one cached version and takes it out of every mod set.
    pub fn mod_remove(&self, id: &str, version: &str) -> Result<()> {
        self.ensure_mods_allowed()?;
        self.mod_cache().remove(id, version)?;
        let drop_pin = |pins: Vec<ModPin>| -> Vec<ModPin> {
            pins.into_iter()
                .filter(|p| !(p.id == id && p.version == version))
                .collect()
        };
        let global = self.global_pins();
        if global.iter().any(|p| p.id == id && p.version == version) {
            self.save_scope_pins(SCOPE_ALL, drop_pin(global))?;
        }
        for p in self.profile_store().list()? {
            if p.mods.iter().any(|m| m.id == id && m.version == version) {
                self.profile_store().set_mods(&p.id, drop_pin(p.mods))?;
            }
        }
        Ok(())
    }

    pub fn mods_dir(&self) -> Result<PathBuf> {
        Ok(self.mod_cache().root().to_path_buf())
    }

    // --- cache size and clean-up ----------------------------------------------------------

    fn used_versions(&self) -> BTreeSet<(String, String)> {
        let mut used: BTreeSet<(String, String)> = self
            .global_pins()
            .into_iter()
            .map(|p| (p.id, p.version))
            .collect();
        for p in self.profile_store().list().unwrap_or_default() {
            used.extend(p.mods.into_iter().map(|m| (m.id, m.version)));
            used.extend(
                self.last_ad(&p.id)
                    .mods
                    .into_iter()
                    .filter(|a| a.required)
                    .map(|a| (a.id, a.version)),
            );
        }
        used
    }

    pub fn mod_cache_status(&self) -> Result<CacheStatus> {
        let cache = self.mod_cache();
        let used = self.used_versions();
        let all = cache.list();
        let unused: Vec<&CachedMod> = all
            .iter()
            .filter(|c| !used.contains(&(c.meta.id.clone(), c.meta.version.clone())))
            .collect();
        Ok(CacheStatus {
            dir: cache.root().display().to_string(),
            entries: all.len(),
            bytes: cache.disk_bytes(),
            unused_entries: unused.len(),
            unused_bytes: unused
                .iter()
                .map(|c| cache.entry_bytes(&c.meta.id, &c.meta.version))
                .sum(),
        })
    }

    /// "Clear unused cached mods": removes every version that no mod set enables and no
    /// server's last advertisement requires.
    pub fn mod_cache_prune(&self) -> Result<Pruned> {
        let cache = self.mod_cache();
        let used = self.used_versions();
        let mut out = Pruned {
            removed: 0,
            bytes: 0,
        };
        for c in cache.list() {
            if used.contains(&(c.meta.id.clone(), c.meta.version.clone())) {
                continue;
            }
            let bytes = cache.entry_bytes(&c.meta.id, &c.meta.version);
            cache.remove(&c.meta.id, &c.meta.version)?;
            out.removed += 1;
            out.bytes += bytes;
        }
        // Scratch folders of interrupted downloads.
        let _ = std::fs::remove_dir_all(self.mod_scratch());
        Ok(out)
    }

    // --- download ---------------------------------------------------------------------------

    /// Downloads one mod into the cache (verified), reporting progress as `mods:progress`.
    pub(crate) async fn download_to_cache(
        &self,
        ad: &AdvertisedMod,
        urls: &[String],
        source: &str,
    ) -> Result<Added> {
        let events = self.events.clone();
        let (id, version) = (ad.id.clone(), ad.version.clone());
        let last = AtomicU64::new(0);
        let progress = move |done: u64, total: u64| {
            let prev = last.load(Ordering::Relaxed);
            if done == total || done >= prev + (256 << 10) {
                last.store(done, Ordering::Relaxed);
                events.emit(
                    "mods:progress",
                    json!({"id": id, "version": version, "done": done, "total": total}),
                );
            }
        };
        fetch_into_cache(
            ad,
            urls,
            source,
            &self.mod_cache(),
            &self.mod_scratch(),
            &progress,
        )
        .await
    }

    /// Downloads and verifies the given advertised mods; returns what was kept and what failed.
    async fn sync_download(
        &self,
        profile: &Profile,
        ads: &[AdvertisedMod],
    ) -> (Vec<String>, Vec<String>) {
        let mut applied = Vec::new();
        let mut errors = Vec::new();
        for a in ads {
            let urls = source_urls(a, profile);
            match self.download_to_cache(a, &urls, "server").await {
                Ok(_) => applied.push(format!("{} {}", a.name, a.version)),
                Err(e) => errors.push(format!("{e:#}")),
            }
        }
        (applied, errors)
    }

    // --- sync ---------------------------------------------------------------------------------

    fn build_plan(
        &self,
        profile: &Profile,
        ads: &[AdvertisedMod],
        skipped: Vec<String>,
        stale: bool,
        mode: SyncMode,
    ) -> SyncPlan {
        let mods = compare(
            ads,
            &self.mod_cache(),
            profile,
            &self.version().version,
            self.hook_abi(),
        );
        let (status, blocker, download_bytes) = assess(&mods, mode, &profile.name);
        SyncPlan {
            profile_id: profile.id.clone(),
            profile_name: profile.name.clone(),
            mode,
            status,
            mods,
            blocker,
            download_bytes,
            skipped,
            stale,
            applied: Vec::new(),
            errors: Vec::new(),
        }
    }

    fn sync_mode(&self, s: &Settings) -> SyncMode {
        SyncMode::parse(&s.mod_sync).unwrap_or_default()
    }

    /// The advertised mods that still have to be downloaded (required ones, or `only`).
    fn to_fetch<'a>(
        plan: &SyncPlan,
        ads: &'a [AdvertisedMod],
        only: Option<&[String]>,
    ) -> Vec<&'a AdvertisedMod> {
        ads.iter()
            .filter(|a| {
                plan.mods
                    .iter()
                    .find(|m| m.id == a.id)
                    .is_some_and(|m| m.needs_download())
                    && match only {
                        Some(ids) => ids.contains(&a.id),
                        None => a.required,
                    }
            })
            .collect()
    }

    /// Asks the server what it requires and compares it with the cache. In Auto mode the missing
    /// required mods are downloaded and verified right away (`applied` lists them); in Ask mode
    /// the plan only says what the dialog should offer. Never fails because the server is down:
    /// the last advertisement is used and the plan says `stale`.
    pub async fn mod_sync_check(&self, profile_id: &str) -> Result<SyncPlan> {
        self.ensure_mods_allowed()?;
        let settings = self.settings()?;
        let mode = self.sync_mode(&settings);
        let (profile, client) = self.client_for(profile_id)?;
        match client.server_info().await {
            Ok(info) => {
                self.save_last_ad(&profile.id, &info.mods.mods);
                let ads = info.mods.mods;
                let mut plan =
                    self.build_plan(&profile, &ads, info.mods.skipped.clone(), false, mode);
                if mode == SyncMode::Auto && plan.status == "auto" {
                    let fetch: Vec<AdvertisedMod> = Self::to_fetch(&plan, &ads, None)
                        .into_iter()
                        .cloned()
                        .collect();
                    let (applied, errors) = self.sync_download(&profile, &fetch).await;
                    plan = self.build_plan(&profile, &ads, info.mods.skipped, false, mode);
                    plan.applied = applied;
                    if !errors.is_empty() {
                        plan.blocker = Some(errors.join(" | "));
                        plan.status = "blocked";
                    }
                    plan.errors = errors;
                }
                Ok(plan)
            }
            Err(_) => {
                let last = self.last_ad(&profile.id);
                Ok(self.build_plan(&profile, &last.mods, Vec::new(), true, mode))
            }
        }
    }

    /// The Download button: fetches the required mods (or just `ids`, e.g. optional ones the
    /// player picked), whatever the sync mode, and returns the new plan. Nothing is kept for a
    /// mod whose hash or package check fails.
    pub async fn mod_sync_apply(
        &self,
        profile_id: &str,
        ids: Option<Vec<String>>,
    ) -> Result<SyncPlan> {
        self.ensure_mods_allowed()?;
        let settings = self.settings()?;
        let mode = self.sync_mode(&settings);
        let (profile, client) = self.client_for(profile_id)?;
        let info = client.server_info().await?;
        self.save_last_ad(&profile.id, &info.mods.mods);
        let ads = info.mods.mods;
        let plan = self.build_plan(&profile, &ads, Vec::new(), false, mode);
        let fetch: Vec<AdvertisedMod> = Self::to_fetch(&plan, &ads, ids.as_deref())
            .into_iter()
            .cloned()
            .collect();
        let (applied, errors) = self.sync_download(&profile, &fetch).await;
        let mut plan = self.build_plan(&profile, &ads, info.mods.skipped, false, mode);
        plan.applied = applied;
        plan.errors = errors;
        Ok(plan)
    }

    // --- Play -----------------------------------------------------------------------------------

    /// Before the hook is deployed: brings the cache up to what the server requires (per the Mod
    /// sync setting), computes the profile's set and writes it into the game. Refuses to go on
    /// when a required mod is missing or unusable. Not called for the official entry.
    pub async fn prepare_mods(
        &self,
        profile: &Profile,
        info: &ServerInfo,
        settings: &Settings,
    ) -> Result<()> {
        let mode = self.sync_mode(settings);
        let ads = &info.mods.mods;
        self.save_last_ad(&profile.id, ads);
        let mut plan = self.build_plan(profile, ads, Vec::new(), false, mode);
        match plan.status {
            "blocked" | "off" => bail!("{}", plan.blocker.unwrap_or_default()),
            "ask" => bail!(
                "{}. Mod sync is Ask: press Download in the dialog the launcher shows before Play \
                 (or set Mod sync to Auto)",
                plan.blocker.unwrap_or_default()
            ),
            "auto" => {
                let fetch: Vec<AdvertisedMod> = Self::to_fetch(&plan, ads, None)
                    .into_iter()
                    .cloned()
                    .collect();
                let (applied, errors) = self.sync_download(profile, &fetch).await;
                if !errors.is_empty() {
                    bail!(
                        "the server's mods could not be downloaded: {}",
                        errors.join(" | ")
                    );
                }
                if !applied.is_empty() {
                    self.emit(
                        "mods:synced",
                        json!({"profile": profile.id, "mods": applied}),
                    );
                }
                plan = self.build_plan(profile, ads, Vec::new(), false, mode);
                if plan.status != "ok" {
                    bail!(
                        "{}",
                        plan.blocker
                            .unwrap_or_else(|| "the server's mods are not ready".into())
                    );
                }
            }
            _ => {}
        }
        let eff = self.effective_for(profile, ads)?;
        if !eff.problems.is_empty() {
            bail!(
                "the mods for {} cannot be loaded: {}",
                profile.name,
                eff.problems.join("; ")
            );
        }
        for d in &eff.dropped {
            self.emit("play:warning", json!({"text": d}));
        }
        let install = self.install_dir()?;
        let cache = self.mod_cache();
        let set = eff.clone();
        let applied: Applied =
            tokio::task::spawn_blocking(move || modsets::apply_to_game(&install, &set, &cache))
                .await??;
        if !applied.kept_foreign.is_empty() {
            self.emit(
                "play:warning",
                json!({"text": format!("poe-hook-mods has folders the launcher did not create ({}); they are not enabled", applied.kept_foreign.join(", "))}),
            );
        }
        Ok(())
    }

    /// [`Core::prepare_mods`] on its own, for a profile: asks the server and prepares the mods
    /// as Play would, without starting anything.
    pub async fn mods_prepare(&self, profile_id: &str) -> Result<()> {
        self.ensure_mods_allowed()?;
        let (profile, client) = self.client_for(profile_id)?;
        let info = client.server_info().await?;
        self.prepare_mods(&profile, &info, &self.settings()?).await
    }

    fn effective_for(&self, profile: &Profile, ads: &[AdvertisedMod]) -> Result<Effective> {
        Ok(modsets::effective(
            &self.mod_cache(),
            &Self::requirements_of(ads),
            &profile.mods,
            &self.global_pins(),
            &self.version().version,
            self.hook_abi(),
        ))
    }
}
