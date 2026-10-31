//! Upgrade (ADR 0024): replacing this box's `tooob` command with another Release, only when its
//! Owner asks (`tooob upgrade`), by running that Release's own installer. Releases are on GitHub;
//! only final ones are offered, never a test Release (`0.2.0-rc.1`), though one can be named.
use anyhow::{Context, Result, bail};
use std::{
    cmp::Ordering,
    fmt,
    path::{Path, PathBuf},
    time::Duration,
};

/// Where tooob's Releases are.
const REPO: &str = "Tooobs/tooob";

/// A Release's number: `0.2.0`, or `0.2.0-rc.1` for a test Release of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    numbers: [u64; 3],
    /// A test Release's `rc.N`.
    rc: Option<u64>,
}

impl Version {
    /// `0.2.0`, `v0.2.0` or `0.2.0-rc.1`; None for anything else.
    pub fn parse(text: &str) -> Option<Version> {
        let text = text.strip_prefix('v').unwrap_or(text);
        let (numbers, rc) = match text.split_once('-') {
            Some((numbers, rc)) => (numbers, Some(rc.strip_prefix("rc.")?.parse().ok()?)),
            None => (text, None),
        };
        let numbers: Vec<u64> = numbers
            .split('.')
            .map(|n| n.parse().ok())
            .collect::<Option<_>>()?;
        Some(Version {
            numbers: numbers.try_into().ok()?,
            rc,
        })
    }

    /// This tooob's Release.
    pub fn this() -> Version {
        Version::parse(env!("CARGO_PKG_VERSION")).expect("a Release's version")
    }

    /// Whether it's a test Release.
    pub fn is_test(&self) -> bool {
        self.rc.is_some()
    }

    /// Whether it's in the same minor Release series as `other` (`0.2.x`), so speaks the same
    /// Protocol Version: only a minor Release changes it (ADR 0017).
    pub fn same_minor(&self, other: &Version) -> bool {
        self.numbers[..2] == other.numbers[..2]
    }
}

impl Ord for Version {
    /// A test Release comes before its final one.
    fn cmp(&self, other: &Version) -> Ordering {
        let rank = |v: &Version| (v.numbers, v.rc.is_none(), v.rc);
        rank(self).cmp(&rank(other))
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Version) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let [major, minor, patch] = self.numbers;
        write!(f, "{major}.{minor}.{patch}")?;
        if let Some(rc) = self.rc {
            write!(f, "-rc.{rc}")?;
        }
        Ok(())
    }
}

/// One published Release.
#[derive(Debug, Clone)]
pub struct Release {
    pub version: Version,
    /// Its notes, on GitHub.
    pub notes: String,
    /// The Protocol Version its notes say it speaks, as release.sh writes them for every final
    /// Release; None when they don't say (a test Release's).
    pub protocol_version: Option<String>,
}

impl Release {
    /// The Protocol Version it speaks: as its notes say, or, when they don't, this tooob's when
    /// it's in the same minor Release series (ADR 0017). None when that can't be told.
    pub fn speaks(&self) -> Option<String> {
        self.protocol_version.clone().or_else(|| {
            self.version
                .same_minor(&Version::this())
                .then(|| crate::PROTOCOL_VERSION.to_string())
        })
    }
}

/// tooob's Releases on GitHub: its Releases API, and its downloads.
pub struct Releases {
    api: reqwest::Url,
    downloads: reqwest::Url,
    http: reqwest::Client,
}

/// A Release as GitHub's API has it: only what tooob reads.
#[derive(serde::Deserialize)]
struct Listed {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    html_url: String,
    #[serde(default)]
    body: Option<String>,
}

impl Listed {
    fn release(self) -> Option<Release> {
        let version = Version::parse(&self.tag_name)?;
        let protocol_version = self.body.as_deref().and_then(protocol_version_in);
        Some(Release {
            version,
            notes: self.html_url,
            protocol_version,
        })
    }
}

/// The Protocol Version a Release's notes say it speaks: release.sh writes
/// "Protocol Version: `tooob/0`." near their top.
fn protocol_version_in(notes: &str) -> Option<String> {
    let after = notes.split("Protocol Version: `").nth(1)?;
    Some(after.split('`').next()?.to_string())
}

