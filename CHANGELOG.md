# Changelog

## Unreleased

- **Client hook download.** The launcher gets the hook DLLs itself: the *Client hook* card on the Install page downloads the newest stable release of Poe2ClientHook for the selected game version (Official (read-only) uses the newest private version's hook), checks the archive against its `.sha256` and every file against `release.json` (refuses unlisted files, unsafe paths, links, a wrong game version), installs it in its own data folder per hook version and uses it for Play automatically; the files are checked again before each deploy. It shows the installed version, "update available" and a Download / Update button. Settings, *Hook DLL directory* stays as an advanced override that wins over the download. Play is blocked with "Get the client hook first" when there is no hook at all.
- **Get ready to play** checklist on the Servers page for a fresh launcher: 1. install the game, 2. get the client hook, 3. host a server or join one.

## 0.2.0

- New look: dark theme (light optional), sidebar and title bar, cards, status chips, progress bars, empty states, in-app dialogs, toasts, page transitions and hover/press feedback (all off with `prefers-reduced-motion`), a new About page.
- **Game version dropdown** in the title bar: every supported private-server version (from the embedded `versions/*.toml`, newest first) and, at the top, **Official (read-only)**. The choice is saved and drives the install folder (one per version), depots, verification, deploy, Play, Host a server's release filter and the server list; a server that reports another client version is marked incompatible (with *Switch*). Adding `versions/<v>.toml` adds an entry.
- **Official (read-only)**: plays on the official servers with the hook in observation mode (what `deploy-client --official` deploys: `mode = "official"`, `login_mode = "steam"`, empty `trusted_servers`, original gateway list, no token, no login), after a one-time risk confirmation; the install is the current official build in its own folder (and, on Linux, its own Proton prefix). Servers, Account, Host a server and Mods are hidden there.
- Settings: **Log out of Steam** (forgets DepotDownloader's remembered login and the saved account name; the tool now keeps its login in the launcher's own data folder on Linux), **Minimize the launcher while the game is running** (off by default), window frame choice, per-version install folders.
- New original app icon (`assets/icon.svg`, all sizes regenerated). Linux: the window's app id is `org.poe2modding.launcher`; the AppImage's bundled `.desktop` file matches it; Settings has **Install desktop entry** / **Remove** (`~/.local/share/applications` and the hicolor icon theme) so the Wayland compositor shows the icon.
- Custom window frame (no system decorations): own title bar with minimize / maximize / close, drag and double-click to maximize, resize handles; Settings can switch back to the system frame.

## 0.1.1

- Test release of the update path: an installed 0.1.0 finds, downloads, verifies and installs this version.

## 0.1.0

- First release: Servers, Account, Install (Steam download of the client), Host a server, Mods, Settings, self-update.
