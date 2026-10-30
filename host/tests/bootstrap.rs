//! What a Relay's Bootstrap says of itself (`/__tooob/version.json`, from its Bundle), judged
//! against the Protocol Version this tooob speaks (ADR 0017, ADR 0020).
mod support;

use std::process::Output;
use support::home::{Home, TestRelay, a_folder, this_version_json, version_json};
use tooob::Network;
use tooob::access::Redeemed;
use tooob::bootstrap::{BootstrapVersion, Found, Problem, Watch, judge};
use tooob::visit::Visit;

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn a_bootstrap_speaking_this_protocol_version_works_whatever_its_release() {
    let body = br#"{"release": "0.0.9", "protocolVersion": "tooob/0", "files": {}}"#;
    assert_eq!(
        judge(200, body),
        Ok(BootstrapVersion {
            release: "0.0.9".into(),
            protocol_version: "tooob/0".into(),
        })
    );
}

#[test]
fn a_bootstrap_speaking_another_protocol_version_is_a_mismatch() {
    let body = br#"{"release": "0.2.0", "protocolVersion": "tooob/1", "files": {}}"#;
    assert_eq!(
        judge(200, body),
        Err(Problem::Mismatch(BootstrapVersion {
            release: "0.2.0".into(),
            protocol_version: "tooob/1".into(),
        }))
    );
}

#[test]
fn a_relay_answering_with_anything_but_a_bundles_version_json_serves_no_bootstrap() {
    for (status, body) in [
        (404, &b"Not Found"[..]),
        (200, b"<!doctype html><title>Welcome to nginx</title>"),
        (200, br#"{"release": "0.1.0"}"#),
        (
            500,
            br#"{"release": "0.1.0", "protocolVersion": "tooob/0"}"#,
        ),
    ] {
        assert_eq!(judge(status, body), Err(Problem::NoBootstrap), "{status}");
    }
}

#[test]
fn run_refuses_a_relay_whose_bootstrap_speaks_another_protocol_version() {
    let home = Home::new();
    let folder = a_folder();
    let relay = TestRelay::start();
    home.ok(&[
        "add",
        "photos",
        folder.path().to_str().unwrap(),
        "--relay",
        &relay.url,
    ]);
    relay.serve_bootstrap(Some(version_json("0.2.0", "tooob/1")));

    let out = home.tooob(&["run", "photos"]);
    assert!(!out.status.success());
    let err = stderr(&out);
    for said in [
        "0.2.0 (tooob/1)",
        "tooob/0",
        "Visitors' browsers",
        "another Relay",
        "Release",
    ] {
        assert!(err.contains(said), "{said:?} in: {err}");
    }
    assert!(!err.contains("serving"), "{err}");
}

#[test]
fn run_refuses_a_relay_that_serves_no_bootstrap() {
    let home = Home::new();
    let folder = a_folder();
    let relay = TestRelay::start();
    let path = folder.path().to_str().unwrap();
    home.ok(&["add", "photos", path, "--relay", &relay.url]);
    relay.serve_bootstrap(None);
    let out = home.tooob(&["run", "photos"]);
    assert!(!out.status.success());
    let err = stderr(&out);
    for said in ["serves no Bootstrap", "Visitors' browsers", "another Relay"] {
        assert!(err.contains(said), "{said:?} in: {err}");
    }
    assert!(home.row("photos").contains("stopped"));
}

#[test]
fn run_starts_with_a_warning_when_its_relay_cant_be_reached_and_its_checks_settle_it() {
    let home = Home::new();
    let folder = a_folder();
    let relay = TestRelay::start();
    let path = folder.path().to_str().unwrap();
    home.ok(&["add", "photos", path, "--relay", &relay.url]);
    relay.hang_up_on_bootstrap();
    let running = home.start_with("photos", &[CHECK_EVERY]);
    let log = running.log();
    assert!(log.contains("(tooob/0), Bootstrap unknown"), "{log}");
    assert!(
        log.contains("warning: its Relay") && log.contains("can't be reached"),
        "{log}"
    );
    assert!(home.shown("photos", "Bootstrap").starts_with("unknown"));
    assert!(!home.row("photos").contains("bootstrap mismatch"));

    relay.serve_bootstrap(Some(version_json("0.0.9", "tooob/0")));
    running.until_it_says("serves Bootstrap 0.0.9 (tooob/0)");
    assert_eq!(home.shown("photos", "Bootstrap"), "0.0.9 (tooob/0)");
}

#[test]
fn a_starting_host_logs_its_relays_bootstrap_beside_its_own_release() {
    let home = Home::new();
    let folder = a_folder();
    let relay = TestRelay::start();
    relay.serve_bootstrap(Some(version_json("0.0.9", "tooob/0")));
    home.ok(&[
        "add",
        "photos",
        folder.path().to_str().unwrap(),
        "--relay",
        &relay.url,
    ]);
    let running = home.start("photos");
    let own = format!("tooob {} (tooob/0)", env!("CARGO_PKG_VERSION"));
    running.until_it_says(&format!("starting {own}, Bootstrap 0.0.9 (tooob/0)"));
}

#[test]
fn add_and_share_refuse_a_relay_whose_bootstrap_wont_do_and_add_nothing() {
    let home = Home::new();
    let folder = a_folder();
    let path = folder.path().to_str().unwrap();
    let relay = TestRelay::start();
    relay.serve_bootstrap(Some(version_json("0.2.0", "tooob/1")));
    let unreachable = TestRelay::start();
    unreachable.hang_up_on_bootstrap();
    for (relay, why) in [
        (&relay, "0.2.0 (tooob/1)"),
        (&unreachable, "can't be reached"),
    ] {
        for args in [
            vec!["add", "photos", path, "--relay", &relay.url],
            vec!["share", path, "--relay", &relay.url],
            vec!["share", path, "--relay", &relay.url, "--invite", "Ann"],
        ] {
            let out = home.tooob(&args);
            assert!(!out.status.success(), "{args:?}");
            let err = stderr(&out);
            assert!(err.contains(why), "{args:?}: {err}");
        }
    }
    assert_eq!(home.ok(&["ls"]).lines().count(), 1, "nothing was added");
}

#[test]
fn share_of_a_hosts_folder_refuses_to_run_it_once_its_relay_serves_no_bootstrap() {
    let home = Home::new();
    let folder = a_folder();
    let path = folder.path().to_str().unwrap();
    let relay = TestRelay::start();
    home.ok(&["add", "photos", path, "--relay", &relay.url]);
    relay.serve_bootstrap(None);
    let out = home.tooob(&["share", path]);
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("serves no Bootstrap"),
        "{}",
        stderr(&out)
    );
}

