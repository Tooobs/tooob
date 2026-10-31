//! A `TOOOB_HOME` of its own for each test, Hosts run in it, and folders to share.
use std::{
    path::Path,
    process::{Child, Command, ExitStatus, Output, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

/// The Bootstrap the tests' Hosts name: no Visitor's browser loads it, so it needn't exist.
pub const BOOTSTRAP: &str = "https://bootstrap.example/";

/// The Relay the tests' stopped Hosts name when they need Invite Links: nothing reaches it, so it
/// needn't exist, and such a Host is made with `Home::add_naming`. A Host that runs needs a
/// TestRelay: one whose Relay serves no Bootstrap won't start (ADR 0020).
pub const RELAY: &str = "https://relay.example/";

/// A real Relay on this machine (iroh-relay's own server) behind a front that serves its
/// Bootstrap's `/__tooob/version.json`, as a Relay does (ADR 0020), and hands everything else to
/// iroh-relay. Stopped when dropped. It's on `localhost`, a domain, so its Hosts have Key
/// Addresses.
pub struct TestRelay {
    pub url: String,
    /// The same Relay at an IP address: its Hosts have no Key Address.
    pub ip_url: String,
    /// What its front answers for version.json: None hangs up, as if it couldn't be reached.
    version_json: Arc<Mutex<Option<Option<String>>>>,
    _stop: tokio::sync::oneshot::Sender<()>,
}

/// The version.json a Bundle of `release` speaking `protocol_version` has.
pub fn version_json(release: &str, protocol_version: &str) -> String {
    format!(r#"{{"release": "{release}", "protocolVersion": "{protocol_version}", "files": {{}}}}"#)
}

/// This build's own Bootstrap's version.json.
pub fn this_version_json() -> String {
    version_json(env!("CARGO_PKG_VERSION"), tooob::PROTOCOL_VERSION)
}

impl TestRelay {
    /// A Relay serving a Bootstrap that speaks this tooob's Protocol Version.
    pub fn start() -> TestRelay {
        TestRelay::spawn(true)
    }

    /// Only the front, with no Relay behind it: it serves a Bootstrap, but a Host never gets
    /// online through it.
    pub fn bootstrap_only() -> TestRelay {
        TestRelay::spawn(false)
    }

    /// From now on, serve `version_json` as its Bootstrap's, or no Bootstrap at all (a 404).
    pub fn serve_bootstrap(&self, version_json: Option<String>) {
        *self.version_json.lock().unwrap() = Some(version_json);
    }

    /// From now on, hang up on anyone asking for its Bootstrap, as a Relay that can't be reached
    /// would, while still being a Relay.
    pub fn hang_up_on_bootstrap(&self) {
        *self.version_json.lock().unwrap() = None;
    }

    fn spawn(with_relay: bool) -> TestRelay {
        use iroh_relay::server::{RelayConfig, Server, ServerConfig};
        let version_json = Arc::new(Mutex::new(Some(Some(this_version_json()))));
        let (port_tx, port_rx) = std::sync::mpsc::channel();
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let served = version_json.clone();
        // A runtime of its own, so sync and async tests alike can start one.
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async {
                let server = match with_relay {
                    true => {
                        let mut config = ServerConfig::default();
                        config.relay = Some(RelayConfig::new(([127, 0, 0, 1], 0)));
                        Some(Server::spawn(config).await.unwrap())
                    }
                    false => None,
                };
                let relay = server.as_ref().map(|s| s.http_addr().unwrap());
                let front = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                port_tx.send(front.local_addr().unwrap().port()).unwrap();
                let fronting = async {
                    while let Ok((conn, _)) = front.accept().await {
                        tokio::spawn(answer(conn, relay, served.clone()));
                    }
                };
                tokio::select! {
                    _ = fronting => {}
                    _ = stopped => {}
                }
                if let Some(server) = server {
                    let _ = server.shutdown().await;
                }
            });
        });
        let port = port_rx.recv().unwrap();
        TestRelay {
            url: format!("http://localhost:{port}/"),
            ip_url: format!("http://127.0.0.1:{port}/"),
            version_json,
            _stop: stop,
        }
    }
}

