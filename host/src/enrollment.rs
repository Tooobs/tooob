//! Whether a Host's Relay carries it (ADR 0030, #130). A closed Relay forwards only traffic that
//! touches an Enrolled Key, and drops the rest without a word, so a Host asks
//! `/__tooob/enrolled/<its Host Key>` and tells its Owner, with the Host Key to give the Relay
//! Operator. A Relay that can't be asked is unknown, never "not enrolled": it may only be down.
//!
//! What it last found is kept in the Host's state folder, so `tooob ls` shows it for a stopped
//! Host as well as a running one.
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

/// What a Host last found of its Relay and its Host Key.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Enrollment {
    /// The Relay couldn't be asked, or didn't answer as a tooob-relay does.
    #[default]
    Unknown,
    /// It has no Enrolled Keys, so it carries everyone.
    Open,
    /// The Host Key is one of its Enrolled Keys.
    Enrolled,
    /// It's closed, and the Host Key isn't enrolled: Visitors reach the Host only directly.
    NotEnrolled,
}

impl Enrollment {
    /// Whether it's known to carry the Host; None when unknown.
    fn carried(self) -> Option<bool> {
        match self {
            Enrollment::Unknown => None,
            Enrollment::Open | Enrollment::Enrolled => Some(true),
            Enrollment::NotEnrolled => Some(false),
        }
    }
}

/// Ask the Relay at `relay` (its URL) whether it carries the Host whose Host Key is `key`.
pub async fn check(relay: &str, key: &str) -> Enrollment {
    #[derive(serde::Deserialize)]
    struct Answer {
        enrolled: bool,
        relay_access: String,
    }
    let ask = async {
        let url = reqwest::Url::parse(relay)
            .ok()?
            .join(&format!("/__tooob/enrolled/{key}"))
            .ok()?;
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .ok()?;
        let answer = http.get(url).send().await.ok()?;
        if !answer.status().is_success() {
            return None;
        }
        let answer: Answer = serde_json::from_slice(&answer.bytes().await.ok()?).ok()?;
        Some(match (answer.enrolled, answer.relay_access.as_str()) {
            (true, _) => Enrollment::Enrolled,
            (false, "open") => Enrollment::Open,
            (false, "enrolled") => Enrollment::NotEnrolled,
            _ => Enrollment::Unknown,
        })
    };
    ask.await.unwrap_or_default()
}

/// The Relay as the Owner knows it: `r.tooobs.net`, its host and any port.
pub fn named(relay: &str) -> String {
    match reqwest::Url::parse(relay) {
        Ok(url) => match (url.host_str(), url.port()) {
            (Some(host), Some(port)) => format!("{host}:{port}"),
            (Some(host), None) => host.to_string(),
            _ => relay.to_string(),
        },
        Err(_) => relay.to_string(),
    }
}

/// What to tell the Owner of a Host the Relay at `relay` doesn't carry.
pub fn not_carried(relay: &str, key: &str) -> String {
    format!(
        "{} doesn't carry this Host yet: give its Relay Operator this Host Key: {key}",
        named(relay)
    )
}

/// An Enrollment Code as its Relay Operator gives it: `r.tooobs.net/7Q4K-M2XD-9PHT-WB3C`, the
/// Relay it's for, then its secret (ADR 0030, #132).
#[derive(Debug, PartialEq, Eq)]
pub struct Code {
    /// The Relay as `named` writes it: its host and any port.
    pub relay: String,
    secret: String,
}

impl Code {
    pub fn parse(text: &str) -> Result<Code, String> {
        let bad = || {
            "an Enrollment Code reads RELAY/CODE, such as r.tooobs.net/7Q4K-M2XD-9PHT-WB3C: give it as \
             its Relay Operator gave it"
                .to_string()
        };
        let (relay, secret) = text.trim().rsplit_once('/').ok_or_else(bad)?;
        if relay.is_empty() || relay.contains('/') || secret.is_empty() {
            return Err(bad());
        }
        let relay = named(&format!("https://{relay}/"));
        Ok(Code {
            relay,
            secret: secret.to_string(),
        })
    }

    /// The Relay it's for, as a URL: the Relay a Host made with it alone uses.
    pub fn relay_url(&self) -> String {
        format!("https://{}/", self.relay)
    }

    /// Whether it's for the Relay at `relay` (a URL).
    pub fn is_for(&self, relay: &str) -> bool {
        named(relay).eq_ignore_ascii_case(&self.relay)
    }

    /// What's said of it: its Relay and the first group of its secret, never all of it.
    pub fn shown(&self) -> String {
        let start: String = self.secret.chars().take(4).collect();
        format!("{}/{start}-…", self.relay)
    }

    /// Why it isn't for the Relay at `relay`.
    pub fn not_for(&self, relay: &str) -> String {
        format!(
            "that Enrollment Code is for {}, not {}",
            self.relay,
            named(relay)
        )
    }
}

