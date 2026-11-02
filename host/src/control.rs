//! The Owner's operations on a running Host (ADR 0010). The CLI reaches them over a local socket,
//! Owner Devices over tooob; both end up here.
use crate::access::{
    Access, ChangeError, INVITE_LIFETIME_SECS, Invite, InviteInfo, InviteOptions,
    PAIRING_LIFETIME_SECS, VisitorInfo,
};
use crate::bootstrap::Found;
use crate::fields;
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

#[derive(Clone)]
pub struct Control {
    pub(crate) access: Arc<Mutex<Access>>,
    /// Tells the Host access was taken away, so it can end open WebSockets.
    pub(crate) changes: Arc<watch::Sender<()>>,
    /// Each Visitor's Site cookies: a Block throws theirs away.
    pub(crate) jars: Arc<crate::jars::Jars>,
    /// Rung when the Owner asks the Host to stop; whoever runs the Host shuts it down.
    pub(crate) stop: Arc<tokio::sync::Notify>,
    /// What whoever runs the Host last found its Relay's Bootstrap to be (ADR 0020).
    pub(crate) bootstrap: Arc<Mutex<Found>>,
    /// A stopped Host's log, which its Owner's commands write themselves; None for a running
    /// Host, whose log is this process's (ADR 0025).
    pub(crate) log: Option<Arc<crate::log::Log>>,
    /// What each Visitor has used: counted as it goes while the Host runs, as kept when it doesn't.
    pub(crate) usage: Arc<crate::usage::Usage>,
}

/// Where an Owner's change came from, as the log says (ADR 0025).
#[derive(Clone, Copy)]
pub(crate) enum Source {
    /// The `tooob` command, on the home box: the local socket, or a stopped Host's state.
    Cli,
}

impl Source {
    fn fields(self) -> &'static str {
        match self {
            Source::Cli => "cli",
        }
    }

    fn said(self) -> &'static str {
        match self {
            Source::Cli => "from the CLI",
        }
    }
}

impl Control {
    /// A stopped Host's state, for the Owner's commands to change directly (ADR 0014). Only while
    /// holding its lock: a running Host keeps its own copy, and saves over this one.
    fn stopped(data_dir: &Path) -> io::Result<Control> {
        Ok(Control {
            access: Arc::new(Mutex::new(Access::open(data_dir.join("access.json"))?)),
            changes: Arc::new(watch::Sender::new(())),
            jars: Arc::new(crate::jars::Jars::open(data_dir.join("jars"))?),
            stop: Default::default(),
            bootstrap: Default::default(),
            log: Some(Arc::new(crate::log::Log::open(
                data_dir,
                &data_dir.file_name().unwrap_or_default().to_string_lossy(),
                false,
            ))),
            usage: Arc::new(crate::usage::Usage::open(data_dir.join("usage.json"))?),
        })
    }

    /// Log the Owner's change, `event`, from `from`: `text` · where it came from.
    fn changed(&self, event: &str, mut fields: crate::log::Fields, text: &str, from: Source) {
        fields.insert("from".into(), from.fields().into());
        let text = format!("{text} · {}", from.said());
        match &self.log {
            Some(log) => log.event(event, fields, &text),
            None => crate::log::event(event, fields, &text),
        }
    }

    /// `dave (3f2a91c0)`: a Visitor as the log names them.
    fn named(&self, visitor_key: &str) -> (String, String) {
        let label = self.access.lock().unwrap().label(visitor_key);
        (
            label.unwrap_or_default(),
            crate::visit_log::short(visitor_key).to_string(),
        )
    }

    pub fn invite(&self, label: &str, options: InviteOptions) -> io::Result<Invite> {
        self.access.lock().unwrap().invite(label, options, now())
    }

    /// Every Visitor, with what they've used since they first got in and when they were last seen.
    pub fn visitors(&self) -> Vec<VisitorInfo> {
        let mut visitors = self.access.lock().unwrap().visitors();
        let now = now();
        for v in &mut visitors {
            let (used, last_seen) = self.usage.of(&v.key, now);
            (v.sent, v.received, v.last_seen) = (used.sent, used.received, last_seen);
        }
        visitors
    }

    /// Unused Invites that still work.
    pub fn invites(&self) -> Vec<InviteInfo> {
        self.access.lock().unwrap().invites(now())
    }

