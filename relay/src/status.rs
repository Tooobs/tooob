//! tooob-relay's status listener, which the image's Caddy routes to: `/__tooob/relay.json` and
//! `/__tooob/enrolled/<key>` (public reads, ADR 0030, #130), and the enroll API a Relay Operator's
//! Sign-up Page calls with an Operator Token (ADR 0031, #140).
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Instant,
};

use iroh_relay::server::clients::Clients;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::info;

use crate::{
    enrolled::{List, Rule},
    gate::{WrongCodes, client_ip},
    parse_key,
};

/// The most a request's line and headers may be.
const HEAD_MAX: usize = 8 * 1024;
/// The most an enroll API body may be: `{"label": "…"}`.
pub const BODY_MAX: usize = 4 * 1024;
/// How long a client has to send its whole request.
const READ_WITHIN: std::time::Duration = std::time::Duration::from_secs(10);

/// What every connection to the listener shares.
pub(crate) struct Status {
    pub(crate) rule: Rule,
    pub(crate) path: PathBuf,
    pub(crate) clients: Option<Clients>,
    pub(crate) sign_up: Option<String>,
    /// Failed tokens, per address; the admin page counts its own (#133).
    pub(crate) wrong: Arc<Mutex<WrongCodes>>,
}

impl Status {
    pub(crate) fn new(
        rule: Rule,
        path: PathBuf,
        clients: Option<Clients>,
        sign_up: Option<String>,
        wrong: Arc<Mutex<WrongCodes>>,
    ) -> Arc<Status> {
        Arc::new(Status {
            rule,
            path,
            clients,
            sign_up,
            wrong,
        })
    }

    /// Answer the one request on `stream`, then close it.
    pub(crate) async fn answer(&self, mut stream: tokio::net::TcpStream) {
        let peer = stream.peer_addr().ok().map(|a| a.ip());
        // A client too slow to send its request doesn't hold a task for ever.
        let reply = match tokio::time::timeout(READ_WITHIN, read_request(&mut stream)).await {
            Ok(Ok(request)) => self.reply(&request, peer),
            Ok(Err(status)) => Reply::empty(status),
            Err(_) => Reply::empty(408),
        };
        let _ = stream.write_all(&reply.bytes()).await;
        let _ = stream.shutdown().await;
    }

