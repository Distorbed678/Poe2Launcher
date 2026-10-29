//! The game version switch ("Official (read-only)" plus the supported private versions), the
//! official Play (read-only deploy, no login, no token) and the Steam log-out, against a
//! temporary launcher home.

use std::sync::Arc;

use poe2_launcher_lib::{
    core::{Collector, Core, PlayState},
    keystore::MemoryStore,
    paths::Dirs,
    settings::Settings,
    version::VersionFile,
};

struct Env {
    tmp: tempfile::TempDir,
    core: Arc<Core>,
    #[cfg_attr(not(unix), allow(dead_code))]
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
    Env { tmp, core, events }
}

#[test]
fn the_list_has_official_first_then_the_private_versions_newest_first() {
    let e = env();
    let list = e.core.game_versions().unwrap();
    assert!(list[0].official && list[0].version == "official");
    let known = VersionFile::known_versions();
    let private: Vec<&str> = list[1..].iter().map(|v| v.version.as_str()).collect();
    let mut newest_first = known.clone();
    newest_first.reverse();
    assert_eq!(private, newest_first);
    assert_eq!(list.iter().filter(|v| v.selected).count(), 1);
    // Nothing picked yet: the newest private version.
    let sel = list.iter().find(|v| v.selected).unwrap();
    assert_eq!(sel.version, *known.last().unwrap());
    assert!(sel.newest && !sel.official && !sel.installed);
    assert_eq!(list.iter().filter(|v| v.newest).count(), 1);
}

#[test]
fn the_choice_is_saved_and_drives_the_install() {
    let e = env();
    let private = VersionFile::known_versions()[0];
    let st = e.core.install_status().unwrap();
    assert_eq!(st.version, *VersionFile::known_versions().last().unwrap());
    assert!(st.depots.iter().all(|d| !d.manifest.is_empty()));
    assert!(st.install_dir.ends_with(format!("game/{}", st.version)));

    // Official: the current build, no pinned manifests, an install folder of its own.
    let list = e.core.set_game_version("official").unwrap();
    assert!(list[0].selected);
    assert_eq!(
        e.core.settings().unwrap().game_version.as_deref(),
        Some("official")
    );
    let st = e.core.install_status().unwrap();
    assert_eq!(st.version, "official");
    assert!(st.depots.len() == 2 && st.depots.iter().all(|d| d.manifest.is_empty()));
    assert!(st.install_dir.ends_with("game/official"));
    // It survives a restart (a new Core over the same folder).
    let again = Core::new(
        Dirs::under(e.tmp.path()),
        Arc::new(MemoryStore::new()),
        Arc::new(Collector::default()),
    )
    .unwrap();
    assert_eq!(again.install_status().unwrap().version, "official");

    e.core.set_game_version(private).unwrap();
    assert_eq!(e.core.install_status().unwrap().version, private);

    // Unknown targets are refused and change nothing.
    assert!(e.core.set_game_version("9.9.9").is_err());
    assert_eq!(e.core.install_status().unwrap().version, private);
    // Through the settings too.
    let mut s = e.core.settings().unwrap();
    s.game_version = Some("9.9.9".into());
    assert!(e.core.set_settings(s).is_err());
}

#[test]
fn each_version_has_its_own_install_directory() {
    let e = env();
    let private = VersionFile::known_versions()[0];
    let a = e.tmp.path().join("a");
    let b = e.tmp.path().join("b");
    let mut s = Settings::default();
    s.install_dirs.insert(private.into(), a.clone());
    s.install_dirs.insert("official".into(), b.clone());
    e.core.set_settings(s).unwrap();
    e.core.set_game_version(private).unwrap();
    assert_eq!(e.core.install_dir().unwrap(), a);
    e.core.set_game_version("official").unwrap();
    assert_eq!(e.core.install_dir().unwrap(), b);
    // Installed flags follow each directory.
    std::fs::create_dir_all(&b).unwrap();
    std::fs::write(b.join("PathOfExileSteam.exe"), b"MZ").unwrap();
    let list = e.core.game_versions().unwrap();
    assert!(list[0].installed);
    assert!(list[1..].iter().all(|v| !v.installed));
}

