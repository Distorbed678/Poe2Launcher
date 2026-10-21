//! The command layer against a mock web API and a mock keyring: profiles, login, logout,
//! register, the game-token exchange and the whole Play preparation (deploy, token file, a fake
//! Proton). Also runs against a real poe-server when `POE_LAUNCHER_TEST_URL`,
//! `POE_LAUNCHER_TEST_USER` and `POE_LAUNCHER_TEST_PASS` are set (see the last test).

use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use poe2_launcher_lib::{
    core::{Collector, Core, PlayState},
    keystore::{MemoryStore, TokenStore},
    paths::Dirs,
    settings::Settings,
};
use serde_json::{Value, json};

const LAUNCHER_TOKEN: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const GAME_TOKEN: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

#[derive(Default)]
struct Mock {
    live: Mutex<HashSet<String>>,
    registered: Mutex<Vec<String>>,
    game_token_calls: Mutex<u32>,
}

fn bearer(h: &HeaderMap) -> Option<String> {
    Some(
        h.get("authorization")?
            .to_str()
            .ok()?
            .strip_prefix("Bearer ")?
            .to_owned(),
    )
}

fn account() -> Value {
    json!({"id": 1, "username": "Alice", "role": "player", "created_at": 0, "disabled": false, "has_password": true})
}

async fn spawn_mock() -> (String, Arc<Mock>) {
    let mock = Arc::new(Mock::default());
    let app = Router::new()
        .route(
            "/api/v1/server",
            get(|| async {
                Json(json!({"name": "Mock Realm", "version": "0.5.5e", "cluster_id": "c", "online": 3,
                    "open_registration": true, "login_host": "127.0.0.1", "login_port": 29481,
                    "instance_port": 29112, "gateway_region": "Local", "uptime_secs": 5}))
            }),
        )
        .route(
            "/api/v1/login",
            post(|State(m): State<Arc<Mock>>, Json(b): Json<Value>| async move {
                if b["password"] != "correct horse" || b["kind"] != "launcher" {
                    return (StatusCode::UNAUTHORIZED, Json(json!({"error": "invalid username or password"})));
                }
                m.live.lock().unwrap().insert(LAUNCHER_TOKEN.into());
                (StatusCode::OK, Json(json!({"token": LAUNCHER_TOKEN, "kind": "launcher",
                    "expires_at": 4_102_444_800i64, "account": account()})))
            }),
        )
        .route(
            "/api/v1/register",
            post(|State(m): State<Arc<Mock>>, Json(b): Json<Value>| async move {
                let name = b["username"].as_str().unwrap_or_default().to_owned();
                if m.registered.lock().unwrap().contains(&name) {
                    return (StatusCode::CONFLICT, Json(json!({"error": "name taken"})));
                }
                m.registered.lock().unwrap().push(name);
                (StatusCode::CREATED, Json(account()))
            }),
        )
        .route(
            "/api/v1/logout",
            post(|State(m): State<Arc<Mock>>, h: HeaderMap| async move {
                match bearer(&h) {
                    Some(t) if m.live.lock().unwrap().remove(&t) => StatusCode::NO_CONTENT,
                    _ => StatusCode::UNAUTHORIZED,
                }
            }),
        )
        .route(
            "/api/v1/game-token",
            post(|State(m): State<Arc<Mock>>, h: HeaderMap| async move {
                match bearer(&h) {
                    Some(t) if m.live.lock().unwrap().contains(&t) => {
                        *m.game_token_calls.lock().unwrap() += 1;
                        (StatusCode::OK, Json(json!({"token": GAME_TOKEN, "kind": "game_remember",
                            "expires_at": 4_102_444_800i64, "account": account()})))
                    }
                    _ => (StatusCode::UNAUTHORIZED, Json(json!({"error": "bad token"}))),
                }
            }),
        )
        .with_state(mock.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (url, mock)
}

struct Env {
    _tmp: tempfile::TempDir,
    core: Arc<Core>,
    tokens: Arc<MemoryStore>,
    events: Arc<Collector>,
}

fn env() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let tokens = Arc::new(MemoryStore::new());
    let events = Arc::new(Collector::default());
    let core = Core::new(Dirs::under(tmp.path()), tokens.clone(), events.clone()).unwrap();
    Env {
        _tmp: tmp,
        core,
        tokens,
        events,
    }
}

