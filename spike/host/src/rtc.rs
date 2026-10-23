//! Direct browser path: a WebRTC peer connection, negotiated over the visitor's (authenticated)
//! iroh connection. All tooob streams share one data channel ("mux"); each is bridged into the
//! same proxy code as an iroh stream.
//!
//! One channel, not one per stream: it saves a channel-open round trip per request and is simpler.
//! (It did NOT fix the spike 2 stalls, as first thought: those came from bursting faster than
//! ~100–200 Mbit/s into Chrome, which loses packets that SCTP's ≥1 s retransmission timeout turns
//! into 1–13 s stalls, with webrtc-rs and str0m alike. `pace` below fixes that; see
//! webrtc-rs/webrtc#938.)
//!
//! Message: [kind u8][stream id u32 BE][payload]. Kinds: DATA, FIN (sender is done; the half-close
//! QUIC gives for free), RESET (abandon the stream, e.g. the browser cancelled a video range).
use crate::{Proxy, Rd, Visitor, Wr, write_line};
use anyhow::{Context, Result};
use serde_json::{Value, json};
use bytes::BytesMut;
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{Notify, mpsc},
};
use webrtc::data_channel::{DataChannel, DataChannelEvent};
use webrtc::peer_connection::{
    PeerConnection, PeerConnectionBuilder, PeerConnectionEventHandler, RTCConfigurationBuilder,
    RTCIceCandidateInit, RTCIceServer, RTCPeerConnectionIceEvent, RTCPeerConnectionState,
    RTCSessionDescription,
};

// ponytail: Google's public STUN server; run our own next to the relay before launch.
const STUN: &str = "stun:stun.l.google.com:19302";
pub(crate) const CHUNK: usize = 32 * 1024; // well under every browser's max data channel message size
pub(crate) const DATA: u8 = 0;
pub(crate) const FIN: u8 = 1;
pub(crate) const RESET: u8 = 2;

/// Answer the browser's offer, then trickle ICE candidates both ways over the same iroh stream
/// (one JSON line each: {"candidate": {...}}) until the direct path connects or we give up.
/// Sending candidates as they're found keeps setup to about one round trip plus the ICE checks.
pub async fn negotiate(offer: String, mut buf: Vec<u8>, send: &mut Wr, mut recv: Rd, proxy: Proxy, visitor: Visitor) -> Result<()> {
    let (candidates, mut ours) = mpsc::unbounded_channel();
    let handler = Arc::new(Handler { proxy, visitor, candidates, connected: Notify::new(), done: Notify::new() });
    let config = RTCConfigurationBuilder::new()
        .with_ice_servers(vec![RTCIceServer { urls: vec![STUN.into()], ..Default::default() }])
        .build();
    let pc = PeerConnectionBuilder::new()
        .with_configuration(config)
        .with_handler(handler.clone())
        .with_udp_addrs(vec!["0.0.0.0:0"])
        // send() waits instead of buffering a whole video. Kept small: it's shared by every stream on
        // the one channel, so it bounds how long a big response can delay a small one.
        .with_data_channel_send_buffer_limit(256 * 1024)
        .build()
        .await?;
    pc.set_remote_description(RTCSessionDescription::offer(offer)?).await?;
    let answer = pc.create_answer(None).await?;
    pc.set_local_description(answer).await?;
    let sdp = pc.local_description().await.context("no local description")?.sdp;
    write_line(send, &json!({"status": 200, "sdp": sdp})).await?;

    let deadline = tokio::time::sleep(Duration::from_secs(10));
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            Some(c) = ours.recv() => write_line(send, &json!({"candidate": c})).await?,
            line = next_line(&mut recv, &mut buf) => match line? {
                Some(v) => if let Some(c) = v.get("candidate") {
                    pc.add_ice_candidate(serde_json::from_value::<RTCIceCandidateInit>(c.clone())?).await?;
                },
                None => break, // browser is done signaling
            },
            _ = handler.connected.notified() => break,
            _ = &mut deadline => break,
        }
    }
    // Keep the peer connection alive until it fails or closes.
    tokio::spawn(async move {
        handler.done.notified().await;
        let _ = pc.close().await;
    });
    Ok(())
}

/// Next JSON line from the stream, or None at end of stream. Cancel-safe: partial input stays in `buf`.
pub(crate) async fn next_line(recv: &mut Rd, buf: &mut Vec<u8>) -> Result<Option<Value>> {
    loop {
        if let Some(i) = buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = buf.drain(..=i).collect();
            return Ok(Some(serde_json::from_slice(&line[..i])?));
        }
        if recv.read_buf(buf).await? == 0 {
            return Ok(None);
        }
    }
}

struct Handler {
    proxy: Proxy,
    visitor: Visitor,
    candidates: mpsc::UnboundedSender<RTCIceCandidateInit>,
    connected: Notify,
    done: Notify,
}

