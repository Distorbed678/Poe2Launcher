//! Mod sync and the mod sets against a mock server (advertisement, package downloads), a mock
//! mod index and temp folders: Off / Ask / Auto, verification failures that leave nothing,
//! per-server sets and `state.toml` at Play, server-only mods refused, `both` mods installing
//! their client half only, the index browser, the cache clean-up. Never starts a game.

use std::{
    collections::BTreeMap,
    io::Write,
    sync::{Arc, Mutex},
};

use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path as AxPath, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
};
use poe2_launcher_lib::{
    core::{Collector, Core},
    keystore::MemoryStore,
    paths::Dirs,
};
use serde_json::{Value, json};

fn sha(bytes: &[u8]) -> String {
    poe_hook_mods::sha256_hex(bytes)
}

/// A package zip as `poe-mod pack` would write it (mod.toml at the root).
fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut buf = Vec::new();
    {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (p, c) in files {
            w.start_file(*p, opts).unwrap();
            w.write_all(c).unwrap();
        }
        w.finish().unwrap();
    }
    buf
}

fn client_toml(id: &str, version: &str) -> String {
    format!(
        "manifest_version = 1\nid = \"{id}\"\nname = \"{id}\"\nversion = \"{version}\"\n[game]\nversions = [\"0.5.5e\"]\nhook_api = 1\n[entry]\ndll = \"{id}.dll\"\n"
    )
}

fn both_toml(id: &str, version: &str) -> String {
    format!(
        "manifest_version = 2\nid = \"{id}\"\nname = \"{id}\"\nversion = \"{version}\"\nside = \"both\"\n[game]\nversions = [\"0.5.5e\"]\n[client]\ndll = \"client/{id}.dll\"\nhook_api = 1\n[server]\nserver_api = 1\n[server.library]\nlinux = \"server/lib{id}.so\"\nwindows = \"server/{id}.dll\"\n"
    )
}

fn server_toml(id: &str, version: &str) -> String {
    format!(
        "manifest_version = 2\nid = \"{id}\"\nname = \"{id}\"\nversion = \"{version}\"\nside = \"server\"\n[server]\nserver_api = 1\n[server.library]\nlinux = \"server/lib{id}.so\"\n"
    )
}

fn client_zip(id: &str, version: &str, dll: &[u8]) -> Vec<u8> {
    zip_of(&[
        ("mod.toml", client_toml(id, version).as_bytes()),
        (&format!("{id}.dll"), dll),
    ])
}

fn both_zip(id: &str, version: &str) -> Vec<u8> {
    zip_of(&[
        ("mod.toml", both_toml(id, version).as_bytes()),
        (&format!("client/{id}.dll"), b"MZ-client"),
        (&format!("server/lib{id}.so"), b"ELF-server"),
        (&format!("server/{id}.dll"), b"MZ-server"),
    ])
}

/// What one mock server advertises and serves.
#[derive(Default)]
struct Realm {
    /// The advertisement (array form).
    ads: Mutex<Vec<Value>>,
    /// `id/version.zip` -> bytes served.
    files: Mutex<BTreeMap<String, Vec<u8>>>,
    downloads: Mutex<Vec<String>>,
}

impl Realm {
    /// Advertises (and serves) a package under its true hash.
    fn offer(&self, id: &str, version: &str, side: &str, zip: Vec<u8>, extra: Value) {
        let mut ad = json!({"id": id, "version": version, "side": side, "sha256": sha(&zip),
            "size": zip.len(), "required": true, "hook_api": 1, "game_versions": ["0.5.5e"], "name": id});
        ad.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        self.ads.lock().unwrap().push(ad);
        self.files
            .lock()
            .unwrap()
            .insert(format!("{id}/{version}.zip"), zip);
    }

    fn serve_other_bytes(&self, id: &str, version: &str, bytes: Vec<u8>) {
        self.files
            .lock()
            .unwrap()
            .insert(format!("{id}/{version}.zip"), bytes);
    }
}

