//! The server update (host_update.rs) against a mock GitHub API and fake server packages: the
//! one-click old -> new update of a running, configured server (stop, backup, install, config
//! carried over, start), the rollback when the new server does not come up, a failed download
//! or backup (nothing changes), the fallback repository and both package layouts.
//!
//! The "server" of a package is a shell script, so this is Unix only.
#![cfg(unix)]

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU32, Ordering},
    },
    time::Duration,
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
    host::{FALLBACK_REPO, HostSettings, REPO},
    host_ops::SetupRequest,
    host_update::UpdateTuning,
    keystore::MemoryStore,
    paths::Dirs,
};
use serde_json::json;
use sha2::{Digest, Sha256};

const GAME: &str = "0.5.5e.hotfix1";

fn pkg_name(version: &str) -> String {
    format!("poe-server-v{version}-game-{GAME}-linux-x86_64")
}

fn repo_name(full: &str) -> &str {
    full.rsplit('/').next().unwrap()
}

const OLD_EXAMPLE: &str =
    "[game]\nversion = \"0.5.5e.hotfix1\"\ngame_dir = \"GameFiles\"\n\n[web]\nrate_limit = 60\n";
const NEW_EXAMPLE: &str = "[game]\nversion = \"0.5.5e.hotfix1\"\ngame_dir = \"GameFiles\"\n\n[web]\nrate_limit = 120\ncors = \"none\"\n\n[mods]\nrequire_client_mods = false\n";

/// A `poe-server` that answers the one-shot `account` commands and otherwise runs until it is
/// stopped, noting each start (and its version) in `started.log` of its folder.
fn running_exe(version: &str) -> Vec<u8> {
    format!(
        "#!/bin/sh\n\
         if [ \"$1\" = account ]; then\n\
           [ \"$2\" = create ] && IFS= read -r pw\n\
           exit 0\n\
         fi\n\
         echo {version} >> started.log\n\
         exec sleep 30\n"
    )
    .into_bytes()
}

/// A `poe-server` that damages the database like a bad migration and ends at once.
fn crashing_exe(db: &Path) -> Vec<u8> {
    format!(
        "#!/bin/sh\n\
         if [ \"$1\" = account ]; then exit 0; fi\n\
         echo broken > '{}'\n\
         exit 3\n",
        db.display()
    )
    .into_bytes()
}

/// A tar.gz package. `new_layout`: the version file under `modules/protocol/versions/`.
fn package(version: &str, new_layout: bool, exe: &[u8], example: &str) -> Vec<u8> {
    let name = pkg_name(version);
    let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    let mut tb = tar::Builder::new(gz);
    let release = json!({"name": name, "server_version": version, "game_version": GAME,
        "platform": "linux", "arch": "x86_64", "commit": "abc"})
    .to_string();
    let version_file = if new_layout {
        format!("modules/protocol/versions/{GAME}.toml")
    } else {
        format!("versions/{GAME}.toml")
    };
    let files: Vec<(String, Vec<u8>, u32)> = vec![
        ("release.json".into(), release.into_bytes(), 0o644),
        ("poe-server".into(), exe.to_vec(), 0o755),
        (
            "config/server.example.toml".into(),
            example.as_bytes().to_vec(),
            0o644,
        ),
        (
            version_file,
            b"version = \"0.5.5e.hotfix1\"\n".to_vec(),
            0o644,
        ),
        (
            "third_party/dat-schema/schema-poe2.min.json".into(),
            b"{}".to_vec(),
            0o644,
        ),
        (
            "modules/protocol/opcodes/0.5.5e.hotfix1/a.toml".into(),
            b"x = 1\n".to_vec(),
            0o644,
        ),
    ];
    for (path, data, mode) in files {
        let mut h = tar::Header::new_gnu();
        h.set_size(data.len() as u64);
        h.set_mode(mode);
        h.set_cksum();
        tb.append_data(&mut h, format!("{name}/{path}"), &data[..])
            .unwrap();
    }
    tb.into_inner().unwrap().finish().unwrap()
}

fn sha256(b: &[u8]) -> String {
    hex::encode(Sha256::digest(b))
}

/// One release in the mock: its archive and the hash its `.sha256` file claims.
#[derive(Clone)]
struct Rel {
    version: String,
    archive: Vec<u8>,
    claimed: String,
    notes: String,
}

impl Rel {
    fn new(version: &str, archive: Vec<u8>) -> Self {
        let claimed = sha256(&archive);
        Self {
            version: version.into(),
            archive,
            claimed,
            notes: format!("Notes of {version}"),
        }
    }
}

