//! What a running Host's log says of its Visitors (ADR 0025): each Visit, from getting in to
//! leaving, the path it takes, and the refusals. A Visit is per Visitor, not per connection: it
//! lasts until a minute after the Visitor's last connection, so reloads, more tabs and network
//! switches in between are one Visit. Never which pages or files a Visitor opened, nor where
//! from.
use crate::fields;
use crate::log::{self, Fields};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::time::Instant;

/// How long after its Visitor's last connection a Visit ends: a minute, or as `TOOOB_VISIT_GRACE`
/// says (`1s`), which tests use.
const GRACE: Duration = Duration::from_secs(60);

/// How long a Visit, or a change of its path, waits for its path to settle before it's logged: a
/// browser's first Direct Path is up within a few seconds, or has failed by the time the Host
/// gives up on it (`direct::SETUP`). `TOOOB_DIRECT_WAIT` (`1s`) says otherwise, which tests use.
const DIRECT_WAIT: Duration = Duration::from_secs(10);

/// Which path a Visitor's traffic takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Path {
    Relay,
    Direct(Kind),
}

/// What kind of Direct Path: on the home network, or punched through NAT. Only the kind is ever
/// logged, never an address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Kind {
    Lan,
    NatPunched,
    /// Its stats didn't say.
    Unknown,
}

impl Path {
    /// `relay`, `direct · LAN`, `direct · NAT punched`.
    fn said(&self) -> &'static str {
        match self {
            Path::Relay => "relay",
            Path::Direct(Kind::Lan) => "direct · LAN",
            Path::Direct(Kind::NatPunched) => "direct · NAT punched",
            Path::Direct(Kind::Unknown) => "direct",
        }
    }

    fn fields(&self) -> Fields {
        match self {
            Path::Relay => fields! {"path" => "relay"},
            Path::Direct(Kind::Lan) => fields! {"path" => "direct", "direct" => "lan"},
            Path::Direct(Kind::NatPunched) => {
                fields! {"path" => "direct", "direct" => "nat_punched"}
            }
            Path::Direct(Kind::Unknown) => fields! {"path" => "direct"},
        }
    }
}

/// Why a device wasn't let in, as the log says it.
pub(crate) enum Refusal {
    /// A device that isn't a Visitor.
    Unknown,
    /// A Blocked Visitor, by Label.
    Blocked(String),
    /// A device that used an Invite with First-Visit Approval (or an Owner Invite), by Label.
    AwaitingApproval(String),
    /// An Invite it couldn't use: `reason` its stable code (`expired`, `already_used`,
    /// `unknown_invite`), with the Invite's short id and Label when the Host has it.
    Invite {
        reason: &'static str,
        invite: Option<(String, String)>,
    },
}

/// The Visits of one running Host.
#[derive(Clone)]
pub(crate) struct Visits(Arc<Shared>);

impl std::fmt::Debug for Visits {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Visits").finish_non_exhaustive()
    }
}

struct Shared {
    grace: Duration,
    direct_wait: Duration,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    /// Each Visit going on, by full Visitor Key.
    visits: HashMap<String, Visit>,
    /// When each device was last refused, by full Visitor Key: refused again within the grace, it
    /// isn't logged again (a reload, a device waiting for approval asking again).
    refused: HashMap<String, Instant>,
    /// The Host has stopped: its Visits have ended.
    over: bool,
}

struct Visit {
    label: String,
    first: bool,
    started: Instant,
    /// Its Visitor's connections open now.
    connections: usize,
    /// When its last connection closed.
    last_left: Instant,
    /// Its Direct Paths up now, by their id.
    directs: HashMap<u64, Kind>,
    /// The path the log last said it takes: None until its getting in is logged.
    said: Option<Path>,
    /// When its path last changed.
    path_since: Instant,
}

impl Visit {
    fn path(&self) -> Path {
        match self.directs.values().min() {
            Some(kind) => Path::Direct(*kind),
            None => Path::Relay,
        }
    }
}

impl Visits {
    /// A running Host's Visits, waiting as the environment says.
    pub(crate) fn new() -> anyhow::Result<Visits> {
        let wait = |var: &str, default: Duration| -> anyhow::Result<Duration> {
            match std::env::var(var) {
                Ok(given) => {
                    let secs =
                        crate::seconds(&given).map_err(|e| e.context(format!("in {var}")))?;
                    Ok(Duration::from_secs(secs))
                }
                Err(_) => Ok(default),
            }
        };
        Ok(Visits(Arc::new(Shared {
            grace: wait("TOOOB_VISIT_GRACE", GRACE)?,
            direct_wait: wait("TOOOB_DIRECT_WAIT", DIRECT_WAIT)?,
            state: Mutex::default(),
        })))
    }

