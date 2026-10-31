//! How much each Visitor uses (ADR 0025): the bytes sent to them and received from them, on
//! every path, as a Visit's "left" line says and `tooob visitors` totals them across restarts.
//! Through the `tooob` command with real Hosts on a Relay of their own, and Visitors connecting
//! as a browser does.
mod support;

use serde_json::Value;
use support::TestSite;
use support::home::{Home, Running, TestRelay};
use tooob::Network;
use tooob::access::Redeemed;
use tooob::visit::Visit;

/// A Visit ends a second after its Visitor's last connection, and a Visit with no Direct Path is
/// said to be relayed after a second: a minute and 10 s otherwise.
const QUICK: [(&str, &str); 2] = [("TOOOB_VISIT_GRACE", "1s"), ("TOOOB_DIRECT_WAIT", "1s")];

/// Host media, for a stand-in web app, running on a Relay of its own.
struct Media {
    home: Home,
    relay: TestRelay,
    running: Running,
    _site: TestSite,
}

impl Media {
    async fn start_with(env: &[(&str, &str)]) -> Media {
        let home = Home::new();
        let site = TestSite::start().await;
        let relay = TestRelay::start();
        home.ok(&[
            "add",
            "media",
            &site.addr.to_string(),
            "--relay",
            &relay.url,
        ]);
        let running = home.start_with("media", env);
        Media {
            home,
            relay,
            running,
            _site: site,
        }
    }

    /// A Visitor let in by an Invite for `label`, on a connection as a browser tab makes one.
    async fn visitor(&self, label: &str) -> ([u8; 32], Visit) {
        let link = self.home.ok(&["invite", "media", label]);
        let token = link.trim().rsplit("#invite=").next().unwrap().to_string();
        let key = new_key();
        let tab = self.connect(key).await;
        assert_eq!(tab.redeem(&token).await, Ok(Redeemed::Admitted));
        (key, tab)
    }

    async fn connect(&self, key: [u8; 32]) -> Visit {
        let host = iroh::PublicKey::from_z32(&self.home.shown("media", "Host Key")).unwrap();
        let relay: iroh::RelayUrl = self.relay.url.parse().unwrap();
        Visit::connect(
            iroh::EndpointAddr::new(host).with_relay_url(relay.clone()),
            key,
            Network::Relay(relay),
        )
        .await
        .unwrap()
    }

    /// Wait until its log says `what`.
    fn until_logged(&self, what: &str) {
        self.home.until_logged("media", what);
    }

    /// Its log, as `tooob logs` shows it, without times.
    fn said(&self) -> Vec<String> {
        let shown = self.home.ok(&["logs", "media", "-n", "all"]);
        shown
            .lines()
            .map(|l| l.split_once("  media: ").map_or(l, |(_, said)| said))
            .map(String::from)
            .collect()
    }

    /// Its log's lines that are `event`, as `tooob logs --json` gives them.
    fn events(&self, event: &str) -> Vec<Value> {
        self.home
            .ok(&["logs", "media", "-n", "all", "--json"])
            .lines()
            .map(|l| serde_json::from_str::<Value>(l).unwrap())
            .filter(|l| l["event"] == event)
            .collect()
    }
}

fn new_key() -> [u8; 32] {
    let mut key = [0u8; 32];
    getrandom::fill(&mut key).unwrap();
    key
}

/// The short Visitor Key the log names a device by.
fn short(key: &[u8; 32]) -> String {
    iroh::SecretKey::from_bytes(key).public().to_string()[..8].to_string()
}

/// `n` is `at_least`, give or take the requests' and replies' own lines: never less, and not
/// more than a few KB over.
fn about(n: &Value, at_least: u64) -> bool {
    n.as_u64()
        .is_some_and(|n| (at_least..at_least + 8_000).contains(&n))
}

#[tokio::test(flavor = "multi_thread")]
async fn a_visits_left_line_says_what_was_sent_to_the_visitor_and_received_from_them() {
    let media = Media::start_with(&QUICK).await;
    let (jm, tab) = media.visitor("jm").await;
    let photo = tab.get("/bytes/2300000").await.unwrap();
    assert_eq!(photo.body.len(), 2_300_000);
    let upload = vec![1u8; 450_000];
    let stored = tab
        .upload("POST", "/sha256", &[], &upload[..])
        .await
        .unwrap();
    assert!(String::from_utf8_lossy(&stored.body).starts_with("450000 "));
    tab.close().await;
    let left = format!("jm ({}) left after ", short(&jm));
    media.until_logged(&left);

    let said = media.said();
    let line = said.iter().find(|l| l.starts_with(&left)).unwrap();
    assert!(
        line.ends_with(" s · sent 2.3 MB, received 450 KB"),
        "{said:#?}"
    );
    let left = &media.events("left")[0]["fields"];
    assert!(about(&left["sent"], 2_300_000), "{left}");
    assert!(about(&left["received"], 450_000), "{left}");
}

