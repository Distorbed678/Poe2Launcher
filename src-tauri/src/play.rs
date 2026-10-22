//! Deploy and launch: turns a server profile and the settings into what `poe-hook-deploy`
//! needs, writes the auto-login token and starts the client.
//!
//! Linux starts the client through Proton with a prefix the launcher owns
//! (`<data>/proton-prefix`, never the Steam install's `compatdata/2694490`); Steam must be
//! running. Windows starts `PathOfExileSteam.exe` directly. Official servers are never
//! touched: the deploy is always `Mode::Private`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use poe_hook_deploy::{
    CLIENT_EXE, CORE_DLL, DeployOptions, GatewayEntry, LOADER_BUILD_NAME, LoginMode, Mode,
    launch::{ProtonLaunch, client_args},
};

use crate::{api::ServerInfo, profiles::url_host};

/// What the launcher needs from the settings to start the game.
#[derive(Debug, Clone)]
pub struct PlayConfig {
    pub install_dir: PathBuf,
    pub dll_dir: PathBuf,
    pub steam_root: PathBuf,
    pub proton: String,
    pub prefix_dir: PathBuf,
    pub crash_dump_dir: PathBuf,
    pub logs_dir: PathBuf,
}

fn is_unusable_host(host: &str) -> bool {
    matches!(
        host,
        "" | "localhost" | "127.0.0.1" | "::1" | "0.0.0.0" | "::"
    )
}

/// The host the game client should connect to. The server advertises `[network] public_host`;
/// when that is still the default loopback but the launcher reached the server at another
/// address, the address the launcher used is the one that works.
pub fn client_host(info: &ServerInfo, profile_url: &str) -> String {
    let advertised = info.login_host.trim();
    match url_host(profile_url) {
        Some(h) if is_unusable_host(advertised) && !is_unusable_host(&h) => h,
        _ if advertised.is_empty() => url_host(profile_url).unwrap_or_default(),
        _ => advertised.to_owned(),
    }
}

pub fn gateway_entry(info: &ServerInfo, host: &str) -> GatewayEntry {
    GatewayEntry {
        region: info
            .gateway_region
            .clone()
            .unwrap_or_else(|| "Local".into()),
        display_name: info.name.clone(),
        host: host.to_owned(),
        port: info.login_port,
    }
}

/// The deploy for a server: gateway entry, `trusted_servers` (its login and instance
/// addresses), standalone login, position reports to the instance port.
pub fn deploy_options(cfg: &PlayConfig, info: &ServerInfo, host: &str) -> Result<DeployOptions> {
    let instance_port = info.instance_port.context(
        "the server does not report its instance port (GET /server `instance_port`): \
         it is older than this launcher; update the server",
    )?;
    Ok(DeployOptions {
        game_dir: cfg.install_dir.clone(),
        dll_dir: cfg.dll_dir.clone(),
        mode: Mode::Private,
        gateway: Some(gateway_entry(info, host)),
        position_sync_port: Some(instance_port),
        dump_dir: None,
        login_mode: LoginMode::Standalone,
        trusted_servers: vec![
            format!("{host}:{}", info.login_port),
            format!("{host}:{instance_port}"),
        ],
    })
}

/// The deploy for the official servers, exactly what `cargo xtask deploy-client --official`
/// does: the original gateway list, `mode = "official"`, `login_mode = "steam"`, no trusted
/// server, no position reports, no output directory. The deploy forces the safety-relevant keys
/// again; nothing here may add a gateway, a trusted server or a login patch.
pub fn official_deploy_options(cfg: &PlayConfig) -> DeployOptions {
    DeployOptions {
        game_dir: cfg.install_dir.clone(),
        dll_dir: cfg.dll_dir.clone(),
        mode: Mode::Official,
        gateway: None,
        position_sync_port: None,
        dump_dir: None,
        login_mode: LoginMode::Steam,
        trusted_servers: Vec::new(),
    }
}

/// The hook DLLs must be where the settings say, before the deploy copies them.
pub fn check_dlls(dll_dir: &Path) -> Result<()> {
    let missing: Vec<&str> = [LOADER_BUILD_NAME, CORE_DLL]
        .into_iter()
        .filter(|n| !dll_dir.join(n).is_file())
        .collect();
    if !missing.is_empty() {
        bail!(
            "{} not found in {}. Build the hook (`cargo build --release --target \
             x86_64-pc-windows-msvc` in Poe2ClientHook) and set the DLL directory in Settings",
            missing.join(" and "),
            dll_dir.display()
        );
    }
    Ok(())
}