/// One connection to the front: its Bootstrap's version.json, or handed whole to iroh-relay.
async fn answer(
    mut conn: tokio::net::TcpStream,
    relay: Option<std::net::SocketAddr>,
    version_json: Arc<Mutex<Option<Option<String>>>>,
) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut head = Vec::new();
    let mut buf = [0; 4096];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") {
        match conn.read(&mut buf).await {
            Ok(0) | Err(_) => return,
            Ok(n) => head.extend_from_slice(&buf[..n]),
        }
    }
    let path = String::from_utf8_lossy(&head)
        .split_whitespace()
        .nth(1)
        .unwrap_or_default()
        .to_string();
    let reply = |status: &str, body: &str| {
        format!(
            "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\
             connection: close\r\n\r\n{body}",
            body.len()
        )
    };
    let upstream = match (path.as_str(), relay) {
        ("/__tooob/version.json", _) => {
            let served = version_json.lock().unwrap().clone();
            let reply = match served {
                Some(Some(body)) => reply("200 OK", &body),
                Some(None) => reply("404 Not Found", ""),
                None => return,
            };
            let _ = conn.write_all(reply.as_bytes()).await;
            return;
        }
        (_, Some(relay)) => relay,
        (_, None) => {
            let _ = conn.write_all(reply("404 Not Found", "").as_bytes()).await;
            return;
        }
    };
    let Ok(mut upstream) = tokio::net::TcpStream::connect(upstream).await else {
        return;
    };
    if upstream.write_all(&head).await.is_ok() {
        let _ = tokio::io::copy_bidirectional(&mut conn, &mut upstream).await;
    }
}

/// z-base-32's alphabet: how a Host Key is written, short enough to be a hostname's label.
pub const Z_BASE_32: &str = "ybndrfg8ejkmcpqxot1uwisza345h769";

/// A `TOOOB_HOME` of its own, so tests never touch the Owner's real settings or Hosts. Its `tooob`
/// never reaches GitHub: it looks for a newer Release only at a stand-in (`Home::releases_at`),
/// and otherwise not at all.
pub struct Home(pub tempfile::TempDir, Mutex<Option<String>>);

/// Where a test's `tooob` finds Releases when none is set: nothing listens there.
const NO_RELEASES: &str = "http://127.0.0.1:9/";

impl Home {
    pub fn new() -> Home {
        let home = Home(tempfile::tempdir().unwrap(), Default::default());
        std::fs::create_dir(home.stand_ins()).unwrap();
        home
    }

    /// From now on, its `tooob` finds GitHub's Releases at `url` (a stand-in), and its running
    /// Hosts look for a newer one there.
    pub fn releases_at(&self, url: &str) {
        *self.1.lock().unwrap() = Some(url.to_string());
    }

    /// What its `tooob` has in its environment about Releases: a stand-in GitHub, or none, with
    /// the daily lookup off.
    pub fn releases_env(&self) -> Vec<(&'static str, String)> {
        match self.1.lock().unwrap().clone() {
            Some(url) => vec![("TOOOB_RELEASES_URL", url)],
            None => vec![
                ("TOOOB_RELEASES_URL", NO_RELEASES.to_string()),
                ("TOOOB_NO_UPDATE_CHECK", "1".to_string()),
            ],
        }
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_tooob"));
        cmd.args(args)
            .env("TOOOB_HOME", self.0.path())
            .env("HOME", self.0.path()) // whatever happens, never the Owner's real Hosts
            // Only stand-ins: never the Owner's real launchctl or systemctl.
            .env("PATH", self.stand_ins())
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("XDG_STATE_HOME")
            .env_remove("TOOOB_AUTOSTART")
            .env_remove("TOOOB_NO_UPDATE_CHECK")
            .env_remove("TOOOB_IN_CONTAINER")
            .envs(self.releases_env())
            .stdin(Stdio::null());
        cmd
    }

    /// The folder that's all of `PATH` for the `tooob` it runs: stand-ins for the tools tooob
    /// calls, and nothing else.
    pub fn stand_ins(&self) -> std::path::PathBuf {
        self.0.path().join("stand-ins")
    }

