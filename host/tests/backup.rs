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
async fn a_host_restored_with_serves_runs_its_site_at_the_new_path_with_its_visitors() {
    let (old, new) = (Home::new(), Home::new());
    let relay = TestRelay::start();
    let (folder, moved) = (a_folder(), a_folder());
    a_host_with_visitors(&old, &relay, folder.path()).await;
    let pass = passphrase_file(new.0.path(), "correct horse battery staple");
    let file = new.0.path().join("photos.tooob-backup");
    let (file_arg, pass_arg) = (file.to_str().unwrap(), pass.to_str().unwrap());
    let moved_path = moved.path().canonicalize().unwrap();

    old.ok(&["backup", "photos", file_arg, "--passphrase-file", pass_arg]);
    new.ok(&[
        "restore",
        "photos",
        file_arg,
        "--passphrase-file",
        pass_arg,
        "--serves",
        moved.path().to_str().unwrap(),
    ]);

    assert_eq!(new.shown("photos", "serves"), moved_path.to_str().unwrap());
    assert_eq!(
        new.shown("photos", "Host Key"),
        old.shown("photos", "Host Key")
    );
    assert_eq!(access(&new, "photos"), access(&old, "photos"));
    let settings = std::fs::read_to_string(new.shown("photos", "settings file")).unwrap();
    assert!(
        settings.contains("# The Relay its Visitors reach"),
        "{settings}"
    );

    // It runs, serving the new path: its record moved with it.
    let _running = new.start("photos");
}

/// Host `photos`, added for `folder` and never run (so no record of its Site), backed up: the
/// Backup file and the passphrase file.
fn a_backup_never_run(
    old: &Home,
    new: &Home,
    relay: &TestRelay,
    folder: &Path,
) -> (String, String) {
    old.ok(&[
        "add",
        "photos",
        folder.to_str().unwrap(),
        "--relay",
        &relay.url,
    ]);
    let pass = passphrase_file(new.0.path(), "hunter22");
    let file = new.0.path().join("photos.tooob-backup");
    let (file, pass) = (file.to_str().unwrap(), pass.to_str().unwrap());
    old.ok(&["backup", "photos", file, "--passphrase-file", pass]);
    (file.to_string(), pass.to_string())
}

#[tokio::test(flavor = "multi_thread")]
async fn a_host_that_never_ran_restored_with_serves_records_the_new_site_when_it_first_runs() {
    let (old, new) = (Home::new(), Home::new());
    let relay = TestRelay::start();
    let (folder, moved) = (a_folder(), a_folder());
    let (file, pass) = a_backup_never_run(&old, &new, &relay, folder.path());
    let moved_arg = moved.path().to_str().unwrap();

    new.ok(&[
        "restore",
        "photos",
        &file,
        "--passphrase-file",
        &pass,
        "--serves",
        moved_arg,
    ]);

    let state = PathBuf::from(new.shown("photos", "state folder"));
    assert!(!state.join("host.json").exists());
    new.start("photos").stop();
    let record = std::fs::read_to_string(state.join("host.json")).unwrap();
    let moved_path = moved.path().canonicalize().unwrap();
    assert!(record.contains(moved_path.to_str().unwrap()), "{record}");
}

#[tokio::test(flavor = "multi_thread")]
async fn restore_can_move_a_site_to_a_web_app() {
    let (old, new) = (Home::new(), Home::new());
    let relay = TestRelay::start();
    let folder = a_folder();
    let (file, pass) = a_backup_never_run(&old, &new, &relay, folder.path());

    new.ok(&[
        "restore",
        "photos",
        &file,
        "--passphrase-file",
        &pass,
        "--serves",
        "127.0.0.1:8123",
    ]);

    assert_eq!(new.shown("photos", "serves"), "127.0.0.1:8123");
}

