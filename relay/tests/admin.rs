//! A Relay's admin page (#133): server-rendered HTML with plain forms over the Enrolled Keys'
//! file, on a listener of its own meant for a tunnel, logged in to with an Operator Token. Driven
//! through its HTTP interface, against a running `tooob-relay`, and checked against the file and
//! the Relay's behaviour.
use std::{net::SocketAddr, path::PathBuf, time::Duration};

use iroh_base::{EndpointId, RelayUrl, SecretKey};
use iroh_relay::{
    client::{Client, ClientBuilder},
    tls::{CaTlsConfig, default_provider},
};
use n0_future::StreamExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tooob_relay::{List, Relay, WRONG_TOKENS_ALLOWED};

struct Admin {
    relay: Relay,
    at: SocketAddr,
    list: PathBuf,
    /// The Operator Token labelled `admin`.
    token: String,
    _dir: tempfile::TempDir,
}

/// A Relay serving its admin page, with no Operator Token made.
async fn bare() -> Admin {
    let dir = tempfile::tempdir().unwrap();
    let list = dir.path().join("enrolled.toml");
    let mut relay = Relay::spawn_dev(([127, 0, 0, 1], 0), list.clone())
        .await
        .unwrap();
    let at = relay
        .serve_admin(([127, 0, 0, 1], 0), Some("relay.example.com".into()))
        .await
        .unwrap();
    Admin {
        relay,
        at,
        list,
        token: String::new(),
        _dir: dir,
    }
}

/// A Relay serving its admin page, with an Operator Token labelled `admin`.
async fn admin() -> Admin {
    let mut a = bare().await;
    a.token = List::edit(&a.list, |l| l.make_operator_token("admin")).unwrap();
    a
}

/// Log in with `token`, as the login form does: the reply.
async fn login_reply(at: SocketAddr, token: &str) -> String {
    let t = form_token(&raw_page(at, "").await);
    let origin = format!(
        "Origin: http://localhost:{}\r\nSec-Fetch-Site: same-origin\r\n",
        at.port()
    );
    post_with(
        at,
        "/login",
        &form(&[("token", t.as_str()), ("operator_token", token)]),
        &origin,
    )
    .await
}

/// Log in with the Admin's token: the session cookie, as a `Cookie:` header line.
async fn login(a: &Admin) -> String {
    let reply = login_reply(a.at, &a.token).await;
    assert!(reply.starts_with("HTTP/1.1 303"), "{reply}");
    let line = reply
        .lines()
        .find(|l| l.to_ascii_lowercase().starts_with("set-cookie:"))
        .expect("a session cookie");
    let pair = line[11..].trim().split(';').next().unwrap().to_string();
    format!("Cookie: {pair}\r\n")
}

/// GET / with `extra` headers: the whole reply.
async fn raw_page(at: SocketAddr, extra: &str) -> String {
    raw(
        at,
        &format!(
            "GET / HTTP/1.1\r\nHost: localhost:{}\r\n{extra}Connection: close\r\n\r\n",
            at.port()
        ),
    )
    .await
}

/// The whole reply to `request`, sent as is.
async fn raw(at: SocketAddr, request: &str) -> String {
    let mut stream = tokio::net::TcpStream::connect(at).await.unwrap();
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut reply = String::new();
    stream.read_to_string(&mut reply).await.unwrap();
    reply
}

/// The page, as a browser through a tunnel gets it, logged in.
async fn page(a: &Admin) -> String {
    let reply = raw_page(a.at, &login(a).await).await;
    assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
    reply
}

