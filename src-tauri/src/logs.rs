//! Log and crash viewer: the game logs the launcher writes (`<data>/logs/game-*.log`), the
//! hosted server's log, and the crash dumps the launcher sets aside before the game starts
//! (`<data>/crash-dumps`, so the client's helper never uploads them). Files are listed and read
//! by id (`kind:name`), never by path. "Copy for a bug report" builds a text with system
//! information and the end of one log, with the home folder, account names and secrets removed.

use std::{
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde::Serialize;

use crate::core::Core;

/// How many bytes from the end of a file are looked at.
pub const MAX_READ_BYTES: u64 = 2 << 20;
pub const DEFAULT_LINES: usize = 2000;
const MAX_FILES: usize = 40;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LogFile {
    /// `game:game-1761000000.log`, `server:server.log`, `crash:<name>`.
    pub id: String,
    pub kind: String,
    pub name: String,
    pub size: u64,
    /// Modification time, Unix seconds.
    pub modified: i64,
    /// Plain text that the viewer can show (a crash dump is binary).
    pub text: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct LogList {
    pub files: Vec<LogFile>,
    pub game_dir: PathBuf,
    pub server_dir: PathBuf,
    pub crash_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
pub struct LogContent {
    pub id: String,
    pub lines: Vec<String>,
    /// Earlier lines exist that are not shown.
    pub truncated: bool,
    pub size: u64,
    /// Not text: nothing is shown.
    pub binary: bool,
}

fn mtime(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs() as i64)
}

fn is_text_name(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    [".log", ".txt", ".json", ".xml", ".toml"]
        .iter()
        .any(|e| n.ends_with(e))
}

fn files_in(dir: &Path, kind: &str, keep: impl Fn(&str) -> bool) -> Vec<LogFile> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<LogFile> = rd
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let meta = e.metadata().ok()?;
            if !meta.is_file() || !keep(&name) || name.contains(['/', '\\']) {
                return None;
            }
            Some(LogFile {
                id: format!("{kind}:{name}"),
                kind: kind.to_owned(),
                text: kind != "crash" || is_text_name(&name),
                name,
                size: meta.len(),
                modified: mtime(&meta),
            })
        })
        .collect();
    out.sort_by(|a, b| (b.modified, &b.name).cmp(&(a.modified, &a.name)));
    out.truncate(MAX_FILES);
    out
}

/// The last `max_lines` lines of the file (looking at no more than the last
/// [`MAX_READ_BYTES`]). Invalid UTF-8 is replaced; a NUL byte in the first KB marks the file
/// as binary.
pub fn read_tail(path: &Path, max_lines: usize) -> Result<(Vec<String>, bool, u64, bool)> {
    let mut f = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let size = f.metadata()?.len();
    let start = size.saturating_sub(MAX_READ_BYTES);
    f.seek(SeekFrom::Start(start))?;
    let mut buf = Vec::new();
    f.take(MAX_READ_BYTES).read_to_end(&mut buf)?;
    if buf.iter().take(1024).any(|b| *b == 0) {
        return Ok((Vec::new(), false, size, true));
    }
    let text = String::from_utf8_lossy(&buf);
    let mut lines: Vec<&str> = text.lines().collect();
    // A cut in the middle of a line: drop the partial first line.
    let mut truncated = start > 0;
    if start > 0 && !lines.is_empty() {
        lines.remove(0);
    }
    if lines.len() > max_lines {
        lines.drain(..lines.len() - max_lines);
        truncated = true;
    }
    let lines = lines.into_iter().map(strip_terminal_codes).collect();
    Ok((lines, truncated, size, false))
}

/// Removes ANSI colour sequences (`ESC[...m`) that Proton and the server write.
pub fn strip_terminal_codes(line: &str) -> String {
    crate::host::strip_ansi(line)
}

// ---------------------------------------------------------------------------------------------
// Redaction

