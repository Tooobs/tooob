//! A Host on a closed Relay that hasn't enrolled its key is told so, with the Host Key to give its
//! Relay Operator (ADR 0030, #130): at `tooob add`, when `tooob run` starts and whenever it changes
//! while it runs, and in `tooob ls`. A Relay that can't be asked is unknown, never "not enrolled".
mod support;

use std::process::Output;
use support::home::{Home, TestRelay, a_folder};

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// How often a running Host checks its Relay again, in these tests.
const CHECK_EVERY: (&str, &str) = ("TOOOB_BOOTSTRAP_CHECK_EVERY", "1s");

/// The Relay as the line names it, `localhost:<port>` (a TestRelay is `http://localhost:<port>/`).
fn named(relay: &TestRelay) -> String {
    let port = relay.url.rsplit(':').next().unwrap().trim_end_matches('/');
    format!("localhost:{port}")
}

/// The line the ticket's decisions settle, word for word.
fn not_carried(relay: &TestRelay, key: &str) -> String {
    let at = named(relay);
    format!("{at} doesn't carry this Host yet: give its Relay Operator this Host Key: {key}")
}

/// A Sign-up Page whose URL already has a query, so the Host Key is joined with `&`.
const SIGN_UP: &str = "https://sign-up.example/join?relay=test";

/// The notice with the Sign-up Page link (#141), word for word.
fn not_carried_with_link(relay: &TestRelay, key: &str) -> String {
    format!(
        "{}; or sign up: {SIGN_UP}&key={key}",
        not_carried(relay, key)
    )
}