/// Where `write_login_token` puts the client's files: the Documents folder of the Proton
/// prefix on Linux, the user's Documents on Windows.
pub fn token_documents_dir(prefix_dir: &Path) -> Result<PathBuf> {
    if cfg!(windows) {
        dirs::document_dir().context("cannot find the Documents folder")
    } else {
        Ok(prefix_dir.join("pfx/drive_c/users/steamuser/Documents"))
    }
}

/// Whether Proton has already created the prefix.
pub fn prefix_ready(prefix_dir: &Path) -> bool {
    prefix_dir.join("pfx/drive_c/users/steamuser").is_dir()
}

fn proton_launch(cfg: &PlayConfig) -> ProtonLaunch {
    ProtonLaunch {
        steam_root: cfg.steam_root.clone(),
        proton: cfg.proton.clone(),
        prefix_dir: cfg.prefix_dir.clone(),
        game_dir: cfg.install_dir.clone(),
        crash_dump_dir: Some(cfg.crash_dump_dir.clone()),
    }
}

/// Creates the Proton prefix on first use (`proton run wineboot -u`), so the token file can be
/// written into its Documents folder before the game starts. Linux only.
pub async fn ensure_prefix(cfg: &PlayConfig) -> Result<()> {
    if cfg!(windows) || prefix_ready(&cfg.prefix_dir) {
        return Ok(());
    }
    std::fs::create_dir_all(&cfg.prefix_dir)?;
    let proton = proton_launch(cfg);
    let mut std_cmd = proton.command()?;
    // Same environment as a game launch, a harmless program instead of the client.
    let args: Vec<_> = vec!["run".into(), "wineboot".into(), "-u".into()];
    std_cmd = rebuild_with_args(std_cmd, &args);
    let mut cmd = tokio::process::Command::from(std_cmd);
    cmd.kill_on_drop(true);
    let status = tokio::time::timeout(std::time::Duration::from_secs(300), cmd.status())
        .await
        .context("creating the Proton prefix took longer than 5 minutes")?
        .context("starting Proton")?;
    if !status.success() || !prefix_ready(&cfg.prefix_dir) {
        bail!(
            "Proton could not create the prefix in {} ({status})",
            cfg.prefix_dir.display()
        );
    }
    Ok(())
}

/// Same program and environment as `cmd`, other arguments and the install dir as cwd.
fn rebuild_with_args(cmd: std::process::Command, args: &[String]) -> std::process::Command {
    let mut out = std::process::Command::new(cmd.get_program());
    out.args(args);
    for (k, v) in cmd.get_envs() {
        if let Some(v) = v {
            out.env(k, v);
        }
    }
    if let Some(d) = cmd.get_current_dir() {
        out.current_dir(d);
    }
    out
}

