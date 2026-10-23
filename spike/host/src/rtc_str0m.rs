//! DIAGNOSTIC: the same direct path and mux protocol as rtc.rs, on str0m instead of webrtc-rs, to
//! tell an SCTP-implementation bug from a Chrome limit. Selected with TOOOB_RTC=str0m.
//! Host candidate only (str0m has no STUN client), so this only connects on the local network.
use crate::rtc::{CHUNK, DATA, FIN, RESET, message, next_line, pace};
use crate::{Proxy, Rd, Visitor, Wr, write_line};
use anyhow::{Context, Result};
use serde_json::json;
use std::{collections::HashMap, net::SocketAddr, sync::Arc, time::Instant};
use str0m::change::SdpOffer;
use str0m::channel::ChannelId;
use str0m::net::{Protocol, Receive};
use str0m::{Candidate, Event, IceConnectionState, Input, Output, Rtc};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UdpSocket,
    sync::{Notify, mpsc},
};

enum Cmd {
    RemoteCandidate(String),
    Send(Vec<u8>),
}

pub async fn negotiate(offer: String, mut buf: Vec<u8>, send: &mut Wr, mut recv: Rd, proxy: Proxy, visitor: Visitor) -> Result<()> {
    let socket = UdpSocket::bind("0.0.0.0:0").await?;
    let local = SocketAddr::new(lan_ip()?, socket.local_addr()?.port());
    let mut rtc = Rtc::builder().build(Instant::now());
    rtc.add_local_candidate(Candidate::host(local, "udp")?);
    let answer = rtc.sdp_api().accept_offer(SdpOffer::from_sdp_string(&offer)?)?;
    write_line(send, &json!({"status": 200, "sdp": answer.to_sdp_string()})).await?;

    let (cmds, cmd_rx) = mpsc::channel(64); // bounded: backpressure for the stream pumps
    let connected = Arc::new(Notify::new());
    tokio::spawn(run(rtc, socket, local, cmds.clone(), cmd_rx, connected.clone(), proxy, visitor));

    // Feed the browser's trickled candidates in until connected.
    let deadline = tokio::time::sleep(std::time::Duration::from_secs(10));
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            line = next_line(&mut recv, &mut buf) => match line? {
                Some(v) => if let Some(c) = v["candidate"]["candidate"].as_str() {
                    let _ = cmds.send(Cmd::RemoteCandidate(c.to_string())).await;
                },
                None => break,
            },
            _ = connected.notified() => break,
            _ = &mut deadline => break,
        }
    }
    Ok(())
}

