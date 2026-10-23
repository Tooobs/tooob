//! The Owner's Hosts on one box, through the `tooob` command run as a script would (no terminal):
//! each made once (`add`) and run in the foreground (`run`), one process per Host (ADR 0014).
mod support;

use std::process::Output;
use support::home::{BOOTSTRAP, Home, a_folder};
use tooob::Network;
use tooob::host::{Host, HostConfig, Site};
use tooob::visit::Visit;

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn a_host_is_added_then_run_and_a_second_run_of_it_is_refused_with_the_pid() {
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
    let _films = home.start("films");
    for name in ["photos", "films"] {
        assert!(home.row(name).contains("running"), "{}", home.row(name));
    }

    let again = home.tooob(&["run", "photos"]);
    assert!(!again.status.success());
    let said = stderr(&again);
    assert!(
        said.contains("already running") && said.contains(&photos.child.id().to_string()),
        "{said}"
    );
}

#[test]
fn a_host_run_again_has_the_same_host_key_and_its_invites() {
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
    let key = home.shown("photos", "Host Key");
    assert!(key.len() > 50, "made by add: {key}");

    let running = home.start("photos");
    home.ok(&["invite", "photos", "Alice"]);
    running.stop();
    assert!(
        home.row("photos").contains("stopped"),
        "{}",
        home.row("photos")
    );

    let again = home.start("photos");
    assert_eq!(home.shown("photos", "Host Key"), key);
    assert!(again.log().contains(&key), "{}", again.log());
    let invites = home.ok(&["invites", "photos"]);
    assert!(invites.contains("Alice"), "{invites}");
}

#[test]
fn add_needs_a_relay_and_a_bootstrap_and_links_start_with_that_bootstrap() {
    let home = Home::new();
    let folder = a_folder();
    let path = folder.path().to_str().unwrap();
    let neither = stderr(&home.tooob(&["add", "photos", path]));
    assert!(
        neither.contains("--relay") && neither.contains("--bootstrap"),
        "{neither}"
    );
    assert!(neither.contains("usage:"), "a usage error: {neither}");
    let query = home.tooob(&[
        "add",
        "photos",
        path,
        "--local",
        "--bootstrap",
        "https://b.example/?x=1",
    ]);
    assert!(
        !query.status.success(),
        "Invite Links add their own ?relay="
    );
    for (args, missing) in [
        (vec!["add", "photos", path], "--relay"),
        (
            vec!["add", "photos", path, "--bootstrap", "https://b.example/"],
            "--relay",
        ),
        (vec!["add", "photos", path, "--local"], "--bootstrap"),
    ] {
        let out = home.tooob(&args);
        assert!(!out.status.success(), "{args:?} guessed");
        let err = stderr(&out);
        assert!(
            err.contains(missing) && err.contains("deploy/"),
            "{args:?}: {err}"
        );
    }
    assert_eq!(home.ok(&["ls"]).lines().count(), 1, "nothing was added");

    home.ok(&[
        "add",
        "photos",
        path,
        "--local",
        "--bootstrap",
        "https://b.example/",
    ]);
    assert_eq!(home.shown("photos", "bootstrap"), "https://b.example/");
    let link = home.ok(&["invite", "photos", "alice"]);
    assert!(link.starts_with("https://b.example/#"), "{link}");
}

#[test]
fn share_without_a_terminal_says_what_a_new_host_needs_and_a_reused_one_keeps_its_own() {
    let home = Home::new();
    let folder = a_folder();
    let path = folder.path().to_str().unwrap();
    let out = home.tooob(&["share", path, "--local"]);
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("--bootstrap") && stderr(&out).contains("deploy/"),
        "{}",
        stderr(&out)
    );

    home.ok(&["add", "photos", path, "--local", "--bootstrap", BOOTSTRAP]);
    let again = home.spawn(&[
        "share",
        path,
        "--relay",
        "nonsense",
        "--bootstrap",
        "nonsense",
        "--invite",
        "Ann",
    ]);
    again.until_it_says(&format!("{BOOTSTRAP}#"));
    assert_eq!(home.shown("photos", "relay"), "none");
}

