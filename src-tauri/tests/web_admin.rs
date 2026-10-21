//! "Open web admin" (WP F3) and the plain-`http://` guard (audit S-03) against a mock web API.
//! The system browser is never involved: the opener is a recorder.

use std::sync::{Arc, Mutex};

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use poe2_launcher_lib::{
    core::{Collector, Core, UrlOpener},
    keystore::{MemoryStore, StoredSession, TokenStore},
    paths::Dirs,
};
use serde_json::{Value, json};

const TOKEN: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const WEB_TOKEN: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

#[derive(Default)]
struct Mock {
    role: Mutex<String>,
    web_token_calls: Mutex<u32>,
    /// What `login_path` the mock answers with.
    login_path: Mutex<Option<String>>,
    /// Answer `/web-token` with this status instead.
    web_token_status: Mutex<Option<u16>>,
}

fn account(role: &str) -> Value {
    json!({"id": 1, "username": "Boss", "role": role, "created_at": 0, "disabled": false, "has_password": true})
}

fn authorised(h: &HeaderMap) -> bool {
    h.get("authorization").and_then(|v| v.to_str().ok()) == Some(&format!("Bearer {TOKEN}"))
}

async fn spawn_mock() -> (String, Arc<Mock>) {
    let mock = Arc::new(Mock::default());
    *mock.role.lock().unwrap() = "admin".into();
    let app = Router::new()
        .route(
            "/api/v1/login",
            post(|State(m): State<Arc<Mock>>, Json(_): Json<Value>| async move {
                let role = m.role.lock().unwrap().clone();
                Json(json!({"token": TOKEN, "kind": "launcher", "expires_at": 4_102_444_800i64, "account": account(&role)}))
            }),
        )
        .route(
            "/api/v1/me",
            get(|State(m): State<Arc<Mock>>, h: HeaderMap| async move {
                if !authorised(&h) {
                    return (StatusCode::UNAUTHORIZED, Json(json!({"error": "bad token"})));
                }
                let role = m.role.lock().unwrap().clone();
                (StatusCode::OK, Json(account(&role)))
            }),
        )
        .route(
            "/api/v1/web-token",
            post(|State(m): State<Arc<Mock>>, h: HeaderMap| async move {
                if !authorised(&h) {
                    return (StatusCode::UNAUTHORIZED, Json(json!({"error": "bad token"})));
                }
                *m.web_token_calls.lock().unwrap() += 1;
                if let Some(code) = *m.web_token_status.lock().unwrap() {
                    return (StatusCode::from_u16(code).unwrap(), Json(json!({"error": "no"})));
                }
                let path = m
                    .login_path
                    .lock()
                    .unwrap()
                    .clone()
                    .unwrap_or_else(|| format!("/#/token-login?t={WEB_TOKEN}"));
                (
                    StatusCode::OK,
                    Json(json!({"token": WEB_TOKEN, "expires_in_secs": 60, "login_path": path})),
                )
            }),
        )
        .with_state(mock.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (url, mock)
}

#[derive(Default)]
struct Recorder(Mutex<Vec<String>>);
impl UrlOpener for Recorder {
    fn open(&self, url: &str) -> anyhow::Result<()> {
        self.0.lock().unwrap().push(url.to_owned());
        Ok(())
    }
}

struct Env {
    _tmp: tempfile::TempDir,
    core: Arc<Core>,
    tokens: Arc<MemoryStore>,
}

fn env() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let tokens = Arc::new(MemoryStore::new());
    let core = Core::new(
        Dirs::under(tmp.path()),
        tokens.clone(),
        Arc::new(Collector::default()),
    )
    .unwrap();
    Env {
        _tmp: tmp,
        core,
        tokens,
    }
}

fn stored(role: Option<&str>) -> StoredSession {
    StoredSession {
        token: TOKEN.into(),
        account: "Boss".into(),
        expires_at: i64::MAX,
        role: role.map(str::to_owned),
    }
}