/// What a bug report must not contain.
#[derive(Debug, Clone, Default)]
pub struct Redactor {
    /// The user's home folder (`/home/alice`, `C:\Users\Alice`).
    pub home: Option<String>,
    /// Account names (server accounts, the Steam account, the host's admin), any case.
    pub names: Vec<String>,
}

const SECRET_KEYS: [&str; 10] = [
    "password",
    "passwd",
    "pwd",
    "token",
    "secret",
    "authorization",
    "api_key",
    "apikey",
    "remember_token",
    "login_token",
];

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Replaces `needle` (case-insensitive) as a whole word.
fn replace_word(text: &str, needle: &str, with: &str) -> String {
    if needle.chars().count() < 3 {
        return text.to_owned();
    }
    let lower = text.to_lowercase();
    let n = needle.to_lowercase();
    // `to_lowercase` can change byte lengths; fall back to a simple scan then.
    if lower.len() != text.len() {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while let Some(pos) = lower[i..].find(&n) {
        let s = i + pos;
        let e = s + n.len();
        let before = text[..s].chars().next_back();
        let after = text[e..].chars().next();
        out.push_str(&text[i..s]);
        if before.is_none_or(|c| !is_word(c)) && after.is_none_or(|c| !is_word(c)) {
            out.push_str(with);
        } else {
            out.push_str(&text[s..e]);
        }
        i = e;
    }
    out.push_str(&text[i..]);
    out
}

/// `key=value`, `key: value`, `"key":"value"` for the secret keys, and `Bearer value`: the
/// value is replaced.
fn redact_secrets(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        out.push_str(&redact_secret_line(line));
    }
    out
}

fn redact_secret_line(line: &str) -> String {
    let lower = line.to_ascii_lowercase();
    // Byte offsets match only when the line is ASCII-case stable; to_ascii_lowercase keeps them.
    let mut spans: Vec<(usize, usize)> = Vec::new();
    let b = line.as_bytes();
    let mut scan = |key: &str, need_sep: bool| {
        let mut from = 0;
        while let Some(p) = lower[from..].find(key) {
            let s = from + p;
            let mut e = s + key.len();
            from = e;
            // The key must not sit in a longer word on the left (`mytoken` is fine to skip).
            if s > 0 && (b[s - 1] as char).is_ascii_alphanumeric() {
                continue;
            }
            // Optional closing quote of a JSON key, spaces, then the separator.
            let mut j = e;
            while j < b.len() && matches!(b[j], b'"' | b'\'') {
                j += 1;
            }
            while j < b.len() && b[j] == b' ' {
                j += 1;
            }
            let sep = j < b.len() && matches!(b[j], b'=' | b':');
            if need_sep && !sep {
                continue;
            }
            if sep {
                j += 1;
            }
            while j < b.len() && matches!(b[j], b' ' | b'"' | b'\'') {
                j += 1;
            }
            e = j;
            let mut k = e;
            while k < b.len()
                && !matches!(
                    b[k],
                    b' ' | b'"' | b'\'' | b',' | b'&' | b'}' | b'\n' | b'\r' | b';'
                )
            {
                k += 1;
            }
            if k > e {
                spans.push((e, k));
                from = k;
            }
        }
    };
    for key in SECRET_KEYS {
        scan(key, true);
    }
    scan("bearer", false);
    scan("-password", false);
    if spans.is_empty() {
        return line.to_owned();
    }
    spans.sort_unstable();
    let mut out = String::with_capacity(line.len());
    let mut at = 0;
    for (s, e) in spans {
        if s < at {
            continue;
        }
        out.push_str(&line[at..s]);
        out.push_str("<redacted>");
        at = e;
    }
    out.push_str(&line[at..]);
    out
}

