//! The Owner's Hosts on this box (ADR 0014). Each is made once, with its settings in a file the
//! Owner can edit (`<config>/hosts/NAME.toml`) and its state in a folder only tooob writes
//! (`<state>/NAME/`), and runs as a process of its own, holding a lock on its state while it does.
use crate::Network;
use crate::control::Client;
use crate::host::Site;
use anyhow::{Context, Result, anyhow, bail};
use std::{
    fs::File,
    io,
    net::{SocketAddr, ToSocketAddrs},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    os::unix::io::AsRawFd,
    path::{Path, PathBuf},
};

/// The `relay` setting for no Relay at all: this machine and its network only.
pub const NO_RELAY: &str = "none";

/// Where the Owner's Hosts live: their settings files, and their state folders.
pub struct Home {
    config: PathBuf,
    state: PathBuf,
}

/// One Host on this box, running or not.
pub struct Entry {
    pub name: String,
    pub settings: Settings,
    pub settings_file: PathBuf,
    pub state_dir: PathBuf,
}

/// What the Owner can change by editing a Host's settings file.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    /// A folder or file (absolute), or `HOST:PORT` / `http://HOST:PORT` for a web app.
    pub serves: String,
    /// A Relay's URL, or `none`.
    pub relay: String,
    /// Hosts made before a Relay served its Bootstrap (ADR 0020) named one here. Ignored.
    #[serde(default, rename = "bootstrap")]
    _bootstrap: Option<toml::Value>,
}

impl Settings {
    pub fn site(&self) -> Result<Site> {
        if Path::new(&self.serves).is_absolute() {
            return Ok(Site::SharedFolder(self.serves.clone().into()));
        }
        Ok(Site::WebApp(web_app(&self.serves)?))
    }

    pub fn network(&self) -> Result<Network> {
        relay_network(&self.relay)
    }
}

/// Where an Owner says to look for a Relay: tooob names none of its own (ADR 0015).
pub const RUN_YOUR_OWN: &str = "deploy/ shows how to run your own";

/// Check a `relay` setting: a Relay's URL, or `none`.
pub fn check_relay(text: &str) -> Result<()> {
    relay_network(text).map(drop)
}

fn relay_network(relay: &str) -> Result<Network> {
    if relay == NO_RELAY {
        return Ok(Network::LocalOnly);
    }
    let url = relay
        .parse()
        .with_context(|| format!("{relay:?} isn't a Relay's URL (or {NO_RELAY:?})"))?;
    Ok(Network::Relay(url))
}

/// Why a Host using `relay` has no Key Address, if it has none: browsers reach Hosts only through a
/// Relay's domain (ADR 0020).
pub fn no_key_address(relay: &str) -> Option<&'static str> {
    if relay == NO_RELAY {
        return Some("it has no Relay");
    }
    match relay.parse::<reqwest::Url>() {
        Ok(url) if url.domain().is_none() => Some("its Relay is at an IP address"),
        _ => None,
    }
}

/// What to say when `host` (a Host Name, or "the new Host") has no Key Address, and `why`.
pub fn no_invite_links(host: &str, why: &str) -> anyhow::Error {
    anyhow!(
        "{host} has no Key Address, so no Invite Links: {why}, and browsers reach Hosts only \
         through a Relay's domain ({RUN_YOUR_OWN})"
    )
}

/// Where a web app listens, from `HOST:PORT` or `http://HOST[:PORT][/]`; IPv4 first, since a web
/// app on `localhost` usually listens there.
fn web_app(address: &str) -> Result<SocketAddr> {
    let bad = || format!("{address:?} isn't a folder, a file, HOST:PORT or http://HOST:PORT");
    let hostport = match address.split_once("://") {
        Some(("http", rest)) => {
            let rest = rest.strip_suffix('/').unwrap_or(rest);
            if rest.contains('/') {
                bail!("{}", bad());
            }
            match rest.rsplit_once(':') {
                Some((_, port)) if port.parse::<u16>().is_ok() => rest.to_string(),
                _ => format!("{rest}:80"),
            }
        }
        Some(_) => bail!("{} (only http:// reaches a web app)", bad()),
        None => address.to_string(),
    };
    let addrs: Vec<SocketAddr> = hostport.to_socket_addrs().with_context(bad)?.collect();
    addrs
        .iter()
        .find(|a| a.is_ipv4())
        .or(addrs.first())
        .copied()
        .with_context(bad)
}

/// Letters, digits, `-` and `_`: safe as a file name, and in a shell, as it is.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.chars().all(name_char)
}

