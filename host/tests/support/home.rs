//! A `TOOOB_HOME` of its own for each test, Hosts run in it, and folders to share.
use std::{
    path::Path,
    process::{Child, Command, ExitStatus, Output, Stdio},
    time::{Duration, Instant},
};

/// The Bootstrap the tests' Hosts name: no Visitor's browser loads it, so it needn't exist.
pub const BOOTSTRAP: &str = "https://bootstrap.example/";

/// A `TOOOB_HOME` of its own, so tests never touch the Owner's real settings or Hosts.
pub struct Home(pub tempfile::TempDir);

impl Home {
    pub fn new() -> Home {
        Home(tempfile::tempdir().unwrap())
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_tooob"));
        cmd.args(args)
            .env("TOOOB_HOME", self.0.path())
            .env("HOME", self.0.path()) // whatever happens, never the Owner's real Hosts
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("XDG_STATE_HOME")
            .stdin(Stdio::null());
        cmd
    }

    /// Run `tooob`, as a script would. It must return within 30 s: a command that stays in the
    /// foreground fails the test instead of hanging it.
    pub fn tooob(&self, args: &[&str]) -> Output {
        let mut child = self
            .command(args)
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

    /// `tooob run NAME` in the background, once it's serving.
    pub fn start(&self, name: &str) -> Running {
        let running = self.spawn(&["run", name]);
        running.until_it_says("serving");
        running
    }

    /// `tooob ARGS` in the background, its output kept in a log of its own.
    pub fn spawn(&self, args: &[&str]) -> Running {
        let log = tempfile::NamedTempFile::new_in(self.0.path()).unwrap();
        let child = self
            .command(args)
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
