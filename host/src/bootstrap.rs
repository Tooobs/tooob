//! A Host's Relay serves the Bootstrap its Visitors' browsers load (ADR 0020), and the two work
//! together only if that Bootstrap speaks this tooob's Protocol Version (ADR 0017). Its Bundle says
//! which, at `/__tooob/version.json` on the Relay's domain. tooob checks it when a Host starts, and
//! now and then while it runs.
use crate::PROTOCOL_VERSION;
use crate::control::Control;
use crate::hosts::RUN_YOUR_OWN;
use std::{fmt, time::Duration};

/// Where a Relay's Bootstrap says which Release and Protocol Version it is.
pub const VERSION_JSON: &str = "/__tooob/version.json";

/// Which Release a Relay's Bootstrap Bundle is, and the Protocol Version it speaks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootstrapVersion {
    pub release: String,
    pub protocol_version: String,
}

/// Why a Relay's Bootstrap won't do for this tooob's Hosts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    /// The Relay couldn't be reached to ask, and why.
    Unreachable(String),
    /// The Relay answered, but with no Bootstrap: nothing at its version.json, or something that
    /// isn't a Bundle's.
    NoBootstrap,
    /// Its Bootstrap speaks another Protocol Version.
    Mismatch(BootstrapVersion),
}

impl Problem {
    /// What's wrong with the Relay at `relay`, why it matters, and what to do.
    pub fn at(&self, relay: &str) -> String {
        format!("its Relay {relay} {self}")
    }
}

/// What a running Host last found its Relay's Bootstrap to be: what `tooob ls` and `tooob show`
/// say.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Found {
    /// Not yet known: the Relay hasn't been reached since the Host started.
    #[default]
    Unknown,
    /// A Bootstrap this tooob works with: `0.1.0 (tooob/0)`.
    Serves(String),
    /// A bootstrap mismatch: another Protocol Version, or no Bootstrap at all; and why.
    Mismatch(String),
}

impl Found {
    pub fn is_mismatch(&self) -> bool {
        matches!(self, Found::Mismatch(_))
    }
}

/// As `tooob show` says it.
impl fmt::Display for Found {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Found::Unknown => write!(f, "unknown: its Relay hasn't been reached to check it yet"),
            Found::Serves(version) => write!(f, "{version}"),
            Found::Mismatch(why) => write!(f, "bootstrap mismatch: {why}"),
        }
    }
}

/// Judge what a Relay answered for `/__tooob/version.json` (its HTTP status and body).
pub fn judge(status: u16, body: &[u8]) -> Result<BootstrapVersion, Problem> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Version {
        release: String,
        protocol_version: String,
    }
    if !(200..300).contains(&status) {
        return Err(Problem::NoBootstrap);
    }
    let version: Version = serde_json::from_slice(body).map_err(|_| Problem::NoBootstrap)?;
    let version = BootstrapVersion {
        release: version.release,
        protocol_version: version.protocol_version,
    };
    if version.protocol_version != PROTOCOL_VERSION {
        return Err(Problem::Mismatch(version));
    }
    Ok(version)
}

/// Ask the Relay at `relay` (a Relay's URL) for its Bootstrap's version, and judge it.
pub async fn check(relay: &str) -> Result<BootstrapVersion, Problem> {
    // reqwest's own message only says it failed: the reason (DNS, refused, TLS) is its source.
    let unreachable = |e: &dyn std::error::Error| {
        let mut why = e.to_string();
        let mut source = e.source();
        while let Some(e) = source {
            why = format!("{why}: {e}");
            source = e.source();
        }
        Problem::Unreachable(why)
    };
    let url = reqwest::Url::parse(relay)
        .and_then(|url| url.join(VERSION_JSON))
        .map_err(|e| unreachable(&e))?;
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| unreachable(&e))?;
    let answer = http.get(url).send().await.map_err(|e| unreachable(&e))?;
    let status = answer.status().as_u16();
    let body = answer.bytes().await.map_err(|e| unreachable(&e))?;
    judge(status, &body)
}

