//! Admission over the wire: a real Host in front of a stand-in Site, Visitors connecting over iroh
//! on this machine (no Relay).
mod support;

use support::TestSite;
use tooob::Network;
use tooob::access::{InviteOptions, Redeemed};
use tooob::host::{Host, HostConfig, Site};
use tooob::visit::{Standing, Visit, VisitError};

async fn start_host(site: &TestSite, dir: &tempfile::TempDir) -> Host {
    Host::start(HostConfig {
        site: Site::WebApp(site.addr),
        data_dir: dir.path().into(),
        network: Network::LocalOnly,
    })
    .await
    .unwrap()
}

fn new_visitor_key() -> [u8; 32] {
    let mut key = [0u8; 32];
    getrandom::fill(&mut key).unwrap();
    key
}

#[tokio::test]
async fn an_unknown_visitor_is_refused_and_the_site_never_sees_it() {
    let site = TestSite::start().await;
    let dir = tempfile::tempdir().unwrap();
    let host = start_host(&site, &dir).await;

    let visit = Visit::connect(host.address(), new_visitor_key(), Network::LocalOnly)
        .await
        .unwrap();
    assert_eq!(visit.get("/").await.unwrap_err(), VisitError::Denied);
    assert_eq!(site.requests(), 0);
}

#[tokio::test]
async fn redeeming_an_invite_over_the_wire_lets_the_visitor_reach_the_site() {
    let site = TestSite::start().await;
    let dir = tempfile::tempdir().unwrap();
    let host = start_host(&site, &dir).await;
    let invite = host
        .control()
        .invite("mom", InviteOptions::default())
        .unwrap();

    let visit = Visit::connect(host.address(), new_visitor_key(), Network::LocalOnly)
        .await
        .unwrap();
    assert_eq!(visit.redeem(&invite.token).await, Ok(Redeemed::Admitted));
    let page = visit.get("/hello").await.unwrap();
    assert_eq!(page.status, 200);
    assert_eq!(
        String::from_utf8(page.body).unwrap(),
        "GET /hello visitor=mom"
    );
}

#[tokio::test]
async fn a_visitor_awaiting_approval_is_told_so_and_the_site_never_sees_it() {
    let site = TestSite::start().await;
    let dir = tempfile::tempdir().unwrap();
    let host = start_host(&site, &dir).await;
    let options = InviteOptions {
        first_visit_approval: true,
        ..Default::default()
    };
    let invite = host.control().invite("mom", options).unwrap();

    let visit = Visit::connect(host.address(), new_visitor_key(), Network::LocalOnly)
        .await
        .unwrap();
    assert_eq!(
        visit.redeem(&invite.token).await,
        Ok(Redeemed::AwaitingApproval)
    );
    assert_eq!(
        visit.get("/").await.unwrap_err(),
        VisitError::AwaitingApproval
    );
    assert_eq!(site.requests(), 0);
}

#[tokio::test]
async fn a_block_takes_effect_on_the_next_request_even_on_an_open_connection() {
    let site = TestSite::start().await;
    let dir = tempfile::tempdir().unwrap();
    let host = start_host(&site, &dir).await;
    let invite = host
        .control()
        .invite("dave", InviteOptions::default())
        .unwrap();
    let visit = Visit::connect(host.address(), new_visitor_key(), Network::LocalOnly)
        .await
        .unwrap();
    visit.redeem(&invite.token).await.unwrap();
    assert_eq!(visit.get("/").await.unwrap().status, 200);

    let dave = host
        .control()
        .visitors()
        .into_iter()
        .find(|v| v.label == "dave")
        .unwrap();
    host.control().block(&dave.key).unwrap();
    assert_eq!(visit.get("/").await.unwrap_err(), VisitError::Denied);
    assert_eq!(
        site.requests(),
        1,
        "only the request before the Block reached the Site"
    );
}

