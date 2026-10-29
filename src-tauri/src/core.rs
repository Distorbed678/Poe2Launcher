//! The launcher's service layer: everything the UI can do, independent of Tauri so it can be
//! tested with a mock keyring and a mock (or real) server. `commands.rs` is a thin wrapper.

use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::{mpsc, watch};

use crate::{
    api::{AccountInfo, ApiClient, ApiError, ServerInfo},
    depot::{self, DepotEvent, DownloadJob, Outcome, Secret, ToolPin},
    host_run::HostRuntime,
    keystore::{StoredSession, TokenStore},
    paths::{Dirs, reject_steam_paths},
    play::{self, PlayConfig},
    profiles::{PlainHttp, Profile, ProfileStore, needs_plain_http_confirmation, plain_http},
    settings::Settings,
    space, steam,
    verify::{self, VerifyReport},
    version::{OFFICIAL, VersionFile},
};

/// Receives the events the UI shows (install log, progress, prompts, play status).
pub trait Events: Send + Sync + 'static {
    fn emit(&self, event: &str, payload: serde_json::Value);
}

/// Drops every event.
pub struct NullEvents;
impl Events for NullEvents {
    fn emit(&self, _: &str, _: serde_json::Value) {}
}

/// Remembers events, for tests.
#[derive(Default)]
pub struct Collector(pub Mutex<Vec<(String, serde_json::Value)>>);
impl Events for Collector {
    fn emit(&self, event: &str, payload: serde_json::Value) {
        self.0.lock().unwrap().push((event.into(), payload));
    }
}

pub fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// Hands a link to the system browser (the Tauri opener in the app, a recorder in tests).
pub trait UrlOpener: Send + Sync {
    fn open(&self, url: &str) -> Result<()>;
}

#[derive(Debug, Clone, Serialize)]
pub struct ProfileView {
    pub id: String,
    pub name: String,
    pub url: String,
    pub logged_in: bool,
    pub account: Option<String>,
    pub expires_at: Option<i64>,
    /// `player` or `admin` as of the last login or refresh; the UI offers "Open web admin"
    /// only for `admin`.
    pub role: Option<String>,
    /// How exposed the address is: `https`, or plain http to `loopback`, `private` or `public`.
    pub plain_http: PlainHttp,
    /// The user confirmed sending the password and token in clear text to this address.
    pub plain_http_confirmed: bool,
    /// The keyring could not be read (shown instead of a silent "logged out").
    pub keyring_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LogoutResult {
    /// The server confirmed the revocation.
    pub revoked: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct InstallStatus {
    pub version: String,
    pub app_id: u32,
    pub install_dir: PathBuf,
    pub installed: bool,
    pub running: bool,
    pub tool_version: String,
    pub tool_ready: bool,
    pub depots: Vec<DepotView>,
    /// The estimated size of a whole install (0 = unknown): the overall progress, speed and time
    /// left are worked out from it.
    pub required_bytes: u64,
    /// A download that was paused and can be resumed.
    pub paused: Option<PausedInstall>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DepotView {
    pub id: u32,
    pub manifest: String,
    pub role: String,
    /// What the depot weighs in the overall progress (estimate).
    pub est_bytes: u64,
}

/// A paused download: what it needs to be started again. The password is not kept.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PausedInstall {
    pub version: String,
    pub steam_username: String,
    pub remember_login: bool,
}

#[derive(Clone, Deserialize)]
pub struct InstallRequest {
    pub steam_username: String,
    /// Used once, handed to DepotDownloader on its stdin, then wiped. Never stored.
    #[serde(default)]
    pub steam_password: String,
    pub remember_login: bool,
    /// Re-check existing files (`-validate`).
    #[serde(default)]
    pub validate: bool,
}

impl std::fmt::Debug for InstallRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InstallRequest")
            .field("steam_username", &self.steam_username)
            .field("steam_password", &"<redacted>")
            .field("remember_login", &self.remember_login)
            .field("validate", &self.validate)
            .finish()
    }
}

struct InstallControl {
    input: mpsc::UnboundedSender<String>,
    cancel: watch::Sender<bool>,
    /// The cancel is a pause: the download is stopped but can be resumed.
    pause: Arc<std::sync::atomic::AtomicBool>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum PlayState {
    Idle,
    Preparing { step: String },
    Running { pid: Option<u32> },
    Exited { code: Option<i32> },
}

/// One supported game version, for the version dropdown.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct GameVersionView {
    pub version: String,
    pub app_id: u32,
    /// The "Official (read-only)" entry (`version` is `official`).
    pub official: bool,
    /// The version the launcher currently works with.
    pub selected: bool,
    /// The newest version this launcher knows.
    pub newest: bool,
    /// Installed in its install directory.
    pub installed: bool,
}