#[tokio::test(flavor = "multi_thread")]
async fn restore_keeps_a_comment_on_the_serves_line_and_refuses_one_it_cant_find() {
    let (old, new) = (Home::new(), Home::new());
    let relay = TestRelay::start();
    let (folder, moved) = (a_folder(), a_folder());
    old.ok(&[
        "add",
        "photos",
        folder.path().to_str().unwrap(),
        "--relay",
        &relay.url,
    ]);
    let settings_file = old.shown("photos", "settings file");
    let settings = std::fs::read_to_string(&settings_file).unwrap();
    let pass = passphrase_file(new.0.path(), "hunter22");
    let pass = pass.to_str().unwrap();
    let backup = |text: &str, file: &str| {
        std::fs::write(&settings_file, text).unwrap();
        let file = new.0.path().join(file);
        old.ok(&[
            "backup",
            "photos",
            file.to_str().unwrap(),
            "--passphrase-file",
            pass,
        ]);
        file.to_str().unwrap().to_string()
    };
    let commented = backup(
        &settings.replace("serves = ", "serves= ").replace(
            &format!("{:?}\n", old.shown("photos", "serves")),
            &format!("{:?} # my #1 folder\n", old.shown("photos", "serves")),
        ),
        "commented",
    );
    let quoted = backup(&settings.replace("serves = ", "\"serves\" = "), "quoted");
    let moved_arg = moved.path().to_str().unwrap();

    let out = new.tooob(&[
        "restore",
        "photos",
        &quoted,
        "--passphrase-file",
        pass,
        "--serves",
        moved_arg,
    ]);
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("nothing was restored"),
        "{}",
        stderr(&out)
    );
    assert!(!new.tooob(&["show", "photos"]).status.success());

    new.ok(&[
        "restore",
        "photos",
        &commented,
        "--passphrase-file",
        pass,
        "--serves",
        moved_arg,
    ]);
    let restored = std::fs::read_to_string(new.shown("photos", "settings file")).unwrap();
    let moved_path = moved.path().canonicalize().unwrap();
    assert!(
        restored.contains(&format!(
            "serves = {:?} # my #1 folder\n",
            moved_path.to_str().unwrap()
        )),
        "{restored}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn restore_refuses_a_serves_that_add_would_refuse() {
    let (old, new) = (Home::new(), Home::new());
    let relay = TestRelay::start();
    let folder = a_folder();
    old.ok(&[
        "add",
        "photos",
        folder.path().to_str().unwrap(),
        "--relay",
        &relay.url,
    ]);
    let pass = passphrase_file(new.0.path(), "hunter22");
    let file = new.0.path().join("photos.tooob-backup");
    let (file_arg, pass_arg) = (file.to_str().unwrap(), pass.to_str().unwrap());
    old.ok(&["backup", "photos", file_arg, "--passphrase-file", pass_arg]);

    let gone = new.0.path().join("no-such-folder");
    let out = new.tooob(&[
        "restore",
        "photos",
        file_arg,
        "--passphrase-file",
        pass_arg,
        "--serves",
        gone.to_str().unwrap(),
    ]);
    assert!(!out.status.success());
    let added = new.tooob(&[
        "add",
        "other",
        gone.to_str().unwrap(),
        "--relay",
        &relay.url,
    ]);
    assert_eq!(stderr(&out), stderr(&added));
    assert!(
        !new.tooob(&["show", "photos"]).status.success(),
        "nothing was restored"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_restored_host_whose_serves_was_hand_edited_wont_run_and_says_how() {
    let (old, new) = (Home::new(), Home::new());
    let relay = TestRelay::start();
    let (folder, moved) = (a_folder(), a_folder());
    a_host_with_visitors(&old, &relay, folder.path()).await;
    let pass = passphrase_file(new.0.path(), "hunter22");
    let file = new.0.path().join("photos.tooob-backup");
    let (file_arg, pass_arg) = (file.to_str().unwrap(), pass.to_str().unwrap());
    old.ok(&["backup", "photos", file_arg, "--passphrase-file", pass_arg]);
    new.ok(&["restore", "photos", file_arg, "--passphrase-file", pass_arg]);
    let settings_file = new.shown("photos", "settings file");
    let was = new.shown("photos", "serves");
    let settings = std::fs::read_to_string(&settings_file).unwrap();
    let now = moved.path().canonicalize().unwrap();
    std::fs::write(
        &settings_file,
        settings.replace(&was, now.to_str().unwrap()),
    )
    .unwrap();

    let out = new.tooob(&["run", "photos"]);
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains(&format!(
            "share {now} as a new Host; or, if it was restored from a Backup, `tooob rm \
             photos`, then `tooob restore photos FILE --passphrase-file PATH --serves {now}`",
            now = now.display()
        )),
        "{}",
        stderr(&out)
    );
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
    // Again over it: replaced whole, nothing left beside it.
    let first = std::fs::read(&file).unwrap();
    home.ok(&["backup", "photos", file_arg, "--passphrase-file", pass_arg]);
    assert_ne!(
        std::fs::read(&file).unwrap(),
        first,
        "a new Backup, new salt"
    );
    let beside: Vec<_> = std::fs::read_dir(home.0.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|n| n.contains(".tmp"))
        .collect();
    assert!(beside.is_empty(), "{beside:?}");
    // A failed write leaves the old one as it was.
    let kept = std::fs::read(&file).unwrap();
    let nowhere = home.0.path().join("no-such-folder").join("b");
    fails(&[
        "backup",
        "photos",
        nowhere.to_str().unwrap(),
        "--passphrase-file",
        pass_arg,
    ]);
    assert_eq!(std::fs::read(&file).unwrap(), kept);
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
    // Under another name, beside it, it won't: two Hosts can't share a Host Key.
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
    assert!(!out.status.success());
    assert_eq!(
        stderr(&out),
        format!(
            "tooob: photos here already has this Backup's Host Key, and two Hosts can't share \
             one: `tooob rm photos` first, then `tooob restore photos-2 {file_arg} \
             --passphrase-file PATH`\n"
        )
    );
    assert!(!home.0.path().join("config/hosts/photos-2.toml").exists());
    assert!(!home.0.path().join("state/photos-2").exists());
}
