//! The Owner's operations on a running Host (ADR 0010). The CLI reaches them over a local socket,
//! Owner Devices over tooob; both end up here.
use crate::access::{Access, ChangeError, Invite, InviteInfo, InviteOptions, VisitorInfo};
use crate::wire;
use anyhow::Result;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::os::unix::fs::{DirBuilderExt, MetadataExt};
use std::{
    io,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::watch;

/// Where a Host is reached: a running Host's own endpoint, or, while it's stopped, its Host Key and
/// the Relay its settings name.
#[derive(Clone)]
pub(crate) enum Address {
    Endpoint(iroh::Endpoint),
    Stopped(HostAddress),
}

#[derive(Clone)]
pub struct Control {
    pub(crate) access: Arc<Mutex<Access>>,
    /// Tells the Host access was taken away, so it can end open WebSockets.
    pub(crate) changes: Arc<watch::Sender<()>>,
    /// Where Visitors reach the Host: what goes into Invite Links.
    pub(crate) address: Address,
    /// Each Visitor's Site cookies: a Block throws theirs away.
    pub(crate) jars: Arc<crate::jars::Jars>,
    /// Rung when the Owner asks the Host to stop; whoever runs the Host shuts it down.
    pub(crate) stop: Arc<tokio::sync::Notify>,
}

impl Control {
    /// A stopped Host's state, for the Owner's commands to change directly (ADR 0014). Only while
    /// holding its lock: a running Host keeps its own copy, and saves over this one.
    fn stopped(data_dir: &Path, address: HostAddress) -> io::Result<Control> {
        Ok(Control {
            access: Arc::new(Mutex::new(Access::open(data_dir.join("access.json"))?)),
            changes: Arc::new(watch::Sender::new(())),
            address: Address::Stopped(address),
            jars: Arc::new(crate::jars::Jars::open(data_dir.join("jars"))?),
            stop: Default::default(),
        })
    }

    /// What goes into Invite Links.
    pub fn address(&self) -> HostAddress {
        match &self.address {
            Address::Endpoint(endpoint) => {
                let addr = endpoint.addr();
                HostAddress {
                    id: addr.id.to_string(),
                    relay: addr.relay_urls().next().map(|u| u.to_string()),
                }
            }
            Address::Stopped(address) => address.clone(),
        }
    }

    pub fn invite(&self, label: &str, options: InviteOptions) -> io::Result<Invite> {
        self.access.lock().unwrap().invite(label, options, now())
    }

    pub fn visitors(&self) -> Vec<VisitorInfo> {
        self.access.lock().unwrap().visitors()
    }

    /// Unused Invites that still work.
    pub fn invites(&self) -> Vec<InviteInfo> {
        self.access.lock().unwrap().invites(now())
    }

    /// Kill an Invite Link before anyone uses it.
    pub fn revoke(&self, invite_id: &str) -> Result<(), ChangeError> {
        self.access.lock().unwrap().revoke(invite_id)
    }

    /// Let in a Visitor awaiting approval (First-Visit Approval, or becoming an Owner Device).
    pub fn approve(&self, visitor_key: &str) -> Result<(), ChangeError> {
        self.access.lock().unwrap().approve(visitor_key)
    }

    pub fn unblock(&self, visitor_key: &str) -> Result<(), ChangeError> {
        self.access.lock().unwrap().unblock(visitor_key)
    }

    /// Ask the Host to stop, as Ctrl-C would: every Visitor's connection closes at once.
    pub fn stop(&self) {
        self.stop.notify_one();
    }

    /// Takes effect on the Visitor's next request, even on a connection that's already open. Their
    /// Site cookies go too: an unblocked Visitor logs in to the Site afresh.
    pub fn block(&self, visitor_key: &str) -> Result<(), ChangeError> {
        self.access.lock().unwrap().block(visitor_key)?;
        self.jars.forget(visitor_key);
        self.changes.send_replace(());
        Ok(())
    }
}

pub(crate) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Where a Host's control socket lives: in its data directory (spelled canonically, so the Host and
/// the CLI agree however the path was typed), unless that's too long for a Unix socket (~104 bytes
/// on macOS). Then it's in a per-account private directory under the temp directory, never a
/// guessable name in shared `/tmp`, where another user could squat on it.
fn socket_path(data_dir: &Path) -> io::Result<PathBuf> {
    let dir = data_dir
        .canonicalize()
        .unwrap_or_else(|_| data_dir.to_path_buf());
    let in_place = dir.join("control.sock");
    if in_place.as_os_str().len() < 100 {
        return Ok(in_place);
    }
    let uid = unsafe { libc::getuid() };
    let private = std::env::temp_dir().join(format!("tooob-{uid}"));
    match std::fs::DirBuilder::new().mode(0o700).create(&private) {
        Err(e) if e.kind() != io::ErrorKind::AlreadyExists => return Err(e),
        _ => {}
    }
    let meta = std::fs::symlink_metadata(&private)?; // a symlink here would be someone else's trap
    if !meta.is_dir() || meta.uid() != uid || meta.mode() & 0o077 != 0 {
        return Err(io::Error::other(format!(
            "{} isn't a private directory of yours",
            private.display()
        )));
    }
    let digest: String = Sha256::digest(dir.as_os_str().as_encoded_bytes())[..8]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok(private.join(format!("{digest}.sock")))
}

/// Bind the control socket (owner-only) and answer requests on it until the Host stops.
/// Make sure no other Host serves this data directory: two would each hold their own copy of the
/// Allowlist, and the Owner's commands would reach only one. A socket nobody answers on is a
/// leftover from a Host that didn't shut down cleanly, and is cleared.
pub(crate) fn claim(data_dir: &Path) -> Result<()> {
    let path = socket_path(data_dir)?;
    if std::os::unix::net::UnixStream::connect(&path).is_ok() {
        anyhow::bail!("a Host is already running for {}", data_dir.display());
    }
    let _ = std::fs::remove_file(&path);
    Ok(())
}

pub(crate) fn listen(data_dir: &Path, control: Control) -> Result<Listening> {
    let path = socket_path(data_dir)?;
    let listener = UnixListener::bind(&path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    let task = tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let control = control.clone();
            tokio::spawn(async move {
                if let Err(e) = answer(stream, &control).await {
                    eprintln!("tooob: control request: {e:#}");
                }
            });
        }
    });
    Ok(Listening { task, path })
}