pub struct Core {
    pub dirs: Dirs,
    pub tokens: Arc<dyn TokenStore>,
    pub events: Arc<dyn Events>,
    install: Mutex<Option<InstallControl>>,
    paused: Mutex<Option<PausedInstall>>,
    play: Mutex<PlayState>,
    /// The hosted `poe-server` processes by server id (see `host_ops.rs`).
    pub hosts: Mutex<std::collections::HashMap<String, HostRuntime>>,
    /// Set while the host page downloads, installs or sets up: one such job at a time.
    pub(crate) host_busy: Mutex<bool>,
    /// Set while the client hook is downloaded and installed.
    pub(crate) hook_busy: Mutex<bool>,
}

impl Core {
    pub fn new(
        dirs: Dirs,
        tokens: Arc<dyn TokenStore>,
        events: Arc<dyn Events>,
    ) -> Result<Arc<Self>> {
        // The single hosted server of launchers before 0.6.0 becomes server s1.
        if let Err(e) = crate::host::migrate_single_server(&dirs) {
            eprintln!("moving the hosted server to its new folder failed: {e:#}");
        }
        Ok(Arc::new(Self {
            dirs,
            tokens,
            events,
            install: Mutex::new(None),
            paused: Mutex::new(None),
            play: Mutex::new(PlayState::Idle),
            hosts: Mutex::new(std::collections::HashMap::new()),
            host_busy: Mutex::new(false),
            hook_busy: Mutex::new(false),
        }))
    }

    pub(crate) fn emit(&self, event: &str, payload: serde_json::Value) {
        self.events.emit(event, payload);
    }

    // --- settings ------------------------------------------------------------------------

    pub fn settings(&self) -> Result<Settings> {
        Settings::load(&self.dirs.settings_file())
    }

    pub fn set_settings(&self, s: Settings) -> Result<Settings> {
        let s = s.normalized();
        s.validate()?;
        // The game version drives install, play, mods and the server package: not while one of
        // those is busy.
        let now = VersionFile::resolve(s.game_version.as_deref())?.version;
        if now != self.version().version {
            self.ensure_idle("switch the game version")?;
        }
        s.save(&self.dirs.settings_file())?;
        Ok(s)
    }

    /// The game version the launcher works with: the one picked in the settings, else the newest
    /// it supports. An unreadable settings file or a version this build dropped falls back to the
    /// newest (the settings page shows the error itself).
    pub fn version(&self) -> VersionFile {
        let picked = self.settings().ok().and_then(|s| s.game_version);
        VersionFile::resolve(picked.as_deref())
            .or_else(|_| VersionFile::latest())
            .expect("this build embeds at least one version file")
    }

    /// Every selectable target for the version dropdown: "Official (read-only)" first, then the
    /// supported private-server versions, newest first.
    pub fn game_versions(&self) -> Result<Vec<GameVersionView>> {
        let settings = self.settings()?;
        let current = self.version().version;
        let known = VersionFile::known_versions();
        let newest = known.last().copied().unwrap_or_default();
        let view = |version: &str, file: &VersionFile, official: bool| {
            let dir = settings.install_dir_or_default(&self.dirs, version);
            GameVersionView {
                version: version.to_owned(),
                app_id: file.app_id,
                official,
                selected: version == current,
                newest: !official && version == newest,
                installed: verify::looks_installed(&dir, file),
            }
        };
        let mut out = vec![view(OFFICIAL, &VersionFile::official()?, true)];
        for v in known.iter().rev() {
            out.push(view(v, &VersionFile::embedded(v)?, false));
        }
        Ok(out)
    }

    /// Picks the game version (persisted in the settings). Refused while an install, the game or
    /// the hosted server is running.
    pub fn set_game_version(&self, version: &str) -> Result<Vec<GameVersionView>> {
        anyhow::ensure!(
            VersionFile::is_selectable(version),
            "unsupported game version {version} (supported: official, {})",
            VersionFile::known_versions().join(", ")
        );
        let mut s = self.settings()?;
        s.game_version = Some(version.to_owned());
        self.set_settings(s)?;
        self.game_versions()
    }

    /// Errors when an install, the game or the hosted server is running.
    fn ensure_idle(&self, what: &str) -> Result<()> {
        if self.install_running() {
            bail!("cannot {what} while the game is being downloaded");
        }
        if matches!(
            self.play_state(),
            PlayState::Preparing { .. } | PlayState::Running { .. }
        ) {
            bail!("cannot {what} while the game is starting or running");
        }
        if !self.running_hosts().is_empty() {
            bail!("cannot {what} while a hosted server is running");
        }
        Ok(())
    }

