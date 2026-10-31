//! How much each Visitor has used (ADR 0025): the bytes sent to them and received from them, on
//! the Relayed and the Direct Path alike, counted where they pass (a request's lines, its body,
//! a WebSocket's frames), never what they were. Each Visitor's totals since they first got in,
//! and when they were last seen, are kept with the Host State in `usage.json`, apart from the
//! Allowlist, so `tooob visitors` shows them across restarts.
//!
//! Counting is an atomic add on the Visitor's own meter, with no lock on the way: the meters are
//! read only when a Visit ends, when the totals are saved, and when the Owner asks.
use crate::visit_log::Presence;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    io,
    path::PathBuf,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    task::{Context, Poll, ready},
    time::Duration,
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// How often a running Host saves what its Visitors have used, if it changed: a minute, or as
/// `TOOOB_USAGE_SAVE_EVERY` says (`1s`), which tests use. It saves at each Visit's end and when it
/// stops too, so only a crash loses any, and never more than this long's.
const SAVE_EVERY: Duration = Duration::from_secs(60);

/// Bytes sent to a Visitor and received from them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Used {
    pub sent: u64,
    pub received: u64,
}

impl std::ops::Sub for Used {
    type Output = Used;
    fn sub(self, before: Used) -> Used {
        Used {
            sent: self.sent.saturating_sub(before.sent),
            received: self.received.saturating_sub(before.received),
        }
    }
}

impl std::ops::Add for Used {
    type Output = Used;
    fn add(self, more: Used) -> Used {
        Used {
            sent: self.sent.saturating_add(more.sent),
            received: self.received.saturating_add(more.received),
        }
    }
}

/// What one Visitor has used: counted as it passes.
#[derive(Debug, Default)]
pub(crate) struct Meter {
    sent: AtomicU64,
    received: AtomicU64,
}

impl Meter {
    pub(crate) fn sent(&self, n: usize) {
        self.sent.fetch_add(n as u64, Ordering::Relaxed);
    }

    pub(crate) fn received(&self, n: usize) {
        self.received.fetch_add(n as u64, Ordering::Relaxed);
    }

    pub(crate) fn read(&self) -> Used {
        Used {
            sent: self.sent.load(Ordering::Relaxed),
            received: self.received.load(Ordering::Relaxed),
        }
    }

    /// Everything on it, leaving it at nothing.
    pub(crate) fn take(&self) -> Used {
        Used {
            sent: self.sent.swap(0, Ordering::Relaxed),
            received: self.received.swap(0, Ordering::Relaxed),
        }
    }

    pub(crate) fn add(&self, used: Used) {
        self.sent.fetch_add(used.sent, Ordering::Relaxed);
        self.received.fetch_add(used.received, Ordering::Relaxed);
    }
}

/// One Visitor's, as kept in `usage.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
struct Kept {
    #[serde(flatten)]
    used: Used,
    /// When they were last seen, in Unix seconds: their last connection's end, or while they
    /// visit, the last save.
    last_seen: Option<u64>,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct File {
    /// By full Visitor Key.
    visitors: HashMap<String, Kept>,
}

/// What a Host's Visitors have used: since they first got in, and this run's, as it's counted.
pub(crate) struct Usage {
    path: PathBuf,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    /// As kept when this run started, but for when each was last seen, which is kept up to date.
    kept: HashMap<String, Kept>,
    /// This run's, by full Visitor Key: only for Visitors let in.
    meters: HashMap<String, Arc<Meter>>,
    /// The Visitors on a Visit now.
    visiting: HashSet<String>,
    /// What was last saved, so an unchanged file isn't written again.
    saved: Vec<u8>,
}

impl Usage {
    /// The usage kept at `path`, or none yet. A file that isn't usage (spoilt) is moved aside to
    /// `usage.bad`, said, and started over: what Visitors used is never worth a Host not starting.
    /// One that can't be read at all (its permissions) is an error, as the Allowlist's is: starting
    /// over would save over what couldn't be read.
    pub(crate) fn open(path: impl Into<PathBuf>) -> io::Result<Usage> {
        let path = path.into();
        let (file, saved) = match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<File>(&bytes) {
                Ok(file) => (file, bytes),
                Err(e) => {
                    let bad = path.with_extension("bad");
                    let moved = match std::fs::rename(&path, &bad) {
                        Ok(()) => format!("moved to {}", bad.display()),
                        Err(e) => format!("couldn't move it aside: {e}"),
                    };
                    crate::log::warning(&format!(
                        "{} isn't readable ({e}), {moved}: what each Visitor used starts over",
                        path.display()
                    ));
                    (File::default(), Vec::new())
                }
            },
            Err(e) if e.kind() == io::ErrorKind::NotFound => (File::default(), Vec::new()),
            Err(e) => return Err(e),
        };
        Ok(Usage {
            path,
            state: Mutex::new(State {
                kept: file.visitors,
                saved,
                ..State::default()
            }),
        })
    }