#[tokio::test]
async fn each_visitor_has_their_own_site_cookies_and_never_sees_them() {
    let site = TestSite::start().await;
    let dir = tempfile::tempdir().unwrap();
    let host = start_host(&site, &dir).await;
    let alice = Visit::connect(host.address(), new_visitor_key(), Network::LocalOnly)
        .await
        .unwrap();
    let bob = Visit::connect(host.address(), new_visitor_key(), Network::LocalOnly)
        .await
        .unwrap();
    alice
        .redeem(
            &host
                .control()
                .invite("alice", InviteOptions::default())
                .unwrap()
                .token,
        )
        .await
        .unwrap();
    bob.redeem(
        &host
            .control()
            .invite("bob", InviteOptions::default())
            .unwrap()
            .token,
    )
    .await
    .unwrap();

    let login = alice.get("/login").await.unwrap();
    assert!(
        !login.headers.iter().any(|(k, _)| k == "set-cookie"),
        "the session cookie stays on the Host"
    );
    assert_eq!(alice.get("/me").await.unwrap().body, b"alice");
    assert_eq!(bob.get("/me").await.unwrap().body, b"nobody");
}

#[tokio::test]
async fn a_visitor_stays_logged_in_to_the_site_across_a_host_restart() {
    let site = TestSite::start().await;
    let dir = tempfile::tempdir().unwrap();
    let host = start_host(&site, &dir).await;
    let alice_key = new_visitor_key();
    let alice = Visit::connect(host.address(), alice_key, Network::LocalOnly)
        .await
        .unwrap();
    alice
        .redeem(
            &host
                .control()
                .invite("alice", InviteOptions::default())
                .unwrap()
                .token,
        )
        .await
        .unwrap();
    alice.get("/login").await.unwrap();
    host.shutdown().await;

    let host = start_host(&site, &dir).await;
    let alice = Visit::connect(host.address(), alice_key, Network::LocalOnly)
        .await
        .unwrap();
    assert_eq!(alice.get("/me").await.unwrap().body, b"alice");
}

#[tokio::test]
async fn a_blocked_visitors_site_cookies_go_with_the_block() {
    let site = TestSite::start().await;
    let dir = tempfile::tempdir().unwrap();
    let host = start_host(&site, &dir).await;
    let dave_key = new_visitor_key();
    let dave = Visit::connect(host.address(), dave_key, Network::LocalOnly)
        .await
        .unwrap();
    dave.redeem(
        &host
            .control()
            .invite("dave", InviteOptions::default())
            .unwrap()
            .token,
    )
    .await
    .unwrap();
    dave.get("/login").await.unwrap();

    let key = host.control().visitors()[0].key.clone();
    host.control().block(&key).unwrap();
    host.control().unblock(&key).unwrap();
    assert_eq!(dave.get("/me").await.unwrap().body, b"nobody");

    host.shutdown().await;
    let host = start_host(&site, &dir).await;
    let dave = Visit::connect(host.address(), dave_key, Network::LocalOnly)
        .await
        .unwrap();
    assert_eq!(
        dave.get("/me").await.unwrap().body,
        b"nobody",
        "nor are they left on disk"
    );
}

#[tokio::test]
async fn a_block_during_a_slow_upload_keeps_the_visitor_logged_out() {
    let site = TestSite::start().await;
    let dir = tempfile::tempdir().unwrap();
    let host = start_host(&site, &dir).await;
    let dave_key = new_visitor_key();
    let invite = host
        .control()
        .invite("dave", InviteOptions::default())
        .unwrap();
    let conn = hello_connection(&host, dave_key).await;
    let redeem = serde_json::json!({"t": "redeem", "token": invite.token});
    assert_eq!(ask(&conn, redeem).await["status"], 200);

    // Admitted before the Block, but the body is still on its way when the Owner blocks dave.
    let (mut send, mut recv) = conn.open_bi().await.unwrap();
    let head = serde_json::json!({"t": "http", "method": "POST", "path": "/login", "headers": []});
    send.write_all(format!("{head}\n").as_bytes())
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let key = host.control().visitors()[0].key.clone();
    host.control().block(&key).unwrap();
    send.write_all(b"password").await.unwrap();
    send.finish().unwrap();
    let mut reply = Vec::new();
    let _ = tokio::io::AsyncReadExt::read_to_end(&mut recv, &mut reply).await;

    host.control().unblock(&key).unwrap();
    let dave = Visit::connect(host.address(), dave_key, Network::LocalOnly)
        .await
        .unwrap();
    assert_eq!(dave.get("/me").await.unwrap().body, b"nobody");
}