    pub fn install_dir(&self) -> Result<PathBuf> {
        let dir = self
            .settings()?
            .install_dir_or_default(&self.dirs, &self.version().version);
        reject_steam_paths(&dir, "install directory")?;
        Ok(dir)
    }

    // --- profiles and accounts ----------------------------------------------------------

    pub(crate) fn profile_store(&self) -> ProfileStore {
        ProfileStore::new(self.dirs.profiles_file())
    }

    fn view(&self, p: Profile) -> ProfileView {
        let (session, err) = match self.tokens.get(&p.id) {
            Ok(s) => (s, None),
            Err(e) => (None, Some(format!("{e:#}"))),
        };
        let live = session.as_ref().filter(|s| s.expires_at > now_secs());
        ProfileView {
            logged_in: live.is_some(),
            account: session.as_ref().map(|s| s.account.clone()),
            expires_at: session.as_ref().map(|s| s.expires_at),
            role: live.and_then(|s| s.role.clone()),
            plain_http: plain_http(&p.url).unwrap_or(PlainHttp::Https),
            plain_http_confirmed: p.plain_http_ok,
            keyring_error: err,
            id: p.id,
            name: p.name,
            url: p.url,
        }
    }

    pub fn profiles(&self) -> Result<Vec<ProfileView>> {
        Ok(self
            .profile_store()
            .list()?
            .into_iter()
            .map(|p| self.view(p))
            .collect())
    }

    /// `confirm_plain_http`: the user saw the warning for a plain `http://` address and agreed.
    pub fn add_profile(
        &self,
        name: &str,
        url: &str,
        confirm_plain_http: bool,
    ) -> Result<ProfileView> {
        let mut p = self.profile_store().add(name, url)?;
        if confirm_plain_http && needs_plain_http_confirmation(&p.url) {
            p = self.profile_store().set_plain_http_ok(&p.id, true)?;
        }
        let mut s = self.settings()?;
        if s.selected_profile.is_none() {
            s.selected_profile = Some(p.id.clone());
            s.save(&self.dirs.settings_file())?;
        }
        Ok(self.view(p))
    }

    pub fn update_profile(
        &self,
        id: &str,
        name: &str,
        url: &str,
        confirm_plain_http: bool,
    ) -> Result<ProfileView> {
        let before = self.profile_store().get(id)?;
        let mut p = self.profile_store().update(id, name, url)?;
        if before.url != p.url {
            // The token belongs to the old server: do not send it to another one.
            self.tokens.delete(id)?;
        }
        if confirm_plain_http && needs_plain_http_confirmation(&p.url) && !p.plain_http_ok {
            p = self.profile_store().set_plain_http_ok(id, true)?;
        }
        Ok(self.view(p))
    }

    /// Records that the user agreed to send the password and token over plain `http://` to
    /// this profile's address (or withdraws that).
    pub fn set_plain_http_confirmed(&self, id: &str, ok: bool) -> Result<ProfileView> {
        Ok(self.view(self.profile_store().set_plain_http_ok(id, ok)?))
    }

    /// Revokes the token on the server (best effort) and forgets profile and token.
    pub async fn remove_profile(&self, id: &str) -> Result<()> {
        let _ = self.logout(id).await;
        self.profile_store().remove(id, self.tokens.as_ref())?;
        let mut s = self.settings()?;
        if s.selected_profile.as_deref() == Some(id) {
            s.selected_profile = self.profile_store().list()?.first().map(|p| p.id.clone());
            s.save(&self.dirs.settings_file())?;
        }
        Ok(())
    }

    pub(crate) fn client_for(&self, id: &str) -> Result<(Profile, ApiClient)> {
        let p = self.profile_store().get(id)?;
        let c = ApiClient::new(&p.url);
        Ok((p, c))
    }

    pub async fn server_info(&self, id: &str) -> Result<ServerInfo> {
        let (_, c) = self.client_for(id)?;
        Ok(c.server_info().await?)
    }

    pub async fn login(&self, id: &str, username: &str, password: &str) -> Result<ProfileView> {
        let (p, c) = self.client_for(id)?;
        require_secure(&p, "password")?;
        let r = c
            .login(username.trim(), password)
            .await
            .map_err(login_error)?;
        // Store before answering: a token that is not stored is useless, so revoke it again.
        let stored = StoredSession {
            token: r.token.clone(),
            account: r.account.username.clone(),
            expires_at: r.expires_at,
            role: Some(r.account.role.clone()),
        };
        if let Err(e) = self.tokens.set(id, &stored) {
            let _ = c.logout(&r.token).await;
            return Err(e);
        }
        Ok(self.view(p))
    }