#[derive(Clone)]
enum Repo {
    Status(u16),
    Releases(Vec<Rel>),
}

#[derive(Default)]
struct Mock {
    base: Mutex<String>,
    repos: Mutex<HashMap<String, Repo>>,
    /// Requests to the release lists.
    list_hits: AtomicU32,
}

async fn list(
    State(m): State<Arc<Mock>>,
    AxPath((_o, repo)): AxPath<(String, String)>,
) -> impl IntoResponse {
    m.list_hits.fetch_add(1, Ordering::SeqCst);
    let state = m.repos.lock().unwrap().get(&repo).cloned();
    let base = m.base.lock().unwrap().clone();
    match state {
        None | Some(Repo::Status(404)) => {
            (StatusCode::NOT_FOUND, Json(json!({"message": "Not Found"}))).into_response()
        }
        Some(Repo::Status(c)) => StatusCode::from_u16(c).unwrap().into_response(),
        Some(Repo::Releases(rels)) => {
            let out: Vec<_> = rels
                .iter()
                .map(|r| {
                    let n = pkg_name(&r.version);
                    json!({
                        "tag_name": format!("server-v{}", r.version), "draft": false, "prerelease": false,
                        "body": r.notes, "html_url": format!("https://github.com/Distorbed678/{repo}/releases/tag/server-v{}", r.version),
                        "assets": [
                            {"name": format!("{n}.tar.gz"), "size": r.archive.len(),
                             "browser_download_url": format!("{base}/dl/{repo}/{}/{n}.tar.gz", r.version)},
                            {"name": format!("{n}.sha256"), "size": 100,
                             "browser_download_url": format!("{base}/dl/{repo}/{}/{n}.sha256", r.version)},
                        ]
                    })
                })
                .collect();
            Json(out).into_response()
        }
    }
}

async fn download(
    State(m): State<Arc<Mock>>,
    AxPath((repo, version, name)): AxPath<(String, String, String)>,
) -> impl IntoResponse {
    let Some(Repo::Releases(rels)) = m.repos.lock().unwrap().get(&repo).cloned() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(r) = rels.iter().find(|r| r.version == version) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if name.ends_with(".sha256") {
        let stem = name.trim_end_matches(".sha256");
        return format!("{}  {stem}.tar.gz\n", r.claimed).into_response();
    }
    r.archive.clone().into_response()
}

async fn spawn_mock(repos: Vec<(&str, Repo)>) -> (String, Arc<Mock>) {
    let mock = Arc::new(Mock::default());
    for (full, state) in repos {
        mock.repos
            .lock()
            .unwrap()
            .insert(repo_name(full).to_owned(), state);
    }
    let app = Router::new()
        .route("/repos/{owner}/{repo}/releases", get(list))
        .route("/dl/{repo}/{version}/{name}", get(download))
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
    /// The hosted server of the test.
    id: String,
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
    let game = tmp.path().join("game");
    std::fs::create_dir_all(&game).unwrap();
    std::fs::write(game.join("PathOfExileSteam.exe"), b"x").unwrap();
    let v = poe2_launcher_lib::version::VersionFile::known_versions()[0].to_owned();
    core.set_settings(poe2_launcher_lib::settings::Settings {
        game_version: Some(v),
        install_dir: Some(game),
        ..Default::default()
    })
    .unwrap();
    let id = core.host_create("Test").unwrap();
    Env {
        tmp,
        core,
        events,
        id,
    }
}

fn tuning() -> UpdateTuning {
    UpdateTuning {
        ready_wait: Duration::from_millis(1500),
    }
}

fn free_ports(n: usize) -> Vec<u16> {
    let ls: Vec<_> = (0..n)
        .map(|_| std::net::TcpListener::bind("127.0.0.1:0").unwrap())
        .collect();
    ls.iter().map(|l| l.local_addr().unwrap().port()).collect()
}

impl Env {
    /// The server's own folder.
    fn folder(&self) -> PathBuf {
        self.tmp.path().join("data/servers").join(&self.id)
    }

    /// The server's config.
    fn cfg(&self) -> PathBuf {
        self.folder().join("server.toml")
    }

    fn db(&self) -> PathBuf {
        self.folder().join("data/server.sqlite")
    }

