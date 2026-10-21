//! Running the hosted `poe-server`: start, stop, status and the streamed log tail.
//!
//! The server is started from its package folder (every path in its config is relative to the
//! working directory) with `POE_CONFIG` naming the config. Its standard output and error are
//! read line by line into a ring buffer, a log file and `host:log` events. Nothing secret goes
//! near the process: no password on the command line or in its environment.

use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::json;
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, BufReader},
    process::Command,
    sync::oneshot,
};

use crate::{core::Events, host::strip_ansi};

/// How many log lines are kept for the tail.
pub const LOG_LINES: usize = 2000;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum RunState {
    /// Never started, or stopped by the user.
    Stopped,
    Running {
        pid: u32,
    },
    /// The stop was requested; the process has not ended yet.
    Stopping {
        pid: u32,
    },
    /// The process ended by itself (a crash or a failed start).
    Exited {
        code: Option<i32>,
    },
}

struct Live {
    pid: u32,
    stop: Option<oneshot::Sender<()>>,
}

#[derive(Default)]
struct Inner {
    live: Option<Live>,
    state: Option<RunState>,
}

/// One hosted server process at a time.
#[derive(Clone, Default)]
pub struct HostRuntime {
    inner: Arc<Mutex<Inner>>,
    log: Arc<Mutex<VecDeque<String>>>,
}

/// Everything needed to start the server.
#[derive(Debug, Clone)]
pub struct StartSpec {
    pub exe: PathBuf,
    /// The package folder; the server's working directory.
    pub cwd: PathBuf,
    /// `POE_CONFIG`, relative to `cwd`.
    pub config: PathBuf,
    /// The log file (truncated at each start).
    pub log_file: PathBuf,
}

impl HostRuntime {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn state(&self) -> RunState {
        let g = self.inner.lock().unwrap();
        g.state.clone().unwrap_or(RunState::Stopped)
    }

    pub fn is_running(&self) -> bool {
        matches!(
            self.state(),
            RunState::Running { .. } | RunState::Stopping { .. }
        )
    }

    pub fn pid(&self) -> Option<u32> {
        match self.state() {
            RunState::Running { pid } | RunState::Stopping { pid } => Some(pid),
            _ => None,
        }
    }

    /// The last `n` log lines (oldest first).
    pub fn tail(&self, n: usize) -> Vec<String> {
        let g = self.log.lock().unwrap();
        let skip = g.len().saturating_sub(n);
        g.iter().skip(skip).cloned().collect()
    }

    fn push_line(&self, events: &dyn Events, line: String) {
        {
            let mut g = self.log.lock().unwrap();
            if g.len() >= LOG_LINES {
                g.pop_front();
            }
            g.push_back(line.clone());
        }
        events.emit("host:log", json!({ "line": line }));
    }

    /// Adds a line the launcher itself wants in the log (not from the server).
    pub fn note(&self, events: &dyn Events, text: &str) {
        self.push_line(events, format!("[launcher] {text}"));
    }

    fn set_state(&self, events: &dyn Events, s: RunState) {
        self.inner.lock().unwrap().state = Some(s.clone());
        events.emit("host:status", serde_json::to_value(&s).unwrap_or_default());
    }

