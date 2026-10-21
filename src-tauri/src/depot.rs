//! DepotDownloader: fetching the pinned release, building its command line, understanding its
//! output (progress and the prompts for the Steam password and Steam Guard code) and running it.
//!
//! The launcher uses the player's own Steam login. The password is typed into the launcher,
//! handed to DepotDownloader on its standard input when it asks for it (never on the command
//! line, where other processes could read it) and dropped afterwards. `-remember-password` is
//! passed only when the player asked for it.

use std::{
    path::{Path, PathBuf},
    process::Stdio,
};

use anyhow::{Context, Result, bail, ensure};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::Command,
    sync::{mpsc, watch},
};

use crate::version::VersionFile;

const PIN_FILE: &str = include_str!("../../versions/depotdownloader.toml");

// ---------------------------------------------------------------------------------------------
// The pinned release

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ToolPin {
    pub version: String,
    pub url_template: String,
    pub assets: Vec<ToolAsset>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ToolAsset {
    pub os: String,
    pub arch: String,
    pub asset: String,
    pub sha256: String,
    pub size: u64,
    /// File name of the executable inside the zip.
    pub exe: String,
}

impl ToolPin {
    pub fn embedded() -> Result<Self> {
        Self::parse(PIN_FILE)
    }

    pub fn parse(text: &str) -> Result<Self> {
        let pin: ToolPin = toml::from_str(text).context("depotdownloader.toml")?;
        ensure!(!pin.version.is_empty(), "tool version is empty");
        ensure!(
            pin.url_template.starts_with("https://"),
            "tool URL must be https"
        );
        for a in &pin.assets {
            ensure!(
                a.sha256.len() == 64 && a.sha256.chars().all(|c| c.is_ascii_hexdigit()),
                "{}: sha256 must be 64 hex digits",
                a.asset
            );
            ensure!(
                !a.asset.contains(['/', '\\']) && !a.exe.contains(['/', '\\']),
                "bad asset name"
            );
        }
        Ok(pin)
    }

    /// The asset for an OS and architecture (`std::env::consts` names).
    pub fn asset_for(&self, os: &str, arch: &str) -> Result<&ToolAsset> {
        self.assets
            .iter()
            .find(|a| a.os == os && a.arch == arch)
            .with_context(|| format!("no DepotDownloader build pinned for {os}/{arch}"))
    }

    pub fn host_asset(&self) -> Result<&ToolAsset> {
        self.asset_for(std::env::consts::OS, std::env::consts::ARCH)
    }

    pub fn url(&self, asset: &ToolAsset) -> String {
        self.url_template
            .replace("{version}", &self.version)
            .replace("{asset}", &asset.asset)
    }

    /// `<tools>/depotdownloader-<version>-<os>-<arch>`
    pub fn install_dir(&self, tools_dir: &Path, asset: &ToolAsset) -> PathBuf {
        tools_dir.join(format!(
            "depotdownloader-{}-{}-{}",
            self.version, asset.os, asset.arch
        ))
    }
}

/// Download progress: bytes so far and the total (0 = unknown).
pub type ProgressFn<'a> = &'a (dyn Fn(u64, u64) + Send + Sync);

/// Makes sure the pinned DepotDownloader is in `tools_dir`; returns the executable.
///
/// Downloads the zip over HTTPS, checks size and SHA-256 against the pin (a mismatch deletes
/// the download and fails), unpacks it next to the final directory and renames it into place.
/// An existing install is reused (its marker file records the checksum it was unpacked from).
pub async fn fetch_tool(
    pin: &ToolPin,
    tools_dir: &Path,
    progress: ProgressFn<'_>,
) -> Result<PathBuf> {
    let asset = pin.host_asset()?;
    let dir = pin.install_dir(tools_dir, asset);
    let exe = dir.join(&asset.exe);
    let marker = dir.join(".sha256");
    if exe.is_file() && std::fs::read_to_string(&marker).is_ok_and(|m| m.trim() == asset.sha256) {
        return Ok(exe);
    }
    tokio::fs::create_dir_all(tools_dir).await?;
    let zip_path = tools_dir.join(format!("{}.part", asset.asset));
    download_verified(
        &pin.url(asset),
        &zip_path,
        &asset.sha256,
        asset.size,
        progress,
    )
    .await?;

    let staging = tools_dir.join(format!(
        "{}.new",
        dir.file_name().unwrap().to_string_lossy()
    ));
    let (zip2, staging2, exe_name) = (zip_path.clone(), staging.clone(), asset.exe.clone());
    let sha = asset.sha256.clone();
    tokio::task::spawn_blocking(move || -> Result<()> {
        if staging2.exists() {
            std::fs::remove_dir_all(&staging2)?;
        }
        extract_zip(&zip2, &staging2)?;
        let exe = staging2.join(&exe_name);
        ensure!(exe.is_file(), "the archive does not contain {exe_name}");
        make_executable(&exe)?;
        std::fs::write(staging2.join(".sha256"), sha)?;
        Ok(())
    })
    .await??;
    let _ = tokio::fs::remove_file(&zip_path).await;
    if dir.exists() {
        tokio::fs::remove_dir_all(&dir).await?;
    }
    tokio::fs::rename(&staging, &dir).await?;
    Ok(exe)
}

async fn download_verified(
    url: &str,
    dest: &Path,
    sha256: &str,
    size: u64,
    progress: ProgressFn<'_>,
) -> Result<()> {
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(15))
        .user_agent(concat!("poe2-launcher/", env!("CARGO_PKG_VERSION")))
        .build()?;
    let res = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("downloading {url}"))?
        .error_for_status()
        .with_context(|| format!("downloading {url}"))?;
    let total = res.content_length().unwrap_or(size);
    let mut hasher = Sha256::new();
    let mut file = tokio::fs::File::create(dest).await?;
    let mut done = 0u64;
    let mut stream = res.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("download interrupted")?;
        hasher.update(&chunk);
        file.write_all(&chunk).await?;
        done += chunk.len() as u64;
        progress(done, total);
    }
    file.flush().await?;
    drop(file);
    let got = hex::encode(hasher.finalize());
    if got != sha256.to_ascii_lowercase() || (size != 0 && done != size) {
        let _ = tokio::fs::remove_file(dest).await;
        bail!(
            "the downloaded DepotDownloader does not match the pinned checksum (got {got}, {done} bytes); deleted"
        );
    }
    Ok(())
}

