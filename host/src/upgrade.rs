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
pub struct ReleaseNumber {
    numbers: [u64; 3],
    pre: Pre,
}

/// What comes after a Release number's `X.Y.Z`, if anything.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Pre {
    Final,
    /// A test Release's `rc.N`.
    Rc(u64),
    /// A build of tooob whose Cargo version has another label (`0.2.0-dev`): never published,
    /// so only ever this tooob's, and taken for a test build.
    Build(String),
}

/// A number in a Release's: digits only, with no leading zero (`0` itself aside).
fn number(text: &str) -> Option<u64> {
    let digits = !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit());
    let leading_zero = text.len() > 1 && text.starts_with('0');
    (digits && !leading_zero).then(|| text.parse().ok())?
}

/// `X.Y.Z`'s three numbers.
fn numbers(text: &str) -> Option<[u64; 3]> {
    let numbers: Vec<u64> = text.split('.').map(number).collect::<Option<_>>()?;
    numbers.try_into().ok()
}

impl ReleaseNumber {
    /// `0.2.0`, `v0.2.0` or `0.2.0-rc.1`; None for anything else.
    pub fn parse(text: &str) -> Option<ReleaseNumber> {
        let text = text.strip_prefix('v').unwrap_or(text);
        let (numbers_text, pre) = match text.split_once('-') {
            Some((numbers, rc)) => (numbers, Pre::Rc(number(rc.strip_prefix("rc.")?)?)),
            None => (text, Pre::Final),
        };
        Some(ReleaseNumber {
            numbers: numbers(numbers_text)?,
            pre,
        })
    }

    /// This tooob's Release; a build whose Cargo version isn't a Release's is a test build.
    pub fn this() -> ReleaseNumber {
        let cargo = env!("CARGO_PKG_VERSION");
        ReleaseNumber::parse(cargo).unwrap_or_else(|| {
            let (numbers_text, label) = cargo.split_once('-').unwrap_or((cargo, ""));
            ReleaseNumber {
                numbers: numbers(numbers_text).unwrap_or_default(),
                pre: Pre::Build(label.to_string()),
            }
        })
    }

    /// Whether it's a test Release (or a test build).
    pub fn is_test(&self) -> bool {
        self.pre != Pre::Final
    }

    /// Whether it's in the same minor Release series as `other` (`0.2.x`), so speaks the same
    /// Protocol Version: only a minor Release changes it (ADR 0017).
    pub fn same_minor(&self, other: &ReleaseNumber) -> bool {
        self.numbers[..2] == other.numbers[..2]
    }
}

impl Ord for ReleaseNumber {
    /// A test Release comes before its final one.
    fn cmp(&self, other: &ReleaseNumber) -> Ordering {
        let rank = |v: &ReleaseNumber| {
            let rc = match v.pre {
                Pre::Rc(n) => n,
                _ => 0,
            };
            (v.numbers, v.pre == Pre::Final, rc)
        };
        rank(self).cmp(&rank(other))
    }
}

impl PartialOrd for ReleaseNumber {
    fn partial_cmp(&self, other: &ReleaseNumber) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for ReleaseNumber {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let [major, minor, patch] = self.numbers;
        write!(f, "{major}.{minor}.{patch}")?;
        match &self.pre {
            Pre::Final => Ok(()),
            Pre::Rc(n) => write!(f, "-rc.{n}"),
            Pre::Build(label) => write!(f, "-{label}"),
        }
    }
}