    /// A connection that proved Visitor Key `key` (ADR 0011): nobody's Visit until it's let in.
    pub(crate) fn connection(&self, key: &str) -> Presence {
        Presence(Arc::new(Connection {
            visits: self.clone(),
            key: key.to_string(),
            joined: Mutex::new(false),
        }))
    }

    /// The Host stops: every Visit ends now, and the log says so.
    pub(crate) fn end_all(&self) {
        let mut state = self.0.state.lock().unwrap();
        state.over = true;
        let now = Instant::now();
        for (key, mut visit) in std::mem::take(&mut state.visits) {
            if visit.connections > 0 {
                visit.last_left = now;
            }
            say_got_in(&key, &mut visit);
            say_left(&key, &visit);
        }
    }

    /// Look at `key`'s Visit again `after` from now.
    fn check_after(&self, key: &str, after: Duration) {
        let (visits, key) = (self.clone(), key.to_string());
        let at = Instant::now() + after;
        tokio::spawn(async move {
            tokio::time::sleep_until(at).await;
            visits.check(&key);
        });
    }

    /// Log what has settled of `key`'s Visit: its getting in, once its path is known; a change of
    /// path that has held; its end, once the grace after its last connection is over.
    fn check(&self, key: &str) {
        let (grace, wait) = (self.0.grace, self.0.direct_wait);
        let mut state = self.0.state.lock().unwrap();
        let Some(visit) = state.visits.get_mut(key) else {
            return;
        };
        let now = Instant::now();
        if visit.connections == 0 && visit.last_left + grace <= now {
            let mut visit = state.visits.remove(key).unwrap();
            say_got_in(key, &mut visit);
            say_left(key, &visit);
            return;
        }
        match visit.said {
            // A Visit's first Direct Path settles it: no need to wait.
            None if visit.started + wait <= now || !visit.directs.is_empty() => {
                say_got_in(key, visit)
            }
            Some(said) if said != visit.path() && visit.path_since + wait <= now => {
                let path = visit.path();
                let (label, short) = (&visit.label, short(key));
                let mut fields = fields! {"label" => label, "key" => short};
                fields.extend(path.fields());
                log::event(
                    "path",
                    fields,
                    &format!("{label} ({short}) moved to {}", path.said()),
                );
                visit.said = Some(path);
            }
            _ => {}
        }
    }

    /// Change `key`'s Visit, if there is one, then look at it again when its path may have
    /// settled.
    fn change(&self, key: &str, f: impl FnOnce(&mut Visit)) {
        let mut state = self.0.state.lock().unwrap();
        let Some(visit) = state.visits.get_mut(key) else {
            return;
        };
        let before = visit.path();
        f(visit);
        if visit.path() != before {
            visit.path_since = Instant::now();
        }
        drop(state);
        self.check(key);
        self.check_after(key, self.0.direct_wait);
    }
}

/// Log that `key`'s Visitor got in, unless it's said already: on the path it takes now.
fn say_got_in(key: &str, visit: &mut Visit) {
    if visit.said.is_some() {
        return;
    }
    let path = visit.path();
    let (label, short) = (&visit.label, short(key));
    let first = if visit.first { "first" } else { "return" };
    let mut fields = fields! {"label" => label, "key" => short, "visit" => first};
    fields.extend(path.fields());
    let first = if visit.first { "first visit" } else { "return" };
    log::event(
        "got_in",
        fields,
        &format!("{label} ({short}) got in · {first} · {}", path.said()),
    );
    visit.said = Some(path);
}

/// Log that `key`'s Visitor left, and how long they were there.
fn say_left(key: &str, visit: &Visit) {
    let stayed = visit.last_left.saturating_duration_since(visit.started);
    let (label, short) = (&visit.label, short(key));
    log::event(
        "left",
        fields! {"label" => label, "key" => short, "seconds" => stayed.as_secs()},
        &format!("{label} ({short}) left after {}", for_how_long(stayed)),
    );
}

/// `40 s`, `7 min`, `2 h 5 min`.
fn for_how_long(stayed: Duration) -> String {
    match stayed.as_secs() {
        s if s < 60 => format!("{s} s"),
        s if s < 3600 => format!("{} min", s / 60),
        s => format!("{} h {} min", s / 3600, s % 3600 / 60),
    }
}

