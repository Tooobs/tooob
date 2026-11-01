//! The `tooob` command with a person at the keyboard, in a real (pseudo) terminal: it asks nothing
//! there either (ADR 0022). Each command finishes, serves, or fails with what's missing, and none
//! waits for input, except `tooob upgrade`, which asks before it installs a Release (ADR 0024).
//! Everything else is in tests/hosts.rs and tests/upgrade.rs, run as a script would.
mod support;

use expectrl::{Expect, Session, process::unix::WaitStatus, session::OsSession};
use std::{process::Command, time::Duration};
use support::github::TestGitHub;
use support::home::{Home, RELAY, TestRelay, a_folder, name_of};

/// `program ARGS` in a terminal of its own, under the test's TOOOB_HOME. A `tooob` it runs finds
/// only stand-ins for launchctl and systemctl, never the Owner's own: on `PATH` ahead of the rest,
/// which a shell needs.
fn spawn_in_terminal(home: &Home, program: &str, args: &[&str]) -> OsSession {
    let path = format!("{}:/usr/bin:/bin", home.stand_ins().display());
    spawn_in_terminal_with(home, program, args, &[("PATH", &path)])
}

/// `program ARGS` in a terminal of its own, with `env` set too.
fn spawn_in_terminal_with(
    home: &Home,
    program: &str,
    args: &[&str],
    env: &[(&str, &str)],
) -> OsSession {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .env("TOOOB_HOME", home.0.path())
        .env("HOME", home.0.path())
        .env("TERM", "xterm-256color")
        .env_remove("TOOOB_AUTOSTART")
        .envs(home.releases_env())
        .envs(env.iter().copied());
    let mut session = Session::spawn(cmd).unwrap();
    session.set_expect_timeout(Some(Duration::from_secs(30)));
    session
}

/// `tooob ARGS` in a terminal of its own, with only stand-ins for launchctl and systemctl on its
/// `PATH`.
fn in_terminal(home: &Home, args: &[&str]) -> OsSession {
    let path = home.stand_ins();
    let path = path.to_str().unwrap();
    spawn_in_terminal_with(home, env!("CARGO_BIN_EXE_tooob"), args, &[("PATH", path)])
}

/// What a terminal showed, as lines: its `\r\n`s as `\n`.
fn shown(said: &[u8]) -> String {
    String::from_utf8_lossy(said).replace("\r\n", "\n")
}

/// The hint a Host run in a terminal shows when it first reads keys.
const KEYS: &str = "(q) quit · (d) detach: keep it running in the background";

/// The terminal `session` runs in, as its programs see it: to look at how it's set, and at what's
/// typed there and not read yet.
fn tty_of(session: &OsSession) -> std::fs::File {
    use std::os::fd::AsRawFd;
    // SAFETY: ptsname reads the pty's name into libc's own buffer, copied out at once.
    let name = unsafe { std::ffi::CStr::from_ptr(libc::ptsname(session.as_raw_fd())) };
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOCTTY | libc::O_NONBLOCK)
        .open(name.to_str().unwrap())
        .unwrap()
}

use std::os::unix::fs::OpenOptionsExt;

