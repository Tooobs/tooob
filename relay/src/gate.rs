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

/// How many wrong codes an address may present before it's refused.
pub const WRONG_CODES_ALLOWED: u32 = 5;
/// How long an address is refused after its last allowed wrong code; wrong codes older than this
/// no longer count against it.
pub const WRONG_WINDOW: Duration = Duration::from_secs(15 * 60);
/// How many failed attempts at the enroll API an address may make (ADR 0031)...
pub const WRONG_TOKENS_ALLOWED: u32 = 10;
/// ...each within this of the one before, before it gets 429s until this after the last.
pub const WRONG_TOKENS_WINDOW: Duration = Duration::from_secs(60);

/// Each address's wrong codes: an address that gets `allowed` wrong, each within `window` of the
/// one before, is refused until `window` after the last of them.
#[derive(Debug)]
pub(crate) struct WrongCodes {
    allowed: u32,
    window: Duration,
    /// How many wrong in a row, and when the last was.
    seen: HashMap<Option<IpAddr>, (u32, Instant)>,
}

impl WrongCodes {
    pub(crate) fn new(allowed: u32, window: Duration) -> WrongCodes {
        WrongCodes {
            allowed,
            window,
            seen: HashMap::new(),
        }
    }

    /// Whether `from` is refused at `now`.
    pub(crate) fn refused(&self, from: Option<IpAddr>, now: Instant) -> bool {
        matches!(self.seen.get(&from), Some((n, last))
            if *n >= self.allowed && now.duration_since(*last) < self.window)
    }

    /// Count a wrong code from `from` at `now`.
    pub(crate) fn count(&mut self, from: Option<IpAddr>, now: Instant) {
        let window = self.window;
        self.seen
            .retain(|_, (_, last)| now.duration_since(*last) < window);
        let entry = self.seen.entry(from).or_insert((0, now));
        entry.0 += 1;
        entry.1 = now;
    }
}

#[derive(Debug)]
pub(crate) struct Gate {
    path: PathBuf,
    rule: Rule,
    /// The check the config file asks for (stock iroh-relay's `access`), applied after this one.
    next: Arc<dyn DynAccessControl>,
    wrong: Mutex<WrongCodes>,
}

impl Gate {
    pub(crate) fn new(path: PathBuf, rule: Rule, next: Arc<dyn DynAccessControl>) -> Gate {
        Gate {
            path,
            rule,
            next,
            wrong: Mutex::new(WrongCodes::new(WRONG_CODES_ALLOWED, WRONG_WINDOW)),
        }
    }

    /// Redeem `code` for `key`, from `from`: the code's label, or why not.
    fn redeem(&self, code: &str, key: EndpointId, from: Option<IpAddr>) -> Result<String, String> {
        if self.wrong.lock().unwrap().refused(from, Instant::now()) {
            return Err(format!(
                "too many wrong Enrollment Codes from this address: try again in {} minutes",
                WRONG_WINDOW.as_secs() / 60
            ));
        }
        // Locked against the CLI's edits too, which run in processes of their own.
        let mut refusal = None;
        let mut list_after = None;
        List::edit(&self.path, |list| match list.redeem(code, key, now()) {
            Ok(label) => {
                list_after = Some(list.clone());
                Ok(label)
            }
            Err(r) => {
                refusal = Some(r);
                Err(r.to_string())
            }
        })
        .map(|label| {
            if let Some(list) = &list_after {
                self.rule.follow(list);
            }
            info!("tooob-relay: enrolled {key} as {label} with an Enrollment Code");
            label
        })
        .inspect_err(|_| {
            if refusal == Some(Refusal::Wrong) {
                self.wrong.lock().unwrap().count(from, Instant::now());
            }
        })
    }
}

/// The address a connection came from: the client's, as the Relay image's Caddy, on this machine,
/// says in `X-Forwarded-For` (it sets the header itself, ignoring a client's); otherwise the
/// connection's own.
fn client_addr(request: &ClientRequest) -> Option<IpAddr> {
    let forwarded = request
        .headers()
        .get_all("x-forwarded-for")
        .iter()
        .filter_map(|v| v.to_str().ok());
    client_ip(request.peer_addr().map(|a| a.ip()), forwarded)
}

/// The client's address, given the connection's (`peer`) and its `X-Forwarded-For` values: the
/// last one these name if the connection is from this machine (the image's Caddy), else `peer`.
pub(crate) fn client_ip<'a>(
    peer: Option<IpAddr>,
    forwarded: impl Iterator<Item = &'a str>,
) -> Option<IpAddr> {
    match peer {
        Some(ip) if ip.is_loopback() => forwarded
            .flat_map(|v| v.split(','))
            .last()
            .and_then(|a| a.trim().parse().ok())
            .or(peer),
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
        self.next.on_connect(request).await
    }

    fn on_disconnect(&self, endpoint_id: EndpointId, connection_id: ConnectionId) {
        self.next.on_disconnect(endpoint_id, connection_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: Duration = Duration::from_secs(60);

    #[test]
    fn an_address_is_refused_after_its_wrong_codes_and_let_try_again_a_window_after_the_last() {
        let mut wrong = WrongCodes::new(3, 10 * MIN);
        let me = Some(IpAddr::from([192, 0, 2, 1]));
        let other = Some(IpAddr::from([192, 0, 2, 2]));
        let start = Instant::now();
        for i in 0..3 {
            assert!(!wrong.refused(me, start + i * 4 * MIN));
            wrong.count(me, start + i * 4 * MIN);
        }
        // The third came 8 minutes in: refused until 18.
        assert!(wrong.refused(me, start + 17 * MIN));
        assert!(!wrong.refused(other, start + 17 * MIN));
        assert!(!wrong.refused(me, start + 18 * MIN));
    }

    #[test]
    fn wrong_codes_further_apart_than_the_window_dont_add_up() {
        let mut wrong = WrongCodes::new(2, 10 * MIN);
        let me = Some(IpAddr::from([192, 0, 2, 1]));
        let start = Instant::now();
        wrong.count(me, start);
        wrong.count(me, start + 11 * MIN);
        assert!(!wrong.refused(me, start + 11 * MIN));
    }
}
