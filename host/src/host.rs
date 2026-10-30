//! A Host: makes one Site reachable to the Visitors its Allowlist lets in (ADR 0009).
use crate::Network;
use crate::access::{Access, Decision, RedeemError, Redeemed};
use crate::control::{Control, now};
use crate::jars::{Jars, SiteClient};
use crate::wire::{self, ALPN};
use anyhow::{Context, Result, bail};
use futures::{SinkExt, StreamExt};
use iroh::{
    EndpointAddr, EndpointId, PublicKey, SecretKey, Signature,
    endpoint::{Connection, RecvStream, SendStream},
    protocol::{AcceptError, ProtocolHandler, Router},
};
use reqwest::cookie::CookieStore;
use serde_json::{Value, json};
use std::{
    io::Write,
    net::SocketAddr,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::watch;
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};

pub struct HostConfig {
    pub site: Site,
    /// Holds the Host Key and the access state.
    pub data_dir: PathBuf,
    pub network: Network,
}

/// What a Host makes reachable.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Site {
    /// A web app on the home box, where it listens: e.g. 127.0.0.1:8123.
    WebApp(SocketAddr),
    /// A Shared Folder: a folder or a single file the Host serves itself (ADR 0012).
    SharedFolder(PathBuf),
}

impl std::fmt::Display for Site {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Site::WebApp(addr) => write!(f, "{addr}"),
            Site::SharedFolder(path) => write!(f, "{}", path.display()),
        }
    }
}

pub struct Host {
    router: Router,
    control: Control,
    control_socket: crate::control::Listening,
}

impl Host {
    pub async fn start(config: HostConfig) -> Result<Host> {
        // It holds the Host Key, the access state and the control socket: anyone who can read the
        // Host Key can impersonate the Host. A directory we create is private to the Owner's account;
        // an existing one (a NAS mount, a Docker volume) is left as it is, with a warning if it's open.
        if config.data_dir.exists() {
            let mode = std::fs::metadata(&config.data_dir)?.permissions().mode();
            if mode & 0o077 != 0 {
                eprintln!(
                    "tooob: warning: {} is readable by other accounts (mode {:o}); consider `chmod 700` on it",
                    config.data_dir.display(),
                    mode & 0o777
                );
            }
        } else {
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(&config.data_dir)?;
        }
        crate::control::claim(&config.data_dir)?;
        let site = match config.site {
            Site::SharedFolder(path) => Site::SharedFolder(
                path.canonicalize()
                    .with_context(|| format!("can't share {}", path.display()))?,
            ),
            site => site,
        };
        Record::keep(&config.data_dir, &site, &config.network)?;
        let access = Access::open(config.data_dir.join("access.json"))?;
        let endpoint = crate::endpoint(&config.network, load_host_key(&config.data_dir)?).await?;
        let access = Arc::new(Mutex::new(access));
        let changes = Arc::new(watch::Sender::new(()));
        let jars = Arc::new(Jars::open(config.data_dir.join("jars"))?);
        let admission = Admission {
            host_key: endpoint.id(),
            access: access.clone(),
            site,
            jars: jars.clone(),
            changes: changes.clone(),
        };
        let control = Control {
            access,
            changes,
            jars,
            stop: Default::default(),
            bootstrap: Default::default(),
        };
        let router = Router::builder(endpoint).accept(ALPN, admission).spawn();
        let control_socket = crate::control::listen(&config.data_dir, control.clone())?;
        Ok(Host {
            router,
            control,
            control_socket,
        })
    }

    /// What a Visitor dials: the Host Key plus how to reach it. Goes into Invite Links.
    pub fn address(&self) -> EndpointAddr {
        self.router.endpoint().addr()
    }

    /// Wait until the Host has a home Relay, so Invite Links made now include it.
    pub async fn online(&self) {
        self.router.endpoint().online().await;
    }

    /// Stop accepting Visitors and Owner commands, and close every connection.
    pub async fn shutdown(self) {
        self.control_socket.stop();
        let _ = self.router.shutdown().await;
    }

    /// Resolves when the Owner asks this Host to stop (`tooob stop`).
    pub async fn stop_requested(&self) {
        self.control.stop.notified().await
    }

    /// The Owner's operations on this Host.
    pub fn control(&self) -> Control {
        self.control.clone()
    }
}

/// What a Host was made for, kept in its data directory: the Site it serves, and the network it
/// last ran on (so it can be started again as it was).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Record {
    pub site: Site,
    pub network: Network,
}