#[tokio::test]
async fn stored_site_cookies_are_readable_only_by_the_hosts_account() {
    use std::os::unix::fs::PermissionsExt;
    let site = TestSite::start().await;
    let dir = tempfile::tempdir().unwrap();
    let host = start_host(&site, &dir).await;
    let alice = Visit::connect(host.address(), new_visitor_key(), Network::LocalOnly)
        .await
        .unwrap();
    alice
        .redeem(
            &host
                .control()
                .invite("alice", InviteOptions::default())
                .unwrap()
                .token,
        )
        .await
        .unwrap();
    alice.get("/login").await.unwrap();

    // Everything the Host keeps, even inside a data directory others can open (a NAS mount).
    let mut stored = vec![dir.path().to_path_buf()];
    let mut checked = 0;
    while let Some(d) = stored.pop() {
        for entry in std::fs::read_dir(d).unwrap() {
            let path = entry.unwrap().path();
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(
                mode & 0o077,
                0,
                "{} is mode {:o}",
                path.display(),
                mode & 0o777
            );
            checked += 1;
            if path.is_dir() {
                stored.push(path);
            }
        }
    }
    assert!(checked >= 3, "the access state, the Host Key and the jars");
}

#[tokio::test]
async fn websockets_reach_the_site_as_the_visitor() {
    let site = TestSite::start().await;
    let dir = tempfile::tempdir().unwrap();
    let host = start_host(&site, &dir).await;
    let visit = Visit::connect(host.address(), new_visitor_key(), Network::LocalOnly)
        .await
        .unwrap();
    visit
        .redeem(
            &host
                .control()
                .invite("mom", InviteOptions::default())
                .unwrap()
                .token,
        )
        .await
        .unwrap();

    let mut ws = visit.websocket("/ws").await.unwrap();
    ws.send_text("hello").await.unwrap();
    assert_eq!(
        ws.next_text().await.unwrap(),
        Some("hello from mom".to_string())
    );
}

#[tokio::test]
async fn the_label_reaches_the_site_percent_encoded() {
    let site = TestSite::start().await;
    let dir = tempfile::tempdir().unwrap();
    let host = start_host(&site, &dir).await;
    let visit = Visit::connect(host.address(), new_visitor_key(), Network::LocalOnly)
        .await
        .unwrap();
    visit
        .redeem(
            &host
                .control()
                .invite("Mamá", InviteOptions::default())
                .unwrap()
                .token,
        )
        .await
        .unwrap();
    assert_eq!(
        String::from_utf8(visit.get("/").await.unwrap().body).unwrap(),
        "GET / visitor=Mam%C3%A1"
    );
}

#[tokio::test]
async fn a_visitor_cannot_make_the_host_request_any_other_server() {
    let site = TestSite::start().await;
    let elsewhere = TestSite::start().await; // stands in for a LAN router or an internet server
    let dir = tempfile::tempdir().unwrap();
    let host = start_host(&site, &dir).await;
    let visit = Visit::connect(host.address(), new_visitor_key(), Network::LocalOnly)
        .await
        .unwrap();
    visit
        .redeem(
            &host
                .control()
                .invite("mom", InviteOptions::default())
                .unwrap()
                .token,
        )
        .await
        .unwrap();

    for path in [
        format!("@{}/admin", elsewhere.addr),
        format!("http://{}/admin", elsewhere.addr),
        "admin".into(),
    ] {
        assert_eq!(visit.get(&path).await.unwrap().status, 400, "path {path:?}");
    }
    assert_eq!(elsewhere.requests(), 0);
    assert_eq!(site.requests(), 0);
}