#[tokio::test]
async fn profiles_login_logout_and_register() {
    let (url, mock) = spawn_mock().await;
    let e = env();
    let p = e.core.add_profile("Mock", &url, false).unwrap();
    assert!(!p.logged_in);
    assert_eq!(
        e.core.settings().unwrap().selected_profile.as_deref(),
        Some(p.id.as_str())
    );

    let info = e.core.server_info(&p.id).await.unwrap();
    assert_eq!(
        (info.name.as_str(), info.online, info.open_registration),
        ("Mock Realm", 3, true)
    );
    assert_eq!(info.instance_port, Some(29112));

    // Wrong password: a plain message, nothing stored.
    let bad = e.core.login(&p.id, "alice", "nope").await.unwrap_err();
    assert_eq!(bad.to_string(), "wrong account name or password");
    assert!(e.tokens.is_empty());

    let view = e
        .core
        .login(&p.id, " alice ", "correct horse")
        .await
        .unwrap();
    assert!(view.logged_in);
    assert_eq!(view.account.as_deref(), Some("Alice"));
    // The token is in the (mock) keyring and nowhere on disk.
    assert_eq!(e.tokens.get(&p.id).unwrap().unwrap().token, LAUNCHER_TOKEN);
    for dir in [&e.core.dirs.config, &e.core.dirs.data] {
        if dir.exists() {
            for f in walk(dir) {
                assert!(
                    !std::fs::read_to_string(&f)
                        .unwrap_or_default()
                        .contains(LAUNCHER_TOKEN),
                    "{f:?}"
                );
            }
        }
    }
    assert!(e.core.profiles().unwrap()[0].logged_in);

    assert_eq!(
        e.core
            .register(&p.id, "bob", "long password")
            .await
            .unwrap()
            .username,
        "Alice"
    );
    assert!(
        e.core
            .register(&p.id, "bob", "long password")
            .await
            .unwrap_err()
            .to_string()
            .contains("name taken")
    );

    // Logout revokes on the server and forgets locally.
    let out = e.core.logout(&p.id).await.unwrap();
    assert!(out.revoked, "{}", out.message);
    assert!(mock.live.lock().unwrap().is_empty());
    assert!(e.tokens.is_empty());
    assert!(!e.core.logout(&p.id).await.unwrap().revoked);
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(walk(&p))
        } else {
            out.push(p)
        }
    }
    out
}

#[tokio::test]
async fn logout_works_offline_and_changing_a_profile_url_drops_its_token() {
    let (url, _mock) = spawn_mock().await;
    let e = env();
    let p = e.core.add_profile("Mock", &url, false).unwrap();
    e.core.login(&p.id, "alice", "correct horse").await.unwrap();
    // Another address: the token must not be sent there.
    let moved = e
        .core
        .update_profile(&p.id, "Mock", "http://127.0.0.1:9", false)
        .unwrap();
    assert!(!moved.logged_in && e.tokens.is_empty());

    e.core
        .login(&p.id, "alice", "correct horse")
        .await
        .unwrap_err(); // port 9: nothing listens
    e.tokens
        .set(
            &p.id,
            &poe2_launcher_lib::keystore::StoredSession {
                token: LAUNCHER_TOKEN.into(),
                account: "Alice".into(),
                expires_at: i64::MAX,
                role: None,
            },
        )
        .unwrap();
    let out = e.core.logout(&p.id).await.unwrap();
    assert!(
        !out.revoked && out.message.contains("could not revoke"),
        "{}",
        out.message
    );
    assert!(
        e.tokens.is_empty(),
        "the local token goes even when the server is down"
    );
    e.core.remove_profile(&p.id).await.unwrap();
    assert!(e.core.profiles().unwrap().is_empty());
}

#[tokio::test]
async fn expired_tokens_do_not_count_as_logged_in() {
    let e = env();
    let p = e
        .core
        .add_profile("Mock", "http://127.0.0.1:9", false)
        .unwrap();
    e.tokens
        .set(
            &p.id,
            &poe2_launcher_lib::keystore::StoredSession {
                token: LAUNCHER_TOKEN.into(),
                account: "Alice".into(),
                expires_at: 1,
                role: None,
            },
        )
        .unwrap();
    let v = &e.core.profiles().unwrap()[0];
    assert!(!v.logged_in);
    assert_eq!(v.account.as_deref(), Some("Alice"));
}