/// A word as a shell would need it: unchanged when it's plain, single-quoted otherwise.
pub fn quote(word: &str) -> String {
    let plain = !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./:@%+=,".contains(c));
    if plain {
        word.to_string()
    } else {
        format!("'{}'", word.replace('\'', r"'\''"))
    }
}

/// Whether `c` can be part of a Host Name.
pub fn name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_'
}

impl Home {
    /// Under `TOOOB_HOME` if it's set; otherwise the XDG config and state folders.
    pub fn from_env() -> Result<Home> {
        if let Some(dir) = std::env::var_os("TOOOB_HOME") {
            let dir = PathBuf::from(dir);
            return Ok(Home {
                config: dir.join("config"),
                state: dir.join("state"),
            });
        }
        let home = PathBuf::from(std::env::var_os("HOME").context("HOME isn't set")?);
        let xdg = |var: &str, fallback: &str| {
            std::env::var_os(var)
                .map(PathBuf::from)
                .filter(|dir| dir.is_absolute())
                .unwrap_or_else(|| home.join(fallback))
                .join("tooob")
        };
        Ok(Home {
            config: xdg("XDG_CONFIG_HOME", ".config"),
            state: xdg("XDG_STATE_HOME", ".local/state"),
        })
    }

    fn settings_file(&self, name: &str) -> PathBuf {
        self.config.join("hosts").join(format!("{name}.toml"))
    }

    /// The names of every Host made here, sorted.
    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.config.join("hosts"))
            .into_iter()
            .flatten()
            .filter_map(|e| {
                let path = e.ok()?.path();
                let name = path.file_name()?.to_str()?.strip_suffix(".toml")?;
                valid_name(name).then(|| name.to_string())
            })
            .collect();
        names.sort_by_key(|n| n.to_lowercase());
        names
    }

    /// Every Host made here whose settings can be read.
    pub fn hosts(&self) -> Vec<Entry> {
        self.names()
            .iter()
            .filter_map(|n| self.entry(n).ok())
            .collect()
    }

    /// The Host named `name`.
    pub fn entry(&self, name: &str) -> Result<Entry> {
        let settings_file = self.settings_file(name);
        let text = std::fs::read_to_string(&settings_file)
            .with_context(|| format!("no Host is named {name:?}"))?;
        let settings = toml::from_str(&text)
            .with_context(|| format!("can't read {}", settings_file.display()))?;
        Ok(Entry {
            name: name.to_string(),
            settings,
            settings_file,
            state_dir: self.state.join(name),
        })
    }

    /// The Host `wanted` names: by its Host Name, its path or a Host Key prefix. Never a guess:
    /// no match, or more than one, is an error.
    pub fn find(&self, wanted: &str) -> Result<Entry> {
        // A Host Name is what the Owner chose: it wins, and a settings file that can't be read
        // says so rather than hiding the Host.
        if valid_name(wanted) && self.settings_file(wanted).exists() {
            return self.entry(wanted);
        }
        let hosts = self.hosts();
        let mut matches: Vec<Entry> = hosts.into_iter().filter(|h| h.answers_to(wanted)).collect();
        match matches.len() {
            1 => Ok(matches.remove(0)),
            0 => bail!("no Host is {wanted:?}{}", self.known()),
            _ => bail!(
                "{wanted:?} could be several Hosts; say which by name{}",
                self.known()
            ),
        }
    }

    /// The Hosts there are, for an error message.
    fn known(&self) -> String {
        match self.names().as_slice() {
            [] => " (none yet: `tooob add NAME TARGET` makes one)".into(),
            names => format!(" (Hosts here: {})", names.join(", ")),
        }
    }

    /// Make a Host: its state, with its Host Key, then its settings file. `target` is a folder or
    /// file (a Shared Folder), or `HOST:PORT` / `http://HOST:PORT` (a web app).
    pub fn add(&self, name: &str, target: &str, relay: &str) -> Result<Entry> {
        if !valid_name(name) {
            bail!("{name:?} can't be a Host Name: use letters, digits, - and _");
        }
        let settings_file = self.settings_file(name);
        let state_dir = self.state.join(name);
        if settings_file.exists() {
            bail!("there's already a Host named {name}");
        }
        if state_dir.exists() {
            bail!(
                "{} is left from an earlier Host named {name}, Host Key and all: delete it, or use another name",
                state_dir.display()
            );
        }
        let serves = match Path::new(target).canonicalize() {
            Ok(path) => path
                .to_str()
                .with_context(|| format!("can't share {target}: its path isn't UTF-8"))?
                .to_string(),
            Err(_) => {
                web_app(target)?;
                target.to_string()
            }
        };
        let settings = Settings {
            serves,
            relay: relay.to_string(),
            _bootstrap: None,
        };
        settings.network()?;
        let made = self.make(&settings, &settings_file, &state_dir);
        if made.is_err() {
            // Never half a Host: a state folder alone would block the name, unlisted.
            let _ = std::fs::remove_dir_all(&state_dir);
        }
        made?;
        Ok(Entry {
            name: name.to_string(),
            settings,
            settings_file,
            state_dir,
        })
    }

    /// The state folder, with its Host Key, then the settings file: a Host exists once that's there.
    fn make(&self, settings: &Settings, settings_file: &Path, state_dir: &Path) -> Result<()> {
        // Private to the Owner's account: anyone who can read the Host Key can impersonate it.
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(state_dir)?;
        crate::host::load_host_key(state_dir)?;
        std::fs::create_dir_all(settings_file.parent().unwrap())?;
        let text = format!(
            "# What this Host serves: a folder or file (a Shared Folder), or HOST:PORT or\n\
             # http://HOST:PORT (a web app). A Host only ever serves what it first ran with.\n\
             serves = {}\n\n\
             # The Relay its Visitors reach it through, which also serves the page their browsers\n\
             # load first; or \"{NO_RELAY}\": this machine and its network only. Takes effect the\n\
             # next time it runs.\n\
             relay = {}\n",
            toml::Value::from(settings.serves.as_str()),
            toml::Value::from(settings.relay.as_str()),
        );
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(settings_file)?;
        io::Write::write_all(&mut file, text.as_bytes())?;
        Ok(())
    }
}