/// What a running Host knows of its Relay's Bootstrap from one check to the next, and what's
/// worth logging: only a change in what the Relay serves, and the start of a spell it can't be
/// reached, which changes nothing it found.
pub struct Watch {
    relay: String,
    found: Found,
    /// The last Bootstrap it served that this tooob works with.
    good: Option<BootstrapVersion>,
    /// Whether the last check couldn't reach the Relay.
    unreachable: bool,
}

impl Watch {
    /// From the check made as the Host started (a Relay it couldn't reach is warned of then).
    pub fn new(relay: &str, first: &Result<BootstrapVersion, Problem>) -> Watch {
        let mut watch = Watch {
            relay: relay.to_string(),
            found: Found::Unknown,
            good: None,
            unreachable: false,
        };
        watch.update(first.clone());
        watch
    }

    pub fn found(&self) -> Found {
        self.found.clone()
    }

    /// Take in one check; what to log, if anything.
    pub fn update(&mut self, now: Result<BootstrapVersion, Problem>) -> Option<String> {
        let relay = &self.relay;
        let (found, said) = match now {
            Err(problem @ Problem::Unreachable(_)) => {
                let started = !std::mem::replace(&mut self.unreachable, true);
                return started.then(|| format!("warning: {}", problem.at(relay)));
            }
            Ok(version) => {
                let said = match &self.good {
                    None => format!("its Relay {relay} serves Bootstrap {version}"),
                    Some(good) if *good == version => {
                        format!("its Relay {relay} serves Bootstrap {version} again")
                    }
                    Some(_) => format!("its Relay {relay} now serves Bootstrap {version}"),
                };
                let found = Found::Serves(version.to_string());
                let unchanged = self.good.as_ref() == Some(&version) && self.found == found;
                self.good = Some(version);
                (found, (!unchanged).then_some(said))
            }
            Err(problem) => {
                let found = Found::Mismatch(problem.at(relay));
                let said = format!(
                    "warning: {}; Visitors already connected keep working",
                    problem.at(relay)
                );
                let unchanged = self.found == found;
                (found, (!unchanged).then_some(said))
            }
        };
        self.unreachable = false;
        self.found = found;
        said
    }
}

/// Check the Relay's Bootstrap every `every` while the Host named `name` runs, logging what
/// changes and telling `control`, for `tooob ls` and `tooob show`. The Host keeps serving
/// whatever is found: Visitors already connected keep working. Stops when dropped.
pub fn keep_checking(name: &str, mut watch: Watch, control: Control, every: Duration) -> Checking {
    let name = name.to_string();
    control.found_bootstrap(watch.found());
    Checking(tokio::spawn(async move {
        loop {
            tokio::time::sleep(every).await;
            if let Some(said) = watch.update(check(&watch.relay).await) {
                eprintln!("tooob: {name}: {said}");
            }
            control.found_bootstrap(watch.found());
        }
    }))
}

/// A running Host's checks of its Relay's Bootstrap; they end when this is dropped.
pub struct Checking(tokio::task::JoinHandle<()>);

impl Drop for Checking {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// `0.1.0 (tooob/0)`: its Release and the Protocol Version it speaks.
impl fmt::Display for BootstrapVersion {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{} ({})", self.release, self.protocol_version)
    }
}

/// What's wrong, why it matters, and what to do, after "its Relay URL …".
impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Problem::Unreachable(why) => write!(
                f,
                "can't be reached to check its Bootstrap ({why}), and Visitors' browsers load \
                 their Bootstrap from it: check this box's network, or use another Relay"
            ),
            Problem::NoBootstrap => write!(
                f,
                "serves no Bootstrap (nothing at {VERSION_JSON}), so Visitors' browsers won't \
                 connect: use another Relay ({RUN_YOUR_OWN})"
            ),
            Problem::Mismatch(theirs) => write!(
                f,
                "serves Bootstrap {theirs}, and this tooob speaks {PROTOCOL_VERSION}, so \
                 Visitors' browsers won't connect: use another Relay, or a tooob Release that \
                 speaks {}",
                theirs.protocol_version
            ),
        }
    }
}
