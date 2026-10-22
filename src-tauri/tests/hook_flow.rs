//! "Client hook": finding, downloading, verifying and installing a hook package against a mock
//! GitHub API with fake packages, which DLLs Play uses (a manual directory wins), and, when
//! `POE2_TEST_HOOK_REAL=1` is set, the same against the real releases of Poe2ClientHook
//! (`real_release_is_found_downloaded_and_verified`, ignored by default: needs the network).

use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use axum::{
    Router,
    extract::{Path as AxPath, State},
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::get,
};
use poe2_launcher_lib::{
    core::{Collector, Core},
    hook::{self, REPO},
    hook_ops::DllSource,
    keystore::MemoryStore,
    paths::Dirs,
    settings::Settings,
    version::VersionFile,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

fn game() -> String {
    VersionFile::known_versions()[0].to_owned()
}

fn sha256(b: &[u8]) -> String {
    hex::encode(Sha256::digest(b))
}

fn asset_stem(version: &str, games: &[&str]) -> String {
    format!(
        "poe-hook-v{version}-game-{}-windows-x86_64",
        games.join("_")
    )
}

/// What a fake package holds and how it is spoiled.
#[derive(Clone)]
struct Pkg {
    version: String,
    games: Vec<String>,
    loader: Vec<u8>,
    core: Vec<u8>,
    /// The loader's hash in `release.json` is the hash of other bytes.
    wrong_loader_hash: bool,
    /// Extra entries `(zip name, bytes)` that `release.json` does not list.
    unlisted: Vec<(String, Vec<u8>)>,
    /// A listed path of this name is added to `release.json` and the zip.
    extra_listed: Option<String>,
    /// Replace the platform in `release.json`.
    platform: &'static str,
    /// Leave the core DLL out of release.json (and the zip).
    skip_core: bool,
}

impl Pkg {
    fn new(version: &str, games: &[&str]) -> Self {
        Self {
            version: version.into(),
            games: games.iter().map(|g| (*g).to_owned()).collect(),
            loader: format!("loader {version}").into_bytes(),
            core: format!("core {version}").into_bytes(),
            wrong_loader_hash: false,
            unlisted: vec![],
            extra_listed: None,
            platform: "windows",
            skip_core: false,
        }
    }

    fn zip(&self) -> Vec<u8> {
        let games: Vec<&str> = self.games.iter().map(String::as_str).collect();
        let stem = asset_stem(&self.version, &games);
        let mut files: Vec<(String, Vec<u8>)> = vec![
            ("poe_hook.dll".into(), self.loader.clone()),
            ("signatures/x.toml".into(), b"[build]\n".to_vec()),
        ];
        if !self.skip_core {
            files.push(("poe_hook_core.dll".into(), self.core.clone()));
        }
        if let Some(p) = &self.extra_listed {
            files.push((p.clone(), b"x".to_vec()));
        }
        let listed: Vec<Value> = files
            .iter()
            .map(|(p, b)| {
                let mut h = sha256(b);
                if p == "poe_hook.dll" && self.wrong_loader_hash {
                    h = sha256(b"something else");
                }
                json!({"path": p, "size": b.len(), "sha256": h})
            })
            .collect();
        let release = json!({
            "schema": 1, "name": stem, "hook_version": self.version, "game_versions": self.games,
            "mod_abi": 1, "platform": self.platform, "arch": "x86_64", "commit": "abc", "files": listed,
        });
        let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default();
        for (p, b) in &files {
            z.start_file(format!("{stem}/{p}"), opts).unwrap();
            z.write_all(b).unwrap();
        }
        for (p, b) in &self.unlisted {
            z.start_file(p.clone(), opts).unwrap();
            z.write_all(b).unwrap();
        }
        z.start_file(format!("{stem}/release.json"), opts).unwrap();
        z.write_all(release.to_string().as_bytes()).unwrap();
        z.finish().unwrap().into_inner()
    }
}

struct MockRelease {
    tag: String,
    prerelease: bool,
    stem: String,
    archive: Vec<u8>,
    /// What the `.sha256` file claims.
    claimed: String,
}

impl MockRelease {
    fn of(p: &Pkg) -> Self {
        let games: Vec<&str> = p.games.iter().map(String::as_str).collect();
        let archive = p.zip();
        Self {
            tag: format!("hook-v{}", p.version),
            prerelease: false,
            stem: asset_stem(&p.version, &games),
            claimed: sha256(&archive),
            archive,
        }
    }
}

#[derive(Default)]
struct Mock {
    base: Mutex<String>,
    releases: Mutex<Vec<MockRelease>>,
    list_status: Mutex<u16>,
    downloads: Mutex<u32>,
}

async fn list(State(m): State<Arc<Mock>>) -> impl IntoResponse {
    let status = *m.list_status.lock().unwrap();
    if status != 200 {
        return (
            StatusCode::from_u16(status).unwrap(),
            Json(json!({"message": "Not Found"})),
        )
            .into_response();
    }
    let base = m.base.lock().unwrap().clone();
    let out: Vec<Value> = m
        .releases
        .lock()
        .unwrap()
        .iter()
        .map(|r| {
            json!({"tag_name": r.tag, "draft": false, "prerelease": r.prerelease, "assets": [
                {"name": format!("{}.zip", r.stem), "size": r.archive.len(), "browser_download_url": format!("{base}/dl/{}.zip", r.stem)},
                {"name": format!("{}.sha256", r.stem), "size": 100, "browser_download_url": format!("{base}/dl/{}.sha256", r.stem)},
            ]})
        })
        .collect();
    Json(out).into_response()
}

async fn download(State(m): State<Arc<Mock>>, AxPath(name): AxPath<String>) -> impl IntoResponse {
    let rels = m.releases.lock().unwrap();
    for r in rels.iter() {
        if name == format!("{}.sha256", r.stem) {
            return format!("{}  {}.zip\n", r.claimed, r.stem).into_response();
        }
        if name == format!("{}.zip", r.stem) {
            *m.downloads.lock().unwrap() += 1;
            return r.archive.clone().into_response();
        }
    }
    StatusCode::NOT_FOUND.into_response()
}

async fn spawn_mock(releases: Vec<MockRelease>) -> (String, Arc<Mock>) {
    let mock = Arc::new(Mock::default());
    *mock.releases.lock().unwrap() = releases;
    *mock.list_status.lock().unwrap() = 200;
    let app = Router::new()
        .route(&format!("/repos/{REPO}/releases"), get(list))
        .route("/dl/{name}", get(download))
        .with_state(mock.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    *mock.base.lock().unwrap() = url.clone();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (url, mock)
}

struct Env {
    tmp: tempfile::TempDir,
    core: Arc<Core>,
    events: Arc<Collector>,
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
    core.set_game_version(&game()).unwrap();
    Env { tmp, core, events }
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            if e.path().is_dir() {
                out.extend(walk(&e.path()));
            } else {
                out.push(e.path());
            }
        }
    }
    out
}