    /// Installs `archive` as the server, sets it up (config written) and makes a real database
    /// with one row.
    async fn install_and_setup(&self, archive: &[u8]) -> PathBuf {
        let src = tempfile::tempdir().unwrap();
        let file = src.path().join("pkg.tar.gz");
        std::fs::write(&file, archive).unwrap();
        let out = self
            .core
            .host_install_local(&self.id, file.to_str().unwrap())
            .await
            .unwrap();
        let ports = free_ports(3);
        self.core
            .host_setup(
                &self.id,
                SetupRequest {
                    settings: HostSettings {
                        login_port: ports[0],
                        instance_port: ports[1],
                        web_port: ports[2],
                        ..Default::default()
                    },
                },
            )
            .await
            .unwrap();
        std::fs::create_dir_all(self.db().parent().unwrap()).unwrap();
        let c = rusqlite::Connection::open(self.db()).unwrap();
        c.execute_batch("CREATE TABLE accounts(name TEXT); INSERT INTO accounts VALUES ('boss');")
            .unwrap();
        out.package.dir
    }

    fn rows(&self) -> i64 {
        rusqlite::Connection::open(self.db())
            .unwrap()
            .query_row("SELECT count(*) FROM accounts", [], |r| r.get(0))
            .unwrap()
    }

    fn package_dirs(&self) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(self.tmp.path().join("data/server/packages"))
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v
    }

    fn backups(&self) -> Vec<String> {
        poe2_launcher_lib::backup::list(&self.folder().join("backups"))
            .unwrap()
            .into_iter()
            .map(|b| b.kind)
            .collect()
    }
}