impl Record {
    pub fn read(data_dir: &std::path::Path) -> Option<Record> {
        serde_json::from_slice(&std::fs::read(data_dir.join("host.json")).ok()?).ok()
    }

    /// A Host only ever serves the Site it was made for. Its Visitors were invited to that Site,
    /// and the Allowlist belongs to the Host: with another Site behind the same Host Key, people
    /// invited to ~/Photos would see ~/Taxes.
    fn keep(data_dir: &std::path::Path, site: &Site, network: &Network) -> Result<()> {
        if let Some(record) = Record::read(data_dir)
            && record.site != *site
        {
            let name = data_dir.file_name().unwrap_or_default().to_string_lossy();
            bail!(
                "this Host ({name}) serves {}, not {site}: share {site} as a new Host",
                record.site
            );
        }
        let record = Record {
            site: site.clone(),
            network: network.clone(),
        };
        crate::save_privately(
            &data_dir.join("host.json"),
            &serde_json::to_vec_pretty(&record)?,
        )?;
        Ok(())
    }
}

/// A Host's Host Key as Invite Links carry it (the public half), read from its data directory;
/// None if it has none yet.
pub fn host_key(data_dir: &std::path::Path) -> Option<String> {
    let bytes: [u8; 32] = std::fs::read(data_dir.join("host.key"))
        .ok()?
        .try_into()
        .ok()?;
    Some(SecretKey::from_bytes(&bytes).public().to_z32())
}

/// The Host Key, kept across restarts so the Key Address and Invite Links stay valid.
pub(crate) fn load_host_key(data_dir: &std::path::Path) -> Result<SecretKey> {
    let path = data_dir.join("host.key");
    if let Ok(bytes) = std::fs::read(&path) {
        let bytes: [u8; 32] = bytes.try_into().ok().context("bad host key file")?;
        // Tighten older key files; if the filesystem won't let us, say so rather than refuse to run.
        if let Err(e) = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)) {
            eprintln!(
                "tooob: warning: couldn't make {} private: {e}",
                path.display()
            );
        }
        return Ok(SecretKey::from_bytes(&bytes));
    }
    let key = SecretKey::generate();
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)?;
    file.write_all(&key.to_bytes())?;
    Ok(key)
}

/// Checks every stream against the Allowlist before anything reaches the Site.
#[derive(Debug, Clone)]
pub(crate) struct Admission {
    /// This Host's own Host Key: every hello is signed for it.
    host_key: EndpointId,
    access: Arc<Mutex<Access>>,
    /// Fires when the Owner takes access away, so open WebSockets get rechecked.
    changes: Arc<watch::Sender<()>>,
    site: Site,
    /// Each Visitor's Site cookies, kept on disk across restarts (ADR 0005).
    jars: Arc<Jars>,
}

/// A request body up to this size goes to the Site whole, with its Content-Length, as a browser
/// would send it. A bigger one (an Immich video) streams to the Site as it arrives, chunked, at
/// the pace the Site takes it: the Host never holds an upload, so uploads have no size cap.
const WHOLE_BODY: usize = 1 << 20;

/// Why an Invite was refused, as a stable code for the browser. Storage trouble stays vague: the
/// Visitor isn't admitted and doesn't need the Host's internals.
fn redeem_error_code(e: &RedeemError) -> &'static str {
    match e {
        RedeemError::UnknownInvite => "unknown_invite",
        RedeemError::AlreadyUsed => "already_used",
        RedeemError::Expired => "expired",
        RedeemError::Blocked => "blocked",
        RedeemError::AlreadyAdmitted => "already_admitted",
        RedeemError::Storage(_) => "unavailable",
    }
}

/// Tells the Site which Visitor is asking: their Label, percent-encoded so any Label is a valid
/// header value ("Mamá" → "Mam%C3%A1").
const LABEL_HEADER: &str = "x-tooob-label";

/// A Visitor's request header the Site may see. `x-tooob-*` is the Host's to set: a Visitor can't
/// pose as another Label.
fn forwardable(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    !SKIP_REQ.contains(&name.as_str()) && !name.starts_with("x-tooob-")
}

