//! The Direct Path (ADR 0004): a WebRTC data channel between a Visitor's browser and the Host,
//! negotiated over the Visitor's iroh connection. The answer carries the Host's DTLS fingerprint
//! over iroh, which binds the Direct Path to the Host Key.
//!
//! Every stream shares one data channel ("mux"): it saves a channel-open round trip per request.
//! Each stream is served by the same request handling as an iroh stream.
//! Message: [kind u8][stream id u32 BE][payload]. Kinds: DATA, FIN (the sender is done: the
//! half-close QUIC gives for free), RESET (abandon the stream, e.g. the browser cancelled a video
//! range), and CREDIT (Host to browser: payload u32 BE, how many more of the stream's bytes it
//! may send).
//!
//! Credit is per-stream flow control, which QUIC gives iroh streams for free: a browser sends at
//! most [`WINDOW`] bytes of a stream the Host hasn't yet passed on to its request handling, so an
//! upload to a Site that takes it slowly waits on its own and never holds up the other streams.
//! A browser asks for it in its offer (`"credit": true`); the answer gives the window
//! (`"credit": WINDOW`). A browser that doesn't ask has the channel held up as before.
use crate::host::Admission;
use crate::wire;
use anyhow::{Context, Result};
use bytes::{Bytes, BytesMut};
use serde_json::json;
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    sync::{Notify, Semaphore, TryAcquireError, mpsc},
    task::JoinHandle,
};
use webrtc::data_channel::{DataChannel, DataChannelEvent};
use webrtc::peer_connection::{
    PeerConnection, PeerConnectionBuilder, PeerConnectionEventHandler, RTCConfigurationBuilder,
    RTCIceCandidateInit, RTCIceServer, RTCPeerConnectionIceEvent, RTCPeerConnectionState,
    RTCSessionDescription,
};

// ponytail: Google's public STUN server; run our own next to the Relay before launch.
const STUN: &str = "stun:stun.l.google.com:19302";
/// Well under every browser's largest data channel message.
const CHUNK: usize = 32 * 1024;
const DATA: u8 = 0;
const FIN: u8 = 1;
const RESET: u8 = 2;
const CREDIT: u8 = 3;
/// How much of a stream's input the Host holds for it at most: a browser that takes credit starts
/// each stream with this much and gets more as the request handling takes it.
const WINDOW: usize = 1 << 20;
/// How long ICE gets to connect before the Host stops listening for the browser's candidates.
const SETUP: Duration = Duration::from_secs(10);

/// The Direct Path of one iroh connection, if it has one. A browser keeps one per connection: a
/// new offer replaces it, and a restart offer renegotiates its ICE in place (after the network
/// changed under it), which keeps its data channel and the streams on it.
#[derive(Clone, Default)]
pub(crate) struct Slot(Arc<tokio::sync::Mutex<Option<Arc<Direct>>>>);

struct Direct {
    pc: Arc<dyn PeerConnection>,
    /// The Host's ICE candidates, for whichever signaling stream is open.
    ours: tokio::sync::Mutex<mpsc::UnboundedReceiver<RTCIceCandidateInit>>,
}