impl Releases {
    /// GitHub's, or the stand-in `TOOOB_RELEASES_URL` names (tests use it): its Releases API at
    /// `repos/Tooobs/tooob/releases` and its downloads at `Tooobs/tooob/releases/download/`.
    pub fn from_env() -> Result<Releases> {
        let (api, downloads) = match std::env::var("TOOOB_RELEASES_URL") {
            Ok(base) => {
                let base: reqwest::Url = base.parse().context("in TOOOB_RELEASES_URL")?;
                (base.clone(), base)
            }
            Err(_) => (
                "https://api.github.com/".parse()?,
                "https://github.com/".parse()?,
            ),
        };
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent(format!("tooob/{}", Version::this()))
            .build()?;
        Ok(Releases {
            api,
            downloads,
            http,
        })
    }

    async fn get(&self, url: reqwest::Url) -> Result<Option<reqwest::Response>> {
        let answer = self
            .http
            .get(url)
            .header("accept", "application/vnd.github+json")
            .send()
            .await
            .map_err(|e| unreachable(&e))?;
        match answer.status() {
            s if s.is_success() => Ok(Some(answer)),
            reqwest::StatusCode::NOT_FOUND => Ok(None),
            s => bail!("GitHub's Releases of tooob answered {s}: try again later"),
        }
    }

    /// The newest final Release: never a draft or a test Release.
    pub async fn newest(&self) -> Result<Release> {
        let url = self
            .api
            .join(&format!("repos/{REPO}/releases?per_page=100"))?;
        let listed: Vec<Listed> = match self.get(url).await? {
            Some(answer) => parsed(answer).await?,
            None => Vec::new(),
        };
        listed
            .into_iter()
            .filter(|l| !l.draft && !l.prerelease)
            .filter_map(Listed::release)
            .filter(|r| !r.version.is_test())
            .max_by(|a, b| a.version.cmp(&b.version))
            .context("no Release of tooob is published yet")
    }

    /// The Release numbered `version`, test Releases included; None if there's none.
    pub async fn exact(&self, version: &Version) -> Result<Option<Release>> {
        let url = self
            .api
            .join(&format!("repos/{REPO}/releases/tags/v{version}"))?;
        let Some(answer) = self.get(url).await? else {
            return Ok(None);
        };
        let listed: Listed = parsed(answer).await?;
        Ok(listed.release().filter(|r| r.version == *version))
    }

    /// Where `version`'s own installer is: by its tag, never `latest`.
    pub fn installer(&self, version: &Version) -> Result<reqwest::Url> {
        Ok(self.downloads.join(&format!(
            "{REPO}/releases/download/v{version}/tooob-installer.sh"
        ))?)
    }
}

/// What installing a Release would do to this box's Hosts' Relays: whether each serves a Bootstrap
/// it works with (ADR 0017, ADR 0020).
pub enum Relays {
    /// Every Relay it could check serves a Bootstrap it works with; a warning for each it couldn't.
    Fine { warnings: Vec<String> },
    /// The Release speaks `speaks`, and these Relays serve a Bootstrap that speaks another: each
    /// line names their Hosts, the Relay and its Bootstrap.
    Mismatch {
        speaks: String,
        stranded: Vec<String>,
        warnings: Vec<String>,
    },
    /// Which Protocol Version it speaks can't be told, and there are Relays to check.
    CantTell,
}