/// What `tooob visitors --json` says of the Visitor whose Visitor Key is `key`.
fn listed(home: &Home, key: &[u8; 32]) -> Value {
    let full = iroh::SecretKey::from_bytes(key).public().to_string();
    let visitors: Value = serde_json::from_str(&home.ok(&["visitors", "media", "--json"])).unwrap();
    visitors
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["key"] == full)
        .unwrap_or_else(|| panic!("{full} isn't listed: {visitors:#}"))
        .clone()
}

/// The row `tooob visitors` prints for the Visitor whose Visitor Key is `key`, and its header.
fn row(home: &Home, key: &[u8; 32]) -> (String, String) {
    let shown = home.ok(&["visitors", "media"]);
    let header = shown.lines().next().unwrap().to_string();
    let row = shown
        .lines()
        .find(|l| l.starts_with(&short(key)))
        .unwrap_or_else(|| panic!("{} isn't listed:\n{shown}", short(key)))
        .to_string();
    (header, row)
}

/// Seconds since the Unix epoch.
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

#[tokio::test(flavor = "multi_thread")]
async fn tooob_visitors_totals_what_each_visitor_used_since_they_first_got_in_across_restarts() {
    let mut media = Media::start_with(&QUICK).await;
    let (jm, tab) = media.visitor("jm").await;
    tab.get("/bytes/1000000").await.unwrap();
    tab.close().await;
    media.until_logged(&format!("jm ({}) left after ", short(&jm)));

    // Stopped, it says what it kept.
    assert!(media.running.stop_with(libc::SIGTERM).success());
    let kept = listed(&media.home, &jm);
    assert!(about(&kept["sent"], 1_000_000), "{kept}");
    let seen = kept["last_seen"].as_u64().unwrap();
    assert!(now() - seen < 30, "{kept}");

    // Running again, the next Visit adds to it, a WebSocket's frames too; and while they're on
    // it, they were last seen now.
    media.running = media.home.start_with("media", &QUICK);
    let tab = media.connect(jm).await;
    tab.get("/bytes/500000").await.unwrap();
    let mut socket = tab.websocket("/ws").await.unwrap();
    let hello = "x".repeat(20_000);
    socket.send_text(&hello).await.unwrap();
    assert!(
        socket
            .next_text()
            .await
            .unwrap()
            .unwrap()
            .starts_with("xxx")
    );
    let visiting = listed(&media.home, &jm);
    assert!(about(&visiting["sent"], 1_520_000), "{visiting}");
    assert!(about(&visiting["received"], 20_000), "{visiting}");
    assert!(
        now() - visiting["last_seen"].as_u64().unwrap() < 5,
        "{visiting}"
    );
    let (header, shown) = row(&media.home, &jm);
    let columns: Vec<&str> = header.split("  ").filter(|c| !c.is_empty()).collect();
    let columns: Vec<&str> = columns.iter().map(|c| c.trim()).collect();
    assert_eq!(
        columns,
        ["KEY", "LABEL", "STATUS", "LAST SEEN", "SENT", "RECEIVED"],
        "{header}"
    );
    let shown: Vec<&str> = shown.split_whitespace().collect();
    assert_eq!(shown[1..6], ["jm", "Allowed", "now", "1.5", "MB"]);
    assert!(shown[6].starts_with("20.") && shown[7] == "KB", "{shown:?}");
    drop(socket);
    tab.close().await;
    media.until_logged(" · sent 520 KB, received 20.");

    // Someone never let in has used nothing, and has never been seen.
    let link = media.home.ok(&["invite", "media", "gran", "--approve"]);
    let token = link.trim().rsplit("#invite=").next().unwrap().to_string();
    let gran = new_key();
    let tab = media.connect(gran).await;
    assert_eq!(tab.redeem(&token).await, Ok(Redeemed::AwaitingApproval));
    let never = listed(&media.home, &gran);
    assert_eq!(
        (&never["sent"], &never["received"], &never["last_seen"]),
        (&0.into(), &0.into(), &Value::Null)
    );
    let (_, shown) = row(&media.home, &gran);
    let shown: Vec<&str> = shown.split_whitespace().collect();
    assert_eq!(
        shown[1..],
        ["gran", "AwaitingApproval", "-", "0", "B", "0", "B"]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_host_that_crashes_mid_visit_still_has_what_it_saved_every_so_often() {
    let mut media = Media::start_with(&[
        ("TOOOB_VISIT_GRACE", "1s"),
        ("TOOOB_DIRECT_WAIT", "1s"),
        ("TOOOB_USAGE_SAVE_EVERY", "1s"),
    ])
    .await;
    let (jm, tab) = media.visitor("jm").await;
    tab.get("/bytes/1000000").await.unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    media.running.stop_with(libc::SIGKILL);

    let kept = listed(&media.home, &jm);
    assert!(about(&kept["sent"], 1_000_000), "{kept}");
    assert!(now() - kept["last_seen"].as_u64().unwrap() < 10, "{kept}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_host_whose_usage_file_is_spoilt_still_starts_and_counts_afresh() {
    let home = Home::new();
    let site = TestSite::start().await;
    let relay = TestRelay::start();
    home.ok(&[
        "add",
        "media",
        &site.addr.to_string(),
        "--relay",
        &relay.url,
    ]);
    let state = home.shown("media", "state folder");
    std::fs::create_dir_all(&state).unwrap();
    std::fs::write(format!("{state}/usage.json"), "{ half a fi").unwrap();
    let running = home.start_with("media", &QUICK);
    let media = Media {
        home,
        relay,
        running,
        _site: site,
    };
    media.until_logged("usage.json isn't readable");
    // Kept aside, as it was.
    let bad = std::fs::read_to_string(format!("{state}/usage.bad")).unwrap();
    assert_eq!(bad, "{ half a fi");
    let (jm, tab) = media.visitor("jm").await;
    tab.get("/bytes/1000").await.unwrap();
    assert!(about(&listed(&media.home, &jm)["sent"], 1000));
}

#[tokio::test(flavor = "multi_thread")]
async fn setting_up_a_direct_path_isnt_counted_as_what_a_visitor_used() {
    let media = Media::start_with(&QUICK).await;
    let (jm, tab) = media.visitor("jm").await;
    // Offers the Host can't use, each nearly as big as a line may be: its signaling, not the Site's.
    let offer = "x".repeat(50_000);
    for _ in 0..4 {
        assert_eq!(tab.offer_direct_path(&offer).await, Ok(400));
    }
    tab.close().await;
    media.until_logged(&format!("jm ({}) left after ", short(&jm)));
    let left = &media.events("left")[0]["fields"];
    assert!(about(&left["sent"], 0), "{left}");
    assert!(about(&left["received"], 0), "{left}");
}

#[tokio::test(flavor = "multi_thread")]
async fn what_a_device_sent_while_it_was_refused_isnt_counted_once_its_let_in() {
    let media = Media::start_with(&QUICK).await;
    let link = media.home.ok(&["invite", "media", "gran", "--approve"]);
    let token = link.trim().rsplit("#invite=").next().unwrap().to_string();
    let gran = new_key();
    let tab = media.connect(gran).await;
    assert_eq!(tab.redeem(&token).await, Ok(Redeemed::AwaitingApproval));
    // Asking again and again while she waits, each time with a big request.
    let padding = "x".repeat(50_000);
    for _ in 0..4 {
        let asked = tab
            .request("GET", "/", &[("x-padding", &padding)], b"")
            .await;
        assert_eq!(asked, Err(tooob::visit::VisitError::AwaitingApproval));
    }
    media.home.ok(&["approve", "media", &short(&gran)]);
    tab.get("/bytes/1000").await.unwrap();
    let used = listed(&media.home, &gran);
    assert!(about(&used["sent"], 1000), "{used}");
    assert!(about(&used["received"], 0), "{used}");
}

#[tokio::test(flavor = "multi_thread")]
async fn saving_usage_every_0s_is_refused() {
    let home = Home::new();
    let site = TestSite::start().await;
    let relay = TestRelay::start();
    home.ok(&[
        "add",
        "media",
        &site.addr.to_string(),
        "--relay",
        &relay.url,
    ]);
    let out = home.tooob_with(&["run", "media"], &[("TOOOB_USAGE_SAVE_EVERY", "0s")]);
    assert!(!out.status.success());
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(said.contains("TOOOB_USAGE_SAVE_EVERY"), "{said}");
}