fn hook_root(e: &Env) -> PathBuf {
    e.tmp.path().join("data/hook")
}

#[tokio::test]
async fn a_release_is_found_downloaded_verified_and_installed() {
    let g = game();
    let pkg = Pkg::new("0.2.0", &[&g]);
    let (api, m) = spawn_mock(vec![MockRelease::of(&pkg)]).await;
    let e = env();

    // Nothing yet: Play would stop at "client hook missing".
    let st = e.core.hook_status().unwrap();
    assert_eq!(st.source, DllSource::Missing);
    assert!(st.installed.is_none());
    let err = e
        .core
        .check_hook(&e.core.settings().unwrap())
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("Download the client hook"), "{err}");

    let c = e.core.hook_check_at(&api).await.unwrap();
    assert!(c.update_available);
    assert_eq!(c.release.version, "0.2.0");
    assert_eq!(c.release.game_versions, std::slice::from_ref(&g));

    let out = e.core.hook_install_latest_from(&api).await.unwrap();
    assert_eq!(out.source, "download");
    assert_eq!(out.sha256.as_deref(), Some(sha256(&pkg.zip()).as_str()));
    assert_eq!(out.hook.version, "0.2.0");
    assert_eq!(
        std::fs::read(out.hook.dir.join("poe_hook.dll")).unwrap(),
        pkg.loader
    );
    assert!(out.hook.dir.ends_with("data/hook/packages/0.2.0"));

    // Play uses it, and passes the pre-deploy check.
    let st = e.core.hook_status().unwrap();
    assert_eq!(
        st.source,
        DllSource::Downloaded {
            version: "0.2.0".into()
        }
    );
    assert_eq!(st.dll_dir, out.hook.dir);
    e.core
        .check_hook(&e.core.settings().unwrap())
        .await
        .unwrap();

    // The download is cleaned up and progress was reported.
    assert!(walk(&hook_root(&e).join("downloads")).is_empty());
    assert!(
        e.events
            .0
            .lock()
            .unwrap()
            .iter()
            .any(|(n, v)| n == "hook:progress" && v["done"].as_u64().unwrap_or(0) > 0)
    );

    // Nothing newer: no second download.
    assert!(!e.core.hook_check_at(&api).await.unwrap().update_available);
    let again = e.core.hook_install_latest_from(&api).await.unwrap();
    assert_eq!(again.source, "current");
    assert_eq!(*m.downloads.lock().unwrap(), 1);
    assert!(!e.core.hook_status().unwrap().busy);
}