/// The starts noted so far, after waiting (up to 3 s) for at least `at_least` of them: the
/// script writes its line a moment after the process exists.
async fn started_after(pkg: &Path, at_least: usize) -> Vec<String> {
    for _ in 0..60 {
        if started(pkg).len() >= at_least {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    started(pkg)
}

fn started(pkg: &Path) -> Vec<String> {
    std::fs::read_to_string(pkg.join("started.log"))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

#[tokio::test]
async fn a_running_server_is_updated_with_its_config_kept_and_started_again() {
    let new = Rel::new(
        "0.2.0",
        package("0.2.0", true, &running_exe("0.2.0"), NEW_EXAMPLE),
    );
    let (api, mock) = spawn_mock(vec![(REPO, Repo::Releases(vec![new]))]).await;
    let e = env();
    let old_pkg = e
        .install_and_setup(&package("0.1.0", false, &running_exe("0.1.0"), OLD_EXAMPLE))
        .await;
    // The user changed a setting by hand (the web admin's config editor does the same).
    let cfg = e.cfg();
    let text = std::fs::read_to_string(&cfg).unwrap();
    assert!(text.contains("rate_limit = 60"));
    std::fs::write(&cfg, text.replace("rate_limit = 60", "rate_limit = 77")).unwrap();
    e.core.host_start(&e.id).await.unwrap();
    assert!(e.core.runtime(&e.id).is_running());

    // The check is quiet and tells what is on offer.
    let check = e.core.host_update_check_at(&e.id, &api).await;
    assert!(check.update_available, "{check:?}");
    assert_eq!(check.installed.as_deref(), Some("0.1.0"));
    let r = check.release.unwrap();
    assert_eq!((r.version.as_str(), r.repo.as_str()), ("0.2.0", REPO));
    assert_eq!(r.notes, "Notes of 0.2.0");
    assert!(
        r.html_url
            .unwrap()
            .ends_with("/Poe2Server/releases/tag/server-v0.2.0")
    );
    assert!(
        e.core.runtime(&e.id).is_running(),
        "a check changes nothing"
    );
    assert_eq!(mock.list_hits.load(Ordering::SeqCst), 1);

    let out = e
        .core
        .host_update_apply_with(&e.id, &api, &tuning())
        .await
        .unwrap();
    assert!(out.updated && out.was_running && out.restarted);
    assert_eq!((out.from.as_str(), out.to.as_str()), ("0.1.0", "0.2.0"));
    assert!(
        out.backup
            .as_deref()
            .is_some_and(|b| b.contains("pre-update"))
    );

    // The new package runs, the old folder is gone, the settings point at the new one.
    let new_pkg = out.package.dir.clone();
    assert_eq!(e.package_dirs(), [pkg_name("0.2.0")]);
    assert_eq!(
        e.core.host_settings(&e.id).unwrap().package_dir.as_deref(),
        Some(new_pkg.as_path())
    );
    assert!(e.core.runtime(&e.id).is_running());
    assert_eq!(started_after(&new_pkg, 1).await, ["0.2.0"]);
    assert!(!old_pkg.exists());

    // The config: the user's value kept, the new release's keys with their defaults, the
    // wizard's answers in place.
    let new_cfg: toml_edit::DocumentMut =
        std::fs::read_to_string(e.cfg()).unwrap().parse().unwrap();
    assert_eq!(new_cfg["web"]["rate_limit"].as_integer(), Some(77));
    assert_eq!(new_cfg["web"]["cors"].as_str(), Some("none"));
    assert_eq!(
        new_cfg["mods"]["require_client_mods"].as_bool(),
        Some(false)
    );
    assert_eq!(new_cfg["cluster"]["id"].as_str(), Some("main"));
    assert_eq!(
        new_cfg["database"]["path"].as_str(),
        Some(e.db().to_str().unwrap())
    );

    // The backup of the database is a real, restorable copy.
    assert_eq!(e.backups(), ["pre-update"]);
    assert_eq!(e.rows(), 1);
    assert!(
        e.events
            .0
            .lock()
            .unwrap()
            .iter()
            .any(|(n, v)| n == "host:installed" && v["package"] == pkg_name("0.2.0"))
    );

    // Nothing newer now: a second apply touches nothing.
    let again = e
        .core
        .host_update_apply_with(&e.id, &api, &tuning())
        .await
        .unwrap();
    assert!(!again.updated);
    assert_eq!(started(&new_pkg), ["0.2.0"]);
    let check = e.core.host_update_check_at(&e.id, &api).await;
    assert!(!check.update_available);
    e.core.host_stop(&e.id).await.unwrap();
}

#[tokio::test]
async fn a_new_server_that_does_not_come_up_is_rolled_back() {
    let (api, _m) = spawn_mock(vec![]).await;
    let e = env();
    let crashing = Rel::new(
        "0.2.0",
        package("0.2.0", true, &crashing_exe(&e.db()), NEW_EXAMPLE),
    );
    *_m.repos.lock().unwrap() =
        HashMap::from([(repo_name(REPO).to_owned(), Repo::Releases(vec![crashing]))]);
    let old_pkg = e
        .install_and_setup(&package("0.1.0", false, &running_exe("0.1.0"), OLD_EXAMPLE))
        .await;
    let old_cfg = std::fs::read_to_string(e.cfg()).unwrap();
    e.core.host_start(&e.id).await.unwrap();

    let err = e
        .core
        .host_update_apply_with(&e.id, &api, &tuning())
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("0.2.0") && err.contains("failed"), "{err}");
    assert!(err.contains("old version 0.1.0 was put back"), "{err}");
    assert!(err.contains("database was restored"), "{err}");

    // The old package is installed and running again, the new one is gone, the database is back
    // (the crashing server had overwritten it).
    assert_eq!(e.package_dirs(), [pkg_name("0.1.0")]);
    assert_eq!(
        e.core.host_settings(&e.id).unwrap().package_dir.as_deref(),
        Some(old_pkg.as_path())
    );
    assert_eq!(std::fs::read_to_string(e.cfg()).unwrap(), old_cfg);
    assert!(e.core.runtime(&e.id).is_running());
    assert_eq!(started_after(&old_pkg, 2).await, ["0.1.0", "0.1.0"]);
    assert_eq!(e.rows(), 1);
    assert!(!e.core.host_status(&e.id).await.unwrap().busy);
    let kinds = e.backups();
    assert!(
        kinds.contains(&"pre-update".to_owned()) && kinds.contains(&"pre-restore".to_owned()),
        "{kinds:?}"
    );
    e.core.host_stop(&e.id).await.unwrap();
}

#[tokio::test]
async fn a_bad_download_changes_nothing_and_the_server_keeps_running() {
    let mut bad = Rel::new(
        "0.2.0",
        package("0.2.0", true, &running_exe("0.2.0"), NEW_EXAMPLE),
    );
    bad.claimed = "0".repeat(64);
    let (api, _m) = spawn_mock(vec![(REPO, Repo::Releases(vec![bad]))]).await;
    let e = env();
    let old_pkg = e
        .install_and_setup(&package("0.1.0", false, &running_exe("0.1.0"), OLD_EXAMPLE))
        .await;
    e.core.host_start(&e.id).await.unwrap();
    let err = e
        .core
        .host_update_apply_with(&e.id, &api, &tuning())
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("does not match its checksum"), "{err}");
    assert!(
        e.core.runtime(&e.id).is_running(),
        "the old server was never stopped"
    );
    assert_eq!(started_after(&old_pkg, 1).await, ["0.1.0"]);
    assert_eq!(e.package_dirs(), [pkg_name("0.1.0")]);
    assert!(e.backups().is_empty());
    assert!(!e.core.host_status(&e.id).await.unwrap().busy);
    e.core.host_stop(&e.id).await.unwrap();
}

#[tokio::test]
async fn a_failed_backup_stops_the_update_before_anything_is_replaced() {
    let new = Rel::new(
        "0.2.0",
        package("0.2.0", true, &running_exe("0.2.0"), NEW_EXAMPLE),
    );
    let (api, _m) = spawn_mock(vec![(REPO, Repo::Releases(vec![new]))]).await;
    let e = env();
    let old_pkg = e
        .install_and_setup(&package("0.1.0", false, &running_exe("0.1.0"), OLD_EXAMPLE))
        .await;
    // A file where the backup folder should be: the backup cannot be made.
    std::fs::write(e.folder().join("backups"), b"in the way").unwrap();
    e.core.host_start(&e.id).await.unwrap();
    let err = e
        .core
        .host_update_apply_with(&e.id, &api, &tuning())
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("could not be backed up") && err.contains("started again"),
        "{err}"
    );
    assert_eq!(e.package_dirs(), [pkg_name("0.1.0")]);
    assert!(e.core.runtime(&e.id).is_running());
    assert_eq!(
        started_after(&old_pkg, 2).await,
        ["0.1.0", "0.1.0"],
        "stopped, then started again"
    );
    assert_eq!(
        e.core.host_settings(&e.id).unwrap().package_dir.as_deref(),
        Some(old_pkg.as_path())
    );
    e.core.host_stop(&e.id).await.unwrap();
}

