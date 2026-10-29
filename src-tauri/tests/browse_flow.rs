//! Home news, the public server list, live status and ping, logs and server backups, against
//! mock GitHub / list / web API servers and temp folders.

use std::{
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
    time::{Duration, Instant},
};

use axum::{
    Json, Router,
    extract::{Path as AxPath, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
};
use poe2_launcher_lib::{
    core::{Collector, Core},
    keystore::{MemoryStore, StoredSession, TokenStore},
    logs, news,
    news::NewsSources,
    paths::Dirs,
};
use serde_json::{Value, json};

struct Env {
    tmp: tempfile::TempDir,
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
    Env { tmp, core, tokens }
}

async fn serve(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    url
}

// --- news -------------------------------------------------------------------------------------

#[derive(Default)]
struct GhState {
    hits: AtomicU32,
    /// Repos that answer 429.
    limited: std::sync::Mutex<Vec<String>>,
}

fn release(tag: &str, date: &str, body: &str) -> Value {
    json!({"tag_name": tag, "name": format!("Release {tag}"), "body": body, "published_at": date,
        "html_url": format!("https://github.com/x/y/releases/tag/{tag}"), "draft": false, "prerelease": false})
}

async fn mock_news() -> (NewsSources, Arc<GhState>) {
    let st = Arc::new(GhState::default());
    let app = Router::new()
        .route(
            "/repos/{owner}/{repo}/releases",
            get(
                |State(s): State<Arc<GhState>>, AxPath((o, r)): AxPath<(String, String)>| async move {
                    s.hits.fetch_add(1, Ordering::SeqCst);
                    if s.limited.lock().unwrap().contains(&r) {
                        return (StatusCode::TOO_MANY_REQUESTS, "rate limited").into_response();
                    }
                    let list = match r.as_str() {
                        "Poe2Launcher" => vec![release("v0.2.0", "2026-10-20T10:00:00Z", "- new look"), release("v0.1.1", "2026-10-10T10:00:00Z", "")],
                        "Poe2Server" => vec![
                            release("server-v0.9.0", "2026-10-21T10:00:00Z", "movement"),
                            release("dat-schema-0.5.5e.hotfix1", "2026-10-21T12:00:00Z", "not news"),
                        ],
                        "Poe2ClientHook" => vec![release("hook-v0.2.0", "2026-10-19T10:00:00Z", "hook notes")],
                        _ => vec![],
                    };
                    assert_eq!(o, "Distorbed678");
                    Json(Value::Array(list)).into_response()
                },
            ),
        )
        .route(
            "/news.json",
            get(|| async {
                Json(json!({"version": 1, "items": [
                    {"id": "welcome", "date": "2026-10-22", "title": "Welcome", "body": "Hello **world**", "link": "https://example.org"},
                    {"id": "broken id", "date": "2026-10-22", "title": "Skipped"}
                ]}))
            }),
        )
        .with_state(st.clone());
    let base = serve(app).await;
    (
        NewsSources {
            api_base: base.clone(),
            news_url: format!("{base}/news.json"),
        },
        st,
    )
}

#[tokio::test]
async fn news_merges_releases_and_announcements_and_caches() {
    let e = env();
    let (src, st) = mock_news().await;
    let feed = e.core.news_get_from(&src, false).await;
    assert!(!feed.offline);
    let ids: Vec<_> = feed.items.iter().map(|i| i.id.as_str()).collect();
    // Newest first; the schema tag is not news; the broken announcement is skipped.
    assert_eq!(
        ids,
        [
            "welcome",
            "server-server-v0.9.0",
            "launcher-v0.2.0",
            "hook-hook-v0.2.0",
            "launcher-v0.1.1"
        ]
    );
    assert_eq!(feed.items[0].source, "project");
    assert_eq!(feed.items[1].version.as_deref(), Some("0.9.0"));
    assert_eq!(st.hits.load(Ordering::SeqCst), 3);

    // A fresh cache answers without asking again.
    let again = e.core.news_get_from(&src, false).await;
    assert_eq!(again.items.len(), 5);
    assert_eq!(st.hits.load(Ordering::SeqCst), 3);
    // A refresh right after a refresh is ignored (rate-limit friendly).
    let forced = e.core.news_get_from(&src, true).await;
    assert_eq!(forced.items.len(), 5);
    assert_eq!(st.hits.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn offline_and_rate_limited_news_fall_back_to_the_saved_copy() {
    let e = env();
    let (src, st) = mock_news().await;
    e.core.news_get_from(&src, false).await;
    let cache_file = e.tmp.path().join("data/cache/news.json");
    assert!(cache_file.is_file());

    // Age the cache so that the next call asks again.
    let age = |at: i64| {
        let mut c = news::load_cache(&cache_file);
        c.attempted_at = at;
        news::save_cache(&cache_file, &c);
    };
    age(1);
    st.limited.lock().unwrap().push("Poe2ClientHook".into());
    let feed = e.core.news_get_from(&src, false).await;
    assert!(!feed.offline, "most sources answered");
    assert!(
        feed.items.iter().any(|i| i.id == "hook-hook-v0.2.0"),
        "the rate-limited source keeps its saved item"
    );

    // Nothing reachable at all: the saved copy, flagged offline, no error.
    age(1);
    let dead = NewsSources {
        api_base: "http://127.0.0.1:9".into(),
        news_url: "http://127.0.0.1:9/news.json".into(),
    };
    let feed = e.core.news_get_from(&dead, false).await;
    assert!(feed.offline);
    assert_eq!(feed.items.len(), 5);

    // No cache and no network: empty, still no error.
    let fresh = env();
    let feed = fresh.core.news_get_from(&dead, false).await;
    assert!(feed.offline && feed.items.is_empty());
}

// --- server list and live status ------------------------------------------------------------

fn server_app(
    name: &'static str,
    online: u32,
    delay_ms: u64,
    motd: Option<&'static str>,
) -> Router {
    Router::new().route(
        "/api/v1/server",
        get(move || async move {
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            let mut v = json!({"name": name, "version": "0.5.5e.hotfix1", "cluster_id": "c", "online": online,
                "open_registration": true, "login_host": "play.example", "login_port": 20481,
                "instance_port": 6112, "gateway_region": "EU", "uptime_secs": 100});
            if let Some(m) = motd {
                v["motd"] = json!(m);
            }
            Json(v)
        }),
    )
}

fn entry(id: &str, addr: &str) -> Value {
    json!({"id": id, "name": id, "address": addr, "description": "d", "region": "EU",
        "game_version": "0.5.5e.hotfix1", "tags": ["pve"], "contact": "c"})
}

#[tokio::test]
async fn the_server_list_is_fetched_validated_and_probed_in_parallel() {
    let e = env();
    let mut addrs = Vec::new();
    for (i, online) in [3u32, 7, 11, 0].iter().enumerate() {
        addrs.push(
            serve(server_app(
                ["alpha", "beta", "gamma", "delta"][i],
                *online,
                500,
                None,
            ))
            .await,
        );
    }
    let dead = "http://127.0.0.1:9".to_owned();
    let mut servers: Vec<Value> = addrs
        .iter()
        .enumerate()
        .map(|(i, a)| entry(&format!("realm-{i}"), a))
        .collect();
    servers.push(entry("realm-dead", &dead));
    servers.push(json!({"id": "x", "name": "bad id"}));
    let list_url = serve(Router::new().route(
        "/servers.json",
        get({
            let body = json!({"schema": 1, "servers": servers});
            move || async move { Json(body.clone()) }
        }),
    ))
    .await;

    // The real rules refuse a list that names this computer.
    let strict = e
        .core
        .server_list_get_from(&format!("{list_url}/servers.json"), true, false)
        .await;
    assert!(strict.servers.is_empty(), "{:?}", strict.servers.len());
    assert!(strict.skipped.iter().any(|s| s.contains("local network")));
    // Let the "attempted" timestamp pass the force gap for the second fetch.
    let f = e.tmp.path().join("data/cache/serverlist.json");
    let mut c: Value = serde_json::from_str(&std::fs::read_to_string(&f).unwrap()).unwrap();
    c["attempted_at"] = json!(1);
    std::fs::write(&f, c.to_string()).unwrap();

    let list = e
        .core
        .server_list_get_from(&format!("{list_url}/servers.json"), false, true)
        .await;
    assert!(!list.offline, "{}", list.error);
    assert_eq!(list.servers.len(), 5);
    assert_eq!(list.skipped.len(), 1, "{:?}", list.skipped);

    let ids: Vec<String> = list.servers.iter().map(|s| s.id.clone()).collect();
    let t = Instant::now();
    let probes = e.core.server_list_probe(&ids).await.unwrap();
    let took = t.elapsed();
    // 4 servers x 500 ms in parallel; the dead one fails fast. Sequential would be 2 s+.
    assert!(took < Duration::from_millis(1700), "{took:?}");
    assert_eq!(probes.len(), 5);
    let by = |id: &str| probes.iter().find(|p| p.id == id).unwrap();
    assert!(by("realm-1").ok);
    assert_eq!(by("realm-1").info.as_ref().unwrap().online, 7);
    assert!(
        by("realm-1").ping_ms.unwrap() >= 450,
        "ping includes the server's time"
    );
    assert!(!by("realm-dead").ok);
    assert!(
        by("realm-dead")
            .error
            .as_ref()
            .unwrap()
            .contains("cannot reach")
    );

    // Unknown ids are an error, not an empty answer.
    assert!(e.core.server_list_probe(&["nope".into()]).await.is_err());

    // The list is cached: an unreachable list address gives the saved copy and a note.
    c = serde_json::from_str(&std::fs::read_to_string(&f).unwrap()).unwrap();
    c["attempted_at"] = json!(1);
    std::fs::write(&f, c.to_string()).unwrap();
    let off = e
        .core
        .server_list_get_from("http://127.0.0.1:9/servers.json", false, true)
        .await;
    assert!(off.offline && !off.error.is_empty());
    assert_eq!(off.servers.len(), 5);
}

#[tokio::test]
async fn a_my_server_status_carries_the_ping_and_the_optional_motd() {
    let e = env();
    let with = serve(server_app(
        "Motd Realm",
        4,
        120,
        Some("Welcome!\nEvent on Saturday."),
    ))
    .await;
    let without = serve(server_app("Plain Realm", 1, 0, None)).await;
    let a = e.core.add_profile("With", &with, false).unwrap();
    let b = e.core.add_profile("Without", &without, false).unwrap();
    let st = e.core.server_status(&a.id).await.unwrap();
    assert_eq!(
        st.info.motd.as_deref(),
        Some("Welcome!\nEvent on Saturday.")
    );
    assert!(st.ping_ms >= 110, "{}", st.ping_ms);
    assert_eq!(e.core.server_status(&b.id).await.unwrap().info.motd, None);
    let gone = e
        .core
        .add_profile("Gone", "http://127.0.0.1:9", false)
        .unwrap();
    assert!(e.core.server_status(&gone.id).await.is_err());
}

// --- logs ---------------------------------------------------------------------------------

#[tokio::test]
async fn logs_are_listed_read_by_id_and_reported_without_private_data() {
    let e = env();
    let p = e
        .core
        .add_profile("Realm", "http://127.0.0.1:1", false)
        .unwrap();
    e.tokens
        .set(
            &p.id,
            &StoredSession {
                token: "t".repeat(64),
                account: "Alice".into(),
                expires_at: 4_102_444_800,
                role: None,
            },
        )
        .unwrap();
    let data = e.tmp.path().join("data");
    std::fs::create_dir_all(data.join("logs")).unwrap();
    std::fs::create_dir_all(data.join("crash-dumps")).unwrap();
    let sid = e.core.host_create("Logs").unwrap();
    std::fs::create_dir_all(data.join("servers").join(&sid)).unwrap();
    let home = dirs::home_dir().unwrap().display().to_string();
    let secret = "ab12".repeat(16);
    std::fs::write(
        data.join("logs/game-1761000000.log"),
        format!(
            "start\nloading {home}/game as Alice\nlogin token={secret}\nAlice joined\nlast line\n"
        ),
    )
    .unwrap();
    std::fs::write(data.join("logs/notes.txt"), "not a game log").unwrap();
    std::fs::write(data.join("servers").join(&sid).join("server.log"), "server up\n").unwrap();
    std::fs::write(data.join("crash-dumps/crash1.dmp"), [b'M', b'D', 0, 0]).unwrap();

    let list = e.core.log_list();
    let ids: Vec<_> = list.files.iter().map(|f| f.id.as_str()).collect();
    assert!(ids.contains(&"game:game-1761000000.log"));
    assert!(ids.contains(&format!("server:{sid}/server.log").as_str()), "{ids:?}");
    assert!(ids.contains(&"crash:crash1.dmp"));
    assert!(
        !ids.iter().any(|i| i.contains("notes.txt")),
        "only game-*.log"
    );
    assert!(!list.files.iter().find(|f| f.kind == "crash").unwrap().text);

    // Reading is by id; a path never works.
    let c = e.core.log_read("game:game-1761000000.log", 2).unwrap();
    assert_eq!(c.lines, ["Alice joined", "last line"]);
    assert!(c.truncated);
    assert!(e.core.log_read("game:../../etc/passwd", 10).is_err());
    assert!(e.core.log_read("game:notes.txt", 10).is_err());
    assert!(e.core.log_read("crash:crash1.dmp", 10).unwrap().binary);

    // The bug report: system info, redacted text.
    let rep = e
        .core
        .log_report(Some("game:game-1761000000.log"), 50)
        .unwrap();
    assert!(rep.starts_with("Poe2 Launcher bug report"), "{rep}");
    assert!(
        rep.contains("launcher: 0.2.0") || rep.contains("launcher: "),
        "{rep}"
    );
    assert!(rep.contains("game version: "), "{rep}");
    assert!(!rep.contains(&home), "the home folder must be gone:\n{rep}");
    assert!(!rep.contains(&secret), "{rep}");
    assert!(!rep.contains("Alice"), "account names must be gone:\n{rep}");
    assert!(
        rep.contains("<account> joined") && rep.contains("last line"),
        "{rep}"
    );
    assert!(e.core.log_report(None, 10).unwrap().contains("os: "));
    assert!(e.core.log_folder("game").unwrap().ends_with("logs"));
    assert!(e.core.log_folder("elsewhere").is_err());
    let _ = logs::DEFAULT_LINES;
}

// --- backups ------------------------------------------------------------------------------

fn make_db(path: &std::path::Path, rows: &[&str]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let c = rusqlite::Connection::open(path).unwrap();
    c.pragma_update(None, "journal_mode", "WAL").unwrap();
    c.execute_batch("CREATE TABLE IF NOT EXISTS accounts (name TEXT)")
        .unwrap();
    for r in rows {
        c.execute("INSERT INTO accounts (name) VALUES (?1)", [r])
            .unwrap();
    }
}

fn names(path: &std::path::Path) -> Vec<String> {
    let c = rusqlite::Connection::open(path).unwrap();
    let mut s = c
        .prepare("SELECT name FROM accounts ORDER BY rowid")
        .unwrap();
    s.query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

#[test]
fn backups_through_the_core() {
    let e = env();
    let id = e.core.host_create("Backups").unwrap();
    let id = id.as_str();
    let db = e.tmp.path().join("data/servers/s1/data/server.sqlite");
    // No database yet: a clear message, an empty list.
    assert!(e.core.backup_status(id).unwrap().backups.is_empty());
    assert!(e.core.backup_status(id).unwrap().database_size.is_none());
    assert!(format!("{:#}", e.core.backup_create(id).unwrap_err()).contains("no database yet"));

    make_db(&db, &["admin", "alice"]);
    let st = e.core.backup_status(id).unwrap();
    assert!(st.database_size.is_some() && !st.server_running);
    assert!(st.dir.ends_with("servers/s1/backups"));
    let b = e.core.backup_create(id).unwrap();
    assert_eq!(b.kind, "manual");
    assert_eq!(names(&st.dir.join(&b.name)), ["admin", "alice"]);

    // The database changes; restore brings the old state back and keeps the new one.
    {
        let c = rusqlite::Connection::open(&db).unwrap();
        c.execute("INSERT INTO accounts (name) VALUES ('bob')", [])
            .unwrap();
        c.execute("DELETE FROM accounts WHERE name = 'alice'", [])
            .unwrap();
    }
    let pre = e.core.backup_restore(id, &b.name).unwrap().unwrap();
    assert_eq!(pre.kind, "pre-restore");
    assert_eq!(names(&db), ["admin", "alice"]);
    assert_eq!(names(&st.dir.join(&pre.name)), ["admin", "bob"]);
    assert_eq!(e.core.backup_status(id).unwrap().backups.len(), 2);

    // Deleting, and refusing names that are not backups.
    assert!(e.core.backup_delete(id, "../../server.sqlite").is_err());
    e.core.backup_delete(id, &pre.name).unwrap();
    assert_eq!(e.core.backup_status(id).unwrap().backups.len(), 1);

    // Automatic backups: off by default, on with a keep count, and only the autos are pruned.
    e.core.backup_auto(id);
    assert_eq!(
        e.core.backup_status(id).unwrap().backups.len(),
        1,
        "off by default"
    );
    assert!(e.core.backup_set_settings(id, true, 0).is_err());
    let s = e.core.backup_set_settings(id, true, 2).unwrap();
    assert!(s.auto && s.keep == 2);
    assert_eq!(e.core.backup_settings(id).unwrap(), s);
    for _ in 0..4 {
        e.core.backup_auto(id);
        std::thread::sleep(Duration::from_millis(1100)); // names carry the second
    }
    let l = e.core.backup_status(id).unwrap().backups;
    assert_eq!(l.iter().filter(|b| b.kind == "auto").count(), 2);
    assert_eq!(
        l.iter().filter(|b| b.kind == "manual").count(),
        1,
        "the manual one stays"
    );
}
