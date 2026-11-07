//! The access check where a Host redeems an Enrollment Code (ADR 0030, #129): a connection that
//! carries a code (iroh's auth token: `Authorization: Bearer` or `?token=`) has the key iroh's
//! handshake just proved enrolled under the code's label, or is refused, told why. A connection
//! with no code, or from a key already enrolled (a Host reconnecting presents its spent code
//! again), is left to the forwarding rule. An address that gets several codes wrong is refused
//! redemption for a while.
use std::{
    collections::HashMap,
    net::IpAddr,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use iroh_base::EndpointId;
use iroh_relay::server::{Access, AccessControl, ClientRequest, ConnectionId, DynAccessControl};
use tracing::{info, warn};

use crate::enrolled::{List, Refusal, Rule, now};

/// How many wrong codes an address may present in a `WRONG_WINDOW` before it's refused.
pub const WRONG_CODES_ALLOWED: u32 = 5;
/// How long an address's wrong codes count against it, and so how long it's refused.
pub const WRONG_WINDOW: Duration = Duration::from_secs(15 * 60);

#[derive(Debug)]
pub(crate) struct Gate {
    path: PathBuf,
    rule: Rule,
    /// The check the config file asks for (stock iroh-relay's `access`), applied after this one.
    then: Arc<dyn DynAccessControl>,
    /// The file is edited here and by the CLI; this keeps two redemptions here from racing.
    file: Mutex<()>,
    /// Each address's wrong codes: how many, since when.
    wrong: Mutex<HashMap<Option<IpAddr>, (u32, Instant)>>,
}

impl Gate {
    pub(crate) fn new(path: PathBuf, rule: Rule, then: Arc<dyn DynAccessControl>) -> Gate {
        Gate {
            path,
            rule,
            then,
            file: Mutex::new(()),
            wrong: Mutex::default(),
        }
    }

    /// Redeem `code` for `key`, from `from`: the code's label, or why not.
    fn redeem(&self, code: &str, key: EndpointId, from: Option<IpAddr>) -> Result<String, String> {
        if self.refused(from) {
            return Err(format!(
                "too many wrong Enrollment Codes from this address: try again in {} minutes",
                WRONG_WINDOW.as_secs() / 60
            ));
        }
        let _file = self.file.lock().unwrap();
        let mut list = List::load(&self.path)?;
        match list.redeem(code, key, now()) {
            Ok(label) => {
                list.save(&self.path)?;
                self.rule.follow(&list);
                info!("tooob-relay: enrolled {key} as {label} with an Enrollment Code");
                Ok(label)
            }
            Err(refusal) => {
                if refusal == Refusal::Wrong {
                    self.count_wrong(from);
                }
                Err(refusal.to_string())
            }
        }
    }

    fn refused(&self, from: Option<IpAddr>) -> bool {
        let wrong = self.wrong.lock().unwrap();
        matches!(wrong.get(&from), Some((n, since))
            if *n >= WRONG_CODES_ALLOWED && since.elapsed() < WRONG_WINDOW)
    }

    fn count_wrong(&self, from: Option<IpAddr>) {
        let mut wrong = self.wrong.lock().unwrap();
        wrong.retain(|_, (_, since)| since.elapsed() < WRONG_WINDOW);
        let entry = wrong.entry(from).or_insert((0, Instant::now()));
        entry.0 += 1;
    }
}

/// The address a connection came from: the client's, as the Relay image's Caddy, on this machine,
/// says in `X-Forwarded-For` (it sets the header itself, ignoring a client's); otherwise the
/// connection's own.
fn client_addr(request: &ClientRequest) -> Option<IpAddr> {
    let peer = request.peer_addr().map(|a| a.ip());
    let forwarded = || {
        request
            .headers()
            .get_all("x-forwarded-for")
            .iter()
            .filter_map(|v| v.to_str().ok())
            .flat_map(|v| v.split(','))
            .next_back()
            .and_then(|a| a.trim().parse().ok())
    };
    match peer {
        Some(ip) if ip.is_loopback() => forwarded().or(peer),
        _ => peer,
    }
}

impl AccessControl for Gate {
    async fn on_connect(&self, request: &ClientRequest) -> Access {
        let key = request.endpoint_id();
        if let Some(code) = request.auth_token()
            && !self.rule.is_enrolled(key)
            && let Err(reason) = self.redeem(&code, key, client_addr(request))
        {
            warn!("tooob-relay: {key} enrolled nothing: {reason}");
            return Access::Deny {
                reason: Some(reason),
            };
        }
        self.then.on_connect(request).await
    }

    fn on_disconnect(&self, endpoint_id: EndpointId, connection_id: ConnectionId) {
        self.then.on_disconnect(endpoint_id, connection_id)
    }
}