impl Entry {
    /// Its Key Address, `<host key>.<relay domain>`, from the Relay its settings name (ADR 0020):
    /// the page its Invite Links open. A Host with no Relay, or a Relay at an IP address, has
    /// none, since browsers reach Hosts only through a Relay's domain.
    pub fn key_address(&self) -> Result<String> {
        let in_settings = || format!("its Relay is set in {}", self.settings_file.display());
        if let Some(why) = no_key_address(&self.settings.relay) {
            return Err(no_invite_links(&self.name, why).context(in_settings()));
        }
        let mut url: reqwest::Url = self.settings.relay.parse().with_context(in_settings)?;
        let domain = url.domain().context("no domain")?.to_string();
        let key = self
            .host_key()
            .with_context(|| format!("{} has no Host Key", self.name))?;
        url.set_host(Some(&format!("{key}.{domain}")))?;
        url.set_path("/");
        url.set_query(None);
        url.set_fragment(None);
        Ok(url.to_string())
    }

    /// The Invite Link for `token`: its Key Address, with the Invite after the `#`, which stays in
    /// the browser (never sent to the Relay).
    pub fn invite_link(&self, token: &str) -> Result<String> {
        Ok(format!("{}#invite={token}", self.key_address()?))
    }

    /// Its Host Key, as Invite Links carry it.
    pub fn host_key(&self) -> Option<String> {
        crate::host::host_key(&self.state_dir)
    }

    /// Where the service manager writes its output (#40).
    pub fn log_file(&self) -> PathBuf {
        self.state_dir.join("host.log")
    }

    /// The pid of the process running this Host, if one is. Asks without taking the lock, so it
    /// never gets in the way of a Host starting. Never ask in the process holding the lock: closing
    /// any other descriptor on the file releases a POSIX lock.
    pub fn running(&self) -> Option<u32> {
        let file = self.lock_file().ok()?;
        let held = record_lock(&file, libc::F_GETLK).ok()?;
        (held.l_type != libc::F_UNLCK as libc::c_short).then_some(held.l_pid as u32)
    }