/// Wait, up to 10 s, until `check` is true.
fn until(what: &str, check: impl Fn() -> bool) {
    let started = std::time::Instant::now();
    while !check() {
        assert!(started.elapsed() < Duration::from_secs(10), "never {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Wait until the terminal takes keys singly, without echoing them: a Host is reading keys.
fn until_keys_are_read(tty: &std::fs::File) {
    use std::os::fd::AsRawFd;
    until("read keys singly without echo", || {
        // SAFETY: a termios is plain data, filled in by tcgetattr, which writes only `now`.
        let now = unsafe {
            let mut now: libc::termios = std::mem::zeroed();
            libc::tcgetattr(tty.as_raw_fd(), &mut now);
            now
        };
        now.c_lflag & (libc::ICANON | libc::ECHO) == 0
    });
}

/// Wait until everything typed in the terminal has been read.
fn until_read(tty: &std::fs::File) {
    use std::os::fd::AsRawFd;
    until("read what was typed", || {
        let mut waiting: libc::c_int = 0;
        // SAFETY: FIONREAD writes how many bytes wait to be read into `waiting`, ours.
        unsafe { libc::ioctl(tty.as_raw_fd(), libc::FIONREAD, &mut waiting) };
        waiting == 0
    });
}

/// An interactive bash in a terminal of its own, its prompt `$ `.
fn bash(home: &Home) -> OsSession {
    let mut sh = spawn_in_terminal(home, "/bin/bash", &["--norc", "--noprofile", "-i"]);
    sh.send_line("PS1='$ '; stty echo").unwrap();
    sh.expect("$ ").unwrap();
    sh
}

/// `tooob ARGS` as a line for `bash`, with only stand-ins for launchctl and systemctl on its
/// `PATH`, never the Owner's own.
fn tooob_line(home: &Home, args: &str) -> String {
    format!(
        "PATH='{}' '{}' {args}",
        home.stand_ins().display(),
        env!("CARGO_BIN_EXE_tooob")
    )
}

/// What `stty -a` says, in the shell `sh`, of the terminal's line editing and echo: as a shell
/// leaves them, `icanon` and `echo`, or `-icanon` and `-echo` while a program reads single keys.
fn line_editing(sh: &mut OsSession) -> (String, String) {
    sh.send_line("stty -a | tr ' ' '\\n' | grep -E '^-?(icanon|echo)$' | tr '\\n' ' '; echo EN''D")
        .unwrap();
    let said = sh.expect("END\r\n").unwrap();
    let said = shown(said.before());
    sh.expect("$ ").unwrap(); // its prompt, so what's expected next comes after it
    let said = said.lines().last().unwrap_or_default().to_string();
    let words: Vec<&str> = said.split_whitespace().collect();
    let find = |what: &str| {
        words
            .iter()
            .find(|w| w.trim_start_matches('-') == what)
            .map_or(String::new(), |w| w.to_string())
    };
    (find("icanon"), find("echo"))
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
fn share_with_everything_given_asks_nothing_prints_its_invites_and_the_keys_and_serves_until_ctrl_c()
 {
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
        "--invite",
        "Bob",
        "--expires",
        "1d",
    ];
    let mut t = in_terminal(&home, &args);

    let said = t.expect(KEYS).unwrap();
    let said = shown(said.before());
    assert!(said.contains(&format!("{name}: serving")), "{said}");
    assert!(!said.contains('?'), "asked something: {said}");
    let block = &said[said.find("\n\n  Invites").expect(&said)..];
    let lines: Vec<&str> = block.split('\n').collect();
    let link = |line: &str| {
        line.starts_with("    http://")
            && line.contains(".localhost:")
            && line.contains("/#invite=")
    };
    assert_eq!(lines.len(), 11, "{block:?}");
    assert_eq!(
        lines[..4],
        ["", "", "  Invites · each opens in one browser", ""]
    );
    assert_eq!(lines[4], "    Alice · works for 1 day", "{block:?}");
    assert!(link(lines[5]), "{block:?}");
    assert_eq!(lines[6..8], ["", "    Bob · works for 1 day"], "{block:?}");
    assert!(link(lines[8]), "{block:?}");
    assert_eq!(lines[9..], ["", ""], "{block:?}");
    assert!(home.row(&name).contains("running"));
    assert!(home.ok(&["invites", &name]).contains("Alice"));

    t.send("\x03").unwrap(); // Ctrl-C
    t.expect(format!("{name}: stopped")).unwrap();
    t.expect(expectrl::Eof).unwrap();
    assert!(home.row(&name).contains("stopped"));
}

#[test]
fn q_stops_a_host_run_in_the_terminal_cleanly_and_gives_the_terminal_back_as_it_was() {
    let home = Home::new();
    let folder = a_folder();
    home.ok(&["add", "photos", folder.path().to_str().unwrap(), "--local"]);
    let mut sh = bash(&home);
    let before = line_editing(&mut sh);
    assert_eq!(before, ("icanon".into(), "echo".into()));

    sh.send_line(tooob_line(&home, "run photos")).unwrap();
    let said = sh.expect(KEYS).unwrap();
    assert!(shown(said.before()).contains("photos: serving"));
    let tty = tty_of(&sh);
    until_keys_are_read(&tty);
    sh.send("x").unwrap(); // not a key it reads: nothing happens
    until_read(&tty);
    assert!(home.row("photos").contains("running"));
    sh.send("q").unwrap();
    let said = sh.expect("$ ").unwrap();
    let said = shown(said.before());
    assert!(said.contains("photos: stopped"), "{said}");
    assert!(!said.contains('x'), "x was echoed: {said}");
    assert!(home.row("photos").contains("stopped"));
    sh.send_line("echo status $?").unwrap();
    sh.expect("status 0").unwrap();
    assert_eq!(line_editing(&mut sh), before);
    sh.send_line("exit").unwrap();
    sh.expect(expectrl::Eof).unwrap();
}

#[test]
fn a_host_suspended_with_ctrl_z_or_stopped_with_ctrl_c_leaves_the_terminal_as_it_was() {
    let home = Home::new();
    let folder = a_folder();
    home.ok(&["add", "photos", folder.path().to_str().unwrap(), "--local"]);
    let mut sh = bash(&home);
    let before = line_editing(&mut sh);
    sh.send_line(tooob_line(&home, "run photos")).unwrap();
    sh.expect(KEYS).unwrap();

    sh.send("\x1a").unwrap(); // Ctrl-Z
    sh.expect("Stopped").unwrap();
    sh.expect("$ ").unwrap();
    assert_eq!(line_editing(&mut sh), before, "suspended");
    sh.send_line("fg").unwrap();
    let tty = tty_of(&sh);
    until_keys_are_read(&tty); // again
    sh.send("x").unwrap(); // read as a key again, not echoed
    until_read(&tty);
    sh.send("\x03").unwrap(); // Ctrl-C
    let said = shown(sh.expect("$ ").unwrap().before());
    assert!(said.contains("photos: stopped"), "{said}");
    assert!(!said.contains('x'), "x was echoed: {said}");
    assert_eq!(line_editing(&mut sh), before, "stopped");
    assert!(home.row("photos").contains("stopped"));
    sh.send_line("exit").unwrap();
    sh.expect(expectrl::Eof).unwrap();
}

#[test]
fn share_started_with_an_ampersand_prints_the_link_reads_no_keys_and_keeps_serving_in_the_background()
 {
    let home = Home::new();
    let folder = a_folder();
    let name = host_name(folder.path());
    let relay = TestRelay::start();
    let mut sh = bash(&home);

    let share = format!(
        "share '{}' --relay {} --invite Alice &",
        folder.path().display(),
        relay.url
    );
    sh.send_line(tooob_line(&home, &share)).unwrap();
    sh.expect("/#invite=").unwrap();
    // A job that read the terminal from the background would be stopped by the shell: given a
    // moment to, it's still running. (Nothing to wait on for what doesn't happen.)
    sh.send_line("sleep 1; jobs -l").unwrap();
    let said = sh.expect("Running").unwrap();
    assert!(home.row(&name).contains("running"));
    assert!(home.ok(&["invites", &name]).contains("Alice"));
    // The shell's to read: q is a command to it, and d too.
    sh.send_line("q; d; echo after-keys; jobs -l").unwrap();
    let said = [
        shown(said.before()),
        shown(sh.expect("Running").unwrap().before()),
    ]
    .concat();
    assert!(said.contains("after-keys"), "{said}");
    assert!(!said.contains(KEYS), "no keys to show: {said}");
    assert!(home.row(&name).contains("running"));
    let (icanon, echo) = line_editing(&mut sh);
    assert_eq!((icanon.as_str(), echo.as_str()), ("icanon", "echo"));

    // Brought to the foreground, it reads them, showing which: q stops it.
    sh.send_line("fg").unwrap();
    sh.expect(KEYS).unwrap();
    until_keys_are_read(&tty_of(&sh));
    sh.send("q").unwrap();
    sh.expect(format!("{name}: stopped")).unwrap();
    sh.expect("$ ").unwrap();
    assert!(home.row(&name).contains("stopped"));
    assert_eq!(line_editing(&mut sh), ("icanon".into(), "echo".into()));
    sh.send_line("exit").unwrap();
    sh.expect(expectrl::Eof).unwrap();
}

/// Host photos, sharing a folder on this network only, with tooob where the install script puts
/// it, as keeping it running needs.
fn photos(home: &Home) -> tempfile::TempDir {
    let folder = a_folder();
    home.ok(&["add", "photos", folder.path().to_str().unwrap(), "--local"]);
    home.install_tooob();
    folder
}

/// Where photos' Autostart entry is, for launchd.
fn launch_agent(home: &Home) -> std::path::PathBuf {
    home.0
        .path()
        .join("Library/LaunchAgents/net.tooobs.photos.plist")
}

#[test]
fn d_hands_a_host_run_in_the_terminal_to_its_service_manager_says_so_in_a_line_and_gives_the_terminal_back()
 {
    let home = Home::new();
    let _folder = photos(&home);
    // What `tooob ls` says of photos as launchd is asked to start it: stopped here first.
    let at_bootstrap = home.0.path().join("ls-at-bootstrap");
    home.stand_in(
        "launchctl",
        &format!(
            "'{}' ls > '{}'",
            env!("CARGO_BIN_EXE_tooob"),
            at_bootstrap.display()
        ),
    );
    let mut sh = bash(&home);
    sh.send_line(tooob_line(&home, "run photos")).unwrap();
    sh.expect(KEYS).unwrap();

    until_keys_are_read(&tty_of(&sh));
    sh.send("d").unwrap();
    let said = shown(sh.expect("$ ").unwrap().before());
    let said: Vec<&str> = said.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(said.len(), 1, "one line: {said:#?}");
    assert!(
        said[0].ends_with(
            "  photos: detached: launchd keeps it running now, and after logins, reboots and \
             crashes: `tooob logs photos -f` follows it, and `tooob stop photos` stops it"
        ),
        "{said:#?}"
    );
    sh.send_line("echo status $?").unwrap();
    sh.expect("status 0").unwrap();
    assert_eq!(line_editing(&mut sh), ("icanon".into(), "echo".into()));
    let uid = unsafe { libc::getuid() };
    let agent = launch_agent(&home);
    assert_eq!(
        home.calls("launchctl"),
        [format!("bootstrap gui/{uid} {}", agent.display())]
    );
    assert!(agent.exists());
    let ls = std::fs::read_to_string(&at_bootstrap).unwrap();
    let row = ls.lines().find(|l| l.starts_with("photos ")).expect(&ls);
    assert_eq!(row.split_whitespace().nth(1), Some("stopped"), "{ls}");
    sh.send_line("exit").unwrap();
    sh.expect(expectrl::Eof).unwrap();
}

#[test]
fn d_where_the_host_cant_be_kept_running_says_what_to_do_and_keeps_serving() {
    let home = Home::new();
    let folder = a_folder();
    home.ok(&["add", "photos", folder.path().to_str().unwrap(), "--local"]);
    let tooob = home.0.path().join(".local/bin/tooob");
    for (stand_in, says) in [
        (
            None,
            "tooob: keeping photos running needs systemd or launchd, and this system has \
             neither.\n  Have your service manager run `~/.local/bin/tooob run photos` as you, \
             restarting it on failure.\n"
                .to_string(),
        ),
        (
            Some("launchctl"),
            format!(
                "tooob: photos would start from {}, where the install script puts tooob, and it \
                 isn't there: install tooob, or put this one there:\n",
                tooob.display()
            ),
        ),
    ] {
        if let Some(tool) = stand_in {
            home.stand_in(tool, "");
        }
        let mut t = in_terminal(&home, &["run", "photos"]);
        t.expect(KEYS).unwrap();
        until_keys_are_read(&tty_of(&t));
        t.send("d").unwrap();
        let said = t.expect(says.replace('\n', "\r\n").as_str()).unwrap();
        let said = shown(said.before());
        assert!(!said.contains("stopped"), "{said}");
        assert!(home.row("photos").contains("running"), "{says}");
        assert!(home.calls("launchctl").is_empty());
        assert!(!launch_agent(&home).exists());

        t.send("q").unwrap();
        t.expect("photos: stopped").unwrap();
        t.expect(expectrl::Eof).unwrap();
        assert!(matches!(
            t.get_process().wait().unwrap(),
            WaitStatus::Exited(_, 0)
        ));
    }
}

#[test]
fn d_whose_service_manager_then_refuses_serves_here_again_saying_why() {
    let home = Home::new();
    let _folder = photos(&home);
    home.stand_in(
        "launchctl",
        "case \"$*\" in *bootstrap*) echo 'no session' >&2; exit 5;; esac",
    );
    let mut t = in_terminal(&home, &["run", "photos"]);
    t.expect(KEYS).unwrap();
    until_keys_are_read(&tty_of(&t));

    t.send("d").unwrap();
    let said = shown(t.expect(KEYS).unwrap().before());
    assert!(!said.contains("stopped"), "{said}");
    let lines: Vec<&str> = said.lines().filter(|l| !l.is_empty()).collect();
    assert!(
        lines[0].contains("photos: warning: it wasn't kept running ("),
        "{said}"
    );
    assert!(
        lines[0].contains("no session") && lines[0].ends_with("), so it serves here again"),
        "{said}"
    );
    assert!(said.contains("photos: serving"), "{said}");
    assert!(home.row("photos").contains("running"));
    assert!(!launch_agent(&home).exists(), "nothing left behind");
    assert!(
        home.row("photos").contains(" no "),
        "{}",
        home.row("photos")
    );

    t.send("q").unwrap();
    t.expect("photos: stopped").unwrap();
    t.expect(expectrl::Eof).unwrap();
    assert!(matches!(
        t.get_process().wait().unwrap(),
        WaitStatus::Exited(_, 0)
    ));
}

#[test]
fn only_q_or_d_pressed_on_its_own_is_a_key_not_a_capital_an_arrow_key_or_pasted_text() {
    let home = Home::new();
    let folder = a_folder();
    home.ok(&["add", "photos", folder.path().to_str().unwrap(), "--local"]);
    // With no service manager, d says so, and serves on: a sign it was read as d.
    let d_said = "tooob: keeping photos running needs systemd or launchd";
    let mut t = in_terminal(&home, &["run", "photos"]);
    t.expect(KEYS).unwrap();
    let tty = tty_of(&t);
    until_keys_are_read(&tty);

    for typed in ["D", "Q", "dq", "\x1b[D"] {
        t.send(typed).unwrap(); // the left arrow is ESC [ D
        until_read(&tty);
    }
    assert!(home.row("photos").contains("running"));
    // d on its own: read as d, once it's not taken for the end of the arrow key.
    t.set_expect_timeout(Some(Duration::from_secs(1)));
    let said = (0..10)
        .find_map(|_| {
            t.send("d").unwrap();
            until_read(&tty);
            t.expect(d_said).ok()
        })
        .expect("d on its own was never read");
    let said = shown(said.before());
    assert!(
        !said.contains(d_said) && !said.contains("stopped"),
        "{said}"
    );
    t.set_expect_timeout(Some(Duration::from_secs(30)));

    t.send("q").unwrap();
    let said = shown(t.expect("photos: stopped").unwrap().before());
    assert!(!said.contains(d_said), "only the one d: {said}");
    t.expect(expectrl::Eof).unwrap();
}

#[test]
fn every_command_missing_something_fails_at_once_with_the_command_to_type() {
    let home = Home::new();
    let (photos, other) = (a_folder(), a_folder());
    home.add_naming("photos", photos.path().to_str().unwrap(), RELAY);
    home.ok(&["add", "films", "127.0.0.1:8096", "--local"]);
    let other = other.path().to_str().unwrap();

    for (args, says) in [
        (vec!["share"], "tooob share PATH"),
        (vec!["share", other], "--relay https://RELAY/"),
        (vec!["add", "other", other], "--relay https://RELAY/"),
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

#[test]
fn upgrade_asks_before_it_installs_and_only_y_installs() {
    let this = env!("CARGO_PKG_VERSION");
    let mut parts = this.split('.');
    let major = parts.next().unwrap();
    let minor: u32 = parts.next().unwrap().parse().unwrap();
    let newer = format!("{major}.{}.0", minor + 1);
    for (answer, installs) in [("y", true), ("Y", true), ("n", false), ("", false)] {
        let home = Home::new();
        let github = TestGitHub::start();
        github.publish(this, Some(tooob::PROTOCOL_VERSION));
        github.publish(&newer, Some(tooob::PROTOCOL_VERSION));
        home.releases_at(&github.url);
        home.install_tooob();
        let mut t = in_terminal(&home, &["upgrade"]);

        let said = t.expect(format!("Install {newer}? [y/N] ")).unwrap();
        let said = String::from_utf8_lossy(said.before()).into_owned();
        assert!(
            said.contains(&format!(
                "tooob {newer} is out (this is {this}): \
                 https://github.com/Tooobs/tooob/releases/tag/v{newer}"
            )),
            "{said}"
        );
        t.send_line(answer).unwrap();
        let said = t.expect(expectrl::Eof).unwrap();
        let said = String::from_utf8_lossy(said.get(0).unwrap()).into_owned();
        assert!(
            matches!(t.get_process().wait().unwrap(), WaitStatus::Exited(_, 0)),
            "{answer:?}: {said}"
        );
        let installed =
            std::fs::read_to_string(home.0.path().join("installed")).unwrap_or_default();
        match installs {
            true => {
                assert_eq!(installed.trim(), newer, "{answer:?}");
                assert!(
                    said.contains(&format!("tooob {newer} is installed")),
                    "{said}"
                );
            }
            false => {
                assert_eq!(installed, "", "{answer:?}");
                assert!(said.contains("nothing installed"), "{answer:?}: {said}");
            }
        }
    }
}
