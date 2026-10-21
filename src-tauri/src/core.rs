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
    mods::{InstalledMod, InstalledNew, ModStore},
    paths::{Dirs, reject_steam_paths},
    play::{self, PlayConfig},
    profiles::{PlainHttp, Profile, ProfileStore, needs_plain_http_confirmation, plain_http},
    settings::Settings,
    verify::{self, VerifyReport},
    version::VersionFile,
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
}

#[derive(Debug, Clone, Serialize)]
pub struct DepotView {
    pub id: u32,
    pub manifest: String,
    pub role: String,
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
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum PlayState {
    Idle,
    Preparing { step: String },
    Running { pid: Option<u32> },
    Exited { code: Option<i32> },
}

pub struct Core {
    pub dirs: Dirs,
    pub tokens: Arc<dyn TokenStore>,
    pub events: Arc<dyn Events>,
    pub version: VersionFile,
    install: Mutex<Option<InstallControl>>,
    play: Mutex<PlayState>,
    /// The hosted `poe-server` process (see `host_ops.rs`).
    pub host: HostRuntime,
    /// Set while the host page downloads, installs or sets up: one such job at a time.
    pub(crate) host_busy: Mutex<bool>,
}

impl Core {
    pub fn new(
        dirs: Dirs,
        tokens: Arc<dyn TokenStore>,
        events: Arc<dyn Events>,
    ) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            dirs,
            tokens,
            events,
            version: VersionFile::latest()?,
            install: Mutex::new(None),
            play: Mutex::new(PlayState::Idle),
            host: HostRuntime::new(),
            host_busy: Mutex::new(false),
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
        s.validate()?;
        s.save(&self.dirs.settings_file())?;
        Ok(s)
    }

    pub fn install_dir(&self) -> Result<PathBuf> {
        let dir = self
            .settings()?
            .install_dir_or_default(&self.dirs, &self.version.version);
        reject_steam_paths(&dir, "install directory")?;
        Ok(dir)
    }

    // --- profiles and accounts ----------------------------------------------------------

