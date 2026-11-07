//! The Enrolled Keys: the file on the Relay's volume that lists them, and the forwarding rule they
//! make (ADR 0030).
use std::{
    collections::{BTreeMap, HashSet},
    io::ErrorKind,
    path::Path,
    sync::{Arc, RwLock},
};

use iroh_base::EndpointId;
use iroh_relay::server::ForwardFilter;
use serde::{Deserialize, Serialize};

/// Where the Relay image keeps its Enrolled Keys: on its `/data` volume, which survives redeploys.
pub const DEFAULT_PATH: &str = "/data/tooob-relay/enrolled.toml";

/// What the file says at its top.
const HEADER: &str = "# This Relay's Enrolled Keys (ADR 0030): it forwards a packet only if its sender or its\n\
# receiver is one of them, and everything while there are none. Edited by `tooob-relay enroll`,\n\
# `remove` and `list`; a running tooob-relay picks up a change within a second. Its Enrollment\n\
# Codes are kept only as hashes (`tooob-relay code`, `codes`, `cancel`).\n";

/// The Enrolled Keys, each with its label, as the file holds them.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct List {
    keys: BTreeMap<String, Entry>,
    codes: Vec<Code>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Entry {
    key: EndpointId,
    label: String,
}

/// An Enrollment Code as the file keeps it: only its hash, so a leaked file gives no working code.
/// Spent, cancelled and expired codes stay, so a Host presenting one learns which it was.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Code {
    sha256: String,
    label: String,
    uses_left: u32,
    /// Unix seconds.
    expires: u64,
    #[serde(default)]
    cancelled: bool,
}

impl Code {
    fn live(&self, now: u64) -> bool {
        self.uses_left > 0 && !self.cancelled && now < self.expires
    }
}

/// Why a code enrolled nothing. Its `Display` is the reason the client is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    Wrong,
    Expired,
    UsedUp,
    Cancelled,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Refusal::Wrong => "Enrollment Code wrong: this Relay made no such code",
            Refusal::Expired => "Enrollment Code expired",
            Refusal::UsedUp => "Enrollment Code used up",
            Refusal::Cancelled => "Enrollment Code cancelled",
        })
    }
}