#[tokio::test]
async fn a_visitor_cannot_pose_as_another_label_or_bring_their_own_cookies() {
    let site = TestSite::start().await;
    let dir = tempfile::tempdir().unwrap();
    let host = start_host(&site, &dir).await;
    let visit = Visit::connect(host.address(), new_visitor_key(), Network::LocalOnly)
        .await
        .unwrap();
    visit
        .redeem(
            &host
                .control()
                .invite("mom", InviteOptions::default())
                .unwrap()
                .token,
        )
        .await
        .unwrap();

    let forged = [("X-Tooob-Label", "admin"), ("Cookie", "sid=s3cret")];
    let page = visit.request("GET", "/", &forged, b"").await.unwrap();
    assert_eq!(String::from_utf8(page.body).unwrap(), "GET / visitor=mom");
    let me = visit.request("GET", "/me", &forged, b"").await.unwrap();
    assert_eq!(
        me.body, b"nobody",
        "a stolen session cookie doesn't log anyone in"
    );
}

#[tokio::test]
async fn a_block_closes_the_visitors_open_websocket() {
    let site = TestSite::start().await;
    let dir = tempfile::tempdir().unwrap();
    let host = start_host(&site, &dir).await;
    let visit = Visit::connect(host.address(), new_visitor_key(), Network::LocalOnly)
        .await
        .unwrap();
    visit
        .redeem(
            &host
                .control()
                .invite("dave", InviteOptions::default())
                .unwrap()
                .token,
        )
        .await
        .unwrap();
    let mut ws = visit.websocket("/ws").await.unwrap();
    ws.send_text("hi").await.unwrap();
    assert_eq!(ws.next_text().await.unwrap(), Some("hi from dave".into()));

    let dave = host
        .control()
        .visitors()
        .into_iter()
        .find(|v| v.label == "dave")
        .unwrap();
    host.control().block(&dave.key).unwrap();
    let closed = tokio::time::timeout(std::time::Duration::from_secs(2), ws.next_text()).await;
    assert_eq!(closed.expect("socket should close promptly").unwrap(), None);
}

#[tokio::test]
async fn a_site_that_is_down_gives_the_visitor_a_502() {
    let closed = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap(); // nothing listens here
    let dir = tempfile::tempdir().unwrap();
    let config = HostConfig {
        site: Site::WebApp(closed),
        data_dir: dir.path().into(),
        network: Network::LocalOnly,
    };
    let host = Host::start(config).await.unwrap();
    let visit = Visit::connect(host.address(), new_visitor_key(), Network::LocalOnly)
        .await
        .unwrap();
    visit
        .redeem(
            &host
                .control()
                .invite("mom", InviteOptions::default())
                .unwrap()
                .token,
        )
        .await
        .unwrap();
    assert_eq!(visit.get("/").await.unwrap().status, 502);
}

#[tokio::test]
async fn a_refused_invite_says_why_with_a_stable_code() {
    let site = TestSite::start().await;
    let dir = tempfile::tempdir().unwrap();
    let host = start_host(&site, &dir).await;
    let invite = host
        .control()
        .invite("mom", InviteOptions::default())
        .unwrap();
    let first = Visit::connect(host.address(), new_visitor_key(), Network::LocalOnly)
        .await
        .unwrap();
    let second = Visit::connect(host.address(), new_visitor_key(), Network::LocalOnly)
        .await
        .unwrap();
    first.redeem(&invite.token).await.unwrap();

    let refused = |code: &str| Err(VisitError::InviteRefused(code.into()));
    assert_eq!(second.redeem(&invite.token).await, refused("already_used"));
    assert_eq!(
        second.redeem("not-a-real-token").await,
        refused("unknown_invite")
    );
}