    /// Kill an Invite Link before anyone uses it.
    pub fn revoke(&self, invite_id: &str) -> Result<(), ChangeError> {
        self.access.lock().unwrap().revoke(invite_id).map(drop)
    }

    /// Let in a Visitor awaiting approval (First-Visit Approval, or becoming an Owner Device).
    pub fn approve(&self, visitor_key: &str) -> Result<(), ChangeError> {
        self.access.lock().unwrap().approve(visitor_key)
    }

    pub fn unblock(&self, visitor_key: &str) -> Result<(), ChangeError> {
        self.access.lock().unwrap().unblock(visitor_key)
    }

    /// What its Relay's Bootstrap was, last it was checked.
    pub fn bootstrap_found(&self) -> Found {
        self.bootstrap.lock().unwrap().clone()
    }

    /// Say what its Relay's Bootstrap is now, for `tooob ls` and `tooob show` to tell the Owner.
    pub fn found_bootstrap(&self, found: Found) {
        *self.bootstrap.lock().unwrap() = found;
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

/// Seconds since the Unix epoch.
pub fn now() -> u64 {
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
                    crate::log::event(
                        "control_error",
                        Default::default(),
                        &format!("control request: {e:#}"),
                    );
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
/// Host's state. Each change is logged, as from the CLI.
fn reply(control: &Control, request: &Value) -> Value {
    let from = Source::Cli;
    match request["op"].as_str() {
        Some("invite") => {
            // Missing options mean defaults; malformed ones are an error reply, never a hangup.
            let options = match &request["options"] {
                Value::Null => Ok(InviteOptions::default()),
                given => serde_json::from_value::<InviteOptions>(given.clone()),
            };
            let label = request["label"].as_str().unwrap_or_default();
            match options {
                Ok(options) => {
                    let made = Made {
                        label,
                        first_visit_approval: options.first_visit_approval,
                        owner_invite: options.owner_invite,
                        pairing: false,
                        expires_in: options.expires_in.unwrap_or(INVITE_LIFETIME_SECS),
                    };
                    match control.invite(label, options) {
                        Ok(invite) => {
                            made.log(control, &invite.id, from);
                            json!({"ok": invite})
                        }
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
            let paired = control
                .access
                .lock()
                .unwrap()
                .pairing_invite(label, None, now());
            match paired {
                Ok(invite) => {
                    let made = Made {
                        label,
                        first_visit_approval: false,
                        owner_invite: true,
                        pairing: true,
                        expires_in: PAIRING_LIFETIME_SECS,
                    };
                    made.log(control, &invite.id, from);
                    json!({"ok": invite})
                }
                Err(e) => json!({"error": e.to_string()}),
            }
        }
        Some("stop") => {
            control.stop();
            json!({"ok": null})
        }
        Some("visitors") => json!({"ok": control.visitors()}),
        Some("bootstrap") => json!({"ok": control.bootstrap_found()}),
        Some("invites") => json!({"ok": control.invites()}),
        Some("revoke") => {
            let id = request["id"].as_str().unwrap_or_default();
            let revoked = control.access.lock().unwrap().revoke(id);
            if let Ok(label) = &revoked {
                let fields = fields! {"invite" => id, "label" => label};
                let text = format!("revoked Invite {id} for {label}");
                control.changed("invite_revoked", fields, &text, from);
            }
            outcome(revoked.map(drop))
        }
        Some(op @ ("block" | "approve" | "unblock")) => {
            // Approve takes a waiting device's Device Code too (#110).
            let key = match request["code"].as_str() {
                Some(code) if op == "approve" => waiting_with(control, code),
                _ => resolve(control, request["key"].as_str().unwrap_or_default()),
            };
            match key {
                Ok(key) => {
                    let owner_device = |control: &Control| {
                        control
                            .visitors()
                            .into_iter()
                            .any(|v| v.key == key && v.owner_device)
                    };
                    let was_owner_device = owner_device(control);
                    let (done, event) = match op {
                        "block" => (control.block(&key), "blocked"),
                        "approve" => (control.approve(&key), "approved"),
                        _ => (control.unblock(&key), "unblocked"),
                    };
                    if done.is_ok() {
                        let (label, short) = control.named(&key);
                        let mut fields = fields! {"label" => label, "key" => short};
                        let mut text = format!("{event} {label} ({short})");
                        if op == "approve" && !was_owner_device && owner_device(control) {
                            fields.insert("owner_device".into(), true.into());
                            text += " as an Owner Device";
                        }
                        control.changed(event, fields, &text, from);
                    }
                    outcome(done)
                }
                Err(e) => json!({"error": e}),
            }
        }
        _ => json!({"error": "unknown operation"}),
    }
}

/// An Invite made, as the log says it.
struct Made<'a> {
    label: &'a str,
    first_visit_approval: bool,
    owner_invite: bool,
    pairing: bool,
    expires_in: u64,
}

impl Made<'_> {
    /// `made Invite a1b2c3d4 for gran · approve first · works for 2 hours`, as `id`.
    fn log(&self, control: &Control, id: &str, from: Source) {
        let fields = fields! {
            "invite" => id,
            "label" => self.label,
            "first_visit_approval" => self.first_visit_approval,
            "owner_invite" => self.owner_invite,
            "pairing" => self.pairing,
            "expires_in" => self.expires_in,
        };
        let kind = if self.pairing {
            "Pairing Invite"
        } else {
            "Invite"
        };
        let mut text = format!("made {kind} {id} for {}", self.label);
        if self.first_visit_approval {
            text += " · approve first";
        }
        if self.owner_invite && !self.pairing {
            text += " · Owner Invite";
        }
        let lasts = crate::lasting(Duration::from_secs(self.expires_in));
        text += &format!(" · works for {lasts}");
        control.changed("invite_made", fields, &text, from);
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

/// The Visitor Key of the one device waiting for approval whose Device Code is `code`: never a
/// device that's in or Blocked, and never a guess between two that share it.
fn waiting_with(control: &Control, code: &str) -> Result<String, String> {
    let code = crate::access::typed_device_code(code).unwrap_or_else(|| code.into());
    let matches: Vec<String> = control
        .visitors()
        .into_iter()
        .filter(|v| v.device_code.as_deref() == Some(code.as_str()))
        .map(|v| v.key)
        .collect();
    match matches.as_slice() {
        [key] => Ok(key.clone()),
        [] => Err(format!(
            "unknown_device_code: no device waiting for approval has Device Code {code} \
             (six digits always read as a Device Code: to name a device by its Visitor Key, give \
             7 characters of it or more)"
        )),
        _ => Err(format!(
            "ambiguous_device_code: {} devices waiting for approval have Device Code {code}; \
             approve one by its Visitor Key: {}",
            matches.len(),
            matches.join(", ")
        )),
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
    /// Whether it changes a stopped Host's state directly.
    stopped: bool,
}

impl Client {
    /// Talks to the running Host only.
    pub fn new(data_dir: &Path) -> Client {
        Client {
            data_dir: data_dir.to_path_buf(),
            stopped: false,
        }
    }

    /// Talks to the Host whether it's running or not (ADR 0014). While it isn't, requests change its
    /// state directly, under the lock a running Host holds.
    pub fn running_or_not(data_dir: &Path) -> Client {
        Client {
            data_dir: data_dir.to_path_buf(),
            stopped: true,
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

    pub async fn visitors(&self) -> Result<Vec<VisitorInfo>, ControlError> {
        decode(self.ask(json!({"op": "visitors"})).await?)
    }

    /// What the running Host last found its Relay's Bootstrap to be. Only a running Host knows.
    pub async fn bootstrap_found(&self) -> Result<Found, ControlError> {
        decode(self.ask_running(&json!({"op": "bootstrap"})).await?)
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

    /// Approve a device waiting for it, named by its Visitor Key (or a unique prefix of it), or by
    /// its Device Code as shown, spaces or not.
    pub async fn approve(&self, key_or_code: &str) -> Result<(), ControlError> {
        let request = match crate::access::typed_device_code(key_or_code) {
            Some(code) => json!({"op": "approve", "code": code}),
            None => json!({"op": "approve", "key": key_or_code}),
        };
        self.ask(request).await.map(drop)
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
            if !self.stopped {
                return Err(ControlError::NotRunning);
            }
            let refused = |e: io::Error| ControlError::Refused(e.to_string());
            if let Some(_lock) = crate::hosts::try_lock(&self.data_dir).map_err(refused)? {
                let control = Control::stopped(&self.data_dir).map_err(refused)?;
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