#[async_trait::async_trait]
impl PeerConnectionEventHandler for Handler {
    async fn on_ice_candidate(&self, event: RTCPeerConnectionIceEvent) {
        if let Ok(c) = event.candidate.to_json() {
            let _ = self.candidates.send(c);
        }
    }

    async fn on_connection_state_change(&self, state: RTCPeerConnectionState) {
        println!("direct path {}: {state}", &self.visitor.id[..8]);
        match state {
            RTCPeerConnectionState::Connected => self.connected.notify_one(),
            RTCPeerConnectionState::Failed | RTCPeerConnectionState::Closed => self.done.notify_one(),
            _ => {}
        }
    }

    async fn on_data_channel(&self, dc: Arc<dyn DataChannel>) {
        let (proxy, visitor) = (self.proxy.clone(), self.visitor.clone());
        tokio::spawn(async move {
            if let Err(e) = mux(dc, proxy, visitor).await {
                eprintln!("data channel error: {e:#}");
            }
        });
    }
}

pub(crate) fn message(kind: u8, id: u32, payload: &[u8]) -> BytesMut {
    let mut m = BytesMut::with_capacity(5 + payload.len());
    m.extend_from_slice(&[kind]);
    m.extend_from_slice(&id.to_be_bytes());
    m.extend_from_slice(payload);
    m
}

/// Demultiplex the channel: each stream id gets an in-memory duplex into the proxy, plus a task
/// pumping the proxy's output back as DATA messages and a final FIN.
async fn mux(dc: Arc<dyn DataChannel>, proxy: Proxy, visitor: Visitor) -> Result<()> {
    let mut streams = HashMap::new();
    while let Some(ev) = dc.poll().await {
        let m = match ev {
            DataChannelEvent::OnMessage(m) => m,
            DataChannelEvent::OnClose => break,
            _ => continue,
        };
        let (Some(&kind), Some(id)) = (m.data.first(), m.data.get(1..5)) else { continue };
        let id = u32::from_be_bytes(id.try_into()?);
        streams.retain(|_, (_, pump): &mut (_, tokio::task::JoinHandle<()>)| !pump.is_finished());
        match kind {
            DATA => {
                let (to_proxy, _) = streams.entry(id).or_insert_with(|| open(id, dc.clone(), proxy.clone(), visitor.clone()));
                to_proxy.write_all(&m.data[5..]).await?;
            }
            FIN => {
                if let Some((to_proxy, _)) = streams.get_mut(&id) {
                    to_proxy.shutdown().await?;
                }
            }
            RESET => {
                if let Some((_, pump)) = streams.remove(&id) {
                    pump.abort(); // drops the proxy's output pipe, so the proxy stops too
                }
            }
            _ => {}
        }
    }
    for (_, pump) in streams.into_values() {
        pump.abort();
    }
    Ok(())
}

/// Start one stream: proxy task on one end of a duplex, output pump on the other.
fn open(id: u32, dc: Arc<dyn DataChannel>, proxy: Proxy, visitor: Visitor) -> (tokio::io::WriteHalf<tokio::io::DuplexStream>, tokio::task::JoinHandle<()>) {
    let (proxy_side, pump_side) = tokio::io::duplex(256 * 1024);
    let (mut from_proxy, to_proxy) = tokio::io::split(pump_side);
    let (rd, wr) = tokio::io::split(proxy_side);
    tokio::spawn(async move {
        if let Err(e) = proxy.serve(Box::new(wr), Box::new(rd), &visitor, "direct").await {
            eprintln!("stream error: {e:#}");
        }
    });
    let pump = tokio::spawn(async move {
        let mut buf = vec![0u8; CHUNK];
        loop {
            match from_proxy.read(&mut buf).await {
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
    (to_proxy, pump)
}

/// Cap the direct path's total send rate. Native senders bursting at >~100-200 Mbit/s into Chrome
/// lose packets, and SCTP's ≥1 s retransmission timeout turns each loss into a stall (spike 2:
/// 6/10 page loads stalled unpaced, 0/10 at 100 Mbit/s). TOOOB_PACE_MBPS overrides; 0 = off.
// ponytail: one fixed cap shared by every visitor; adapt it to the measured path rate later.
const DEFAULT_PACE_MBPS: f64 = 80.0;
static NEXT_SEND: std::sync::Mutex<Option<tokio::time::Instant>> = std::sync::Mutex::new(None);
pub(crate) async fn pace(bytes: usize) {
    let mbps = std::env::var("TOOOB_PACE_MBPS").ok().and_then(|v| v.parse::<f64>().ok()).unwrap_or(DEFAULT_PACE_MBPS);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_layout() {
        assert_eq!(&message(FIN, 0x0102_0304, b"hi")[..], &[1, 1, 2, 3, 4, b'h', b'i']);
    }
}
