//! What a running Host's log says of its Visitors (ADR 0025): each Visit, from getting in to
//! leaving, the path it takes, and the refusals. (`visit` is the other side: a Visitor's own end
//! of the protocol.) A Visit is per Visitor, however many tabs and reloads: it lasts until a
//! minute after the Visitor's last connection, so reloads, more tabs and network switches in
//! between are one Visit. Never which pages or files a Visitor opened, nor where from.
use crate::access::Access;
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
use tokio::{sync::Notify, time::Instant};

/// How long after its Visitor's last connection a Visit ends: a minute, or as `TOOOB_VISIT_GRACE`
/// says (`1s`), which tests use. A device refused within it isn't logged again.
const GRACE: Duration = Duration::from_secs(60);

/// How long a Visit, or a change of its path, waits for its path to settle before it's logged: a
/// browser's first Direct Path is up within a few seconds, or has failed by the time the Host
/// gives up on it (`direct::SETUP`). `TOOOB_DIRECT_WAIT` (`1s`) says otherwise, which tests use.
const DIRECT_WAIT: Duration = Duration::from_secs(10);

/// How long a Host may go on thinking a closed browser tab is connected: iroh gives up on a
/// silent connection after about 30 s (#101), while the tab's Direct Path goes at once. So a Visit
/// whose Direct Path went is said to have moved to the Relay only once its Visitor has been heard
/// on the Relay since, or once this long has gone with its connection still open: a tab that
/// closed is never said to have moved.
const UNNOTICED: Duration = Duration::from_secs(45);

/// How many devices refused within the grace the Host names one by one; past that, a line for
/// all the rest each grace, so a stranger minting Visitor Keys can't fill its memory or its log.
/// `TOOOB_REFUSED_CAP` says otherwise, which tests use.
const REFUSED_CAP: usize = 1000;

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
    refused_cap: usize,
    /// Its Visitors' Labels, as they are when the log names them, and whether they've been in.
    access: Arc<Mutex<Access>>,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    /// Each Visit going on, by full Visitor Key.
    visits: HashMap<String, Visit>,
    /// When each device was last logged as refused, by full Visitor Key: refused again within the
    /// grace, it isn't logged again (a reload, a device waiting for approval asking again). Never
    /// more than the cap.
    refused: HashMap<String, Instant>,
    /// Refusals past the cap since the line about them was last logged; None if there are none.
    more_refused: Option<u64>,
    /// The Host has stopped: its Visits have ended.
    over: bool,
}

struct Visit {
    /// Its Label when it started, if the Visitor has gone from the Allowlist by the time it's said.
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
    /// When its Visitor was last heard on the Relay: a stream on an iroh connection.
    heard: Instant,
    /// Tells its watch something changed.
    nudge: Arc<Notify>,
}

/// What the log is to say of a Visit.
enum Said {
    GotIn { first: bool, path: Path },
    Moved(Path),
    Left { stayed: Duration },
}

impl Visit {
    fn path(&self) -> Path {
        match self.directs.values().min() {
            Some(kind) => Path::Direct(*kind),
            None => Path::Relay,
        }
    }

    /// Its getting in, unless that's said already: on the path it takes now.
    fn got_in(&mut self, says: &mut Vec<Said>) {
        if self.said.is_none() {
            let path = self.path();
            says.push(Said::GotIn {
                first: self.first,
                path,
            });
            self.said = Some(path);
        }
    }

    /// What has settled of it at `now`, to say; when to look again (None: when it changes); and
    /// whether it's over.
    fn settle(&mut self, now: Instant, shared: &Shared) -> (Vec<Said>, Option<Instant>, bool) {
        let mut says = Vec::new();
        if self.connections == 0 {
            // Its path isn't news while nobody's connected: a tab that closed took it.
            let ends = self.last_left + shared.grace;
            if ends > now {
                return (says, Some(ends), false);
            }
            self.got_in(&mut says);
            let stayed = self.last_left.saturating_duration_since(self.started);
            says.push(Said::Left { stayed });
            return (says, None, true);
        }
        let path = self.path();
        let due = match self.said {
            // A Visit's first Direct Path settles it: no need to wait.
            None if !self.directs.is_empty() => now,
            None => self.started + shared.direct_wait,
            Some(said) if said == path => return (says, None, false),
            Some(_) if path == Path::Relay && self.heard <= self.path_since => {
                self.path_since + UNNOTICED.max(shared.direct_wait)
            }
            Some(_) => self.path_since + shared.direct_wait,
        };
        if due > now {
            return (says, Some(due), false);
        }
        match self.said {
            None => self.got_in(&mut says),
            Some(_) => {
                says.push(Said::Moved(path));
                self.said = Some(path);
            }
        }
        (says, None, false)
    }
}

