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

/// Exit code of `poe-server` after an admin's restart request when its config has
/// `[server] restart_mode = "exit"` (`poe_config::RESTART_EXIT_CODE` in Poe2ModdingProject, a
/// copy because this repository does not depend on the server crates). The launcher starts the
/// server again when it sees this code and did not ask it to stop.
pub const RESTART_EXIT_CODE: i32 = 75;
/// A crash loop ends: more than this many restart requests within [`RESTART_WINDOW`] are not
/// answered.
pub const MAX_RESTARTS: usize = 3;
pub const RESTART_WINDOW: Duration = Duration::from_secs(30);
/// How long the server gets after SIGTERM before it is killed. The server's graceful stop
/// (kick players, save sessions) takes up to about 5 s; this leaves room for a slow disk.
pub const STOP_GRACE: Duration = Duration::from_secs(15);

/// Allows [`MAX_RESTARTS`] restarts in any [`RESTART_WINDOW`].
#[derive(Debug, Default)]
pub struct RestartBudget(VecDeque<Instant>);

impl RestartBudget {
    /// Records a restart at `now` and says whether it is allowed.
    pub fn allow(&mut self, now: Instant) -> bool {
        while self
            .0
            .front()
            .is_some_and(|t| now.duration_since(*t) > RESTART_WINDOW)
        {
            self.0.pop_front();
        }
        if self.0.len() >= MAX_RESTARTS {
            return false;
        }
        self.0.push_back(now);
        true
    }
}