async fn spawn_realm(realm: Arc<Realm>) -> String {
    let app = Router::new()
        .route(
            "/api/v1/server",
            get(|State(r): State<Arc<Realm>>| async move {
                Json(json!({"name": "Mock Realm", "version": "0.5.5e", "cluster_id": "c", "online": 1,
                    "open_registration": true, "login_host": "127.0.0.1", "login_port": 29481,
                    "instance_port": 29112, "uptime_secs": 5, "mods": r.ads.lock().unwrap().clone()}))
            }),
        )
        .route(
            "/api/v1/mods/{id}/{file}",
            get(|State(r): State<Arc<Realm>>, AxPath((id, file)): AxPath<(String, String)>| async move {
                let key = format!("{id}/{file}");
                r.downloads.lock().unwrap().push(key.clone());
                match r.files.lock().unwrap().get(&key) {
                    Some(b) => (StatusCode::OK, Bytes::from(b.clone())).into_response(),
                    None => StatusCode::NOT_FOUND.into_response(),
                }
            }),
        )
        .with_state(realm);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    url
}

struct Env {
    tmp: tempfile::TempDir,
    core: Arc<Core>,
}

fn env() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let events = Arc::new(Collector::default());
    let core = Core::new(
        Dirs::under(tmp.path()),
        Arc::new(MemoryStore::new()),
        events.clone(),
    )
    .unwrap();
    // The fixture mods and realms are made for 0.5.5e (not necessarily the newest version).
    core.set_game_version("0.5.5e").unwrap();
    Env { tmp, core }
}

impl Env {
    fn set_mode(&self, mode: &str) {
        let mut s = self.core.settings().unwrap();
        s.mod_sync = mode.into();
        self.core.set_settings(s).unwrap();
    }

    fn game_mods(&self) -> std::path::PathBuf {
        self.core.install_dir().unwrap().join("poe-hook-mods")
    }

    fn state_ids(&self) -> Vec<String> {
        let text = std::fs::read_to_string(self.game_mods().join("state.toml")).unwrap();
        poe_hook_mods::parse_state(&text).unwrap().enabled
    }

    fn cached(&self) -> Vec<String> {
        self.core
            .mod_cache()
            .list()
            .iter()
            .map(|m| format!("{} {}", m.meta.id, m.meta.version))
            .collect()
    }
}

#[tokio::test]
async fn ask_mode_offers_a_download_and_then_the_cache_is_ready() {
    let e = env();
    let realm = Arc::new(Realm::default());
    realm.offer(
        "hello-mod",
        "1.0.0",
        "client",
        client_zip("hello-mod", "1.0.0", b"MZ"),
        json!({}),
    );
    let url = spawn_realm(realm.clone()).await;
    let p = e.core.add_profile("Realm", &url, false).unwrap();

    // Default is Ask: the check only reports.
    assert_eq!(e.core.settings().unwrap().mod_sync, "ask");
    let plan = e.core.mod_sync_check(&p.id).await.unwrap();
    assert_eq!(plan.status, "ask");
    assert_eq!(plan.mods.len(), 1);
    assert!(plan.mods[0].code && plan.download_bytes > 0);
    assert!(e.cached().is_empty());
    assert!(
        realm.downloads.lock().unwrap().is_empty(),
        "Ask downloads nothing by itself"
    );

    // Play refuses until the player agreed (the dialog's Download button).
    let err = e.core.mods_prepare(&p.id).await.unwrap_err().to_string();
    assert!(err.contains("Mod sync is Ask"), "{err}");

    let plan = e.core.mod_sync_apply(&p.id, None).await.unwrap();
    assert_eq!(plan.status, "ok", "{:?}", plan.errors);
    assert_eq!(plan.applied.len(), 1);
    assert_eq!(e.cached(), ["hello-mod 1.0.0"]);
    assert_eq!(
        e.core
            .mod_cache()
            .get("hello-mod", "1.0.0")
            .unwrap()
            .meta
            .source,
        "server"
    );

    // Now Play's preparation puts the server's mod into the game and writes state.toml.
    e.core.mods_prepare(&p.id).await.unwrap();
    assert_eq!(e.state_ids(), ["hello-mod"]);
    assert!(e.game_mods().join("hello-mod/hello-mod.dll").is_file());
}