impl Visits {
    /// A running Host's Visits, of the Visitors `access` has, waiting as the environment says.
    pub(crate) fn new(access: Arc<Mutex<Access>>) -> anyhow::Result<Visits> {
        let var = |var: &str| std::env::var(var).ok();
        let wait = |name: &str, default: Duration| -> anyhow::Result<Duration> {
            match var(name) {
                Some(given) => {
                    let secs =
                        crate::seconds(&given).map_err(|e| e.context(format!("in {name}")))?;
                    Ok(Duration::from_secs(secs))
                }
                None => Ok(default),
            }
        };
        let refused_cap = match var("TOOOB_REFUSED_CAP") {
            Some(cap) => cap
                .parse()
                .map_err(|e| anyhow::anyhow!("in TOOOB_REFUSED_CAP: {e}"))?,
            None => REFUSED_CAP,
        };
        Ok(Visits(Arc::new(Shared {
            grace: wait("TOOOB_VISIT_GRACE", GRACE)?,
            direct_wait: wait("TOOOB_DIRECT_WAIT", DIRECT_WAIT)?,
            refused_cap,
            access,
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
        let ended: Vec<(String, String, Vec<Said>)> = {
            let mut state = self.0.state.lock().unwrap();
            state.over = true;
            let now = Instant::now();
            std::mem::take(&mut state.visits)
                .into_iter()
                .map(|(key, mut visit)| {
                    let mut says = Vec::new();
                    visit.got_in(&mut says);
                    let last = if visit.connections > 0 {
                        now
                    } else {
                        visit.last_left
                    };
                    let stayed = last.saturating_duration_since(visit.started);
                    says.push(Said::Left { stayed });
                    (key, visit.label, says)
                })
                .collect()
        };
        for (key, label, says) in ended {
            self.say(&key, &label, says);
        }
    }

    /// Change `key`'s Visit, if there is one, and have its watch look at it again.
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
        visit.nudge.notify_one();
    }

    /// Watch `key`'s new Visit until it ends: log what settles of it, as it settles. One task a
    /// Visit, woken when it's due to look again or the Visit changes.
    fn watch(&self, key: String, nudge: Arc<Notify>) {
        let visits = self.clone();
        tokio::spawn(async move {
            loop {
                let (says, next, ended, label) = {
                    let mut state = visits.0.state.lock().unwrap();
                    let Some(visit) = state.visits.get_mut(&key) else {
                        return;
                    };
                    let (says, next, ended) = visit.settle(Instant::now(), &visits.0);
                    let label = visit.label.clone();
                    if ended {
                        state.visits.remove(&key);
                    }
                    (says, next, ended, label)
                };
                visits.say(&key, &label, says);
                if ended {
                    return;
                }
                match next {
                    Some(at) => tokio::select! {
                        _ = tokio::time::sleep_until(at) => {}
                        _ = nudge.notified() => {}
                    },
                    None => nudge.notified().await,
                }
            }
        });
    }

    /// Log what's said of `key`'s Visit, naming them by their Label now, or `label`, the one
    /// their Visit started with, if they've gone from the Allowlist. Never under the Visits' lock:
    /// the log is written to disk.
    fn say(&self, key: &str, label: &str, says: Vec<Said>) {
        if says.is_empty() {
            return;
        }
        let now = self.0.access.lock().unwrap().label(key);
        let label = now.as_deref().unwrap_or(label);
        let short = short(key);
        for said in says {
            let mut fields = who(label, key);
            let (event, text) = match said {
                Said::GotIn { first, path } => {
                    let (visit, said) = match first {
                        true => ("first", "first visit"),
                        false => ("return", "return"),
                    };
                    fields.insert("visit".into(), visit.into());
                    fields.extend(path.fields());
                    ("got_in", format!("got in · {said} · {}", path.said()))
                }
                Said::Moved(path) => {
                    fields.extend(path.fields());
                    ("path", format!("moved to {}", path.said()))
                }
                Said::Left { stayed } => {
                    fields.insert("seconds".into(), stayed.as_secs().into());
                    ("left", format!("left after {}", for_how_long(stayed)))
                }
            };
            log::event(event, fields, &format!("{label} ({short}) {text}"));
        }
    }
}

/// A Visitor as an event's fields name them: their Label and short Visitor Key.
fn who(label: &str, key: &str) -> Fields {
    fields! {"label" => label, "key" => short(key)}
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
    /// starting one if there's none.
    pub(crate) fn let_in(&self, label: &str) {
        let Connection { visits, key, .. } = &*self.0;
        let shared = &visits.0;
        let mut joined = self.0.joined.lock().unwrap();
        if *joined {
            return;
        }
        let starts = {
            let state = shared.state.lock().unwrap();
            if state.over {
                return;
            }
            !state.visits.contains_key(key)
        };
        // Saved with the access state, so not under the Visits' lock.
        let first = starts && shared.access.lock().unwrap().first_visit(key);
        let mut state = shared.state.lock().unwrap();
        if state.over {
            return;
        }
        *joined = true;
        let now = Instant::now();
        if let Some(visit) = state.visits.get_mut(key) {
            // Another of their connections started it meanwhile.
            visit.connections += 1;
            visit.first |= first;
            visit.nudge.notify_one();
            return;
        }
        let nudge = Arc::new(Notify::new());
        state.visits.insert(
            key.clone(),
            Visit {
                label: label.to_string(),
                first,
                started: now,
                connections: 1,
                last_left: now,
                directs: HashMap::new(),
                said: None,
                path_since: now,
                heard: now,
                nudge: nudge.clone(),
            },
        );
        drop(state);
        visits.watch(key.clone(), nudge);
    }

    /// The connection closed.
    pub(crate) fn closed(&self) {
        let mut joined = self.0.joined.lock().unwrap();
        if !std::mem::take(&mut *joined) {
            return;
        }
        let Connection { visits, key, .. } = &*self.0;
        visits.change(key, |visit| {
            visit.connections -= 1;
            if visit.connections == 0 {
                visit.last_left = Instant::now();
            }
        });
    }

    /// Its Visitor sent a stream on it: they're there, on the Relay.
    pub(crate) fn heard(&self) {
        let Connection { visits, key, .. } = &*self.0;
        let mut state = visits.0.state.lock().unwrap();
        if let Some(visit) = state.visits.get_mut(key) {
            visit.heard = Instant::now();
            // Only a move onto the Relay waits to hear from them.
            if visit.said.is_some_and(|said| said != visit.path()) {
                visit.nudge.notify_one();
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

    /// The connection's device wasn't let in: logged unless it was within the grace already. Past
    /// the cap on devices named, it's counted in a line for them all instead.
    pub(crate) fn refused(&self, why: Refusal) {
        let Connection { visits, key, .. } = &*self.0;
        let shared = &visits.0;
        {
            let mut state = shared.state.lock().unwrap();
            let now = Instant::now();
            let fresh = |at: &Instant| *at + shared.grace > now;
            if state.over || state.refused.get(key).is_some_and(fresh) {
                return;
            }
            state.refused.retain(|_, at| fresh(at));
            if state.refused.len() >= shared.refused_cap {
                let more = state.more_refused.get_or_insert(0);
                *more += 1;
                if *more == 1 {
                    visits.say_more_refused_after(shared.grace);
                }
                return;
            }
            state.refused.insert(key.clone(), now);
        }
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

impl Visits {
    /// In `after`, log how many refusals went unnamed since, past the cap.
    fn say_more_refused_after(&self, after: Duration) {
        let visits = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(after).await;
            let Some(more) = visits.0.state.lock().unwrap().more_refused.take() else {
                return;
            };
            let within = match after.as_secs() {
                60 => "minute".to_string(),
                _ => crate::lasting(after),
            };
            let times = if more == 1 { "time" } else { "times" };
            log::event(
                "refused_more",
                fields! {"count" => more, "seconds" => after.as_secs()},
                &format!(
                    "refused {more} more {times} in the last {within}, too many devices to name each"
                ),
            );
        });
    }
}