    fn reply(&self, request: &Request, peer: Option<std::net::IpAddr>) -> Reply {
        let method = request.method.as_str();
        // Any query on the path is ignored.
        let path = request.target.split('?').next().unwrap_or("");
        if let Some(key) = path.strip_prefix("/__tooob/api/enrolled/") {
            return match method {
                "PUT" | "DELETE" => {
                    let from = client_ip(peer, request.headers("x-forwarded-for"));
                    self.api(method, key, request, from)
                }
                _ => Reply::empty(405),
            };
        }
        if method != "GET" && method != "HEAD" {
            return Reply::empty(404);
        }
        let body = if path == "/__tooob/relay.json" {
            let sign_up = self
                .sign_up
                .as_deref()
                .map(|url| format!(r#","sign_up":{}"#, json_string(url)))
                .unwrap_or_default();
            format!(r#"{{"relay_access":"{}"{sign_up}}}"#, self.rule.access())
        } else if let Some(Ok(key)) = path.strip_prefix("/__tooob/enrolled/").map(parse_key) {
            // A Host asks whether its key is enrolled (#130): nothing a Key Address doesn't
            // already show, and relay.json says whether the Relay is open.
            self.enrolled_json(key)
        } else {
            return Reply::empty(404);
        };
        Reply::json(200, body, method == "HEAD")
    }

    fn enrolled_json(&self, key: iroh_base::EndpointId) -> String {
        format!(
            r#"{{"enrolled":{},"relay_access":"{}"}}"#,
            self.rule.is_enrolled(key),
            self.rule.access()
        )
    }

    /// `PUT` (enroll or relabel) or `DELETE` (remove) `key`, for a valid Operator Token only.
    fn api(
        &self,
        method: &str,
        key: &str,
        request: &Request,
        from: Option<std::net::IpAddr>,
    ) -> Reply {
        if self.wrong.lock().unwrap().refused(from, Instant::now()) {
            return Reply::empty(429);
        }
        let presented = request
            .headers("authorization")
            .next()
            .and_then(|v| v.trim().split_once(' '))
            .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("Bearer"))
            .map(|(_, token)| token)
            .unwrap_or("");
        // Read now, not as last watched, so a revoked token fails at once.
        let service = match List::load(&self.path) {
            Ok(list) => list.verify_operator_token(presented).map(str::to_string),
            Err(_) => return Reply::empty(500),
        };
        let Some(service) = service else {
            self.wrong.lock().unwrap().count(from, Instant::now());
            return Reply::empty(401);
        };
        let Ok(key) = parse_key(key) else {
            return Reply::empty(400);
        };
        let label = match method {
            "PUT" => match serde_json::from_slice::<Label>(&request.body) {
                Ok(Label { label }) if !label.trim().is_empty() => Some(label),
                _ => return Reply::empty(400),
            },
            _ => None,
        };
        let edited = List::edit(&self.path, |list| {
            let changed = match &label {
                Some(label) => {
                    list.enroll(key, label);
                    true
                }
                None => list.remove(key),
            };
            // Applied now, not at the watcher's next look, and under the lock, so no older edit's
            // keys are followed after a newer one's (as Gate::redeem does).
            Ok((changed, self.rule.follow(list)))
        });
        let Ok((changed, gone)) = edited else {
            return Reply::empty(500);
        };
        // A removed key is disconnected at once.
        for gone in gone {
            if let Some(clients) = &self.clients {
                clients.disconnect(gone, None);
            }
        }
        match &label {
            Some(label) => info!("tooob-relay: {service} enrolled {key} as {label}"),
            None if changed => info!("tooob-relay: {service} removed {key}: disconnecting it"),
            None => return Reply::empty(404),
        }
        Reply::json(200, self.enrolled_json(key), false)
    }
}

#[derive(serde::Deserialize)]
struct Label {
    label: String,
}

/// `text` as a JSON string.
fn json_string(text: &str) -> String {
    serde_json::to_string(text).expect("a string is JSON")
}

struct Request {
    method: String,
    target: String,
    head: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Request {
    /// The values of the header `name` (lower case).
    fn headers<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a str> {
        self.head
            .iter()
            .filter(move |(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }
}

/// One HTTP/1.1 request, its body as long as its `Content-Length`; or the status to refuse it with.
async fn read_request(stream: &mut tokio::net::TcpStream) -> Result<Request, u16> {
    let mut got = Vec::new();
    let mut chunk = [0u8; 2048];
    let end = loop {
        if let Some(at) = got.windows(4).position(|w| w == b"\r\n\r\n") {
            break at;
        }
        if got.len() > HEAD_MAX {
            return Err(431);
        }
        match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => return Err(400),
            Ok(n) => got.extend_from_slice(&chunk[..n]),
        }
    };
    let text = String::from_utf8_lossy(&got[..end]).into_owned();
    let mut lines = text.split("\r\n");
    let mut first = lines.next().unwrap_or("").split_whitespace();
    let method = first.next().unwrap_or("").to_string();
    let target = first.next().unwrap_or("").to_string();
    let head: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(n, v)| (n.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    let length = match head.iter().find(|(n, _)| n == "content-length") {
        None => 0,
        Some((_, v)) => v.parse::<usize>().map_err(|_| 400u16)?,
    };
    if length > BODY_MAX {
        return Err(413);
    }
    let mut body = got[end + 4..].to_vec();
    while body.len() < length {
        match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => return Err(400),
            Ok(n) => body.extend_from_slice(&chunk[..n]),
        }
    }
    body.truncate(length);
    Ok(Request {
        method,
        target,
        head,
        body,
    })
}

struct Reply {
    status: u16,
    body: String,
    head_only: bool,
}

impl Reply {
    fn empty(status: u16) -> Reply {
        Reply {
            status,
            body: String::new(),
            head_only: false,
        }
    }

    fn json(status: u16, body: String, head_only: bool) -> Reply {
        Reply {
            status,
            body,
            head_only,
        }
    }

    fn bytes(&self) -> Vec<u8> {
        let reason = match self.status {
            200 => "OK",
            400 => "Bad Request",
            401 => "Unauthorized",
            404 => "Not Found",
            405 => "Method Not Allowed",
            408 => "Request Timeout",
            413 => "Content Too Large",
            429 => "Too Many Requests",
            431 => "Request Header Fields Too Large",
            _ => "Internal Server Error",
        };
        let mut head = format!("HTTP/1.1 {} {reason}\r\n", self.status);
        if self.body.is_empty() {
            head += "content-length: 0\r\n";
        } else {
            head += &format!(
                "content-type: application/json\r\ncache-control: no-store\r\naccess-control-allow-origin: *\r\ncontent-length: {}\r\n",
                self.body.len()
            );
        }
        if self.status == 401 {
            head += "www-authenticate: Bearer\r\n";
        }
        head += "connection: close\r\n\r\n";
        if !self.head_only {
            head += &self.body;
        }
        head.into_bytes()
    }
}