#[tokio::test]
async fn auto_mode_downloads_verifies_and_switches_by_itself() {
    let e = env();
    e.set_mode("auto");
    let realm = Arc::new(Realm::default());
    realm.offer(
        "hello-mod",
        "1.0.0",
        "client",
        client_zip("hello-mod", "1.0.0", b"MZ"),
        json!({}),
    );
    realm.offer(
        "dual-mod",
        "2.0.0",
        "both",
        both_zip("dual-mod", "2.0.0"),
        json!({"hook_api": 1}),
    );
    let url = spawn_realm(realm.clone()).await;
    let p = e.core.add_profile("Realm", &url, false).unwrap();

    let plan = e.core.mod_sync_check(&p.id).await.unwrap();
    assert_eq!(plan.status, "ok", "{:?}", plan.errors);
    assert_eq!(plan.applied.len(), 2);
    assert_eq!(e.cached(), ["dual-mod 2.0.0", "hello-mod 1.0.0"]);

    // A second check downloads nothing more.
    let before = realm.downloads.lock().unwrap().len();
    let plan = e.core.mod_sync_check(&p.id).await.unwrap();
    assert!(plan.applied.is_empty());
    assert_eq!(realm.downloads.lock().unwrap().len(), before);

    // Play: both mods are enabled, and the server half of the `both` mod never reached the disk.
    e.core.mods_prepare(&p.id).await.unwrap();
    assert_eq!(
        e.state_ids(),
        ["hello-mod", "dual-mod"],
        "the server's order"
    );
    assert!(e.game_mods().join("dual-mod/client/dual-mod.dll").is_file());
    assert!(!e.game_mods().join("dual-mod/server").exists());
    let cached = e.core.mod_cache().get("dual-mod", "2.0.0").unwrap();
    assert!(!cached.tree().join("server").exists());
    assert!(cached.meta.server_half_left_out);
}

#[tokio::test]
async fn off_mode_shows_what_is_missing_and_blocks_play() {
    let e = env();
    e.set_mode("off");
    let realm = Arc::new(Realm::default());
    realm.offer(
        "hello-mod",
        "1.0.0",
        "client",
        client_zip("hello-mod", "1.0.0", b"MZ"),
        json!({}),
    );
    let url = spawn_realm(realm.clone()).await;
    let p = e.core.add_profile("Realm", &url, false).unwrap();

    let plan = e.core.mod_sync_check(&p.id).await.unwrap();
    assert_eq!(plan.status, "off");
    assert!(plan.blocker.as_deref().unwrap().contains("Mod sync is Off"));
    assert_eq!(plan.mods[0].id, "hello-mod");
    let err = e.core.mods_prepare(&p.id).await.unwrap_err().to_string();
    assert!(
        err.contains("requires mods you do not have") && err.contains("Off"),
        "{err}"
    );
    assert!(realm.downloads.lock().unwrap().is_empty());
    assert!(e.cached().is_empty());
    assert!(!e.game_mods().exists());
}

