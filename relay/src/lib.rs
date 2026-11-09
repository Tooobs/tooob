//! tooob-relay (ADR 0030, #128): iroh-relay's server, forwarding a packet only if its sender or its
//! receiver is an Enrolled Key, and everything while there are none. Its Enrolled Keys are a file
//! it watches, so a change applies within a second, and a key removed is disconnected at once.
use std::{net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

use iroh_relay::server::{RelayConfig, Server, ServerConfig};
use tracing::{info, warn};

mod admin;
pub mod config;
mod enrolled;
mod gate;
mod presence;
mod status;

pub use enrolled::{DEFAULT_PATH, List, LiveCode, Refusal, Rule, access, now};
pub use gate::{WRONG_CODES_ALLOWED, WRONG_TOKENS_ALLOWED, WRONG_TOKENS_WINDOW, WRONG_WINDOW};
pub use presence::LastSeen;
pub use status::BODY_MAX;

/// How often a running Relay looks at its Enrolled Keys' file for a change.
const WATCH_EVERY: Duration = Duration::from_millis(250);

/// A running `tooob-relay`. Stops when dropped, or at `shutdown`.
pub struct Relay {
    server: Server,
    _watch: Aborting,
    rule: Rule,
    status: Option<Aborting>,
    admin: Option<Aborting>,
    enrolled: PathBuf,
    sign_up: Option<String>,
    presence: presence::Presence,
    clients: Option<iroh_relay::server::clients::Clients>,
    /// Failed Operator Tokens, per address: the enroll API's and the admin page's logins alike.
    wrong_tokens: Arc<std::sync::Mutex<gate::WrongCodes>>,
}

impl Relay {
    /// A Relay as `config` says (stock iroh-relay's settings), whose Enrolled Keys are in the file
    /// at `enrolled`: none there, or no file, and it forwards everything.
    pub async fn spawn(mut config: ServerConfig, enrolled: PathBuf) -> Result<Relay, String> {
        let rule = Rule::default();
        let list = List::load(&enrolled)?;
        rule.follow(&list);
        say_access(&list, &enrolled);
        let presence = presence::Presence::default();
        let path = enrolled.clone();
        if let Some(relay) = config.relay.as_mut() {
            relay.forward = Arc::new(rule.clone());
            relay.access = Arc::new(gate::Gate::new(
                enrolled.clone(),
                rule.clone(),
                relay.access.clone(),
                presence.clone(),
            ));
        }
        let server = Server::spawn(config).await.map_err(|e| e.to_string())?;
        let clients = server.relay_service().map(|s| s.clients().clone());
        let kept = rule.clone();
        let kept_clients = clients.clone();
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
            admin: None,
            enrolled: path,
            sign_up: None,
            presence,
            clients: kept_clients,
            wrong_tokens: Arc::new(std::sync::Mutex::new(gate::WrongCodes::new(
                WRONG_TOKENS_ALLOWED,
                WRONG_TOKENS_WINDOW,
            ))),
        })
    }

    /// Serve the admin page (#133) at `addr`: it logs in with an Operator Token, and is meant for
    /// a tunnel, never the Relay's public address. `domain` is this Relay's, which the Enrollment
    /// Codes it makes name. Returns where it listens.
    pub async fn serve_admin(
        &mut self,
        addr: impl Into<SocketAddr>,
        domain: Option<String>,
    ) -> Result<SocketAddr, String> {
        let listener = tokio::net::TcpListener::bind(addr.into())
            .await
            .map_err(|e| e.to_string())?;
        let at = listener.local_addr().map_err(|e| e.to_string())?;
        let page = admin::Admin::new(
            self.enrolled.clone(),
            self.rule.clone(),
            self.presence.clone(),
            self.clients.clone(),
            domain,
            self.wrong_tokens.clone(),
        );
        self.admin = Some(Aborting(tokio::spawn(page.serve(listener))));
        Ok(at)
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

    /// The Relay Operator's Sign-up Page (ADR 0031), which relay.json names as `sign_up`; set it
    /// before `serve_status`.
    pub fn set_sign_up_url(&mut self, url: Option<String>) {
        self.sign_up = url;
    }

    /// Answer, at `addr`, for the image's Caddy to route to: `GET /__tooob/relay.json` with
    /// `{"relay_access":"open"|"enrolled"}` (and `"sign_up"` if set), `GET
    /// /__tooob/enrolled/<key>` with `{"enrolled":true|false,"relay_access":…}`, and the enroll
    /// API, `PUT` and `DELETE /__tooob/api/enrolled/<key>` with an Operator Token (ADR 0031).
    /// Returns where it listens.
    pub async fn serve_status(
        &mut self,
        addr: impl Into<SocketAddr>,
    ) -> Result<SocketAddr, String> {
        let listener = tokio::net::TcpListener::bind(addr.into())
            .await
            .map_err(|e| e.to_string())?;
        let at = listener.local_addr().map_err(|e| e.to_string())?;
        let status = status::Status::new(
            self.rule.clone(),
            self.enrolled.clone(),
            self.server.relay_service().map(|s| s.clients().clone()),
            self.sign_up.clone(),
            self.wrong_tokens.clone(),
        );
        let task = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let status = status.clone();
                tokio::spawn(async move { status.answer(stream).await });
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

/// A key as z-base-32 (as Hosts and Key Addresses write it) or hex (as iroh does).
pub fn parse_key(s: &str) -> Result<iroh_base::EndpointId, String> {
    iroh_base::EndpointId::from_z32(s)
        .or_else(|_| s.parse())
        .map_err(|_| format!("{s} isn't a key"))
}

/// A task that stops when this is dropped.
struct Aborting(tokio::task::JoinHandle<()>);

impl Drop for Aborting {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// An Enrollment Code's uses left, in words.
pub fn uses_text(uses: u32) -> String {
    match uses {
        1 => "1 use left".to_string(),
        n => format!("{n} uses left"),
    }
}

/// When an Enrollment Code expiring at `expires` (Unix seconds) expires, in words.
pub fn expiry_text(expires: u64) -> String {
    let left = expires.saturating_sub(now());
    match left {
        0 => "expired".to_string(),
        s if s >= 86_400 => format!("expires in {} days", s.div_ceil(86_400)),
        s if s >= 3_600 => format!("expires in {} hours", s.div_ceil(3_600)),
        s => format!("expires in {} minutes", s.div_ceil(60)),
    }
}

/// A length of time such as 7d, 12h, 30m or 90s, in seconds.
pub fn parse_duration(s: &str) -> Result<u64, String> {
    let bad = || format!("{s} isn't a length of time such as 7d, 12h or 30m");
    let (n, unit) = s.split_at(s.len().saturating_sub(1));
    let n: u64 = n.parse().map_err(|_| bad())?;
    let per = match unit {
        "d" => 86_400,
        "h" => 3_600,
        "m" => 60,
        "s" => 1,
        _ => return Err(bad()),
    };
    n.checked_mul(per).ok_or_else(bad)
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
