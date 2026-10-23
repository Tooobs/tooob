//! The `tooob` command with a person at the keyboard: questions and pickers, in a real (pseudo)
//! terminal. Everything else is in tests/hosts.rs, run as a script would.
mod support;

use expectrl::{Expect, Session, session::OsSession};
use std::{process::Command, time::Duration};
use support::home::{BOOTSTRAP, Home, a_folder, name_of};

/// `tooob` in a terminal of its own, under the test's TOOOB_HOME.
fn in_terminal(home: &Home, args: &[&str]) -> OsSession {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_tooob"));
    cmd.args(args)
        .env("TOOOB_HOME", home.0.path())
        .env("HOME", home.0.path())
        .env("TERM", "xterm-256color");
    let mut session = Session::spawn(cmd).unwrap();
    session.set_expect_timeout(Some(Duration::from_secs(30)));
    session
}

const ENTER: &str = "\r";

/// The Host Name `share` gives a folder.
fn host_name(folder: &std::path::Path) -> String {
    name_of(folder).to_lowercase()
}

/// Only the heading: no Host was added.
fn no_hosts(home: &Home) -> bool {
    home.ok(&["ls"]).lines().count() == 1
}

#[test]
fn guided_share_asks_what_and_who_then_serves_here_until_ctrl_c() {
    let home = Home::new();
    let folder = a_folder();
    let name = host_name(folder.path());
    let mut t = in_terminal(&home, &["share", "--local", "--bootstrap", BOOTSTRAP]);

    t.expect("What do you want to share?").unwrap();
    t.send_line(folder.path().to_str().unwrap()).unwrap();
    t.expect("1 file").unwrap();
    t.expect("Who's it for?").unwrap();
    t.send_line("Alice").unwrap();
    t.expect("How long should the link work?").unwrap();
    t.send_line("1d").unwrap();
    t.expect("Approve their device").unwrap();
    t.send(ENTER).unwrap();
    t.expect("Who else?").unwrap();
    t.send(ENTER).unwrap();
    t.expect(format!("{name}: serving")).unwrap();
    t.expect(format!("Invite for Alice to {name}, works for 1 day"))
        .unwrap();
    t.expect(format!("{BOOTSTRAP}#")).unwrap();
    assert!(home.row(&name).contains("running"));

    t.send("\x03").unwrap();
    t.expect(format!("{name}: stopped")).unwrap();
    t.expect(expectrl::Eof).unwrap();
    assert!(home.row(&name).contains("stopped"));
}

#[test]
fn share_of_a_new_folder_asks_for_its_relay_and_bootstrap_and_never_guesses() {
    let home = Home::new();
    let folder = a_folder();
    let name = host_name(folder.path());
    let mut t = in_terminal(&home, &["share", folder.path().to_str().unwrap()]);

    t.expect("Which Relay do Visitors reach it through?")
        .unwrap();
    t.send_line("none").unwrap();
    t.expect("Which Bootstrap do its Invite Links open?")
        .unwrap();
    t.send_line("https://b.example/").unwrap();
    t.expect("Who's it for?").unwrap();
    t.send_line("Alice").unwrap();
    t.expect("How long should the link work?").unwrap();
    t.send(ENTER).unwrap();
    t.expect("Approve their device").unwrap();
    t.send(ENTER).unwrap();
    t.expect("Who else?").unwrap();
    t.send(ENTER).unwrap();
    t.expect("https://b.example/#").unwrap();
    assert_eq!(home.shown(&name, "relay"), "none");
    assert_eq!(home.shown(&name, "bootstrap"), "https://b.example/");
    t.send("\x03").unwrap();
    t.expect(expectrl::Eof).unwrap();

    let script = home.tooob(&["share", a_folder().path().to_str().unwrap()]);
    assert!(
        !script.status.success(),
        "a script is never asked, nor guessed for"
    );
}

