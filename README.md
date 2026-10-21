# Poe2Launcher

Desktop launcher for the Path of Exile 2 **0.5.5e** private server (Poe2ModdingProject): one small program per player that does what `cargo xtask deploy-client` and `run-client` do for the developer.

Tauri 2 (Rust backend in `src-tauri/`) with a Svelte 5 + Vite + TypeScript UI (`src/`). Dark theme by default.

| Page | Does |
|---|---|
| Servers | profiles: a name and the cluster's web API address; *Check* shows `GET /server` (realm, version, players online, whether registration is open) |
| Account | log in (`POST /api/v1/login`, kind `launcher`), create an account (`POST /register`), log out (`POST /logout` revokes the token). The token lives in the OS keyring only |
| Install | downloads client 0.5.5e from Steam depots with the player's own Steam login, then verifies it |
| Mods | list, add from a folder or zip, enable/disable, load order, compatibility check |
| Settings | install dir, hook DLL dir, Proton (Linux), Steam download options, theme, launcher updates (check, release notes, download + verify + restart) |
| Play (footer) | deploys the hook, writes the auto-login token, starts the game, shows the status |

The full guide (set-up, first run, troubleshooting) is `docs/guides/launcher.md` in Poe2ModdingProject; this file is the developer's view.

## Clone and build

The hook deploy library is a nested submodule, like Poe2ClientHook nests Poe2Protocol, so a recursive clone builds with no git tricks in cargo:

```sh
git clone --recursive https://github.com/Distorbed678/Poe2Launcher.git      # private: needs access to the three repos
cd Poe2Launcher
npm ci
npm run tauri dev                  # development window (Vite on :1420)
npm run tauri build -- --debug     # debug build + deb/rpm/AppImage on Linux
```

Linux build dependencies: `webkit2gtk-4.1`, `gtk3`, `libsoup3`, `dbus`, `librsvg` (Arch: `webkit2gtk-4.1 gtk3 libsoup3 librsvg`; Debian: `libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev libdbus-1-dev librsvg2-dev`). Node 22, current stable Rust.

In a checkout of Poe2ModdingProject the launcher is `modules/launcher`; its `src-tauri` has its own `[workspace]`, so the main workspace (which excludes `modules/`) never adopts it.

## Layout

```
src/                 Svelte UI: App.svelte (shell + Play bar), pages/, lib/ (ipc.ts, install.ts, mods.ts, update.ts (self-update logic), ...)
src-tauri/src/
  core.rs            service layer (everything the UI can do; no Tauri types, testable)
  commands.rs        thin #[tauri::command] wrappers; lib.rs wires events to the window
  api.rs             web API client (reqwest, rustls)
  profiles.rs        server profiles (profiles.json, no secrets)
  keystore.rs        TokenStore: KeyringStore (keyring crate) / MemoryStore (tests)
  settings.rs        settings.json
  version.rs         versions/<version>.toml (embedded by build.rs: every file, newest build_revision last)
  depot.rs           DepotDownloader: pinned fetch, argument builder, prompt/progress parser, runner
  verify.rs          install verification
  play.rs            deploy options, token file, Proton/native launch (poe-hook-deploy)
  mods.rs            mod.toml validation, compatibility, load order, state.toml, zip/folder install
  paths.rs           launcher dirs and the "never the Steam install" guard
versions/
  <version>.toml     COPY of Poe2ModdingProject's versions/<version>.toml (the single source; refreshed by
                     `cargo xtask versions` there, never edited here): app, depots + manifests (install
                     order), [exe] and [verify] values; the other sections are ignored
  depotdownloader.toml  pinned DepotDownloader release, per-platform SHA-256
client-hook/         submodule: Poe2ClientHook (poe-hook-deploy used as a path dependency)
```

## Data and secrets

| What | Where |
|---|---|
| Launcher token (+ account name, expiry) | OS keyring: service `poe2-launcher`, user = profile id (Secret Service / Credential Manager / Keychain). **Never a file** |
| Profiles, settings | `<config>/poe2-launcher/profiles.json`, `settings.json` (`~/.config/...`, `%APPDATA%\...`) |
| DepotDownloader, game, Proton prefix, logs | `<data>/poe2-launcher/{tools,game/<version>,proton-prefix,logs,crash-dumps}` |
| Steam password | typed in the UI, written once to DepotDownloader's stdin when it asks, wiped. Not stored, not on a command line |

`POE2_LAUNCHER_HOME=<dir>` puts config and data under `<dir>/config` and `<dir>/data` (tests, portable use).

**Never the Steam install.** The launcher does not read or write `steamapps/common/Path of Exile 2` or `compatdata/2694490`. Install, DLL and similar directories inside a Steam library are refused (`paths.rs`); the Proton prefix is its own. On Linux it only reads Proton itself from `<steam root>/steamapps/common/<Proton>` to start it.

## Game install