#[tokio::test]
async fn an_update_replaces_the_old_version_and_only_the_old_one() {
    let g = game();
    let (api, m) = spawn_mock(vec![MockRelease::of(&Pkg::new("0.2.0", &[&g]))]).await;
    let e = env();
    e.core.hook_install_latest_from(&api).await.unwrap();
    // A hook for another game version stays when a newer hook for this one arrives.
    let other = Pkg::new("0.1.0", &["9.9.9z"]);
    let other_dir = hook_root(&e).join("packages");
    let tmpzip = e.tmp.path().join("other.zip");
    std::fs::write(&tmpzip, other.zip()).unwrap();
    hook::install_package(&tmpzip, &other_dir, "9.9.9z").unwrap();

    m.releases
        .lock()
        .unwrap()
        .push(MockRelease::of(&Pkg::new("0.3.0", &[&g])));
    let c = e.core.hook_check_at(&api).await.unwrap();
    assert!(c.update_available);
    assert_eq!(c.release.version, "0.3.0");
    let out = e.core.hook_install_latest_from(&api).await.unwrap();
    assert_eq!(out.hook.version, "0.3.0");
    let mut names: Vec<_> = std::fs::read_dir(&other_dir)
        .unwrap()
        .flatten()
        .map(|d| d.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(
        names,
        ["0.1.0", "0.3.0"],
        "0.2.0 was replaced, the other game's hook stays"
    );
    let st = e.core.hook_status().unwrap();
    assert_eq!(st.installed.unwrap().version, "0.3.0");
    assert_eq!(st.other_installed.len(), 1);
}

#[tokio::test]
async fn a_download_that_does_not_match_its_checksum_installs_nothing() {
    let g = game();
    let mut rel = MockRelease::of(&Pkg::new("0.2.0", &[&g]));
    rel.claimed = "0".repeat(64);
    let (api, _m) = spawn_mock(vec![rel]).await;
    let e = env();
    let err = e
        .core
        .hook_install_latest_from(&api)
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("does not match its checksum"), "{err}");
    assert!(
        walk(&hook_root(&e)).is_empty(),
        "{:?}",
        walk(&hook_root(&e))
    );
    assert!(e.core.hook_status().unwrap().installed.is_none());
    assert!(
        !e.core.hook_status().unwrap().busy,
        "a failed task does not block the next"
    );
}

