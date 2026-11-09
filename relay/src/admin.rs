//! A Relay's admin page (#133): its Enrolled Keys, who is connected, and its Enrollment Codes, as
//! server-rendered HTML with plain forms over the same file the `tooob-relay` commands edit.
//!
//! It logs in with an Operator Token (ADR 0031), checked against the file as it is now, and then
//! keeps a session: a cookie naming it, which ends when the token is revoked (checked at every
//! request), after `SESSION_LASTS`, at logging out, or when the Relay restarts. Failed logins
//! count with the enroll API's failed tokens, per address. It is meant for a tunnel (`fly proxy`,
//! `ssh -L`), never the Relay's public address. It also guards against another website open in
//! the same browser: a request naming any host but this machine (a site rebinding its own name to
//! it) is refused, and a form is taken only from the page itself (its Origin and Fetch Metadata)
//! carrying the page's form token, which is fresh each time the Relay starts.
use std::{
    collections::HashMap,
    fmt::Write as _,
    net::IpAddr,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use iroh_base::EndpointId;
use iroh_relay::server::clients::Clients;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use tracing::info;

use crate::{
    LastSeen, List, Rule,
    enrolled::{now, token_hash},
    expiry_text,
    gate::WrongCodes,
    parse_duration, parse_key,
    presence::Presence,
    uses_text,
};

/// The most of a request the page reads: its forms are small.
const MOST: usize = 64 * 1024;

/// How long a session lasts after logging in.
pub(crate) const SESSION_LASTS: Duration = Duration::from_secs(12 * 60 * 60);

/// The session cookie's name.
const COOKIE: &str = "tooob_admin";

/// A logged-in browser: the hash of the Operator Token it logged in with, so that revoking the
/// token ends it, and when.
struct Session {
    hash: String,
    since: Instant,
}

pub(crate) struct Admin {
    path: PathBuf,
    rule: Rule,
    presence: Presence,
    clients: Option<Clients>,
    domain: Option<String>,
    /// Put in every form, and required back: another website can't read the page to learn it.
    token: String,
    /// Failed logins, shared with the enroll API's failed tokens.
    wrong: Arc<Mutex<WrongCodes>>,
    sessions: Mutex<HashMap<String, Session>>,
}

struct Request {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    fn form(&self) -> Vec<(String, String)> {
        url::form_urlencoded::parse(&self.body)
            .into_owned()
            .collect()
    }
}

struct Reply {
    status: &'static str,
    location: Option<&'static str>,
    cookie: Option<String>,
    body: String,
}

impl Reply {
    fn page(status: &'static str, body: String) -> Reply {
        Reply {
            status,
            location: None,
            cookie: None,
            body,
        }
    }

    fn back() -> Reply {
        Reply {
            status: "303 See Other",
            location: Some("/"),
            cookie: None,
            body: String::new(),
        }
    }

    /// Back to the page, setting the session cookie to `value` for `max_age` seconds. Not
    /// `Secure`: the tunnel ends in plain HTTP on localhost, where a browser would never send a
    /// `Secure` cookie back. HttpOnly and SameSite=Strict keep it from scripts and other sites.
    fn back_with_cookie(value: &str, max_age: u64) -> Reply {
        Reply {
            cookie: Some(format!(
                "{COOKIE}={value}; Path=/; HttpOnly; SameSite=Strict; Max-Age={max_age}"
            )),
            ..Reply::back()
        }
    }

    fn refused(why: &str) -> Reply {
        Reply::page("403 Forbidden", format!("{why}\n"))
    }
}

impl Admin {
    pub(crate) fn new(
        path: PathBuf,
        rule: Rule,
        presence: Presence,
        clients: Option<Clients>,
        domain: Option<String>,
        wrong: Arc<Mutex<WrongCodes>>,
    ) -> Admin {
        Admin {
            path,
            rule,
            presence,
            clients,
            domain,
            token: random_hex(),
            wrong,
            sessions: Mutex::default(),
        }
    }

    pub(crate) async fn serve(self, listener: TcpListener) {
        let admin = std::sync::Arc::new(self);
        while let Ok((stream, _)) = listener.accept().await {
            let admin = admin.clone();
            tokio::spawn(async move { admin.answer(stream).await });
        }
    }

    async fn answer(&self, mut stream: TcpStream) {
        let peer = stream.peer_addr().ok().map(|a| a.ip());
        let reply = match read(&mut stream).await {
            Some(request) => self.reply(&request, peer),
            None => Reply::page("400 Bad Request", "Not a request this page takes.\n".into()),
        };
        let kind = match reply.body.starts_with("<!doctype html>") {
            true => "text/html; charset=utf-8",
            false => "text/plain; charset=utf-8",
        };
        let mut head = format!(
            "HTTP/1.1 {}\r\ncontent-type: {kind}\r\ncache-control: no-store\r\n\
             content-security-policy: default-src 'none'; style-src 'unsafe-inline'; form-action 'self'; frame-ancestors 'none'\r\n\
             x-frame-options: DENY\r\nreferrer-policy: no-referrer\r\nx-content-type-options: nosniff\r\n\
             content-length: {}\r\nconnection: close\r\n",
            reply.status,
            reply.body.len()
        );
        if let Some(location) = reply.location {
            let _ = write!(head, "location: {location}\r\n");
        }
        if let Some(cookie) = &reply.cookie {
            let _ = write!(head, "set-cookie: {cookie}\r\n");
        }
        head.push_str("\r\n");
        let _ = stream.write_all(head.as_bytes()).await;
        let _ = stream.write_all(reply.body.as_bytes()).await;
        let _ = stream.shutdown().await;
    }

    fn reply(&self, request: &Request, peer: Option<IpAddr>) -> Reply {
        let Some(host) = request.header("host").filter(|h| is_this_machine(h)) else {
            return Reply::refused("This page answers only to localhost, through a tunnel.");
        };
        let session = self.session(request);
        match (request.method.as_str(), request.path.as_str()) {
            ("GET", "/") => match session {
                Some(_) => Reply::page("200 OK", self.render(None)),
                None => Reply::page("200 OK", self.render_login(None)),
            },
            ("POST", path) => {
                if let Err(why) = self.check_form(request, host) {
                    return Reply::refused(why);
                }
                let form = request.form();
                let field = |name: &str| {
                    form.iter()
                        .find(|(n, _)| n == name)
                        .map(|(_, v)| v.trim().to_string())
                        .unwrap_or_default()
                };
                if path == "/login" {
                    return self.login(&field("operator_token"), peer);
                }
                let Some(id) = session else {
                    return Reply::refused(
                        "Not logged in (or the session ended): reload the page and log in.",
                    );
                };
                if path == "/logout" {
                    self.sessions.lock().unwrap().remove(&id);
                    return Reply::back_with_cookie("", 0);
                }
                match self.act(path, &field) {
                    Ok(None) => Reply::back(),
                    Ok(Some(notice)) => Reply::page("200 OK", self.render(Some(&notice))),
                    Err(e) => Reply::page(
                        "400 Bad Request",
                        self.render(Some(&format!(r#"<p class="error">{}</p>"#, escape(&e)))),
                    ),
                }
            }
            _ => Reply::page("404 Not Found", "Not on this page.\n".into()),
        }
    }

    /// The id of the live session `request`'s cookie names, if it names one. A session whose token
    /// is no longer in the file (revoked), or that is older than `SESSION_LASTS`, is ended here.
    fn session(&self, request: &Request) -> Option<String> {
        let id = request
            .header("cookie")?
            .split(';')
            .filter_map(|pair| pair.trim().split_once('='))
            .find(|(name, _)| *name == COOKIE)
            .map(|(_, value)| value.to_string())?;
        let mut sessions = self.sessions.lock().unwrap();
        let session = sessions.get(&id)?;
        // Read now, not as last watched, so a revoked token's sessions end at once.
        let live = session.since.elapsed() < SESSION_LASTS
            && List::load(&self.path).is_ok_and(|l| l.has_operator_token_hash(&session.hash));
        if !live {
            sessions.remove(&id);
            return None;
        }
        Some(id)
    }

    /// Log in with `presented`, an Operator Token, from `from`.
    fn login(&self, presented: &str, from: Option<IpAddr>) -> Reply {
        if self.wrong.lock().unwrap().refused(from, Instant::now()) {
            return Reply::page(
                "429 Too Many Requests",
                self.render_login(Some("Too many wrong tokens from here: wait a minute.")),
            );
        }
        // Read now, not as last watched, so a revoked token fails at once.
        let list = match List::load(&self.path) {
            Ok(list) => list,
            Err(e) => {
                return Reply::page(
                    "500 Internal Server Error",
                    self.render_login(Some(&format!("Can't read the Enrolled Keys: {e}"))),
                );
            }
        };
        let Some(label) = list.verify_operator_token(presented) else {
            self.wrong.lock().unwrap().count(from, Instant::now());
            return Reply::page(
                "401 Unauthorized",
                self.render_login(Some("That isn't one of this Relay's Operator Tokens.")),
            );
        };
        info!("tooob-relay: logged in to the admin page with the Operator Token for {label}");
        let id = random_hex();
        let mut sessions = self.sessions.lock().unwrap();
        sessions.retain(|_, s| s.since.elapsed() < SESSION_LASTS);
        sessions.insert(
            id.clone(),
            Session {
                hash: token_hash(presented),
                since: Instant::now(),
            },
        );
        Reply::back_with_cookie(&id, SESSION_LASTS.as_secs())
    }

    /// The login form, with `error` above it, or how to make an Operator Token if there is none.
    fn render_login(&self, error: Option<&str>) -> String {
        let mut out = head();
        if let Some(error) = error {
            let _ = write!(out, r#"<p class="error">{}</p>"#, escape(error));
        }
        if !List::load(&self.path).is_ok_and(|l| l.has_operator_tokens()) {
            out.push_str(
                "<p>This Relay has no Operator Token yet, so nobody can log in. Make one inside the Relay's container with <code>tooob-relay token new &lt;label&gt;</code>, such as <code>tooob-relay token new admin</code>, then log in with what it prints.</p>",
            );
        }
        let _ = write!(
            out,
            r#"<form method="post" action="/login"><input type="hidden" name="token" value="{}"><label>Operator Token <input name="operator_token" type="password" size="64" required autocomplete="off"></label> <button>Log in</button></form></body></html>"#,
            self.token
        );
        out
    }

    /// Whether a form came from this page, as its Origin, Fetch Metadata and token say.
    fn check_form(&self, request: &Request, host: &str) -> Result<(), &'static str> {
        if let Some(site) = request.header("sec-fetch-site")
            && site != "same-origin"
            && site != "none"
        {
            return Err("A form from another website: refused.");
        }
        if let Some(origin) = request.header("origin")
            && origin != format!("http://{host}")
        {
            return Err("A form from another website: refused.");
        }
        let form = request.form();
        match form.iter().find(|(n, _)| n == "token") {
            Some((_, t)) if *t == self.token => Ok(()),
            _ => Err(
                "A form without this page's token (the Relay may have restarted): reload the page.",
            ),
        }
    }

    /// Do what the form at `path` asks. `Some` is a notice to show in place of going back.
    fn act(&self, path: &str, field: &dyn Fn(&str) -> String) -> Result<Option<String>, String> {
        match path {
            "/add" | "/label" => {
                let key = parse_key(&field("key"))?;
                let label = field("label");
                if label.is_empty() {
                    return Err("Give the key a label.".into());
                }
                let relabel = path == "/label";
                let list = List::edit(&self.path, |list| {
                    if relabel && !list.entries().any(|(k, _)| k == key) {
                        return Err(format!("{} isn't enrolled", key.to_z32()));
                    }
                    list.enroll(key, &label);
                    Ok(list.clone())
                })?;
                self.rule.follow(&list);
                info!("tooob-relay: the admin page enrolled {key} as {label}");
                Ok(None)
            }
            "/remove" => {
                let key = parse_key(&field("key"))?;
                let list = List::edit(&self.path, |list| match list.remove(key) {
                    true => Ok(list.clone()),
                    false => Err(format!("{} isn't enrolled", key.to_z32())),
                })?;
                self.disconnect(self.rule.follow(&list));
                Ok(None)
            }
            "/code" => {
                let Some(domain) = &self.domain else {
                    return Err(
                        "This Relay has no domain to name in a code: set RELAY_DOMAIN.".into(),
                    );
                };
                let label = field("label");
                if label.is_empty() {
                    return Err("Give the code a label: the key it enrolls gets it.".into());
                }
                let uses: u32 = field("uses")
                    .parse()
                    .ok()
                    .filter(|n| *n > 0)
                    .ok_or("Uses is a number, 1 or more.")?;
                let until = now() + parse_duration(&field("expires"))?;
                let secret =
                    List::edit(&self.path, |list| Ok(list.make_code(&label, uses, until)))?;
                info!("tooob-relay: the admin page made an Enrollment Code for {label}");
                Ok(Some(format!(
                    r#"<div class="notice"><p>The Enrollment Code for {}: {}, {}. Copy it now: it isn't shown again.</p><p><input readonly size="40" value="{}/{secret}"></p></div>"#,
                    escape(&label),
                    uses_text(uses),
                    expiry_text(until),
                    escape(domain),
                )))
            }
            "/cancel" => {
                let id = field("code");
                List::edit(&self.path, |list| match list.cancel_id(&id, now()) {
                    true => Ok(()),
                    false => Err("No unused Enrollment Code is that one.".to_string()),
                })?;
                Ok(None)
            }
            _ => Err("Not a form this page has.".into()),
        }
    }

    fn disconnect(&self, gone: Vec<EndpointId>) {
        for key in gone {
            info!("tooob-relay: {key} is no longer enrolled: disconnecting it");
            if let Some(clients) = &self.clients {
                clients.disconnect(key, None);
            }
        }
    }

    fn render(&self, notice: Option<&str>) -> String {
        let list = List::load(&self.path);
        let t = &self.token;
        let mut out = head();
        let _ = write!(
            out,
            r#"<form method="post" action="/logout"><input type="hidden" name="token" value="{t}"><button>Log out</button></form>"#
        );
        if let Some(notice) = notice {
            out.push_str(notice);
        }
        let list = match list {
            Ok(list) => list,
            Err(e) => {
                let _ = write!(
                    out,
                    r#"<p class="error">Can't read the Enrolled Keys: {}</p></body></html>"#,
                    escape(&e)
                );
                return out;
            }
        };
        let _ = match list.len() {
            0 => write!(
                out,
                "<p><strong>Open</strong>: no Enrolled Keys, so this Relay forwards everyone's traffic.</p>"
            ),
            n => write!(
                out,
                "<p><strong>Closed</strong>: {n} Enrolled Keys; this Relay forwards only traffic to or from one.</p>"
            ),
        };
        out.push_str("<h2>Enrolled Keys</h2>");
        if !list.is_empty() {
            out.push_str("<table><tr><th>Key</th><th>Label</th><th>Seen</th><th></th></tr>");
            for (key, label) in list.entries() {
                let z = key.to_z32();
                let seen = match self.presence.of(key) {
                    LastSeen::Now => "connected now".to_string(),
                    LastSeen::At(t) => format!("last seen {}", ago(t)),
                    LastSeen::Never => "never, since this Relay started".to_string(),
                };
                let _ = write!(
                    out,
                    r#"<tr><td><code>{z}</code></td><td><form method="post" action="/label"><input type="hidden" name="token" value="{t}"><input type="hidden" name="key" value="{z}"><input name="label" value="{}" required> <button>Save</button></form></td><td>{seen}</td><td><form method="post" action="/remove"><input type="hidden" name="token" value="{t}"><input type="hidden" name="key" value="{z}"><button>Remove</button></form></td></tr>"#,
                    escape(label)
                );
            }
            out.push_str("</table>");
        }
        let _ = write!(
            out,
            r#"<fieldset><legend>Add a key</legend><form method="post" action="/add"><input type="hidden" name="token" value="{t}"><label>Key <input name="key" size="56" required placeholder="as the Host logs it"></label> <label>Label <input name="label" required></label> <button>Add</button></form></fieldset>"#
        );
        out.push_str("<h2>Enrollment Codes</h2>");
        let now = now();
        let mut none = true;
        for code in list.live_codes(now) {
            if none {
                out.push_str(
                    "<table><tr><th>Label</th><th>Uses</th><th>Expires</th><th></th></tr>",
                );
                none = false;
            }
            let _ = write!(
                out,
                r#"<tr><td>{}</td><td>{}</td><td>{}</td><td><form method="post" action="/cancel"><input type="hidden" name="token" value="{t}"><input type="hidden" name="code" value="{}"><button>Cancel</button></form></td></tr>"#,
                escape(code.label),
                uses_text(code.uses_left),
                expiry_text(code.expires),
                escape(code.id)
            );
        }
        match none {
            true => out.push_str("<p>no unused Enrollment Codes</p>"),
            false => out.push_str("</table>"),
        }
        let _ = write!(
            out,
            r#"<fieldset><legend>Make an Enrollment Code</legend><form method="post" action="/code"><input type="hidden" name="token" value="{t}"><label>Label <input name="label" required></label> <label>Uses <input name="uses" type="number" min="1" value="1" required></label> <label>Expires in <input name="expires" value="7d" size="5" required></label> <button>Make</button></form></fieldset></body></html>"#
        );
        out
    }
}

/// The page's head and title, which every page starts with.
fn head() -> String {
    String::from(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>Relay admin</title><style>
body{font:15px/1.4 system-ui,sans-serif;max-width:60rem;margin:1rem auto;padding:0 1rem;color:#222;background:#fff}
table{border-collapse:collapse;width:100%}td,th{text-align:left;padding:.3rem;border-bottom:1px solid #ddd;vertical-align:top}
code{font-size:13px;word-break:break-all}.notice{background:#ffd;padding:.5rem 1rem;border:1px solid #cc8}.error{color:#a00}
form{display:inline}fieldset{margin:1rem 0}
</style></head><body><h1>Relay admin</h1>"#,
    )
}

/// 16 random bytes in hex: a form token or a session id.
fn random_hex() -> String {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).expect("no randomness");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Whether a Host header names this machine, as a tunnel's far end does: not another website's
/// name rebound to this machine's address.
fn is_this_machine(host: &str) -> bool {
    let name = match host.strip_prefix('[') {
        Some(rest) => rest.split(']').next().unwrap_or(""),
        None => host.split(':').next().unwrap_or(""),
    };
    matches!(name, "localhost" | "127.0.0.1" | "::1")
}

/// How long ago `t` (Unix seconds) was, in words.
fn ago(t: u64) -> String {
    match now().saturating_sub(t) {
        s if s < 60 => "just now".to_string(),
        s if s < 3_600 => format!("{} minutes ago", s / 60),
        s if s < 86_400 => format!("{} hours ago", s / 3_600),
        s => format!("{} days ago", s / 86_400),
    }
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// A request's line, headers (names in lower case) and body, if it is one and small enough.
async fn read(stream: &mut TcpStream) -> Option<Request> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let end = loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
        if buf.len() > MOST {
            return None;
        }
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let head = std::str::from_utf8(&buf[..end]).ok()?;
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split_whitespace();
    let method = first.next()?.to_string();
    let path = first.next()?.split('?').next()?.to_string();
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(n, v)| (n.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    let length: usize = headers
        .iter()
        .find(|(n, _)| n == "content-length")
        .map(|(_, v)| v.parse().ok())
        .unwrap_or(Some(0))?;
    if length > MOST {
        return None;
    }
    let mut body = buf[end..].to_vec();
    while body.len() < length {
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    body.truncate(length);
    Some(Request {
        method,
        path,
        headers,
        body,
    })
}