/// A running control socket; stopping it closes the socket and removes its file.
pub(crate) struct Listening {
    task: tokio::task::JoinHandle<()>,
    path: PathBuf,
}

impl Listening {
    pub(crate) fn stop(self) {
        self.task.abort();
        let _ = std::fs::remove_file(&self.path);
    }
}

/// One request per connection: a JSON line in, a JSON line out.
async fn answer(mut stream: UnixStream, control: &Control) -> Result<()> {
    let (request, _) = wire::read_line(&mut stream).await?;
    wire::write_line(&mut stream, &reply(control, &request)).await
}

/// The answer to one of the Owner's requests on this box, from the socket or straight to a stopped
/// Host's state.
fn reply(control: &Control, request: &Value) -> Value {
    match request["op"].as_str() {
        Some("invite") => {
            // Missing options mean defaults; malformed ones are an error reply, never a hangup.
            let options = match &request["options"] {
                Value::Null => Ok(InviteOptions::default()),
                given => serde_json::from_value::<InviteOptions>(given.clone()),
            };
            match options {
                Ok(options) => {
                    match control.invite(request["label"].as_str().unwrap_or_default(), options) {
                        Ok(invite) => json!({"ok": invite}),
                        Err(e) => json!({"error": e.to_string()}),
                    }
                }
                Err(e) => json!({"error": format!("bad options: {e}")}),
            }
        }
        // Pairing Invites exist only here, on the local socket: physical access counts as ownership
        // (ADR 0010). They're deliberately not on Control, which Owner Devices will reach over tooob.
        Some("pair") => {
            let label = request["label"].as_str().unwrap_or_default();
            match control
                .access
                .lock()
                .unwrap()
                .pairing_invite(label, None, now())
            {
                Ok(invite) => json!({"ok": invite}),
                Err(e) => json!({"error": e.to_string()}),
            }
        }
        Some("address") => json!({"ok": control.address()}),
        Some("stop") => {
            control.stop();
            json!({"ok": null})
        }
        Some("visitors") => json!({"ok": control.visitors()}),
        Some("invites") => json!({"ok": control.invites()}),
        Some("revoke") => outcome(control.revoke(request["id"].as_str().unwrap_or_default())),
        Some(op @ ("block" | "approve" | "unblock")) => {
            match resolve(control, request["key"].as_str().unwrap_or_default()) {
                Ok(key) => outcome(match op {
                    "block" => control.block(&key),
                    "approve" => control.approve(&key),
                    _ => control.unblock(&key),
                }),
                Err(e) => json!({"error": e}),
            }
        }
        _ => json!({"error": "unknown operation"}),
    }
}