    pub async fn register(&self, id: &str, username: &str, password: &str) -> Result<AccountInfo> {
        let (p, c) = self.client_for(id)?;
        require_secure(&p, "password")?;
        Ok(c.register(username.trim(), password).await?)
    }

    /// Asks the server who the stored token belongs to (`GET /me`) and keeps the role, so a
    /// promotion or demotion shows without logging in again. A token the server no longer
    /// accepts is forgotten, like in Play.
    pub async fn refresh_account(&self, id: &str) -> Result<ProfileView> {
        let (p, c) = self.client_for(id)?;
        let Some(mut session) = self.tokens.get(id)?.filter(|s| s.expires_at > now_secs()) else {
            return Ok(self.view(p));
        };
        require_secure(&p, "login token")?;
        match c.me(&session.token).await {
            Ok(me) => {
                session.role = Some(me.role);
                session.account = me.username;
                self.tokens.set(id, &session)?;
            }
            Err(e) if e.is_unauthorized() => self.tokens.delete(id)?,
            Err(e) => return Err(e.into()),
        }
        Ok(self.view(p))
    }

    /// The link that opens the server's web admin already signed in: checks the account is an
    /// admin, trades the launcher token for a single-use web token and returns
    /// `<server address>/#/token-login?t=<token>`. The result is a credential: do not log it.
    pub async fn web_admin_link(&self, id: &str) -> Result<String> {
        let (p, c) = self.client_for(id)?;
        let mut session = self
            .tokens
            .get(id)?
            .filter(|s| s.expires_at > now_secs())
            .ok_or_else(|| anyhow!("log in to {} first (Account page)", p.name))?;
        require_secure(&p, "login token")?;
        let me = match c.me(&session.token).await {
            Ok(me) => me,
            Err(e) if e.is_unauthorized() => {
                self.tokens.delete(id)?;
                bail!("the server no longer accepts this login; log in again");
            }
            Err(e) => return Err(e.into()),
        };
        if session.role.as_deref() != Some(me.role.as_str()) {
            session.role = Some(me.role.clone());
            self.tokens.set(id, &session)?;
        }
        if me.role != "admin" {
            bail!("{} is not an admin on {}", me.username, p.name);
        }
        let w = c.web_token(&session.token).await.map_err(|e| match e {
            ApiError::Status { status: 403, .. } => anyhow!(
                "the server refused to open the web admin for this login; log out and in again"
            ),
            ApiError::Status {
                status: 404 | 405, ..
            } => anyhow!(
                "this server is too old for \"Open web admin\" (it has no sign-in link yet); \
                 open its address in the browser and log in there"
            ),
            other => other.into(),
        })?;
        web_admin_url(&p.url, &w.login_path, &w.token)
    }

    /// [`Self::web_admin_link`] and then the system browser.
    pub async fn web_admin_open(&self, id: &str, opener: &dyn UrlOpener) -> Result<()> {
        let url = self.web_admin_link(id).await?;
        opener
            .open(&url)
            .map_err(|e| anyhow!("could not open the browser: {e:#}"))
    }

    /// The server's own page for new accounts (`<address>/#/register` of its web admin), opened
    /// in the system browser. Accounts are made there, not in the launcher.
    pub fn register_page_open(&self, id: &str, opener: &dyn UrlOpener) -> Result<()> {
        let p = self.profile_store().get(id)?;
        opener
            .open(&format!("{}/#/register", p.url.trim_end_matches('/')))
            .map_err(|e| anyhow!("could not open the browser: {e:#}"))
    }

    /// "Open web admin" of a server hosted here: signed in when its Servers entry is logged in
    /// as an admin, else the plain address (the web admin shows its login, or on a new server
    /// the page that creates the first admin).
    pub async fn host_web_admin_open(&self, id: &str, opener: &dyn UrlOpener) -> Result<()> {
        let s = self.host_settings(id)?;
        if let Some(pid) = &s.profile_id
            && let Some(p) = self.profiles()?.into_iter().find(|p| &p.id == pid)
            && p.logged_in
            && p.role.as_deref() == Some("admin")
        {
            return self.web_admin_open(pid, opener).await;
        }
        opener
            .open(&format!("{}/", crate::host::local_web_url(&s)))
            .map_err(|e| anyhow!("could not open the browser: {e:#}"))
    }