#[tokio::test]
async fn a_tampered_package_is_never_installed() {
    let e = env();
    e.set_mode("auto");
    let realm = Arc::new(Realm::default());
    let good = client_zip("hello-mod", "1.0.0", b"MZ");
    realm.offer("hello-mod", "1.0.0", "client", good, json!({}));
    // The server now serves other bytes than it advertised (same size is not enough).
    realm.serve_other_bytes(
        "hello-mod",
        "1.0.0",
        client_zip("hello-mod", "1.0.0", b"XX"),
    );
    let url = spawn_realm(realm.clone()).await;
    let p = e.core.add_profile("Realm", &url, false).unwrap();

    let plan = e.core.mod_sync_check(&p.id).await.unwrap();
    assert_eq!(plan.status, "blocked");
    assert!(
        plan.errors[0].contains("SHA-256 mismatch"),
        "{:?}",
        plan.errors
    );
    assert!(
        e.cached().is_empty(),
        "nothing is installed after a failed check"
    );
    let leftovers = std::fs::read_dir(e.core.mod_cache().root())
        .map(|d| d.flatten().count())
        .unwrap_or(0);
    assert_eq!(leftovers, 0);
    let err = e.core.mods_prepare(&p.id).await.unwrap_err().to_string();
    assert!(err.contains("could not be downloaded"), "{err}");
    assert!(!e.game_mods().exists());

    // A package whose bytes hash correctly but whose content breaks the rules is refused too.
    let bad = zip_of(&[("mod.toml", client_toml("hello-mod", "1.0.0").as_bytes())]); // the DLL is missing
    let realm2 = Arc::new(Realm::default());
    realm2.offer("hello-mod", "1.0.0", "client", bad, json!({}));
    let url2 = spawn_realm(realm2).await;
    let p2 = e.core.add_profile("Realm 2", &url2, false).unwrap();
    let plan = e.core.mod_sync_check(&p2.id).await.unwrap();
    assert_eq!(plan.status, "blocked");
    assert!(e.cached().is_empty());

    // A download that is longer than advertised is cut off.
    let realm3 = Arc::new(Realm::default());
    realm3.offer(
        "hello-mod",
        "1.0.0",
        "client",
        client_zip("hello-mod", "1.0.0", b"MZ"),
        json!({}),
    );
    let mut long = client_zip("hello-mod", "1.0.0", b"MZ");
    long.extend_from_slice(&[0u8; 4096]);
    realm3.serve_other_bytes("hello-mod", "1.0.0", long);
    let url3 = spawn_realm(realm3).await;
    let p3 = e.core.add_profile("Realm 3", &url3, false).unwrap();
    let plan = e.core.mod_sync_check(&p3.id).await.unwrap();
    assert_eq!(plan.status, "blocked");
    assert!(plan.errors[0].contains("more than"), "{:?}", plan.errors);
    assert!(e.cached().is_empty());
}

#[tokio::test]
async fn a_server_that_advertises_nothing_keeps_working() {
    let e = env();
    let realm = Arc::new(Realm::default());
    let url = spawn_realm(realm).await;
    let p = e.core.add_profile("Realm", &url, false).unwrap();
    let plan = e.core.mod_sync_check(&p.id).await.unwrap();
    assert_eq!(plan.status, "ok");
    assert!(plan.mods.is_empty());
    e.core.mods_prepare(&p.id).await.unwrap();
    assert!(
        !e.game_mods().exists(),
        "no mods anywhere: nothing is written"
    );
}

