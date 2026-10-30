//! Browser side of tooob: an iroh endpoint that opens bidirectional streams to a Host.
//! JS does the HTTP/WebSocket framing; this crate only moves bytes.
use iroh::{
    Endpoint, EndpointAddr, EndpointId, RelayMode, RelayUrl, SecretKey,
    endpoint::{Connection, RecvStream, SendStream, presets},
};
use wasm_bindgen::prelude::*;

/// The Protocol Version this Bootstrap speaks: a Host must speak the same one (ADR 0017).
/// bootstrap/manifest.mjs reads it from this line.
const PROTOCOL_VERSION: &str = "tooob/0";
const ALPN: &[u8] = PROTOCOL_VERSION.as_bytes();

fn err(e: impl std::fmt::Display) -> JsError {
    JsError::new(&e.to_string())
}

#[wasm_bindgen]
pub struct Tooob {
    ep: Endpoint, // dropping it would close the connection
    host_key: EndpointId,
    conn: Connection,
}

#[wasm_bindgen]
impl Tooob {
    /// Dial a Host by its Host Key (z-base-32, as a Key Address carries it). With `relay` (the
    /// Key Address's own domain) we go straight to the Host's Relay; without it, address lookup
    /// (pkarr over HTTPS) finds the Relay first.
    /// The connection uses a key of its own, so each tab has its own Relay connection; JS then
    /// sends a hello proving the Visitor Key, which never enters WASM (ADR 0011).
    pub async fn connect(host_key: String, relay: Option<String>) -> Result<Tooob, JsError> {
        console_error_panic_hook::set_once();
        let host_key = EndpointId::from_z32(&host_key).map_err(err)?;
        let mut addr = EndpointAddr::new(host_key);
        let mut builder = Endpoint::builder(presets::N0).secret_key(SecretKey::generate());
        if let Some(relay) = relay {
            let url: RelayUrl = relay.parse().map_err(err)?;
            builder = builder.relay_mode(RelayMode::custom([url.clone()]));
            addr = addr.with_relay_url(url);
        }
        let ep = builder.bind().await.map_err(err)?;
        let conn = ep.connect(addr, ALPN).await.map_err(err)?;
        Ok(Tooob { ep, host_key, conn })
    }

    /// What the hello's signature covers: the protocol, the Host and this connection, the same
    /// bytes as `hello_message` in host/src/wire.rs. JS signs it with the Visitor Key.
    pub fn hello_message(&self) -> Vec<u8> {
        [
            PROTOCOL_VERSION.as_bytes(),
            b" hello",
            self.host_key.as_bytes(),
            self.ep.id().as_bytes(),
        ]
        .concat()
    }

    /// Open a stream. JS writes the request (or WebSocket frames) and reads the reply.
    pub async fn open(&self) -> Result<Bi, JsError> {
        let (send, recv) = self.conn.open_bi().await.map_err(err)?;
        Ok(Bi {
            send: Some(send),
            recv: Some(recv),
        })
    }

    /// Resolves once the connection has closed, by either end (the Host stopping, an idle
    /// timeout, `close`); with the reason.
    pub async fn closed(&self) -> String {
        self.conn.closed().await.to_string()
    }

    /// Close the connection and its Relay link. Safe while other calls are still pending; they fail.
    pub fn close(&self) {
        self.conn.close(0u32.into(), b"closed");
        let ep = self.ep.clone();
        wasm_bindgen_futures::spawn_local(async move { ep.close().await });
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
    /// Abandon what was sent: the Host sees the stream broken off, never a short one that ended.
    pub fn reset(&mut self) -> Result<(), JsError> {
        self.s.reset(0u32.into()).map_err(err)
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