#[tokio::test]
async fn settings_refuse_steam_directories() {
    let e = env();
    let s = Settings {
        install_dir: Some("/home/u/.local/share/Steam/steamapps/common/Path of Exile 2".into()),
        ..Settings::default()
    };
    assert!(e.core.set_settings(s).is_err());
    // The defaults are below the launcher's own data directory.
    let st = e.core.install_status().unwrap();
    assert!(st.install_dir.starts_with(&e.core.dirs.data));
    assert!(!st.installed && !st.running);
    assert_eq!(st.depots.len(), 2);
}

/// Play against the mock server with a fake install, fake DLLs and a fake Proton (a shell
/// script): the preparation writes the game token (not the launcher token) for the client,
/// deploys the hook and starts the client the way ProtonLaunch does.
#[cfg(unix)]
#[tokio::test]
async fn play_prepares_everything_and_starts_proton() {
    use std::os::unix::fs::PermissionsExt;
    let (url, mock) = spawn_mock().await;
    let e = env();
    let root = e._tmp.path();
    let install = root.join("game");
    let dlls = root.join("dlls");
    let steam = root.join("steam");
    std::fs::create_dir_all(&install).unwrap();
    std::fs::create_dir_all(&dlls).unwrap();
    std::fs::write(install.join("PathOfExileSteam.exe"), b"MZ").unwrap();
    std::fs::write(dlls.join("poe_hook.dll"), b"loader").unwrap();
    std::fs::write(dlls.join("poe_hook_core.dll"), b"core").unwrap();
    let proton_dir = steam.join("steamapps/common/Fake Proton");
    std::fs::create_dir_all(&proton_dir).unwrap();
    let record = root.join("proton-calls.txt");
    let script = format!(
        "#!/bin/sh\necho \"$@\" >> '{rec}'\necho \"PREFIX=$STEAM_COMPAT_DATA_PATH\" >> '{rec}'\n\
         if [ \"$2\" = wineboot ]; then mkdir -p \"$STEAM_COMPAT_DATA_PATH/pfx/drive_c/users/steamuser\"; fi\n",
        rec = record.display()
    );
    std::fs::write(proton_dir.join("proton"), script).unwrap();
    std::fs::set_permissions(
        proton_dir.join("proton"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();

    e.core
        .set_settings(Settings {
            install_dir: Some(install.clone()),
            dll_dir: Some(dlls),
            steam_root: Some(steam),
            proton: "Fake Proton".into(),
            ..Settings::default()
        })
        .unwrap();
    let p = e.core.add_profile("Mock", &url, false).unwrap();

    // Not logged in: refused with a pointer to the Account page.
    let err = e.core.play(&p.id).await.unwrap_err().to_string();
    assert!(err.contains("log in"), "{err}");
    assert_eq!(e.core.play_state(), PlayState::Idle);

    e.core.login(&p.id, "alice", "correct horse").await.unwrap();
    e.core.play(&p.id).await.unwrap();

    // The client got the GAME token; the launcher still holds its own, still valid.
    let prefix = e.core.dirs.prefix_dir();
    let docs = prefix.join("pfx/drive_c/users/steamuser/Documents/My Games/Path of Exile 2");
    let cred = std::fs::read(docs.join("poe2_production_login.credentials")).unwrap();
    assert_eq!(cred, [0xbb; 32]);
    let ini = std::fs::read_to_string(docs.join("poe2_production_Config.ini")).unwrap();
    assert!(
        ini.contains("username=Alice") && ini.contains("save_login=true"),
        "{ini}"
    );
    assert_eq!(e.tokens.get(&p.id).unwrap().unwrap().token, LAUNCHER_TOKEN);
    assert!(mock.live.lock().unwrap().contains(LAUNCHER_TOKEN));
    assert_eq!(*mock.game_token_calls.lock().unwrap(), 1);

    // Deployed into the (temporary) install: gateway, hook config, DLLs, appid.
    let gateway = std::fs::read(install.join("gateway_list.txt")).unwrap();
    let units: Vec<u16> = gateway[2..]
        .chunks(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    assert_eq!(
        String::from_utf16(&units).unwrap(),
        "Local;Mock Realm;127.0.0.1:29481;127.0.0.1;false"
    );
    let cfg = std::fs::read_to_string(install.join("poe-hook.toml")).unwrap();
    assert!(cfg.contains("login_mode = \"standalone\""), "{cfg}");
    assert!(
        cfg.contains("127.0.0.1:29481") && cfg.contains("127.0.0.1:29112"),
        "{cfg}"
    );
    assert!(install.join("version.dll").is_file() && install.join("poe_hook_core.dll").is_file());
    assert_eq!(
        std::fs::read_to_string(install.join("steam_appid.txt")).unwrap(),
        "2694490"
    );

    // Proton ran twice: prefix creation, then the client with the standalone option.
    let mut waited = 0;
    let calls = loop {
        let t = std::fs::read_to_string(&record).unwrap_or_default();
        if t.matches("PREFIX=").count() >= 2 || waited > 50 {
            break t;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        waited += 1;
    };
    assert!(calls.contains("run wineboot -u"), "{calls}");
    assert!(
        calls.contains("PathOfExileSteam.exe --success-event 0"),
        "{calls}"
    );
    assert!(
        calls.contains(&format!("PREFIX={}", prefix.display())),
        "{calls}"
    );
    assert!(!calls.to_lowercase().contains("compatdata/2694490"));

    // The child exits at once; the state follows.
    let mut st = e.core.play_state();
    for _ in 0..50 {
        if matches!(st, PlayState::Exited { .. }) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        st = e.core.play_state();
    }
    assert!(matches!(st, PlayState::Exited { code: Some(0) }), "{st:?}");
    assert!(
        e.events
            .0
            .lock()
            .unwrap()
            .iter()
            .any(|(n, _)| n == "play:status")
    );
}

/// Against a real server started by the caller (see docs/guides/launcher.md): login, token
/// storage, the game-token exchange and logout through the same code the UI uses.
#[tokio::test]
async fn real_server_roundtrip_when_configured() {
    let (Ok(url), Ok(user), Ok(pass)) = (
        std::env::var("POE_LAUNCHER_TEST_URL"),
        std::env::var("POE_LAUNCHER_TEST_USER"),
        std::env::var("POE_LAUNCHER_TEST_PASS"),
    ) else {
        eprintln!("POE_LAUNCHER_TEST_URL/USER/PASS not set: skipping the real-server round trip");
        return;
    };
    let e = env();
    let p = e.core.add_profile("Real", &url, false).unwrap();
    let info = e.core.server_info(&p.id).await.unwrap();
    println!("server: {info:?}");
    assert!(
        info.instance_port.is_some(),
        "the server must report instance_port"
    );
    let view = e.core.login(&p.id, &user, &pass).await.unwrap();
    assert!(view.logged_in);
    let stored = e.tokens.get(&p.id).unwrap().unwrap();
    assert_eq!(stored.token.len(), 64);
    let client = poe2_launcher_lib::api::ApiClient::new(&url);
    let game = client.game_token(&stored.token).await.unwrap();
    assert_eq!(game.kind, "game_remember");
    assert_ne!(game.token, stored.token);
    assert!(
        client
            .game_token(&game.token)
            .await
            .unwrap_err()
            .is_unauthorized(),
        "a game token is not a launcher token"
    );
    let out = e.core.logout(&p.id).await.unwrap();
    assert!(out.revoked, "{}", out.message);
    assert!(
        client
            .game_token(&stored.token)
            .await
            .unwrap_err()
            .is_unauthorized(),
        "revoked"
    );
}

/// The OS keyring itself (Secret Service on Linux). Needs an unlocked keyring service, so it
/// only runs on request: `cargo test --test core_flow real_keyring -- --ignored`.
#[test]
#[ignore = "needs an unlocked system keyring"]
fn real_keyring_roundtrip() {
    use poe2_launcher_lib::keystore::{KeyringStore, StoredSession};
    let store = KeyringStore::with_service("poe2-launcher-test");
    let id = format!("test-{}", std::process::id());
    let s = StoredSession {
        token: "cd".repeat(32),
        account: "alice".into(),
        expires_at: 4_102_444_800,
        role: None,
    };
    assert!(store.get(&id).unwrap().is_none());
    store.set(&id, &s).unwrap();
    assert_eq!(store.get(&id).unwrap().unwrap(), s);
    store.delete(&id).unwrap();
    store.delete(&id).unwrap();
    assert!(store.get(&id).unwrap().is_none());
}