#[tokio::test]
async fn a_file_that_differs_from_release_json_installs_nothing() {
    let g = game();
    let mut pkg = Pkg::new("0.2.0", &[&g]);
    pkg.wrong_loader_hash = true;
    // The archive checksum is right (the release is "honest" about the zip), the content is not.
    let (api, _m) = spawn_mock(vec![MockRelease::of(&pkg)]).await;
    let e = env();
    let err = e
        .core
        .hook_install_latest_from(&api)
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("poe_hook.dll") && err.contains("checksum in release.json"),
        "{err}"
    );
    assert!(
        walk(&hook_root(&e)).is_empty(),
        "{:?}",
        walk(&hook_root(&e))
    );
}

#[tokio::test]
async fn a_release_for_another_game_version_is_refused() {
    let (api, _m) = spawn_mock(vec![MockRelease::of(&Pkg::new("0.2.0", &["8.8.8x"]))]).await;
    let e = env();
    let err = e.core.hook_check_at(&api).await.unwrap_err().to_string();
    assert!(err.contains("8.8.8x") && err.contains(&game()), "{err}");
    assert!(e.core.hook_install_latest_from(&api).await.is_err());
    assert!(walk(&hook_root(&e)).is_empty());

    // A package whose name claims the right game but whose release.json says another.
    let g = game();
    let liar = Pkg {
        games: vec!["8.8.8x".into()],
        ..Pkg::new("0.2.0", &[&g])
    };
    let mut rel = MockRelease::of(&liar);
    rel.stem = asset_stem("0.2.0", &[&g]);
    let (api, _m) = spawn_mock(vec![rel]).await;
    let err = e
        .core
        .hook_install_latest_from(&api)
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("supports game version 8.8.8x"), "{err}");
    assert!(walk(&hook_root(&e)).is_empty());
}

#[tokio::test]
async fn a_missing_repository_is_a_clear_error() {
    let (api, m) = spawn_mock(vec![]).await;
    *m.list_status.lock().unwrap() = 404;
    let e = env();
    let err = e.core.hook_check_at(&api).await.unwrap_err().to_string();
    assert!(
        err.contains("no client hook releases") && !err.contains("{\"message\""),
        "{err}"
    );
    *m.list_status.lock().unwrap() = 403;
    assert!(
        e.core
            .hook_check_at(&api)
            .await
            .unwrap_err()
            .to_string()
            .contains("rate limit")
    );
    *m.list_status.lock().unwrap() = 200;
    let err = e.core.hook_check_at(&api).await.unwrap_err().to_string();
    assert!(err.contains("no client hook release was found"), "{err}");
}

#[tokio::test]
async fn hostile_or_broken_packages_are_refused() {
    let g = game();
    let mut cases: Vec<(&str, Pkg, &str)> = Vec::new();
    let mut p = Pkg::new("0.2.0", &[&g]);
    p.unlisted = vec![("../evil.dll".into(), b"x".to_vec())];
    cases.push(("path traversal", p, "unsafe path"));
    let mut p = Pkg::new("0.2.0", &[&g]);
    p.unlisted = vec![(
        format!("{}/extra.exe", asset_stem("0.2.0", &[&g])),
        b"x".to_vec(),
    )];
    cases.push(("an unlisted file", p, "does not list"));
    let mut p = Pkg::new("0.2.0", &[&g]);
    p.extra_listed = Some("../up.dll".into());
    cases.push(("a listed path outside", p, "unsafe path"));
    let mut p = Pkg::new("0.2.0", &[&g]);
    p.skip_core = true;
    cases.push(("no core dll", p, "does not list poe_hook_core.dll"));
    let mut p = Pkg::new("0.2.0", &[&g]);
    p.platform = "linux";
    cases.push(("wrong platform", p, "Windows x86_64"));
    for (what, pkg, want) in cases {
        let e = env();
        let (api, _m) = spawn_mock(vec![MockRelease::of(&pkg)]).await;
        let err = e.core.hook_install_latest_from(&api).await.unwrap_err();
        let err = format!("{err:#}");
        assert!(err.contains(want), "{what}: {err}");
        assert!(
            walk(&hook_root(&e)).is_empty(),
            "{what}: {:?}",
            walk(&hook_root(&e))
        );
        assert!(e.core.hook_status().unwrap().installed.is_none(), "{what}");
    }
}