    /// Wait until it isn't running, for up to 10 s.
    pub fn until_stopped(&self) -> Result<()> {
        let started = std::time::Instant::now();
        while self.running().is_some() {
            if started.elapsed() > std::time::Duration::from_secs(10) {
                bail!("{} didn't stop within 10 s", self.name);
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        Ok(())
    }

    /// Take the lock on its state, which a running Host holds as long as it runs: one process per
    /// Host, and nothing else writes its state meanwhile. Refused, with the pid, while it's held.
    /// A command changing a stopped Host's state holds the lock for a moment; that's waited out,
    /// even when several take turns at it (#65): only one process holding it for a whole second is
    /// a running Host.
    pub fn lock(&self) -> Result<Lock> {
        use std::time::{Duration, Instant};
        let started = Instant::now();
        let mut holder: Option<(u32, Instant)> = None;
        let lock = loop {
            if let Some(lock) = try_lock(&self.state_dir)? {
                break lock;
            }
            holder = match (holder, self.running()) {
                (Some((was, since)), Some(pid)) if was == pid => {
                    if since.elapsed() > Duration::from_secs(1) {
                        bail!("{} is already running (pid {pid})", self.name);
                    }
                    Some((was, since))
                }
                (_, Some(pid)) => Some((pid, Instant::now())),
                (_, None) => None,
            };
            if started.elapsed() > Duration::from_secs(10) {
                bail!(
                    "{} is busy: other commands keep taking turns at its lock",
                    self.name
                );
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        // `rm` may have deleted it while this waited to open the lock file.
        if !self.settings_file.exists() {
            bail!("no Host is named {:?}", self.name);
        }
        Ok(lock)
    }

    fn lock_file(&self) -> io::Result<File> {
        lock_file(&self.state_dir)
    }

    /// The Owner's commands on it, running or not (ADR 0014).
    pub fn client(&self) -> Client {
        Client::running_or_not(&self.state_dir)
    }

    /// Delete its settings and state, Host Key and all. Refused while it's running.
    pub fn remove(self) -> Result<()> {
        let _lock = self.lock().map_err(|e| {
            anyhow::anyhow!(
                "{e}: stop it first (Ctrl-C where it runs, or `tooob stop {}`)",
                self.name
            )
        })?;
        std::fs::remove_file(&self.settings_file)?;
        // Moved aside first, while locked: a command waiting for the lock finds nothing where the
        // state was, rather than making a new lock file inside a folder being deleted.
        let doomed = self.state_dir.with_file_name(format!(
            ".{}.removing-{}",
            self.name,
            std::process::id()
        ));
        std::fs::rename(&self.state_dir, &doomed)?;
        // A command waiting for the lock may still make a `lock` file in it: it found the state
        // folder just before the rename (#65). It finds that path gone next, and makes no more.
        let mut tries = 0;
        loop {
            match std::fs::remove_dir_all(&doomed) {
                Ok(()) => return Ok(()),
                Err(e) if e.raw_os_error() == Some(libc::ENOTEMPTY) && tries < 20 => {
                    tries += 1;
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(e) => return Err(e.into()),
            }
        }
    }

    /// Whether `wanted` names this Host: its Host Name, what it serves, or a prefix of its Host
    /// Key (four characters or more).
    pub fn answers_to(&self, wanted: &str) -> bool {
        let served = match Path::new(wanted).canonicalize() {
            Ok(path) => path.to_string_lossy().into_owned(),
            Err(_) => wanted.to_string(),
        };
        self.name == wanted
            || self.settings.serves == served
            || (wanted.len() >= 4 && self.host_key().is_some_and(|id| id.starts_with(wanted)))
    }
}

/// Take the lock on the Host's state in `state_dir`; None if a running Host (or another command)
/// holds it. An error if the Host was removed while this waited.
pub(crate) fn try_lock(state_dir: &Path) -> io::Result<Option<Lock>> {
    let file = lock_file(state_dir)?;
    match record_lock(&file, libc::F_SETLK) {
        Ok(_) => {}
        Err(e) if matches!(e.raw_os_error(), Some(libc::EAGAIN | libc::EACCES)) => return Ok(None),
        Err(e) => return Err(e),
    }
    // `rm` held it, and deleted the state: this lock is on a file nobody else will ever open.
    let here = std::fs::metadata(state_dir.join("lock"))?;
    if here.ino() != file.metadata()?.ino() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "the Host was removed",
        ));
    }
    Ok(Some(Lock(file)))
}

fn lock_file(state_dir: &Path) -> io::Result<File> {
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(state_dir.join("lock"))
}

/// The lock on a Host's state; released when dropped, or when the process ends however it ends.
pub struct Lock(#[allow(dead_code)] File);

/// A POSIX record lock on the whole file: `F_SETLK` takes it, `F_GETLK` says who holds it.
fn record_lock(file: &File, cmd: libc::c_int) -> io::Result<libc::flock> {
    let mut lock: libc::flock = unsafe { std::mem::zeroed() };
    lock.l_type = libc::F_WRLCK as _;
    lock.l_whence = libc::SEEK_SET as _;
    if unsafe { libc::fcntl(file.as_raw_fd(), cmd, &mut lock) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(lock)
}