    fn profile_store(&self) -> ProfileStore {
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

    fn client_for(&self, id: &str) -> Result<(Profile, ApiClient)> {
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
        Ok(InstallStatus {
            version: self.version.version.clone(),
            app_id: self.version.app_id,
            installed: verify::looks_installed(&dir, &self.version),
            install_dir: dir,
            running: self.install_running(),
            tool_version: pin.version,
            tool_ready,
            depots: self
                .version
                .depots
                .iter()
                .map(|d| DepotView {
                    id: d.id,
                    manifest: d.manifest.clone(),
                    role: d.role.clone(),
                })
                .collect(),
        })
    }

    pub async fn verify_install(&self) -> Result<VerifyReport> {
        let dir = self.install_dir()?;
        let version = self.version.clone();
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
        {
            let mut slot = self.install.lock().unwrap();
            if slot.is_some() {
                bail!("an install is already running");
            }
            *slot = Some(InstallControl {
                input: itx,
                cancel: ctx,
            });
        }
        let core = self.clone();
        tokio::spawn(async move {
            let result = core.run_install(req, dir, settings, irx, crx).await;
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

    async fn run_install(
        self: &Arc<Self>,
        req: InstallRequest,
        dir: PathBuf,
        settings: Settings,
        input: mpsc::UnboundedReceiver<String>,
        cancel: watch::Receiver<bool>,
    ) -> Result<()> {
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
            self.emit("install:state", json!({"state": "cancelled"}));
            return Ok(());
        }

        tokio::fs::create_dir_all(&dir).await?;
        self.check_free_space(&dir)?;
        let job = DownloadJob {
            version: self.version.clone(),
            dir: dir.clone(),
            username: req.steam_username.trim().to_owned(),
            remember_password: req.remember_login,
            prefer_code_entry: settings.prefer_code_entry,
            validate: req.validate,
        };
        let args = depot::depot_args(&job);
        let password = (!req.steam_password.is_empty()).then(|| Secret::new(req.steam_password));
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
        let outcome = depot::run(&exe, &args, password, etx, input, cancel).await;
        let _ = pump.await;
        match outcome? {
            Outcome::Cancelled => {
                self.emit("install:state", json!({"state": "cancelled"}));
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

    fn check_free_space(&self, dir: &std::path::Path) -> Result<()> {
        let needed = self.version.verify.min_bytes;
        if needed == 0 {
            return Ok(());
        }
        let have_already = verify::scan(dir).map(|(_, b)| b).unwrap_or(0);
        let missing = needed.saturating_sub(have_already);
        if let Ok(free) = fs4::available_space(dir)
            && free < missing
        {
            bail!(
                "not enough free space in {}: about {} are needed, {} are free",
                dir.display(),
                verify::human(missing),
                verify::human(free)
            );
        }
        Ok(())
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
        let c = slot
            .as_ref()
            .ok_or_else(|| anyhow!("no install is running"))?;
        let _ = c.cancel.send(true);
        Ok(())
    }

    // --- mods --------------------------------------------------------------------------

    fn mod_store(&self) -> Result<ModStore> {
        Ok(ModStore::new(&self.install_dir()?, &self.version.version))
    }

    pub fn mods_list(&self) -> Result<Vec<InstalledMod>> {
        self.mod_store()?.list()
    }

    pub fn mod_install(&self, path: &str, replace: bool) -> Result<InstalledNew> {
        let p = PathBuf::from(path);
        let store = self.mod_store()?;
        if p.is_dir() {
            store.install_dir(&p, replace)
        } else {
            store.install_zip(&p, replace)
        }
    }

    pub fn mod_enable(&self, id: &str, override_compat: bool) -> Result<Vec<String>> {
        self.mod_store()?.enable(id, override_compat)
    }

    pub fn mod_disable(&self, id: &str) -> Result<Vec<String>> {
        self.mod_store()?.disable(id)
    }

    pub fn mod_move(&self, id: &str, position: usize) -> Result<Vec<String>> {
        self.mod_store()?.move_to(id, position)
    }

    pub fn mod_auto_order(&self) -> Result<Vec<String>> {
        self.mod_store()?.auto_order()
    }

    pub fn mod_remove(&self, id: &str) -> Result<()> {
        self.mod_store()?.remove(id)
    }

    pub fn mods_dir(&self) -> Result<PathBuf> {
        Ok(self.mod_store()?.dir().to_path_buf())
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
            dll_dir: s.dll_dir_or_default(&self.dirs),
            steam_root: if cfg!(windows) {
                PathBuf::new()
            } else {
                s.steam_root_or_default()?
            },
            proton: s.proton.clone(),
            prefix_dir: self.dirs.prefix_dir(),
            crash_dump_dir: self.dirs.crash_dump_dir(),
            logs_dir: self.dirs.logs_dir(),
        })
    }

    /// Prepares and starts the game for a profile. Returns once the client process is running;
    /// the exit is reported as a `play:status` event.
    pub async fn play(self: &Arc<Self>, profile_id: &str) -> Result<()> {
        {
            let mut st = self.play.lock().unwrap();
            if matches!(*st, PlayState::Preparing { .. } | PlayState::Running { .. }) {
                bail!("the game is already starting or running");
            }
            *st = PlayState::Preparing {
                step: "checking".into(),
            };
        }
        match self.play_inner(profile_id).await {
            Ok(()) => Ok(()),
            Err(e) => {
                self.set_play(PlayState::Idle);
                Err(e)
            }
        }
    }

    async fn play_inner(self: &Arc<Self>, profile_id: &str) -> Result<()> {
        let step = |s: &str| self.set_play(PlayState::Preparing { step: s.into() });
        let settings = self.settings()?;
        let cfg = self.play_config(&settings)?;
        if !verify::looks_installed(&cfg.install_dir, &self.version) {
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
        if info.version != self.version.version {
            self.emit(
                "play:warning",
                json!({"text": format!("the server speaks client {}, this install is {}", info.version, self.version.version)}),
            );
        }
        let host = play::client_host(&info, &profile.url);
        let options = play::deploy_options(&cfg, &info, &host)?;
        play::check_dlls(&cfg.dll_dir)?;

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

        step("starting the client");
        std::fs::create_dir_all(&cfg.logs_dir)?;
        let log_path = cfg.logs_dir.join(format!("game-{}.log", now_secs()));
        let log = std::fs::File::create(&log_path).context("creating the game log")?;
        let mut cmd = play::launch_command(&cfg)?;
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