/// Hex runs of 32 or more characters (tokens, keys) are replaced; a hash that is labelled as
/// SHA-256 right before it stays, since a bug report can use it.
fn redact_hex(text: &str) -> String {
    let b = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    let mut last = 0;
    while i < b.len() {
        if b[i].is_ascii_hexdigit() && (i == 0 || !b[i - 1].is_ascii_alphanumeric()) {
            let mut j = i;
            while j < b.len() && b[j].is_ascii_hexdigit() {
                j += 1;
            }
            let bounded = j == b.len() || !b[j].is_ascii_alphanumeric();
            if bounded && j - i >= 32 {
                let from = i.saturating_sub(24);
                let before = text.get(from..i).unwrap_or("").to_ascii_lowercase();
                if !(before.contains("sha256") || before.contains("sha-256")) {
                    out.push_str(&text[last..i]);
                    out.push_str("<redacted-hex>");
                    last = j;
                }
            }
            i = j;
        } else {
            i += 1;
        }
    }
    out.push_str(&text[last..]);
    out
}

impl Redactor {
    pub fn apply(&self, text: &str) -> String {
        let mut t = text.to_owned();
        if let Some(home) = self.home.as_deref().filter(|h| h.len() > 3) {
            let fwd = home.replace('\\', "/");
            let back = home.replace('/', "\\");
            for variant in [home.to_owned(), fwd.clone(), back, fwd.to_lowercase()] {
                if !variant.is_empty() {
                    t = t.replace(&variant, "~");
                }
            }
            // The user name of the OS account, wherever it still shows.
            if let Some(user) = Path::new(&fwd).file_name().and_then(|n| n.to_str()) {
                t = replace_word(&t, user, "<user>");
            }
        }
        t = redact_secrets(&t);
        t = redact_hex(&t);
        for n in &self.names {
            t = replace_word(&t, n.trim(), "<account>");
        }
        t
    }
}

// ---------------------------------------------------------------------------------------------
// Core

impl Core {
    /// The folder of the hosted servers' folders (`<id>/server.log` each).
    fn server_log_dir(&self) -> PathBuf {
        self.dirs.data.join("servers")
    }

    pub fn log_list(&self) -> LogList {
        let mut files = files_in(&self.dirs.logs_dir(), "game", |n| {
            n.starts_with("game-") && n.ends_with(".log")
        });
        for sid in self.host_ids() {
            let server = self.host_dirs(&sid).log_file();
            if let Ok(meta) = std::fs::metadata(&server)
                && meta.is_file()
            {
                let name = format!("{sid}/server.log");
                files.push(LogFile {
                    id: format!("server:{name}"),
                    kind: "server".into(),
                    name,
                    size: meta.len(),
                    modified: mtime(&meta),
                    text: true,
                });
            }
        }
        files.extend(files_in(&self.dirs.crash_dump_dir(), "crash", |_| true));
        LogList {
            files,
            game_dir: self.dirs.logs_dir(),
            server_dir: self.server_log_dir(),
            crash_dir: self.dirs.crash_dump_dir(),
        }
    }

    /// `kind:name` to a path of a file we listed (so never a path from the UI).
    fn log_path(&self, id: &str) -> Result<(PathBuf, LogFile)> {
        let list = self.log_list();
        let Some(f) = list.files.into_iter().find(|f| f.id == id) else {
            bail!("that log file is no longer there");
        };
        let dir = match f.kind.as_str() {
            "game" => list.game_dir,
            "server" => list.server_dir,
            _ => list.crash_dir,
        };
        Ok((dir.join(&f.name), f))
    }

    pub fn log_read(&self, id: &str, lines: usize) -> Result<LogContent> {
        let (path, f) = self.log_path(id)?;
        if !f.text {
            return Ok(LogContent {
                id: f.id,
                lines: Vec::new(),
                truncated: false,
                size: f.size,
                binary: true,
            });
        }
        let (lines, truncated, size, binary) = read_tail(&path, lines.clamp(1, 20_000))?;
        Ok(LogContent {
            id: f.id,
            lines,
            truncated,
            size,
            binary,
        })
    }

    /// The folder of a kind (`game`, `server`, `crash`), created if missing.
    pub fn log_folder(&self, kind: &str) -> Result<PathBuf> {
        let dir = match kind {
            "game" => self.dirs.logs_dir(),
            "server" => self.server_log_dir(),
            "crash" => self.dirs.crash_dump_dir(),
            other => bail!("unknown log kind {other}"),
        };
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        Ok(dir)
    }

