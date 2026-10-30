//! The Owner's Hosts on one box, through the `tooob` command run as a script would (no terminal):
//! each made once (`add`) and run in the foreground (`run`), one process per Host (ADR 0014).
mod support;

use std::process::Output;
use support::home::{BOOTSTRAP, Home, RELAY, TestRelay, Z_BASE_32, a_folder};
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
    home.ok(&["add", "photos", folder.path().to_str().unwrap(), "--local"]);
    home.ok(&["add", "films", "127.0.0.1:8096", "--local"]);

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
    let relay = TestRelay::start();
    let path = folder.path().to_str().unwrap();
    home.ok(&["add", "photos", path, "--relay", &relay.url]);
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
fn add_needs_a_relay_and_a_bootstrap_is_refused() {
    let home = Home::new();
    let folder = a_folder();
    let path = folder.path().to_str().unwrap();
    let none = stderr(&home.tooob(&["add", "photos", path]));
    assert!(
        none.contains("--relay") && none.contains("deploy/") && none.contains("usage:"),
        "{none}"
    );
    let usage = stderr(&home.tooob(&[]));
    assert!(
        usage.contains("usage:") && !usage.to_lowercase().contains("bootstrap"),
        "{usage}"
    );
    for args in [
        vec![
            "add",
            "photos",
            path,
            "--relay",
            RELAY,
            "--bootstrap",
            BOOTSTRAP,
        ],
        vec!["share", path, "--relay", RELAY, "--bootstrap", BOOTSTRAP],
    ] {
        let out = home.tooob(&args);
        assert!(!out.status.success(), "{args:?}");
        let err = stderr(&out);
        assert!(
            err.contains("--bootstrap") && err.contains("only its Relay"),
            "{args:?}: {err}"
        );
    }
    assert_eq!(home.ok(&["ls"]).lines().count(), 1, "nothing was added");
}

#[test]
fn share_without_a_terminal_says_what_a_new_host_needs_and_a_reused_one_keeps_its_own() {
    let home = Home::new();
    let folder = a_folder();
    let path = folder.path().to_str().unwrap();
    let out = home.tooob(&["share", path]);
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("--relay") && stderr(&out).contains("deploy/"),
        "{}",
        stderr(&out)
    );

    let relay = TestRelay::start();
    home.ok(&["add", "photos", path, "--relay", &relay.url]);
    let key = home.shown("photos", "Host Key");
    let again = home.spawn(&["share", path, "--relay", "nonsense", "--invite", "Ann"]);
    again.until_it_says(&format!("http://{key}.localhost:"));
    assert_eq!(home.shown("photos", "relay"), relay.url);
}

#[test]
fn an_old_settings_file_naming_a_bootstrap_still_runs_and_its_links_are_on_its_relay() {
    let home = Home::new();
    let folder = a_folder();
    let relay = TestRelay::start();
    home.ok(&[
        "add",
        "photos",
        folder.path().to_str().unwrap(),
        "--relay",
        &relay.url,
    ]);
    let file = home.shown("photos", "settings file");
    let settings = std::fs::read_to_string(&file).unwrap();
    assert!(!settings.contains("bootstrap"), "{settings}");
    std::fs::write(&file, format!("{settings}bootstrap = \"{BOOTSTRAP}\"\n")).unwrap();
    let key = home.shown("photos", "Host Key");
    let port = relay.url.trim_end_matches('/').rsplit(':').next().unwrap();
    let on_its_relay = format!("http://{key}.localhost:{port}/#invite=");

    let stopped = home.ok(&["invite", "photos", "alice"]);
    assert!(stopped.starts_with(&on_its_relay), "{stopped}");
    assert!(
        !home
            .ok(&["show", "photos"])
            .to_lowercase()
            .contains("bootstrap")
    );
    let _running = home.start("photos");
    let paired = home.ok(&["pair", "photos", "my phone"]);
    assert!(paired.starts_with(&on_its_relay), "{paired}");
}