/// A Visitor Key from a unique prefix, like git: keys are 64 characters.
fn resolve(control: &Control, prefix: &str) -> Result<String, String> {
    if prefix.is_empty() {
        return Err("key_required".into()); // "" would match every Visitor
    }
    let matches: Vec<String> = control
        .visitors()
        .into_iter()
        .map(|v| v.key)
        .filter(|k| k.starts_with(prefix))
        .collect();
    match matches.as_slice() {
        [key] => Ok(key.clone()),
        [] => Err("unknown_visitor".into()),
        _ => Err(format!("ambiguous: {}", matches.join(", "))),
    }
}

/// A change's result as a reply, with stable error codes.
fn outcome(result: Result<(), ChangeError>) -> Value {
    match result {
        Ok(()) => json!({"ok": null}),
        Err(ChangeError::UnknownVisitor) => json!({"error": "unknown_visitor"}),
        Err(ChangeError::UnknownInvite) => json!({"error": "unknown_invite"}),
        Err(ChangeError::Blocked) => json!({"error": "blocked"}),
        Err(ChangeError::Storage(_)) => json!({"error": "unavailable"}),
    }
}

/// What a Visitor's browser needs to reach a Host: its Host Key and, if it has one, its Relay.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HostAddress {
    pub id: String,
    pub relay: Option<String>,
}