/// Poe2Server has no release that fits (404): the two old releases in Poe2ModdingProject are
/// the fallback, and their package layout (`versions/`) is accepted next to the new one.
#[tokio::test]
async fn the_fallback_repository_and_both_layouts_work_for_a_stopped_server() {
    let v2_old_layout = Rel::new(
        "0.2.0",
        package("0.2.0", false, &running_exe("0.2.0"), NEW_EXAMPLE),
    );
    let (api, mock) = spawn_mock(vec![
        (REPO, Repo::Status(404)),
        (FALLBACK_REPO, Repo::Releases(vec![v2_old_layout])),
    ])
    .await;
    let e = env();
    // Installed: 0.1.0 in the new layout.
    let old_pkg = e
        .install_and_setup(&package("0.1.0", true, &running_exe("0.1.0"), OLD_EXAMPLE))
        .await;
    let check = e.core.host_update_check_at(&e.id, &api).await;
    assert!(check.update_available, "{check:?}");
    assert_eq!(check.release.unwrap().repo, FALLBACK_REPO);
    assert_eq!(
        mock.list_hits.load(Ordering::SeqCst),
        2,
        "both repositories asked"
    );

    let out = e
        .core
        .host_update_apply_with(&e.id, &api, &tuning())
        .await
        .unwrap();
    assert!(out.updated && !out.was_running && !out.restarted);
    assert_eq!(e.package_dirs(), [pkg_name("0.2.0")]);
    assert!(!old_pkg.exists());
    assert!(
        !e.core.runtime(&e.id).is_running(),
        "a stopped server stays stopped"
    );
    assert!(
        out.package
            .dir
            .join("versions/0.5.5e.hotfix1.toml")
            .is_file()
    );
    let cfg = std::fs::read_to_string(e.cfg()).unwrap();
    assert!(cfg.contains("cors = \"none\""));

    // Poe2Server answering with a release wins over the fallback.
    let newer = Rel::new(
        "0.3.0",
        package("0.3.0", true, &running_exe("0.3.0"), NEW_EXAMPLE),
    );
    mock.repos
        .lock()
        .unwrap()
        .insert(repo_name(REPO).into(), Repo::Releases(vec![newer]));
    let check = e.core.host_update_check_at(&e.id, &api).await;
    let r = check.release.unwrap();
    assert_eq!((r.version.as_str(), r.repo.as_str()), ("0.3.0", REPO));
}

#[tokio::test]
async fn nothing_is_asked_of_github_without_an_installed_server() {
    let (api, mock) = spawn_mock(vec![]).await;
    let e = env();
    let check = e.core.host_update_check_at(&e.id, &api).await;
    assert!(!check.update_available && check.installed.is_none() && check.error.is_none());
    assert_eq!(mock.list_hits.load(Ordering::SeqCst), 0);
    let err = e
        .core
        .host_update_apply_with(&e.id, &api, &tuning())
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("no server to update"), "{err}");

    // With a server but no way to reach GitHub: quiet, with the reason.
    let _ = e
        .install_and_setup(&package("0.1.0", true, &running_exe("0.1.0"), OLD_EXAMPLE))
        .await;
    let check = e
        .core
        .host_update_check_at(&e.id, "http://127.0.0.1:1")
        .await;
    assert!(!check.update_available);
    assert_eq!(check.installed.as_deref(), Some("0.1.0"));
    assert!(
        check
            .error
            .is_some_and(|m| m.contains("cannot reach GitHub"))
    );
}