/// Check each Relay this box's Hosts name, running or not, against what `release` speaks. A
/// Relay that can't be reached isn't a Bootstrap Mismatch: what it serves is unknown, which is a
/// warning. One that serves no Bootstrap is a mismatch already, whatever the Release.
pub async fn check_relays(home: &crate::hosts::Home, release: &Release) -> Relays {
    use crate::bootstrap::{self, Problem};
    let mut relays: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    for host in home.hosts() {
        if host.settings.relay != crate::hosts::NO_RELAY {
            relays
                .entry(host.settings.relay)
                .or_default()
                .push(host.name);
        }
    }
    if relays.is_empty() {
        return Relays::Fine {
            warnings: Vec::new(),
        };
    }
    let Some(speaks) = release.speaks() else {
        return Relays::CantTell;
    };
    let (mut stranded, mut warnings) = (Vec::new(), Vec::new());
    for (relay, hosts) in relays {
        let serves = match bootstrap::check(&relay).await {
            Ok(serves) | Err(Problem::Mismatch(serves)) => serves,
            Err(Problem::NoBootstrap) => continue,
            Err(Problem::Unreachable(why)) => {
                warnings.push(format!(
                    "can't check {}'s Relay {relay}, so can't tell whether tooob {} works with its \
                     Bootstrap ({why})",
                    hosts.join(", "),
                    release.version
                ));
                continue;
            }
        };
        if serves.protocol_version != speaks {
            stranded.push(format!(
                "{}: Relay {relay} serves Bootstrap {serves}",
                hosts.join(", ")
            ));
        }
    }
    match stranded.is_empty() {
        true => Relays::Fine { warnings },
        false => Relays::Mismatch {
            speaks,
            stranded,
            warnings,
        },
    }
}

/// Set in tooob's container image (#12, #13): a tooob there is upgraded by pulling a new image.
pub const IN_CONTAINER: &str = "TOOOB_IN_CONTAINER";

/// Refuse to install `release` over a `tooob` that upgrade doesn't replace: one in a container,
/// or one that isn't `~/.local/bin/tooob`, where the installer puts it (built from source, or
/// moved). What to do instead, when it's refused.
pub fn replaceable(releases: &Releases, release: &Release) -> Result<()> {
    let version = &release.version;
    if std::env::var_os(IN_CONTAINER).is_some_and(|v| !v.is_empty()) {
        bail!(
            "this tooob runs in a container, and upgrade replaces only ~/.local/bin/tooob: pull \
             tooob {version}'s image instead"
        );
    }
    let this = std::env::current_exe()?;
    let installed = crate::autostart::installed_tooob()?;
    let same = match (
        std::fs::canonicalize(&this),
        std::fs::canonicalize(&installed),
    ) {
        (Ok(this), Ok(installed)) => this == installed,
        _ => false,
    };
    if !same {
        bail!(
            "this tooob is {}, not ~/.local/bin/tooob where the installer puts it, so upgrade \
             won't replace it: install tooob {version} by hand with\n  curl --proto '=https' \
             --tlsv1.2 -LsSf {} | sh",
            this.display(),
            releases.installer(version)?
        );
    }
    Ok(())
}

/// Install `release` by running its own installer, as `curl … | sh` does by hand, into
/// `~/.local/bin/tooob` (ADR 0014): the installer's own settings for putting it elsewhere are
/// left out. Its output goes to stderr. Then checks the installed `tooob` says it's `release`.
pub async fn install(releases: &Releases, release: &Release) -> Result<()> {
    use std::io::Write;
    let url = releases.installer(&release.version)?;
    let script = match releases.get(url.clone()).await? {
        Some(answer) => answer.bytes().await.map_err(|e| unreachable(&e))?,
        None => bail!("tooob {} has no installer at {url}", release.version),
    };
    let mut sh = std::process::Command::new("/bin/sh")
        .arg("-s")
        .env_remove("TOOOB_INSTALL_DIR")
        .env_remove("CARGO_DIST_FORCE_INSTALL_DIR")
        .env_remove("TOOOB_UNMANAGED_INSTALL")
        .stdin(std::process::Stdio::piped())
        .stdout(std::io::stderr())
        .spawn()
        .context("can't run /bin/sh for the installer")?;
    sh.stdin.take().unwrap().write_all(&script)?;
    let status = sh.wait()?;
    if !status.success() {
        bail!(
            "tooob {}'s installer failed ({status}): nothing more was changed",
            release.version
        );
    }
    let tooob = crate::autostart::installed_tooob()?;
    let says = std::process::Command::new(&tooob)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .output()
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .unwrap_or_default();
    if says.split_whitespace().nth(1) != Some(&release.version.to_string()) {
        bail!(
            "tooob {}'s installer ran, but {} says {:?}",
            release.version,
            tooob.display(),
            says
        );
    }
    Ok(())
}

/// Whether a running Host's lookup for a newer Release is off: `TOOOB_NO_UPDATE_CHECK=1`.
pub fn lookups_off() -> bool {
    std::env::var("TOOOB_NO_UPDATE_CHECK").is_ok_and(|v| !v.is_empty() && v != "0")
}