fn encode_label(label: &str) -> String {
    label
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

// Hop-by-hop or recomputed headers that never cross the proxy.
const SKIP_REQ: &[&str] = &[
    "host",
    "connection",
    "accept-encoding",
    "content-length",
    "keep-alive",
    "upgrade",
    // The Site's cookies come from the Visitor's jar on the Host, never from the Visitor (ADR 0005).
    "cookie",
];
const SKIP_RESP: &[&str] = &[
    "connection",
    "transfer-encoding",
    "keep-alive",
    "set-cookie",
];

impl ProtocolHandler for Admission {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        // Nobody until the first stream, a hello, proves a Visitor Key for this very connection: each
        // browser tab connects with a key of its own (ADR 0011).
        let hello = async {
            let (send, recv) = conn.accept_bi().await?;
            self.hello(send, recv, conn.remote_id()).await
        };
        let wait = std::time::Duration::from_secs(10);
        let visitor = match tokio::time::timeout(wait, hello).await {
            Ok(Ok(visitor)) => visitor,
            Ok(Err(e)) => {
                eprintln!("tooob: refused a hello: {e:#}");
                conn.close(1u32.into(), b"bad hello");
                return Ok(());
            }
            Err(_) => {
                conn.close(1u32.into(), b"no hello");
                return Ok(());
            }
        };
        let direct = crate::direct::Slot::default();
        while let Ok((send, recv)) = conn.accept_bi().await {
            let (this, visitor, conn) = (self.clone(), visitor.clone(), conn.clone());
            let direct = direct.clone();
            tokio::spawn(async move {
                if let Err(e) = this.iroh_stream(send, recv, &visitor, conn, &direct).await {
                    eprintln!("tooob: stream from {}: {e:#}", &visitor[..8]);
                }
            });
        }
        Ok(())
    }
}