/// Unpacks a zip without letting an entry escape `dest` and without symlinks.
pub fn extract_zip(zip_path: &Path, dest: &Path) -> Result<()> {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(zip_path)?)?;
    std::fs::create_dir_all(dest)?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let Some(rel) = entry.enclosed_name() else {
            bail!("unsafe path in the archive: {}", entry.name());
        };
        if entry.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000) {
            bail!("the archive contains a symbolic link: {}", entry.name());
        }
        let out = dest.join(rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out)?;
        } else {
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut f = std::fs::File::create(&out)?;
            std::io::copy(&mut entry, &mut f)?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
    Ok(())
}
#[cfg(not(unix))]
fn make_executable(_: &Path) -> Result<()> {
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Command line

/// One DepotDownloader run: all depots of a version, in install order.
#[derive(Debug, Clone)]
pub struct DownloadJob {
    pub version: VersionFile,
    pub dir: PathBuf,
    pub username: String,
    /// `-remember-password`: DepotDownloader keeps a login token of its own (not the password).
    pub remember_password: bool,
    /// `-no-mobile`: ask for a typed Steam Guard code instead of a confirmation in the mobile app.
    pub prefer_code_entry: bool,
    /// `-validate`: re-check files that are already there (repair).
    pub validate: bool,
}

/// The arguments of the run. Never contains the password.
///
/// All depots are given to one process, in version-file order (DepotDownloader downloads them
/// one after the other in that order), so the Steam login and the Steam Guard code are asked
/// once and the exe depot lands over the content depot.
pub fn depot_args(job: &DownloadJob) -> Vec<String> {
    let mut a: Vec<String> = vec!["-app".into(), job.version.app_id.to_string()];
    for d in &job.version.depots {
        a.extend([
            "-depot".into(),
            d.id.to_string(),
            "-manifest".into(),
            d.manifest.clone(),
        ]);
    }
    a.extend(["-username".into(), job.username.clone()]);
    if job.remember_password {
        a.push("-remember-password".into());
    }
    if job.prefer_code_entry {
        a.push("-no-mobile".into());
    }
    a.extend(["-dir".into(), job.dir.display().to_string()]);
    if job.validate {
        a.push("-validate".into());
    }
    a
}

// ---------------------------------------------------------------------------------------------
// Understanding the output

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptKind {
    /// `Enter account password for "user":`
    Password,
    /// `STEAM GUARD! Please enter your 2-factor auth code from your authenticator app:`
    GuardDevice,
    /// `STEAM GUARD! Please enter the auth code sent to the email at ...:`
    GuardEmail,
}

/// A question DepotDownloader is waiting for, found in text that has no line end yet.
pub fn detect_prompt(pending: &str) -> Option<PromptKind> {
    let t = pending.trim_end();
    if !pending.ends_with(' ') && !pending.ends_with(':') {
        return None;
    }
    if t.contains("Enter account password for") && t.ends_with(':') {
        return Some(PromptKind::Password);
    }
    if t.contains("STEAM GUARD!") && t.ends_with(':') {
        if t.contains("sent to the email") || t.contains("email") {
            return Some(PromptKind::GuardEmail);
        }
        return Some(PromptKind::GuardDevice);
    }
    // Older wording, printed by the console login path.
    if t.contains("Please enter your 2 factor auth code") && t.ends_with(':') {
        return Some(PromptKind::GuardDevice);
    }
    None
}

/// Percent of a depot, from lines like ` 12.34% path/to/file`.
pub fn parse_progress(line: &str) -> Option<f32> {
    let (num, rest) = line.trim_start().split_once('%')?;
    if !rest.is_empty() && !rest.starts_with(' ') {
        return None;
    }
    let v: f32 = num.trim().parse().ok()?;
    (0.0..=100.0).contains(&v).then_some(v)
}

/// The depot named by `Downloading depot 2694491` (not the `... manifest` variant's file lines).
pub fn parse_depot_start(line: &str) -> Option<u32> {
    let rest = line.trim().strip_prefix("Downloading depot ")?;
    rest.split_whitespace().next()?.parse().ok()
}

/// Whether a line is DepotDownloader reporting a failure.
pub fn is_failure_line(line: &str) -> bool {
    let l = line.trim();
    l.starts_with("Failed to authenticate")
        || l.starts_with("Error:")
        || l.contains("is not available from this account")
        || l.starts_with("Download failed")
        || l.contains("was not completely downloaded")
        || l.contains("Access token was rejected")
        || l.contains("Could not connect to Steam")
}

/// Splits a byte stream into lines (`\n`, `\r\n` or a bare `\r`, which progress output uses) and
/// keeps the unfinished text for prompt detection.
#[derive(Debug, Default)]
pub struct LineSplitter {
    pending: String,
}

impl LineSplitter {
    /// Feeds text; returns the completed lines (without line ends, empty ones dropped).
    pub fn push(&mut self, text: &str) -> Vec<String> {
        let mut out = Vec::new();
        for ch in text.chars() {
            if ch == '\n' || ch == '\r' {
                if !self.pending.trim().is_empty() {
                    out.push(std::mem::take(&mut self.pending));
                } else {
                    self.pending.clear();
                }
            } else {
                self.pending.push(ch);
            }
        }
        out
    }

    /// Text received after the last line end.
    pub fn pending(&self) -> &str {
        &self.pending
    }

    pub fn clear_pending(&mut self) {
        self.pending.clear();
    }
}

// ---------------------------------------------------------------------------------------------
// Running it

/// What a run reports.
#[derive(Debug, Clone, PartialEq)]
pub enum DepotEvent {
    Line {
        text: String,
        stream: &'static str,
    },
    Progress {
        depot: Option<u32>,
        percent: f32,
    },
    /// DepotDownloader waits for input; answer with [`DepotInput`] unless it is the password,
    /// which the runner supplies itself.
    Prompt(PromptKind),
}

/// A password that is wiped when dropped.
pub struct Secret(String);

impl Secret {
    pub fn new(s: String) -> Self {
        Self(s)
    }
    fn expose(&self) -> &str {
        &self.0
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.0.zeroize();
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

/// How a run ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Success,
    Failed { code: Option<i32>, reason: String },
    Cancelled,
}

/// Runs DepotDownloader to the end.
///
/// `password` is written to its stdin when it asks for it. Other prompts are reported through
/// `events`; the answer (a Steam Guard code) arrives on `input`. `cancel` turning `true` kills
/// the process.
pub async fn run(
    exe: &Path,
    args: &[String],
    password: Option<Secret>,
    events: mpsc::UnboundedSender<DepotEvent>,
    mut input: mpsc::UnboundedReceiver<String>,
    mut cancel: watch::Receiver<bool>,
) -> Result<Outcome> {
    let mut cmd = Command::new(exe);
    cmd.args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        // Self-contained .NET would otherwise need the ICU system library.
        .env("DOTNET_SYSTEM_GLOBALIZATION_INVARIANT", "1")
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1");
    let mut child = cmd
        .spawn()
        .with_context(|| format!("starting {}", exe.display()))?;
    let mut stdin = child.stdin.take().context("no stdin")?;
    let mut stdout = child.stdout.take().context("no stdout")?;
    let mut stderr = child.stderr.take().context("no stderr")?;

    let mut out_split = LineSplitter::default();
    let mut err_split = LineSplitter::default();
    let (mut out_buf, mut err_buf) = ([0u8; 4096], [0u8; 4096]);
    let (mut out_open, mut err_open) = (true, true);
    let mut depot: Option<u32> = None;
    let mut failure: Option<String> = None;
    let mut password = password;
    let mut asked_password = false;

    let handle = |split: &mut LineSplitter,
                  stream: &'static str,
                  text: &str,
                  depot: &mut Option<u32>,
                  failure: &mut Option<String>|
     -> Option<PromptKind> {
        for line in split.push(text) {
            if let Some(d) = parse_depot_start(&line) {
                *depot = Some(d);
            }
            if let Some(p) = parse_progress(&line) {
                let _ = events.send(DepotEvent::Progress {
                    depot: *depot,
                    percent: p,
                });
            }
            if is_failure_line(&line) && failure.is_none() {
                *failure = Some(line.trim().to_owned());
            }
            let _ = events.send(DepotEvent::Line { text: line, stream });
        }
        detect_prompt(split.pending())
    };

    let status = loop {
        let prompt = tokio::select! {
            r = stdout.read(&mut out_buf), if out_open => match r {
                Ok(0) | Err(_) => { out_open = false; None }
                Ok(n) => {
                    let text = String::from_utf8_lossy(&out_buf[..n]).into_owned();
                    handle(&mut out_split, "stdout", &text, &mut depot, &mut failure).map(|k| (k, true))
                }
            },
            r = stderr.read(&mut err_buf), if err_open => match r {
                Ok(0) | Err(_) => { err_open = false; None }
                Ok(n) => {
                    let text = String::from_utf8_lossy(&err_buf[..n]).into_owned();
                    handle(&mut err_split, "stderr", &text, &mut depot, &mut failure).map(|k| (k, false))
                }
            },
            Some(code) = input.recv() => {
                // The answer to a Steam Guard prompt.
                let _ = stdin.write_all(format!("{}\n", code.trim()).as_bytes()).await;
                let _ = stdin.flush().await;
                None
            }
            _ = cancel.changed() => {
                if *cancel.borrow() {
                    let _ = child.kill().await;
                    return Ok(Outcome::Cancelled);
                }
                None
            }
            status = child.wait(), if !out_open && !err_open => break status?,
        };
        if let Some((kind, on_stdout)) = prompt {
            if kind == PromptKind::Password {
                match password.take() {
                    Some(p) if !asked_password => {
                        asked_password = true;
                        let _ = stdin
                            .write_all(format!("{}\n", p.expose()).as_bytes())
                            .await;
                        let _ = stdin.flush().await;
                    }
                    _ => {
                        // Asked again (wrong password) or never given one: stop instead of hanging.
                        let _ = child.kill().await;
                        return Ok(Outcome::Failed {
                            code: None,
                            reason: "Steam asked for the password again; check it and retry".into(),
                        });
                    }
                }
            } else {
                let _ = events.send(DepotEvent::Prompt(kind));
            }
            if on_stdout {
                out_split.clear_pending()
            } else {
                err_split.clear_pending()
            }
        }
    };
    Ok(if status.success() && failure.is_none() {
        Outcome::Success
    } else {
        Outcome::Failed {
            code: status.code(),
            reason: failure.unwrap_or_else(|| format!("DepotDownloader exited with {status}")),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pin_file_is_valid_and_covers_the_hosts() {
        let pin = ToolPin::embedded().unwrap();
        assert_eq!(pin.version, "3.4.0");
        for (os, arch) in [("linux", "x86_64"), ("windows", "x86_64")] {
            let a = pin.asset_for(os, arch).unwrap();
            assert_eq!(a.sha256.len(), 64);
            assert!(pin.url(a).starts_with(
                "https://github.com/SteamRE/DepotDownloader/releases/download/DepotDownloader_3.4.0/DepotDownloader-"
            ));
        }
        assert!(pin.asset_for("plan9", "x86_64").is_err());
        assert_eq!(
            pin.asset_for("windows", "x86_64").unwrap().exe,
            "DepotDownloader.exe"
        );
    }

    fn job(remember: bool, code: bool, validate: bool) -> DownloadJob {
        DownloadJob {
            version: VersionFile::embedded("0.5.5e").unwrap(),
            dir: PathBuf::from("/games/poe2"),
            username: "steamuser".into(),
            remember_password: remember,
            prefer_code_entry: code,
            validate,
        }
    }

    #[test]
    fn arguments_list_both_depots_in_order_and_never_the_password() {
        let args = depot_args(&job(false, false, false));
        assert_eq!(
            args,
            [
                "-app",
                "2694490",
                "-depot",
                "2694491",
                "-manifest",
                "7366357142270402355",
                "-depot",
                "2694492",
                "-manifest",
                "246779027001700092",
                "-username",
                "steamuser",
                "-dir",
                "/games/poe2"
            ]
        );
        for a in &args {
            assert!(!a.to_lowercase().contains("password"));
        }
        assert!(!args.contains(&"-password".to_owned()));
    }

    #[test]
    fn remember_password_is_opt_in() {
        assert!(!depot_args(&job(false, true, false)).contains(&"-remember-password".to_owned()));
        let on = depot_args(&job(true, true, true));
        assert!(on.contains(&"-remember-password".to_owned()));
        assert!(on.contains(&"-no-mobile".to_owned()));
        assert!(on.contains(&"-validate".to_owned()));
        let off = depot_args(&job(false, false, false));
        assert!(!off.contains(&"-no-mobile".to_owned()) && !off.contains(&"-validate".to_owned()));
    }

    #[test]
    fn prompts_are_recognised_before_the_line_ends() {
        assert_eq!(
            detect_prompt("Enter account password for \"alice\": "),
            Some(PromptKind::Password)
        );
        assert_eq!(
            detect_prompt(
                "STEAM GUARD! Please enter your 2-factor auth code from your authenticator app: "
            ),
            Some(PromptKind::GuardDevice)
        );
        assert_eq!(
            detect_prompt(
                "STEAM GUARD! Please enter the auth code sent to the email at a***@b.c: "
            ),
            Some(PromptKind::GuardEmail)
        );
        assert_eq!(
            detect_prompt("Please enter your 2 factor auth code from your authenticator app: "),
            Some(PromptKind::GuardDevice)
        );
        assert_eq!(detect_prompt("Logging 'alice' into Steam3..."), None);
        assert_eq!(detect_prompt("Enter account password for \"alice"), None);
        assert_eq!(detect_prompt(""), None);
    }

    #[test]
    fn progress_and_depot_lines() {
        assert_eq!(
            parse_progress(" 12.34% Bundles2/Tiny.bundle.bin"),
            Some(12.34)
        );
        assert_eq!(parse_progress("100.00% x"), Some(100.0));
        assert_eq!(parse_progress("Total downloaded: 5 bytes"), None);
        assert_eq!(parse_progress("abc% x"), None);
        assert_eq!(
            parse_depot_start("Downloading depot 2694491"),
            Some(2694491)
        );
        assert_eq!(
            parse_depot_start("Downloading depot 2694491 manifest"),
            Some(2694491)
        );
        assert_eq!(parse_depot_start("Downloading something"), None);
        assert!(is_failure_line(
            "Failed to authenticate with Steam: InvalidPassword"
        ));
        assert!(is_failure_line(
            "Depot 5 is not available from this account."
        ));
        assert!(!is_failure_line("Got depot key for 5 result: OK"));
    }

    #[test]
    fn line_splitter_handles_partial_chunks_and_carriage_returns() {
        let mut s = LineSplitter::default();
        assert!(s.push("Connecting to Steam3...").is_empty());
        assert_eq!(s.pending(), "Connecting to Steam3...");
        assert_eq!(
            s.push(" Done!\r\nLogging in\n"),
            ["Connecting to Steam3... Done!", "Logging in"]
        );
        assert_eq!(s.pending(), "");
        assert_eq!(s.push(" 1.00% a\r 2.00% b\r"), [" 1.00% a", " 2.00% b"]);
        s.push("Enter account password for \"x\": ");
        assert_eq!(detect_prompt(s.pending()), Some(PromptKind::Password));
        s.clear_pending();
        assert_eq!(s.pending(), "");
    }

    #[test]
    fn zip_extraction_refuses_escaping_paths() {
        use std::io::Write;
        let tmp = tempfile::tempdir().unwrap();
        let zip_path = tmp.path().join("a.zip");
        {
            let mut w = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
            let o = zip::write::SimpleFileOptions::default();
            w.start_file("ok/file.txt", o).unwrap();
            w.write_all(b"hi").unwrap();
            w.start_file("../evil.txt", o).unwrap();
            w.write_all(b"x").unwrap();
            w.finish().unwrap();
        }
        let dest = tmp.path().join("out");
        assert!(extract_zip(&zip_path, &dest).is_err());
        assert!(!tmp.path().join("evil.txt").exists());
    }

    #[test]
    fn secrets_do_not_print() {
        assert_eq!(
            format!("{:?}", Secret::new("hunter2".into())),
            "Secret(<redacted>)"
        );
    }

    /// A stand-in for DepotDownloader: a shell script that behaves like it (prompts without a
    /// line end, progress lines with a carriage return, exit code).
    #[cfg(unix)]
    #[tokio::test]
    async fn run_feeds_the_password_and_the_guard_code_and_reports_progress() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let script = tmp.path().join("fake-dd");
        std::fs::write(
            &script,
            r#"#!/bin/sh
printf 'Enter account password for "alice": '
read pw
if [ "$pw" != "s3cret" ]; then echo "bad password" >&2; exit 3; fi
printf '\n'
printf 'STEAM GUARD! Please enter your 2-factor auth code from your authenticator app: ' >&2
read code
echo "Downloading depot 2694491"
printf ' 50.00%% a/b\r'
printf '100.00%% a/c\n'
echo "got $code"
"#,
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let (etx, mut erx) = mpsc::unbounded_channel();
        let (itx, irx) = mpsc::unbounded_channel();
        let (_ctx, crx) = watch::channel(false);
        let task = tokio::spawn({
            let script = script.clone();
            async move {
                run(
                    &script,
                    &[],
                    Some(Secret::new("s3cret".into())),
                    etx,
                    irx,
                    crx,
                )
                .await
            }
        });
        let mut lines = Vec::new();
        let mut progress = Vec::new();
        while let Some(ev) = erx.recv().await {
            match ev {
                DepotEvent::Prompt(k) => {
                    assert_eq!(k, PromptKind::GuardDevice);
                    itx.send("12345".into()).unwrap();
                }
                DepotEvent::Progress { depot, percent } => progress.push((depot, percent)),
                DepotEvent::Line { text, .. } => lines.push(text),
            }
        }
        assert_eq!(task.await.unwrap().unwrap(), Outcome::Success);
        assert!(lines.iter().any(|l| l == "got 12345"), "{lines:?}");
        assert_eq!(progress, [(Some(2694491), 50.0), (Some(2694491), 100.0)]);
        // The password was never echoed into the log.
        assert!(!lines.iter().any(|l| l.contains("s3cret")));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_wrong_password_prompt_twice_fails_instead_of_hanging() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let script = tmp.path().join("fake-dd");
        std::fs::write(
            &script,
            "#!/bin/sh\nprintf 'Enter account password for \"a\": '\nread x\nprintf '\\nEnter account password for \"a\": '\nread y\n",
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let (etx, _erx) = mpsc::unbounded_channel();
        let (_itx, irx) = mpsc::unbounded_channel();
        let (_ctx, crx) = watch::channel(false);
        let out = run(&script, &[], Some(Secret::new("x".into())), etx, irx, crx)
            .await
            .unwrap();
        assert!(matches!(out, Outcome::Failed { .. }), "{out:?}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cancel_kills_the_process() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let script = tmp.path().join("fake-dd");
        std::fs::write(&script, "#!/bin/sh\nsleep 30\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let (etx, _erx) = mpsc::unbounded_channel();
        let (_itx, irx) = mpsc::unbounded_channel();
        let (ctx, crx) = watch::channel(false);
        let t = tokio::spawn(async move { run(&script, &[], None, etx, irx, crx).await });
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        ctx.send(true).unwrap();
        assert_eq!(t.await.unwrap().unwrap(), Outcome::Cancelled);
    }
}