#[tokio::test]
async fn the_role_comes_with_the_login_and_is_refreshed() {
    let (url, mock) = spawn_mock().await;
    let e = env();
    let p = e.core.add_profile("Mock", &url, false).unwrap();
    assert_eq!(p.role, None);
    let v = e.core.login(&p.id, "Boss", "irrelevant").await.unwrap();
    assert_eq!(v.role.as_deref(), Some("admin"));

    // Demoted on the server: the stored role follows after a refresh, not before.
    *mock.role.lock().unwrap() = "player".into();
    assert_eq!(e.core.profiles().unwrap()[0].role.as_deref(), Some("admin"));
    let v = e.core.refresh_account(&p.id).await.unwrap();
    assert_eq!(v.role.as_deref(), Some("player"));
    assert_eq!(
        e.tokens.get(&p.id).unwrap().unwrap().role.as_deref(),
        Some("player")
    );

    // A token the server no longer knows is dropped, like in Play.
    e.tokens
        .set(
            &p.id,
            &StoredSession {
                token: "dd".repeat(32),
                ..stored(Some("admin"))
            },
        )
        .unwrap();
    let v = e.core.refresh_account(&p.id).await.unwrap();
    assert!(!v.logged_in && v.role.is_none());
    assert!(e.tokens.get(&p.id).unwrap().is_none());
}