/// The letters and digits a code is written in: 32 of them, without I, L, O and U, so each is 5
/// bits and none is mistaken for another.
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// A code's secret as typed, in any case, with or without its Relay's domain and dashes, reduced
/// to the 16 characters that are hashed.
fn normalize(code: &str) -> String {
    let secret = code.rsplit('/').next().unwrap_or("");
    secret
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

fn sha256(code: &str) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(normalize(code).as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// 80 random bits as four groups of four, such as `7Q4K-M2XD-9PHT-WB3C`.
fn fresh_secret() -> String {
    let mut bytes = [0u8; 10];
    getrandom::fill(&mut bytes).expect("no randomness");
    let mut bits: u128 = 0;
    for b in bytes {
        bits = (bits << 8) | u128::from(b);
    }
    let mut out = String::new();
    for i in 0..16 {
        if i > 0 && i % 4 == 0 {
            out.push('-');
        }
        out.push(ALPHABET[((bits >> (75 - 5 * i)) & 31) as usize] as char);
    }
    out
}

/// Now, in Unix seconds.
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct File {
    #[serde(default, rename = "enrolled")]
    enrolled: Vec<Entry>,
    #[serde(default, rename = "code")]
    codes: Vec<Code>,
}

impl List {
    /// The list in the file at `path`: empty if there's no file.
    pub fn load(path: &Path) -> Result<List, String> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(List::default()),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        let file: File = toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut list = List {
            codes: file.codes,
            ..List::default()
        };
        for entry in file.enrolled {
            list.keys.insert(entry.key.to_string(), entry);
        }
        Ok(list)
    }

    /// Write the list to `path`, whole (a reader never sees half of it), making its folder.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let fail = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| fail(&e))?;
        }
        let file = File {
            enrolled: self.keys.values().cloned().collect(),
            codes: self.codes.clone(),
        };
        let text = format!(
            "{HEADER}\n{}",
            toml::to_string(&file).map_err(|e| fail(&e))?
        );
        let temp = path.with_extension("toml.new");
        std::fs::write(&temp, text).map_err(|e| fail(&e))?;
        std::fs::rename(&temp, path).map_err(|e| fail(&e))
    }

    /// Enroll `key` as `label`, or relabel it if it's enrolled already.
    pub fn enroll(&mut self, key: EndpointId, label: &str) {
        let label = label.to_string();
        self.keys.insert(key.to_string(), Entry { key, label });
    }

    /// Remove `key`; false if it wasn't enrolled.
    pub fn remove(&mut self, key: EndpointId) -> bool {
        self.keys.remove(&key.to_string()).is_some()
    }

    /// Each Enrolled Key and its label, in the key's order.
    pub fn entries(&self) -> impl Iterator<Item = (EndpointId, &str)> {
        self.keys.values().map(|e| (e.key, e.label.as_str()))
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// Whether `other` has the same Enrolled Keys, whatever its codes.
    pub fn same_keys(&self, other: &List) -> bool {
        self.keys == other.keys
    }

    /// Make an Enrollment Code for `label`, good for `uses` Hosts until `expires` (Unix seconds).
    /// Returns its secret, which only the caller ever sees: the list keeps its hash.
    pub fn make_code(&mut self, label: &str, uses: u32, expires: u64) -> String {
        let secret = fresh_secret();
        self.codes.push(Code {
            sha256: sha256(&secret),
            label: label.to_string(),
            uses_left: uses,
            expires,
            cancelled: false,
        });
        secret
    }

    /// The codes that can still enroll a Host at `now`: label, uses left, expiry.
    pub fn live_codes(&self, now: u64) -> impl Iterator<Item = (&str, u32, u64)> {
        self.codes
            .iter()
            .filter(move |c| c.live(now))
            .map(|c| (c.label.as_str(), c.uses_left, c.expires))
    }

    /// Cancel the live code that is `code_or_label`, or every live one so labelled; how many.
    pub fn cancel(&mut self, code_or_label: &str, now: u64) -> usize {
        let hash = sha256(code_or_label);
        let mut n = 0;
        for c in self.codes.iter_mut().filter(|c| c.live(now)) {
            if c.sha256 == hash || c.label == code_or_label {
                c.cancelled = true;
                n += 1;
            }
        }
        n
    }

    /// Redeem `code` for `key` at `now`: enroll the key under the code's label and spend one use.
    /// Returns the label.
    pub fn redeem(&mut self, code: &str, key: EndpointId, now: u64) -> Result<String, Refusal> {
        let hash = sha256(code);
        let found = self
            .codes
            .iter_mut()
            .find(|c| c.sha256 == hash)
            .ok_or(Refusal::Wrong)?;
        if found.cancelled {
            return Err(Refusal::Cancelled);
        }
        if found.uses_left == 0 {
            return Err(Refusal::UsedUp);
        }
        if now >= found.expires {
            return Err(Refusal::Expired);
        }
        found.uses_left -= 1;
        let label = found.label.clone();
        self.enroll(key, &label);
        Ok(label)
    }

    fn key_set(&self) -> HashSet<EndpointId> {
        self.keys.values().map(|e| e.key).collect()
    }
}

/// The forwarding rule a running Relay follows: a packet passes if its sender or its receiver is
/// an Enrolled Key, and every packet passes while there are none (the Relay is open).
#[derive(Debug, Default, Clone)]
pub struct Rule(Arc<RwLock<HashSet<EndpointId>>>);

impl Rule {
    /// Follow `list` from now on; returns the keys it no longer has.
    pub(crate) fn follow(&self, list: &List) -> Vec<EndpointId> {
        let new = list.key_set();
        let mut keys = self.0.write().unwrap();
        let gone = keys.difference(&new).copied().collect();
        *keys = new;
        gone
    }

    pub(crate) fn is_enrolled(&self, key: EndpointId) -> bool {
        self.0.read().unwrap().contains(&key)
    }

    /// "open" while there are no Enrolled Keys; "enrolled" otherwise.
    pub fn access(&self) -> &'static str {
        if self.0.read().unwrap().is_empty() {
            "open"
        } else {
            "enrolled"
        }
    }
}

impl ForwardFilter for Rule {
    fn forwards(&self, src: EndpointId, dst: EndpointId) -> bool {
        let keys = self.0.read().unwrap();
        keys.is_empty() || keys.contains(&src) || keys.contains(&dst)
    }
}

/// "open" for a Relay with no Enrolled Keys, which forwards everything; "enrolled" otherwise.
pub fn access(list: &List) -> &'static str {
    if list.is_empty() { "open" } else { "enrolled" }
}