    fn redactor(&self) -> Redactor {
        let mut names: Vec<String> = Vec::new();
        if let Ok(profiles) = self.profiles() {
            names.extend(profiles.into_iter().filter_map(|p| p.account));
        }
        if let Ok(s) = self.settings() {
            names.push(s.steam_username);
        }
        names.retain(|n| n.trim().chars().count() >= 3);
        names.sort();
        names.dedup();
        Redactor {
            home: dirs::home_dir().map(|h| h.display().to_string()),
            names,
        }
    }

    /// System information that belongs in a bug report.
    fn report_header(&self) -> String {
        let v = self.version();
        let dir = self.install_dir().ok();
        let installed = dir
            .as_deref()
            .is_some_and(|d| crate::verify::looks_installed(d, &v));
        let hook = match self.hook_status() {
            Ok(h) => match (&h.source, &h.installed) {
                (crate::hook_ops::DllSource::Manual, _) => {
                    "own build (Hook DLL directory)".to_owned()
                }
                (_, Some(i)) => {
                    format!("{} (mod ABI {}, commit {})", i.version, i.mod_abi, i.commit)
                }
                _ => "none".to_owned(),
            },
            Err(_) => "unknown".to_owned(),
        };
        let server = self
            .host_status_blocking_info()
            .unwrap_or_else(|| "none".to_owned());
        format!(
            "Poe2 Launcher bug report\nlauncher: {}\nos: {} {}\ngame version: {} ({})\nclient hook: {}\nserver package: {}\n",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH,
            v.version,
            if installed {
                "installed"
            } else {
                "not installed"
            },
            hook,
            server,
        )
    }

    fn host_status_blocking_info(&self) -> Option<String> {
        let mut out = Vec::new();
        for sid in self.host_ids() {
            let Ok(s) = self.host_settings(&sid) else { continue };
            let Some(dir) = s.package_dir.as_ref() else { continue };
            if let Ok(p) =
                crate::host::inspect_package(dir, &self.version().version, std::env::consts::OS)
            {
                out.push(format!(
                    "{sid}: {} (server {}, game {})",
                    p.name, p.server_version, p.game_version
                ));
            }
        }
        (!out.is_empty()).then(|| out.join("; "))
    }