/// How long redeeming a code waits for the Relay's answer.
const REDEEM_WAIT: Duration = Duration::from_secs(20);

/// Redeem `code` at the Relay at `relay` for the Host Key `key`: connect once with it as iroh's
/// auth token, and wait for the Relay to let the Host in, enrolled, or say why not.
pub async fn redeem(
    relay: &iroh::RelayUrl,
    key: iroh::SecretKey,
    code: &Code,
) -> Result<(), String> {
    use iroh::{Endpoint, RelayConfig, RelayMap, RelayMode, Watcher, endpoint::presets};
    let map: RelayMap = RelayConfig::new(relay.clone(), None)
        .with_auth_token(code.secret.clone())
        .into();
    let endpoint = Endpoint::builder(presets::N0)
        .relay_mode(RelayMode::Custom(map))
        .secret_key(key)
        .bind()
        .await
        .map_err(|e| format!("can't redeem the Enrollment Code: {e}"))?;
    let mut status = endpoint.home_relay_status();
    let answer = tokio::time::timeout(REDEEM_WAIT, async {
        loop {
            for relay in status.get() {
                if let Some(reason) = relay.auth_denied_reason() {
                    return Err(reason.to_string());
                }
                if relay.is_connected() {
                    return Ok(());
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .unwrap_or_else(|_| {
        Err(format!(
            "{} didn't answer within {} seconds: the Enrollment Code wasn't redeemed",
            code.relay,
            REDEEM_WAIT.as_secs()
        ))
    });
    endpoint.close().await;
    answer
}

fn file(state_dir: &Path) -> PathBuf {
    state_dir.join("enrollment.json")
}

/// What the Host in `state_dir` last found; unknown if it never asked.
pub fn last_found(state_dir: &Path) -> Enrollment {
    std::fs::read(file(state_dir))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// Keep what was found, for `tooob ls`. Written whole and renamed in; a failure only loses it.
pub fn keep(state_dir: &Path, found: Enrollment) {
    if let Ok(json) = serde_json::to_vec(&found) {
        let _ = crate::save_privately(&file(state_dir), &json);
    }
}

/// What a running Host knows of its Relay's enrollment from one check to the next, and what's
/// worth logging: the Host not carried when it starts, and each change between carried and not.
/// A check that can't ask changes nothing.
pub struct Watch {
    relay: String,
    key: String,
    carried: Option<bool>,
}

impl Watch {
    pub fn new(relay: &str, key: &str) -> Watch {
        Watch {
            relay: relay.to_string(),
            key: key.to_string(),
            carried: None,
        }
    }

    /// Take in one check; what to log, if anything, and whether it's a warning.
    pub fn update(&mut self, now: Enrollment) -> Option<(bool, String)> {
        let carried = now.carried()?;
        let before = self.carried.replace(carried);
        let relay = named(&self.relay);
        match (before, carried) {
            (None, false) => Some((true, not_carried(&self.relay, &self.key))),
            (Some(true), false) => Some((
                true,
                format!(
                    "{relay} doesn't carry this Host any more: give its Relay Operator this \
                     Host Key: {}",
                    self.key
                ),
            )),
            (Some(false), true) => Some((
                false,
                match now {
                    Enrollment::Open => format!("{relay} now carries this Host: it's open"),
                    _ => format!("{relay} now carries this Host: its Host Key is enrolled"),
                },
            )),
            _ => None,
        }
    }
}

/// Check every `every` while the Host runs (and once at once), logging what changes and keeping
/// what's found for `tooob ls`. Stops when dropped.
pub fn keep_checking(mut watch: Watch, state_dir: PathBuf, every: Duration) -> Checking {
    Checking(tokio::spawn(async move {
        loop {
            let now = check(&watch.relay, &watch.key).await;
            if now != Enrollment::Unknown {
                keep(&state_dir, now);
            }
            if let Some((warning, said)) = watch.update(now) {
                let fields = crate::fields! {"enrollment" => now};
                match warning {
                    true => crate::log::event("warning", fields, &format!("warning: {said}")),
                    false => crate::log::event("enrollment", fields, &said),
                }
            }
            tokio::time::sleep(every).await;
        }
    }))
}

/// A running Host's checks of its Relay's enrollment; they end when this is dropped.
pub struct Checking(tokio::task::JoinHandle<()>);

impl Drop for Checking {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::Code;

    #[test]
    fn a_code_names_its_relay_and_is_never_shown_whole() {
        let code = Code::parse("r.tooobs.net/7Q4K-M2XD-9PHT-WB3C").unwrap();
        assert_eq!(code.relay_url(), "https://r.tooobs.net/");
        assert!(code.is_for("https://R.tooobs.net/"));
        assert!(!code.is_for("https://r.example.net/"));
        assert_eq!(code.shown(), "r.tooobs.net/7Q4K-…");
        assert!(Code::parse("7Q4K-M2XD-9PHT-WB3C").is_err());
        assert!(Code::parse("https://r.tooobs.net/7Q4K").is_err());
    }
}