/// Answer the browser's offer, then trickle ICE candidates both ways over the same iroh stream
/// (one JSON line each: {"candidate": {...}}) until the browser is done signaling or setup gives
/// up. Sending candidates as they're found keeps setup to about one round trip plus the ICE checks.
/// The Direct Path lasts until it's replaced or `ended` (its iroh connection closing) resolves; one
/// that hasn't connected by the end of setup is closed then. A failed one stays, so a restart can
/// bring it back.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn open(
    slot: &Slot,
    offer: &str,
    credit: bool,
    buf: Vec<u8>,
    send: &mut (impl AsyncWrite + Unpin + Send),
    recv: impl AsyncRead + Unpin + Send,
    admission: Admission,
    visitor: String,
    ended: impl Future<Output = ()> + Send + 'static,
) -> Result<()> {
    let (candidates, ours) = mpsc::unbounded_channel();
    let handler = Arc::new(Handler {
        admission,
        visitor,
        credit,
        candidates,
        up: AtomicBool::new(false),
        closed: Notify::new(),
    });
    let config = RTCConfigurationBuilder::new()
        .with_ice_servers(vec![RTCIceServer {
            urls: vec![STUN.into()],
            ..Default::default()
        }])
        .build();
    let pc: Arc<dyn PeerConnection> = Arc::new(
        PeerConnectionBuilder::new()
            .with_configuration(config)
            .with_handler(handler.clone())
            .with_udp_addrs(vec!["0.0.0.0:0"])
            // send() waits instead of buffering a whole video. Kept small: it's shared by every
            // stream on the one channel, so it bounds how long a big response can delay a small one.
            .with_data_channel_send_buffer_limit(256 * 1024)
            .build()
            .await?,
    );
    let sdp = match answer(&*pc, offer).await {
        Ok(sdp) => sdp,
        Err(e) => {
            let _ = pc.close().await;
            wire::write_line(send, &json!({"status": 400, "error": "bad_offer"})).await?;
            return Err(e);
        }
    };
    let direct = Arc::new(Direct {
        pc: pc.clone(),
        ours: tokio::sync::Mutex::new(ours),
    });
    let replaced = slot.0.lock().await.replace(direct.clone());
    if let Some(old) = replaced {
        let _ = old.pc.close().await;
    }
    let answer = match credit {
        true => json!({"status": 200, "sdp": sdp, "credit": WINDOW}),
        false => json!({"status": 200, "sdp": sdp}),
    };
    wire::write_line(send, &answer).await?;

    let setup_ends = tokio::time::Instant::now() + SETUP;
    let slot = slot.clone();
    let signaled = signal(
        &*pc,
        &mut *direct.ours.lock().await,
        buf,
        send,
        recv,
        setup_ends,
    )
    .await;
    tokio::spawn(async move {
        // The browser gives up sooner and never says so: without this, its peer connection and UDP
        // sockets would last as long as the iroh connection.
        let never_connected = async {
            tokio::time::sleep_until(setup_ends).await;
            if handler.up.load(Ordering::Relaxed) {
                std::future::pending::<()>().await;
            }
        };
        tokio::select! {
            _ = handler.closed.notified() => {}
            _ = ended => {}
            _ = never_connected => {}
        }
        let _ = pc.close().await;
        // Gone for good: a restart now gets "no_direct_path", and the browser starts a new one.
        let mut held = slot.0.lock().await;
        if held.as_ref().is_some_and(|d| Arc::ptr_eq(d, &direct)) {
            *held = None;
        }
    });
    signaled
}

/// Renegotiate the connection's Direct Path with the browser's ICE restart offer: new ICE
/// credentials and candidates, signaled like the first time, on the same peer connection.
pub(crate) async fn restart(
    slot: &Slot,
    offer: &str,
    buf: Vec<u8>,
    send: &mut (impl AsyncWrite + Unpin + Send),
    recv: impl AsyncRead + Unpin + Send,
) -> Result<()> {
    let Some(direct) = slot.0.lock().await.clone() else {
        wire::write_line(send, &json!({"status": 404, "error": "no_direct_path"})).await?;
        return Ok(());
    };
    // One renegotiation at a time, and none of the last one's leftover candidates.
    let mut ours = direct.ours.lock().await;
    while ours.try_recv().is_ok() {}
    match answer(&*direct.pc, offer).await {
        Ok(sdp) => wire::write_line(send, &json!({"status": 200, "sdp": sdp})).await?,
        Err(e) => {
            wire::write_line(send, &json!({"status": 400, "error": "bad_offer"})).await?;
            return Err(e);
        }
    }
    let setup_ends = tokio::time::Instant::now() + SETUP;
    signal(&*direct.pc, &mut ours, buf, send, recv, setup_ends).await
}

/// Apply the browser's offer; the answer's SDP.
async fn answer(pc: &dyn PeerConnection, offer: &str) -> Result<String> {
    pc.set_remote_description(RTCSessionDescription::offer(offer.to_string())?)
        .await?;
    let answer = pc.create_answer(None).await?;
    pc.set_local_description(answer).await?;
    Ok(pc
        .local_description()
        .await
        .context("no local description")?
        .sdp)
}

/// Trickle candidates both ways until the browser is done signaling or `until`.
async fn signal(
    pc: &dyn PeerConnection,
    ours: &mut mpsc::UnboundedReceiver<RTCIceCandidateInit>,
    mut buf: Vec<u8>,
    send: &mut (impl AsyncWrite + Unpin + Send),
    mut recv: impl AsyncRead + Unpin + Send,
    until: tokio::time::Instant,
) -> Result<()> {
    loop {
        tokio::select! {
            Some(c) = ours.recv() => wire::write_line(send, &json!({"candidate": c})).await?,
            line = wire::next_line(&mut recv, &mut buf) => match line? {
                Some(v) => if let Some(c) = v.get("candidate") {
                    pc.add_ice_candidate(serde_json::from_value::<RTCIceCandidateInit>(c.clone())?).await?;
                },
                None => break, // the browser is done signaling
            },
            _ = tokio::time::sleep_until(until) => break,
        }
    }
    Ok(())
}

