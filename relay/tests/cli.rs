//! `tooob-relay`'s command line: `enroll`, `remove` and `list` edit the Enrolled Keys' file, and a
//! Relay says at every start whether it's open.
use std::{
    io::{BufRead, BufReader},
    path::Path,
    process::{Command, Output, Stdio},
};

use iroh_base::SecretKey;
use tooob_relay::List;

fn tooob_relay(list: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_tooob-relay"))
        .arg("--enrolled")
        .arg(list)
        .args(args)
        .output()
        .unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn enroll_remove_and_list_edit_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let list = dir.path().join("tooob-relay/enrolled.toml");
    let photos = SecretKey::from_bytes(&rand::random()).public();
    let notes = SecretKey::from_bytes(&rand::random()).public();

    let empty = tooob_relay(&list, &["list"]);
    assert!(empty.status.success());
    assert!(stdout(&empty).contains("open"), "{}", stdout(&empty));

    let photos_key = photos.to_string();
    let notes_key = notes.to_string();
    for (key, label) in [(&photos_key, "photos"), (&notes_key, "notes")] {
        let out = tooob_relay(&list, &["enroll", key, "--label", label]);
        assert!(out.status.success(), "{out:?}");
    }
    let names: Vec<_> = List::load(&list)
        .unwrap()
        .entries()
        .map(|(k, l)| (k, l.to_string()))
        .collect();
    assert_eq!(names.len(), 2);
    assert!(names.contains(&(photos, "photos".to_string())));
    let listed = stdout(&tooob_relay(&list, &["list"]));
    assert!(
        listed.contains(&format!("{photos_key}  photos")),
        "{listed}"
    );

    assert!(
        tooob_relay(&list, &["remove", &photos_key])
            .status
            .success()
    );
    let left: Vec<_> = List::load(&list)
        .unwrap()
        .entries()
        .map(|(k, _)| k)
        .collect();
    assert_eq!(left, vec![notes]);
    assert!(
        !tooob_relay(&list, &["remove", &photos_key])
            .status
            .success()
    );
}

/// The code in what `tooob-relay code` printed: the line's last word.
fn printed_code(output: &Output) -> String {
    stdout(output)
        .split_whitespace()
        .find(|w| w.starts_with("relay.example.com/"))
        .unwrap_or_else(|| panic!("no code in {output:?}"))
        .to_string()
}

#[test]
fn code_prints_one_naming_its_relay_and_codes_lists_it_until_cancelled() {
    let dir = tempfile::tempdir().unwrap();
    let list = dir.path().join("enrolled.toml");
    let made = tooob_relay(
        &list,
        &["--domain", "relay.example.com", "code", "--label", "sam"],
    );
    assert!(made.status.success(), "{made:?}");
    let code = printed_code(&made);
    let secret = code.strip_prefix("relay.example.com/").unwrap();
    // Four groups of four letters and digits: 16 characters of 5 bits, 80 bits.
    let groups: Vec<_> = secret.split('-').collect();
    assert_eq!(groups.len(), 4, "{code}");
    assert!(groups.iter().all(|g| {
        g.len() == 4
            && g.chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
    }));

    let other = printed_code(&tooob_relay(
        &list,
        &[
            "--domain",
            "relay.example.com",
            "code",
            "--label",
            "kim",
            "--uses",
            "3",
            "--expires",
            "2d",
        ],
    ));
    assert_ne!(code, other);

    let listed = stdout(&tooob_relay(&list, &["codes"]));
    let sam = listed.lines().find(|l| l.contains("sam")).expect(&listed);
    assert!(sam.contains("1 use"), "{sam}");
    let kim = listed.lines().find(|l| l.contains("kim")).expect(&listed);
    assert!(kim.contains("3 uses"), "{kim}");

    // The file holds no code in the clear.
    let file = std::fs::read_to_string(&list).unwrap();
    assert!(!file.contains(secret), "{file}");
    assert!(!file.contains(&secret.replace('-', "")), "{file}");

    // Cancelled by label, or by the code itself.
    assert!(tooob_relay(&list, &["cancel", "sam"]).status.success());
    assert!(tooob_relay(&list, &["cancel", &other]).status.success());
    let listed = stdout(&tooob_relay(&list, &["codes"]));
    assert!(
        !listed.contains("sam") && !listed.contains("kim"),
        "{listed}"
    );
    assert!(!tooob_relay(&list, &["cancel", "nobody"]).status.success());
}

#[test]
fn code_needs_the_relays_domain() {
    let dir = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_tooob-relay"))
        .env_remove("RELAY_DOMAIN")
        .arg("--enrolled")
        .arg(dir.path().join("enrolled.toml"))
        .args(["code", "--label", "sam"])
        .output()
        .unwrap();
    assert!(!out.status.success());
}

#[test]
fn a_host_key_as_its_host_logs_it_can_be_enrolled() {
    let dir = tempfile::tempdir().unwrap();
    let list = dir.path().join("enrolled.toml");
    let host = SecretKey::from_bytes(&rand::random()).public();
    let out = tooob_relay(&list, &["enroll", &host.to_z32(), "--label", "photos"]);
    assert!(out.status.success(), "{out:?}");
    let keys: Vec<_> = List::load(&list)
        .unwrap()
        .entries()
        .map(|(k, _)| k)
        .collect();
    assert_eq!(keys, vec![host]);
}

#[test]
fn a_relay_with_no_enrolled_keys_says_at_its_start_that_its_open() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("iroh-relay.toml");
    std::fs::write(
        &config,
        "http_bind_addr = \"127.0.0.1:0\"\nenable_metrics = false\n",
    )
    .unwrap();
    let mut relay = Command::new(env!("CARGO_BIN_EXE_tooob-relay"))
        .arg("--enrolled")
        .arg(dir.path().join("enrolled.toml"))
        .arg("--config-path")
        .arg(&config)
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let said = BufReader::new(relay.stderr.take().unwrap())
        .lines()
        .map_while(Result::ok)
        .find(|line| line.contains("relay_access"));
    relay.kill().unwrap();
    let _ = relay.wait();
    let said = said.expect("no relay_access line");
    assert!(said.contains("relay_access open"), "{said}");
}