#[test]
fn editing_the_bootstrap_in_the_settings_file_changes_the_next_link_running_or_not() {
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
    let file = home.shown("photos", "settings file");
    let settings = std::fs::read_to_string(&file).unwrap();
    std::fs::write(
        &file,
        settings.replace(BOOTSTRAP, "https://other.example/app/"),
    )
    .unwrap();

    let stopped = home.ok(&["invite", "photos", "alice"]);
    assert!(
        stopped.starts_with("https://other.example/app/#"),
        "{stopped}"
    );
    let _running = home.start("photos");
    let running = home.ok(&["pair", "photos", "my phone"]);
    assert!(
        running.starts_with("https://other.example/app/#"),
        "{running}"
    );
}

#[test]
fn a_settings_file_with_no_bootstrap_says_what_line_to_add_and_makes_no_invite() {
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
    let file = home.shown("photos", "settings file");
    let settings = std::fs::read_to_string(&file).unwrap();
    let without: String = settings
        .lines()
        .filter(|l| !l.starts_with("bootstrap"))
        .map(|l| format!("{l}\n"))
        .collect();
    std::fs::write(&file, without).unwrap();

    assert!(home.shown("photos", "bootstrap").starts_with("not set"));
    let out = home.tooob(&["invite", "photos", "alice"]);
    assert!(!out.status.success());
    let err = stderr(&out);
    assert!(err.contains(&file) && err.contains("bootstrap = "), "{err}");
    assert!(
        home.ok(&["invites", "photos"]).is_empty(),
        "no Invite without a link"
    );
    let _running = home.start("photos"); // runs, just can't make links
}

#[test]
fn an_invite_made_while_the_host_is_stopped_is_there_once_it_runs() {
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
    let key = home.shown("photos", "Host Key");

    let link = home.ok(&["invite", "photos", "alice"]);
    assert_eq!(link.lines().count(), 1, "{link}");
    assert!(
        link.starts_with(&format!("{BOOTSTRAP}#{key}&invite=")),
        "{link}"
    );
    assert!(home.ok(&["invites", "photos"]).contains("alice"));

    let _running = home.start("photos");
    let listed = home.ok(&["invites", "photos"]);
    assert!(
        listed.contains("alice"),
        "the running Host has it: {listed}"
    );
}