struct Handler {
    admission: Admission,
    visitor: String,
    /// The browser takes credit: see the module docs.
    credit: bool,
    candidates: mpsc::UnboundedSender<RTCIceCandidateInit>,
    /// Connected at least once.
    up: AtomicBool,
    closed: Notify,
}

#[async_trait::async_trait]
impl PeerConnectionEventHandler for Handler {
    async fn on_ice_candidate(&self, event: RTCPeerConnectionIceEvent) {
        if let Ok(c) = event.candidate.to_json() {
            let _ = self.candidates.send(c);
        }
    }

    async fn on_connection_state_change(&self, state: RTCPeerConnectionState) {
        match state {
            RTCPeerConnectionState::Connected => self.up.store(true, Ordering::Relaxed),
            RTCPeerConnectionState::Closed => self.closed.notify_one(),
            _ => {}
        }
    }

    async fn on_data_channel(&self, dc: Arc<dyn DataChannel>) {
        let (admission, visitor, credit) =
            (self.admission.clone(), self.visitor.clone(), self.credit);
        tokio::spawn(async move {
            if let Err(e) = mux(dc, admission, visitor, credit).await {
                eprintln!("tooob: direct path: {e:#}");
            }
        });
    }
}

fn message(kind: u8, id: u32, payload: &[u8]) -> BytesMut {
    let mut m = BytesMut::with_capacity(5 + payload.len());
    m.extend_from_slice(&[kind]);
    m.extend_from_slice(&id.to_be_bytes());
    m.extend_from_slice(payload);
    m
}

/// One stream: its input on its way to the request handling, the request handling, and its reply
/// pump.
struct Stream {
    /// Input for the feeder to pass on; dropped at FIN.
    input: Option<mpsc::UnboundedSender<Bytes>>,
    /// Input bytes the stream may still have outstanding: taken as they arrive, given back as the
    /// request handling takes them. Closed once the request handling wants no more.
    room: Arc<Semaphore>,
    feeder: JoinHandle<()>,
    serving: JoinHandle<()>,
    pump: JoinHandle<()>,
}

impl Stream {
    /// Abandon the stream. The request handling stops first: once its input pipe goes, it would
    /// read a clean end there and pass a broken-off upload to the Site as whole.
    async fn reset(self) {
        self.serving.abort();
        let _ = self.serving.await;
        self.feeder.abort();
        self.pump.abort();
    }
}

/// Demultiplex the channel: each stream id gets its request handling, a feeder passing its input
/// on, and a task pumping its reply back as DATA messages and a final FIN.
async fn mux(
    dc: Arc<dyn DataChannel>,
    admission: Admission,
    visitor: String,
    credit: bool,
) -> Result<()> {
    let mut streams: HashMap<u32, Stream> = HashMap::new();
    // The browser numbers its streams 1, 2, 3...: an id up to this one is a stream that's over,
    // whose late body bytes must not start a request of their own.
    let mut newest = 0;
    while let Some(ev) = dc.poll().await {
        let mut m = match ev {
            DataChannelEvent::OnMessage(m) => m,
            DataChannelEvent::OnClose => break,
            _ => continue,
        };
        let (Some(&kind), Some(id)) = (m.data.first(), m.data.get(1..5)) else {
            continue;
        };
        let id = u32::from_be_bytes(id.try_into()?);
        streams.retain(|_, stream| !stream.pump.is_finished());
        match kind {
            DATA => {
                if id > newest {
                    newest = id;
                    let stream = serve(id, dc.clone(), admission.clone(), visitor.clone(), credit);
                    streams.insert(id, stream);
                }
                let Some(stream) = streams.get(&id) else {
                    continue;
                };
                let (room, input) = (stream.room.clone(), stream.input.clone());
                let payload = m.data.split_off(5).freeze();
                let n = taken(&payload);
                let room = if credit {
                    // Never waits: a browser that takes credit stays within it.
                    match room.try_acquire_many_owned(n) {
                        Err(TryAcquireError::NoPermits) => {
                            eprintln!(
                                "tooob: direct stream from {}: sent past its credit",
                                &visitor[..8]
                            );
                            if let Some(stream) = streams.remove(&id) {
                                stream.reset().await;
                            }
                            continue;
                        }
                        room => room.ok(),
                    }
                } else {
                    // A browser that doesn't: the whole channel waits for this stream, as before.
                    room.acquire_many_owned(n).await.ok()
                };
                // No room left means the request is done with its input (a refusal never reads the
                // body), not the channel failing: the rest is dropped.
                if let (Some(room), Some(input)) = (room, input) {
                    room.forget();
                    let _ = input.send(payload);
                }
            }
            FIN => {
                if let Some(stream) = streams.get_mut(&id) {
                    stream.input = None;
                }
            }
            RESET => {
                if let Some(stream) = streams.remove(&id) {
                    stream.reset().await;
                }
            }
            _ => {}
        }
    }
    // The channel closed: every stream still open on it is broken off.
    for stream in streams.into_values() {
        stream.reset().await;
    }
    Ok(())
}