#[test]
fn an_invite_link_is_its_hosts_key_address_on_its_relays_domain() {
    let home = Home::new();
    let folder = a_folder();
    let path = folder.path().to_str().unwrap();
    home.add_naming("photos", path, "https://relay.example:8443/");
    let key = home.shown("photos", "Host Key");
    assert!(
        key.len() == 52 && key.chars().all(|c| Z_BASE_32.contains(c)),
        "a hostname's label, in z-base-32: {key}"
    );

    let link = home.ok(&["invite", "photos", "alice"]);
    let token = link
        .trim()
        .strip_prefix(&format!("https://{key}.relay.example:8443/#invite="))
        .unwrap_or_else(|| panic!("{link}"));
    assert!(!token.is_empty() && !token.contains('&'), "{link}");
}

#[test]
fn a_host_with_no_relay_or_a_relay_at_an_ip_address_has_no_key_address_so_no_invites() {
    let at_an_ip = TestRelay::bootstrap_only();
    for (relay, why) in [
        (vec!["--local"], "no Relay"),
        (vec!["--relay", at_an_ip.ip_url.as_str()], "IP address"),
    ] {
        let home = Home::new();
        let folder = a_folder();
        let path = folder.path().to_str().unwrap();
        home.ok(&[vec!["add", "photos", path], relay.clone()].concat());
        let file = home.shown("photos", "settings file");
        for args in [
            ["invite", "photos", "alice"],
            ["pair", "photos", "my phone"],
        ] {
            let out = home.tooob(&args);
            assert!(!out.status.success(), "{relay:?} {args:?}");
            let err = stderr(&out);
            assert!(
                err.contains("no Key Address") && err.contains(why) && err.contains(&file),
                "{relay:?} {args:?}: {err}"
            );
        }
        assert!(
            home.ok(&["invites", "photos"]).is_empty(),
            "{relay:?}: an Invite nobody has a link for"
        );

        let other = a_folder();
        let new = [
            vec!["share", other.path().to_str().unwrap()],
            relay.clone(),
            vec!["--invite", "Ann"],
        ]
        .concat();
        for args in [new, vec!["share", path, "--invite", "Ann"]] {
            let out = home.tooob(&args);
            assert!(!out.status.success(), "{args:?}");
            let err = stderr(&out);
            assert!(
                err.contains("no Key Address") && err.contains(why),
                "{args:?}: {err}"
            );
        }
        assert_eq!(home.ok(&["ls"]).lines().count(), 2, "nothing was added");
        assert!(home.row("photos").contains("stopped"), "nor run");
    }
}