impl HostAddress {
    /// The Invite Link: the Bootstrap page, the Host's Relay, the Host Key and the Invite token.
    /// Everything after `#` stays in the browser (never sent to the Bootstrap's server).
    pub fn invite_link(&self, bootstrap: &str, token: &str) -> String {
        let relay = self
            .relay
            .as_ref()
            .map(|r| format!("?relay={r}"))
            .unwrap_or_default();
        format!("{bootstrap}{relay}#{}&invite={token}", self.id)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ControlError {
    /// No Host is running for this data directory (or it can't be reached).
    NotRunning,
    /// The Host refused the request.
    Refused(String),
}

/// Talks to the Host for one data directory: what the `tooob` CLI uses.
pub struct Client {
    data_dir: PathBuf,
    /// Its address while it's stopped, for a Client that changes a stopped Host's state directly.
    stopped: Option<HostAddress>,
}

impl Client {
    /// Talks to the running Host only.
    pub fn new(data_dir: &Path) -> Client {
        Client {
            data_dir: data_dir.to_path_buf(),
            stopped: None,
        }
    }

    /// Talks to the Host whether it's running or not (ADR 0014). While it isn't, requests change its
    /// state directly, under the lock a running Host holds, and `stopped` goes into Invite Links.
    pub fn running_or_not(data_dir: &Path, stopped: HostAddress) -> Client {
        Client {
            data_dir: data_dir.to_path_buf(),
            stopped: Some(stopped),
        }
    }

    pub async fn invite(
        &self,
        label: &str,
        options: InviteOptions,
    ) -> Result<Invite, ControlError> {
        decode(
            self.ask(json!({"op": "invite", "label": label, "options": options}))
                .await?,
        )
    }

    /// A Pairing Invite: whoever uses it becomes an Owner Device without approval. Only the local
    /// socket offers it.
    pub async fn pair(&self, label: &str) -> Result<Invite, ControlError> {
        decode(self.ask(json!({"op": "pair", "label": label})).await?)
    }

    /// The running Host's Host Key and Relay: what an Invite Link needs.
    pub async fn address(&self) -> Result<HostAddress, ControlError> {
        decode(self.ask(json!({"op": "address"})).await?)
    }

    pub async fn visitors(&self) -> Result<Vec<VisitorInfo>, ControlError> {
        decode(self.ask(json!({"op": "visitors"})).await?)
    }

    /// Block a Visitor, named by their key or a unique prefix of it.
    pub async fn block(&self, key: &str) -> Result<(), ControlError> {
        self.ask(json!({"op": "block", "key": key})).await.map(drop)
    }

    /// Ask the running Host to stop.
    pub async fn stop(&self) -> Result<(), ControlError> {
        self.ask_running(&json!({"op": "stop"})).await.map(drop)
    }

    pub async fn invites(&self) -> Result<Vec<InviteInfo>, ControlError> {
        decode(self.ask(json!({"op": "invites"})).await?)
    }

    pub async fn revoke(&self, invite_id: &str) -> Result<(), ControlError> {
        self.ask(json!({"op": "revoke", "id": invite_id}))
            .await
            .map(drop)
    }

    pub async fn approve(&self, key: &str) -> Result<(), ControlError> {
        self.ask(json!({"op": "approve", "key": key}))
            .await
            .map(drop)
    }

    pub async fn unblock(&self, key: &str) -> Result<(), ControlError> {
        self.ask(json!({"op": "unblock", "key": key}))
            .await
            .map(drop)
    }

    /// The running Host answers if there is one. If not, and the lock on its state is free, the
    /// state is changed here, holding the lock so no Host starts meanwhile. Held with nobody
    /// answering is a Host starting or stopping: try again, for a while.
    async fn ask(&self, request: Value) -> Result<Value, ControlError> {
        let started = Instant::now();
        loop {
            match self.ask_running(&request).await {
                Err(ControlError::NotRunning) => {}
                answered => return answered,
            }
            let Some(address) = &self.stopped else {
                return Err(ControlError::NotRunning);
            };
            let refused = |e: io::Error| ControlError::Refused(e.to_string());
            if let Some(_lock) = crate::hosts::try_lock(&self.data_dir).map_err(refused)? {
                let control = Control::stopped(&self.data_dir, address.clone()).map_err(refused)?;
                return outcome_of(reply(&control, &request));
            }
            if started.elapsed() > Duration::from_secs(5) {
                return Err(ControlError::Refused(
                    "the Host holds its lock but doesn't answer: it may be starting or stopping, so try again".into(),
                ));
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    async fn ask_running(&self, request: &Value) -> Result<Value, ControlError> {
        let socket =
            socket_path(&self.data_dir).map_err(|e| ControlError::Refused(e.to_string()))?;
        let mut stream = UnixStream::connect(&socket)
            .await
            .map_err(|_| ControlError::NotRunning)?;
        // Only talk to a Host run by this same account: whoever is on the other end of the socket,
        // as the OS reports it after connecting (a file-ownership check could be raced with a
        // symlink swap). Otherwise another user could answer with their own Invite Links.
        let peer = stream
            .peer_cred()
            .map_err(|e| ControlError::Refused(e.to_string()))?;
        if peer.uid() != unsafe { libc::getuid() } {
            return Err(ControlError::Refused(
                "the control socket belongs to another account".into(),
            ));
        }
        let io_err = |e: anyhow::Error| ControlError::Refused(e.to_string());
        wire::write_line(&mut stream, request)
            .await
            .map_err(io_err)?;
        let (reply, _) = wire::read_line(&mut stream).await.map_err(io_err)?;
        outcome_of(reply)
    }
}

/// A reply's `ok`, or its `error` as a refusal.
fn outcome_of(reply: Value) -> Result<Value, ControlError> {
    match reply.get("ok") {
        Some(ok) => Ok(ok.clone()),
        None => Err(ControlError::Refused(
            reply["error"].as_str().unwrap_or("refused").to_string(),
        )),
    }
}

fn decode<T: serde::de::DeserializeOwned>(reply: Value) -> Result<T, ControlError> {
    serde_json::from_value(reply).map_err(|e| ControlError::Refused(e.to_string()))
}