/// The room a message's payload takes: all of it, though never more than the whole window (a
/// message that big would otherwise wait for room forever).
fn taken(payload: &[u8]) -> u32 {
    payload.len().min(WINDOW) as u32
}

/// Start one stream: the request handling on one end of a pipe; on the other, the feeder passing
/// its input in (and, with `credit`, telling the browser it may send that much more) and the reply
/// pump.
fn serve(
    id: u32,
    dc: Arc<dyn DataChannel>,
    admission: Admission,
    visitor: String,
    credit: bool,
) -> Stream {
    let (host_side, pump_side) = tokio::io::duplex(256 * 1024);
    let (mut reply, mut to_host) = tokio::io::split(pump_side);
    let (recv, send) = tokio::io::split(host_side);
    let serving = tokio::spawn(async move {
        if let Err(e) = admission.stream(send, recv, &visitor).await {
            eprintln!("tooob: direct stream from {}: {e:#}", &visitor[..8]);
        }
    });
    let (input, mut queue) = mpsc::unbounded_channel::<Bytes>();
    let room = Arc::new(Semaphore::new(WINDOW));
    let feeder = tokio::spawn({
        let (dc, room) = (dc.clone(), room.clone());
        async move {
            while let Some(bytes) = queue.recv().await {
                if to_host.write_all(&bytes).await.is_err() {
                    break; // the request handling is done with its input
                }
                room.add_permits(taken(&bytes) as usize);
                if credit {
                    let more = (bytes.len() as u32).to_be_bytes();
                    if dc.send(message(CREDIT, id, &more)).await.is_err() {
                        break;
                    }
                }
            }
            // No more room: the browser gets no more credit, and what it sends anyway is dropped.
            room.close();
            let _ = to_host.shutdown().await;
        }
    });
    let pump = tokio::spawn(async move {
        let mut buf = vec![0u8; CHUNK];
        loop {
            match reply.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    pace(n).await;
                    if dc.send(message(DATA, id, &buf[..n])).await.is_err() {
                        return;
                    }
                }
            }
        }
        let _ = dc.send(message(FIN, id, &[])).await;
    });
    Stream {
        input: Some(input),
        room,
        feeder,
        serving,
        pump,
    }
}

/// Cap the Direct Path's total send rate. Native senders bursting at more than ~100-200 Mbit/s into
/// Chrome lose packets, and SCTP's >= 1 s retransmission timeout turns each loss into a stall
/// (spike 2: 6/10 page loads stalled unpaced, 0/10 at 100 Mbit/s; webrtc-rs/webrtc#938).
/// TOOOB_PACE_MBPS overrides the default; 0 turns pacing off.
// ponytail: one fixed cap shared by every Visitor; adapt it to the measured path rate later.
const DEFAULT_PACE_MBPS: f64 = 80.0;
static NEXT_SEND: std::sync::Mutex<Option<tokio::time::Instant>> = std::sync::Mutex::new(None);

async fn pace(bytes: usize) {
    static MBPS: std::sync::OnceLock<f64> = std::sync::OnceLock::new();
    let mbps = *MBPS.get_or_init(|| {
        std::env::var("TOOOB_PACE_MBPS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_PACE_MBPS)
    });
    if mbps <= 0.0 {
        return;
    }
    let cost = Duration::from_secs_f64(bytes as f64 * 8.0 / (mbps * 1e6));
    let wait_until = {
        let mut next = NEXT_SEND.lock().unwrap();
        let now = tokio::time::Instant::now();
        let at = next.map_or(now, |t| t.max(now));
        *next = Some(at + cost);
        at
    };
    tokio::time::sleep_until(wait_until).await;
}
