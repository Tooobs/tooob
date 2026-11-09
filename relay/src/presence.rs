//! Which keys are connected to a running Relay now, and when each was last seen, as its access
//! check sees connections open and close (#133). Kept in memory: a restart forgets it.
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};

use iroh_base::EndpointId;
use iroh_relay::server::ConnectionId;

use crate::enrolled::now;

#[derive(Debug, Default, Clone)]
pub(crate) struct Presence(Arc<Mutex<HashMap<EndpointId, Seen>>>);

#[derive(Debug, Default)]
struct Seen {
    open: HashSet<ConnectionId>,
    /// Unix seconds the last connection closed.
    last: u64,
}

/// Where a key stands with this Relay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LastSeen {
    Now,
    /// Unix seconds.
    At(u64),
    Never,
}

impl Presence {
    pub(crate) fn connected(&self, key: EndpointId, id: ConnectionId) {
        self.0
            .lock()
            .unwrap()
            .entry(key)
            .or_default()
            .open
            .insert(id);
    }

    /// A connection of `key`'s closed. A key that isn't enrolled (`keep` false) is forgotten once
    /// it has none open, so an open Relay's strangers don't pile up.
    pub(crate) fn disconnected(&self, key: EndpointId, id: ConnectionId, keep: bool) {
        let mut all = self.0.lock().unwrap();
        let seen = all.entry(key).or_default();
        seen.open.remove(&id);
        seen.last = now();
        if !keep && seen.open.is_empty() {
            all.remove(&key);
        }
    }

    pub(crate) fn of(&self, key: EndpointId) -> LastSeen {
        match self.0.lock().unwrap().get(&key) {
            Some(s) if !s.open.is_empty() => LastSeen::Now,
            Some(s) => LastSeen::At(s.last),
            None => LastSeen::Never,
        }
    }
}
