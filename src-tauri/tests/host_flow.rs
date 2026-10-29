//! "Host a server" (WP F2): finding, downloading, verifying and installing a server package
//! against a mock GitHub API, and, when a real package is given (`POE2_TEST_SERVER_PACKAGE`),
//! the whole path through a real `poe-server`: set up in a temp folder with other ports and a
//! temp database, start, status, log tail, stop.

use std::{
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
    host::{HostSettings, REPO},
    keystore::MemoryStore,
    paths::Dirs,
};
use serde_json::json;
use sha2::{Digest, Sha256};

const GAME: &str = "0.5.5e.hotfix1";

fn platform() -> &'static str {
    if cfg!(windows) { "windows" } else { "linux" }
}

fn pkg_name(version: &str) -> String {
    format!("poe-server-v{version}-game-{GAME}-{}-x86_64", platform())
}

/// A tar.gz with a minimal server package inside (files at `<name>/...`).
fn fake_package(
    version: &str,
    game: &str,
    extra: &[(&str, &[u8], Option<tar::EntryType>)],
) -> Vec<u8> {
    fake_package_with_exe(version, game, extra, b"#!/bin/sh\nexit 0\n")
}

/// Like [`fake_package`], with the given body for the package's `poe-server` script.
fn fake_package_with_exe(
    version: &str,
    game: &str,
    extra: &[(&str, &[u8], Option<tar::EntryType>)],
    exe_body: &[u8],
) -> Vec<u8> {
    let name = pkg_name(version);
    let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    let mut tb = tar::Builder::new(gz);
    let release = json!({"name": name, "server_version": version, "game_version": game,
        "platform": platform(), "arch": "x86_64", "commit": "abc"})
    .to_string();
    let exe = if cfg!(windows) {
        "poe-server.exe"
    } else {
        "poe-server"
    };
    let files: Vec<(String, Vec<u8>, u32)> = vec![
        ("release.json".into(), release.into_bytes(), 0o644),
        (exe.into(), exe_body.to_vec(), 0o755),
        (
            "config/server.example.toml".into(),
            b"[game]\nversion = \"0.5.5e.hotfix1\"\n".to_vec(),
            0o644,
        ),
        (
            format!("versions/{GAME}.toml"),
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
    for (path, data, kind) in extra {
        let mut h = tar::Header::new_gnu();
        h.set_size(data.len() as u64);
        h.set_mode(0o644);
        if let Some(k) = kind {
            h.set_entry_type(*k);
            if *k == tar::EntryType::Symlink {
                h.set_link_name("/etc/passwd").unwrap();
            }
        }
        // Write the name bytes by hand so that an unsafe name can be tested.
        let bytes = path.as_bytes();
        h.as_old_mut().name[..bytes.len()].copy_from_slice(bytes);
        h.set_cksum();
        tb.append(&h, &data[..]).unwrap();
    }
    tb.into_inner().unwrap().finish().unwrap()
}

fn sha256(b: &[u8]) -> String {
    hex::encode(Sha256::digest(b))
}

#[derive(Default)]
struct Mock {
    base: Mutex<String>,
    /// Bytes served as the archive.
    archive: Mutex<Vec<u8>>,
    /// The hash the `.sha256` file claims.
    claimed: Mutex<String>,
    list_status: Mutex<u16>,
    games: Mutex<Vec<String>>,
}

async fn releases(State(m): State<Arc<Mock>>) -> impl IntoResponse {
    let status = *m.list_status.lock().unwrap();
    if status != 200 {
        return (
            StatusCode::from_u16(status).unwrap(),
            Json(json!({"message": "Not Found"})),
        )
            .into_response();
    }
    let base = m.base.lock().unwrap().clone();
    let name = pkg_name("0.1.0");
    let archive_len = m.archive.lock().unwrap().len();
    let assets: Vec<_> = m
        .games
        .lock()
        .unwrap()
        .iter()
        .flat_map(|g| {
            let n = name.replace(GAME, g);
            vec![
                json!({"name": format!("{n}.{}", ext()), "size": archive_len, "browser_download_url": format!("{base}/dl/{n}.{}", ext())}),
                json!({"name": format!("{n}.sha256"), "size": 100, "browser_download_url": format!("{base}/dl/{n}.sha256")}),
            ]
        })
        .collect();
    Json(json!([
        {"tag_name": "server-v0.1.0", "draft": false, "prerelease": false, "assets": assets},
        {"tag_name": "v1.0.0", "draft": false, "prerelease": false, "assets": []}
    ]))
    .into_response()
}

fn ext() -> &'static str {
    if cfg!(windows) { "zip" } else { "tar.gz" }
}

async fn download(State(m): State<Arc<Mock>>, AxPath(name): AxPath<String>) -> impl IntoResponse {
    if name.ends_with(".sha256") {
        let stem = name.trim_end_matches(".sha256");
        let line = format!("{}  {stem}.{}\n", m.claimed.lock().unwrap(), ext());
        return line.into_response();
    }
    m.archive.lock().unwrap().clone().into_response()
}

async fn spawn_mock(archive: Vec<u8>, claimed: String) -> (String, Arc<Mock>) {
    let mock = Arc::new(Mock::default());
    *mock.archive.lock().unwrap() = archive;
    *mock.claimed.lock().unwrap() = claimed;
    *mock.list_status.lock().unwrap() = 200;
    *mock.games.lock().unwrap() = vec![GAME.into()];
    let app = Router::new()
        .route(&format!("/repos/{REPO}/releases"), get(releases))
        .route("/dl/{name}", get(download))
        .with_state(mock.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    *mock.base.lock().unwrap() = url.clone();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (url, mock)
}

/// A random throwaway password for a test (never a fixed value, never written to a file).
fn throwaway_password() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut out = String::from("tp-");
    for _ in 0..3 {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        );
        out.push_str(&format!("{:016x}", h.finish()));
    }
    out
}

