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
# `remove` and `list`; a running tooob-relay picks up a change within a second.\n";

/// The Enrolled Keys, each with its label, as the file holds them.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct List {
    keys: BTreeMap<String, Entry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Entry {
    key: EndpointId,
    label: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct File {
    #[serde(default, rename = "enrolled")]
    enrolled: Vec<Entry>,
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
        let mut list = List::default();
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
