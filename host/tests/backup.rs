//! A Backup (ADR 0029): one Host sealed under a passphrase by `tooob backup`, and brought back by
//! `tooob restore` on a fresh box, with its Host Key and its Visitors, Invites and Blocks.
mod support;

use std::path::{Path, PathBuf};
use std::process::Output;
use support::home::{Home, TestRelay, a_folder};
use tooob::Network;
use tooob::host::{Host, HostConfig, Site};
use tooob::visit::Visit;

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// A Visitor let in by an Invite, through the Host run for a moment. Their Visitor Key.
async fn a_visitor(home: &Home, name: &str, folder: &Path, label: &str) -> String {
    let link = home.ok(&["invite", name, label]);
    let token = link.trim().rsplit("#invite=").next().unwrap().to_string();
    let host = Host::start(HostConfig {
        site: Site::SharedFolder(folder.into()),
        data_dir: home.shown(name, "state folder").into(),
        network: Network::LocalOnly,
    })
    .await
    .unwrap();
    let mut key = [0u8; 32];
    getrandom::fill(&mut key).unwrap();
    let visit = Visit::connect(host.address(), key, Network::LocalOnly)
        .await
        .unwrap();
    visit.redeem(&token).await.unwrap();
    host.shutdown().await;
    iroh::SecretKey::from_bytes(&key).public().to_string()
}

/// A file holding `passphrase`, as an Owner would keep it.
fn passphrase_file(dir: &Path, passphrase: &str) -> PathBuf {
    let file = dir.join(format!("pass-{}", passphrase.len()));
    std::fs::write(&file, format!("{passphrase}\n")).unwrap();
    file
}

/// What `tooob visitors` and `tooob invites` say, as JSON, without what's rebuilt after a restore.
fn access(home: &Home, name: &str) -> (serde_json::Value, serde_json::Value) {
    let mut visitors: serde_json::Value =
        serde_json::from_str(&home.ok(&["visitors", name, "--json"])).unwrap();
    // Listed most recently seen first, which is rebuilt too: compared in a fixed order.
    let list = visitors.as_array_mut().unwrap();
    list.sort_by_key(|v| v["key"].as_str().unwrap().to_string());
    for v in list {
        for usage in ["sent", "received", "last_seen"] {
            v.as_object_mut().unwrap().remove(usage);
        }
    }
    let invites = serde_json::from_str(&home.ok(&["invites", name, "--json"])).unwrap();
    (visitors, invites)
}

/// A Host with a Key Address, two Visitors (one blocked) and an unused Invite.
async fn a_host_with_visitors(home: &Home, relay: &TestRelay, folder: &Path) -> String {
    home.ok(&[
        "add",
        "photos",
        folder.to_str().unwrap(),
        "--relay",
        &relay.url,
    ]);
    a_visitor(home, "photos", folder, "alice").await;
    let bob = a_visitor(home, "photos", folder, "bob").await;
    home.ok(&["block", "photos", &bob[..8]]);
    home.ok(&["invite", "photos", "carol"]);
    bob
}