#[tokio::test]
async fn each_server_has_its_own_set_and_play_switches_between_them() {
    let e = env();
    e.set_mode("auto");
    let realm_a = Arc::new(Realm::default());
    realm_a.offer(
        "shared-mod",
        "1.0.0",
        "client",
        client_zip("shared-mod", "1.0.0", b"V1"),
        json!({}),
    );
    let realm_b = Arc::new(Realm::default());
    realm_b.offer(
        "shared-mod",
        "2.0.0",
        "client",
        client_zip("shared-mod", "2.0.0", b"V2"),
        json!({}),
    );
    realm_b.offer(
        "only-b",
        "1.0.0",
        "client",
        client_zip("only-b", "1.0.0", b"B"),
        json!({}),
    );
    let a = e
        .core
        .add_profile("A", &spawn_realm(realm_a).await, false)
        .unwrap();
    let b = e
        .core
        .add_profile("B", &spawn_realm(realm_b).await, false)
        .unwrap();
    let c = e
        .core
        .add_profile(
            "C (no mods)",
            &spawn_realm(Arc::new(Realm::default())).await,
            false,
        )
        .unwrap();
    e.core.mod_sync_check(&a.id).await.unwrap();
    e.core.mod_sync_check(&b.id).await.unwrap();
    assert_eq!(
        e.cached(),
        ["only-b 1.0.0", "shared-mod 2.0.0", "shared-mod 1.0.0"]
    );

    // The player has their own mod, enabled for server A only.
    let own = zip_of(&[
        ("mod.toml", client_toml("own-mod", "0.1.0").as_bytes()),
        ("own-mod.dll", b"O"),
    ]);
    let own_path = e.tmp.path().join("own-mod-0.1.0.zip");
    std::fs::write(&own_path, own).unwrap();
    let added = e
        .core
        .mod_install(own_path.to_str().unwrap(), Some(&a.id), false)
        .unwrap();
    assert!(
        added.warnings.iter().any(|w| w.contains("not pinned")),
        "a bare local zip warns"
    );

    e.core.mods_prepare(&a.id).await.unwrap();
    assert_eq!(e.state_ids(), ["shared-mod", "own-mod"]);
    assert_eq!(
        std::fs::read(e.game_mods().join("shared-mod/shared-mod.dll")).unwrap(),
        b"V1"
    );

    e.core.mods_prepare(&b.id).await.unwrap();
    assert_eq!(
        e.state_ids(),
        ["shared-mod", "only-b"],
        "B: its two required mods, the player's own mod is off here"
    );
    assert_eq!(
        std::fs::read(e.game_mods().join("shared-mod/shared-mod.dll")).unwrap(),
        b"V2"
    );
    assert!(!e.game_mods().join("own-mod").exists());

    e.core.mods_prepare(&c.id).await.unwrap();
    assert!(e.state_ids().is_empty());
    assert!(!e.game_mods().join("shared-mod").exists() && !e.game_mods().join("only-b").exists());

    // Enabling for all servers applies everywhere, also on C.
    e.core
        .mod_set_enabled("all", "own-mod", "0.1.0", true, false)
        .unwrap();
    e.core.mods_prepare(&c.id).await.unwrap();
    assert_eq!(e.state_ids(), ["own-mod"]);
    // The library view shows the scope's state.
    let view = e.core.mods_list(&b.id).unwrap();
    let own_entry = view.entries.iter().find(|m| m.id == "own-mod").unwrap();
    assert!(own_entry.enabled_all && !own_entry.enabled);
    let req = view.entries.iter().find(|m| m.id == "only-b").unwrap();
    assert!(req.required && req.in_effect);
}

#[tokio::test]
async fn a_server_only_mod_is_refused_and_a_both_mod_installs_its_client_half() {
    let e = env();
    let srv = zip_of(&[
        ("mod.toml", server_toml("srv-mod", "1.0.0").as_bytes()),
        ("server/libsrv-mod.so", b"ELF"),
    ]);
    let path = e.tmp.path().join("srv-mod-1.0.0.zip");
    std::fs::write(&path, srv).unwrap();
    let err = e
        .core
        .mod_install(path.to_str().unwrap(), None, false)
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("server-only") && err.contains("poe-server mod install"),
        "{err}"
    );
    assert!(e.cached().is_empty());

    let both = both_zip("dual-mod", "1.0.0");
    let path = e.tmp.path().join("dual-mod-1.0.0.zip");
    std::fs::write(&path, both).unwrap();
    let a = e
        .core
        .mod_install(path.to_str().unwrap(), Some("all"), false)
        .unwrap();
    assert_eq!(a.side, "both");
    assert!(a.server_half_left_out);
    // On a server that requires nothing, the globally enabled `both` mod loads: client half only.
    let p = e
        .core
        .add_profile(
            "Plain",
            &spawn_realm(Arc::new(Realm::default())).await,
            false,
        )
        .unwrap();
    e.core.mods_prepare(&p.id).await.unwrap();
    assert_eq!(e.state_ids(), ["dual-mod"]);
    assert!(e.game_mods().join("dual-mod/client/dual-mod.dll").is_file());
    assert!(!e.game_mods().join("dual-mod/server").exists());
    assert!(
        !e.game_mods()
            .join("dual-mod/server/libdual-mod.so")
            .exists()
    );
}