    /// Visitor `key`'s meter for this run, which their connections count on.
    pub(crate) fn meter(&self, key: &str) -> Arc<Meter> {
        let mut state = self.state.lock().unwrap();
        state.meters.entry(key.to_string()).or_default().clone()
    }

    /// Visitor `key`'s Visit started.
    pub(crate) fn arrived(&self, key: &str) {
        self.state.lock().unwrap().visiting.insert(key.to_string());
    }

    /// Visitor `key`'s Visit ended: they were last seen at `at`, in Unix seconds.
    pub(crate) fn left(&self, key: &str, at: u64) {
        let mut state = self.state.lock().unwrap();
        state.visiting.remove(key);
        state.kept.entry(key.to_string()).or_default().last_seen = Some(at);
    }

    /// What Visitor `key` has used since they first got in, and when they were last seen: now,
    /// while they visit.
    pub(crate) fn of(&self, key: &str, now: u64) -> (Used, Option<u64>) {
        let state = self.state.lock().unwrap();
        let kept = state.kept.get(key).cloned().unwrap_or_default();
        let this_run = state.meters.get(key).map(|m| m.read()).unwrap_or_default();
        let total = kept.used + this_run;
        let last_seen = match state.visiting.contains(key) {
            true => Some(now),
            false => kept.last_seen,
        };
        (total, last_seen)
    }

    /// Save every Visitor's totals, if they changed since they were last saved. Under the lock, so
    /// two saves never write at once; nothing on the data path takes it.
    pub(crate) fn save(&self) {
        let now = crate::control::now();
        let mut state = self.state.lock().unwrap();
        let mut visitors = state.kept.clone();
        for (key, meter) in &state.meters {
            let kept = visitors.entry(key.clone()).or_default();
            kept.used = kept.used + meter.read();
        }
        for key in &state.visiting {
            visitors.entry(key.clone()).or_default().last_seen = Some(now);
        }
        let bytes = match serde_json::to_vec_pretty(&File { visitors }) {
            Ok(bytes) => bytes,
            Err(e) => return crate::log::warning(&format!("couldn't save Visitors' usage: {e}")),
        };
        if bytes == state.saved {
            return;
        }
        match crate::save_privately(&self.path, &bytes) {
            Ok(()) => state.saved = bytes,
            Err(e) => crate::log::warning(&format!("couldn't save Visitors' usage: {e}")),
        }
    }

    /// Save every so often, as the environment says, until the task is aborted.
    pub(crate) fn save_every(self: &Arc<Usage>) -> anyhow::Result<tokio::task::JoinHandle<()>> {
        let every = match std::env::var("TOOOB_USAGE_SAVE_EVERY") {
            Ok(given) => Duration::from_secs(
                crate::seconds(&given).map_err(|e| e.context("in TOOOB_USAGE_SAVE_EVERY"))?,
            ),
            Err(_) => SAVE_EVERY,
        };
        if every.is_zero() {
            anyhow::bail!("in TOOOB_USAGE_SAVE_EVERY: say how often, more than 0s");
        }
        let usage = self.clone();
        Ok(tokio::spawn(async move {
            let mut ticks = tokio::time::interval(every);
            ticks.tick().await;
            loop {
                ticks.tick().await;
                let usage = usage.clone();
                let _ = tokio::task::spawn_blocking(move || usage.save()).await;
            }
        }))
    }
}

/// A stream of a Visitor's, counting what passes on it to their Visit.
pub(crate) struct Counted<T> {
    inner: T,
    presence: Presence,
}

impl<T> Counted<T> {
    pub(crate) fn new(inner: T, presence: Presence) -> Counted<T> {
        Counted { inner, presence }
    }
}

/// A stream's reading, tallied to be counted later, or not.
pub(crate) struct Tally<'a, R> {
    inner: &'a mut R,
    read: usize,
}

impl<'a, R> Tally<'a, R> {
    pub(crate) fn new(inner: &'a mut R) -> Tally<'a, R> {
        Tally { inner, read: 0 }
    }

    /// How many bytes were read through it.
    pub(crate) fn read(&self) -> usize {
        self.read
    }
}

impl<R: AsyncRead + Unpin> AsyncRead for Tally<'_, R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let before = buf.filled().len();
        ready!(Pin::new(&mut *self.inner).poll_read(cx, buf))?;
        self.read += buf.filled().len() - before;
        Poll::Ready(Ok(()))
    }
}

impl<T: AsyncRead + Unpin> AsyncRead for Counted<T> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let before = buf.filled().len();
        ready!(Pin::new(&mut self.inner).poll_read(cx, buf))?;
        self.presence.received(buf.filled().len() - before);
        Poll::Ready(Ok(()))
    }
}

impl<T: AsyncWrite + Unpin> AsyncWrite for Counted<T> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let n = ready!(Pin::new(&mut self.inner).poll_write(cx, buf))?;
        self.presence.sent(n);
        Poll::Ready(Ok(n))
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}