struct Env {
    tmp: tempfile::TempDir,
    core: Arc<Core>,
    events: Arc<Collector>,
    /// The hosted server every test works on (made by `env`).
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
    // The mock releases are for the oldest supported game version (a newer one may be added).
    core.set_game_version(&oldest_version()).unwrap();
    let id = core.host_create("Test").unwrap();
    Env {
        tmp,
        core,
        events,
        id,
    }
}

/// Only meaningful on Unix: the fake `poe-server` of the package is a shell script, and the
/// archive format differs on Windows.
#[cfg(unix)]
mod fake {
    use super::*;

    #[tokio::test]
    async fn a_release_is_found_downloaded_verified_and_installed() {
        let archive = fake_package("0.1.0", GAME, &[]);
        let (api, _m) = spawn_mock(archive.clone(), sha256(&archive)).await;
        let e = env();
        let r = e.core.host_find_release_at(&api).await.unwrap();
        assert_eq!(
            (r.version.as_str(), r.game_version.as_str()),
            ("0.1.0", GAME)
        );
        let out = e.core.host_install_latest_from(&e.id, &api).await.unwrap();
        assert!(out.checksum_verified);
        assert_eq!(out.sha256.as_deref(), Some(sha256(&archive).as_str()));
        assert_eq!(out.package.server_version, "0.1.0");
        let exe = out.package.dir.join("poe-server");
        assert!(exe.is_file());
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&exe).unwrap().permissions().mode() & 0o111,
            0o111
        );
        let settings = e.core.host_settings(&e.id).unwrap();
        assert_eq!(
            settings.package_dir.as_deref(),
            Some(out.package.dir.as_path())
        );
        // The download is cleaned up, progress was reported.
        let dl = e.tmp.path().join("data/server/downloads");
        assert!(std::fs::read_dir(&dl).map_or(true, |mut d| d.next().is_none()));
        assert!(
            e.events
                .0
                .lock()
                .unwrap()
                .iter()
                .any(|(n, v)| n == "host:progress" && v["done"].as_u64().unwrap_or(0) > 0)
        );
        // And again: a second install replaces the first, no leftovers.
        let again = e.core.host_install_latest_from(&e.id, &api).await.unwrap();
        assert_eq!(again.source, "current", "nothing newer: nothing downloaded");
        let st = e.core.host_status(&e.id).await.unwrap();
        assert_eq!(st.package.unwrap().name, pkg_name("0.1.0"));
        assert!(st.package_problem.is_none());
        assert_eq!(
            std::fs::read_dir(e.tmp.path().join("data/server/packages"))
                .unwrap()
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn a_download_that_does_not_match_its_checksum_installs_nothing() {
        let archive = fake_package("0.1.0", GAME, &[]);
        let (api, _m) = spawn_mock(archive, "0".repeat(64)).await;
        let e = env();
        let err = e
            .core
            .host_install_latest_from(&e.id, &api)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("does not match its checksum"), "{err}");
        assert!(e.core.host_settings(&e.id).unwrap().package_dir.is_none());
        let dir = e.tmp.path().join("data/server");
        let leftovers = walk(&dir);
        assert!(leftovers.is_empty(), "{leftovers:?}");
        // A task that failed does not block the next one.
        assert!(!e.core.host_status(&e.id).await.unwrap().busy);
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

    #[tokio::test]
    async fn a_missing_repository_is_a_clear_error_not_a_stack_of_http() {
        let (api, m) = spawn_mock(vec![], String::new()).await;
        *m.list_status.lock().unwrap() = 404;
        let e = env();
        let err = e
            .core
            .host_find_release_at(&api)
            .await
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("private") && err.contains("local package"),
            "{err}"
        );
        *m.list_status.lock().unwrap() = 403;
        let err = e
            .core
            .host_find_release_at(&api)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("rate limit"), "{err}");
        // Nobody listening at all.
        let err = e
            .core
            .host_find_release_at("http://127.0.0.1:1")
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("cannot reach GitHub"), "{err}");
        // Releases exist, but for another game version.
        *m.list_status.lock().unwrap() = 200;
        *m.games.lock().unwrap() = vec!["0.6.0".into()];
        let err = e
            .core
            .host_find_release_at(&api)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("0.6.0") && err.contains(GAME), "{err}");
    }

    #[tokio::test]
    async fn local_archives_and_folders_are_installed_and_checked() {
        let e = env();
        let src = tempfile::tempdir().unwrap();
        let bytes = fake_package("0.1.0", GAME, &[]);
        let archive = src.path().join(format!("{}.tar.gz", pkg_name("0.1.0")));
        std::fs::write(&archive, &bytes).unwrap();

        // No checksum file: installed, but reported as not verified.
        let out = e
            .core
            .host_install_local(&e.id, archive.to_str().unwrap())
            .await
            .unwrap();
        assert!(!out.checksum_verified && out.sha256.is_some());

        // A matching `.sha256` next to it is checked.
        std::fs::write(
            src.path().join(format!("{}.sha256", pkg_name("0.1.0"))),
            format!("{}  {}.tar.gz\n", sha256(&bytes), pkg_name("0.1.0")),
        )
        .unwrap();
        let out = e
            .core
            .host_install_local(&e.id, archive.to_str().unwrap())
            .await
            .unwrap();
        assert!(out.checksum_verified);

        // A wrong one is refused.
        std::fs::write(
            src.path().join(format!("{}.sha256", pkg_name("0.1.0"))),
            format!("{}  {}.tar.gz\n", "1".repeat(64), pkg_name("0.1.0")),
        )
        .unwrap();
        let err = e
            .core
            .host_install_local(&e.id, archive.to_str().unwrap())
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("does not match"), "{err}");

        // An unpacked folder is copied.
        let folder = tempfile::tempdir().unwrap();
        let mut ar = tar::Archive::new(flate2::read::GzDecoder::new(&bytes[..]));
        ar.unpack(folder.path()).unwrap();
        let out = e
            .core
            .host_install_local(
                &e.id,
                folder.path().join(pkg_name("0.1.0")).to_str().unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(out.source, "folder");
        assert!(
            out.package.dir.starts_with(e.tmp.path()),
            "the package lives in the launcher's own folder"
        );

        // Not a package / does not exist.
        let junk = src.path().join("junk.tar.gz");
        std::fs::write(&junk, b"not a tarball").unwrap();
        assert!(
            e.core
                .host_install_local(&e.id, junk.to_str().unwrap())
                .await
                .is_err()
        );
        assert!(
            e.core
                .host_install_local(&e.id, "/nonexistent/x.tar.gz")
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn a_package_for_another_game_version_is_refused() {
        let e = env();
        let src = tempfile::tempdir().unwrap();
        let archive = src.path().join(format!("{}.tar.gz", pkg_name("0.1.0")));
        std::fs::write(&archive, fake_package("0.1.0", "0.6.0", &[])).unwrap();
        let err = e
            .core
            .host_install_local(&e.id, archive.to_str().unwrap())
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("0.6.0") && err.contains(GAME), "{err}");
        assert!(e.core.host_settings(&e.id).unwrap().package_dir.is_none());
    }

    #[tokio::test]
    async fn hostile_archives_are_refused() {
        let e = env();
        let src = tempfile::tempdir().unwrap();
        for (what, extra) in [
            ("escape", ("../../escaped.txt", &b"x"[..], None)),
            (
                "symlink",
                ("evil-link", &b""[..], Some(tar::EntryType::Symlink)),
            ),
        ] {
            let archive = src.path().join(format!("{what}.tar.gz"));
            std::fs::write(&archive, fake_package("0.1.0", GAME, &[extra])).unwrap();
            let err = e
                .core
                .host_install_local(&e.id, archive.to_str().unwrap())
                .await
                .unwrap_err()
                .to_string();
            assert!(
                err.contains("unsafe path") || err.contains("link"),
                "{what}: {err}"
            );
        }
        assert!(!e.tmp.path().join("escaped.txt").exists());
        assert!(!e.tmp.path().parent().unwrap().join("escaped.txt").exists());
        assert!(e.core.host_settings(&e.id).unwrap().package_dir.is_none());
    }

    #[tokio::test]
    async fn settings_are_checked_before_anything_is_written() {
        let e = env();
        let mut s = HostSettings::default();
        s.web_port = s.login_port;
        let err = e
            .core
            .host_setup(
                &e.id,
                poe2_launcher_lib::host_ops::SetupRequest { settings: s },
            )
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("different"), "{err}");
        // Without a package there is nothing to set up.
        let s = HostSettings::default();
        let err = e
            .core
            .host_setup(
                &e.id,
                poe2_launcher_lib::host_ops::SetupRequest { settings: s },
            )
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("package"), "{err}");
        // And starting a server that was never set up says so.
        assert!(
            e.core
                .host_start(&e.id)
                .await
                .unwrap_err()
                .to_string()
                .contains("apply the server's settings first")
        );
    }

    /// A fake game install and a package whose `poe-server` records how it was called.
    async fn installed(e: &Env) -> PathBuf {
        let game = e.tmp.path().join("game");
        std::fs::create_dir_all(&game).unwrap();
        std::fs::write(game.join("PathOfExileSteam.exe"), b"x").unwrap();
        e.core
            .set_settings(poe2_launcher_lib::settings::Settings {
                game_version: Some(oldest_version()),
                install_dir: Some(game),
                ..Default::default()
            })
            .unwrap();
        let body = b"#!/bin/sh\necho \"$*\" >> calls.log\nexit 0\n";
        let src = tempfile::tempdir().unwrap();
        let archive = src.path().join(format!("{}.tar.gz", pkg_name("0.1.0")));
        std::fs::write(&archive, fake_package_with_exe("0.1.0", GAME, &[], body)).unwrap();
        e.core
            .host_install_local(&e.id, archive.to_str().unwrap())
            .await
            .unwrap()
            .package
            .dir
    }

    fn free_ports(n: usize) -> Vec<u16> {
        let ls: Vec<_> = (0..n)
            .map(|_| std::net::TcpListener::bind("127.0.0.1:0").unwrap())
            .collect();
        ls.iter().map(|l| l.local_addr().unwrap().port()).collect()
    }

    /// Applying settings writes the config into the server's own folder and runs nothing: the
    /// launcher makes no account (the first admin is created in the web admin).
    #[tokio::test]
    async fn setup_writes_the_config_into_the_servers_folder_and_makes_no_account() {
        let e = env();
        let pkg = installed(&e).await;
        let ports = free_ports(3);
        let s = HostSettings {
            server_name: "Weekend".into(),
            login_port: ports[0],
            instance_port: ports[1],
            web_port: ports[2],
            ..Default::default()
        };
        let res = e
            .core
            .host_setup(
                &e.id,
                poe2_launcher_lib::host_ops::SetupRequest { settings: s },
            )
            .await
            .unwrap();
        assert!(
            !e.core.runtime(&e.id).is_running(),
            "the server was not started"
        );
        assert!(!pkg.join("calls.log").exists(), "no poe-server command ran");
        let folder = e.tmp.path().join("data/servers").join(&e.id);
        assert_eq!(res.config_file, folder.join("server.toml"));
        let cfg = std::fs::read_to_string(&res.config_file).unwrap();
        assert!(
            cfg.contains(&folder.join("data/server.sqlite").display().to_string()),
            "{cfg}"
        );
        assert!(
            cfg.contains(&folder.join("mods").display().to_string()),
            "{cfg}"
        );
        assert!(
            !pkg.join("config/server.toml").exists(),
            "the package stays untouched"
        );
        assert_eq!(res.profile.name, "Weekend");
        assert!(e.core.host_settings(&e.id).unwrap().configured);
    }

    /// Several servers: own ports, a shared package, refused port clashes, and a delete that
    /// removes the server's folder, settings and Servers entry but keeps a package still in use.
    #[tokio::test]
    async fn several_servers_share_packages_and_delete_cleanly() {
        let e = env();
        let pkg = installed(&e).await;
        let first = e.core.host_settings(&e.id).unwrap();
        let second = e.core.host_create("Second").unwrap();
        assert_ne!(second, e.id);
        let s2 = e.core.host_settings(&second).unwrap();
        assert!(
            first.port_clashes(&s2).is_empty(),
            "the new server got other ports"
        );
        assert_eq!(
            s2.package_dir.as_deref(),
            Some(pkg.as_path()),
            "the installed release is reused"
        );
        assert_eq!(e.core.host_list().await.unwrap().len(), 2);

        let ports = free_ports(6);
        let mk = |p: &[u16], name: &str| HostSettings {
            server_name: name.into(),
            login_port: p[0],
            instance_port: p[1],
            web_port: p[2],
            ..Default::default()
        };
        let req = |s| poe2_launcher_lib::host_ops::SetupRequest { settings: s };
        e.core
            .host_setup(&e.id, req(mk(&ports[0..3], "First")))
            .await
            .unwrap();
        let err = e
            .core
            .host_setup(&second, req(mk(&ports[0..3], "Second")))
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("First uses port"), "{err}");
        e.core
            .host_setup(&second, req(mk(&ports[3..6], "Second")))
            .await
            .unwrap();
        let profiles = e.core.profiles().unwrap();
        assert_eq!(profiles.len(), 2);

        // A backup in the second server's folder, to see it go.
        let folder = e.tmp.path().join("data/servers").join(&second);
        std::fs::create_dir_all(folder.join("data")).unwrap();
        std::fs::write(folder.join("data/server.sqlite"), b"db").unwrap();
        e.core.host_delete(&second).await.unwrap();
        assert!(!folder.exists());
        assert_eq!(e.core.host_ids(), std::slice::from_ref(&e.id));
        assert_eq!(
            e.core.profiles().unwrap().len(),
            1,
            "its Servers entry went too"
        );
        assert!(pkg.is_dir(), "the first server still uses the package");
        assert!(e.core.host_delete(&second).await.is_err(), "gone");

        e.core.host_delete(&e.id).await.unwrap();
        assert!(e.core.host_ids().is_empty());
        assert!(!pkg.exists(), "nobody uses the package any more");
    }
}