/// The page's per-run form token, from its forms.
fn form_token(page: &str) -> String {
    let from = page.find(r#"name="token" value=""#).expect("a token") + 20;
    page[from..from + 32].to_string()
}

fn form(fields: &[(&str, &str)]) -> String {
    url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(fields)
        .finish()
}

/// POST a form to `path` as the page itself does, with `extra` headers.
async fn post_with(at: SocketAddr, path: &str, body: &str, extra: &str) -> String {
    raw(
        at,
        &format!(
            "POST {path} HTTP/1.1\r\nHost: localhost:{port}\r\n{extra}Content-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len(),
            port = at.port()
        ),
    )
    .await
}

/// POST `fields` with the page's form token, as a browser on the page does, logged in.
async fn post(a: &Admin, path: &str, fields: &[(&str, &str)]) -> String {
    let cookie = login(a).await;
    post_as(a.at, &cookie, path, fields).await
}

/// POST `fields` with the page's form token and the session `cookie` (a header line).
async fn post_as(at: SocketAddr, cookie: &str, path: &str, fields: &[(&str, &str)]) -> String {
    let t = form_token(&raw_page(at, "").await);
    let mut all = vec![("token", t.as_str())];
    all.extend_from_slice(fields);
    let extra = format!(
        "{cookie}Origin: http://localhost:{}\r\nSec-Fetch-Site: same-origin\r\n",
        at.port()
    );
    post_with(at, path, &form(&all), &extra).await
}

fn fresh_key() -> EndpointId {
    SecretKey::from_bytes(&rand::random()).public()
}

async fn client(relay: &Relay) -> (Client, EndpointId) {
    let secret = SecretKey::from_bytes(&rand::random());
    let key = secret.public();
    (client_as(relay, secret).await, key)
}

async fn client_as(relay: &Relay, secret: SecretKey) -> Client {
    let url: RelayUrl = format!("http://{}", relay.http_addr()).parse().unwrap();
    let tls = CaTlsConfig::default()
        .client_config(default_provider())
        .unwrap();
    ClientBuilder::new(url, secret, iroh_dns::dns::DnsResolver::new())
        .tls_client_config(tls)
        .connect()
        .await
        .unwrap()
}

/// Enroll `key` as `label` in the file, as `tooob-relay enroll` does.
fn enroll(a: &Admin, key: EndpointId, label: &str) {
    List::edit(&a.list, |l| {
        l.enroll(key, label);
        Ok(())
    })
    .unwrap();
}

#[tokio::test]
async fn with_no_operator_token_nobody_can_log_in_and_the_page_says_how_to_make_one() {
    let a = bare().await;
    let reply = raw_page(a.at, "").await;
    assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
    assert!(reply.contains("tooob-relay token new"), "{reply}");
    let reply = login_reply(a.at, "anything").await;
    assert!(reply.starts_with("HTTP/1.1 401"), "{reply}");
    assert!(
        !reply.to_ascii_lowercase().contains("set-cookie"),
        "{reply}"
    );
}

#[tokio::test]
async fn without_a_session_the_page_shows_only_its_login() {
    let a = admin().await;
    let key = fresh_key();
    enroll(&a, key, "home box");
    let reply = raw_page(a.at, "").await;
    assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
    assert!(reply.contains("Operator Token"), "{reply}");
    assert!(!reply.contains(&key.to_z32()), "{reply}");
    assert!(!reply.contains("home box"), "{reply}");
}

#[tokio::test]
async fn logging_in_sets_a_session_cookie_only_the_page_can_send() {
    let a = admin().await;
    let reply = login_reply(a.at, &a.token).await;
    let line = reply
        .lines()
        .find(|l| l.to_ascii_lowercase().starts_with("set-cookie:"))
        .expect("a session cookie");
    assert!(line.contains("HttpOnly"), "{line}");
    assert!(line.contains("SameSite=Strict"), "{line}");
    assert!(line.contains("Path=/"), "{line}");
}

#[tokio::test]
async fn a_wrong_token_is_refused_and_too_many_lock_the_address_out() {
    let a = admin().await;
    for _ in 0..WRONG_TOKENS_ALLOWED {
        let reply = login_reply(a.at, "wrong").await;
        assert!(reply.starts_with("HTTP/1.1 401"), "{reply}");
    }
    let reply = login_reply(a.at, &a.token).await;
    assert!(reply.starts_with("HTTP/1.1 429"), "{reply}");
}

#[tokio::test]
async fn wrong_tokens_at_the_public_enroll_api_never_lock_the_admin_page_out() {
    // On Fly every public client may look like one address (#135).
    let mut a = admin().await;
    let status = a.relay.serve_status(([127, 0, 0, 1], 0)).await.unwrap();
    let key = fresh_key().to_z32();
    for _ in 0..WRONG_TOKENS_ALLOWED {
        let reply = raw(
            status,
            &format!(
                "DELETE /__tooob/api/enrolled/{key} HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer wrong\r\nConnection: close\r\n\r\n"
            ),
        )
        .await;
        assert!(reply.starts_with("HTTP/1.1 401"), "{reply}");
    }
    let reply = login_reply(a.at, &a.token).await;
    assert!(reply.starts_with("HTTP/1.1 303"), "{reply}");
}

#[tokio::test]
async fn a_client_too_slow_to_send_its_request_gets_408() {
    let a = admin().await;
    let mut stream = tokio::net::TcpStream::connect(a.at).await.unwrap();
    stream.write_all(b"GET / HTTP/1.1\r\n").await.unwrap();
    let mut reply = String::new();
    tokio::time::timeout(Duration::from_secs(15), stream.read_to_string(&mut reply))
        .await
        .expect("an answer within the read timeout")
        .unwrap();
    assert!(reply.starts_with("HTTP/1.1 408"), "{reply}");
}

#[tokio::test]
async fn a_form_posted_without_a_session_changes_nothing() {
    let a = admin().await;
    let reply = post_as(
        a.at,
        "Cookie: tooob_admin=0123456789abcdef0123456789abcdef\r\n",
        "/add",
        &[("key", &fresh_key().to_z32()), ("label", "x")],
    )
    .await;
    assert!(reply.starts_with("HTTP/1.1 403"), "{reply}");
    assert!(List::load(&a.list).unwrap().is_empty());
}

#[tokio::test]
async fn revoking_the_token_ends_its_sessions_at_once() {
    let a = admin().await;
    let cookie = login(&a).await;
    assert!(raw_page(a.at, &cookie).await.contains("Enrolled Keys"));
    List::edit(&a.list, |l| Ok(l.revoke_operator_token("admin"))).unwrap();
    let reply = raw_page(a.at, &cookie).await;
    assert!(!reply.contains("Add a key"), "{reply}");
    let reply = post_as(
        a.at,
        &cookie,
        "/add",
        &[("key", &fresh_key().to_z32()), ("label", "x")],
    )
    .await;
    assert!(reply.starts_with("HTTP/1.1 403"), "{reply}");
    assert!(List::load(&a.list).unwrap().is_empty());
}

#[tokio::test]
async fn logging_out_ends_the_session() {
    let a = admin().await;
    let cookie = login(&a).await;
    let reply = post_as(a.at, &cookie, "/logout", &[]).await;
    assert!(reply.starts_with("HTTP/1.1 303"), "{reply}");
    assert!(!raw_page(a.at, &cookie).await.contains("Add a key"));
}

#[tokio::test]
async fn the_page_says_a_relay_with_no_keys_is_open() {
    let a = admin().await;
    let page = page(&a).await;
    assert!(page.contains("text/html"), "{page}");
    assert!(page.contains("Open"), "{page}");
    assert!(page.contains("no Enrolled Keys"), "{page}");
}

#[tokio::test]
async fn the_page_lists_each_key_with_its_label_and_says_the_relay_is_closed() {
    let a = admin().await;
    let key = fresh_key();
    enroll(&a, key, "Ann's <home> box");
    let page = page(&a).await;
    assert!(page.contains(&key.to_z32()), "{page}");
    assert!(page.contains("Ann&#39;s &lt;home&gt; box"), "{page}");
    assert!(page.contains("Closed"), "{page}");
    assert!(page.contains("never, since this Relay started"), "{page}");
}

#[tokio::test]
async fn a_connected_key_shows_connected_now_and_then_when_it_was_last_seen() {
    let a = admin().await;
    let (host, key) = client(&a.relay).await;
    enroll(&a, key, "home box");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(page(&a).await.contains("connected now"));
    drop(host);
    tokio::time::sleep(Duration::from_millis(500)).await;
    let page = page(&a).await;
    assert!(!page.contains("connected now"), "{page}");
    assert!(page.contains("last seen just now"), "{page}");
}

#[tokio::test]
async fn a_key_with_two_connections_stays_connected_until_both_close() {
    let a = admin().await;
    let secret = SecretKey::from_bytes(&rand::random());
    let key = secret.public();
    enroll(&a, key, "home box");
    let first = client_as(&a.relay, secret.clone()).await;
    let second = client_as(&a.relay, secret).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    drop(first);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(page(&a).await.contains("connected now"));
    drop(second);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(page(&a).await.contains("last seen just now"));
}

#[tokio::test]
async fn adding_relabelling_and_removing_a_key_edit_the_file() {
    let a = admin().await;
    let key = fresh_key();
    let reply = post(&a, "/add", &[("key", &key.to_z32()), ("label", "home box")]).await;
    assert!(reply.starts_with("HTTP/1.1 303"), "{reply}");
    let entries = |l: &List| {
        l.entries()
            .map(|(k, s)| (k, s.to_string()))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        entries(&List::load(&a.list).unwrap()),
        [(key, "home box".into())]
    );
    post(&a, "/label", &[("key", &key.to_z32()), ("label", "Ann")]).await;
    assert_eq!(
        entries(&List::load(&a.list).unwrap()),
        [(key, "Ann".into())]
    );
    post(&a, "/remove", &[("key", &key.to_z32())]).await;
    assert!(List::load(&a.list).unwrap().is_empty());
}

#[tokio::test]
async fn a_key_that_isnt_one_is_refused_with_a_reason() {
    let a = admin().await;
    let reply = post(&a, "/add", &[("key", "nope"), ("label", "x")]).await;
    assert!(reply.starts_with("HTTP/1.1 400"), "{reply}");
    assert!(reply.contains("nope isn&#39;t a key"), "{reply}");
}

#[tokio::test]
async fn a_key_removed_on_the_page_is_disconnected_at_once() {
    let a = admin().await;
    let (mut host, key) = client(&a.relay).await;
    enroll(&a, key, "home box");
    enroll(&a, fresh_key(), "other");
    tokio::time::sleep(Duration::from_millis(600)).await;
    post(&a, "/remove", &[("key", &key.to_z32())]).await;
    let ended = tokio::time::timeout(Duration::from_millis(200), async {
        loop {
            match host.next().await {
                None | Some(Err(_)) => return,
                Some(Ok(_)) => continue,
            }
        }
    })
    .await;
    assert!(ended.is_ok(), "the removed key's connection is still up");
}

#[tokio::test]
async fn a_code_made_on_the_page_is_shown_once_then_listed_and_can_be_cancelled() {
    let a = admin().await;
    let reply = post(
        &a,
        "/code",
        &[("label", "Ann"), ("uses", "2"), ("expires", "3d")],
    )
    .await;
    assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
    assert!(reply.contains("no-store"), "{reply}");
    let at = reply.find("relay.example.com/").expect("the code") + 18;
    let secret = &reply[at..at + 19];
    assert_eq!(secret.len(), 19);
    assert_eq!(secret.matches('-').count(), 3, "{secret}");
    let shown = page(&a).await;
    assert!(!shown.contains(secret), "the code is shown again");
    assert!(shown.contains("Ann"), "{shown}");
    assert!(shown.contains("2 uses left"), "{shown}");
    assert!(shown.contains("expires in 3 days"), "{shown}");
    let list = List::load(&a.list).unwrap();
    let id = list
        .live_codes(tooob_relay::now())
        .next()
        .unwrap()
        .id
        .to_string();
    post(&a, "/cancel", &[("code", &id)]).await;
    assert_eq!(
        List::load(&a.list)
            .unwrap()
            .live_codes(tooob_relay::now())
            .count(),
        0
    );
    assert!(page(&a).await.contains("no unused Enrollment Codes"));
}

#[tokio::test]
async fn a_form_posted_without_the_pages_token_changes_nothing() {
    let a = admin().await;
    let cookie = login(&a).await;
    let body = form(&[("key", &fresh_key().to_z32()), ("label", "x")]);
    let reply = post_with(a.at, "/add", &body, &cookie).await;
    assert!(reply.starts_with("HTTP/1.1 403"), "{reply}");
    assert!(List::load(&a.list).unwrap().is_empty());
}

#[tokio::test]
async fn a_form_posted_from_another_site_changes_nothing_even_with_the_token() {
    let a = admin().await;
    let cookie = login(&a).await;
    let t = form_token(&page(&a).await);
    let body = form(&[
        ("token", &t),
        ("key", &fresh_key().to_z32()),
        ("label", "x"),
    ]);
    let login = form(&[("token", t.as_str()), ("operator_token", a.token.as_str())]);
    for extra in [
        "Origin: https://evil.example\r\n",
        "Sec-Fetch-Site: cross-site\r\n",
        "Origin: null\r\n",
    ] {
        let reply = post_with(a.at, "/add", &body, &format!("{cookie}{extra}")).await;
        assert!(reply.starts_with("HTTP/1.1 403"), "{extra}: {reply}");
        let reply = post_with(a.at, "/login", &login, extra).await;
        assert!(reply.starts_with("HTTP/1.1 403"), "login, {extra}: {reply}");
    }
    assert!(List::load(&a.list).unwrap().is_empty());
}

#[tokio::test]
async fn a_request_naming_any_host_but_this_machine_is_refused() {
    // Another website rebinding its own name to this machine's address still sends its name.
    let a = admin().await;
    let reply = raw(
        a.at,
        "GET / HTTP/1.1\r\nHost: evil.example:8081\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert!(reply.starts_with("HTTP/1.1 403"), "{reply}");
    assert!(!reply.contains("token"), "{reply}");
}
