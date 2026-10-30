//! A Visitor's side of the tooob protocol, natively: what the browser shell does in JavaScript.
//! Used by tests and, later, the native visitor app.
use crate::Network;
use crate::access::Redeemed;
use crate::wire::{self, ALPN};
use iroh::{
    Endpoint, EndpointAddr, SecretKey,
    endpoint::{Connection, RecvStream, SendStream},
};
use serde_json::json;
use tokio::io::{AsyncRead, AsyncReadExt};

#[derive(Debug, PartialEq, Eq)]
pub enum VisitError {
    /// The Host doesn't let this Visitor in.
    Denied,
    /// The Visitor used an Invite with First-Visit Approval; the Owner hasn't approved it yet.
    AwaitingApproval,
    /// The Host refused the Invite (unknown, used, expired, blocked, already admitted).
    InviteRefused(String),
    Connection(String),
}

impl<E: std::fmt::Display> From<E> for VisitError
where
    E: Into<anyhow::Error>,
{
    fn from(e: E) -> Self {
        VisitError::Connection(e.to_string())
    }
}

/// Where this Visitor stands with the Host.
#[derive(Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Standing {
    Allowed,
    AwaitingApproval,
    Denied,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

pub struct Visit {
    _endpoint: Endpoint,
    conn: Connection,
}

impl Visit {
    /// Connect to a Host as the Visitor whose Visitor Key is `visitor_key`: like a browser tab, on a
    /// connection key of its own, then a hello proving the Visitor Key for that connection.
    pub async fn connect(
        host: EndpointAddr,
        visitor_key: [u8; 32],
        network: Network,
    ) -> Result<Visit, VisitError> {
        let endpoint = crate::endpoint(&network, SecretKey::generate()).await?;
        let host_key = host.id;
        let conn = endpoint.connect(host, ALPN).await?;
        let visitor = SecretKey::from_bytes(&visitor_key);
        let sig = visitor.sign(&wire::hello_message(&host_key, &endpoint.id()));
        let hello = json!({"t": "hello", "visitor": visitor.public().to_string(), "sig": sig});
        let (mut send, mut recv) = conn.open_bi().await?;
        wire::write_line(&mut send, &hello).await?;
        send.finish()?;
        let (head, _) = wire::read_line(&mut recv).await?;
        if head["status"] != 200 {
            return Err(VisitError::Connection(format!("hello refused: {head}")));
        }
        Ok(Visit {
            _endpoint: endpoint,
            conn,
        })
    }

    /// Wait until the connection to the Host is closed, by either end.
    pub async fn closed(&self) {
        self.conn.closed().await;
    }

    /// Use an Invite: the step that makes this device a Visitor.
    pub async fn redeem(&self, token: &str) -> Result<Redeemed, VisitError> {
        let (mut send, mut recv) = self.conn.open_bi().await?;
        wire::write_line(&mut send, &json!({"t": "redeem", "token": token})).await?;
        send.finish()?;
        let (head, _) = wire::read_line(&mut recv).await?;
        match head["redeemed"].as_str() {
            Some("admitted") => Ok(Redeemed::Admitted),
            Some("awaiting_approval") => Ok(Redeemed::AwaitingApproval),
            _ => Err(VisitError::InviteRefused(
                head["error"].as_str().unwrap_or("unknown").to_string(),
            )),
        }
    }

    /// Ask where this Visitor stands, without sending anything to the Site.
    pub async fn standing(&self) -> Result<Standing, VisitError> {
        let (mut send, mut recv) = self.conn.open_bi().await?;
        wire::write_line(&mut send, &json!({"t": "standing"})).await?;
        send.finish()?;
        let (head, _) = wire::read_line(&mut recv).await?;
        serde_json::from_value(head["standing"].clone())
            .map_err(|_| VisitError::Connection(format!("unexpected standing reply: {head}")))
    }

