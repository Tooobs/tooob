//! tooob-relay (ADR 0030, #128): iroh-relay's server, forwarding a packet only if its sender or its
//! receiver is an Enrolled Key, and everything while there are none. Its Enrolled Keys are a file
//! it watches, so a change applies within a second, and a key removed is disconnected at once.
use std::{net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

use iroh_relay::server::{RelayConfig, Server, ServerConfig};
use tracing::{info, warn};

pub mod config;
mod enrolled;
mod gate;

pub use enrolled::{DEFAULT_PATH, List, Refusal, Rule, access, now};
pub use gate::{WRONG_CODES_ALLOWED, WRONG_WINDOW};

/// How often a running Relay looks at its Enrolled Keys' file for a change.
const WATCH_EVERY: Duration = Duration::from_millis(250);

/// A running `tooob-relay`. Stops when dropped, or at `shutdown`.
pub struct Relay {
    server: Server,
    _watch: Aborting,
    rule: Rule,
    status: Option<Aborting>,
}

impl Relay {
    /// A Relay as `config` says (stock iroh-relay's settings), whose Enrolled Keys are in the file
    /// at `enrolled`: none there, or no file, and it forwards everything.
    pub async fn spawn(mut config: ServerConfig, enrolled: PathBuf) -> Result<Relay, String> {
        let rule = Rule::default();
        let list = List::load(&enrolled)?;
        rule.follow(&list);
        say_access(&list, &enrolled);
        if let Some(relay) = config.relay.as_mut() {
            relay.forward = Arc::new(rule.clone());
            relay.access = Arc::new(gate::Gate::new(
                enrolled.clone(),
                rule.clone(),
                relay.access.clone(),
            ));
        }
        let server = Server::spawn(config).await.map_err(|e| e.to_string())?;
        let clients = server.relay_service().map(|s| s.clients().clone());
        let kept = rule.clone();
        let watch = tokio::spawn(async move {
            // The file is small: read it each time, rather than trust a timestamp's granularity.
            let mut seen = list;
            let mut unreadable = None;
            loop {
                tokio::time::sleep(WATCH_EVERY).await;
                let list = match List::load(&enrolled) {
                    Ok(list) => list,
                    Err(e) => {
                        if unreadable.as_ref() != Some(&e) {
                            warn!(
                                "tooob-relay: can't read the Enrolled Keys, so keeping the last ones: {e}"
                            );
                        }
                        unreadable = Some(e);
                        continue;
                    }
                };
                unreadable = None;
                if !list.same_keys(&seen) {
                    for key in rule.follow(&list) {
                        info!("tooob-relay: {key} is no longer enrolled: disconnecting it");
                        if let Some(clients) = &clients {
                            clients.disconnect(key, None);
                        }
                    }
                    say_access(&list, &enrolled);
                }
                seen = list;
            }
        });
        Ok(Relay {
            server,
            _watch: Aborting(watch),
            rule: kept,
            status: None,
        })
    }

    /// A Relay over plain HTTP at `addr`, as `iroh-relay --dev` runs, with no metrics.
    pub async fn spawn_dev(
        addr: impl Into<SocketAddr>,
        enrolled: PathBuf,
    ) -> Result<Relay, String> {
        let mut config = ServerConfig::default();
        config.relay = Some(RelayConfig::new(addr));
        Relay::spawn(config, enrolled).await
    }

    /// Where it serves plain HTTP.
    pub fn http_addr(&self) -> SocketAddr {
        self.server.http_addr().expect("a Relay serving HTTP")
    }

    /// Until it stops on its own.
    pub async fn join(&mut self) {
        let _ = self.server.join().await;
    }

    /// Answer `GET /__tooob/relay.json` at `addr` with `{"relay_access":"open"|"enrolled"}`, for
    /// the image's Caddy to route to. Returns where it listens.
    pub async fn serve_status(
        &mut self,
        addr: impl Into<SocketAddr>,
    ) -> Result<SocketAddr, String> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind(addr.into())
            .await
            .map_err(|e| e.to_string())?;
        let at = listener.local_addr().map_err(|e| e.to_string())?;
        let rule = self.rule.clone();
        let task = tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let rule = rule.clone();
                tokio::spawn(async move {
                    let mut request = [0u8; 2048];
                    let n = stream.read(&mut request).await.unwrap_or(0);
                    let asked = String::from_utf8_lossy(&request[..n]);
                    // "METHOD TARGET HTTP/1.1": any query on the path is ignored.
                    let mut line = asked.split_whitespace();
                    let method = line.next().unwrap_or("");
                    let path = line.next().unwrap_or("").split('?').next().unwrap_or("");
                    let reply = if path == "/__tooob/relay.json"
                        && (method == "GET" || method == "HEAD")
                    {
                        let body = format!(r#"{{"relay_access":"{}"}}"#, rule.access());
                        let head = format!(
                            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncache-control: no-store\r\naccess-control-allow-origin: *\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                            body.len()
                        );
                        match method {
                            "HEAD" => head,
                            _ => head + &body,
                        }
                    } else {
                        "HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
                            .to_string()
                    };
                    let _ = stream.write_all(reply.as_bytes()).await;
                    let _ = stream.shutdown().await;
                });
            }
        });
        self.status = Some(Aborting(task));
        Ok(at)
    }

    pub async fn shutdown(self) {
        drop(self._watch);
        let _ = self.server.shutdown().await;
    }
}

/// A task that stops when this is dropped.
struct Aborting(tokio::task::JoinHandle<()>);

impl Drop for Aborting {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Log whether the Relay is open or carries only Enrolled Keys' traffic.
fn say_access(list: &List, path: &std::path::Path) {
    match list.len() {
        0 => info!(
            "tooob-relay: relay_access open: no Enrolled Keys in {}, so this Relay forwards everyone's traffic",
            path.display()
        ),
        n => info!(
            "tooob-relay: relay_access enrolled: {n} Enrolled Keys in {}; this Relay forwards only traffic to or from one",
            path.display()
        ),
    }
}