impl Admission {
    /// Read the hello and answer it; the Visitor Key it proves, or why not.
    async fn hello(
        &self,
        mut send: SendStream,
        mut recv: RecvStream,
        connection: EndpointId,
    ) -> Result<String> {
        let (head, _) = wire::read_line(&mut recv).await?;
        let proven = (|| {
            if head["t"] != "hello" {
                bail!("first stream wasn't a hello");
            }
            let visitor: PublicKey = head["visitor"].as_str().context("no visitor")?.parse()?;
            let sig: Signature = serde_json::from_value(head["sig"].clone())?;
            visitor.verify(&wire::hello_message(&self.host_key, &connection), &sig)?;
            Ok(visitor.to_string())
        })();
        let reply = match proven {
            Ok(_) => json!({"status": 200}),
            Err(_) => json!({"status": 403, "error": "bad_hello"}),
        };
        wire::write_line(&mut send, &reply).await?;
        send.finish()?;
        // Let the reply arrive before a refusal closes the connection.
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), send.stopped()).await;
        proven
    }

    /// An iroh stream: a request, or the browser setting up its Direct Path. Signaling goes only
    /// over iroh, on the connection the hello proved, which binds the Direct Path to this Visitor.
    async fn iroh_stream(
        &self,
        mut send: SendStream,
        mut recv: RecvStream,
        visitor: &str,
        conn: Connection,
        direct: &crate::direct::Slot,
    ) -> Result<()> {
        let (head, rest) = wire::read_line(&mut recv).await?;
        if head["t"] != "rtc" {
            return self.serve(head, rest, send, recv, visitor).await;
        }
        // Only for a Visitor who's in: a Direct Path costs the home box a peer connection and its
        // UDP sockets. Its requests still go through the access check one by one.
        let decision = self.access.lock().unwrap().admits(visitor);
        if !matches!(decision, Decision::Allowed { .. }) {
            let reply = json!({"status": 403, "standing": standing(&decision)});
            wire::write_line(&mut send, &reply).await?;
            send.finish()?;
            return Ok(());
        }
        let offer = head["sdp"].as_str().unwrap_or_default();
        if head["restart"] == true {
            let restarted = crate::direct::restart(direct, offer, rest, &mut send, recv).await;
            send.finish()?;
            return restarted;
        }
        let ended = async move {
            conn.closed().await;
        };
        let opened = crate::direct::open(
            direct,
            offer,
            rest,
            &mut send,
            recv,
            self.clone(),
            visitor.into(),
            ended,
        )
        .await;
        send.finish()?;
        opened
    }

    /// Serve one request on a two-way byte stream: an iroh stream, or one carried on the Direct
    /// Path. One JSON line in, one JSON line (then a body) out; `shutdown` ends the reply.
    pub(crate) async fn stream(
        &self,
        send: impl AsyncWrite + Unpin + Send,
        mut recv: impl AsyncRead + Unpin + Send + 'static,
        visitor: &str,
    ) -> Result<()> {
        let (head, rest) = wire::read_line(&mut recv).await?;
        self.serve(head, rest, send, recv, visitor).await
    }

    async fn serve(
        &self,
        head: Value,
        rest: Vec<u8>,
        mut send: impl AsyncWrite + Unpin + Send,
        recv: impl AsyncRead + Unpin + Send + 'static,
        visitor: &str,
    ) -> Result<()> {
        if head["t"] == "standing" {
            // Where this Visitor stands, without touching the Site: the browser decides what to show.
            let standing = standing(&self.access.lock().unwrap().admits(visitor));
            wire::write_line(&mut send, &json!({"status": 200, "standing": standing})).await?;
        } else if head["t"] == "redeem" {
            let token = head["token"].as_str().unwrap_or_default();
            let reply = match self.access.lock().unwrap().redeem(token, visitor, now()) {
                Ok(Redeemed::Admitted) => json!({"status": 200, "redeemed": "admitted"}),
                Ok(Redeemed::AwaitingApproval) => {
                    json!({"status": 200, "redeemed": "awaiting_approval"})
                }
                Err(e) => json!({"status": 403, "error": redeem_error_code(&e)}),
            };
            wire::write_line(&mut send, &reply).await?;
        } else {
            let decision = self.access.lock().unwrap().admits(visitor);
            let path = head["path"].as_str().unwrap_or("/");
            match (decision, &self.site) {
                (Decision::Allowed { .. }, Site::SharedFolder(root)) => {
                    crate::shared_folder::serve(root, &head, &mut send).await?
                }
                (Decision::Allowed { .. }, Site::WebApp(site)) if !on_site(site, path) => {
                    let reply = json!({"status": 400, "error": "bad_path"});
                    wire::write_line(&mut send, &reply).await?
                }
                (Decision::Allowed { label, .. }, Site::WebApp(site)) if head["t"] == "ws" => {
                    self.websocket(*site, &head, rest, &mut send, recv, visitor, &label)
                        .await?
                }
                (Decision::Allowed { label, .. }, Site::WebApp(site)) => {
                    self.http(*site, &head, rest, &mut send, recv, visitor, &label)
                        .await?
                }
                (refused @ (Decision::AwaitingApproval { .. } | Decision::Denied), _) => {
                    let reply = json!({"status": 403, "standing": standing(&refused)});
                    wire::write_line(&mut send, &reply).await?
                }
            }
        }
        send.shutdown().await?;
        Ok(())
    }

    /// The Visitor's jar and HTTP client, unless a Block came in since their request was admitted.
    fn client_for(&self, visitor: &str) -> Result<Option<SiteClient>> {
        self.jars.client_for(visitor, || {
            matches!(
                self.access.lock().unwrap().admits(visitor),
                Decision::Allowed { .. }
            )
        })
    }

    /// Bridge a WebSocket to the Site, as the Visitor (Label header, their cookies).
    #[allow(clippy::too_many_arguments)]
    async fn websocket(
        &self,
        site: SocketAddr,
        head: &Value,
        mut buf: Vec<u8>,
        send: &mut (impl AsyncWrite + Unpin + Send),
        mut recv: impl AsyncRead + Unpin + Send,
        visitor: &str,
        label: &str,
    ) -> Result<()> {
        let path = head["path"].as_str().unwrap_or("/");
        let mut req = format!("ws://{site}{path}").into_client_request()?;
        req.headers_mut()
            .insert(LABEL_HEADER, encode_label(label).parse()?);
        let Some(SiteClient { jar, .. }) = self.client_for(visitor)? else {
            return wire::write_line(send, &json!({"status": 403, "standing": "denied"})).await;
        };
        if let Some(cookies) = jar.cookies(&format!("http://{site}{path}").parse()?) {
            req.headers_mut().insert("cookie", cookies);
        }
        let protocols: Vec<&str> = head["protocols"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        if !protocols.is_empty() {
            req.headers_mut()
                .insert("sec-websocket-protocol", protocols.join(", ").parse()?);
        }
        let (ws, resp) = tokio_tungstenite::connect_async(req).await?;
        let protocol = resp
            .headers()
            .get("sec-websocket-protocol")
            .and_then(|v| v.to_str().ok());
        wire::write_line(
            send,
            &json!({"status": 101, "protocol": protocol.unwrap_or("")}),
        )
        .await?;
        let (mut to_site, mut from_site) = ws.split();

        let up = async {
            loop {
                while let Some((kind, payload)) = wire::take_frame(&mut buf) {
                    match kind {
                        wire::TEXT => {
                            to_site
                                .send(Message::text(String::from_utf8(payload)?))
                                .await?
                        }
                        wire::BINARY => to_site.send(Message::binary(payload)).await?,
                        _ => return Ok(to_site.close().await?),
                    }
                }
                if recv.read_buf(&mut buf).await? == 0 {
                    return anyhow::Ok(());
                }
            }
        };
        let down = async {
            while let Some(msg) = from_site.next().await {
                let (kind, payload) = match msg? {
                    Message::Text(t) => (wire::TEXT, t.as_bytes().to_vec()),
                    Message::Binary(b) => (wire::BINARY, b.to_vec()),
                    Message::Close(_) => (wire::CLOSE, vec![]),
                    _ => continue,
                };
                send.write_all(&wire::frame(kind, &payload)).await?;
                if kind == wire::CLOSE {
                    break;
                }
            }
            anyhow::Ok(())
        };
        let mut changes = self.changes.subscribe();
        let revoked = async {
            while changes.changed().await.is_ok() {
                if !matches!(
                    self.access.lock().unwrap().admits(visitor),
                    Decision::Allowed { .. }
                ) {
                    return;
                }
            }
            std::future::pending().await // Host shutting down: the socket ends with it
        };
        let blocked = tokio::select! {
            r = up => { r?; false }
            r = down => { r?; false }
            () = revoked => true,
        };
        if blocked {
            send.write_all(&wire::frame(wire::CLOSE, &[])).await?;
        }
        Ok(())
    }

    /// Forward one request to the Site, tagged with the Visitor's Label, and stream the reply back.
    #[allow(clippy::too_many_arguments)]
    async fn http(
        &self,
        site: SocketAddr,
        head: &Value,
        mut body: Vec<u8>,
        send: &mut (impl AsyncWrite + Unpin + Send),
        mut recv: impl AsyncRead + Unpin + Send + 'static,
        visitor: &str,
        label: &str,
    ) -> Result<()> {
        // The header read may already have brought the body, or all of a short one.
        let whole = loop {
            if body.len() > WHOLE_BODY {
                break false;
            }
            body.reserve(64 * 1024);
            if recv.read_buf(&mut body).await? == 0 {
                break true;
            }
        };
        let method =
            reqwest::Method::from_bytes(head["method"].as_str().unwrap_or("GET").as_bytes())?;
        let path = head["path"].as_str().unwrap_or("/");
        let Some(SiteClient { http, .. }) = self.client_for(visitor)? else {
            return wire::write_line(send, &json!({"status": 403, "standing": "denied"})).await;
        };
        let mut req = http
            .request(method, format!("http://{site}{path}"))
            .header(LABEL_HEADER, encode_label(label));
        for (k, v) in wire::request_headers(head).filter(|(k, _)| forwardable(k)) {
            req = req.header(k, v);
        }
        if !whole {
            // A Visitor whose stream breaks off mid-upload ends the stream with an error: the Site
            // sees the upload abandoned, never a short one that looks complete.
            let rest = futures::stream::try_unfold(recv, |mut recv| async move {
                let mut chunk = bytes::BytesMut::with_capacity(64 * 1024);
                Ok::<_, std::io::Error>(match recv.read_buf(&mut chunk).await? {
                    0 => None,
                    _ => Some((chunk.freeze(), recv)),
                })
            });
            let first = futures::stream::once(async { Ok(bytes::Bytes::from(body)) });
            req = req.body(reqwest::Body::wrap_stream(first.chain(rest)));
        } else if !body.is_empty() {
            req = req.body(body);
        }
        let Ok(resp) = req.send().await else {
            // The Site is down or unreachable: say so instead of resetting the Visitor's stream.
            return wire::write_line(send, &json!({"status": 502, "error": "site_unreachable"}))
                .await;
        };
        let reply = wire::response_head(resp.status(), resp.headers(), |k| !SKIP_RESP.contains(&k));
        wire::write_line(send, &reply).await?;
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            send.write_all(&chunk?).await?;
        }
        Ok(())
    }
}

/// Only paths on the Site itself: a Visitor must never make the Host fetch another server
/// (`@192.168.1.1/admin` or a full URL would otherwise reach the LAN or the internet).
fn on_site(site: &SocketAddr, path: &str) -> bool {
    // Compare host:port as text: SocketAddr and Url both bracket IPv6 ("[::1]:8123").
    path.starts_with('/')
        && reqwest::Url::parse(&format!("http://{site}{path}")).is_ok_and(|url| {
            format!(
                "{}:{}",
                url.host_str().unwrap_or(""),
                url.port().unwrap_or(80)
            ) == site.to_string()
        })
}

/// A Visitor's Standing on the wire (standing replies and refusals alike).
fn standing(decision: &Decision) -> &'static str {
    match decision {
        Decision::Allowed { .. } => "allowed",
        Decision::AwaitingApproval { .. } => "awaiting_approval",
        Decision::Denied => "denied",
    }
}