/// One published Release.
#[derive(Debug, Clone)]
pub struct Release {
    pub number: ReleaseNumber,
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
            self.number
                .same_minor(&ReleaseNumber::this())
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
        let number = ReleaseNumber::parse(&self.tag_name)?;
        let protocol_version = self.body.as_deref().and_then(protocol_version_in);
        Some(Release {
            number,
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
    /// `repos/Tooobs/tooob/releases` and its downloads at `Tooobs/tooob/releases/download/`. The
    /// installer `upgrade` runs comes from there, trusted as the documented `curl … | sh` is:
    /// fetched by its tag, over https (ADR 0024). So a stand-in must be https too, or on this
    /// machine.
    pub fn from_env() -> Result<Releases> {
        let (api, downloads) = match std::env::var("TOOOB_RELEASES_URL") {
            Ok(base) => {
                let base: reqwest::Url = base.parse().context("in TOOOB_RELEASES_URL")?;
                let host = base.host_str().unwrap_or_default();
                let here = host == "localhost"
                    || host
                        .trim_start_matches('[')
                        .trim_end_matches(']')
                        .parse::<std::net::IpAddr>()
                        .is_ok_and(|ip| ip.is_loopback());
                if !(base.scheme() == "https" || base.scheme() == "http" && here) {
                    bail!("TOOOB_RELEASES_URL must be https://, or http:// to this machine");
                }
                (base.clone(), base)
            }
            Err(_) => (
                "https://api.github.com/".parse()?,
                "https://github.com/".parse()?,
            ),
        };
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent(format!("tooob/{}", ReleaseNumber::this()))
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
            .filter(|r| !r.number.is_test())
            .max_by(|a, b| a.number.cmp(&b.number))
            .context("no Release of tooob is published yet")
    }

    /// The Release numbered `number`, test Releases included; None if there's none.
    pub async fn exact(&self, number: &ReleaseNumber) -> Result<Option<Release>> {
        let url = self
            .api
            .join(&format!("repos/{REPO}/releases/tags/v{number}"))?;
        let Some(answer) = self.get(url).await? else {
            return Ok(None);
        };
        let listed: Listed = parsed(answer).await?;
        Ok(listed.release().filter(|r| r.number == *number))
    }

    /// Where `number`'s own installer is: by its tag, never `latest`.
    pub fn installer(&self, number: &ReleaseNumber) -> Result<reqwest::Url> {
        Ok(self.downloads.join(&format!(
            "{REPO}/releases/download/v{number}/tooob-installer.sh"
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
/// warning. One that serves no Bootstrap is a mismatch already, whatever the Release: a warning
/// too.
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
            Err(problem @ Problem::NoBootstrap) => {
                warnings.push(format!(
                    "{}'s Relay {relay} serves no Bootstrap, so it's on a Bootstrap Mismatch \
                     whichever Release it runs: {problem}",
                    hosts.join(", ")
                ));
                continue;
            }
            Err(Problem::Unreachable(why)) => {
                warnings.push(format!(
                    "can't check {}'s Relay {relay}, so can't tell whether tooob {} works with its \
                     Bootstrap ({why})",
                    hosts.join(", "),
                    release.number
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

fn in_container() -> bool {
    std::env::var_os(IN_CONTAINER).is_some_and(|v| !v.is_empty())
}

/// Refuse to install `release` over a `tooob` that upgrade doesn't replace: one in a container,
/// or one that isn't `~/.local/bin/tooob`, where the installer puts it (built from source, or
/// moved). What to do instead, when it's refused.
pub fn replaceable(releases: &Releases, release: &Release) -> Result<()> {
    let version = &release.number;
    if in_container() {
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
/// left out. It's run from a file in a folder of its own, removed after, so it reads nothing of
/// itself from its input. Its output goes to stderr. Then checks the installed `tooob` says it's
/// `release`.
pub async fn install(releases: &Releases, release: &Release) -> Result<()> {
    let number = &release.number;
    let url = releases.installer(number)?;
    let script = match releases.get(url.clone()).await? {
        Some(answer) => answer.bytes().await.map_err(|e| unreachable(&e))?,
        None => bail!("tooob {number} has no installer at {url}"),
    };
    let dir = Scratch::new()?;
    let file = dir.0.join("tooob-installer.sh");
    tokio::fs::write(&file, &script).await?;
    let status = tokio::process::Command::new("/bin/sh")
        .arg(&file)
        .env_remove("TOOOB_INSTALL_DIR")
        .env_remove("CARGO_DIST_FORCE_INSTALL_DIR")
        .env_remove("TOOOB_UNMANAGED_INSTALL")
        .stdin(std::process::Stdio::null())
        .stdout(std::io::stderr())
        .status()
        .await
        .context("can't run /bin/sh for the installer")?;
    if !status.success() {
        bail!("tooob {number}'s installer failed ({status}): nothing more was changed");
    }
    let tooob = crate::autostart::installed_tooob()?;
    let says = tokio::process::Command::new(&tooob)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .unwrap_or_default();
    if says.split_whitespace().nth(1) != Some(&number.to_string()) {
        bail!(
            "tooob {number}'s installer ran, but {} says {says:?}",
            tooob.display()
        );
    }
    Ok(())
}

/// A folder of its own in the temporary folder, only this user's; removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Result<Scratch> {
        use std::os::unix::fs::DirBuilderExt;
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let dir =
            std::env::temp_dir().join(format!("tooob-upgrade-{}-{nanos}", std::process::id()));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&dir)
            .with_context(|| format!("can't make {}", dir.display()))?;
        Ok(Scratch(dir))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Whether a running Host's release check, its lookup for a newer Release, is off:
/// `TOOOB_NO_UPDATE_CHECK=1`.
pub fn release_check_off() -> bool {
    std::env::var("TOOOB_NO_UPDATE_CHECK").is_ok_and(|v| !v.is_empty() && v != "0")
}

/// What a running Host and `tooob ls` say of a newer Release: in a container, which `upgrade`
/// won't replace, to pull the new image.
pub fn is_out(number: &ReleaseNumber) -> String {
    match in_container() {
        true => format!("tooob {number} is out: pull the new image"),
        false => format!("tooob {number} is out: tooob upgrade"),
    }
}

/// The box's last release check, as kept in its state.
#[derive(serde::Serialize, serde::Deserialize)]
struct Checked {
    /// When, in seconds since 1970, whether it reached GitHub or not.
    at: u64,
    /// The newest final Release it, or an earlier one, found; None if none has yet.
    #[serde(default)]
    newest: Option<String>,
}

/// Where the box's last release check is kept, in the folder `state` (a Home's).
fn checked_file(state: &Path) -> PathBuf {
    state.join("newest-release.json")
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn last_checked(state: &Path) -> Option<Checked> {
    serde_json::from_slice(&std::fs::read(checked_file(state)).ok()?).ok()
}

/// A Release newer than this tooob, as the box's last release check found: what `tooob ls`
/// says, without asking GitHub. None when there's none, or the release check is off.
pub fn newer_release_known(state: &Path) -> Option<ReleaseNumber> {
    if release_check_off() {
        return None;
    }
    let number = ReleaseNumber::parse(&last_checked(state)?.newest?)?;
    (number > ReleaseNumber::this()).then_some(number)
}

/// The newest final Release, as a release check finds it at most once each `every` for the whole
/// box, whether it reaches GitHub or not: several running Hosts take turns at a lock, and the
/// first to find the last check stale checks again, keeping the newest found before when it
/// can't reach GitHub. A check from the future (the clock went back) is stale. The lock is a
/// record lock, as a Host's state has (`hosts::record_lock`): each Host is a process of its own.
async fn newest_checked(state: &Path, every: Duration) -> Option<ReleaseNumber> {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(state.join("newest-release.lock"))
        .ok()?;
    // Held until this returns: released when the file closes.
    let _lock = tokio::task::spawn_blocking(move || {
        crate::hosts::record_lock(&file, libc::F_SETLKW).map(|_| file)
    })
    .await
    .ok()?
    .ok()?;
    let last = last_checked(state);
    let now = now();
    let fresh = last
        .as_ref()
        .is_some_and(|l| l.at <= now && now - l.at < every.as_secs());
    let newest = match (fresh, last) {
        (true, Some(last)) => last.newest,
        (_, last) => {
            let found = async { Releases::from_env()?.newest().await }.await;
            let newest = match found {
                Ok(release) => Some(release.number.to_string()),
                Err(_) => last.and_then(|l| l.newest),
            };
            let checked = Checked {
                at: now,
                newest: newest.clone(),
            };
            if let Ok(json) = serde_json::to_vec(&checked) {
                let _ = crate::save_privately(&checked_file(state), &json);
            }
            newest
        }
    };
    ReleaseNumber::parse(&newest?)
}

/// While the Host named `name` runs, check for a newer Release each `every` (at most once each
/// `every` for the whole box: see `newest_checked`), and say once in its log when one is out.
/// `state` is its Home's state folder. Stops when dropped.
pub fn keep_checking_releases(name: &str, state: &Path, every: Duration) -> ReleaseChecks {
    let (name, state) = (name.to_string(), state.to_path_buf());
    // Often enough to notice another Host's check.
    let nap = every.min(Duration::from_secs(3600));
    ReleaseChecks(tokio::spawn(async move {
        let mut said: Option<ReleaseNumber> = None;
        loop {
            if let Some(newest) = newest_checked(&state, every).await
                && newest > ReleaseNumber::this()
                && said.as_ref() != Some(&newest)
            {
                eprintln!("tooob: {name}: {}", is_out(&newest));
                said = Some(newest);
            }
            tokio::time::sleep(nap).await;
        }
    }))
}

/// A running Host's release checks; they end when this is dropped.
pub struct ReleaseChecks(tokio::task::JoinHandle<()>);

impl Drop for ReleaseChecks {
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
