//! An Owner enrolls a Host with an Enrollment Code (ADR 0030, #132): `tooob add … --relay-code`,
//! `tooob share … --relay-code`, or `tooob enroll NAME CODE` for a Host already made. A refused
//! code leaves nothing behind.
mod support;

use std::process::Output;
use support::home::{Home, TestRelay, a_folder};

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn enrollment(home: &Home, name: &str) -> serde_json::Value {
    let listed: serde_json::Value = serde_json::from_str(&home.ok(&["ls", "--json"])).unwrap();
    listed
        .as_array()
        .unwrap()
        .iter()
        .find(|h| h["name"] == name)
        .map(|h| h["enrollment"].clone())
        .unwrap_or(serde_json::Value::Null)
}

fn names(home: &Home) -> usize {
    let listed: serde_json::Value = serde_json::from_str(&home.ok(&["ls", "--json"])).unwrap();
    listed.as_array().unwrap().len()
}

fn add_with_code(home: &Home, relay: &TestRelay, code: &str) -> Output {
    let folder = a_folder();
    home.tooob(&[
        "add",
        "photos",
        folder.path().to_str().unwrap(),
        "--relay",
        &relay.url,
        "--relay-code",
        code,
    ])
}

#[test]
fn add_with_a_relay_code_makes_a_host_enrolled_on_its_relay() {
    let home = Home::new();
    let relay = TestRelay::start();
    relay.close();
    let code = relay.code(1, 3600);
    let out = add_with_code(&home, &relay, &code);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stderr(&out).contains("is enrolled"), "{}", stderr(&out));
    assert!(!stderr(&out).contains("doesn't carry"), "{}", stderr(&out));
    assert!(!stderr(&out).contains(code.rsplit('/').next().unwrap()));
    assert_eq!(enrollment(&home, "photos"), "enrolled");
}

#[test]
fn share_with_a_relay_code_makes_a_host_enrolled_on_its_relay() {
    let home = Home::new();
    let folder = a_folder();
    let relay = TestRelay::start();
    relay.close();
    let code = relay.code(1, 3600);
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
        "--relay-code",
        &code,
        "-d",
    ]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(enrollment(&home, "photos"), "enrolled");
}

#[test]
fn a_relay_code_for_another_relay_than_relay_names_is_refused_before_anything_is_made() {
    let home = Home::new();
    let relay = TestRelay::start();
    let out = add_with_code(&home, &relay, "r.example.net/7Q4K-M2XD-9PHT-WB3C");
    assert!(!out.status.success());
    assert!(stderr(&out).contains("r.example.net"), "{}", stderr(&out));
    assert_eq!(names(&home), 0);
}

fn refused_in_one_line(home: &Home, out: &Output, why: &str) {
    assert!(!out.status.success());
    let said = stderr(out);
    assert_eq!(said.trim().lines().count(), 1, "{said}");
    assert!(said.contains(why), "{said}");
    assert_eq!(names(home), 0, "no Host left: {said}");
}

#[test]
fn a_wrong_code_fails_in_one_line_and_leaves_no_host() {
    let home = Home::new();
    let relay = TestRelay::start();
    relay.close();
    let at = relay
        .url
        .trim_start_matches("http://")
        .trim_end_matches('/');
    let out = add_with_code(&home, &relay, &format!("{at}/AAAA-BBBB-CCCC-DDDD"));
    refused_in_one_line(&home, &out, "wrong");
}

#[test]
fn an_expired_code_fails_in_one_line_and_leaves_no_host() {
    let home = Home::new();
    let relay = TestRelay::start();
    relay.close();
    let code = relay.code(1, 0);
    let out = add_with_code(&home, &relay, &code);
    refused_in_one_line(&home, &out, "expired");
}

#[test]
fn a_used_up_code_fails_in_one_line_and_leaves_no_host() {
    let home = Home::new();
    let relay = TestRelay::start();
    relay.close();
    let code = relay.code(0, 3600);
    let out = add_with_code(&home, &relay, &code);
    refused_in_one_line(&home, &out, "used up");
}

#[test]
fn a_cancelled_code_fails_in_one_line_and_leaves_no_host() {
    let home = Home::new();
    let relay = TestRelay::start();
    relay.close();
    let code = relay.code(1, 3600);
    relay.cancel(&code);
    let out = add_with_code(&home, &relay, &code);
    refused_in_one_line(&home, &out, "cancelled");
}

fn add_plain(home: &Home, relay: &TestRelay) {
    let folder = a_folder();
    home.ok(&[
        "add",
        "photos",
        folder.path().to_str().unwrap(),
        "--relay",
        &relay.url,
    ]);
}

#[test]
fn enroll_enrolls_a_host_already_made() {
    let home = Home::new();
    let relay = TestRelay::start();
    relay.close();
    add_plain(&home, &relay);
    assert_eq!(enrollment(&home, "photos"), "not_enrolled");
    let code = relay.code(1, 3600);
    let out = home.tooob(&["enroll", "photos", &code]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(enrollment(&home, "photos"), "enrolled");
}

#[test]
fn enroll_refuses_a_code_for_another_relay() {
    let home = Home::new();
    let relay = TestRelay::start();
    relay.close();
    add_plain(&home, &relay);
    let out = home.tooob(&["enroll", "photos", "r.example.net/7Q4K-M2XD-9PHT-WB3C"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("r.example.net"), "{}", stderr(&out));
    assert_eq!(enrollment(&home, "photos"), "not_enrolled");
}

#[test]
fn enroll_with_a_refused_code_leaves_the_host_as_it_was() {
    let home = Home::new();
    let relay = TestRelay::start();
    relay.close();
    add_plain(&home, &relay);
    let key = home.shown("photos", "Host Key");
    let code = relay.code(0, 3600);
    let out = home.tooob(&["enroll", "photos", &code]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("used up"), "{}", stderr(&out));
    assert_eq!(home.shown("photos", "Host Key"), key);
    assert_eq!(enrollment(&home, "photos"), "not_enrolled");
}

#[test]
fn share_refuses_a_relay_code_for_a_host_already_serving_the_folder() {
    let home = Home::new();
    let folder = a_folder();
    let relay = TestRelay::start();
    relay.close();
    let path = folder.path().to_str().unwrap();
    home.ok(&["add", "photos", path, "--relay", &relay.url]);
    let code = relay.code(1, 3600);
    let out = home.tooob(&["share", path, "--relay-code", &code]);
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("tooob enroll photos"),
        "{}",
        stderr(&out)
    );
}