#[test]
fn sharing_a_folder_with_a_git_history_warns_and_declining_shares_nothing() {
    let home = Home::new();
    let folder = a_folder();
    std::fs::create_dir(folder.path().join(".git")).unwrap();
    let mut t = in_terminal(
        &home,
        &[
            "share",
            folder.path().to_str().unwrap(),
            "--local",
            "--bootstrap",
            BOOTSTRAP,
        ],
    );

    t.expect("it includes .git").unwrap();
    t.expect("Share it anyway?").unwrap();
    t.send("n").unwrap();
    t.expect("nothing shared").unwrap();
    t.expect(expectrl::Eof).unwrap();
    assert!(no_hosts(&home), "{}", home.ok(&["ls"]));
}

#[test]
fn invite_with_no_name_asks_which_host_and_with_everything_given_asks_nothing() {
    let home = Home::new();
    let folder = a_folder();
    home.ok(&[
        "add",
        "photos",
        folder.path().to_str().unwrap(),
        "--local",
        "--bootstrap",
        BOOTSTRAP,
    ]);
    home.ok(&[
        "add",
        "films",
        "127.0.0.1:8096",
        "--local",
        "--bootstrap",
        BOOTSTRAP,
    ]);

    let mut t = in_terminal(&home, &["invite"]);
    t.expect("Which Host is this Invite for?").unwrap();
    t.expect("photos").unwrap();
    t.send("\x1b").unwrap(); // Esc
    t.expect("No Invite made").unwrap();
    t.expect(expectrl::Eof).unwrap();
    assert!(home.ok(&["invites", "photos"]).is_empty());

    let mut t = in_terminal(&home, &["invite", "photos", "Bob"]);
    let said = t.expect(expectrl::Eof).unwrap();
    let said = String::from_utf8_lossy(said.get(0).unwrap()).into_owned();
    assert!(said.trim().starts_with(&format!("{BOOTSTRAP}#")), "{said}");
    assert_eq!(said.trim().lines().count(), 1, "asked nothing: {said}");
    assert!(home.ok(&["invites", "photos"]).contains("Bob"));
}

#[test]
fn invite_with_nothing_after_it_asks_which_host_who_and_how_then_shows_the_one_line_command() {
    let home = Home::new();
    let folder = a_folder();
    home.ok(&[
        "add",
        "photos",
        folder.path().to_str().unwrap(),
        "--local",
        "--bootstrap",
        BOOTSTRAP,
    ]);
    let _running = home.start("photos");

    let mut t = in_terminal(&home, &["invite"]);
    t.expect("photos").unwrap();
    t.send(ENTER).unwrap();
    t.expect("Who's it for?").unwrap();
    t.send_line("Carol Smith").unwrap();
    t.expect("How long should the link work?").unwrap();
    t.send_line("2h").unwrap();
    t.expect("Approve their device").unwrap();
    t.send("y").unwrap();
    t.expect("Invite for Carol Smith to photos, works for 2 hours")
        .unwrap();
    t.expect(format!("{BOOTSTRAP}#")).unwrap();
    t.expect("Next time, in one line:").unwrap();
    t.expect("tooob invite photos 'Carol Smith' --expires 2h --approve")
        .unwrap();
    t.expect(expectrl::Eof).unwrap();
    assert!(home.ok(&["invites", "photos"]).contains("Carol Smith"));
}

#[test]
fn stop_with_no_name_picks_from_the_running_hosts() {
    let home = Home::new();
    let folder = a_folder();
    home.ok(&[
        "add",
        "photos",
        folder.path().to_str().unwrap(),
        "--local",
        "--bootstrap",
        BOOTSTRAP,
    ]);
    home.ok(&[
        "add",
        "films",
        "127.0.0.1:8096",
        "--local",
        "--bootstrap",
        BOOTSTRAP,
    ]);
    let photos = home.start("photos");

    let mut t = in_terminal(&home, &["stop"]);
    t.expect("Which Host should stop?").unwrap();
    t.expect("photos").unwrap();
    // Listed by name, films would come first, and be the one Enter picks.
    t.send(ENTER).unwrap();
    t.expect(expectrl::Eof).unwrap();
    photos.until_it_says("photos: stopped");
}

