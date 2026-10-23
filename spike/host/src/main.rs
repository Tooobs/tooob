//! Host side of the spike: accept tooob streams over iroh and proxy them to a local HTTP server.
//!
//! Wire format per bidirectional stream (client opens it):
//!   client -> host: one JSON line, then the request body (HTTP) or frames (WebSocket)
//!   host -> client: one JSON line {"status":..}, then the response body or frames
//! WebSocket frame: [kind u8: 0 text, 1 binary, 2 close][len u32 BE][payload]
//!
//! A stream is an iroh bidirectional stream, or (direct path) a WebRTC data channel; see rtc.rs.
//! The first iroh stream can instead carry {"t":"rtc","sdp":offer} to set up the direct path.
mod rtc;
mod rtc_str0m;

use anyhow::{Context, Result, bail};
use futures::{SinkExt, StreamExt};
use iroh::{
    Endpoint, RelayMode, RelayUrl, SecretKey,
    endpoint::{Connection, presets},
    protocol::{AcceptError, ProtocolHandler, Router},
};
use reqwest::cookie::{CookieStore, Jar};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};

const ALPN: &[u8] = b"tooob/spike/0";
const KEY_FILE: &str = "tooob-host.key";
// Hop-by-hop or recomputed headers we never forward.
const SKIP_REQ: &[&str] = &["host", "connection", "accept-encoding", "content-length", "keep-alive", "upgrade"];
// set-cookie stays on the host: each visitor gets a cookie jar here, so HttpOnly cookies never reach the browser.
const SKIP_RESP: &[&str] = &["connection", "transfer-encoding", "keep-alive", "set-cookie"];

type Rd = Box<dyn AsyncRead + Unpin + Send>;
type Wr = Box<dyn AsyncWrite + Unpin + Send>;

#[tokio::main]
async fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let port: u16 = args.next().context("usage: host <local-port> [relay-url]")?.parse()?;
    let mut builder = Endpoint::builder(presets::N0).secret_key(load_key()?);
    if let Some(relay) = args.next() {
        builder = builder.relay_mode(RelayMode::custom([relay.parse::<RelayUrl>()?]));
    }
    let ep = builder.bind().await?;
    let router = Router::builder(ep)
        .accept(ALPN, Proxy { port, visitors: Default::default() })
        .spawn();
    router.endpoint().online().await;
    let addr = router.endpoint().addr();
    let id = addr.id;
    // The invite carries the home relay so visitors skip the address lookup.
    let relay = addr.relay_urls().next().map(|u| format!("?relay={u}")).unwrap_or_default();
    println!("tooob host {id} -> http://127.0.0.1:{port}");
    println!("visit: http://localhost:8088/{relay}#{id}");
    tokio::signal::ctrl_c().await?;
    router.shutdown().await?;
    Ok(())
}

/// Keep the host key across restarts so the visit URL stays the same.
fn load_key() -> Result<SecretKey> {
    if let Ok(bytes) = std::fs::read(KEY_FILE) {
        let bytes: [u8; 32] = bytes.try_into().ok().context("bad key file")?;
        return Ok(SecretKey::from_bytes(&bytes));
    }
    let key = SecretKey::generate();
    std::fs::write(KEY_FILE, key.to_bytes())?;
    Ok(key)
}

#[derive(Debug, Clone)]
struct Proxy {
    port: u16,
    // ponytail: in-memory and unbounded; a host restart logs every visitor out. Persist with the visitor db.
    visitors: Arc<Mutex<HashMap<String, Visitor>>>,
}

/// Per-visitor HTTP client and the cookie jar it fills.
#[derive(Debug, Clone)]
struct Visitor {
    id: String,
    http: reqwest::Client,
    jar: Arc<Jar>,
}

impl ProtocolHandler for Proxy {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        let id = conn.remote_id().to_string();
        println!("visitor connected: {id}");
        // ponytail: no allowlist yet, the spike only tests transport.
        let visitor = self.visitors.lock().unwrap().entry(id.clone()).or_insert_with(|| {
            let jar = Arc::new(Jar::default());
            let http = reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .cookie_provider(jar.clone())
                .build()
                .expect("http client");
            Visitor { id, http, jar }
        }).clone();
        while let Ok((send, recv)) = conn.accept_bi().await {
            let (this, visitor) = (self.clone(), visitor.clone());
            tokio::spawn(async move {
                if let Err(e) = this.serve(Box::new(send), Box::new(recv), &visitor, "iroh").await {
                    eprintln!("stream error: {e:#}");
                }
            });
        }
        Ok(())
    }
}

impl Proxy {
    /// Serve one stream. `via` is the path it arrived on: "iroh" (possibly relayed) or "direct".
    async fn serve(&self, mut send: Wr, mut recv: Rd, visitor: &Visitor, via: &str) -> Result<()> {
        let (head, rest) = read_line(&mut recv).await?;
        let res = match head["t"].as_str() {
            Some("ws") => self.websocket(&head, rest, &mut send, recv, visitor).await,
            // Signaling only over iroh: that's what ties the direct path to this visitor's key.
            Some("rtc") if via == "iroh" => {
                let offer = head["sdp"].as_str().context("rtc: missing sdp")?.to_string();
                if std::env::var("TOOOB_RTC").as_deref() == Ok("str0m") {
                    rtc_str0m::negotiate(offer, rest, &mut send, recv, self.clone(), visitor.clone()).await
                } else {
                    rtc::negotiate(offer, rest, &mut send, recv, self.clone(), visitor.clone()).await
                }
            }
            _ => self.http(&head, rest, &mut send, recv, visitor, via).await,
        };
        if let Err(e) = &res {
            // Best effort: the header may already be sent, in which case this just ends the body.
            let _ = write_line(&mut send, &json!({"status": 502, "error": format!("{e:#}")})).await;
        }
        let _ = send.shutdown().await;
        res
    }