    /// A stand-in for `tool` (launchctl, systemctl) on `tooob`'s PATH: it records each call, one
    /// line of its arguments, in `<tool>.calls`, then runs `then` (sh), which may fail it.
    pub fn stand_in(&self, tool: &str, then: &str) {
        use std::os::unix::fs::PermissionsExt;
        let dir = self.stand_ins();
        let script = dir.join(tool);
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\n{then}\n",
                dir.join(format!("{tool}.calls")).display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// How `tool`'s stand-in was called, one call a line.
    pub fn calls(&self, tool: &str) -> Vec<String> {
        std::fs::read_to_string(self.stand_ins().join(format!("{tool}.calls")))
            .unwrap_or_default()
            .lines()
            .map(String::from)
            .collect()
    }

    /// tooob where the install script puts it, `~/.local/bin/tooob`: this build.
    pub fn install_tooob(&self) -> std::path::PathBuf {
        let bin = self.0.path().join(".local/bin");
        std::fs::create_dir_all(&bin).unwrap();
        let tooob = bin.join("tooob");
        std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_tooob"), &tooob).unwrap();
        tooob
    }

    /// Run `tooob`, as a script would. It must return within 30 s: a command that stays in the
    /// foreground fails the test instead of hanging it.
    pub fn tooob(&self, args: &[&str]) -> Output {
        self.tooob_with(args, &[])
    }

    /// Run `tooob` with `env` set too, as a script would.
    pub fn tooob_with(&self, args: &[&str], env: &[(&str, &str)]) -> Output {
        let mut child = self
            .command(args)
            .envs(env.iter().copied())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let started = Instant::now();
        while child.try_wait().unwrap().is_none() {
            if started.elapsed() > Duration::from_secs(30) {
                let _ = child.kill();
                panic!("tooob {args:?} didn't return within 30 s");
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        child.wait_with_output().unwrap()
    }

    /// Run, expecting success; its stdout.
    pub fn ok(&self, args: &[&str]) -> String {
        let out = self.tooob(args);
        assert!(
            out.status.success(),
            "tooob {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// `tooob add NAME TARGET` for a Host whose settings name `relay`, a Relay nothing here
    /// reaches (so `add` itself would refuse it): added through a stand-in serving a Bootstrap,
    /// then its settings file edited to name `relay`, as an Owner may. It won't run, and Owner
    /// commands work on it as on any stopped Host.
    pub fn add_naming(&self, name: &str, target: &str, relay: &str) {
        let stand_in = TestRelay::bootstrap_only();
        self.ok(&["add", name, target, "--relay", &stand_in.url]);
        let file = self.shown(name, "settings file");
        let settings = std::fs::read_to_string(&file).unwrap();
        let named = format!("relay = \"{}\"", stand_in.url);
        assert!(settings.contains(&named), "{settings}");
        std::fs::write(
            &file,
            settings.replace(&named, &format!("relay = \"{relay}\"")),
        )
        .unwrap();
    }

    /// `tooob run NAME` in the background, once it's serving.
    pub fn start(&self, name: &str) -> Running {
        let running = self.spawn(&["run", name]);
        running.until_it_says("serving");
        running
    }

    /// `tooob run NAME` in the background with `env` set too, once it's serving.
    pub fn start_with(&self, name: &str, env: &[(&str, &str)]) -> Running {
        let running = self.spawn_with(&["run", name], env);
        running.until_it_says("serving");
        running
    }

    /// `tooob ARGS` in the background, its output kept in a log of its own.
    pub fn spawn(&self, args: &[&str]) -> Running {
        self.spawn_with(args, &[])
    }

    /// `tooob ARGS` in the background with `env` set too.
    pub fn spawn_with(&self, args: &[&str], env: &[(&str, &str)]) -> Running {
        let log = tempfile::NamedTempFile::new_in(self.0.path()).unwrap();
        let child = self
            .command(args)
            .envs(env.iter().copied())
            .stdout(log.reopen().unwrap())
            .stderr(log.reopen().unwrap())
            .spawn()
            .unwrap();
        Running {
            child,
            log: log.into_temp_path(),
        }
    }

    /// The row `tooob ls` prints for a Host.
    pub fn row(&self, name: &str) -> String {
        let listed = self.ok(&["ls"]);
        listed
            .lines()
            .find(|l| l.split_whitespace().next() == Some(name))
            .unwrap_or_else(|| panic!("{name} isn't listed:\n{listed}"))
            .to_string()
    }

    /// What `tooob show NAME` says for `field`.
    pub fn shown(&self, name: &str, field: &str) -> String {
        let shown = self.ok(&["show", name]);
        shown
            .lines()
            .find_map(|l| l.trim().strip_prefix(field))
            .unwrap_or_else(|| panic!("no {field} in:\n{shown}"))
            .trim()
            .to_string()
    }
}

/// A `tooob` running in the background, killed when the test ends.
pub struct Running {
    pub child: Child,
    log: tempfile::TempPath,
}

impl Running {
    /// Everything it has written to stdout and stderr so far.
    pub fn log(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }

    pub fn until_it_says(&self, what: &str) {
        let started = Instant::now();
        while !self.log().contains(what) {
            assert!(
                started.elapsed() < Duration::from_secs(20),
                "it never said {what:?}:\n{}",
                self.log()
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Send `signal`; how it exited, within 5 s.
    pub fn stop_with(&mut self, signal: i32) -> ExitStatus {
        unsafe { libc::kill(self.child.id() as i32, signal) };
        let started = Instant::now();
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status;
            }
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "it didn't stop within 5 s"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    pub fn stop(mut self) {
        let status = self.stop_with(libc::SIGTERM);
        assert!(status.success(), "{status}: {}", self.log());
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A folder to share, named like a real one (temp folders start with a dot).
pub fn a_folder() -> tempfile::TempDir {
    let folder = tempfile::Builder::new()
        .prefix("photos-")
        .tempdir()
        .unwrap();
    std::fs::write(folder.path().join("notes.txt"), "hello friends\n").unwrap();
    folder
}

pub fn name_of(folder: &Path) -> String {
    folder.file_name().unwrap().to_string_lossy().into_owned()
}