The version file (`versions/<version>.toml`, a byte-identical copy of the main repo's file, which is the single definition of a supported build): for 0.5.5e app 2694490; depot 2694491 manifest 7366357142270402355 (content), depot 2694492 manifest 246779027001700092 (Windows exe). Both go into one directory in file order (exe over content), in **one** DepotDownloader run so Steam Guard is asked once.

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

Official servers are never touched: the deploy is always private mode.

The hook DLLs come from the *Hook DLL directory* setting: a build of Poe2ClientHook (`cargo build --release --target x86_64-pc-windows-msvc` there, or `cargo xtask build-dll` in Poe2ModdingProject), i.e. `modules/client-hook/target/x86_64-pc-windows-msvc/release` holding `poe_hook.dll` and `poe_hook_core.dll`. Bundling them is not done yet.

## Mods

Spec: `modules/client-hook/docs/mod-manifest.md` (Poe2ClientHook). Mods are installed into `<install>/poe-hook-mods/<id>/` and `<install>/poe-hook-mods/state.toml` holds the contract with the DLL's mod loader (`poe-hook/src/mods.rs` in Poe2ClientHook; it loads the enabled mods in this order, never in official mode):

```toml
enabled = ["core-lib", "my-mod"]   # in load order
```

The launcher validates `mod.toml` (all rules of the spec, every problem listed), checks `game.versions` against the install's client version (refuses enabling a mismatch unless overridden) and `game.hook_api` (launcher offers API 1), enables dependencies first (semver check), refuses conflicts and ordering cycles, and keeps the order as topological order of dependencies and `load.after/before` with the user's arrangement as tie-breaker (*Auto order* = the spec's ties-by-id order). Installing from a zip refuses paths that escape, symlinks and oversized archives. Nothing under the game files is ever written by mod management.

## Tests and CI

```sh
npm run check && npm test                 # svelte-check + tsc, vitest
cd src-tauri && cargo test                # unit tests + tests/core_flow.rs (mock server, mock keyring, fake Proton)
cargo test --test fetch_tool -- --ignored          # downloads the pinned DepotDownloader, runs `-V` (never logs in)
cargo test --test core_flow real_keyring -- --ignored   # needs an unlocked system keyring
POE_LAUNCHER_TEST_URL=http://127.0.0.1:18080 POE_LAUNCHER_TEST_USER=u POE_LAUNCHER_TEST_PASS=p \
  cargo test --test core_flow real_server -- --nocapture   # against a real poe-server
```

GitHub Actions (`.github/workflows/ci.yml`): Ubuntu (WebKitGTK dev packages, `npm run check/test/build`, `cargo fmt --check`, `clippy -D warnings`, `cargo test`) and Windows (`cargo check`). The private submodules are fetched with two read-only **deploy keys** stored as the secrets `HOOK_DEPLOY_KEY` (on Poe2ClientHook) and `PROTOCOL_DEPLOY_KEY` (on Poe2Protocol); a deploy key can belong to one repository only. To rotate: generate a new key pair, replace the deploy key in that repository's settings and the secret here.

## Self-update and releases

The launcher updates itself with the Tauri updater plugin (`tauri-plugin-updater`, `tauri-plugin-process` for the restart; capabilities `updater:default`, `process:allow-restart`). `tauri.conf.json` has `bundle.createUpdaterArtifacts = true` and `plugins.updater`: the endpoint `https://github.com/Distorbed678/Poe2Launcher/releases/latest/download/latest.json`, the **public** signing key (`pubkey`), Windows install mode `passive`. The UI side is `src/lib/update.ts` (logic, tested with a fake plugin), `update.svelte.ts` (state), `ui/UpdatePanel.svelte` (Settings) and the badge in the top bar: a check runs at start; a failed check (404 on a private repo or before the first release, offline) is the quiet "no update info" state, a failed download or signature check is shown as an error and installs nothing. Linux updates replace the AppImage in place (only works when running as an AppImage); Windows runs the NSIS installer.

`.github/workflows/release.yml` builds on a pushed tag `v*`: a draft release with generated notes, then the AppImage (`ubuntu-22.04`, `tauri build --bundles appimage`) and the NSIS installer (`windows-latest`, `--bundles nsis`) with `tauri-apps/tauri-action@v1`, signed with the secrets `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, `latest.json` merged, and the release published at the end. The submodules are fetched with the same two deploy keys as `ci.yml` (until the repos are public). How to tag, make the secrets and look after the key: the "Releasing" section of `docs/guides/launcher.md` in Poe2ModdingProject. Local check: `TAURI_SIGNING_PRIVATE_KEY=... TAURI_SIGNING_PRIVATE_KEY_PASSWORD=... npx tauri build --bundles appimage` gives the AppImage and its `.sig`.

## Commands the UI invokes

`app_info`, `profiles_list`, `profile_add`, `profile_update`, `profile_remove`, `server_info`, `account_login`, `account_register`, `account_logout`, `settings_get`, `settings_set`, `install_status`, `install_start`, `install_input`, `install_cancel`, `install_verify`, `mods_list`, `mod_install`, `mod_enable`, `mod_disable`, `mod_move`, `mod_auto_order`, `mod_remove`, `mods_dir`, `play`, `play_state`. Arguments are camelCase from JS (`profileId`, `overrideCompat`); errors are readable strings.

Events: `install:state` (`fetching_tool`, `downloading`, `verifying`, `done`, `failed`, `cancelled`), `install:log`, `install:progress`, `install:prompt` (Steam Guard), `install:verify`, `play:status` (`idle`, `preparing`, `running`, `exited`), `play:warning`.

## Licence

MIT OR Apache-2.0, like the other repos of the project.
