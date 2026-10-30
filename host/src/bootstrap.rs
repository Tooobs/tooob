//! A Host's Relay serves the Bootstrap its Visitors' browsers load (ADR 0020), and the two work
//! together only if that Bootstrap speaks this tooob's Protocol Version (ADR 0017). Its Bundle says
//! which, at `/__tooob/version.json` on the Relay's domain.
use crate::PROTOCOL_VERSION;
use crate::hosts::RUN_YOUR_OWN;
use std::{fmt, time::Duration};

/// Where a Relay's Bootstrap says which Release and Protocol Version it is.
pub const VERSION_JSON: &str = "/__tooob/version.json";

/// What a Relay's Bootstrap says of itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bootstrap {
    pub release: String,
    pub protocol_version: String,
}

/// What a running Host last found its Relay's Bootstrap to be, for `tooob ls` and `tooob show`.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RelaysBootstrap {
    /// The Bootstrap its Relay last served: `0.1.0 (tooob/0)`.
    pub serves: Option<String>,
    /// While its Relay's Bootstrap won't do for this tooob (another Protocol Version, or none
    /// at all), why: a bootstrap mismatch.
    pub mismatch: Option<String>,
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
    Mismatch(Bootstrap),
}

/// Judge what a Relay answered for `/__tooob/version.json` (its HTTP status and body).
pub fn judge(status: u16, body: &[u8]) -> Result<Bootstrap, Problem> {
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
    let bootstrap = Bootstrap {
        release: version.release,
        protocol_version: version.protocol_version,
    };
    if bootstrap.protocol_version != PROTOCOL_VERSION {
        return Err(Problem::Mismatch(bootstrap));
    }
    Ok(bootstrap)
}

/// Ask the Relay at `relay` (a Relay's URL) for its Bootstrap, and judge it.
pub async fn check(relay: &str) -> Result<Bootstrap, Problem> {
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

/// `0.1.0 (tooob/0)`: its Release and the Protocol Version it speaks.
impl fmt::Display for Bootstrap {
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