    /// Revokes the launcher token on the server, then removes it from the keyring. The local
    /// copy is removed even when the server cannot be reached (and the result says so).
    pub async fn logout(&self, id: &str) -> Result<LogoutResult> {
        let (_, c) = self.client_for(id)?;
        let Some(session) = self.tokens.get(id)? else {
            return Ok(LogoutResult {
                revoked: false,
                message: "not logged in".into(),
            });
        };
        let result = match c.logout(&session.token).await {
            Ok(()) => LogoutResult {
                revoked: true,
                message: "logged out; the token was revoked".into(),
            },
            Err(e) if e.is_unauthorized() => LogoutResult {
                revoked: true,
                message: "logged out (the token was already invalid)".into(),
            },
            Err(e) => LogoutResult {
                revoked: false,
                message: format!(
                    "logged out locally, but the server could not revoke the token: {e}"
                ),
            },
        };
        self.tokens.delete(id)?;
        Ok(result)
    }

    // --- install -----------------------------------------------------------------------

    pub fn install_running(&self) -> bool {
        self.install.lock().unwrap().is_some()
    }

    pub fn install_status(&self) -> Result<InstallStatus> {
        let dir = self.install_dir()?;
        let pin = ToolPin::embedded()?;
        let tool_ready = pin.host_asset().is_ok_and(|a| {
            let d = pin.install_dir(&self.dirs.tools_dir(), a);
            d.join(&a.exe).is_file()
        });
        let version = self.version();
        let weights = space::depot_weights(&version);
        let paused = self
            .paused
            .lock()
            .unwrap()
            .clone()
            .filter(|p| p.version == version.version && !self.install_running());
        Ok(InstallStatus {
            required_bytes: space::estimated_install_bytes(&version),
            paused,
            installed: verify::looks_installed(&dir, &version),
            version: version.version.clone(),
            app_id: version.app_id,
            install_dir: dir,
            running: self.install_running(),
            tool_version: pin.version,
            tool_ready,
            depots: version
                .depots
                .iter()
                .zip(weights)
                .map(|(d, est_bytes)| DepotView {
                    id: d.id,
                    manifest: d.manifest.clone(),
                    role: d.role.clone(),
                    est_bytes,
                })
                .collect(),
        })
    }

    pub async fn verify_install(&self) -> Result<VerifyReport> {
        let dir = self.install_dir()?;
        let version = self.version();
        let report = tokio::task::spawn_blocking(move || verify::verify(&dir, &version)).await?;
        self.emit("install:verify", serde_json::to_value(&report)?);
        Ok(report)
    }

    /// Starts the install in the background. Progress arrives as events: `install:state`,
    /// `install:log`, `install:progress`, `install:prompt`, `install:verify`.
    pub fn install_start(self: &Arc<Self>, req: InstallRequest) -> Result<()> {
        if req.steam_username.trim().is_empty() {
            bail!("enter your Steam account name");
        }
        let dir = self.install_dir()?;
        let settings = self.settings()?;
        let (itx, irx) = mpsc::unbounded_channel();
        let (ctx, crx) = watch::channel(false);
        let pause = Arc::new(std::sync::atomic::AtomicBool::new(false));
        {
            let mut slot = self.install.lock().unwrap();
            if slot.is_some() {
                bail!("an install is already running");
            }
            *slot = Some(InstallControl {
                input: itx,
                cancel: ctx,
                pause: pause.clone(),
            });
        }
        *self.paused.lock().unwrap() = None;
        let core = self.clone();
        tokio::spawn(async move {
            let result = core.run_install(req, dir, settings, irx, crx, pause).await;
            *core.install.lock().unwrap() = None;
            match result {
                Ok(()) => {}
                Err(e) => {
                    core.emit(
                        "install:state",
                        json!({"state": "failed", "message": format!("{e:#}")}),
                    );
                }
            }
        });
        Ok(())
    }

    /// Pauses the download: DepotDownloader is stopped (what is on disk stays) and the state
    /// becomes `paused`. [`Self::install_resume`] starts it again with the same arguments.
    pub fn install_pause(&self) -> Result<()> {
        let slot = self.install.lock().unwrap();
        let c = slot
            .as_ref()
            .ok_or_else(|| anyhow!("no install is running"))?;
        c.pause.store(true, std::sync::atomic::Ordering::SeqCst);
        let _ = c.cancel.send(true);
        Ok(())
    }

    /// Continues a paused download: the same run again, with `-validate`, so DepotDownloader
    /// checks the files that are on disk and fetches only what is missing or incomplete. The
    /// password is asked again unless the Steam login is remembered.
    pub fn install_resume(self: &Arc<Self>, steam_password: String) -> Result<()> {
        let p = self
            .paused
            .lock()
            .unwrap()
            .clone()
            .filter(|p| p.version == self.version().version)
            .ok_or_else(|| anyhow!("there is no paused download to resume"))?;
        self.install_start(InstallRequest {
            steam_username: p.steam_username,
            steam_password,
            remember_login: p.remember_login,
            validate: true,
        })
    }