#[tokio::test]
async fn approving_a_waiting_visitor_lets_them_in_on_the_same_connection() {
    let site = TestSite::start().await;
    let dir = tempfile::tempdir().unwrap();
    let host = start_host(&site, &dir).await;
    let options = InviteOptions {
        first_visit_approval: true,
        ..Default::default()
    };
    let visit = Visit::connect(host.address(), new_visitor_key(), Network::LocalOnly)
        .await
        .unwrap();
    visit
        .redeem(&host.control().invite("mom", options).unwrap().token)
        .await
        .unwrap();
    assert_eq!(
        visit.get("/").await.unwrap_err(),
        VisitError::AwaitingApproval
    );

    let mom = host
        .control()
        .visitors()
        .into_iter()
        .find(|v| v.label == "mom")
        .unwrap();
    host.control().approve(&mom.key).unwrap();
    assert_eq!(visit.get("/").await.unwrap().status, 200);
}

#[tokio::test]
async fn unblocking_lets_a_visitor_back_in() {
    let site = TestSite::start().await;
    let dir = tempfile::tempdir().unwrap();
    let host = start_host(&site, &dir).await;
    let visit = Visit::connect(host.address(), new_visitor_key(), Network::LocalOnly)
        .await
        .unwrap();
    visit
        .redeem(
            &host
                .control()
                .invite("dave", InviteOptions::default())
                .unwrap()
                .token,
        )
        .await
        .unwrap();
    let dave = host
        .control()
        .visitors()
        .into_iter()
        .find(|v| v.label == "dave")
        .unwrap();
    host.control().block(&dave.key).unwrap();
    host.control().unblock(&dave.key).unwrap();
    assert_eq!(visit.get("/").await.unwrap().status, 200);
}

#[tokio::test]
async fn after_shutdown_nobody_can_connect() {
    let site = TestSite::start().await;
    let dir = tempfile::tempdir().unwrap();
    let host = start_host(&site, &dir).await;
    let address = host.address();
    host.shutdown().await;
    let attempt = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        Visit::connect(address, new_visitor_key(), Network::LocalOnly),
    )
    .await;
    assert!(
        !matches!(attempt, Ok(Ok(_))),
        "connected to a Host that was shut down"
    );
}

#[tokio::test]
async fn a_stranger_cannot_make_the_host_buffer_a_huge_header() {
    let site = TestSite::start().await;
    let dir = tempfile::tempdir().unwrap();
    let host = start_host(&site, &dir).await;
    let stranger = Visit::connect(host.address(), new_visitor_key(), Network::LocalOnly)
        .await
        .unwrap();
    let huge = "x".repeat(100 * 1024); // well past any real header block
    let attempt = stranger
        .request("GET", "/", &[("x-padding", huge.as_str())], b"")
        .await;
    assert!(
        matches!(attempt, Err(VisitError::Connection(_))),
        "refused before admission: {attempt:?}"
    );
}

#[tokio::test]
async fn a_visitor_can_ask_where_it_stands_without_touching_the_site() {
    let site = TestSite::start().await;
    let dir = tempfile::tempdir().unwrap();
    let host = start_host(&site, &dir).await;
    let visit = Visit::connect(host.address(), new_visitor_key(), Network::LocalOnly)
        .await
        .unwrap();
    assert_eq!(visit.standing().await.unwrap(), Standing::Denied);

    let options = InviteOptions {
        first_visit_approval: true,
        ..Default::default()
    };
    visit
        .redeem(&host.control().invite("mom", options).unwrap().token)
        .await
        .unwrap();
    assert_eq!(visit.standing().await.unwrap(), Standing::AwaitingApproval);
    let mom = host
        .control()
        .visitors()
        .into_iter()
        .find(|v| v.label == "mom")
        .unwrap();
    host.control().approve(&mom.key).unwrap();
    assert_eq!(visit.standing().await.unwrap(), Standing::Allowed);
    assert_eq!(
        site.requests(),
        0,
        "asking where you stand never reaches the Site"
    );
}

/// A connection speaking the protocol by hand, for what `Visit` never does (forged hellos).
async fn raw_connection(host: &Host, key: iroh::SecretKey) -> iroh::endpoint::Connection {
    let endpoint = iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
        .relay_mode(iroh::RelayMode::Disabled)
        .secret_key(key)
        .bind()
        .await
        .unwrap();
    let conn = endpoint.connect(host.address(), b"tooob/0").await.unwrap();
    std::mem::forget(endpoint); // keep it alive for the test
    conn
}