/// How often a running Host checks its Relay's Bootstrap again, in these tests.
const CHECK_EVERY: (&str, &str) = ("TOOOB_BOOTSTRAP_CHECK_EVERY", "1s");

#[tokio::test(flavor = "multi_thread")]
async fn a_bootstrap_that_changes_under_a_running_host_is_a_warning_and_it_keeps_serving() {
    let home = Home::new();
    let folder = a_folder();
    let relay = TestRelay::start();
    home.ok(&[
        "add",
        "photos",
        folder.path().to_str().unwrap(),
        "--relay",
        &relay.url,
    ]);
    let running = home.start_with("photos", &[CHECK_EVERY]);
    let link = home.ok(&["invite", "photos", "alice"]);
    let token = link.trim().rsplit("#invite=").next().unwrap();
    let key = iroh::PublicKey::from_z32(&home.shown("photos", "Host Key")).unwrap();
    let relay_url: iroh::RelayUrl = relay.url.parse().unwrap();
    let mut alice = [0u8; 32];
    getrandom::fill(&mut alice).unwrap();
    let visit = Visit::connect(
        iroh::EndpointAddr::new(key).with_relay_url(relay_url.clone()),
        alice,
        Network::Relay(relay_url),
    )
    .await
    .unwrap();
    assert_eq!(visit.redeem(token).await, Ok(Redeemed::Admitted));
    assert!(!home.row("photos").contains("bootstrap mismatch"));

    relay.serve_bootstrap(Some(version_json("0.2.0", "tooob/1")));
    running.until_it_says("warning: its Relay");
    assert!(
        running.log().contains("0.2.0 (tooob/1)"),
        "{}",
        running.log()
    );
    let row = home.row("photos");
    assert!(
        row.contains("running") && row.contains("bootstrap mismatch"),
        "{row}"
    );
    let shown = home.shown("photos", "Bootstrap");
    assert!(
        shown.contains("bootstrap mismatch") && shown.contains("tooob/1"),
        "{shown}"
    );
    let listed: serde_json::Value = serde_json::from_str(&home.ok(&["ls", "--json"])).unwrap();
    assert_eq!(listed[0]["bootstrap_mismatch"], true, "{listed}");

    let page = visit.get("/notes.txt").await.unwrap();
    assert_eq!(
        page.body, b"hello friends\n",
        "Visitors already connected keep working"
    );

    relay.serve_bootstrap(None);
    running.until_it_says("serves no Bootstrap");
    relay.serve_bootstrap(Some(this_version_json()));
    running.until_it_says("again");
    assert!(!home.row("photos").contains("bootstrap mismatch"));
    let shown = home.shown("photos", "Bootstrap");
    assert_eq!(shown, format!("{} (tooob/0)", env!("CARGO_PKG_VERSION")));
}