/// A Visitor Key as the log names a device: its first 8 characters, a unique prefix in any
/// household, which `tooob block` takes.
pub(crate) fn short(key: &str) -> &str {
    key.get(..8).unwrap_or(key)
}

/// One connection's part in its Visitor's Visit. Shared by the connection's streams and its Direct
/// Paths.
#[derive(Clone)]
pub(crate) struct Presence(Arc<Connection>);

struct Connection {
    visits: Visits,
    key: String,
    /// Whether it's counted in its Visitor's Visit.
    joined: Mutex<bool>,
}

/// Ids for Direct Paths, unique in this process.
static DIRECTS: AtomicU64 = AtomicU64::new(0);

impl Presence {
    /// The connection's Visitor, `label`, is let in: it's part of their Visit from now on,
    /// starting one if there's none. `first` says whether the Host has let them in before
    /// (`Access::first_visit`), only asked for a new Visit.
    pub(crate) fn let_in(&self, label: &str, first: impl FnOnce() -> bool) {
        let Connection { visits, key, .. } = &*self.0;
        let mut joined = self.0.joined.lock().unwrap();
        if *joined {
            return;
        }
        let mut state = visits.0.state.lock().unwrap();
        if state.over {
            return;
        }
        *joined = true;
        state.refused.remove(key);
        let now = Instant::now();
        let started = !state.visits.contains_key(key);
        let visit = state.visits.entry(key.clone()).or_insert_with(|| Visit {
            label: label.to_string(),
            first: first(),
            started: now,
            connections: 0,
            last_left: now,
            directs: HashMap::new(),
            said: None,
            path_since: now,
        });
        visit.connections += 1;
        drop(state);
        if started {
            visits.check_after(key, visits.0.direct_wait);
        }
    }

    /// The connection closed.
    pub(crate) fn closed(&self) {
        let mut joined = self.0.joined.lock().unwrap();
        if !std::mem::take(&mut *joined) {
            return;
        }
        let Connection { visits, key, .. } = &*self.0;
        let mut state = visits.0.state.lock().unwrap();
        if let Some(visit) = state.visits.get_mut(key) {
            visit.connections -= 1;
            if visit.connections == 0 {
                visit.last_left = Instant::now();
                drop(state);
                visits.check_after(key, visits.0.grace);
            }
        }
    }

    /// A new Direct Path for this connection; its id.
    pub(crate) fn direct_path(&self) -> u64 {
        DIRECTS.fetch_add(1, Ordering::Relaxed)
    }

    /// Direct Path `id` is up, of `kind`.
    pub(crate) fn direct_up(&self, id: u64, kind: Kind) {
        let Connection { visits, key, .. } = &*self.0;
        visits.change(key, |visit| {
            visit.directs.insert(id, kind);
        });
    }

    /// Direct Path `id` is down: failed, paused or closed.
    pub(crate) fn direct_down(&self, id: u64) {
        let Connection { visits, key, .. } = &*self.0;
        visits.change(key, |visit| {
            visit.directs.remove(&id);
        });
    }

    /// The connection's device wasn't let in: logged unless it was refused within the grace
    /// already.
    pub(crate) fn refused(&self, why: Refusal) {
        let Connection { visits, key, .. } = &*self.0;
        let mut state = visits.0.state.lock().unwrap();
        let now = Instant::now();
        let again = state
            .refused
            .insert(key.clone(), now)
            .is_some_and(|last| last + visits.0.grace > now);
        if again || state.over {
            return;
        }
        drop(state);
        let short = short(key);
        let (reason, mut fields, text) = match why {
            Refusal::Unknown => (
                "unknown_device",
                Fields::new(),
                format!("refused a device ({short}) · not invited"),
            ),
            Refusal::Blocked(label) => (
                "blocked",
                fields! {"label" => label},
                format!("refused {label} ({short}) · blocked"),
            ),
            Refusal::AwaitingApproval(label) => (
                "awaiting_approval",
                fields! {"label" => label},
                format!("{label} ({short}) is waiting for approval"),
            ),
            Refusal::Invite {
                reason,
                invite: Some((id, label)),
            } => (
                reason,
                fields! {"invite" => id, "label" => label},
                format!(
                    "refused a device ({short}) · Invite {id} for {label} · {}",
                    reason.replace('_', " ")
                ),
            ),
            Refusal::Invite {
                reason,
                invite: None,
            } => (
                reason,
                Fields::new(),
                format!("refused a device ({short}) · unknown Invite"),
            ),
        };
        fields.insert("reason".into(), reason.into());
        fields.insert("key".into(), short.into());
        log::event("refused", fields, &text);
    }
}