async fn ask(conn: &iroh::endpoint::Connection, message: serde_json::Value) -> serde_json::Value {
    use tokio::io::AsyncReadExt;
    let (mut send, mut recv) = conn.open_bi().await.unwrap();
    send.write_all(format!("{message}\n").as_bytes())
        .await
        .unwrap();
    send.finish().unwrap();
    let mut reply = Vec::new();
    let _ = AsyncReadExt::read_to_end(&mut recv, &mut reply).await;
    let line = reply.split(|&b| b == b'\n').next().unwrap_or_default();
    serde_json::from_slice(line).unwrap_or(serde_json::Value::Null)
}

/// What a hello signs: the protocol, the Host, and the connection it vouches for.
fn hello_message(host_key: &iroh::EndpointId, connection: &iroh::EndpointId) -> Vec<u8> {
    [
        b"tooob/0 hello".as_slice(),
        host_key.as_bytes(),
        connection.as_bytes(),
    ]
    .concat()
}

#[tokio::test]
async fn a_hello_copied_from_another_connection_is_refused() {
    let site = TestSite::start().await;
    let dir = tempfile::tempdir().unwrap();
    let host = start_host(&site, &dir).await;
    let mom = new_visitor_key();
    let visit = Visit::connect(host.address(), mom, Network::LocalOnly)
        .await
        .unwrap();
    let invite = host
        .control()
        .invite("mom", InviteOptions::default())
        .unwrap();
    visit.redeem(&invite.token).await.unwrap();

    // Mom's hello for some other connection, replayed on the attacker's own.
    let mom_key = iroh::SecretKey::from_bytes(&mom);
    let elsewhere = iroh::SecretKey::from_bytes(&new_visitor_key()).public();
    let sig = mom_key.sign(&hello_message(&host.address().id, &elsewhere));
    let attacker = raw_connection(&host, iroh::SecretKey::from_bytes(&new_visitor_key())).await;
    let hello =
        serde_json::json!({"t": "hello", "visitor": mom_key.public().to_string(), "sig": sig});
    assert_eq!(ask(&attacker, hello).await["status"], 403);
    assert_ne!(
        ask(&attacker, serde_json::json!({"t": "standing"})).await["standing"],
        "allowed"
    );
    assert_eq!(site.requests(), 0);
}

#[tokio::test]
async fn a_connection_is_nobody_until_it_says_hello() {
    let site = TestSite::start().await;
    let dir = tempfile::tempdir().unwrap();
    let host = start_host(&site, &dir).await;
    let mom = new_visitor_key();
    let visit = Visit::connect(host.address(), mom, Network::LocalOnly)
        .await
        .unwrap();
    let invite = host
        .control()
        .invite("mom", InviteOptions::default())
        .unwrap();
    visit.redeem(&invite.token).await.unwrap();

    // Even connecting with Mom's key as the connection's own key proves nothing without a hello.
    let conn = raw_connection(&host, iroh::SecretKey::from_bytes(&mom)).await;
    assert_ne!(
        ask(&conn, serde_json::json!({"t": "standing"})).await["standing"],
        "allowed"
    );
}

/// A raw connection that has said a valid hello as the Visitor whose key is `visitor`.
async fn hello_connection(host: &Host, visitor: [u8; 32]) -> iroh::endpoint::Connection {
    let conn_key = iroh::SecretKey::from_bytes(&new_visitor_key());
    let conn = raw_connection(host, conn_key.clone()).await;
    let visitor = iroh::SecretKey::from_bytes(&visitor);
    let sig = visitor.sign(&hello_message(&host.address().id, &conn_key.public()));
    let hello =
        serde_json::json!({"t": "hello", "visitor": visitor.public().to_string(), "sig": sig});
    assert_eq!(ask(&conn, hello).await["status"], 200);
    conn
}