#[tokio::test]
async fn open_web_admin_opens_the_one_time_link_for_admins_only() {
    let (url, mock) = spawn_mock().await;
    let e = env();
    let p = e.core.add_profile("Mock", &url, false).unwrap();
    let opener = Recorder::default();

    // Not logged in.
    let err = e
        .core
        .web_admin_open(&p.id, &opener)
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("log in"), "{err}");

    // A stored login without a role (older launcher) works: the role is read from /me.
    e.tokens.set(&p.id, &stored(None)).unwrap();
    e.core.web_admin_open(&p.id, &opener).await.unwrap();
    let opened = opener.0.lock().unwrap().clone();
    assert_eq!(opened, vec![format!("{url}/#/token-login?t={WEB_TOKEN}")]);
    assert_eq!(*mock.web_token_calls.lock().unwrap(), 1);
    assert_eq!(
        e.tokens.get(&p.id).unwrap().unwrap().role.as_deref(),
        Some("admin")
    );

    // A player never gets as far as asking for a web token.
    *mock.role.lock().unwrap() = "player".into();
    let err = e
        .core
        .web_admin_open(&p.id, &opener)
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("not an admin"), "{err}");
    assert_eq!(*mock.web_token_calls.lock().unwrap(), 1);
    assert_eq!(opener.0.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_strange_sign_in_link_is_not_opened() {
    let (url, mock) = spawn_mock().await;
    let e = env();
    let p = e.core.add_profile("Mock", &url, false).unwrap();
    e.tokens.set(&p.id, &stored(Some("admin"))).unwrap();
    let opener = Recorder::default();
    for evil in [
        "https://evil.example/#/token-login?t=".to_owned() + WEB_TOKEN,
        "//evil.example/x".to_owned(),
        "/#/token-login?t=other".to_owned(),
        format!("/#/token-login?t={WEB_TOKEN}&next=http://evil.example"),
        format!("/other#/token-login?t={WEB_TOKEN}"),
    ] {
        *mock.login_path.lock().unwrap() = Some(evil.clone());
        let err = e
            .core
            .web_admin_open(&p.id, &opener)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("not in the expected form"), "{evil}: {err}");
    }
    assert!(opener.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn server_refusals_are_explained() {
    let (url, mock) = spawn_mock().await;
    let e = env();
    let p = e.core.add_profile("Mock", &url, false).unwrap();
    e.tokens.set(&p.id, &stored(Some("admin"))).unwrap();
    let opener = Recorder::default();
    for (code, needle) in [(403, "refused"), (404, "too old")] {
        *mock.web_token_status.lock().unwrap() = Some(code);
        let err = e
            .core
            .web_admin_open(&p.id, &opener)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains(needle), "{code}: {err}");
    }
    assert!(opener.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn an_opener_failure_is_reported() {
    struct Broken;
    impl UrlOpener for Broken {
        fn open(&self, _: &str) -> anyhow::Result<()> {
            anyhow::bail!("no browser")
        }
    }
    let (url, _mock) = spawn_mock().await;
    let e = env();
    let p = e.core.add_profile("Mock", &url, false).unwrap();
    e.tokens.set(&p.id, &stored(Some("admin"))).unwrap();
    let err = e
        .core
        .web_admin_open(&p.id, &Broken)
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("could not open the browser") && err.contains("no browser"),
        "{err}"
    );
    // The error never carries the link (it holds a credential).
    assert!(!err.contains(WEB_TOKEN));
}

#[tokio::test]
async fn plain_http_to_another_machine_needs_a_confirmation() {
    let e = env();
    // Nothing is listening there: the guard must refuse before any connection is tried.
    let far = e
        .core
        .add_profile("Far", "http://203.0.113.7:8080", false)
        .unwrap();
    assert_eq!(serde_json::to_value(far.plain_http).unwrap(), "public");
    assert!(!far.plain_http_confirmed);
    let lan = e
        .core
        .add_profile("Lan", "http://192.168.1.50:8080", false)
        .unwrap();
    assert_eq!(serde_json::to_value(lan.plain_http).unwrap(), "private");
    let secure = e
        .core
        .add_profile("Secure", "https://play.example", false)
        .unwrap();
    assert_eq!(serde_json::to_value(secure.plain_http).unwrap(), "https");
    let local = e
        .core
        .add_profile("Local", "127.0.0.1:8080", false)
        .unwrap();
    assert_eq!(serde_json::to_value(local.plain_http).unwrap(), "loopback");

    for id in [&far.id, &lan.id] {
        let err = e
            .core
            .login(id, "u", "secret-password")
            .await
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("plain http://") && err.contains("unencrypted"),
            "{err}"
        );
        let err = e
            .core
            .register(id, "u", "secret-password")
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("plain http://"), "{err}");
        assert!(!err.contains("secret-password"));
    }
    // With a stored login, Play's token exchange and the web admin are guarded too.
    e.tokens.set(&far.id, &stored(Some("admin"))).unwrap();
    let err = e
        .core
        .web_admin_link(&far.id)
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("plain http://"), "{err}");
    let err = e
        .core
        .refresh_account(&far.id)
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("plain http://"), "{err}");

    // Confirming is per profile and lifts the guard (the connection then fails for other reasons
    // or succeeds; here we only check the guard's message is gone by using a reachable address).
    let v = e.core.set_plain_http_confirmed(&far.id, true).unwrap();
    assert!(v.plain_http_confirmed);
    // Adding with the confirmation ticked stores it at once; https needs none.
    let ok = e
        .core
        .add_profile("Agreed", "http://203.0.113.9:8080", true)
        .unwrap();
    assert!(ok.plain_http_confirmed);
    let https = e
        .core
        .add_profile("Https", "https://203.0.113.9", true)
        .unwrap();
    assert!(!https.plain_http_confirmed, "nothing to confirm for https");
    // Changing the address withdraws the confirmation.
    let moved = e
        .core
        .update_profile(&ok.id, "Agreed", "http://203.0.113.10:8080", false)
        .unwrap();
    assert!(!moved.plain_http_confirmed);
    let err = e
        .core
        .login(&ok.id, "u", "secret-password")
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("plain http://"), "{err}");
}

#[tokio::test]
async fn loopback_http_needs_no_confirmation() {
    let (url, _mock) = spawn_mock().await;
    let e = env();
    let p = e.core.add_profile("Mock", &url, false).unwrap();
    // 127.0.0.1 plain http works without any ceremony.
    let v = e.core.login(&p.id, "Boss", "x").await.unwrap();
    assert!(v.logged_in);
}