#[tokio::test]
async fn pinning_and_removing_mods_keeps_the_sets_consistent() {
    let e = env();
    let mk = |id: &str, v: &str| {
        let p = e.tmp.path().join(format!("{id}-{v}.zip"));
        std::fs::write(&p, client_zip(id, v, v.as_bytes())).unwrap();
        e.core
            .mod_install(p.to_str().unwrap(), None, false)
            .unwrap();
    };
    mk("aa-mod", "1.0.0");
    mk("aa-mod", "1.1.0");
    mk("bb-mod", "1.0.0");
    e.core
        .mod_set_enabled("all", "aa-mod", "1.0.0", true, false)
        .unwrap();
    e.core
        .mod_set_enabled("all", "aa-mod", "1.1.0", true, false)
        .unwrap(); // one version per id
    e.core
        .mod_set_enabled("all", "bb-mod", "1.0.0", true, false)
        .unwrap();
    let v = e.core.mods_list("all").unwrap();
    assert_eq!(v.entries.iter().filter(|m| m.enabled).count(), 2);
    assert!(
        v.entries
            .iter()
            .find(|m| m.id == "aa-mod" && m.version == "1.0.0")
            .is_some_and(|m| !m.enabled)
    );
    // Reordering and auto order.
    e.core.mod_move("all", "bb-mod", 0).unwrap();
    assert_eq!(
        e.core
            .mods_list("all")
            .unwrap()
            .entries
            .iter()
            .filter(|m| m.in_effect)
            .count(),
        2
    );
    assert_eq!(e.core.mod_auto_order("all").unwrap(), ["aa-mod", "bb-mod"]);
    // Removing a version takes its pins with it.
    e.core.mod_remove("aa-mod", "1.1.0").unwrap();
    assert!(
        e.core
            .mods_list("all")
            .unwrap()
            .entries
            .iter()
            .all(|m| !(m.id == "aa-mod" && m.enabled))
    );
    assert!(
        e.core
            .mod_set_enabled("all", "aa-mod", "9.9.9", true, false)
            .is_err()
    );
    // The cache clean-up removes what no set uses.
    let st = e.core.mod_cache_status().unwrap();
    assert_eq!((st.entries, st.unused_entries), (2, 1)); // aa-mod 1.0.0 is unused
    let pr = e.core.mod_cache_prune().unwrap();
    assert_eq!(pr.removed, 1);
    assert_eq!(e.cached(), ["bb-mod 1.0.0"]);
    assert_eq!(e.core.mod_cache_status().unwrap().unused_entries, 0);
}

// --- the index ------------------------------------------------------------------------------

struct IndexServer {
    index: Mutex<String>,
    files: Mutex<BTreeMap<String, Vec<u8>>>,
    up: std::sync::atomic::AtomicBool,
}

async fn spawn_index(s: Arc<IndexServer>) -> String {
    let app = Router::new()
        .route(
            "/index.json",
            get(|State(s): State<Arc<IndexServer>>| async move {
                if !s.up.load(std::sync::atomic::Ordering::SeqCst) {
                    return StatusCode::SERVICE_UNAVAILABLE.into_response();
                }
                s.index.lock().unwrap().clone().into_response()
            }),
        )
        .route(
            "/dl/{file}",
            get(
                |State(s): State<Arc<IndexServer>>, AxPath(f): AxPath<String>| async move {
                    match s.files.lock().unwrap().get(&f) {
                        Some(b) => (StatusCode::OK, Bytes::from(b.clone())).into_response(),
                        None => StatusCode::NOT_FOUND.into_response(),
                    }
                },
            ),
        )
        .with_state(s);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    url
}