    /// The text for "Copy for a bug report": system information, then the last `lines` lines of
    /// the chosen log (`None`: no log), all redacted.
    pub fn log_report(&self, id: Option<&str>, lines: usize) -> Result<String> {
        let mut text = self.report_header();
        if let Some(id) = id {
            let c = self.log_read(id, lines.clamp(1, 2000))?;
            if c.binary {
                text.push_str(&format!("\n{id}: binary file, not included\n"));
            } else {
                text.push_str(&format!(
                    "\n--- {id}: last {} lines{} ---\n{}\n",
                    c.lines.len(),
                    if c.truncated {
                        " (earlier lines left out)"
                    } else {
                        ""
                    },
                    c.lines.join("\n")
                ));
            }
        }
        Ok(self.redactor().apply(&text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn red(home: &str, names: &[&str]) -> Redactor {
        Redactor {
            home: Some(home.into()),
            names: names.iter().map(|s| (*s).to_owned()).collect(),
        }
    }

    #[test]
    fn the_home_folder_and_user_name_are_removed() {
        let r = red("/home/alice", &[]);
        let out = r.apply("loading /home/alice/.local/share/poe2-launcher/game by alice (Alice)");
        assert_eq!(
            out,
            "loading ~/.local/share/poe2-launcher/game by <user> (<user>)"
        );
        let w = red(r"C:\Users\Bob", &[]);
        assert_eq!(
            w.apply(r"C:\Users\Bob\AppData and C:/Users/Bob/x"),
            r"~\AppData and ~/x"
        );
        // Short or empty homes are left alone (replacing "/" would wreck the text).
        assert_eq!(red("/", &[]).apply("/a/b"), "/a/b");
        assert_eq!(red("", &[]).apply("x"), "x");
    }

    #[test]
    fn account_names_are_replaced_as_whole_words() {
        let r = red("/home/x", &["Alice", "ab", "Bobby"]);
        let out = r.apply("login alice ok; ALICE2 stays; Bobby, bobby! alices");
        assert_eq!(
            out,
            "login <account> ok; ALICE2 stays; <account>, <account>! alices"
        );
        // A name shorter than 3 letters is not worth replacing and would mangle text.
        assert!(r.apply("about").contains("about"));
    }

    #[test]
    fn secrets_after_keys_are_removed() {
        let r = red("/home/x", &[]);
        for (input, expect) in [
            ("password=hunter2 next", "password=<redacted> next"),
            ("Password: hunter2", "Password: <redacted>"),
            (
                r#"{"token":"abc123def","x":1}"#,
                r#"{"token":"<redacted>","x":1}"#,
            ),
            (
                "Authorization: Bearer abcdef",
                "Authorization: <redacted> <redacted>",
            ),
            ("curl -password s3cret -x", "curl -password <redacted> -x"),
            (
                "GET /x?remember_token=zzz&b=1",
                "GET /x?remember_token=<redacted>&b=1",
            ),
            ("the token file was written", "the token file was written"),
            ("tokens: 3 active", "tokens: 3 active"),
        ] {
            // ("tokens" is a longer word than "token": no key match.)
            assert_eq!(r.apply(input), expect, "{input}");
        }
    }

    #[test]
    fn long_hex_is_removed_but_a_labelled_sha256_stays() {
        let r = red("/home/x", &[]);
        let tok = "ab12".repeat(16);
        assert_eq!(r.apply(&format!("t={tok} end")), "t=<redacted-hex> end");
        assert_eq!(
            r.apply(&format!("link /#/x?y={tok}.")),
            "link /#/x?y=<redacted-hex>."
        );
        let sha = "7157984534da1b146e9b0be342aa9fd24272915b6f478aadef9830fc7873f89f";
        assert_eq!(
            r.apply(&format!("exe sha256: {sha}")),
            format!("exe sha256: {sha}")
        );
        // A hex run inside a longer word is not a token.
        assert_eq!(r.apply(&format!("x{tok}")), format!("x{tok}"));
        // 31 characters is not a key.
        assert_eq!(r.apply(&"a".repeat(31)), "a".repeat(31));
    }

    #[test]
    fn tail_reads_the_end_and_flags_binary() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("a.log");
        let body: String = (0..100).map(|i| format!("line {i}\n")).collect();
        std::fs::write(&f, &body).unwrap();
        let (lines, truncated, size, binary) = read_tail(&f, 3).unwrap();
        assert_eq!(lines, ["line 97", "line 98", "line 99"]);
        assert!(truncated && !binary);
        assert_eq!(size, body.len() as u64);
        let (all, truncated, _, _) = read_tail(&f, 500).unwrap();
        assert_eq!(all.len(), 100);
        assert!(!truncated);
        let b = tmp.path().join("b.dmp");
        std::fs::write(&b, [b'M', b'D', 0, 0, 1, 2]).unwrap();
        assert!(read_tail(&b, 10).unwrap().3);
        // ANSI colours are stripped.
        std::fs::write(&f, "\x1b[31mred\x1b[0m\n").unwrap();
        assert_eq!(read_tail(&f, 5).unwrap().0, ["red"]);
    }

    #[test]
    fn a_huge_file_is_read_from_its_end_without_a_broken_first_line() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("big.log");
        let line = "x".repeat(99) + "\n";
        let n = (MAX_READ_BYTES as usize / 100) + 500;
        std::fs::write(&f, line.repeat(n)).unwrap();
        let (lines, truncated, size, _) = read_tail(&f, 1_000_000).unwrap();
        assert!(truncated);
        assert_eq!(size, (n * 100) as u64);
        assert!(
            lines.iter().all(|l| l.len() == 99),
            "a cut first line must be dropped"
        );
    }
}