    pub async fn get(&self, path: &str) -> Result<Response, VisitError> {
        self.request("GET", path, &[], b"").await
    }

    pub async fn request(
        &self,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: &[u8],
    ) -> Result<Response, VisitError> {
        self.upload(method, path, headers, body).await
    }

    /// A request whose body is sent as it's read from `body`, as the browser shell sends an
    /// upload: never held whole. If reading `body` fails, the request is broken off, never sent
    /// short. A Host that stops taking the body (the Site answered before reading it all) still
    /// has its answer read.
    pub async fn upload(
        &self,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
        mut body: impl AsyncRead + Unpin,
    ) -> Result<Response, VisitError> {
        let (mut send, mut recv) = self.conn.open_bi().await?;
        let headers: Vec<_> = headers.iter().map(|(k, v)| json!([k, v])).collect();
        let head = json!({"t": "http", "method": method, "path": path, "headers": headers});
        wire::write_line(&mut send, &head).await?;
        let mut chunk = vec![0u8; 64 * 1024];
        loop {
            let n = match body.read(&mut chunk).await {
                Ok(n) => n,
                Err(e) => {
                    let _ = send.reset(0u32.into());
                    return Err(e.into());
                }
            };
            if n == 0 {
                let _ = send.finish();
                break;
            }
            if send.write_all(&chunk[..n]).await.is_err() {
                break; // the Host stopped reading: its answer says why
            }
        }
        let (head, mut body) = wire::read_line(&mut recv).await?;
        refused(&head)?;
        AsyncReadExt::read_to_end(&mut recv, &mut body).await?; // tolerates an already-finished stream
        let headers = head["headers"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|h| Some((h[0].as_str()?.to_string(), h[1].as_str()?.to_string())))
            .collect();
        Ok(Response {
            status: head["status"].as_u64().unwrap_or(0) as u16,
            headers,
            body,
        })
    }

    /// Open a WebSocket to the Site, as this Visitor.
    pub async fn websocket(&self, path: &str) -> Result<WebSocket, VisitError> {
        let (mut send, mut recv) = self.conn.open_bi().await?;
        wire::write_line(
            &mut send,
            &json!({"t": "ws", "path": path, "protocols": []}),
        )
        .await?;
        let (head, buf) = wire::read_line(&mut recv).await?;
        refused(&head)?;
        if head["status"] != 101 {
            // As the browser shell does: anything else means there's no socket.
            return Err(VisitError::Connection(format!("no WebSocket: {head}")));
        }
        Ok(WebSocket { send, recv, buf })
    }
}

/// A WebSocket to the Site, carried over a tooob stream.
pub struct WebSocket {
    send: SendStream,
    recv: RecvStream,
    buf: Vec<u8>,
}

impl WebSocket {
    pub async fn send_text(&mut self, text: &str) -> Result<(), VisitError> {
        self.send
            .write_all(&wire::frame(wire::TEXT, text.as_bytes()))
            .await?;
        Ok(())
    }

    /// The next text message, or None once the Site closes the socket.
    pub async fn next_text(&mut self) -> Result<Option<String>, VisitError> {
        loop {
            while let Some((kind, payload)) = wire::take_frame(&mut self.buf) {
                match kind {
                    wire::TEXT => return Ok(Some(String::from_utf8_lossy(&payload).into_owned())),
                    wire::CLOSE => return Ok(None),
                    _ => {}
                }
            }
            if self.recv.read_buf(&mut self.buf).await? == 0 {
                return Ok(None);
            }
        }
    }
}

/// The Host's answer when it won't let this Visitor through to the Site.
fn refused(head: &serde_json::Value) -> Result<(), VisitError> {
    match serde_json::from_value(head["standing"].clone()) {
        Ok(Standing::Denied) => Err(VisitError::Denied),
        Ok(Standing::AwaitingApproval) => Err(VisitError::AwaitingApproval),
        _ => Ok(()),
    }
}