#[test]
fn add_on_a_closed_relay_with_a_sign_up_page_shows_its_link_and_returns() {
    let home = Home::new();
    let folder = a_folder();
    let relay = TestRelay::with_sign_up(SIGN_UP);
    relay.close();
    let started = std::time::Instant::now();
    let out = home.tooob(&[
        "add",
        "photos",
        folder.path().to_str().unwrap(),
        "--relay",
        &relay.url,
    ]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(started.elapsed() < std::time::Duration::from_secs(15));
    let key = home.shown("photos", "Host Key");
    assert!(
        stderr(&out).contains(&not_carried_with_link(&relay, &key)),
        "{}",
        stderr(&out)
    );
    let listed = home.ok(&["ls"]);
    let line = format!("photos: {}", not_carried_with_link(&relay, &key));
    assert!(listed.contains(&line), "{listed}");
    let json: serde_json::Value = serde_json::from_str(&home.ok(&["ls", "--json"])).unwrap();
    assert_eq!(json[0]["sign_up"], format!("{SIGN_UP}&key={key}"));
}

#[test]
fn share_that_adds_a_host_on_a_closed_relay_that_hasnt_enrolled_it_says_so_with_the_host_key() {
    let home = Home::new();
    let folder = a_folder();
    let relay = TestRelay::start();
    relay.close();
    // Detached, its service manager runs it, so only share itself can tell the Owner.
    home.install_tooob();
    home.stand_in("launchctl", "");
    home.stand_in("systemctl", "");
    let out = home.tooob(&[
        "share",
        folder.path().to_str().unwrap(),
        "--name",
        "photos",
        "--relay",
        &relay.url,
        "-d",
    ]);
    assert!(out.status.success(), "{}", stderr(&out));
    let key = home.shown("photos", "Host Key");
    assert!(
        stderr(&out).contains(&not_carried(&relay, &key)),
        "{}",
        stderr(&out)
    );
    assert_eq!(enrollment(&home), "not_enrolled");
}

fn enrollment(home: &Home) -> serde_json::Value {
    let listed: serde_json::Value = serde_json::from_str(&home.ok(&["ls", "--json"])).unwrap();
    listed[0]["enrollment"].clone()
}

#[test]
fn add_on_a_closed_relay_that_hasnt_enrolled_it_succeeds_and_says_so_with_the_host_key() {
    let home = Home::new();
    let folder = a_folder();
    let relay = TestRelay::start();
    relay.close();
    let out = home.tooob(&[
        "add",
        "photos",
        folder.path().to_str().unwrap(),
        "--relay",
        &relay.url,
    ]);
    assert!(out.status.success(), "{}", stderr(&out));
    let key = home.shown("photos", "Host Key");
    assert!(
        stderr(&out).contains(&format!("{}\n", not_carried(&relay, &key))),
        "{}",
        stderr(&out)
    );
    // No Sign-up Page: the notice as #130 made it, and no link in ls --json.
    assert!(!stderr(&out).contains("sign up"), "{}", stderr(&out));
    assert!(!home.ok(&["ls"]).contains("sign up"));
    let json: serde_json::Value = serde_json::from_str(&home.ok(&["ls", "--json"])).unwrap();
    assert!(json[0].get("sign_up").is_none(), "{json}");
}

#[test]
fn share_on_a_closed_relay_with_a_sign_up_page_shows_its_link() {
    let home = Home::new();
    let folder = a_folder();
    let relay = TestRelay::with_sign_up(SIGN_UP);
    relay.close();
    home.install_tooob();
    home.stand_in("launchctl", "");
    home.stand_in("systemctl", "");
    let out = home.tooob(&[
        "share",
        folder.path().to_str().unwrap(),
        "--name",
        "photos",
        "--relay",
        &relay.url,
        "-d",
    ]);
    assert!(out.status.success(), "{}", stderr(&out));
    let key = home.shown("photos", "Host Key");
    assert!(
        stderr(&out).contains(&not_carried_with_link(&relay, &key)),
        "{}",
        stderr(&out)
    );
}

#[test]
fn run_shows_the_link_checks_again_soon_and_drops_it_once_signed_up() {
    let home = Home::new();
    let folder = a_folder();
    let relay = TestRelay::with_sign_up(SIGN_UP);
    relay.close();
    home.ok(&[
        "add",
        "photos",
        folder.path().to_str().unwrap(),
        "--relay",
        &relay.url,
    ]);
    let key = home.shown("photos", "Host Key");
    // The usual schedule is an hour, so only the faster one can pick up the sign-up.
    let running = home.start_with(
        "photos",
        &[
            ("TOOOB_BOOTSTRAP_CHECK_EVERY", "1h"),
            ("TOOOB_SIGN_UP_CHECK_EVERY", "1s"),
        ],
    );
    running.until_it_says(&not_carried_with_link(&relay, &key));
    home.until_logged("photos", &not_carried_with_link(&relay, &key));

    relay.enroll_through_api(&key);
    running.until_it_says(&format!("{} now carries this Host", named(&relay)));
    assert_eq!(enrollment(&home), "enrolled");
    assert!(!home.ok(&["ls"]).contains("sign up"));
    let json: serde_json::Value = serde_json::from_str(&home.ok(&["ls", "--json"])).unwrap();
    assert!(json[0].get("sign_up").is_none(), "{json}");
}

#[test]
fn add_on_an_open_relay_says_nothing_of_it() {
    let home = Home::new();
    let folder = a_folder();
    let relay = TestRelay::start();
    let out = home.tooob(&[
        "add",
        "photos",
        folder.path().to_str().unwrap(),
        "--relay",
        &relay.url,
    ]);
    assert!(out.status.success());
    assert!(!stderr(&out).contains("carry"), "{}", stderr(&out));
    assert_eq!(enrollment(&home), "open");
}

#[test]
fn ls_shows_a_host_its_relay_doesnt_carry_under_its_table() {
    let home = Home::new();
    let folder = a_folder();
    let relay = TestRelay::start();
    relay.close();
    home.ok(&[
        "add",
        "photos",
        folder.path().to_str().unwrap(),
        "--relay",
        &relay.url,
    ]);
    let key = home.shown("photos", "Host Key");
    let listed = home.ok(&["ls"]);
    let line = format!("photos: {}", not_carried(&relay, &key));
    assert!(listed.contains(&format!("\n\n{line}")), "{listed}");
    assert_eq!(enrollment(&home), "not_enrolled");
}

#[test]
fn a_relay_that_cant_be_asked_is_unknown_not_not_enrolled() {
    let home = Home::new();
    let folder = a_folder();
    let relay = TestRelay::start();
    relay.close();
    relay.hang_up_on_enrolled();
    let out = home.tooob(&[
        "add",
        "photos",
        folder.path().to_str().unwrap(),
        "--relay",
        &relay.url,
    ]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(!stderr(&out).contains("carry"), "{}", stderr(&out));
    assert_eq!(enrollment(&home), "unknown");
    assert!(!home.ok(&["ls"]).contains("carry"));

    let running = home.start_with("photos", &[CHECK_EVERY]);
    std::thread::sleep(std::time::Duration::from_secs(3));
    assert!(!running.log().contains("carry"), "{}", running.log());
    assert_eq!(enrollment(&home), "unknown");
}

#[test]
fn run_says_so_at_start_and_logs_when_the_host_is_enrolled_and_when_its_removed() {
    let home = Home::new();
    let folder = a_folder();
    let relay = TestRelay::start();
    relay.close();
    home.ok(&[
        "add",
        "photos",
        folder.path().to_str().unwrap(),
        "--relay",
        &relay.url,
    ]);
    let key = home.shown("photos", "Host Key");
    let running = home.start_with("photos", &[CHECK_EVERY]);
    running.until_it_says(&not_carried(&relay, &key));
    home.until_logged("photos", &not_carried(&relay, &key));
    // Checked again (every 1 s) and still not carried: said at start, and not again.
    std::thread::sleep(std::time::Duration::from_secs(3));
    let logged = home.ok(&["logs", "photos", "-n", "all"]);
    assert_eq!(
        logged.matches(&not_carried(&relay, &key)).count(),
        1,
        "logged once: {logged}"
    );

    relay.enroll(&key);
    running.until_it_says(&format!("{} now carries this Host", named(&relay)));
    assert_eq!(enrollment(&home), "enrolled");
    assert!(!home.ok(&["ls"]).contains("carry"));

    relay.remove(&key);
    running.until_it_says(&format!(
        "{} doesn't carry this Host any more: give its Relay Operator this Host Key: {key}",
        named(&relay)
    ));
    assert_eq!(enrollment(&home), "not_enrolled");
    let log = running.log();
    assert_eq!(
        log.matches("now carries this Host").count(),
        1,
        "logged once: {log}"
    );
}
