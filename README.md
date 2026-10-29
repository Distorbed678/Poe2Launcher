# Poe2Launcher

Desktop launcher for the Path of Exile 2 private server (Poe2ModdingProject; supports every client build that has a `versions/<version>.toml` in Poe2Protocol, today 0.5.5e.hotfix1, plus an *Official (read-only)* mode): one small program per player that does what `cargo xtask deploy-client` and `run-client` do for the developer.

Tauri 2 (Rust backend in `src-tauri/`) with a Svelte 5 + Vite + TypeScript UI (`src/`). Game-styled dark theme by default (light parchment in Settings) from the design tokens in `src/tokens.css` and `src/lib/motion.ts` (identical copies in the web admin), a custom window frame, fades and slides that follow `prefers-reduced-motion`. A development preview without the backend: `npm run dev`, then open `http://localhost:1420/?mock` in a browser (`&versions` adds a second game version, `&official` the official mode, `&empty`, `&fresh`, `&mods` (a mod cache), `&modsync[=off|auto]` (the server requires two mods), `&theme=light`, `&installing`, `&paused`, `&lowspace`, `&offline`, `&nonews`, `&motd`, `&nolist`, `&nobackups`, `&nologs`; the mock lives in `src/lib/mock.ts` and is not in release builds).

**Window:** a thin top strip with minimize / maximize / close (drag to move, double-click to maximize, resize handles on all edges) and a sidebar with the brand, the **game version picker**, the pages (Play, Servers, News, Library, Settings), the update badge, the account and About.