    /// The disk-space check for installing the selected version into its folder.
    pub async fn install_space(&self) -> Result<space::SpaceCheck> {
        let dir = self.install_dir()?;
        let version = self.version();
        Ok(tokio::task::spawn_blocking(move || space::check(&dir, &version)).await?)
    }

    async fn run_install(
        self: &Arc<Self>,
        mut req: InstallRequest,
        dir: PathBuf,
        settings: Settings,
        input: mpsc::UnboundedReceiver<String>,
        cancel: watch::Receiver<bool>,
        pause: Arc<std::sync::atomic::AtomicBool>,
    ) -> Result<()> {
        // A cancel is either the Cancel button or the Pause button.
        let stopped = |core: &Arc<Self>, req: &InstallRequest| {
            if pause.load(std::sync::atomic::Ordering::SeqCst) {
                *core.paused.lock().unwrap() = Some(PausedInstall {
                    version: core.version().version,
                    steam_username: req.steam_username.trim().to_owned(),
                    remember_login: req.remember_login,
                });
                core.emit("install:state", json!({"state": "paused"}));
            } else {
                core.emit("install:state", json!({"state": "cancelled"}));
            }
        };
        self.emit("install:state", json!({"state": "fetching_tool"}));
        let pin = ToolPin::embedded()?;
        let events = self.events.clone();
        let exe = depot::fetch_tool(&pin, &self.dirs.tools_dir(), &move |done, total| {
            events.emit(
                "install:progress",
                json!({"phase": "tool", "done": done, "total": total}),
            );
        })
        .await?;
        if *cancel.borrow() {
            stopped(self, &req);
            return Ok(());
        }

        tokio::fs::create_dir_all(&dir).await?;
        let space = {
            let (d, v) = (dir.clone(), self.version());
            tokio::task::spawn_blocking(move || space::check(&d, &v)).await?
        };
        if !space.ok {
            bail!("{}", space.message);
        }
        let job = DownloadJob {
            version: self.version(),
            dir: dir.clone(),
            username: req.steam_username.trim().to_owned(),
            remember_password: req.remember_login,
            prefer_code_entry: settings.prefer_code_entry,
            validate: req.validate,
        };
        let args = depot::depot_args(&job);
        let password = (!req.steam_password.is_empty())
            .then(|| Secret::new(std::mem::take(&mut req.steam_password)));
        self.emit("install:state", json!({"state": "downloading"}));

        let (etx, mut erx) = mpsc::unbounded_channel();
        let pump = {
            let core = self.clone();
            tokio::spawn(async move {
                while let Some(ev) = erx.recv().await {
                    match ev {
                        DepotEvent::Line { text, stream } => {
                            core.emit("install:log", json!({"text": text, "stream": stream}))
                        }
                        DepotEvent::Progress { depot, percent } => core.emit(
                            "install:progress",
                            json!({"phase": "depot", "depot": depot, "percent": percent}),
                        ),
                        DepotEvent::Prompt(kind) => {
                            core.emit("install:prompt", json!({"kind": kind}))
                        }
                    }
                }
            })
        };
        let outcome = depot::run(
            &exe,
            &args,
            password,
            etx,
            input,
            cancel,
            Some(&self.dirs.steam_login_home()),
        )
        .await;
        let _ = pump.await;
        match outcome? {
            Outcome::Cancelled => {
                stopped(self, &req);
                Ok(())
            }
            Outcome::Failed { reason, .. } => bail!("{reason}"),
            Outcome::Success => {
                self.emit("install:state", json!({"state": "verifying"}));
                let report = self.verify_install().await?;
                if report.ok {
                    self.emit("install:state", json!({"state": "done"}));
                    Ok(())
                } else {
                    let failed: Vec<String> = report
                        .checks
                        .iter()
                        .filter(|c| c.status == verify::Status::Fail)
                        .map(|c| format!("{}: {}", c.name, c.detail))
                        .collect();
                    bail!(
                        "the download finished but verification failed: {}",
                        failed.join("; ")
                    )
                }
            }
        }
    }

    /// Answers a Steam Guard prompt.
    pub fn install_input(&self, text: &str) -> Result<()> {
        let slot = self.install.lock().unwrap();
        let c = slot
            .as_ref()
            .ok_or_else(|| anyhow!("no install is running"))?;
        c.input
            .send(text.to_owned())
            .map_err(|_| anyhow!("the download has ended"))
    }