fn spawn_child(spec: &StartSpec) -> Result<tokio::process::Child> {
    let mut cmd = Command::new(&spec.exe);
    cmd.current_dir(&spec.cwd)
        .env("POE_CONFIG", &spec.config)
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    crate::child_env::clean_appimage_env(cmd.as_std_mut());
    #[cfg(windows)]
    {
        // No console window for the server.
        cmd.creation_flags(0x0800_0000);
    }
    cmd.spawn()
        .with_context(|| format!("starting {}", spec.exe.display()))
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

        let mut child = spawn_child(spec)?;
        let pid = child.id().context("the server ended at once")?;
        let (stop_tx, mut stop_rx) = oneshot::channel::<()>();
        self.inner.lock().unwrap().live = Some(Live {
            pid,
            stop: Some(stop_tx),
        });
        self.set_state(events.as_ref(), RunState::Running { pid });
        self.note(
            events.as_ref(),
            &format!("started {} (pid {pid})", spec.exe.display()),
        );

        let rt = self.clone();
        let spec = spec.clone();
        tokio::spawn(async move {
            let mut pid = pid;
            let mut budget = RestartBudget::default();
            loop {
                let readers = [
                    child
                        .stdout
                        .take()
                        .map(|s| rt.spawn_reader(s, events.clone(), file.clone())),
                    child
                        .stderr
                        .take()
                        .map(|s| rt.spawn_reader(s, events.clone(), file.clone())),
                ];
                let mut stopping = false;
                let status = tokio::select! {
                    s = child.wait() => s,
                    _ = &mut stop_rx => {
                        stopping = true;
                        rt.set_state(events.as_ref(), RunState::Stopping { pid });
                        terminate(&mut child, pid).await
                    }
                };
                for r in readers.into_iter().flatten() {
                    let _ = tokio::time::timeout(Duration::from_secs(2), r).await;
                }
                let code = status.ok().and_then(|s| s.code());
                rt.note(
                    events.as_ref(),
                    &format!(
                        "the server ended (exit code {})",
                        code.map_or("none".to_owned(), |c| c.to_string())
                    ),
                );
                // An admin's restart (`restart_mode = "exit"`): start the same server again.
                if !stopping && code == Some(RESTART_EXIT_CODE) {
                    if !budget.allow(Instant::now()) {
                        rt.note(
                            events.as_ref(),
                            &format!(
                                "the server asked for a restart more than {MAX_RESTARTS} times \
                                 within {} seconds; not starting it again",
                                RESTART_WINDOW.as_secs()
                            ),
                        );
                    } else {
                        rt.note(events.as_ref(), "the server asked for a restart");
                        match spawn_child(&spec).and_then(|c| {
                            let p = c.id().context("the server ended at once")?;
                            Ok((c, p))
                        }) {
                            Ok((c, p)) => {
                                child = c;
                                pid = p;
                                if let Some(l) = rt.inner.lock().unwrap().live.as_mut() {
                                    l.pid = p;
                                }
                                rt.set_state(events.as_ref(), RunState::Running { pid });
                                rt.note(events.as_ref(), &format!("restarted (pid {pid})"));
                                continue;
                            }
                            Err(e) => {
                                rt.note(
                                    events.as_ref(),
                                    &format!("could not restart the server: {e:#}"),
                                );
                            }
                        }
                    }
                }
                rt.inner.lock().unwrap().live = None;
                rt.set_state(
                    events.as_ref(),
                    if stopping {
                        RunState::Stopped
                    } else {
                        RunState::Exited { code }
                    },
                );
                break;
            }
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

/// SIGTERM (a polite stop; the server then kicks its players and saves their sessions, up to
/// about 5 s), then a hard kill after [`STOP_GRACE`]. On Windows a hard kill.
async fn terminate(
    child: &mut tokio::process::Child,
    pid: u32,
) -> std::io::Result<std::process::ExitStatus> {
    #[cfg(unix)]
    {
        // SAFETY: plain kill(2) on the pid of our own child.
        unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
        if let Ok(r) = tokio::time::timeout(STOP_GRACE, child.wait()).await {
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
        for _ in 0..(STOP_GRACE.as_millis() / 100) {
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
/// `character ...` commands) and returns what it printed. Never pass a secret in `args`; use
/// [`run_oneshot_stdin`].
pub async fn run_oneshot(exe: &Path, cwd: &Path, config: &Path, args: &[&str]) -> Result<String> {
    run_oneshot_stdin(exe, cwd, config, args, None).await
}

/// Like [`run_oneshot`], with `stdin` written to the child's standard input, which is then
/// closed. This is how a secret reaches the server (`account create <name> --password-stdin`):
/// unlike an argument or an environment variable, other programs cannot read it. The bytes are
/// not copied, logged or put in the error text.
pub async fn run_oneshot_stdin(
    exe: &Path,
    cwd: &Path,
    config: &Path,
    args: &[&str],
    stdin: Option<&[u8]>,
) -> Result<String> {
    use tokio::io::AsyncWriteExt;

    let mut cmd = Command::new(exe);
    cmd.args(args)
        .current_dir(cwd)
        .env("POE_CONFIG", config)
        .env("NO_COLOR", "1")
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    crate::child_env::clean_appimage_env(cmd.as_std_mut());
    let mut child = cmd
        .spawn()
        .with_context(|| format!("running {}", exe.display()))?;
    if let (Some(bytes), Some(mut pipe)) = (stdin, child.stdin.take()) {
        // A child that ends before it reads (a bad config) closes the pipe: its exit status
        // and output below say why, so a write error is not reported on its own.
        let _ = pipe.write_all(bytes).await;
        let _ = pipe.shutdown().await;
        drop(pipe);
    }
    let out = child
        .wait_with_output()
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

    #[cfg(unix)]
    #[tokio::test]
    async fn stdin_reaches_the_child_and_not_its_command_line() {
        let tmp = tempfile::tempdir().unwrap();
        // Prints how many arguments it got and what it read, then fails for `fail`.
        let exe = script(
            tmp.path(),
            "read line; echo \"args=$# line=$line\"; [ \"$1\" = fail ] && exit 2; exit 0",
        );
        let cfg = Path::new("c.toml");
        let out = run_oneshot_stdin(&exe, tmp.path(), cfg, &["create"], Some(b"s3cret value\n"))
            .await
            .unwrap();
        assert_eq!(out, "args=1 line=s3cret value");
        // Without stdin the child sees end of input at once.
        let out = run_oneshot(&exe, tmp.path(), cfg, &["x"]).await.unwrap();
        assert_eq!(out, "args=1 line=");
        // A failure reports the child's output and its exit status.
        let e = run_oneshot_stdin(&exe, tmp.path(), cfg, &["fail"], Some(b"hunter22 pw\n"))
            .await
            .unwrap_err()
            .to_string();
        assert!(e.contains("poe-server fail failed"), "{e}");
        assert!(e.contains("args=1"), "{e}");
    }

    #[test]
    fn the_restart_budget_ends_a_crash_loop() {
        let mut b = RestartBudget::default();
        let t0 = Instant::now();
        assert!(b.allow(t0));
        assert!(b.allow(t0 + Duration::from_secs(5)));
        assert!(b.allow(t0 + Duration::from_secs(10)));
        assert!(
            !b.allow(t0 + Duration::from_secs(11)),
            "a fourth within 30 s"
        );
        assert!(!b.allow(t0 + Duration::from_secs(29)));
        // The first one has left the window: one more is fine, then it is full again.
        assert!(b.allow(t0 + Duration::from_secs(31)));
        assert!(!b.allow(t0 + Duration::from_secs(32)));
        // A server that is restarted now and then (an admin every few minutes) is never refused.
        let mut b = RestartBudget::default();
        for k in 0..20 {
            assert!(b.allow(t0 + Duration::from_secs(k * 60)));
        }
    }

    async fn wait_for(rt: &HostRuntime, f: impl Fn(&RunState) -> bool) {
        for _ in 0..200 {
            if f(&rt.state()) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("state {:?} never reached", rt.state());
    }

    /// A fake server that asks for a restart (exit 75) on its first `n` runs, then runs on.
    #[cfg(unix)]
    fn restarting(dir: &Path, n: u32, then: &str) -> StartSpec {
        let counter = dir.join("runs");
        let exe = script(
            dir,
            &format!(
                "c=$(cat '{c}' 2>/dev/null || echo 0); c=$((c+1)); echo $c > '{c}'; echo \"run $c\"; \
                 if [ $c -le {n} ]; then exit 75; fi; {then}",
                c = counter.display()
            ),
        );
        StartSpec {
            exe,
            cwd: dir.to_path_buf(),
            config: "c.toml".into(),
            log_file: dir.join("s.log"),
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn an_exit_75_starts_the_server_again_with_the_same_spec() {
        let tmp = tempfile::tempdir().unwrap();
        let spec = restarting(tmp.path(), 1, "exec sleep 30");
        let rt = HostRuntime::new();
        let events = Arc::new(Collector::default());
        let first = rt.start(events.clone(), &spec).unwrap();
        // The second run is up, with another pid, and the state says Running throughout.
        wait_for(
            &rt,
            |s| matches!(s, RunState::Running { pid } if *pid != first),
        )
        .await;
        let RunState::Running { pid: second } = rt.state() else {
            unreachable!()
        };
        assert_eq!(rt.pid(), Some(second));
        // The new process's output is read asynchronously: Running can be announced before its
        // first line reaches the tail.
        for _ in 0..200 {
            if rt.tail(50).iter().any(|l| l.contains("run 2")) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let tail = rt.tail(50).join("\n");
        assert!(tail.contains("run 1") && tail.contains("run 2"), "{tail}");
        assert!(tail.contains("the server asked for a restart"), "{tail}");
        {
            let ev = events.0.lock().unwrap();
            let running = ev
                .iter()
                .filter(|(n, v)| n == "host:status" && v["state"] == "running")
                .count();
            assert_eq!(running, 2, "Running is announced again with the new pid");
            assert!(
                !ev.iter()
                    .any(|(n, v)| n == "host:status" && v["state"] == "exited")
            );
        }
        // The log file continues in one file.
        let log = std::fs::read_to_string(tmp.path().join("s.log")).unwrap();
        assert!(log.contains("run 1") && log.contains("run 2"), "{log}");
        // Stop still works on the new process and ends in Stopped, not in another restart.
        rt.stop(Duration::from_secs(10)).await.unwrap();
        assert_eq!(rt.state(), RunState::Stopped);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_restart_loop_ends_in_exited_75() {
        let tmp = tempfile::tempdir().unwrap();
        let spec = restarting(tmp.path(), 1000, "true");
        let rt = HostRuntime::new();
        rt.start(Arc::new(NullEvents), &spec).unwrap();
        wait_for(&rt, |s| matches!(s, RunState::Exited { .. })).await;
        assert_eq!(
            rt.state(),
            RunState::Exited {
                code: Some(RESTART_EXIT_CODE)
            }
        );
        // The first run plus MAX_RESTARTS restarts, no more.
        let runs: u32 = std::fs::read_to_string(tmp.path().join("runs"))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert_eq!(runs as usize, 1 + MAX_RESTARTS);
        let tail = rt.tail(100).join("\n");
        assert!(tail.contains("not starting it again"), "{tail}");
        assert!(rt.pid().is_none());
        // It can be started by hand again.
        std::fs::write(tmp.path().join("runs"), "5000").unwrap();
        rt.start(Arc::new(NullEvents), &spec).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn other_exit_codes_and_requested_stops_are_not_restarted() {
        let tmp = tempfile::tempdir().unwrap();
        // 75 only counts when the server was not asked to stop: a stop that the server answers
        // with 75 (it would not) ends in Stopped. Exit 0 and 1 are plain ends.
        let exe = script(
            tmp.path(),
            "trap 'exit 75' TERM; echo ready; while true; do sleep 0.1; done",
        );
        let spec = StartSpec {
            exe,
            cwd: tmp.path().into(),
            config: "c".into(),
            log_file: tmp.path().join("l"),
        };
        let rt = HostRuntime::new();
        rt.start(Arc::new(NullEvents), &spec).unwrap();
        for _ in 0..100 {
            if rt.tail(5).iter().any(|l| l == "ready") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        rt.stop(Duration::from_secs(10)).await.unwrap();
        assert_eq!(rt.state(), RunState::Stopped);
        assert!(
            !rt.tail(50)
                .iter()
                .any(|l| l.contains("asked for a restart"))
        );
        let spec1 = restarting(tmp.path(), 0, "exit 1");
        rt.start(Arc::new(NullEvents), &spec1).unwrap();
        wait_for(&rt, |s| matches!(s, RunState::Exited { .. })).await;
        assert_eq!(rt.state(), RunState::Exited { code: Some(1) });
    }
}