/// The command that starts the client (does not run it).
pub fn launch_command(cfg: &PlayConfig) -> Result<std::process::Command> {
    if cfg!(windows) {
        let exe = cfg.install_dir.join(CLIENT_EXE);
        if !exe.is_file() {
            bail!("{} not found", exe.display());
        }
        let mut cmd = std::process::Command::new(exe);
        cmd.args(client_args(&cfg.install_dir))
            .current_dir(&cfg.install_dir);
        Ok(cmd)
    } else {
        let proton = proton_launch(cfg);
        let cmd = proton.command()?;
        // Pending crash dumps would be uploaded by the client's helper: set them aside first.
        let _ =
            poe_hook_deploy::launch::quarantine_crash_dumps(&cfg.prefix_dir, &cfg.crash_dump_dir);
        Ok(cmd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(host: &str) -> ServerInfo {
        ServerInfo {
            name: "Test Realm".into(),
            version: "0.5.5e".into(),
            cluster_id: "c".into(),
            online: 0,
            open_registration: true,
            login_host: host.into(),
            login_port: 20481,
            instance_port: Some(6112),
            gateway_region: Some("Local".into()),
            uptime_secs: 1,
        }
    }

    fn cfg(root: &Path) -> PlayConfig {
        PlayConfig {
            install_dir: root.join("game"),
            dll_dir: root.join("dlls"),
            steam_root: root.join("steam"),
            proton: "Proton - Experimental".into(),
            prefix_dir: root.join("prefix"),
            crash_dump_dir: root.join("crash"),
            logs_dir: root.join("logs"),
        }
    }

    #[test]
    fn the_client_host_is_the_advertised_one_unless_it_is_loopback_and_we_came_from_elsewhere() {
        assert_eq!(
            client_host(&info("play.example"), "http://10.0.0.5:8080"),
            "play.example"
        );
        assert_eq!(
            client_host(&info("127.0.0.1"), "http://10.0.0.5:8080"),
            "10.0.0.5"
        );
        assert_eq!(
            client_host(&info("127.0.0.1"), "http://localhost:8080"),
            "127.0.0.1"
        );
        assert_eq!(
            client_host(&info("0.0.0.0"), "http://friends.example"),
            "friends.example"
        );
        assert_eq!(
            client_host(&info(""), "http://friends.example"),
            "friends.example"
        );
    }

    #[test]
    fn deploy_options_trust_the_login_and_instance_addresses() {
        let tmp = tempfile::tempdir().unwrap();
        let o = deploy_options(&cfg(tmp.path()), &info("play.example"), "play.example").unwrap();
        assert_eq!(o.mode, Mode::Private);
        assert_eq!(o.login_mode, LoginMode::Standalone);
        assert_eq!(
            o.trusted_servers,
            ["play.example:20481", "play.example:6112"]
        );
        assert_eq!(o.position_sync_port, Some(6112));
        let g = o.gateway.unwrap();
        assert_eq!(
            g.line(),
            "Local;Test Realm;play.example:20481;play.example;false"
        );
        let mut old = info("h");
        old.instance_port = None;
        assert!(
            deploy_options(&cfg(tmp.path()), &old, "h")
                .unwrap_err()
                .to_string()
                .contains("instance port")
        );
    }

    fn fake_game(c: &PlayConfig) {
        std::fs::create_dir_all(&c.install_dir).unwrap();
        std::fs::create_dir_all(&c.dll_dir).unwrap();
        std::fs::write(c.dll_dir.join("poe_hook.dll"), b"loader").unwrap();
        std::fs::write(c.dll_dir.join("poe_hook_core.dll"), b"core").unwrap();
        // The official gateway list as the download leaves it (any bytes; it is only copied).
        std::fs::write(c.install_dir.join("gateway_list.txt"), b"OFFICIAL-LIST").unwrap();
    }

    /// Official versus private against a temporary game dir: the official deploy keeps the
    /// original realm list, trusts nothing, does not patch the login, and even a private
    /// deploy made earlier in the same folder is turned back.
    #[test]
    fn the_official_deploy_is_read_only_and_undoes_a_private_one() {
        let tmp = tempfile::tempdir().unwrap();
        let c = cfg(tmp.path());
        fake_game(&c);
        let read = |name: &str| std::fs::read(c.install_dir.join(name)).unwrap();
        let toml = || String::from_utf8(read("poe-hook.toml")).unwrap();

        // Official on a fresh download.
        let o = official_deploy_options(&c);
        assert_eq!(
            (o.mode, o.login_mode, o.gateway.is_none()),
            (Mode::Official, LoginMode::Steam, true)
        );
        assert!(
            o.trusted_servers.is_empty() && o.position_sync_port.is_none() && o.dump_dir.is_none()
        );
        let report = poe_hook_deploy::deploy(&o).unwrap();
        assert_eq!(report.login_mode, Some(LoginMode::Steam));
        assert!(report.trusted_servers.is_empty() && report.gateway.is_none());
        let t = toml();
        assert!(t.contains("mode = \"official\""), "{t}");
        assert!(t.contains("login_mode = \"steam\""), "{t}");
        assert!(t.contains("trusted_servers = []"), "{t}");
        assert!(!t.contains("10.1.2.3"), "{t}");
        assert_eq!(read("gateway_list.txt"), b"OFFICIAL-LIST");
        assert_eq!(read("steam_appid.txt"), b"2694490");
        assert!(c.install_dir.join("version.dll").is_file());
        // No private token was written anywhere (official play never logs in).
        assert!(!c.prefix_dir.exists());

        // Private, then official again in the same folder.
        poe_hook_deploy::deploy(&deploy_options(&c, &info("10.1.2.3"), "10.1.2.3").unwrap())
            .unwrap();
        assert_ne!(read("gateway_list.txt"), b"OFFICIAL-LIST");
        let t = toml();
        assert!(
            t.contains("mode = \"private\"") && t.contains("10.1.2.3:20481"),
            "{t}"
        );
        poe_hook_deploy::deploy(&o).unwrap();
        let t = toml();
        assert!(
            t.contains("mode = \"official\"") && t.contains("login_mode = \"steam\""),
            "{t}"
        );
        assert!(
            t.contains("trusted_servers = []") && !t.contains("10.1.2.3"),
            "{t}"
        );
        assert_eq!(
            read("gateway_list.txt"),
            b"OFFICIAL-LIST",
            "the original list is back"
        );
    }

    #[test]
    fn missing_dlls_are_explained() {
        let tmp = tempfile::tempdir().unwrap();
        let e = check_dlls(tmp.path()).unwrap_err().to_string();
        assert!(e.contains("poe_hook.dll") && e.contains("Settings"), "{e}");
        std::fs::write(tmp.path().join("poe_hook.dll"), b"a").unwrap();
        std::fs::write(tmp.path().join("poe_hook_core.dll"), b"b").unwrap();
        check_dlls(tmp.path()).unwrap();
    }

    /// The whole file side of Play against a temporary directory: deploy with fake DLLs, then
    /// the login token into the prefix's Documents folder.
    #[test]
    fn deploy_and_token_land_where_the_client_looks() {
        let tmp = tempfile::tempdir().unwrap();
        let c = cfg(tmp.path());
        std::fs::create_dir_all(&c.install_dir).unwrap();
        std::fs::create_dir_all(&c.dll_dir).unwrap();
        std::fs::write(c.dll_dir.join("poe_hook.dll"), b"loader").unwrap();
        std::fs::write(c.dll_dir.join("poe_hook_core.dll"), b"core").unwrap();
        let report = poe_hook_deploy::deploy(
            &deploy_options(&c, &info("play.example"), "127.0.0.1").unwrap(),
        )
        .unwrap();
        assert_eq!(report.login_mode, Some(LoginMode::Standalone));
        let toml = std::fs::read_to_string(c.install_dir.join("poe-hook.toml")).unwrap();
        assert!(
            toml.contains("login_mode = \"standalone\"") && toml.contains("mode = \"private\""),
            "{toml}"
        );
        assert!(
            toml.contains("127.0.0.1:20481") && toml.contains("127.0.0.1:6112"),
            "{toml}"
        );
        assert_eq!(
            std::fs::read_to_string(c.install_dir.join("steam_appid.txt")).unwrap(),
            "2694490"
        );
        assert!(c.install_dir.join("version.dll").is_file());
        assert_eq!(client_args(&c.install_dir), ["--success-event", "0"]);

        if !cfg!(windows) {
            let docs = token_documents_dir(&c.prefix_dir).unwrap();
            let token = [7u8; 32];
            let r = poe_hook_deploy::write_login_token(&docs, "alice", token).unwrap();
            assert_eq!(std::fs::read(&r.credentials).unwrap(), token);
            assert!(
                r.credentials.starts_with(
                    c.prefix_dir
                        .join("pfx/drive_c/users/steamuser/Documents/My Games/Path of Exile 2")
                )
            );
            let ini = std::fs::read_to_string(&r.ini).unwrap();
            assert!(
                ini.contains("username=alice") && ini.contains("save_login=true"),
                "{ini}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn the_proton_command_uses_the_launcher_prefix_and_never_steams_compatdata() {
        let tmp = tempfile::tempdir().unwrap();
        let c = cfg(tmp.path());
        let proton = c.steam_root.join("steamapps/common/Proton - Experimental");
        std::fs::create_dir_all(&proton).unwrap();
        std::fs::write(proton.join("proton"), b"#!/bin/sh\n").unwrap();
        std::fs::create_dir_all(&c.install_dir).unwrap();
        std::fs::write(
            c.install_dir.join("poe-hook.toml"),
            "login_mode = \"standalone\"\n",
        )
        .unwrap();
        let cmd = launch_command(&c).unwrap();
        let args: Vec<_> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args[0], "run");
        assert!(args[1].ends_with("PathOfExileSteam.exe"));
        assert_eq!(&args[2..], ["--success-event", "0"]);
        let env: std::collections::HashMap<_, _> = cmd
            .get_envs()
            .filter_map(|(k, v)| {
                Some((
                    k.to_string_lossy().into_owned(),
                    v?.to_string_lossy().into_owned(),
                ))
            })
            .collect();
        assert_eq!(
            env["STEAM_COMPAT_DATA_PATH"],
            c.prefix_dir.to_string_lossy()
        );
        assert!(!env["STEAM_COMPAT_DATA_PATH"].contains("compatdata"));
        assert_eq!(env["WINEDLLOVERRIDES"], "version=n,b");
        assert!(!crate::paths::is_steam_path(Path::new(
            &env["STEAM_COMPAT_DATA_PATH"]
        )));
        // Prefix creation reuses the environment with other arguments.
        let rebuilt = rebuild_with_args(cmd, &["run".into(), "wineboot".into(), "-u".into()]);
        assert_eq!(rebuilt.get_args().count(), 3);
        assert_eq!(rebuilt.get_envs().count(), env.len());
        assert!(!prefix_ready(&c.prefix_dir));
    }
}
