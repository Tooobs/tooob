//! The `tooob` Owner commands, end to end: a Host run with `tooob run` in one process, Owner
//! commands in others.
mod support;

use std::process::Output;
use support::TestSite;
use support::home::{Home, Running, TestRelay};

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// A Host named `site` for the web app, running through a Relay of its own.
fn serve(home: &Home, site: &TestSite) -> (Running, TestRelay) {
    let relay = TestRelay::start();
    let addr = site.addr.to_string();
    home.ok(&["add", "site", &addr, "--relay", &relay.url]);
    (home.start("site"), relay)
}

/// What `--version` and a starting Host say: the Release, from Cargo.toml, and the Protocol Version.
fn release_and_protocol_version() -> String {
    format!("tooob {} (tooob/0)", env!("CARGO_PKG_VERSION"))
}

#[test]
fn version_prints_the_release_and_the_protocol_version() {
    let out = Home::new().tooob(&["--version"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out).trim(), release_and_protocol_version());
}

#[tokio::test]
async fn a_starting_host_logs_the_release_and_the_protocol_version() {
    let site = TestSite::start().await;
    let home = Home::new();
    let (serving, _relay) = serve(&home, &site);
    serving.until_it_says(&release_and_protocol_version());
}

#[tokio::test]
async fn invite_prints_a_link_for_the_running_host() {
    let site = TestSite::start().await;
    let home = Home::new();
    let _serving = serve(&home, &site);

    let link = home.ok(&["invite", "site", "mom"]);
    let link = link.trim();
    let key = home.shown("site", "Host Key");
    assert!(
        link.starts_with(&format!("http://{key}.localhost:")),
        "{link}"
    );
    assert!(link.contains("/#invite="), "{link}");

    let listed = home.ok(&["invites", "site"]);
    assert!(listed.contains("mom"), "{listed}");
}

#[test]
fn stopping_a_host_that_isnt_running_says_how_to_run_it() {
    let home = Home::new();
    home.ok(&["add", "site", "127.0.0.1:8096", "--local"]);
    let out = home.tooob(&["stop", "site"]);
    assert!(!out.status.success());
    let err = stderr(&out);
    assert!(
        err.contains("isn't running") && err.contains("tooob run site"),
        "{err}"
    );
}

#[tokio::test]
async fn pair_prints_a_link_and_a_bad_key_is_reported_not_ignored() {
    let site = TestSite::start().await;
    let home = Home::new();
    let _serving = serve(&home, &site);

    assert!(home.ok(&["pair", "site", "my phone"]).contains("#invite="));
    let out = home.tooob(&["block", "site", "ffffffffffff"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("unknown_visitor"));
}

#[tokio::test]
async fn invite_expiry_can_be_given_in_hours_and_bad_durations_are_errors() {
    let site = TestSite::start().await;
    let home = Home::new();
    let _serving = serve(&home, &site);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    home.ok(&["invite", "site", "mom", "--expires", "1h"]);
    let listed = home.ok(&["invites", "site"]);
    let expires: u64 = listed.split_whitespace().last().unwrap().parse().unwrap();
    assert!((now + 3590..=now + 3610).contains(&expires), "{listed}");

    let bad = home.tooob(&["invite", "site", "mom", "--expires", "soon"]);
    assert!(!bad.status.success() && stderr(&bad).contains("bad duration"));
}

#[tokio::test]
async fn an_absurd_expiry_is_a_bad_duration_not_a_crash() {
    let site = TestSite::start().await;
    let home = Home::new();
    let _serving = serve(&home, &site);
    let out = home.tooob(&["invite", "site", "mom", "--expires", "999999999999999999d"]);
    let err = stderr(&out);
    assert!(
        !out.status.success() && err.contains("bad duration") && !err.contains("panicked"),
        "{err}"
    );
    assert_eq!(stdout(&out), "");
}