    /// Starts the server. Returns once the process exists; whether it is serving is for the
    /// caller to find out (the web API answers).
    pub fn start(&self, events: Arc<dyn Events>, spec: &StartSpec) -> Result<u32> {
        if self.is_running() {
            bail!("the server is already running");
        }
        if !spec.exe.is_file() {
            bail!(
                "the server program {} is missing; install the server package again",
                spec.exe.display()
            );
        }
        if let Some(parent) = spec.log_file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = Arc::new(Mutex::new(
            std::fs::File::create(&spec.log_file)
                .with_context(|| format!("creating {}", spec.log_file.display()))?,
        ));
        self.log.lock().unwrap().clear();

        let mut cmd = Command::new(&spec.exe);
        cmd.current_dir(&spec.cwd)
            .env("POE_CONFIG", &spec.config)
            .env("NO_COLOR", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(windows)]
        {
            // No console window for the server.
            cmd.creation_flags(0x0800_0000);
        }
        let mut child = cmd
            .spawn()
            .with_context(|| format!("starting {}", spec.exe.display()))?;
        let pid = child.id().context("the server ended at once")?;
        let (stop_tx, stop_rx) = oneshot::channel::<()>();
        self.inner.lock().unwrap().live = Some(Live {
            pid,
            stop: Some(stop_tx),
        });
        self.set_state(events.as_ref(), RunState::Running { pid });
        self.note(
            events.as_ref(),
            &format!("started {} (pid {pid})", spec.exe.display()),
        );

        let readers = [
            child
                .stdout
                .take()
                .map(|s| self.spawn_reader(s, events.clone(), file.clone())),
            child
                .stderr
                .take()
                .map(|s| self.spawn_reader(s, events.clone(), file.clone())),
        ];
        let rt = self.clone();
        tokio::spawn(async move {
            let mut stopping = false;
            let status = tokio::select! {
                s = child.wait() => s,
                _ = stop_rx => {
                    stopping = true;
                    rt.set_state(events.as_ref(), RunState::Stopping { pid });
                    terminate(&mut child, pid).await
                }
            };
            for r in readers.into_iter().flatten() {
                let _ = tokio::time::timeout(Duration::from_secs(2), r).await;
            }
            let code = status.ok().and_then(|s| s.code());
            rt.inner.lock().unwrap().live = None;
            rt.note(
                events.as_ref(),
                &format!(
                    "the server ended (exit code {})",
                    code.map_or("none".to_owned(), |c| c.to_string())
                ),
            );
            rt.set_state(
                events.as_ref(),
                if stopping {
                    RunState::Stopped
                } else {
                    RunState::Exited { code }
                },
            );
        });
        Ok(pid)
    }