#[tokio::test]
async fn only_a_visitor_who_is_in_can_set_up_a_direct_path() {
    let site = TestSite::start().await;
    let dir = tempfile::tempdir().unwrap();
    let host = start_host(&site, &dir).await;
    let offer = serde_json::json!({"t": "rtc", "sdp": "v=0\r\n"});

    let conn = hello_connection(&host, new_visitor_key()).await;
    let reply = ask(&conn, offer.clone()).await;
    assert_eq!(
        (reply["status"].clone(), reply["standing"].clone()),
        (403.into(), "denied".into()),
        "a stranger got {reply}"
    );

    let options = InviteOptions {
        first_visit_approval: true,
        ..Default::default()
    };
    let token = host.control().invite("mom", options).unwrap().token;
    ask(&conn, serde_json::json!({"t": "redeem", "token": token})).await;
    let reply = ask(&conn, offer).await;
    assert_eq!(reply["standing"], "awaiting_approval", "got {reply}");
    assert_eq!(site.requests(), 0);
}

#[tokio::test]
async fn a_connected_visitor_learns_at_once_when_the_host_stops() {
    let site = TestSite::start().await;
    let dir = tempfile::tempdir().unwrap();
    let host = start_host(&site, &dir).await;
    let invite = host
        .control()
        .invite("mom", InviteOptions::default())
        .unwrap();
    let visit = Visit::connect(host.address(), new_visitor_key(), Network::LocalOnly)
        .await
        .unwrap();
    visit.redeem(&invite.token).await.unwrap();

    host.shutdown().await;
    tokio::time::timeout(std::time::Duration::from_secs(1), visit.closed())
        .await
        .expect("the Visitor wasn't told within a second");
}

#[tokio::test]
async fn the_host_stops_promptly_even_with_a_request_stuck_on_the_site() {
    let site = TestSite::start().await;
    let dir = tempfile::tempdir().unwrap();
    let host = start_host(&site, &dir).await;
    let invite = host
        .control()
        .invite("mom", InviteOptions::default())
        .unwrap();
    let visit = Visit::connect(host.address(), new_visitor_key(), Network::LocalOnly)
        .await
        .unwrap();
    visit.redeem(&invite.token).await.unwrap();
    let stuck = visit.get("/hang");
    tokio::pin!(stuck);
    while site.requests() == 0 {
        tokio::select! {
            _ = &mut stuck => panic!("the Site answered /hang"),
            _ = tokio::time::sleep(std::time::Duration::from_millis(20)) => {}
        }
    }

    tokio::time::timeout(std::time::Duration::from_secs(5), host.shutdown())
        .await
        .expect("the Host didn't stop within 5 s");
}

#[tokio::test]
async fn a_host_only_ever_serves_the_site_it_was_made_for() {
    let site = TestSite::start().await;
    let other = TestSite::start().await;
    let folder = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let config = |site: Site| HostConfig {
        site,
        data_dir: dir.path().into(),
        network: Network::LocalOnly,
    };
    Host::start(config(Site::WebApp(site.addr)))
        .await
        .unwrap()
        .shutdown()
        .await;

    // Its Visitors were invited to this Site: the same Host Key must never serve them another.
    for wrong in [
        Site::WebApp(other.addr),
        Site::SharedFolder(folder.path().into()),
    ] {
        let refused = Host::start(config(wrong.clone())).await.err();
        let message = format!("{:#}", refused.expect("a different Site was let in"));
        assert!(message.contains(&site.addr.to_string()), "{message}");
    }
    Host::start(config(Site::WebApp(site.addr)))
        .await
        .expect("the same Site starts again")
        .shutdown()
        .await;
}

#[tokio::test]
async fn the_owner_can_ask_a_running_host_to_stop() {
    let site = TestSite::start().await;
    let dir = tempfile::tempdir().unwrap();
    let host = start_host(&site, &dir).await;

    host.control().stop();
    let asked = tokio::time::timeout(std::time::Duration::from_secs(1), host.stop_requested());
    assert!(
        asked.await.is_ok(),
        "the Host heard the Owner ask it to stop"
    );
    host.shutdown().await;
}