#[tokio::test]
async fn official_has_no_mods_and_no_hosting() {
    let e = env();
    e.core.set_game_version("official").unwrap();
    assert!(
        e.core
            .mods_list("all")
            .unwrap_err()
            .to_string()
            .contains("official")
    );
    assert!(
        e.core
            .host_find_release()
            .await
            .unwrap_err()
            .to_string()
            .contains("private")
    );
    assert!(e.core.host_start("s1").await.is_err());
    assert!(e.core.host_create("x").is_err(), "no hosting for the official version");
    e.core
        .set_game_version(VersionFile::known_versions()[0])
        .unwrap();
    assert!(e.core.mods_list("all").is_ok());
}

#[tokio::test]
async fn private_play_is_refused_in_official_mode_and_official_play_needs_the_warning_accepted() {
    let e = env();
    e.core.set_game_version("official").unwrap();
    let err = e.core.play("anything").await.unwrap_err().to_string();
    assert!(err.contains("official"), "{err}");
    let err = e.core.play_official().await.unwrap_err().to_string();
    assert!(err.contains("warning"), "{err}");
    assert_eq!(e.core.play_state(), PlayState::Idle);
    // The warning is accepted, but the game is not there: explained, nothing written.
    let mut s = e.core.settings().unwrap();
    s.official_ack = true;
    e.core.set_settings(s).unwrap();
    let err = e.core.play_official().await.unwrap_err().to_string();
    assert!(err.contains("not installed"), "{err}");
    assert_eq!(e.core.play_state(), PlayState::Idle);
    // And a private version never starts as official.
    e.core
        .set_game_version(VersionFile::known_versions()[0])
        .unwrap();
    assert!(e.core.play_official().await.is_err());
}