#[test]
fn a_symbolic_link_in_the_zip_is_refused() {
    let g = game();
    let stem = asset_stem("0.2.0", &[&g]);
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    z.add_symlink(
        format!("{stem}/poe_hook.dll"),
        "/etc/passwd",
        zip::write::SimpleFileOptions::default(),
    )
    .unwrap();
    let bytes = z.finish().unwrap().into_inner();
    let tmp = tempfile::tempdir().unwrap();
    let zipf = tmp.path().join(format!("{stem}.zip"));
    std::fs::write(&zipf, bytes).unwrap();
    let packages = tmp.path().join("packages");
    assert!(hook::install_package(&zipf, &packages, &g).is_err());
    assert!(walk(&packages).is_empty());
}

#[tokio::test]
async fn the_manual_dll_directory_wins_and_a_damaged_download_is_caught() {
    let g = game();
    let (api, _m) = spawn_mock(vec![MockRelease::of(&Pkg::new("0.2.0", &[&g]))]).await;
    let e = env();
    let out = e.core.hook_install_latest_from(&api).await.unwrap();

    // The manual override (Settings, "Hook DLL directory") takes over without touching the download.
    let manual = e.tmp.path().join("my-build");
    std::fs::create_dir_all(&manual).unwrap();
    for n in ["poe_hook.dll", "poe_hook_core.dll"] {
        std::fs::write(manual.join(n), b"mine").unwrap();
    }
    let mut s = e.core.settings().unwrap();
    s.dll_dir = Some(manual.clone());
    e.core.set_settings(s).unwrap();
    let st = e.core.hook_status().unwrap();
    assert_eq!(st.source, DllSource::Manual);
    assert_eq!(st.dll_dir, manual);
    assert_eq!(st.manual_dir.as_deref(), Some(manual.as_path()));
    assert_eq!(
        st.installed.unwrap().version,
        "0.2.0",
        "the download is still known"
    );
    e.core
        .check_hook(&e.core.settings().unwrap())
        .await
        .unwrap();

    // Back to automatic: the download is used again; a changed DLL is caught before a deploy.
    let s = Settings {
        dll_dir: None,
        ..e.core.settings().unwrap()
    };
    e.core.set_settings(s).unwrap();
    assert!(matches!(
        e.core.hook_status().unwrap().source,
        DllSource::Downloaded { .. }
    ));
    // Same size, other bytes: only the hash notices, right before a deploy.
    std::fs::write(out.hook.dir.join("poe_hook_core.dll"), b"core 0.2.X").unwrap();
    let err = e
        .core
        .check_hook(&e.core.settings().unwrap())
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("damaged") && err.contains("download it again"),
        "{err}"
    );
    // A changed size is seen at once: not offered as installed, and the next download repairs it.
    std::fs::write(
        out.hook.dir.join("poe_hook_core.dll"),
        b"patched by someone",
    )
    .unwrap();
    assert!(e.core.hook_status().unwrap().installed.is_none());
    let fixed = e.core.hook_install_latest_from(&api).await.unwrap();
    assert_eq!(fixed.source, "download");
    e.core
        .check_hook(&e.core.settings().unwrap())
        .await
        .unwrap();
}