#[test]
fn an_invite_made_while_the_host_is_stopped_is_there_once_it_runs() {
    let home = Home::new();
    let folder = a_folder();
    let relay = TestRelay::start();
    let path = folder.path().to_str().unwrap();
    home.ok(&["add", "photos", path, "--relay", &relay.url]);
    let key = home.shown("photos", "Host Key");

    let link = home.ok(&["invite", "photos", "alice"]);
    assert_eq!(link.lines().count(), 1, "{link}");
    assert!(
        link.starts_with(&format!("http://{key}.localhost:")) && link.contains("/#invite="),
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

#[tokio::test(flavor = "multi_thread")]
async fn a_block_made_while_the_host_is_stopped_holds_once_it_runs() {
    let home = Home::new();
    let folder = a_folder();
    let relay = TestRelay::start();
    let path = folder.path().to_str().unwrap();
    home.ok(&["add", "photos", path, "--relay", &relay.url]);
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
    let relay = TestRelay::start();
    let path = folder.path().to_str().unwrap();
    home.ok(&["add", "photos", path, "--relay", &relay.url]);

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
    let relay = TestRelay::start();
    let path = folder.path().to_str().unwrap();
    home.ok(&["add", "photos", path, "--relay", &relay.url]);
    let alice = a_visitor(&home, "photos", folder.path()).await;
    let _running = home.start("photos");

    let link = home.tooob(&["invite", "photos", "bob", "--expires", "1d"]);
    assert_eq!(stderr(&link), "", "nothing but the link");
    let link = String::from_utf8_lossy(&link.stdout).into_owned();
    assert_eq!(link.lines().count(), 1, "{link}");
    assert!(link.contains("#invite="), "{link}");

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
        let path = folder.path().to_str().unwrap();
        home.add_naming("photos", path, RELAY);
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
fn editing_the_relay_in_the_settings_file_changes_it_on_the_next_run() {
    let home = Home::new();
    let folder = a_folder();
    home.ok(&["add", "photos", folder.path().to_str().unwrap(), "--local"]);
    let first = home.start("photos");
    assert!(first.log().contains("no Relay"), "{}", first.log());
    first.stop();

    let relay = TestRelay::start();
    let file = home.shown("photos", "settings file");
    let settings = std::fs::read_to_string(&file).unwrap();
    std::fs::write(
        &file,
        settings.replace("\"none\"", &format!("\"{}\"", relay.url)),
    )
    .unwrap();
    assert_eq!(home.shown("photos", "relay"), relay.url);
    let again = home.start("photos");
    assert!(
        again
            .log()
            .contains(&format!("through Relay {}", relay.url)),
        "{}",
        again.log()
    );
}

#[test]
fn ls_shows_running_and_stopped_hosts_and_its_json_parses() {
    let home = Home::new();
    let folder = a_folder();
    let path = folder.path().canonicalize().unwrap();
    let path = path.to_str().unwrap();
    home.ok(&["add", "photos", path, "--local"]);
    home.ok(&["add", "films", "127.0.0.1:8096", "--local"]);
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
    home.ok(&["add", "photos", folder.path().to_str().unwrap(), "--local"]);
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

    home.ok(&["add", "photos", folder.path().to_str().unwrap(), "--local"]);
    assert_ne!(home.shown("photos", "Host Key"), key, "a new Host");
}

#[test]
fn add_refuses_a_bad_or_taken_name_and_makes_nothing() {
    let home = Home::new();
    let (photos, films) = (a_folder(), a_folder());
    home.ok(&["add", "photos", photos.path().to_str().unwrap(), "--local"]);
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
        let out = home.tooob(&["add", name, films.path().to_str().unwrap(), "--local"]);
        assert!(!out.status.success(), "{name:?} was taken as a name");
        assert!(stderr(&out).contains(name), "{name:?}: {}", stderr(&out));
    }
    assert_eq!(home.ok(&["ls"]).lines().count(), 2, "{}", home.ok(&["ls"]));
    assert_eq!(home.shown("photos", "Host Key"), key, "untouched");
    assert_eq!(
        home.shown("photos", "serves"),
        photos.path().canonicalize().unwrap().to_str().unwrap()
    );

    let nowhere = home.tooob(&["add", "nowhere", "/no/such/folder", "--local"]);
    assert!(!nowhere.status.success());
    assert_eq!(home.ok(&["ls"]).lines().count(), 2);
}

#[test]
fn a_host_is_named_by_its_name_path_or_host_key_and_never_guessed() {
    let home = Home::new();
    let (photos, films) = (a_folder(), a_folder());
    let relay = TestRelay::start();
    let path = photos.path().to_str().unwrap();
    home.ok(&["add", "photos", path, "--relay", &relay.url]);
    home.ok(&["add", "films", films.path().to_str().unwrap(), "--local"]);
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
    assert!(by_path.contains("#invite="), "{by_path}");
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
    home.ok(&["add", "photos", folder.path().to_str().unwrap(), "--local"]);
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
    // A Relay that serves its Bootstrap but never lets a Host on: the Host waits up to 15 s for it
    // before it says it's serving.
    let stuck = TestRelay::bootstrap_only();
    let dead = stuck.url.as_str();
    home.ok(&[
        "add",
        "photos",
        folder.path().to_str().unwrap(),
        "--relay",
        dead,
    ]);
    home.ok(&[
        "add",
        "films",
        folder.path().to_str().unwrap(),
        "--relay",
        dead,
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
    let relay = TestRelay::start();
    home.ok(&["add", "photos", photos.path().to_str().unwrap(), "--local"]);
    let path = films.path().to_str().unwrap();
    home.ok(&["add", "films", path, "--relay", &relay.url]);
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
    home.ok(&["add", "photos", folder.path().to_str().unwrap(), "--local"]);
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

    let relay = TestRelay::start();
    let shared = home.spawn(&[
        "share", path, "--relay", &relay.url, "--invite", "Alice", "--invite", "Bob",
    ]);
    shared.until_it_says("Invite for Bob");
    shared.until_it_says("#invite=");
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
    assert_eq!(
        home.shown("my-photos", "relay"),
        relay.url,
        "as it was added"
    );
}

#[test]
fn a_settings_file_that_cant_be_read_says_why_rather_than_hiding_the_host() {
    let home = Home::new();
    let folder = a_folder();
    home.ok(&["add", "photos", folder.path().to_str().unwrap(), "--local"]);
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