    pub fn install_cancel(&self) -> Result<()> {
        let slot = self.install.lock().unwrap();
        let Some(c) = slot.as_ref() else {
            // Not running: cancelling a paused download forgets it (the files stay).
            drop(slot);
            if self.paused.lock().unwrap().take().is_some() {
                self.emit("install:state", json!({"state": "cancelled"}));
                return Ok(());
            }
            bail!("no install is running");
        };
        // A Cancel after a Pause request wins.
        c.pause.store(false, std::sync::atomic::Ordering::SeqCst);
        let _ = c.cancel.send(true);
        Ok(())
    }

    // --- play --------------------------------------------------------------------------

    pub fn play_state(&self) -> PlayState {
        self.play.lock().unwrap().clone()
    }

    fn set_play(&self, s: PlayState) {
        *self.play.lock().unwrap() = s.clone();
        self.emit("play:status", serde_json::to_value(&s).unwrap_or_default());
    }

    fn play_config(&self, s: &Settings) -> Result<PlayConfig> {
        Ok(PlayConfig {
            install_dir: self.install_dir()?,
            dll_dir: self.hook_dll_dir(s).0,
            steam_root: if cfg!(windows) {
                PathBuf::new()
            } else {
                s.steam_root_or_default()?
            },
            proton: s.proton.clone(),
            prefix_dir: if self.version().is_official() {
                self.dirs.prefix_dir_official()
            } else {
                self.dirs.prefix_dir()
            },
            crash_dump_dir: self.dirs.crash_dump_dir(),
            logs_dir: self.dirs.logs_dir(),
        })
    }

    /// Marks the game as starting; refuses a second start.
    fn begin_play(&self) -> Result<()> {
        let mut st = self.play.lock().unwrap();
        if matches!(*st, PlayState::Preparing { .. } | PlayState::Running { .. }) {
            bail!("the game is already starting or running");
        }
        *st = PlayState::Preparing {
            step: "checking".into(),
        };
        Ok(())
    }

    /// Prepares and starts the game for a profile. Returns once the client process is running;
    /// the exit is reported as a `play:status` event.
    pub async fn play(self: &Arc<Self>, profile_id: &str) -> Result<()> {
        if self.version().is_official() {
            bail!("Play on the official servers does not use a server profile (play_official)");
        }
        self.begin_play()?;
        match self.play_inner(profile_id).await {
            Ok(()) => Ok(()),
            Err(e) => {
                self.set_play(PlayState::Idle);
                Err(e)
            }
        }
    }

    /// Official (read-only) mode: deploys the hook in `mode = "official"` and starts the game
    /// with the original gateway list and the game's own Steam login. The launcher logs in
    /// nowhere, writes no token and no private gateway or trusted server.
    pub async fn play_official(self: &Arc<Self>) -> Result<()> {
        if !self.version().is_official() {
            bail!("the official mode is not selected");
        }
        if !self.settings()?.official_ack {
            bail!("read and accept the official-mode warning first");
        }
        self.begin_play()?;
        match self.play_official_inner().await {
            Ok(()) => Ok(()),
            Err(e) => {
                self.set_play(PlayState::Idle);
                Err(e)
            }
        }
    }

    async fn play_official_inner(self: &Arc<Self>) -> Result<()> {
        let step = |s: &str| self.set_play(PlayState::Preparing { step: s.into() });
        let settings = self.settings()?;
        let cfg = self.play_config(&settings)?;
        if !verify::looks_installed(&cfg.install_dir, &self.version()) {
            bail!(
                "the game is not installed in {}: install it first (Install page)",
                cfg.install_dir.display()
            );
        }
        self.check_hook(&settings).await?;
        step("preparing the Proton prefix");
        play::ensure_prefix(&cfg).await?;
        step("deploying the read-only hook");
        let opts = play::official_deploy_options(&cfg);
        let report = tokio::task::spawn_blocking(move || poe_hook_deploy::deploy(&opts)).await??;
        for w in &report.warnings {
            self.emit("play:warning", json!({"text": w}));
        }
        self.launch_game(&cfg).await
    }