    fn spawn_reader<R: AsyncRead + Unpin + Send + 'static>(
        &self,
        stream: R,
        events: Arc<dyn Events>,
        file: Arc<Mutex<std::fs::File>>,
    ) -> tokio::task::JoinHandle<()> {
        let rt = self.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stream).lines();
            while let Ok(Some(raw)) = lines.next_line().await {
                let line = strip_ansi(&raw);
                {
                    use std::io::Write;
                    let _ = writeln!(file.lock().unwrap(), "{line}");
                }
                rt.push_line(events.as_ref(), line);
            }
        })
    }

    /// Asks the server to stop and waits (up to `wait`) for the process to end.
    pub async fn stop(&self, wait: Duration) -> Result<()> {
        let tx = {
            let mut g = self.inner.lock().unwrap();
            g.live.as_mut().and_then(|l| l.stop.take())
        };
        match tx {
            Some(tx) => {
                let _ = tx.send(());
            }
            None if self.is_running() => {} // already stopping
            None => return Ok(()),
        }
        let t0 = Instant::now();
        while self.is_running() {
            if t0.elapsed() > wait {
                bail!("the server did not stop within {} seconds", wait.as_secs());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Ok(())
    }

    /// For the launcher's exit: ends the process without the async machinery.
    pub fn stop_blocking(&self) {
        let pid = self.inner.lock().unwrap().live.as_ref().map(|l| l.pid);
        if let Some(pid) = pid {
            kill_blocking(pid);
        }
    }
}

/// SIGTERM (a polite stop), then a hard kill after a few seconds. On Windows a hard kill.
async fn terminate(
    child: &mut tokio::process::Child,
    pid: u32,
) -> std::io::Result<std::process::ExitStatus> {
    #[cfg(unix)]
    {
        // SAFETY: plain kill(2) on the pid of our own child.
        unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
        if let Ok(r) = tokio::time::timeout(Duration::from_secs(8), child.wait()).await {
            return r;
        }
    }
    let _ = pid;
    let _ = child.start_kill();
    child.wait().await
}

#[cfg(unix)]
fn kill_blocking(pid: u32) {
    // SAFETY: kill(2) on a pid we spawned; signal 0 only probes.
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGTERM);
        for _ in 0..30 {
            if libc::kill(pid as libc::pid_t, 0) != 0 {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        libc::kill(pid as libc::pid_t, libc::SIGKILL);
    }
}

#[cfg(windows)]
fn kill_blocking(pid: u32) {
    let _ = std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// Runs `poe-server <args>` to completion in the package folder (the offline `account ...` and
/// `character ...` commands) and returns what it printed. Never pass a secret in `args`.
pub async fn run_oneshot(exe: &Path, cwd: &Path, config: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new(exe)
        .args(args)
        .current_dir(cwd)
        .env("POE_CONFIG", config)
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .output()
        .await
        .with_context(|| format!("running {}", exe.display()))?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    if !out.status.success() {
        bail!(
            "poe-server {} failed: {}",
            args.first().unwrap_or(&""),
            strip_ansi(text.trim())
        );
    }
    Ok(strip_ansi(text.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{Collector, NullEvents};

    #[cfg(unix)]
    fn script(dir: &Path, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let p = dir.join("fake-server");
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn output_is_streamed_and_stop_ends_the_process() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = script(
            tmp.path(),
            "echo \"started with $POE_CONFIG in $(pwd | xargs basename)\"; echo oops >&2; exec sleep 30",
        );
        let rt = HostRuntime::new();
        let events = Arc::new(Collector::default());
        let spec = StartSpec {
            exe,
            cwd: tmp.path().to_path_buf(),
            config: PathBuf::from("config/x.toml"),
            log_file: tmp.path().join("logs/server.log"),
        };
        let pid = rt.start(events.clone(), &spec).unwrap();
        assert_eq!(rt.state(), RunState::Running { pid });
        assert!(
            rt.start(events.clone(), &spec).is_err(),
            "one server at a time"
        );
        for _ in 0..50 {
            if rt.tail(10).iter().any(|l| l.contains("oops")) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let tail = rt.tail(10).join("\n");
        assert!(tail.contains("started with config/x.toml"), "{tail}");
        assert!(tail.contains("oops"), "{tail}");
        rt.stop(Duration::from_secs(10)).await.unwrap();
        assert_eq!(rt.state(), RunState::Stopped);
        assert!(rt.pid().is_none());
        let log = std::fs::read_to_string(tmp.path().join("logs/server.log")).unwrap();
        assert!(log.contains("oops"));
        let ev = events.0.lock().unwrap();
        assert!(ev.iter().any(|(n, _)| n == "host:log"));
        assert!(
            ev.iter()
                .any(|(n, v)| n == "host:status" && v["state"] == "stopped")
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_server_that_ends_by_itself_is_reported() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = script(tmp.path(), "echo bye; exit 3");
        let rt = HostRuntime::new();
        let spec = StartSpec {
            exe,
            cwd: tmp.path().to_path_buf(),
            config: "c.toml".into(),
            log_file: tmp.path().join("s.log"),
        };
        rt.start(Arc::new(NullEvents), &spec).unwrap();
        for _ in 0..100 {
            if matches!(rt.state(), RunState::Exited { .. }) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert_eq!(rt.state(), RunState::Exited { code: Some(3) });
        assert!(rt.tail(5).iter().any(|l| l == "bye"));
        // Stopping a server that is not running is fine, and it can be started again.
        rt.stop(Duration::from_secs(1)).await.unwrap();
        rt.start(Arc::new(NullEvents), &spec).unwrap();
    }

    #[test]
    fn a_missing_program_is_a_readable_error() {
        let rt = HostRuntime::new();
        let tmp = tempfile::tempdir().unwrap();
        let e = rt
            .start(
                Arc::new(NullEvents),
                &StartSpec {
                    exe: tmp.path().join("nope"),
                    cwd: tmp.path().into(),
                    config: "c".into(),
                    log_file: tmp.path().join("l"),
                },
            )
            .unwrap_err()
            .to_string();
        assert!(e.contains("missing"), "{e}");
    }

    #[test]
    fn the_tail_is_bounded() {
        let rt = HostRuntime::new();
        for i in 0..(LOG_LINES + 50) {
            rt.push_line(&NullEvents, format!("line {i}"));
        }
        let all = rt.tail(usize::MAX);
        assert_eq!(all.len(), LOG_LINES);
        assert_eq!(all.last().unwrap(), &format!("line {}", LOG_LINES + 49));
        assert_eq!(rt.tail(2).len(), 2);
    }
}