/// What a running Host and `tooob ls` say of a newer Release.
pub fn is_out(version: &Version) -> String {
    format!("tooob {version} is out: tooob upgrade")
}

/// The newest final Release last looked up, as kept in the box's state.
#[derive(serde::Serialize, serde::Deserialize)]
struct Looked {
    /// When, in seconds since 1970.
    at: u64,
    release: String,
    notes: String,
}

/// Where the newest Release last looked up is kept, in the folder `state` (a Home's).
fn looked_file(state: &Path) -> PathBuf {
    state.join("newest-release.json")
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn read_looked(state: &Path) -> Option<Looked> {
    serde_json::from_slice(&std::fs::read(looked_file(state)).ok()?).ok()
}

/// A Release newer than this tooob, as last looked up on this box: what `tooob ls` says, without
/// asking GitHub. None when there's none, or the lookup is off.
pub fn newer_known(state: &Path) -> Option<Version> {
    if lookups_off() {
        return None;
    }
    let version = Version::parse(&read_looked(state)?.release)?;
    (version > Version::this()).then_some(version)
}

/// The newest final Release, as looked up at most once each `every` for the whole box: several
/// running Hosts take turns at a lock, and the first to find the last answer stale looks it up and
/// keeps it in `state`. A lookup that fails leaves the last answer, and is tried again next time.
async fn newest_looked_up(state: &Path, every: Duration) -> Option<Version> {
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(state.join("newest-release.lock"))
        .ok()?;
    // Held until this returns: the lock goes with the file.
    let lock = tokio::task::spawn_blocking(move || {
        use std::os::unix::io::AsRawFd;
        // SAFETY: flock only reads the descriptor, which `lock` keeps open.
        let taken = unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) } == 0;
        taken.then_some(lock)
    })
    .await
    .ok()??;
    let last = read_looked(state);
    let fresh = last
        .as_ref()
        .is_some_and(|l| now().saturating_sub(l.at) < every.as_secs());
    if !fresh && let Ok(newest) = async { Releases::from_env()?.newest().await }.await {
        let looked = Looked {
            at: now(),
            release: newest.version.to_string(),
            notes: newest.notes,
        };
        if let Ok(json) = serde_json::to_vec(&looked) {
            let _ = crate::save_privately(&looked_file(state), &json);
        }
        drop(lock);
        return Some(newest.version);
    }
    drop(lock);
    Version::parse(&last?.release)
}

/// While the Host named `name` runs, look for a newer Release each `every` (at most once each
/// `every` for the whole box: see `newest_looked_up`), and say once in its log when one is out.
/// Stops when dropped.
pub fn keep_looking(name: &str, state: &Path, every: Duration) -> Looking {
    let (name, state) = (name.to_string(), state.to_path_buf());
    // Often enough to notice another Host's answer, or the end of a spell offline.
    let nap = every.min(Duration::from_secs(3600));
    Looking(tokio::spawn(async move {
        let mut said: Option<Version> = None;
        loop {
            if let Some(newest) = newest_looked_up(&state, every).await
                && newest > Version::this()
                && said.as_ref() != Some(&newest)
            {
                eprintln!("tooob: {name}: {}", is_out(&newest));
                said = Some(newest);
            }
            tokio::time::sleep(nap).await;
        }
    }))
}

/// A running Host's lookups for a newer Release; they end when this is dropped.
pub struct Looking(tokio::task::JoinHandle<()>);

impl Drop for Looking {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// What GitHub answered, as JSON.
async fn parsed<T: serde::de::DeserializeOwned>(answer: reqwest::Response) -> Result<T> {
    let body = answer.bytes().await.map_err(|e| unreachable(&e))?;
    serde_json::from_slice(&body).context("GitHub's answer about tooob's Releases can't be read")
}

/// GitHub couldn't be reached, and why: reqwest's own message only says it failed.
fn unreachable(e: &dyn std::error::Error) -> anyhow::Error {
    let mut why = e.to_string();
    let mut source = e.source();
    while let Some(e) = source {
        why = format!("{why}: {e}");
        source = e.source();
    }
    anyhow::anyhow!("can't reach GitHub to look for tooob's Releases ({why})")
}