    async fn play_inner(self: &Arc<Self>, profile_id: &str) -> Result<()> {
        let step = |s: &str| self.set_play(PlayState::Preparing { step: s.into() });
        let settings = self.settings()?;
        let cfg = self.play_config(&settings)?;
        let version = self.version();
        if !verify::looks_installed(&cfg.install_dir, &version) {
            bail!(
                "the game is not installed in {}: install it first",
                cfg.install_dir.display()
            );
        }
        let (profile, client) = self.client_for(profile_id)?;
        require_secure(&profile, "login token")?;
        let session = self
            .tokens
            .get(profile_id)?
            .filter(|s| s.expires_at > now_secs())
            .ok_or_else(|| anyhow!("log in to {} first (Account page)", profile.name))?;

        step("asking the server");
        let info = client.server_info().await?;
        if info.version != version.version {
            self.emit(
                "play:warning",
                json!({"text": format!("the server speaks client {}, this install is {}", info.version, version.version)}),
            );
        }
        let host = play::client_host(&info, &profile.url);
        let options = play::deploy_options(&cfg, &info, &host, &profile.url)?;
        self.check_hook(&settings).await?;

        // The server's required mods (downloaded and verified per the Mod sync setting) and the
        // player's own for this server go into the game before the hook starts.
        step("preparing the mods");
        self.prepare_mods(&profile, &info, &settings).await?;

        step("preparing the Proton prefix");
        play::ensure_prefix(&cfg).await?;

        step("signing in");
        let game = match client.game_token(&session.token).await {
            Ok(g) => g,
            Err(e) if e.is_unauthorized() => {
                self.tokens.delete(profile_id)?;
                bail!("the server no longer accepts this login; log in again");
            }
            Err(e) => return Err(e.into()),
        };
        let token = poe_hook_deploy::login_token::parse_token_hex(&game.token)?;
        let docs = play::token_documents_dir(&cfg.prefix_dir)?;
        poe_hook_deploy::write_login_token(&docs, &game.account.username, token)?;

        step("deploying the hook");
        let opts = options.clone();
        let report = tokio::task::spawn_blocking(move || poe_hook_deploy::deploy(&opts)).await??;
        for w in &report.warnings {
            self.emit("play:warning", json!({"text": w}));
        }
        self.launch_game(&cfg).await
    }

    /// Starts the client and watches it: `play:status` goes to `running`, later `exited`.
    async fn launch_game(self: &Arc<Self>, cfg: &PlayConfig) -> Result<()> {
        self.set_play(PlayState::Preparing {
            step: "starting the client".into(),
        });
        std::fs::create_dir_all(&cfg.logs_dir)?;
        let log_path = cfg.logs_dir.join(format!("game-{}.log", now_secs()));
        let log = std::fs::File::create(&log_path).context("creating the game log")?;
        let mut cmd = play::launch_command(cfg)?;
        cmd.stdout(log.try_clone()?).stderr(log);
        let mut child = tokio::process::Command::from(cmd)
            .kill_on_drop(false)
            .spawn()
            .context("starting the game")?;
        self.set_play(PlayState::Running { pid: child.id() });
        let core = self.clone();
        tokio::spawn(async move {
            let code = child.wait().await.ok().and_then(|s| s.code());
            core.set_play(PlayState::Exited { code });
        });
        Ok(())
    }

    // --- the remembered Steam login ----------------------------------------------------

    /// Whether DepotDownloader remembers a Steam login (and for which account name).
    pub fn steam_login(&self) -> Result<steam::SteamLogin> {
        let s = self.settings()?;
        Ok(steam::status(&self.dirs, &s.steam_username))
    }

    /// "Log out of Steam": deletes the login token DepotDownloader remembers and clears the
    /// remembered account name and the "remember" switch in the settings. Only files of the
    /// launcher's own DepotDownloader are touched, never the Steam client.
    pub fn steam_logout(&self) -> Result<steam::SteamLogin> {
        if self.install_running() {
            bail!("cannot log out of Steam while the game is being downloaded");
        }
        steam::forget(&self.dirs)?;
        let mut s = self.settings()?;
        s.steam_username.clear();
        s.remember_steam_login = false;
        self.set_settings(s)?;
        self.steam_login()
    }
}

/// S-03: a password or token must not go over plain `http://` to another machine unless the
/// user agreed (the UI asks; this is the backstop). `what` names what would be sent.
pub fn require_secure(p: &Profile, what: &str) -> Result<()> {
    if needs_plain_http_confirmation(&p.url) && !p.plain_http_ok {
        bail!(
            "{} is reached over plain http://, so your {what} would travel unencrypted and \
             could be read on the way. Confirm that on the Servers page first, or use an https:// address",
            p.name
        );
    }
    Ok(())
}

/// `base` + `login_path`, after checking the path is the sign-in route and carries exactly
/// the token the server just issued (a hostile or broken server must not make the launcher
/// open some other address).
pub fn web_admin_url(base: &str, login_path: &str, token: &str) -> Result<String> {
    let expected = format!("/#/token-login?t={token}");
    if token.len() != 64 || !token.chars().all(|c| c.is_ascii_hexdigit()) || login_path != expected
    {
        bail!("the server's sign-in link is not in the expected form; it was not opened");
    }
    Ok(format!("{}{}", base.trim_end_matches('/'), login_path))
}

fn login_error(e: ApiError) -> anyhow::Error {
    match e {
        ApiError::Status { status: 401, .. } => anyhow!("wrong account name or password"),
        other => other.into(),
    }
}