| Page | Does |
|---|---|
| Game version picker (top of the sidebar) | *Official (read-only)* first, then every embedded private version (newest first). Saved as `game_version` in `settings.json`. Drives the install folder and depots, verify, deploy, Play, the server-release filter of *Create a server*, mods and the server compatibility marks. Refused while an install, the game or the hosted server runs |
| Play (`#/home`) | The selected server with live status (online, players, ping measured by the launcher; click to change), the big Play button with one state line, the *Get ready to play* checklist, the newest news item. **News** (its own page) (release notes of Poe2Launcher, Poe2Server `server-v*` and Poe2ClientHook `hook-v*` from the GitHub releases API, plus `news.json` of the hub repository Poe2ModdingProject; cached, quiet offline), a notice when a **server update** is available and the server's optional message of the day (`motd` of `GET /api/v1/server`) |
| Servers | *My servers*: profiles (a name and the cluster's web API address); *Check* shows `GET /server`; **log in, create an account, log out** inside each server's card (token in the OS keyring only); a server reporting another client version is marked *incompatible* / *needs client X* (with *Switch*); a plain `http://` address to another machine asks for a confirmation first (see Security); admins see *Web admin*. *Browse*: the public list (`Distorbed678/Poe2ServerList`, `servers.json`), live status and ping asked by the launcher itself, filter/search/sort, *Add to my servers* |
| Servers > Host: several servers on this computer (own folder, ports, config, backups; shared packages), Create, Manage (tabs Overview with usage and log, Settings in folding sections, Program, Backups), Delete (type the name) | download the matching `poe-server` release from Poe2Server (SHA-256 checked; the two earliest releases in Poe2ModdingProject are the fallback; both package layouts) or pick a local package, **server updates** (a quiet check at start and when Manage or Play opens; *Update* = download and verify, graceful stop, database backup, install next to the old package, config carried over from the new `server.example.toml` plus your changes, start again, rollback of package, config and database on a failure), settings (name, ports, public host, registration, remote web API), start and stop with a streamed log, usage (CPU, memory, tick time, network), optional autostart, restart supervision (exit code 75 from the web admin's Restart starts it again), **database backups** (online SQLite backup, restore with a safety backup, automatic backup before each start, keep the last N); the server is added to My servers. No accounts are made in the launcher: the first admin in the web admin, players on the server's register page. Private versions only |
| Library | *Game*: downloads the selected client from Steam depots with the player's own Steam login (pinned manifests for a private version, the current build for Official) with pause/resume, speed and time left and a disk-space check, verify/repair, and the **Client hook** card. *Mods*: the mod cache per server (Installed) and the mod browser (Browse): side badges, add a package or folder, enable per server or for all, load order, compatibility check, the server's required mods (private versions only). *Logs*: game logs, the server log, crash dumps, search, *Copy for a bug report* (redacted) |
| Settings | per-version install dir, hook DLL dir, Proton (Linux), **Steam account** (*Logged in as X* / *Not logged in*, *Log out of Steam*), Steam download options, theme, window frame (own or system), *Minimize the launcher while the game is running* (off by default), the desktop entry (Linux), launcher updates (check, release notes, download + verify + restart). About (version, supported game versions, data folders, disclaimer) is the link at the foot of the sidebar |
| Play (the Play page) | private version: deploys the hook, writes the auto-login token, starts the game. Official: the one-time risk confirmation, then the read-only deploy (`mode = "official"`, empty `trusted_servers`, original gateway list, Steam login in the game) and the game; no token, no login. Shows the status |

The full guide (set-up, first run, troubleshooting) is `docs/guides/launcher.md` in Poe2ModdingProject; this file is the developer's view.

## Clone and build

The hook deploy library is a nested submodule, like Poe2ClientHook nests Poe2Protocol, so a recursive clone builds with no git tricks in cargo:

```sh
git clone --recursive https://github.com/Distorbed678/Poe2Launcher.git      # pulls Poe2ClientHook and Poe2Protocol too
cd Poe2Launcher
npm ci
npm run tauri dev                  # development window (Vite on :1420)
npm run tauri build -- --debug     # debug build + deb/rpm/AppImage on Linux
```

Linux build dependencies: `webkit2gtk-4.1`, `gtk3`, `libsoup3`, `dbus`, `librsvg` (Arch: `webkit2gtk-4.1 gtk3 libsoup3 librsvg`; Debian: `libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev libdbus-1-dev librsvg2-dev`). Node 22, current stable Rust.

In a checkout of Poe2ModdingProject the launcher is `modules/launcher`; its `src-tauri` has its own `[workspace]`, so the main workspace (which excludes `modules/`) never adopts it.

## Layout

```
src/                 Svelte UI: App.svelte (window shell, sidebar), pages/ (Home = Play, Servers, News, Library (tabs Install = Game, Mods, Logs), Settings, About), lib/ (ipc.ts, app.svelte.ts (state), gameversion.ts, install.ts, transfer.ts (speed, time left), markdown.ts (safe subset), browse.ts (server list filter/sort), logview.ts, rate.svelte.ts, mods.ts, update.ts, frame.ts (window calls), motion.ts, mock.ts (dev preview), ui/ (HostPanel (the former Host page), Drawer, VersionPicker, Icon, Dropdown, Tabs, Markdown, AccountPanel, ServerBrowser, Switch, Chip, Progress, Toasts, ConfirmDialog, TitleBar, ...)), tokens.css (design tokens), app.css (all components), assets/
assets/icon.svg      the app icon source (original artwork); `npx tauri icon assets/icon.svg` regenerates src-tauri/icons/
src-tauri/src/
  core.rs            service layer (everything the UI can do; no Tauri types, testable)
  commands.rs        thin #[tauri::command] wrappers; lib.rs wires events to the window
  api.rs             web API client (reqwest, rustls), incl. GET /me and POST /web-token
  host.rs            Host a server, pure parts: settings and folders per server (servers/<id>.json), migration of the old single server, release lookup (Poe2Server, fallback Poe2ModdingProject), checksum, package check (both layouts) and extraction, server.toml from server.example.toml (+ `carry_over_config` for updates), LAN/public IP, port check
  host_run.rs        the poe-server process: start/stop, streamed log ring buffer, one-shot commands
  hook.rs            Client hook, pure parts: release lookup (GitHub, tag `hook-v*`), asset names, `release.json`, per-file SHA-256 check, safe install into `<data>/hook/packages/<version>`, which package serves a game version
  hook_ops.rs        Client hook on Core: status, check for an update, download + verify + install, `hook_dll_dir` (manual directory > downloaded > old default folder), the check before each deploy
  host_ops.rs        Hosted servers on Core: create/delete, install (download, local or shared), apply settings (config), start/stop/status/usage
  host_update.rs     server updates on Core: the quiet check and the one-click update with rollback (see docs/guides/launcher.md, Updating the server)
  backup.rs          server database backups: SQLite online backup, list, restore with a safety backup, prune, settings (backup.json)
  backup_ops.rs      backups on Core (+ the automatic backup before a start)
  news.rs            Home news: GitHub release notes of Poe2Launcher, Poe2Server and Poe2ClientHook + news.json, validation, on-disk cache
  serverlist.rs      the public server list (Poe2ServerList): fetch, per-entry validation, cache, parallel probes of GET /server with ping
  logs.rs            log and crash viewer: listing by id, tail reading, the bug-report text and its redaction
  space.rs           disk-space check before an install, depot weights for the progress estimate
  profiles.rs        server profiles (profiles.json, no secrets)
  keystore.rs        TokenStore: KeyringStore (keyring crate) / MemoryStore (tests)
  settings.rs        settings.json
  version.rs         versions/<version>.toml (embedded by build.rs: every file, newest build_revision last); VersionFile::resolve (picked, else newest), VersionFile::official() (no pinned manifests)
  steam.rs           the remembered Steam login (account.config of DepotDownloader): status and log out
  desktop.rs         Linux .desktop entry + icons for the AppImage (install / uninstall / status)
  depot.rs           DepotDownloader: pinned fetch, argument builder, prompt/progress parser, runner
  verify.rs          install verification
  play.rs            deploy options, token file, Proton/native launch (poe-hook-deploy)
  mods.rs            the launcher's view of poe-hook-mods: compatibility (game version, hook ABI), load-order resolver, server-half paths
  modcache.rs        the mod cache (verified packages, several versions, client half only)
  modsets.rs         per-profile mod sets, effective set, apply_to_game (poe-hook-mods/ + state.toml)
  modsync.rs         the server's `mods` advertisement, compare, Off/Ask/Auto, verified download
  modindex.rs        the mod browser's index.json (Poe2Mods): parse, cache, install
  mod_ops.rs         Core operations for all of the above, prepare_mods at Play
  paths.rs           launcher dirs and the "never the Steam install" guard
versions/
  <version>.toml     COPY of Poe2Protocol's versions/<version>.toml (the single source, the hub's
                     modules/protocol/versions/; refreshed by `cargo xtask versions` in the hub, never edited here): app, depots + manifests (install
                     order), [exe] and [verify] values; the other sections are ignored
  depotdownloader.toml  pinned DepotDownloader release, per-platform SHA-256
client-hook/         submodule: Poe2ClientHook (poe-hook-deploy used as a path dependency)
```

## Data and secrets

| What | Where |
|---|---|
| App identifier (`org.poe2modding.launcher`) | The window's Wayland `app_id` / WM class (`glib::set_prgname` in `lib.rs`), so the compositor finds `org.poe2modding.launcher.desktop`, and the webview storage folder; the keyring service and folders below are fixed names and do not derive from it |
| Launcher token (+ account name, expiry) | OS keyring: service `poe2-launcher`, user = profile id (Secret Service / Credential Manager / Keychain). **Never a file** |
| Profiles, settings | `<config>/poe2-launcher/profiles.json`, `settings.json` (`~/.config/...`, `%APPDATA%\...`) |
| DepotDownloader, games, Proton prefixes, logs | `<data>/poe2-launcher/{tools,game/<version>,game/official,proton-prefix,proton-prefix-official,logs,crash-dumps}` |
| Remembered Steam login (DepotDownloader's `account.config`, a token, never the password) | `<data>/poe2-launcher/steam-login/IsolatedStorage/...` on Linux (the tool runs with `XDG_DATA_HOME` there); `%LOCALAPPDATA%\IsolatedStorage` on Windows. *Log out of Steam* deletes it |
| `.desktop` entry and icons (only if you press *Install desktop entry*) | `~/.local/share/applications/org.poe2modding.launcher.desktop`, `~/.local/share/icons/hicolor/*/apps/org.poe2modding.launcher.*` |
| Hosted servers' settings (name, ports, public host; no secrets) | `<config>/poe2-launcher/servers/<id>.json` (a launcher before 0.6.0 kept one in `host.json`; it moves to `s1`) |
| News and server-list cache (copies, safe to delete) | `<data>/poe2-launcher/cache/{news.json,serverlist.json}` |
| Backup choices (automatic backup, how many to keep) | `<config>/poe2-launcher/servers/<id>.backup.json` |
| Server packages (shared) | `<data>/poe2-launcher/server/{packages,downloads}` |
| A hosted server's database, config, mods, log, backups | `<data>/poe2-launcher/servers/<id>/{data,server.toml,mods,config-backups,server.log,backups}` (outside the package, so an update keeps accounts and characters) |
| Steam password | typed in the UI, written once to DepotDownloader's stdin when it asks, wiped. Not stored, not on a command line |

`POE2_LAUNCHER_HOME=<dir>` puts config and data under `<dir>/config` and `<dir>/data` (tests, portable use).

**Never the Steam install.** The launcher does not read or write `steamapps/common/Path of Exile 2` or `compatdata/2694490`. Install, DLL and similar directories inside a Steam library are refused (`paths.rs`); the Proton prefix is its own. On Linux it only reads Proton itself from `<steam root>/steamapps/common/<Proton>` to start it.

## Game versions and install

The dropdown lists every embedded `versions/<version>.toml` (`build.rs` embeds all of them, so adding a build is adding a file; see `docs/guides/new-game-version.md` in Poe2ModdingProject) plus *Official (read-only)*, which reuses the newest file's depots without manifests, with its own folder `game/official` and the hook in read-only mode. The selection is `Settings.game_version`; install folders are per version (`Settings.install_dirs`; the old single `install_dir` is moved to the oldest version when read).

The version file (`versions/<version>.toml`, a byte-identical copy of Poe2Protocol's file, which is the single definition of a supported build): for 0.5.5e.hotfix1 app 2694490; depot 2694491 manifest 4712263798014036045 (content), depot 2694492 manifest 4973272861086296374 (Windows exe). Both go into one directory in file order (exe over content), in **one** DepotDownloader run so Steam Guard is asked once.

- DepotDownloader **3.4.0**, the self-contained release (no .NET install), pinned with SHA-256 in `versions/depotdownloader.toml`; downloaded over HTTPS to `<data>/tools/`, refused and deleted on a mismatch.
- Arguments: `-app 2694490 -depot A -manifest MA -depot B -manifest MB -username U [-remember-password] [-no-mobile] -dir DIR [-validate]`. `-remember-password` only if the box is ticked; there is never a `-password`.
- The runner reads output as raw chunks (the prompts have no line end), recognises `Enter account password for ...` (answered from the form), `STEAM GUARD! ...` (device or email code: the UI asks, the code goes to stdin), parses `NN.NN% file` progress and `Downloading depot N`, and reports failures (`Failed to authenticate...`).
- Verification (`verify.rs`): file count and size floors, key files with exact sizes, SHA-256 of `PathOfExileSteam.exe`. The reference numbers in the version file were measured on the GameFiles copy in Poe2ModdingProject, not on a launcher download: they are unconfirmed until a first real install (a checksum mismatch is a warning until `[exe] sha256_enforce = true`).

## Play

1. The server's `GET /server` gives the login and instance addresses (`login_host`, `login_port`, `instance_port`, `gateway_region`).
2. `POST /api/v1/game-token` (launcher token in the `Authorization` header) returns a fresh `game_remember` token. The launcher token itself is never given to the game: the game's login retires a token when it uses it, which would log the launcher out.
3. `poe_hook_deploy::write_login_token`: the 32-byte token file and `[LOGIN] username=..., save_login=true` in the client's `Documents/My Games/Path of Exile 2/` (inside the launcher's Proton prefix on Linux).
4. `poe_hook_deploy::deploy`: `gateway_list.txt` with one entry, `version.dll` + `poe_hook_core.dll` from the configured DLL directory, `poe-hook.toml` with `mode = "private"`, `login_mode = "standalone"`, `trusted_servers` = the server's login and instance `host:port`, `steam_appid.txt`.
5. Launch: Linux `proton run PathOfExileSteam.exe --success-event 0` through `ProtonLaunch` (launcher-owned prefix; the prefix is created with `proton run wineboot -u` on first use; Steam must be running). Windows: `PathOfExileSteam.exe --success-event 0` directly (written, not testable on Linux CI). Game output goes to `<data>/logs/game-<time>.log`.

Official (read-only) is the one other deploy: `play::official_deploy_options` writes exactly what `cargo xtask deploy-client --official` does and never a trusted server or gateway; its tests are in `play.rs` and `tests/version_switch.rs`.

The hook DLLs (`hook.rs`, `hook_ops.rs`, UI: `ui/HookPanel.svelte`, `ui/SetupChecklist.svelte`, `lib/hook.ts`) come from, in this order: the *Hook DLL directory* setting (advanced; a build of Poe2ClientHook, e.g. `modules/client-hook/target/x86_64-pc-windows-msvc/release`), the **client hook downloaded by the launcher**, or files in the old default folder `<data>/hook-dlls`. The download is the newest stable release of `Distorbed678/Poe2ClientHook` (tag `hook-v*`, asset `poe-hook-v<ver>-game-<games>-windows-x86_64.zip` + `.sha256`) whose name lists the selected game version (Official uses the newest private version's). The archive is checked against the `.sha256`, unpacked with the same guards as server packages, every file is checked against the size and SHA-256 in the package's `release.json` (an unlisted file refuses the package) and the package must support the game version. It lives in `<data>/hook/packages/<hook version>/`; the same hashes are checked again before each deploy. Without any hook Play is blocked. Details and release process: `docs/guides/launcher.md` (Client hook) and `docs/guides/releasing-hook.md` in Poe2ModdingProject.

## Mods

Spec: `modules/client-hook/docs/mod-manifest.md` (Poe2ClientHook); the manifest (v1 and v2) is parsed by the shared `poe-hook-mods` crate (the nested `client-hook/` checkout, a path dependency, the same crate the hook's loader, the server and the SDK use). A mod has a **side**: `client`, `server` or `both`. The launcher's store holds the client half only: a server-only mod is refused with a message, and a `both` package is unpacked without its `server/` half (so no server file lands on the player's disk).

**Cache.** `<data>/mods/cache/<id>/<version>/` holds every verified version (`mod/` = the unpacked client half, `meta.json` = SHA-256, size, source). A package enters only through `poe_hook_mods::verify_package` / `verify_package_bytes` (hash, `<id>-<version>.zip` name, paths, links, limits, CRCs, manifest): unpacking happens in a scratch folder renamed into place at the end, so a failed check leaves nothing (`modcache.rs`).

**Per-server sets.** A server profile (`profiles.json`, `mods`) stores the mods enabled for it (id + exact version); `<config>/modsets.json` holds the ones enabled for all servers. At Play the set is: the server's required mods (always on, the server's version) + the profile's + the global ones, with dependencies pulled from the cache, conflicts/incompatible mods dropped with a note, ordered by dependencies and `after/before` (`modsets.rs`). `apply_to_game` copies it from the cache into `<install>/poe-hook-mods/<id>/` (a marker file says the launcher put it there; other folders are never touched) and writes `state.toml`, removing what the previous server needed. The official (read-only) entry has no mods and none of this runs.

**Mod sync** (`modsync.rs`, `mod_ops.rs`). `GET /api/v1/server` may carry `mods` (an array, or `{"client_mods": [...]}`); each entry: `id`, `version`, `side` (`client`/`both`; `server` entries are ignored), `sha256` (64 hex), `size`, optional `required` (default true), `hook_api`, `game_versions`, `sources` (`{"type":"server","path":"/api/v1/mods/<id>/<version>.zip"}` and/or `{"type":"url","url":"https://..."}`; with none, the default server path is used), `name`, `authors`, `description`, optional `code` (default: has a `hook_api`). The launcher compares with the cache (ready / missing / other build / incompatible / no source) and acts by the **Mod sync** setting (`mod_sync` in `settings.json`): `off` shows what is missing and blocks Play with an explanation, `ask` (default) shows a dialog (name, version, side, size, code) with Download, `auto` downloads, verifies and switches silently with a toast, code mods included. A download is kept only if its SHA-256 equals the advertised one and `verify_package` accepts it; size is capped at the advertised size. Plain `http://` to another machine needs the profile's confirmation (the server's path is then not used); extra URLs must be `https://` (or this computer). Old servers without `mods` work unchanged; an unreadable `mods` field counts as none.

**Mod browser** (`modindex.rs`, Library > Mods > Browse): reads `index.json` of `Distorbed678/Poe2Mods` (schema in `docs/mod-index-template/` of Poe2ModdingProject), validated entry by entry, cached 10 min (`<data>/cache/modindex.json`), quiet and showing the saved copy offline; filter by text, side, tag and game version; install a version into the cache (hash from the index) and enable it for the selected server or all servers.

**Settings**: *Mod sync* (Off / Ask / Auto), cache size, *Clear unused cached mods* (versions no set enables and no server's last advertisement requires).

## Tests and CI

```sh
npm run check && npm test                 # svelte-check + tsc, vitest
cd src-tauri && cargo test                # unit tests + tests/mods_flow.rs (mock server and mod index: Off/Ask/Auto, tampered packages, per-server sets) + tests/version_switch.rs (version switch, official Play deploy, Steam log-out) + tests/core_flow.rs (mock server, mock keyring, fake Proton), tests/web_admin.rs (Open web admin, plain-http guard) tests/host_flow.rs (mock GitHub API, fake server packages), tests/host_update.rs (server update: mock GitHub API with a fallback repository, fake packages in both layouts, shell-script servers: update, config carry-over, rollback) and tests/hook_flow.rs (mock GitHub API, fake hook packages: find, download, verify, install, refusals, override)
cargo test --test fetch_tool -- --ignored          # downloads the pinned DepotDownloader, runs `-V` (never logs in)
cargo test --test host_flow real_package -- --ignored --nocapture   # a REAL server package: install, set up, start, status, log, stop (needs POE2_TEST_SERVER_PACKAGE and POE2_TEST_GAME_DIR, see the test's comment)
cargo test --test hook_flow real_release -- --ignored --nocapture   # the REAL hook release of Poe2ClientHook from GitHub (network; needs POE2_TEST_HOOK_REAL=1)
cargo test --test core_flow real_keyring -- --ignored   # needs an unlocked system keyring
POE_LAUNCHER_TEST_URL=http://127.0.0.1:18080 POE_LAUNCHER_TEST_USER=u POE_LAUNCHER_TEST_PASS=p \
  cargo test --test core_flow real_server -- --nocapture   # against a real poe-server
```

GitHub Actions (`.github/workflows/ci.yml`): Ubuntu (WebKitGTK dev packages, `npm run check/test/build`, `cargo fmt --check`, `clippy -D warnings`, `cargo test`) and Windows (`cargo check`). The submodules (client-hook and its nested protocol, both public) are fetched by `actions/checkout` with `submodules: recursive`.

## Window, icon and Wayland

`tauri.conf.json`: `decorations: false`, `visible: false` (shown from `setup` after the frame choice and the window icon are applied), square window (a transparent one rendered only partly under WebKitGTK/Wayland in a test), capabilities for minimize, maximize, close, start-dragging, start-resize-dragging and set-decorations. `data-tauri-drag-region` marks the draggable parts of the title bar; the resize handles are `ui/ResizeHandles.svelte` (`startResizeDragging`). `Settings.custom_frame = false` switches to the system frame at runtime. On Linux `lib.rs` sets GTK's program name to the identifier before GTK starts, which is the Wayland `app_id`; the window icon on Wayland comes from the `.desktop` file (the bundle's, or the one written by Settings, Desktop entry; `desktop-template.desktop` gives the bundled one `StartupWMClass=org.poe2modding.launcher` and `Categories=Game;`).

## Self-update and releases

The launcher updates itself with the Tauri updater plugin (`tauri-plugin-updater`, `tauri-plugin-process` for the restart; capabilities `updater:default`, `process:allow-restart`). `tauri.conf.json` has `bundle.createUpdaterArtifacts = true` and `plugins.updater`: the endpoint `https://github.com/Distorbed678/Poe2Launcher/releases/latest/download/latest.json`, the **public** signing key (`pubkey`), Windows install mode `passive`. The UI side is `src/lib/update.ts` (logic, tested with a fake plugin), `update.svelte.ts` (state), `ui/UpdatePanel.svelte` (Settings) and the badge in the top bar: a check runs at start; a failed check (404 on a private repo or before the first release, offline) is the quiet "no update info" state, a failed download or signature check is shown as an error and installs nothing. Linux updates replace the AppImage in place (only works when running as an AppImage); Windows runs the NSIS installer.

`.github/workflows/release.yml` builds on a pushed tag `v*`: a draft release with generated notes, then the AppImage (`ubuntu-22.04`, `tauri build --bundles appimage`) and the NSIS installer (`windows-latest`, `--bundles nsis`) with `tauri-apps/tauri-action@v1`, signed with the secrets `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, `latest.json` merged, and the release published at the end. The submodules are checked out like in `ci.yml`. How to tag, make the secrets and look after the key: the "Releasing" section of `docs/guides/launcher.md` in Poe2ModdingProject. Local check: `TAURI_SIGNING_PRIVATE_KEY=... TAURI_SIGNING_PRIVATE_KEY_PASSWORD=... npx tauri build --bundles appimage` gives the AppImage and its `.sig`.

## Commands the UI invokes

`app_info`, `profiles_list`, `profile_add`, `profile_update`, `profile_remove`, `profile_confirm_plain_http`, `server_info`, `account_login`, `account_register`, `account_logout`, `account_refresh`, `web_admin_open`, `settings_get`, `settings_set`, `game_version_set`, `steam_login_status`, `steam_logout`, `desktop_entry_status`, `desktop_entry_install`, `desktop_entry_uninstall`, `install_status`, `install_start`, `install_input`, `install_cancel`, `install_verify`, `mods_list`, `mod_install`, `mod_set_enabled`, `mod_move`, `mod_auto_order`, `mod_remove`, `mods_dir`, `mod_cache_status`, `mod_cache_prune`, `mod_sync_check`, `mod_sync_apply`, `mod_index_get`, `mod_index_install`, `play` (the profile id is optional: the official mode takes none), `play_state`, for the client hook `hook_status`, `hook_check`, `hook_install_latest`, and for hosting `host_status`, `host_find_release`, `host_install_latest`, `host_update_check`, `host_update_apply`, `host_install_local`, `host_lan_ip`, `host_public_ip`, `host_setup`, `host_start`, `host_stop`, `host_log_tail`, `host_set_autostart`, `host_open_guide`. Arguments are camelCase from JS (`profileId`, `overrideCompat`); errors are readable strings.

Events: `install:state` (`fetching_tool`, `downloading`, `verifying`, `done`, `failed`, `cancelled`), `install:log`, `install:progress`, `install:prompt` (Steam Guard), `install:verify`, `play:status` (`idle`, `preparing`, `running`, `exited`), `play:warning`, for the client hook `hook:progress` and `hook:step`, `hook:installed`, and for hosting `host:status` (`stopped`, `running`, `stopping`, `exited`), `host:log`, `host:progress`, `host:setup` (step text), `host:installed`.

## Security

- Passwords and tokens: the keyring only; `Debug` output redacts them; the Steam password never touches a command line or a file; the launcher handles no server account passwords except the login it sends to the server.
- **Plain `http://`** (audit S-03): to an address that is not this computer, the launcher sends nothing (no login, no registration, no token for Play or the web admin) until you confirm that address on the Servers or Account page; the confirmation is per address and withdrawn when the address changes. A bare `host:port` gets `https://` unless it is this computer or a local network.
- **Open web admin** (admins only): `GET /me` for the role, `POST /web-token` for a single-use token, then `<server address>/#/token-login?t=<token>` in the system browser (Tauri opener). The link is checked to be exactly that form before it is opened and is never logged. See `docs/architecture/web-api.md` in Poe2ModdingProject.
- Server packages are verified against their `.sha256` file; archives are unpacked with path, link and size checks.
- Reporting a vulnerability: [SECURITY.md](SECURITY.md).

## Disclaimer

This is an unofficial, fan-made project for personal and educational use. It is **not affiliated with, endorsed by or connected to Grinding Gear Games**; "Path of Exile" is their trademark. **No game files are distributed** in this repository or by the launcher's own releases: the Install page downloads the game from Steam with *your* Steam account, so you need your own legal copy. Use is at your own risk. The client hook that the launcher deploys can be detected as a modification by anti-cheat software. It is for the private server only: the launcher always deploys private mode, and use on official servers is limited to read-only observation by developers (not supported, not recommended).

Contributing: [CONTRIBUTING.md](https://github.com/Distorbed678/Poe2ModdingProject/blob/main/CONTRIBUTING.md) in the main repository (checks for this repo: `npm run check`, `npm test`, `npm run build`, and in `src-tauri` `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`).

## Licence

MIT: [LICENSE](LICENSE). Copyright (c) 2026 Distorbed678 and contributors.