#[tokio::test(flavor = "multi_thread")]
async fn a_host_restored_on_a_fresh_box_has_its_host_key_and_its_visitors_invites_and_blocks() {
    let (old, new) = (Home::new(), Home::new());
    let relay = TestRelay::start();
    let folder = a_folder();
    let bob = a_host_with_visitors(&old, &relay, folder.path()).await;
    let pass = passphrase_file(new.0.path(), "correct horse battery staple");
    let file = new.0.path().join("photos.tooob-backup");
    let (file_arg, pass_arg) = (file.to_str().unwrap(), pass.to_str().unwrap());

    old.ok(&["backup", "photos", file_arg, "--passphrase-file", pass_arg]);
    new.ok(&["restore", "photos", file_arg, "--passphrase-file", pass_arg]);

    for field in ["Host Key", "serves", "relay"] {
        assert_eq!(
            new.shown("photos", field),
            old.shown("photos", field),
            "{field}"
        );
    }
    let (visitors, invites) = access(&new, "photos");
    assert_eq!(
        (&visitors, &invites),
        (&access(&old, "photos").0, &access(&old, "photos").1)
    );
    assert_eq!(visitors.as_array().unwrap().len(), 2, "{visitors}");
    assert!(visitors.to_string().contains(&bob), "{visitors}");
    assert!(visitors.to_string().contains("Blocked"), "{visitors}");
    assert!(invites.to_string().contains("carol"), "{invites}");

    // And it runs as it did, on its Relay.
    let _running = new.start("photos");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_backup_holds_no_plaintext_host_key_or_access_state() {
    let home = Home::new();
    let relay = TestRelay::start();
    let folder = a_folder();
    let bob = a_host_with_visitors(&home, &relay, folder.path()).await;
    let pass = passphrase_file(home.0.path(), "hunter22");
    let file = home.0.path().join("photos.tooob-backup");
    home.ok(&[
        "backup",
        "photos",
        file.to_str().unwrap(),
        "--passphrase-file",
        pass.to_str().unwrap(),
    ]);

    let sealed = std::fs::read(&file).unwrap();
    let state = PathBuf::from(home.shown("photos", "state folder"));
    let key = std::fs::read(state.join("host.key")).unwrap();
    let host_key = home.shown("photos", "Host Key");
    assert!(!sealed.windows(32).any(|w| w == key), "the Host Key, raw");
    let text = String::from_utf8_lossy(&sealed);
    for plain in [
        &host_key[..],
        &bob[..16],
        "alice",
        "carol",
        "visitors",
        "relay",
        "hunter22",
    ] {
        assert!(!text.contains(plain), "{plain} is in the Backup");
    }
    use std::os::unix::fs::MetadataExt;
    assert_eq!(std::fs::metadata(&file).unwrap().mode() & 0o777, 0o600);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_wrong_passphrase_fails_cleanly_and_writes_nothing() {
    let (old, new) = (Home::new(), Home::new());
    let relay = TestRelay::start();
    let folder = a_folder();
    a_host_with_visitors(&old, &relay, folder.path()).await;
    let file = new.0.path().join("photos.tooob-backup");
    let right = passphrase_file(new.0.path(), "the right one");
    let wrong = passphrase_file(new.0.path(), "wrong");
    let file_arg = file.to_str().unwrap();
    old.ok(&[
        "backup",
        "photos",
        file_arg,
        "--passphrase-file",
        right.to_str().unwrap(),
    ]);

    let out = new.tooob(&[
        "restore",
        "photos",
        file_arg,
        "--passphrase-file",
        wrong.to_str().unwrap(),
    ]);
    assert!(!out.status.success());
    assert_eq!(
        stderr(&out),
        format!(
            "tooob: {file_arg} can't be opened with that passphrase (or it isn't a tooob Backup): \
             nothing was restored\n"
        )
    );
    assert!(new.ok(&["ls"]).lines().count() == 1, "{}", new.ok(&["ls"]));
    assert!(!new.0.path().join("state/photos").exists());
    assert!(!new.0.path().join("config/hosts/photos.toml").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_missing_passphrase_a_taken_name_and_a_running_host_each_say_what_to_type() {
    let home = Home::new();
    let relay = TestRelay::start();
    let folder = a_folder();
    home.ok(&[
        "add",
        "photos",
        folder.path().to_str().unwrap(),
        "--relay",
        &relay.url,
    ]);
    let pass = passphrase_file(home.0.path(), "a passphrase");
    let (pass_arg, file) = (pass.to_str().unwrap(), home.0.path().join("b"));
    let file_arg = file.to_str().unwrap();
    let missing = home.0.path().join("nowhere").display().to_string();
    let fails = |args: &[&str]| {
        let out = home.tooob(args);
        assert!(!out.status.success(), "{args:?}");
        stderr(&out)
    };

    assert_eq!(
        fails(&["backup", "photos", file_arg]),
        format!(
            "tooob: say where the passphrase is, a file or - for stdin (it's never asked for):\n  \
             tooob backup photos {file_arg} --passphrase-file PATH\n"
        )
    );
    assert_eq!(
        fails(&["backup", "photos", file_arg, "--passphrase-file", &missing]),
        format!(
            "tooob: can't read the passphrase from {missing} (No such file or directory): say a \
             file that holds it, or - for stdin: `--passphrase-file PATH`\n"
        )
    );
    assert!(!file.exists(), "no Backup without a passphrase");

    home.ok(&["backup", "photos", file_arg, "--passphrase-file", pass_arg]);
    assert_eq!(
        fails(&["backup", "photos", file_arg, "--passphrase-file", pass_arg]),
        format!("tooob: {file_arg} is already there: say another FILE, or delete it first\n")
    );
    assert_eq!(
        fails(&["restore", "photos", file_arg, "--passphrase-file", pass_arg]),
        format!(
            "tooob: there's already a Host named photos: restore it under another name, `tooob \
             restore NAME {file_arg} --passphrase-file PATH`, or `tooob rm photos` first\n"
        )
    );
    let _running = home.start("photos");
    assert_eq!(
        fails(&["restore", "photos", file_arg, "--passphrase-file", pass_arg]),
        format!(
            "tooob: photos is running: restore it under another name, `tooob restore NAME \
             {file_arg} --passphrase-file PATH`, or stop it (`tooob stop photos`) and `tooob rm \
             photos` first\n"
        )
    );
    // Under another name, beside it, it restores: from stdin.
    let mut restore = home.command(&["restore", "photos-2", file_arg, "--passphrase-file", "-"]);
    let mut child = restore
        .stdin(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"a passphrase\n")
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(
        home.shown("photos-2", "Host Key"),
        home.shown("photos", "Host Key")
    );
}