    async fn http(&self, head: &Value, mut body: Vec<u8>, send: &mut Wr, recv: Rd, visitor: &Visitor, via: &str) -> Result<()> {
        recv.take(64 << 20).read_to_end(&mut body).await?;
        let method = reqwest::Method::from_bytes(head["method"].as_str().unwrap_or("GET").as_bytes())?;
        let path = head["path"].as_str().unwrap_or("/");
        println!("{method} {path} via {via}");
        let mut req = visitor
            .http
            .request(method, format!("http://127.0.0.1:{}{path}", self.port))
            .header("x-tooob-visitor", &visitor.id);
        for (k, v) in headers(head) {
            if !SKIP_REQ.contains(&k.to_ascii_lowercase().as_str()) {
                req = req.header(k, v);
            }
        }
        if !body.is_empty() {
            req = req.body(body);
        }
        let resp = req.send().await?;
        let resp_headers: Vec<_> = resp
            .headers()
            .iter()
            .filter(|(k, _)| !SKIP_RESP.contains(&k.as_str()))
            .map(|(k, v)| json!([k.as_str(), v.to_str().unwrap_or("")]))
            .collect();
        write_line(send, &json!({"status": resp.status().as_u16(), "headers": resp_headers})).await?;
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            send.write_all(&chunk?).await?;
        }
        Ok(())
    }

    async fn websocket(&self, head: &Value, mut buf: Vec<u8>, send: &mut Wr, mut recv: Rd, visitor: &Visitor) -> Result<()> {
        let path = head["path"].as_str().unwrap_or("/");
        let mut req = format!("ws://127.0.0.1:{}{path}", self.port).into_client_request()?;
        if let Some(cookies) = visitor.jar.cookies(&format!("http://127.0.0.1:{}{path}", self.port).parse()?) {
            req.headers_mut().insert("cookie", cookies);
        }
        let protocols: Vec<&str> = head["protocols"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
        if !protocols.is_empty() {
            req.headers_mut().insert("sec-websocket-protocol", protocols.join(", ").parse()?);
        }
        let (ws, resp) = tokio_tungstenite::connect_async(req).await?;
        let protocol = resp.headers().get("sec-websocket-protocol").and_then(|v| v.to_str().ok()).unwrap_or("");
        write_line(send, &json!({"status": 101, "protocol": protocol})).await?;
        let (mut ws_tx, mut ws_rx) = ws.split();

        let up = async {
            // tooob stream -> local websocket
            loop {
                while let Some((kind, payload, used)) = parse_frame(&buf) {
                    buf.drain(..used);
                    match kind {
                        0 => ws_tx.send(Message::text(String::from_utf8(payload)?)).await?,
                        1 => ws_tx.send(Message::binary(payload)).await?,
                        _ => {
                            ws_tx.close().await?;
                            return Ok(());
                        }
                    }
                }
                if recv.read_buf(&mut buf).await? == 0 {
                    return anyhow::Ok(());
                }
            }
        };
        let down = async {
            // local websocket -> tooob stream
            while let Some(msg) = ws_rx.next().await {
                let (kind, payload) = match msg? {
                    Message::Text(t) => (0u8, t.as_bytes().to_vec()),
                    Message::Binary(b) => (1, b.to_vec()),
                    Message::Close(_) => (2, vec![]),
                    _ => continue,
                };
                send.write_all(&frame(kind, &payload)).await?;
                if kind == 2 {
                    break;
                }
            }
            anyhow::Ok(())
        };
        tokio::select! { r = up => r, r = down => r }
    }
}

fn headers(head: &Value) -> impl Iterator<Item = (&str, &str)> {
    head["headers"].as_array().into_iter().flatten().filter_map(|h| Some((h[0].as_str()?, h[1].as_str()?)))
}

/// Read up to the first newline; return the parsed JSON and any bytes after it.
async fn read_line(recv: &mut Rd) -> Result<(Value, Vec<u8>)> {
    let mut buf = Vec::new();
    loop {
        if let Some(i) = buf.iter().position(|&b| b == b'\n') {
            let rest = buf.split_off(i + 1);
            return Ok((serde_json::from_slice(&buf[..i])?, rest));
        }
        if buf.len() > 1 << 20 {
            bail!("header line too long");
        }
        if recv.read_buf(&mut buf).await? == 0 {
            bail!("stream ended before header");
        }
    }
}

async fn write_line(send: &mut Wr, v: &Value) -> Result<()> {
    let mut line = serde_json::to_vec(v)?;
    line.push(b'\n');
    send.write_all(&line).await?;
    Ok(())
}

fn frame(kind: u8, payload: &[u8]) -> Vec<u8> {
    let mut f = Vec::with_capacity(5 + payload.len());
    f.push(kind);
    f.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    f.extend_from_slice(payload);
    f
}

/// Returns (kind, payload, bytes consumed) when a whole frame is buffered.
fn parse_frame(buf: &[u8]) -> Option<(u8, Vec<u8>, usize)> {
    let len = u32::from_be_bytes(buf.get(1..5)?.try_into().ok()?) as usize;
    Some((buf[0], buf.get(5..5 + len)?.to_vec(), 5 + len))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_roundtrip() {
        let mut buf = frame(0, b"hi");
        buf.extend(frame(1, &[9; 3]));
        let (k, p, n) = parse_frame(&buf).unwrap();
        assert_eq!((k, p.as_slice(), n), (0, &b"hi"[..], 7));
        assert!(parse_frame(&buf[7..9]).is_none(), "partial frame must wait");
        let (k, p, _) = parse_frame(&buf[7..]).unwrap();
        assert_eq!((k, p), (1, vec![9; 3]));
    }
}