#[test]
fn a_host_with_no_bootstrap_says_so_before_asking_anything() {
    let home = Home::new();
    let folder = a_folder();
    std::fs::create_dir(folder.path().join(".git")).unwrap(); // share would ask about it
    let path = folder.path().to_str().unwrap();
    home.ok(&["add", "photos", path, "--local", "--bootstrap", BOOTSTRAP]);
    let file = home.shown("photos", "settings file");
    let settings = std::fs::read_to_string(&file).unwrap();
    std::fs::write(
        &file,
        settings.replace(&format!("bootstrap = \"{BOOTSTRAP}\""), ""),
    )
    .unwrap();

    for args in [
        vec!["invite", "photos"],
        vec!["pair", "photos"],
        vec!["share", path],
    ] {
        let mut t = in_terminal(&home, &args);
        let said = t.expect(expectrl::Eof).unwrap();
        let said = String::from_utf8_lossy(said.get(0).unwrap()).into_owned();
        assert!(
            said.contains("bootstrap = ") && !said.contains('?'),
            "{args:?} asked first: {said}"
        );
    }
}

#[test]
fn pair_with_no_label_asks_which_device_it_is_for() {
    let home = Home::new();
    let folder = a_folder();
    home.ok(&[
        "add",
        "photos",
        folder.path().to_str().unwrap(),
        "--local",
        "--bootstrap",
        BOOTSTRAP,
    ]);

    let mut t = in_terminal(&home, &["pair", "photos"]);
    t.expect("Which device is it for?").unwrap();
    t.send_line("my phone").unwrap();
    t.expect("&invite=").unwrap();
    t.expect(expectrl::Eof).unwrap();
    assert!(home.ok(&["invites", "photos"]).contains("my phone"));

    let out = home.tooob(&["pair", "photos"]);
    assert!(!out.status.success(), "a script is never asked");
}

#[test]
fn an_invite_given_on_the_command_line_is_made_along_with_the_ones_answered() {
    let home = Home::new();
    let folder = a_folder();
    let name = host_name(folder.path());
    let path = folder.path().to_str().unwrap();
    let mut t = in_terminal(
        &home,
        &[
            "share",
            path,
            "--local",
            "--bootstrap",
            BOOTSTRAP,
            "--invite",
            "Dave",
        ],
    );

    t.expect("Who else?").unwrap();
    t.send(ENTER).unwrap();
    t.expect(format!("Invite for Dave to {name}")).unwrap();
    t.expect(format!("{BOOTSTRAP}#")).unwrap();
    assert!(home.ok(&["invites", &name]).contains("Dave"));
    t.send("\x03").unwrap();
    t.expect(expectrl::Eof).unwrap();
}

#[test]
fn backing_out_of_the_questions_adds_no_host() {
    let home = Home::new();
    let folder = a_folder();
    let mut t = in_terminal(
        &home,
        &[
            "share",
            folder.path().to_str().unwrap(),
            "--local",
            "--bootstrap",
            BOOTSTRAP,
        ],
    );

    t.expect("Who's it for?").unwrap();
    t.send("\x03").unwrap(); // Ctrl-C
    t.expect(expectrl::Eof).unwrap();
    assert!(no_hosts(&home), "{}", home.ok(&["ls"]));
}

#[test]
fn closing_the_terminal_while_sharing_stops_the_host() {
    let home = Home::new();
    let folder = a_folder();
    let name = host_name(folder.path());
    let mut t = in_terminal(
        &home,
        &[
            "share",
            folder.path().to_str().unwrap(),
            "--local",
            "--bootstrap",
            BOOTSTRAP,
        ],
    );

    t.expect("Who's it for?").unwrap();
    t.send(ENTER).unwrap();
    t.expect(format!("{name}: serving")).unwrap();
    let pid = t.get_process().pid().as_raw();
    unsafe { libc::kill(pid, libc::SIGHUP) }; // what a closing terminal sends
    t.expect(format!("{name}: stopped")).unwrap();
    assert!(home.row(&name).contains("stopped"));
}
