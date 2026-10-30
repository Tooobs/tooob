//! The `tooob` command with a person at the keyboard, in a real (pseudo) terminal: it asks nothing
//! there either (ADR 0022). Each command finishes, serves, or fails with what's missing, and none
//! waits for input. Everything else is in tests/hosts.rs, run as a script would.
mod support;

use expectrl::{Expect, Session, process::unix::WaitStatus, session::OsSession};
use std::{process::Command, time::Duration};
use support::home::{Home, RELAY, TestRelay, a_folder, name_of};

/// `program ARGS` in a terminal of its own, under the test's TOOOB_HOME.
fn spawn_in_terminal(home: &Home, program: &str, args: &[&str]) -> OsSession {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .env("TOOOB_HOME", home.0.path())
        .env("HOME", home.0.path())
        .env("TERM", "xterm-256color");
    let mut session = Session::spawn(cmd).unwrap();
    session.set_expect_timeout(Some(Duration::from_secs(30)));
    session
}

/// `tooob ARGS` in a terminal of its own.
fn in_terminal(home: &Home, args: &[&str]) -> OsSession {
    spawn_in_terminal(home, env!("CARGO_BIN_EXE_tooob"), args)
}

/// Run `tooob ARGS` in a terminal, sending it nothing: everything it said, once it has ended,
/// and whether it succeeded. It must end within 10 s, so a command waiting for input fails.
fn said_in_terminal(home: &Home, args: &[&str]) -> (String, bool) {
    let mut t = in_terminal(home, args);
    t.set_expect_timeout(Some(Duration::from_secs(10)));
    let said = t
        .expect(expectrl::Eof)
        .unwrap_or_else(|e| panic!("{args:?} is still waiting: {e}"));
    let said = String::from_utf8_lossy(said.get(0).unwrap()).into_owned();
    let ok = matches!(t.get_process().wait().unwrap(), WaitStatus::Exited(_, 0));
    (said, ok)
}

/// The Host Name `share` gives a folder.
fn host_name(folder: &std::path::Path) -> String {
    name_of(folder).to_lowercase()
}

#[test]
fn share_with_everything_given_asks_nothing_prints_the_link_and_serves_until_ctrl_c() {
    let home = Home::new();
    let folder = a_folder();
    let name = host_name(folder.path());
    let relay = TestRelay::start();
    let path = folder.path().to_str().unwrap();
    let args = [
        "share",
        path,
        "--relay",
        &relay.url,
        "--invite",
        "Alice",
        "--expires",
        "1d",
    ];
    let mut t = in_terminal(&home, &args);

    let said = t
        .expect(format!("Invite for Alice to {name}, works for 1 day"))
        .unwrap();
    let said = String::from_utf8_lossy(said.before()).into_owned();
    assert!(said.contains(&format!("{name}: serving")), "{said}");
    assert!(!said.contains('?'), "asked something: {said}");
    t.expect(".localhost:").unwrap();
    t.expect("/#invite=").unwrap();
    assert!(home.row(&name).contains("running"));
    assert!(home.ok(&["invites", &name]).contains("Alice"));

    t.send("\x03").unwrap(); // Ctrl-C
    t.expect(format!("{name}: stopped")).unwrap();
    t.expect(expectrl::Eof).unwrap();
    assert!(home.row(&name).contains("stopped"));
}

#[test]
fn share_started_with_an_ampersand_prints_the_link_and_keeps_serving_in_the_background() {
    let home = Home::new();
    let folder = a_folder();
    let name = host_name(folder.path());
    let relay = TestRelay::start();
    let mut sh = spawn_in_terminal(&home, "/bin/bash", &["--norc", "--noprofile", "-i"]);
    sh.send_line("PS1='$ '").unwrap();

    sh.send_line(format!(
        "'{}' share '{}' --relay {} --invite Alice &",
        env!("CARGO_BIN_EXE_tooob"),
        folder.path().display(),
        relay.url
    ))
    .unwrap();
    sh.expect("/#invite=").unwrap();
    // A job that read the terminal from the background would be stopped by the shell.
    sh.send_line("sleep 1; jobs -l").unwrap();
    sh.expect("Running").unwrap();
    assert!(home.row(&name).contains("running"));
    assert!(home.ok(&["invites", &name]).contains("Alice"));

    sh.send_line("kill %1").unwrap();
    sh.expect(format!("{name}: stopped")).unwrap();
    sh.send_line("exit").unwrap();
    sh.expect(expectrl::Eof).unwrap();
}

#[test]
fn every_command_missing_something_fails_at_once_with_the_command_to_type() {
    let home = Home::new();
    let (photos, other, repo) = (a_folder(), a_folder(), a_folder());
    std::fs::create_dir(repo.path().join(".git")).unwrap();
    home.add_naming("photos", photos.path().to_str().unwrap(), RELAY);
    home.ok(&["add", "films", "127.0.0.1:8096", "--local"]);
    let other = other.path().to_str().unwrap();
    let repo = repo.path().to_str().unwrap();

    for (args, says) in [
        (vec!["share"], "tooob share PATH"),
        (vec!["share", other], "--relay https://RELAY/"),
        (vec!["add", "other", other], "--relay https://RELAY/"),
        (vec!["share", repo, "--local"], "--include-git --local"),
        (vec!["invite"], "(Hosts here: films, photos)"),
        (vec!["pair"], "(Hosts here: films, photos)"),
        (vec!["stop"], "(none is running)"),
        (vec!["invite", "photos"], "tooob invite photos LABEL"),
        (vec!["pair", "photos"], "tooob pair photos LABEL"),
        (vec!["invite", "films"], "no Key Address"),
        (vec!["pair", "films"], "no Key Address"),
    ] {
        let (said, ok) = said_in_terminal(&home, &args);
        assert!(!ok, "{args:?} succeeded: {said}");
        assert!(said.contains(says), "{args:?}: {said}");
        assert!(!said.contains('?'), "{args:?} asked: {said}");
    }
    assert_eq!(home.ok(&["ls"]).lines().count(), 3, "nothing was added");
    assert!(home.ok(&["invites", "photos"]).is_empty(), "no Invite made");
}

#[test]
fn invite_with_everything_given_prints_only_the_link() {
    let home = Home::new();
    let folder = a_folder();
    home.add_naming("photos", folder.path().to_str().unwrap(), RELAY);

    let (said, ok) = said_in_terminal(&home, &["invite", "photos", "Bob"]);
    assert!(ok, "{said}");
    let key = home.shown("photos", "Host Key");
    let link = format!("https://{key}.relay.example/#invite=");
    assert!(said.trim().starts_with(&link), "{said}");
    assert_eq!(said.trim().lines().count(), 1, "the link alone: {said}");
    assert!(home.ok(&["invites", "photos"]).contains("Bob"));
}

#[test]
fn closing_the_terminal_while_sharing_stops_the_host() {
    let home = Home::new();
    let folder = a_folder();
    let name = host_name(folder.path());
    let mut t = in_terminal(
        &home,
        &["share", folder.path().to_str().unwrap(), "--local"],
    );

    let said = t.expect(format!("{name}: serving")).unwrap();
    let said = String::from_utf8_lossy(said.before()).into_owned();
    assert!(!said.contains('?'), "asked something: {said}");
    let pid = t.get_process().pid().as_raw();
    unsafe { libc::kill(pid, libc::SIGHUP) }; // what a closing terminal sends
    t.expect(format!("{name}: stopped")).unwrap();
    assert!(home.row(&name).contains("stopped"));
}