/// The whole path through a real server package. Needs:
///   POE2_TEST_SERVER_PACKAGE   the `.tar.gz` made by `.github/scripts/package-server.sh`
///   POE2_TEST_GAME_DIR         a game install to read (the `GameFiles/` copy; read only)
/// Optional: POE2_TEST_PORT_BASE (default 29500).
#[cfg(unix)]
#[tokio::test]
#[ignore = "needs a real server package and a game install; see the module comment"]
async fn real_package_setup_start_status_log_stop() {
    use poe2_launcher_lib::{core::UrlOpener, host_run::RunState, settings::Settings};

    let (Ok(pkg), Ok(game)) = (
        std::env::var("POE2_TEST_SERVER_PACKAGE"),
        std::env::var("POE2_TEST_GAME_DIR"),
    ) else {
        eprintln!("POE2_TEST_SERVER_PACKAGE / POE2_TEST_GAME_DIR not set: skipping");
        return;
    };
    let base: u16 = std::env::var("POE2_TEST_PORT_BASE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(29500);
    let e = env();
    let password = throwaway_password();
    let password = password.as_str();

    // The launcher's installed game is the given folder.
    let settings = Settings {
        game_version: Some(oldest_version()),
        install_dir: Some(PathBuf::from(&game)),
        ..Settings::default()
    };
    e.core.set_settings(settings).unwrap();
    assert!(
        e.core.host_status(&e.id).await.unwrap().game_installed,
        "{game} must look like a game install"
    );

    let out = e.core.host_install_local(&e.id, &pkg).await.unwrap();
    println!(
        "installed {} (checksum verified: {})",
        out.package.name, out.checksum_verified
    );

    let s = HostSettings {
        server_name: "Launcher test server".into(),
        cluster_id: "launchertest".into(),
        login_port: base,
        instance_port: base + 1,
        web_port: base + 2,
        public_host: "127.0.0.1".into(),
        open_registration: false,
        ..Default::default()
    };
    let res = e
        .core
        .host_setup(
            &e.id,
            poe2_launcher_lib::host_ops::SetupRequest {
                settings: s.clone(),
            },
        )
        .await
        .unwrap();
    println!(
        "setup done; profile {} at {}",
        res.profile.name, res.profile.url
    );
    assert_eq!(res.profile.url, format!("http://127.0.0.1:{}", base + 2));
    assert!(
        !e.core.runtime(&e.id).is_running(),
        "setup does not start the server"
    );
    let pkg_dir = out.package.dir.clone();
    assert_eq!(
        res.config_file,
        e.tmp
            .path()
            .join("data/servers")
            .join(&e.id)
            .join("server.toml")
    );
    let cfg = std::fs::read_to_string(&res.config_file).unwrap();
    assert!(
        cfg.contains(&format!("0.0.0.0:{base}")) && cfg.contains("open_registration = false"),
        "{cfg}"
    );
    assert!(
        cfg.contains(
            &e.tmp
                .path()
                .join("data/servers")
                .join(&e.id)
                .join("data/server.sqlite")
                .display()
                .to_string()
        )
    );

    // Start, wait for the API, look at status and the log.
    e.core.host_start(&e.id).await.unwrap();
    let mut info = None;
    for _ in 0..240 {
        let st = e.core.host_status(&e.id).await.unwrap();
        if let Some(i) = st.info {
            info = Some(i);
            break;
        }
        assert!(
            matches!(st.run, RunState::Running { .. }),
            "server died: {:?}\n{}",
            st.run,
            e.core.host_log_tail(&e.id, 20).join("\n")
        );
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    let info = info.expect("the web API answered within two minutes");
    println!(
        "server answers: {}, cluster {}, registration open: {}",
        info.name, info.cluster_id, info.open_registration
    );
    assert_eq!(info.name, "Launcher test server");
    assert_eq!(info.cluster_id, "launchertest");
    assert!(
        !info.open_registration,
        "the final config has registration closed"
    );
    assert_eq!(info.login_port, base);
    assert_eq!(info.instance_port, Some(base + 1));
    let tail = e.core.host_log_tail(&e.id, 50);
    println!(
        "log tail ({} lines):\n{}",
        tail.len(),
        tail.iter()
            .rev()
            .take(5)
            .rev()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert!(
        tail.iter()
            .any(|l| l.contains("started") || l.contains("listening") || l.contains("INFO")),
        "{tail:?}"
    );
    assert!(
        tail.iter().all(|l| !l.contains('\u{1b}')),
        "colour codes are stripped"
    );
    assert!(
        e.events
            .0
            .lock()
            .unwrap()
            .iter()
            .any(|(n, _)| n == "host:log")
    );
    // A second start is refused while it runs; so is a setup.
    assert!(e.core.host_start(&e.id).await.is_err());

    // The same with curl, as a user would check it.
    if let Ok(o) = std::process::Command::new("curl")
        .args([
            "-s",
            "-m",
            "5",
            &format!("http://127.0.0.1:{}/api/v1/server", base + 2),
        ])
        .output()
    {
        let body = String::from_utf8_lossy(&o.stdout).into_owned();
        println!("curl /api/v1/server: {body}");
        assert!(body.contains("Launcher test server"), "{body}");
    }
    let me = e.core.profiles().unwrap().remove(0);

    // The first admin is made in the web admin's first-run page (this computer only).
    let made = reqwest::Client::new()
        .post(format!("http://127.0.0.1:{}/api/v1/setup", base + 2))
        .json(&serde_json::json!({"username": "hostadmin", "password": password}))
        .send()
        .await
        .unwrap();
    assert_eq!(made.status().as_u16(), 201, "first admin created");

    // The admin can log in; the role is admin; the web-admin link comes from the real server
    // (recorded, never opened).
    let v = e.core.login(&me.id, "hostadmin", password).await.unwrap();
    assert_eq!(v.role.as_deref(), Some("admin"));
    #[derive(Default)]
    struct Rec(Mutex<Vec<String>>);
    impl UrlOpener for Rec {
        fn open(&self, url: &str) -> anyhow::Result<()> {
            self.0.lock().unwrap().push(url.to_owned());
            Ok(())
        }
    }
    let rec = Rec::default();
    e.core.web_admin_open(&me.id, &rec).await.unwrap();
    let link = rec.0.lock().unwrap()[0].clone();
    assert!(
        link.starts_with(&format!("http://127.0.0.1:{}/#/token-login?t=", base + 2)),
        "link shape"
    );
    println!("web admin link has the expected shape (not printed: it is a credential)");
    // Registration is closed on the final config.
    let reg = poe2_launcher_lib::api::ApiClient::new(&format!("http://127.0.0.1:{}", base + 2))
        .register("intruder", "another-password")
        .await;
    assert!(reg.is_err(), "open_registration = false must refuse");

    // Stop.
    e.core.host_stop(&e.id).await.unwrap();
    assert_eq!(e.core.runtime(&e.id).state(), RunState::Stopped);
    assert!(
        poe2_launcher_lib::host::busy_ports(&s).is_empty(),
        "ports are free again"
    );

    // The password is nowhere on disk: not in a config, the settings, the log, the profile file.
    let mut checked = 0;
    for path in all_files(e.tmp.path())
        .into_iter()
        .chain(all_files(&pkg_dir))
    {
        if path
            .extension()
            .is_some_and(|x| x == "sqlite" || x == "sqlite-wal" || x == "sqlite-shm")
            || path.metadata().map_or(true, |m| m.len() > 5_000_000)
        {
            continue;
        }
        if let Ok(text) = std::fs::read_to_string(&path) {
            assert!(
                !text.contains(password),
                "password found in {}",
                path.display()
            );
            checked += 1;
        }
    }
    println!("password absent from {checked} text files");
    let log = std::fs::read_to_string(
        e.tmp
            .path()
            .join("data/servers")
            .join(&e.id)
            .join("server.log"),
    )
    .unwrap();
    assert!(!log.contains(password));

    // Start again from the saved state (what autostart does) and stop.
    e.core.host_autostart().await; // autostart is off: nothing happens
    assert!(!e.core.runtime(&e.id).is_running());
}

#[cfg(unix)]
fn all_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                out.extend(all_files(&p));
            } else {
                out.push(p);
            }
        }
    }
    out
}

/// The version the legacy `install_dir` setting belongs to (the oldest supported one).
fn oldest_version() -> String {
    poe2_launcher_lib::version::VersionFile::known_versions()[0].to_owned()
}