fn version(release: &str, protocol_version: &str) -> BootstrapVersion {
    BootstrapVersion {
        release: release.into(),
        protocol_version: protocol_version.into(),
    }
}

fn unreachable() -> Result<BootstrapVersion, Problem> {
    Err(Problem::Unreachable("connection refused".into()))
}

const RELAY: &str = "https://relay.example/";

#[test]
fn a_short_outage_of_the_relay_is_one_warning_and_nothing_when_it_ends() {
    let mut watch = Watch::new(RELAY, &Ok(version("0.1.0", "tooob/0")));
    let said = watch.update(unreachable()).unwrap();
    assert!(
        said.contains("warning") && said.contains("can't be reached"),
        "{said}"
    );
    assert_eq!(watch.update(unreachable()), None);
    assert_eq!(watch.found(), Found::Serves("0.1.0 (tooob/0)".into()));
    assert_eq!(watch.update(Ok(version("0.1.0", "tooob/0"))), None);
}

#[test]
fn a_mismatch_is_warned_of_once_even_across_an_outage() {
    let mut watch = Watch::new(RELAY, &Ok(version("0.1.0", "tooob/0")));
    let mismatch = || Err(Problem::Mismatch(version("0.2.0", "tooob/1")));
    let said = watch.update(mismatch()).unwrap();
    assert!(
        said.contains("warning") && said.contains("0.2.0 (tooob/1)"),
        "{said}"
    );
    assert!(watch.found().is_mismatch());
    assert!(watch.update(unreachable()).is_some());
    assert_eq!(watch.update(mismatch()), None);
    assert!(
        watch.found().is_mismatch(),
        "an outage doesn't undo what was seen"
    );

    let said = watch.update(Ok(version("0.1.0", "tooob/0"))).unwrap();
    assert!(
        said.contains("serves Bootstrap 0.1.0 (tooob/0) again"),
        "{said}"
    );
    let said = watch.update(Ok(version("0.1.1", "tooob/0"))).unwrap();
    assert!(
        said.contains("now serves Bootstrap 0.1.1 (tooob/0)"),
        "{said}"
    );
    assert!(!watch.found().is_mismatch());
}

#[test]
fn a_host_that_started_without_reaching_its_relay_says_what_the_first_check_finds() {
    let mut watch = Watch::new(RELAY, &unreachable());
    assert_eq!(watch.found(), Found::Unknown);
    assert_eq!(watch.update(unreachable()), None, "warned of at start");
    let said = watch.update(Ok(version("0.1.0", "tooob/0"))).unwrap();
    assert!(said.contains("serves Bootstrap 0.1.0 (tooob/0)"), "{said}");
    assert!(!said.contains("again"), "{said}");
}
