//! The `tooob` command's own words (ADR 0022): help asked for goes to stdout and exits 0; a
//! command or option it doesn't know is one line, with the command to type.
mod support;

use std::process::Output;
use support::home::{Home, a_folder};

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn an_unknown_command_names_the_nearest_one_and_the_command_to_type() {
    let home = Home::new();
    for (args, said) in [
        (
            vec!["la"],
            "tooob: there's no tooob la; did you mean ls?\n  tooob ls\n",
        ),
        (
            vec!["remove", "jm-tooob-share"],
            "tooob: there's no tooob remove; did you mean rm?\n  tooob rm jm-tooob-share\n",
        ),
        (
            vec!["list", "--json"],
            "tooob: there's no tooob list; did you mean ls?\n  tooob ls --json\n",
        ),
        (
            vec!["shwo", "photos"],
            "tooob: there's no tooob shwo; did you mean show?\n  tooob show photos\n",
        ),
        (
            vec!["xyzzy"],
            "tooob: there's no tooob xyzzy; tooob --help lists the commands\n",
        ),
        (
            vec!["help", "lss"],
            "tooob: there's no tooob lss; did you mean ls?\n  tooob help ls\n",
        ),
    ] {
        let out = home.tooob(&args);
        assert!(!out.status.success(), "{args:?}");
        assert_eq!(stderr(&out), said, "{args:?}");
        assert_eq!(stdout(&out), "", "{args:?}");
    }
}

#[test]
fn no_command_or_a_near_tie_is_one_line_pointing_at_help() {
    let home = Home::new();
    for (args, said) in [
        (
            vec![],
            "tooob: say a command; tooob --help lists the commands\n",
        ),
        (
            vec!["--json"],
            "tooob: say a command; tooob --help lists the commands\n",
        ),
        (
            vec!["--version", "--json"],
            "tooob: --version takes no other option:\n  tooob --version\n",
        ),
        // rn is as near run as rm: no guess.
        (
            vec!["rn", "photos"],
            "tooob: there's no tooob rn; tooob --help lists the commands\n",
        ),
    ] {
        let out = home.tooob(&args);
        assert!(!out.status.success(), "{args:?}");
        assert_eq!(stderr(&out), said, "{args:?}");
        assert_eq!(stdout(&out), "", "{args:?}");
    }
}

#[test]
fn an_unknown_option_names_the_nearest_own_option_or_says_where_its_options_are() {
    let home = Home::new();
    let folder = a_folder();
    let path = folder.path().to_str().unwrap();
    for (args, said) in [
        // A short option for another letter would change what was asked.
        (
            vec!["run", "photos", "-f"],
            "tooob: run takes no -f; its options: -d, --detach\n",
        ),
        (
            vec!["share", path, "--json"],
            "tooob: share takes no --json; its options: --invite, --expires, --approve, \
             --no-qr, --name, --relay, --local, --relay-code, --uploads, -d\n",
        ),
    ] {
        let out = home.tooob(&args);
        assert!(!out.status.success(), "{args:?}");
        assert_eq!(stderr(&out), said, "{args:?}");
    }
    assert_eq!(home.ok(&["ls", "--json"]).trim(), "[]");
    for (args, said) in [
        (
            vec!["ls", "--jsn"],
            "tooob: ls takes no --jsn; did you mean --json?\n  tooob ls --json\n",
        ),
        (
            vec!["show", "photos", "--json"],
            "tooob: show takes no --json; tooob help show shows its usage\n",
        ),
        (
            vec!["logs", "photos", "--colour"],
            "tooob: logs takes no --colour; its options: -n, -f, --json\n",
        ),
    ] {
        let out = home.tooob(&args);
        assert!(!out.status.success(), "{args:?}");
        assert_eq!(stderr(&out), said, "{args:?}");
    }
}

const COMMANDS: &[&str] = &[
    "add", "enroll", "run", "share", "ls", "show", "stop", "start", "rm", "logs", "invite", "pair",
    "visitors", "invites", "approve", "block", "unblock", "revoke", "backup", "restore", "upgrade",
];

#[test]
fn help_goes_to_stdout_and_exits_0_for_tooob_and_every_command() {
    let home = Home::new();
    for args in [vec!["--help"], vec!["help"], vec!["-h"]] {
        let out = home.tooob(&args);
        assert!(out.status.success(), "{args:?}");
        let shown = stdout(&out);
        assert!(shown.starts_with("usage:\n"), "{args:?}: {shown}");
        assert!(shown.contains("tooob upgrade"), "{args:?}: {shown}");
    }
    for command in COMMANDS {
        let line = format!("tooob {command} ");
        let shared = format!("| {command} ");
        for args in [
            vec![*command, "--help"],
            vec!["help", command],
            vec![*command, "photos", "-h"],
        ] {
            let out = home.tooob(&args);
            assert!(out.status.success(), "{args:?}: {}", stderr(&out));
            assert_eq!(stderr(&out), "", "{args:?}");
            let shown = stdout(&out);
            assert!(shown.starts_with("usage:\n"), "{args:?}: {shown}");
            assert!(
                shown.contains(&line) || shown.contains(&shared),
                "{args:?}: {shown}"
            );
            assert!(!shown.contains("tooob --version"), "only its own: {shown}");
        }
    }
    let logs = stdout(&home.tooob(&["logs", "--help"]));
    assert!(logs.contains("-f follows it"), "{logs}");
    // NAME there is a new name, not a Host's.
    let share = stdout(&home.tooob(&["share", "--help"]));
    assert!(!share.contains("NAME is a Host Name"), "{share}");
}

#[test]
fn every_command_the_usage_names_has_its_help_checked() {
    let usage = stdout(&Home::new().tooob(&["--help"]));
    for line in usage.lines() {
        let Some(rest) = line.strip_prefix("  tooob ") else {
            continue;
        };
        for part in rest.split(" | ") {
            let name = part.split_whitespace().next().unwrap_or("");
            if name.chars().all(|c| c.is_ascii_lowercase()) && name != "help" {
                assert!(COMMANDS.contains(&name), "{name} isn't in COMMANDS");
            }
        }
    }
}

#[test]
fn share_with_help_only_prints_its_help() {
    let home = Home::new();
    let folder = a_folder();
    let path = folder.path().to_str().unwrap();
    let out = home.tooob(&["share", path, "--invite", "sam", "--help"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stderr(&out), "");
    assert!(stdout(&out).contains("tooob share PATH"));
    let made: Vec<_> = std::fs::read_dir(home.0.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .filter(|n| n != "stand-ins")
        .collect();
    assert!(made.is_empty(), "{made:?}");
    let listed = home.ok(&["ls", "--json"]);
    assert_eq!(listed.trim(), "[]", "{listed}");
}