#[tokio::test]
async fn the_mod_browser_reads_the_index_installs_a_version_and_survives_offline() {
    let e = env();
    let good = client_zip("idx-mod", "1.2.0", b"MZ");
    let s = Arc::new(IndexServer {
        index: Mutex::new(String::new()),
        files: Mutex::new(BTreeMap::new()),
        up: true.into(),
    });
    let base = spawn_index(s.clone()).await;
    let ver = |v: &str, zip: &[u8], file: &str| json!({"version": v, "game_versions": ["0.5.5e"], "hook_api": 1, "sha256": sha(zip), "size": zip.len(), "url": format!("{base}/dl/{file}")});
    s.files
        .lock()
        .unwrap()
        .insert("idx-mod-1.2.0.zip".into(), good.clone());
    // 1.1.0's file is served with other content than the index's hash.
    let old = client_zip("idx-mod", "1.1.0", b"MZ-old");
    s.files.lock().unwrap().insert(
        "idx-mod-1.1.0.zip".into(),
        client_zip("idx-mod", "1.1.0", b"MZ-bad"),
    );
    *s.index.lock().unwrap() = json!({"schema": 1, "updated": "2026-10-22T00:00:00Z", "mods": [
        {"id": "idx-mod", "name": "Index mod", "description": "d", "author": "me", "side": "client", "tags": ["qol"],
         "homepage": "https://example.org", "licence": "MIT",
         "versions": [ver("1.2.0", &good, "idx-mod-1.2.0.zip"), ver("1.1.0", &old, "idx-mod-1.1.0.zip")]},
        {"id": "srv-only", "name": "Server only", "description": "", "author": "me", "side": "server", "tags": [],
         "versions": [ver("1.0.0", &good, "idx-mod-1.2.0.zip")]},
        {"id": "Broken Entry"}
    ]}).to_string();
    let url = format!("{base}/index.json");

    let view = e.core.mod_index_get_from(&url, false, true).await;
    assert!(!view.offline && view.error.is_empty());
    assert_eq!(view.mods.len(), 2);
    assert_eq!(view.skipped.len(), 1);
    let m = view.mods.iter().find(|m| m.m.id == "idx-mod").unwrap();
    assert_eq!(m.versions[0].v.version, "1.2.0");
    assert!(!m.versions[0].cached);

    // Install 1.2.0 and enable it for all servers.
    let a = e
        .core
        .mod_index_install_from("idx-mod", "1.2.0", Some("all"), true)
        .await
        .unwrap();
    assert_eq!(a.sha256, sha(&good));
    assert_eq!(
        e.core
            .mod_cache()
            .get("idx-mod", "1.2.0")
            .unwrap()
            .meta
            .source,
        "index"
    );
    assert!(e.core.mods_list("all").unwrap().entries[0].enabled);
    let view = e.core.mod_index_get_from(&url, false, true).await;
    assert!(
        view.mods
            .iter()
            .find(|m| m.m.id == "idx-mod")
            .unwrap()
            .versions[0]
            .cached
    );

    // A file that does not match the index's hash installs nothing.
    let err = e
        .core
        .mod_index_install_from("idx-mod", "1.1.0", None, true)
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("SHA-256 mismatch"), "{err}");
    assert_eq!(e.cached(), ["idx-mod 1.2.0"]);
    // A server-only mod is not installable here.
    let err = e
        .core
        .mod_index_install_from("srv-only", "1.0.0", None, true)
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("server-only"), "{err}");
    assert!(
        e.core
            .mod_index_install_from("nope", "1.0.0", None, true)
            .await
            .is_err()
    );

    // Offline: the saved copy is shown, flagged quietly. (Age the cache so the next call
    // really tries the network.)
    s.up.store(false, std::sync::atomic::Ordering::SeqCst);
    let cache_file = e.tmp.path().join("data/cache/modindex.json");
    let mut v: Value =
        serde_json::from_str(&std::fs::read_to_string(&cache_file).unwrap()).unwrap();
    v["attempted_at"] = json!(0);
    std::fs::write(&cache_file, v.to_string()).unwrap();
    let off = e.core.mod_index_get_from(&url, false, true).await;
    assert!(off.offline && !off.error.is_empty());
    assert_eq!(off.mods.len(), 2, "the saved copy stays visible");
    assert!(off.fetched_at > 0);
}