/// A Visitor of the stopped Host `name`, which shares `folder`: an Invite made by `tooob invite`,
/// used while the Host runs for a moment. Their Visitor Key.
async fn a_visitor(home: &Home, name: &str, folder: &std::path::Path) -> String {
    let link = home.ok(&["invite", name, "alice"]);
    let token = link.trim().rsplit("&invite=").next().unwrap().to_string();
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

#[tokio::test(flavor = "multi_thread")]
async fn a_block_made_while_the_host_is_stopped_holds_once_it_runs() {
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
    let alice = a_visitor(&home, "photos", folder.path()).await;

    let blocked = home.tooob(&["block", "photos", &alice[..8]]);
    assert!(blocked.status.success(), "{}", stderr(&blocked));
    assert_eq!(String::from_utf8_lossy(&blocked.stdout), "", "says nothing");

    let _running = home.start("photos");
    let visitors: serde_json::Value =
        serde_json::from_str(&home.ok(&["visitors", "photos", "--json"])).unwrap();
    assert_eq!(visitors[0]["key"], alice.as_str());
    assert_eq!(visitors[0]["status"], "Blocked", "{visitors}");
}

#[test]
fn invites_made_while_the_host_starts_are_all_kept_and_it_still_starts() {
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

    let running = home.spawn(&["run", "photos"]);
    let guests: Vec<String> = (0..12).map(|n| format!("guest{n}")).collect();
    std::thread::scope(|s| {
        for guest in &guests {
            let home = &home;
            s.spawn(move || home.ok(&["invite", "photos", guest]));
        }
    });
    running.until_it_says("serving");
    let listed = home.ok(&["invites", "photos"]);
    for guest in &guests {
        assert!(listed.contains(guest), "{guest} was lost: {listed}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn changes_print_nothing_invite_prints_only_the_link_and_reads_have_json() {
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
    let alice = a_visitor(&home, "photos", folder.path()).await;
    let _running = home.start("photos");

    let link = home.tooob(&["invite", "photos", "bob", "--expires", "1d"]);
    assert_eq!(stderr(&link), "", "nothing but the link");
    let link = String::from_utf8_lossy(&link.stdout).into_owned();
    assert_eq!(link.lines().count(), 1, "{link}");
    assert!(link.contains("&invite="), "{link}");

    let invites: serde_json::Value =
        serde_json::from_str(&home.ok(&["invites", "photos", "--json"])).unwrap();
    let bob = invites
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["label"] == "bob")
        .unwrap();
    let bob = bob["id"].as_str().unwrap();
    for change in [
        ["block", "photos", &alice[..8]],
        ["unblock", "photos", &alice[..8]],
        ["approve", "photos", &alice[..8]],
        ["revoke", "photos", bob],
    ] {
        let out = home.tooob(&change);
        assert!(out.status.success(), "{change:?}: {}", stderr(&out));
        assert_eq!(String::from_utf8_lossy(&out.stdout), "", "{change:?}");
        assert_eq!(stderr(&out), "", "{change:?}");
    }
    assert!(!home.ok(&["invites", "photos"]).contains("bob"));
    let visitors: serde_json::Value =
        serde_json::from_str(&home.ok(&["visitors", "photos", "--json"])).unwrap();
    assert_eq!(visitors[0]["status"], "Allowed");

    let wrong = home.tooob(&["block", "photos", "ffffffff"]);
    assert!(!wrong.status.success());
    assert_eq!(String::from_utf8_lossy(&wrong.stdout), "");
    assert!(
        stderr(&wrong).contains("unknown_visitor"),
        "{}",
        stderr(&wrong)
    );
}

#[test]
fn rm_while_owner_commands_are_made_deletes_the_host_completely() {
    for _ in 0..5 {
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
        let state = home.shown("photos", "state folder");
        let removed = std::thread::scope(|s| {
            for n in 0..8 {
                let home = &home;
                s.spawn(move || home.tooob(&["invite", "photos", &format!("guest{n}")]));
            }
            home.tooob(&["rm", "photos"])
        });
        assert!(removed.status.success(), "{}", stderr(&removed));
        assert!(!std::path::Path::new(&state).exists(), "{state} is left");
    }
}

#[test]
fn a_link_made_while_the_host_is_stopped_carries_the_relay_its_settings_name() {
    let home = Home::new();
    let folder = a_folder();
    let relay = "http://127.0.0.1:9/";
    let path = folder.path().to_str().unwrap();
    home.ok(&[
        "add",
        "photos",
        path,
        "--relay",
        relay,
        "--bootstrap",
        BOOTSTRAP,
    ]);
    let link = home.ok(&["invite", "photos", "alice"]);
    assert!(
        link.starts_with(&format!("{BOOTSTRAP}?relay={relay}#")),
        "{link}"
    );
}

#[test]
fn editing_the_relay_in_the_settings_file_changes_it_on_the_next_run() {
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
    let first = home.start("photos");
    assert!(first.log().contains("no Relay"), "{}", first.log());
    first.stop();

    let file = home.shown("photos", "settings file");
    let settings = std::fs::read_to_string(&file).unwrap();
    std::fs::write(
        &file,
        settings.replace("\"none\"", "\"http://127.0.0.1:9/\""),
    )
    .unwrap();
    assert_eq!(home.shown("photos", "relay"), "http://127.0.0.1:9/");
    let again = home.spawn(&["run", "photos"]);
    again.until_it_says("Relay http://127.0.0.1:9/");
}

#[test]
fn ls_shows_running_and_stopped_hosts_and_its_json_parses() {
    let home = Home::new();
    let folder = a_folder();
    let path = folder.path().canonicalize().unwrap();
    let path = path.to_str().unwrap();
    home.ok(&["add", "photos", path, "--local", "--bootstrap", BOOTSTRAP]);
    home.ok(&[
        "add",
        "films",
        "127.0.0.1:8096",
        "--local",
        "--bootstrap",
        BOOTSTRAP,
    ]);
    let _photos = home.start("photos");

    let listed = home.ok(&["ls"]);
    let heading: Vec<&str> = listed.lines().next().unwrap().split_whitespace().collect();
    assert_eq!(heading, ["NAME", "STATE", "ENABLED", "VISITORS", "SERVES"]);
    let photos: Vec<String> = home
        .row("photos")
        .split_whitespace()
        .map(String::from)
        .collect();
    assert_eq!(photos, ["photos", "running", "no", "0", path]);
    let films: Vec<String> = home
        .row("films")
        .split_whitespace()
        .map(String::from)
        .collect();
    assert_eq!(films, ["films", "stopped", "no", "-", "127.0.0.1:8096"]);

    let json: serde_json::Value = serde_json::from_str(&home.ok(&["ls", "--json"])).unwrap();
    let hosts = json.as_array().unwrap();
    assert_eq!(hosts.len(), 2);
    let photos = hosts.iter().find(|h| h["name"] == "photos").unwrap();
    assert_eq!(photos["state"], "running");
    assert_eq!(photos["enabled"], false);
    assert_eq!(photos["visitors"], 0);
    assert_eq!(photos["serves"], path);
    let films = hosts.iter().find(|h| h["name"] == "films").unwrap();
    assert_eq!(films["state"], "stopped");
}

#[test]
fn rm_refuses_a_running_host_and_deletes_a_stopped_one_completely() {
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
    let settings = home.shown("photos", "settings file");
    let state = home.shown("photos", "state folder");
    let key = home.shown("photos", "Host Key");
    let running = home.start("photos");

    let refused = home.tooob(&["rm", "photos"]);
    assert!(!refused.status.success());
    assert!(stderr(&refused).contains("running"), "{}", stderr(&refused));
    assert!(std::path::Path::new(&state).join("host.key").exists());

    running.stop();
    let removed = home.tooob(&["rm", "photos"]);
    assert!(removed.status.success(), "{}", stderr(&removed));
    assert_eq!(String::from_utf8_lossy(&removed.stdout), "", "says nothing");
    assert!(!std::path::Path::new(&settings).exists());
    assert!(!std::path::Path::new(&state).exists());
    assert!(!home.ok(&["ls"]).contains("photos"));

    home.ok(&[
        "add",
        "photos",
        folder.path().to_str().unwrap(),
        "--local",
        "--bootstrap",
        BOOTSTRAP,
    ]);
    assert_ne!(home.shown("photos", "Host Key"), key, "a new Host");
}

#[test]
fn add_refuses_a_bad_or_taken_name_and_makes_nothing() {
    let home = Home::new();
    let (photos, films) = (a_folder(), a_folder());
    home.ok(&[
        "add",
        "photos",
        photos.path().to_str().unwrap(),
        "--local",
        "--bootstrap",
        BOOTSTRAP,
    ]);
    let key = home.shown("photos", "Host Key");

    for name in [
        "my photos",
        "../photos",
        "photos/2",
        "",
        ".hidden",
        "fotos!",
        "photos",
    ] {
        let out = home.tooob(&[
            "add",
            name,
            films.path().to_str().unwrap(),
            "--local",
            "--bootstrap",
            BOOTSTRAP,
        ]);
        assert!(!out.status.success(), "{name:?} was taken as a name");
        assert!(stderr(&out).contains(name), "{name:?}: {}", stderr(&out));
    }
    assert_eq!(home.ok(&["ls"]).lines().count(), 2, "{}", home.ok(&["ls"]));
    assert_eq!(home.shown("photos", "Host Key"), key, "untouched");
    assert_eq!(
        home.shown("photos", "serves"),
        photos.path().canonicalize().unwrap().to_str().unwrap()
    );

    let nowhere = home.tooob(&[
        "add",
        "nowhere",
        "/no/such/folder",
        "--local",
        "--bootstrap",
        BOOTSTRAP,
    ]);
    assert!(!nowhere.status.success());
    assert_eq!(home.ok(&["ls"]).lines().count(), 2);
}

#[test]
fn a_host_is_named_by_its_name_path_or_host_key_and_never_guessed() {
    let home = Home::new();
    let (photos, films) = (a_folder(), a_folder());
    home.ok(&[
        "add",
        "photos",
        photos.path().to_str().unwrap(),
        "--local",
        "--bootstrap",
        BOOTSTRAP,
    ]);
    home.ok(&[
        "add",
        "films",
        films.path().to_str().unwrap(),
        "--local",
        "--bootstrap",
        BOOTSTRAP,
    ]);
    let films_key = home.shown("films", "Host Key");

    let by_path = home.ok(&["show", photos.path().to_str().unwrap()]);
    assert!(by_path.starts_with("photos:"), "{by_path}");
    let by_key = home.ok(&["show", &films_key[..4]]);
    assert!(by_key.starts_with("films:"), "{by_key}");

    for wanted in ["nope", &films_key[..3], "phot"] {
        let out = home.tooob(&["show", wanted]);
        assert!(!out.status.success(), "{wanted:?} was guessed");
        let err = stderr(&out);
        assert!(
            err.contains("photos") && err.contains("films"),
            "{wanted:?}: {err}"
        );
    }

    // Owner commands name the Host first, the same ways, running or not.
    let _photos = home.start("photos");
    let by_path = home.ok(&["invite", photos.path().to_str().unwrap(), "Alice"]);
    assert!(by_path.contains("&invite="), "{by_path}");
    assert!(home.ok(&["invites", &films_key[..6]]).is_empty());
    for args in [
        vec!["invite"],
        vec!["visitors"],
        vec!["invite", "Carol"],
        vec!["invite", "nope", "Carol"],
    ] {
        let out = home.tooob(&args);
        assert!(!out.status.success(), "{args:?} guessed a Host");
        let err = stderr(&out);
        assert!(
            err.contains("photos") && err.contains("films"),
            "{args:?}: {err}"
        );
    }
    assert!(!home.ok(&["invites", "photos"]).contains("Carol"));
}

#[test]
fn run_stops_cleanly_on_sigterm_ctrl_c_and_a_closing_terminal() {
    // Docker and service managers send SIGTERM; Ctrl-C is SIGINT; a closing terminal sends SIGHUP.
    // Stopping cleanly is what tells connected Visitors at once (tests/host.rs).
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
    let state = home.shown("photos", "state folder");
    for signal in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
        let mut running = home.start("photos");
        let status = running.stop_with(signal);
        assert!(status.success(), "signal {signal}: {status}");
        assert!(
            running.log().contains("photos: stopped"),
            "{}",
            running.log()
        );
        assert!(
            !std::path::Path::new(&state).join("control.sock").exists(),
            "signal {signal}: control socket left behind"
        );
        assert!(home.row("photos").contains("stopped"));
    }
}

#[test]
fn a_host_still_waiting_for_its_relay_stops_when_asked() {
    let home = Home::new();
    let folder = a_folder();
    // A Relay that never answers: the Host waits up to 15 s for it before it says it's serving.
    let dead = "http://127.0.0.1:9/";
    home.ok(&[
        "add",
        "photos",
        folder.path().to_str().unwrap(),
        "--relay",
        dead,
        "--bootstrap",
        BOOTSTRAP,
    ]);
    home.ok(&[
        "add",
        "films",
        folder.path().to_str().unwrap(),
        "--relay",
        dead,
        "--bootstrap",
        BOOTSTRAP,
    ]);

    let mut photos = home.spawn(&["run", "photos"]);
    photos.until_it_says("through Relay");
    let asked = std::time::Instant::now();
    assert!(photos.stop_with(libc::SIGTERM).success());

    let films = home.spawn(&["run", "films"]);
    films.until_it_says("through Relay");
    home.ok(&["stop", "films"]);
    films.until_it_says("films: stopped");
    // The Relay wait alone would take until 15 s after starting.
    assert!(asked.elapsed() < std::time::Duration::from_secs(10));
}

#[test]
fn ending_one_host_leaves_the_others_serving() {
    let home = Home::new();
    let (photos, films) = (a_folder(), a_folder());
    home.ok(&[
        "add",
        "photos",
        photos.path().to_str().unwrap(),
        "--local",
        "--bootstrap",
        BOOTSTRAP,
    ]);
    home.ok(&[
        "add",
        "films",
        films.path().to_str().unwrap(),
        "--local",
        "--bootstrap",
        BOOTSTRAP,
    ]);
    let photos = home.start("photos");
    let _films = home.start("films");
    photos.stop();
    assert!(home.row("photos").contains("stopped"));
    assert!(home.row("films").contains("running"));
    home.ok(&["invite", "films", "Alice"]);
}

#[test]
fn the_old_commands_and_options_are_gone() {
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
    for args in [
        vec!["serve", "--site", "127.0.0.1:8096"],
        vec!["start", "--host", "photos"],
        vec!["list"],
        vec!["run", "photos", "--detach"],
        vec!["run", "photos", "--foreground"],
        vec!["visitors", "--data", "/tmp"],
        vec!["visitors", "photos", "--host", "photos"],
        vec!["run", "photos", "--relay", "http://127.0.0.1:9/"],
    ] {
        let out = home.tooob(&args);
        assert!(!out.status.success(), "{args:?} still works");
    }
    assert!(home.row("photos").contains("stopped"));
}

#[test]
fn share_adds_a_host_named_after_the_folder_invites_and_runs_it_and_reuses_it_next_time() {
    let home = Home::new();
    let parent = tempfile::tempdir().unwrap();
    let folder = parent.path().join("My Photos");
    std::fs::create_dir(&folder).unwrap();
    let path = folder.to_str().unwrap();

    let shared = home.spawn(&[
        "share",
        path,
        "--local",
        "--bootstrap",
        BOOTSTRAP,
        "--invite",
        "Alice",
        "--invite",
        "Bob",
    ]);
    shared.until_it_says("Invite for Bob");
    shared.until_it_says("&invite=");
    let invites = home.ok(&["invites", "my-photos"]);
    assert!(
        invites.contains("Alice") && invites.contains("Bob"),
        "{invites}"
    );
    let key = home.shown("my-photos", "Host Key");
    shared.stop();

    let again = home.spawn(&["share", path, "--invite", "Carol"]);
    again.until_it_says("Invite for Carol to my-photos");
    assert_eq!(home.shown("my-photos", "Host Key"), key, "the same Host");
    assert_eq!(home.ok(&["ls"]).lines().count(), 2, "{}", home.ok(&["ls"]));
    assert_eq!(home.shown("my-photos", "relay"), "none", "as it was added");
}

#[test]
fn a_settings_file_that_cant_be_read_says_why_rather_than_hiding_the_host() {
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
    let file = home.shown("photos", "settings file");
    let settings = std::fs::read_to_string(&file).unwrap();
    std::fs::write(&file, settings.replace("\"none\"", "none")).unwrap();

    for command in ["run", "show", "rm"] {
        let out = home.tooob(&[command, "photos"]);
        assert!(!out.status.success(), "{command}");
        assert!(
            stderr(&out).contains("photos.toml"),
            "{command}: {}",
            stderr(&out)
        );
    }
}