#[tokio::test]
async fn the_old_default_folder_still_works_when_nothing_is_downloaded() {
    let e = env();
    let legacy = e.tmp.path().join("data/hook-dlls");
    std::fs::create_dir_all(&legacy).unwrap();
    assert_eq!(e.core.hook_status().unwrap().source, DllSource::Missing);
    for n in ["poe_hook.dll", "poe_hook_core.dll"] {
        std::fs::write(legacy.join(n), b"x").unwrap();
    }
    let st = e.core.hook_status().unwrap();
    assert_eq!(st.source, DllSource::Legacy);
    assert_eq!(st.dll_dir, legacy);
}

#[tokio::test]
async fn official_read_only_uses_the_hook_of_the_newest_private_version() {
    let newest = VersionFile::latest().unwrap().version;
    let (api, _m) = spawn_mock(vec![MockRelease::of(&Pkg::new("0.2.0", &[&newest]))]).await;
    let e = env();
    e.core.set_game_version("official").unwrap();
    assert_eq!(e.core.hook_status().unwrap().game_version, newest);
    let out = e.core.hook_install_latest_from(&api).await.unwrap();
    assert_eq!(out.hook.version, "0.2.0");
    assert_eq!(e.core.hook_status().unwrap().dll_dir, out.hook.dir);
}

#[test]
fn the_steam_install_is_never_a_hook_folder() {
    let g = game();
    let tmp = tempfile::tempdir().unwrap();
    let zipf = tmp.path().join("p.zip");
    std::fs::write(&zipf, Pkg::new("0.2.0", &[&g]).zip()).unwrap();
    let bad = Path::new("/home/u/.local/share/Steam/steamapps/common/Path of Exile 2/hook");
    assert!(hook::install_package(&zipf, bad, &g).is_err());
}

/// Against the real releases of Poe2ClientHook (network). Run with:
///   POE2_TEST_HOOK_REAL=1 cargo test --test hook_flow -- --ignored real_release
/// Optional: POE2_TEST_HOOK_GAME (default: the oldest supported game version).
#[tokio::test]
#[ignore = "needs the network and a published hook release; set POE2_TEST_HOOK_REAL=1"]
async fn real_release_is_found_downloaded_and_verified() {
    if std::env::var("POE2_TEST_HOOK_REAL").as_deref() != Ok("1") {
        eprintln!("POE2_TEST_HOOK_REAL is not 1: skipping");
        return;
    }
    let e = env();
    if let Ok(g) = std::env::var("POE2_TEST_HOOK_GAME") {
        e.core.set_game_version(&g).unwrap();
    }
    let check = e.core.hook_check().await.expect("find the real release");
    eprintln!("found {} ({})", check.release.tag, check.release.asset_name);
    assert!(check.update_available);
    let out = e
        .core
        .hook_install_latest()
        .await
        .expect("download and verify");
    eprintln!(
        "installed {} in {}",
        out.hook.version,
        out.hook.dir.display()
    );
    assert_eq!(out.source, "download");
    assert!(out.sha256.is_some());
    // The checks ran on the real files; do them again from disk and look at the DLLs.
    let again = hook::verify_package(&out.hook.dir).unwrap();
    assert_eq!(again.version, out.hook.version);
    for n in ["poe_hook.dll", "poe_hook_core.dll"] {
        let bytes = std::fs::read(out.hook.dir.join(n)).unwrap();
        assert_eq!(&bytes[..2], b"MZ", "{n} is a PE file");
        eprintln!("{n}: {} bytes, sha256 {}", bytes.len(), sha256(&bytes));
    }
    assert!(matches!(
        e.core.hook_status().unwrap().source,
        DllSource::Downloaded { .. }
    ));
    e.core
        .check_hook(&e.core.settings().unwrap())
        .await
        .unwrap();
    assert!(!e.core.hook_check().await.unwrap().update_available);
}
