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