/// The str0m event loop: owns the Rtc, the socket, and the mux state.
#[allow(clippy::too_many_arguments)]
async fn run(
    mut rtc: Rtc, socket: UdpSocket, local: SocketAddr, cmds: mpsc::Sender<Cmd>, mut cmd_rx: mpsc::Receiver<Cmd>,
    connected: Arc<Notify>, proxy: Proxy, visitor: Visitor,
) {
    let mut mux: Option<ChannelId> = None;
    let mut streams: HashMap<u32, (mpsc::UnboundedSender<Option<Vec<u8>>>, tokio::task::JoinHandle<()>)> = HashMap::new();
    let mut pending: Option<Vec<u8>> = None; // a message the channel wouldn't take yet
    let mut udp = vec![0u8; 2000];
    loop {
        // Drain output until str0m wants to wait.
        let timeout = loop {
            match rtc.poll_output() {
                Ok(Output::Timeout(t)) => break t,
                Ok(Output::Transmit(t)) => {
                    let _ = socket.send_to(&t.contents, t.destination).await;
                }
                Ok(Output::Event(e)) => match e {
                    Event::IceConnectionStateChange(IceConnectionState::Connected) => {
                        println!("direct path (str0m) {}: connected", &visitor.id[..8]);
                        connected.notify_one();
                    }
                    Event::IceConnectionStateChange(IceConnectionState::Disconnected) => return,
                    Event::ChannelOpen(cid, label) if label == "mux" => {
                        mux = Some(cid);
                        if let Some(mut ch) = rtc.channel(cid) {
                            ch.set_buffered_amount_low_threshold(256 * 1024);
                        }
                    }
                    Event::ChannelData(d) => {
                        let (Some(&kind), Some(id)) = (d.data.first(), d.data.get(1..5)) else { continue };
                        let id = u32::from_be_bytes(id.try_into().unwrap());
                        streams.retain(|_, (_, pump)| !pump.is_finished());
                        match kind {
                            DATA => {
                                let (to_stream, _) = streams.entry(id).or_insert_with(|| open(id, cmds.clone(), proxy.clone(), visitor.clone()));
                                let _ = to_stream.send(Some(d.data[5..].to_vec()));
                            }
                            FIN => {
                                if let Some((to_stream, _)) = streams.get(&id) {
                                    let _ = to_stream.send(None);
                                }
                            }
                            RESET => {
                                if let Some((_, pump)) = streams.remove(&id) {
                                    pump.abort();
                                }
                            }
                            _ => {}
                        }
                    }
                    Event::ChannelBufferedAmountLow(_) => {
                        if let (Some(msg), Some(mut ch)) = (pending.take(), mux.and_then(|c| rtc.channel(c))) {
                            if !ch.write(true, &msg).unwrap_or(false) {
                                pending = Some(msg);
                            }
                        }
                    }
                    _ => {}
                },
                Err(e) => {
                    eprintln!("str0m: {e}");
                    return;
                }
            }
        };
        // Retry a held message every pass: "buffered amount low" only fires on crossing the
        // threshold, so it never comes if write() refused for lack of SCTP space below it.
        if let (Some(msg), Some(mut ch)) = (pending.as_ref(), mux.and_then(|c| rtc.channel(c))) {
            if ch.buffered_amount() <= 1 << 20 && ch.write(true, msg).unwrap_or(false) {
                pending = None;
                continue; // writing produced output; drain it before waiting
            }
        }
        let blocked = pending.is_some();
        tokio::select! {
            r = socket.recv_from(&mut udp) => {
                let Ok((n, source)) = r else { return };
                if let Ok(contents) = udp[..n].try_into() {
                    let input = Input::Receive(Instant::now(), Receive { proto: Protocol::Udp, source, destination: local, contents });
                    if rtc.handle_input(input).is_err() { return }
                }
            }
            _ = tokio::time::sleep_until(tokio::time::Instant::from_std(timeout)) => {
                if rtc.handle_input(Input::Timeout(Instant::now())).is_err() { return }
            }
            Some(cmd) = cmd_rx.recv(), if !blocked => match cmd {
                Cmd::RemoteCandidate(s) => {
                    if let Ok(c) = Candidate::from_sdp_string(&s) {
                        rtc.add_remote_candidate(c); // mDNS (.local) candidates fail to parse; prflx covers them
                    }
                }
                Cmd::Send(msg) => {
                    let Some(mut ch) = mux.and_then(|c| rtc.channel(c)) else { continue };
                    if ch.buffered_amount() > 1 << 20 || !ch.write(true, &msg).unwrap_or(false) {
                        pending = Some(msg);
                    }
                }
            },
        }
        if !rtc.is_alive() {
            return;
        }
    }
}

/// Start one stream: proxy on one end of a duplex; a writer task feeding it browser data; a pump
/// sending the proxy's output back through the event loop.
fn open(id: u32, cmds: mpsc::Sender<Cmd>, proxy: Proxy, visitor: Visitor) -> (mpsc::UnboundedSender<Option<Vec<u8>>>, tokio::task::JoinHandle<()>) {
    let (proxy_side, pump_side) = tokio::io::duplex(256 * 1024);
    let (mut from_proxy, mut to_proxy) = tokio::io::split(pump_side);
    let (rd, wr) = tokio::io::split(proxy_side);
    tokio::spawn(async move {
        if let Err(e) = proxy.serve(Box::new(wr), Box::new(rd), &visitor, "direct").await {
            eprintln!("stream error: {e:#}");
        }
    });
    let (to_stream, mut inbound) = mpsc::unbounded_channel::<Option<Vec<u8>>>();
    tokio::spawn(async move {
        while let Some(chunk) = inbound.recv().await {
            match chunk {
                Some(b) => { if to_proxy.write_all(&b).await.is_err() { break } }
                None => { let _ = to_proxy.shutdown().await; break }
            }
        }
    });
    let pump = tokio::spawn(async move {
        let mut buf = vec![0u8; CHUNK];
        while let Ok(n) = from_proxy.read(&mut buf).await {
            if n > 0 {
                pace(n).await;
            }
            if n == 0 || cmds.send(Cmd::Send(message(DATA, id, &buf[..n]).to_vec())).await.is_err() {
                break;
            }
        }
        let _ = cmds.send(Cmd::Send(message(FIN, id, &[]).to_vec())).await;
    });
    (to_stream, pump)
}

/// The LAN address other machines (and the browser) would reach us on.
fn lan_ip() -> Result<std::net::IpAddr> {
    let s = std::net::UdpSocket::bind("0.0.0.0:0")?;
    s.connect("8.8.8.8:80")?; // no packets sent; just picks the outbound interface
    Ok(s.local_addr().context("no local address")?.ip())
}
