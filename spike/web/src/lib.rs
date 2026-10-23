//! Browser side of the spike: an iroh endpoint that opens bidirectional streams to a tooob host.
//! JS does the HTTP/WebSocket framing; this crate only moves bytes.
use iroh::{
    Endpoint, EndpointAddr, EndpointId, RelayMode, RelayUrl, SecretKey,
    endpoint::{Connection, RecvStream, SendStream, presets},
};
use wasm_bindgen::prelude::*;

const ALPN: &[u8] = b"tooob/spike/0";

fn err(e: impl std::fmt::Display) -> JsError {
    JsError::new(&e.to_string())
}

#[wasm_bindgen]
pub struct Tooob {
    ep: Endpoint,
    conn: Connection,
}

#[wasm_bindgen]
impl Tooob {
    /// Dial a host by endpoint id. With `relay` (from the invite link) we go straight to the host's
    /// relay; without it, address lookup (pkarr over HTTPS) finds the relay first.
    /// `visitor_key` is the visitor's 32-byte secret, kept by the shell so the host recognizes
    /// this browser across reloads (its allowlist entry and cookie jar hang off that identity).
    pub async fn connect(host: String, relay: Option<String>, visitor_key: Vec<u8>) -> Result<Tooob, JsError> {
        console_error_panic_hook::set_once();
        let id: EndpointId = host.parse().map_err(err)?;
        let key: [u8; 32] = visitor_key.try_into().map_err(|_| err("visitor key must be 32 bytes"))?;
        let mut addr = EndpointAddr::new(id);
        let mut builder = Endpoint::builder(presets::N0).secret_key(SecretKey::from_bytes(&key));
        if let Some(relay) = relay {
            let url: RelayUrl = relay.parse().map_err(err)?;
            builder = builder.relay_mode(RelayMode::custom([url.clone()]));
            addr = addr.with_relay_url(url);
        }
        let ep = builder.bind().await.map_err(err)?;
        let conn = ep.connect(addr, ALPN).await.map_err(err)?;
        Ok(Tooob { ep, conn })
    }

    /// Open a stream. JS writes the request (or WebSocket frames) and reads the reply.
    pub async fn open(&self) -> Result<Bi, JsError> {
        let (send, recv) = self.conn.open_bi().await.map_err(err)?;
        Ok(Bi { send: Some(send), recv: Some(recv) })
    }

    /// This browser's endpoint id, i.e. what the host sees as the visitor.
    pub fn visitor_id(&self) -> String {
        self.ep.id().to_string()
    }

    /// Debug string describing the current network paths (relay vs direct).
    pub fn paths(&self) -> String {
        format!("{:?}", self.conn.paths())
    }
}

/// Split into a Reader and a Writer so JS can read and write concurrently.
#[wasm_bindgen]
pub struct Bi {
    send: Option<SendStream>,
    recv: Option<RecvStream>,
}

#[wasm_bindgen]
impl Bi {
    pub fn writer(&mut self) -> Option<Writer> {
        self.send.take().map(|s| Writer { s })
    }
    pub fn reader(&mut self) -> Option<Reader> {
        self.recv.take().map(|r| Reader { r })
    }
}

#[wasm_bindgen]
pub struct Writer {
    s: SendStream,
}

#[wasm_bindgen]
impl Writer {
    pub async fn write(&mut self, data: Vec<u8>) -> Result<(), JsError> {
        self.s.write_all(&data).await.map_err(err)
    }
    pub fn finish(&mut self) -> Result<(), JsError> {
        self.s.finish().map_err(err)
    }
}

#[wasm_bindgen]
pub struct Reader {
    r: RecvStream,
}

#[wasm_bindgen]
impl Reader {
    /// Next chunk of bytes, or undefined at end of stream.
    pub async fn read(&mut self) -> Result<Option<Vec<u8>>, JsError> {
        let chunk = self.r.read_chunk(64 * 1024).await.map_err(err)?;
        Ok(chunk.map(|c| c.to_vec()))
    }
}