/// Official Play with a fake install, fake DLLs and a fake Proton: the read-only config is
/// deployed, the original gateway list stays, no token or private gateway is written, the
/// official Proton prefix is used, and the game version cannot be switched while it runs.
#[cfg(unix)]
#[tokio::test]
async fn official_play_deploys_the_read_only_config_and_starts_proton() {
    use std::os::unix::fs::PermissionsExt;
    let e = env();
    let root = e.tmp.path();
    let install = root.join("official-game");
    let (dlls, steam) = (root.join("dlls"), root.join("steam"));
    std::fs::create_dir_all(&install).unwrap();
    std::fs::create_dir_all(&dlls).unwrap();
    std::fs::write(install.join("PathOfExileSteam.exe"), b"MZ").unwrap();
    std::fs::write(install.join("gateway_list.txt"), b"OFFICIAL-LIST").unwrap();
    std::fs::write(dlls.join("poe_hook.dll"), b"loader").unwrap();
    std::fs::write(dlls.join("poe_hook_core.dll"), b"core").unwrap();
    let proton_dir = steam.join("steamapps/common/Fake Proton");
    std::fs::create_dir_all(&proton_dir).unwrap();
    let record = root.join("proton-calls.txt");
    let script = format!(
        "#!/bin/sh\necho \"$@\" >> '{rec}'\necho \"PREFIX=$STEAM_COMPAT_DATA_PATH\" >> '{rec}'\n\
         if [ \"$2\" = wineboot ]; then mkdir -p \"$STEAM_COMPAT_DATA_PATH/pfx/drive_c/users/steamuser\"; \
         else sleep 2; fi\n",
        rec = record.display()
    );
    std::fs::write(proton_dir.join("proton"), script).unwrap();
    std::fs::set_permissions(
        proton_dir.join("proton"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();

    let mut s = Settings {
        dll_dir: Some(dlls),
        steam_root: Some(steam),
        proton: "Fake Proton".into(),
        official_ack: true,
        game_version: Some("official".into()),
        ..Settings::default()
    };
    s.install_dirs.insert("official".into(), install.clone());
    e.core.set_settings(s).unwrap();

    e.core.play_official().await.unwrap();
    let cfg = std::fs::read_to_string(install.join("poe-hook.toml")).unwrap();
    assert!(
        cfg.contains("mode = \"official\"") && cfg.contains("login_mode = \"steam\""),
        "{cfg}"
    );
    assert!(cfg.contains("trusted_servers = []"), "{cfg}");
    assert_eq!(
        std::fs::read(install.join("gateway_list.txt")).unwrap(),
        b"OFFICIAL-LIST"
    );
    assert!(install.join("version.dll").is_file());

    // The official prefix, never the private one, and no login token anywhere.
    let official_prefix = e.core.dirs.prefix_dir_official();
    assert!(official_prefix.join("pfx/drive_c/users/steamuser").is_dir());
    assert!(!e.core.dirs.prefix_dir().exists());
    let mut tokens = std::fs::read_dir(&official_prefix).map(|_| ()).is_ok();
    tokens &= !official_prefix
        .join("pfx/drive_c/users/steamuser/Documents/My Games")
        .exists();
    assert!(
        tokens,
        "no token or config was written for the game's login"
    );

    // Running: the version cannot change under it, a second start is refused.
    for _ in 0..50 {
        if matches!(e.core.play_state(), PlayState::Running { .. }) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(matches!(e.core.play_state(), PlayState::Running { .. }));
    let err = e
        .core
        .set_game_version(VersionFile::known_versions()[0])
        .unwrap_err()
        .to_string();
    assert!(err.contains("running"), "{err}");
    assert!(e.core.play_official().await.is_err());
    let calls = std::fs::read_to_string(&record).unwrap();
    // Started as shipped: no standalone-login argument.
    assert!(calls.contains("PathOfExileSteam.exe"), "{calls}");
    assert!(!calls.contains("--success-event"), "{calls}");
    assert!(
        calls.contains(&format!("PREFIX={}", official_prefix.display())),
        "{calls}"
    );

    for _ in 0..80 {
        if matches!(e.core.play_state(), PlayState::Exited { .. }) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(matches!(e.core.play_state(), PlayState::Exited { .. }));
    assert!(
        e.events
            .0
            .lock()
            .unwrap()
            .iter()
            .any(|(n, _)| n == "play:status")
    );
    // Allowed again once the game has ended.
    e.core
        .set_game_version(VersionFile::known_versions()[0])
        .unwrap();
}

#[test]
fn steam_logout_forgets_the_login_and_the_remembered_name() {
    let e = env();
    let store = e
        .core
        .dirs
        .steam_login_home()
        .join("IsolatedStorage/aaaa.bbb/cccc.ddd/Url.xyz/AssemFiles");
    std::fs::create_dir_all(&store).unwrap();
    std::fs::write(store.join("account.config"), b"token").unwrap();
    e.core
        .set_settings(Settings {
            steam_username: "alice".into(),
            remember_steam_login: true,
            ..Settings::default()
        })
        .unwrap();

    let st = e.core.steam_login().unwrap();
    assert!(st.remembered && st.account.as_deref() == Some("alice"));

    let st = e.core.steam_logout().unwrap();
    assert!(!st.remembered && st.account.is_none());
    assert!(!store.join("account.config").exists());
    let s = e.core.settings().unwrap();
    assert!(s.steam_username.is_empty() && !s.remember_steam_login);
    // A second log-out is harmless.
    assert!(!e.core.steam_logout().unwrap().remembered);
}

/// With two or more version files embedded (add `versions/<v>.toml`, see docs/guides/launcher.md)
/// every version is listed, selectable and has its own install folder and depots. Passes
/// trivially while only one private version exists.
#[test]
fn every_embedded_version_is_selectable() {
    let e = env();
    let known = VersionFile::known_versions();
    let mut dirs = std::collections::HashSet::new();
    for v in &known {
        e.core.set_game_version(v).unwrap();
        let st = e.core.install_status().unwrap();
        assert_eq!(st.version, *v);
        assert_eq!(
            st.depots.len(),
            VersionFile::embedded(v).unwrap().depots.len()
        );
        assert!(
            dirs.insert(st.install_dir.clone()),
            "{v} shares an install folder"
        );
        assert_eq!(
            e.core
                .game_versions()
                .unwrap()
                .iter()
                .filter(|g| g.selected)
                .count(),
            1
        );
    }
    assert_eq!(
        e.core.game_versions().unwrap().len(),
        known.len() + 1,
        "plus Official"
    );
}
