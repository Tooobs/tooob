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
        none.contains("--relay https://RELAY/") && none.contains("deploy/"),
        "{none}"
    );
    let usage = stderr(&home.tooob(&[]));
    assert!(
        usage.contains("usage:") && !usage.to_lowercase().contains("bootstrap"),
        "{usage}"
    );
    for command in [
        "tooob run NAME [-d]",
        "[--relay URL | --local] [--uploads] [-d]",
        "tooob stop NAME",
        "tooob logs NAME [-n N|all] [-f] [--json]",
    ] {
        assert!(usage.contains(command), "{command}: {usage}");
    }
    for gone in ["enable", "disable"] {
        assert!(!usage.contains(&format!("tooob {gone}")), "{gone}: {usage}");
    }
    assert!(
        !usage.contains("TOOOB_"),
        "test hooks aren't the Owner's: {usage}"
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
    assert_eq!(
        heading,
        ["NAME", "STATE", "AUTOSTART", "VISITORS", "SERVES"]
    );
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
    assert_eq!(photos["autostart"], false);
    assert_eq!(photos["visitors"], 0);
    assert_eq!(photos["serves"], path);
    let films = hosts.iter().find(|h| h["name"] == "films").unwrap();
    assert_eq!(films["state"], "stopped");
}

#[test]
fn rm_stops_a_running_host_and_deletes_it_completely() {
    let home = Home::new();
    let folder = a_folder();
    home.ok(&["add", "photos", folder.path().to_str().unwrap(), "--local"]);
    let settings = home.shown("photos", "settings file");
    let state = home.shown("photos", "state folder");
    let key = home.shown("photos", "Host Key");
    let running = home.start("photos");

    let removed = home.tooob(&["rm", "photos"]);
    assert!(removed.status.success(), "{}", stderr(&removed));
    assert_eq!(String::from_utf8_lossy(&removed.stdout), "", "says nothing");
    assert!(!std::path::Path::new(&settings).exists());
    assert!(!std::path::Path::new(&state).exists());
    assert!(!home.ok(&["ls"]).contains("photos"));
    running.until_it_says("photos: stopped");

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
fn enable_and_disable_are_gone_and_each_says_what_replaced_it() {
    let home = Home::new();
    let folder = a_folder();
    home.ok(&["add", "photos", folder.path().to_str().unwrap(), "--local"]);
    home.install_tooob();
    home.stand_in("launchctl", "");
    for (args, said) in [
        (
            vec!["enable", "photos"],
            "tooob: enable is now run -d: tooob run photos -d\n",
        ),
        (
            vec!["enable"],
            "tooob: enable is now run -d: tooob run NAME -d\n",
        ),
        (
            vec!["disable", "photos"],
            "tooob: disable is now stop (or rm): tooob stop photos\n",
        ),
        (
            vec!["disable"],
            "tooob: disable is now stop (or rm): tooob stop NAME\n",
        ),
    ] {
        let out = home.tooob(&args);
        assert!(!out.status.success(), "{args:?}");
        assert_eq!(stderr(&out), said, "{args:?}");
    }
    assert!(entries(&home).is_empty(), "{:?}", entries(&home));
    assert!(home.calls("launchctl").is_empty());
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
    shared.until_it_says("    Bob · works for 7 days\n");
    shared.until_it_says("#invite=");
    let invites = home.ok(&["invites", "my-photos"]);
    assert!(
        invites.contains("Alice") && invites.contains("Bob"),
        "{invites}"
    );
    let key = home.shown("my-photos", "Host Key");
    shared.stop();

    let again = home.spawn(&["share", path, "--invite", "Carol"]);
    again.until_it_says("    Carol · works for 7 days\n");
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

#[test]
fn share_with_no_path_says_what_to_share_and_shows_the_command_with_a_placeholder() {
    let home = Home::new();
    let out = home.tooob(&["share", "--relay", RELAY, "--invite", "sam"]);
    assert!(!out.status.success());
    assert_eq!(
        stderr(&out),
        "tooob: say what to share, a folder or a file:\n  \
         tooob share PATH --relay https://relay.example/ --invite sam\n"
    );
}

#[test]
fn a_new_host_with_no_relay_says_which_and_shows_the_command_quoted_to_paste_back() {
    let home = Home::new();
    let parent = tempfile::tempdir().unwrap();
    let folder = parent.path().join("My Photos");
    std::fs::create_dir(&folder).unwrap();
    let path = folder.to_str().unwrap();
    let say = "tooob: say which Relay its Visitors reach it through (or --local for this network \
               only; deploy/ shows how to run your own):\n  ";
    for (args, shown) in [
        (
            vec!["share", path, "--invite", "Sam Smith"],
            format!("tooob share '{path}' --relay https://RELAY/ --invite 'Sam Smith'\n"),
        ),
        (
            vec!["add", "photos", path],
            format!("tooob add photos '{path}' --relay https://RELAY/\n"),
        ),
    ] {
        let out = home.tooob(&args);
        assert!(!out.status.success(), "{args:?}");
        assert_eq!(stderr(&out), format!("{say}{shown}"), "{args:?}");
    }
    assert_eq!(home.ok(&["ls"]).lines().count(), 1, "nothing was added");
}

#[test]
fn an_owner_command_with_no_host_name_lists_the_hosts_and_stop_lists_the_running_ones() {
    let home = Home::new();
    let fails = |args: &[&str]| {
        let out = home.tooob(args);
        assert!(!out.status.success(), "{args:?}");
        stderr(&out)
    };
    assert_eq!(
        fails(&["invite", "--expires", "1d"]),
        "tooob: say which Host the Invite is for (none yet: `tooob add NAME TARGET` makes \
         one):\n  tooob invite NAME LABEL --expires 1d\n"
    );
    let folder = a_folder();
    home.ok(&["add", "photos", folder.path().to_str().unwrap(), "--local"]);
    home.ok(&["add", "films", "127.0.0.1:8096", "--local"]);
    assert_eq!(
        fails(&["stop"]),
        "tooob: say which Host to stop (none is running):\n  tooob stop NAME\n"
    );
    let _photos = home.start("photos");
    for (args, said) in [
        (
            vec!["invite"],
            "tooob: say which Host the Invite is for (Hosts here: films, photos):\n  \
             tooob invite NAME LABEL\n",
        ),
        (
            vec!["pair"],
            "tooob: say which Host the Pairing Invite is for (Hosts here: films, photos):\n  \
             tooob pair NAME LABEL\n",
        ),
        (
            vec!["visitors", "--json"],
            "tooob: say which Host (Hosts here: films, photos):\n  \
             tooob visitors NAME --json\n",
        ),
        (
            vec!["stop"],
            "tooob: say which Host to stop (running: photos):\n  tooob stop NAME\n",
        ),
    ] {
        assert_eq!(fails(&args), said, "{args:?}");
    }
    assert!(
        home.row("photos").contains("running"),
        "nothing was stopped"
    );
}

#[test]
fn an_owner_command_with_no_label_or_key_says_what_and_shows_the_command() {
    let home = Home::new();
    let folder = a_folder();
    home.add_naming("photos", folder.path().to_str().unwrap(), RELAY);
    for (args, said) in [
        (
            vec!["invite", "photos", "--approve"],
            "tooob: say who the Invite is for:\n  tooob invite photos LABEL --approve\n",
        ),
        (
            vec!["pair", "photos"],
            "tooob: say which device the Pairing Invite is for:\n  tooob pair photos LABEL\n",
        ),
        (
            vec!["block", "photos"],
            "tooob: say which Visitor, by their Visitor Key (a unique prefix is enough):\n  \
             tooob block photos KEY\n",
        ),
    ] {
        let out = home.tooob(&args);
        assert!(!out.status.success(), "{args:?}");
        assert_eq!(stderr(&out), said, "{args:?}");
    }
    assert!(home.ok(&["invites", "photos"]).is_empty(), "no Invite made");
}

#[test]
fn share_warns_in_a_line_for_each_thing_a_folder_holds_that_everyone_invited_could_read() {
    let home = Home::new();
    let folder = a_folder();
    let path = folder.path().to_str().unwrap();
    std::fs::create_dir_all(folder.path().join("blog/.git")).unwrap();
    std::fs::write(folder.path().join("blog/.env"), "KEY=1\n").unwrap();
    std::fs::create_dir(folder.path().join(".ssh")).unwrap();
    let warnings = [
        format!(
            "tooob: warning: {path} holds blog/.git, a repository's whole history, which \
             everyone you invite could read\n"
        ),
        format!("tooob: warning: {path} holds blog/.env, which everyone you invite could read\n"),
        format!("tooob: warning: {path} holds .ssh, which everyone you invite could read\n"),
    ];

    let shared = home.spawn(&["share", path, "--local"]);
    shared.until_it_says("serving");
    let log = shared.log();
    let serving = log.find("serving").unwrap();
    for warning in &warnings {
        let at = log
            .find(warning.as_str())
            .unwrap_or_else(|| panic!("{warning}in:\n{log}"));
        assert!(at < serving, "before serving: {log}");
    }
    assert!(!log.contains("too big"), "{log}");
    shared.stop();

    let again = home.spawn(&["share", path]);
    again.until_it_says("serving");
    for warning in &warnings {
        assert!(
            again.log().contains(warning.as_str()),
            "reused: {}",
            again.log()
        );
    }
    let usage = stderr(&home.tooob(&[]));
    assert!(
        !usage.contains("include-git") && !usage.contains("asks"),
        "{usage}"
    );
}

#[test]
fn share_warns_about_the_whole_home_folder() {
    let home = Home::new();
    let path = home.0.path().to_str().unwrap();
    let shared = home.spawn(&["share", path, "--local"]);
    shared.until_it_says("serving");
    let warning = format!(
        "tooob: warning: {path} is your whole home folder, which everyone you invite could read\n"
    );
    assert!(shared.log().contains(&warning), "{}", shared.log());
}

#[test]
fn share_says_when_a_folder_is_too_big_to_check_all_of_it_rather_than_calling_it_clear() {
    let home = Home::new();
    let folder = a_folder();
    let path = folder.path().to_str().unwrap();
    let many = folder.path().join("a");
    std::fs::create_dir(&many).unwrap();
    for n in 0..21_000 {
        std::fs::write(many.join(n.to_string()), "").unwrap();
    }
    std::fs::create_dir_all(folder.path().join("z/repo/.git")).unwrap();

    let shared = home.spawn(&["share", path, "--local"]);
    shared.until_it_says("serving");
    let warning =
        format!("tooob: warning: {path} is too big to check all of it for .git, .env or .ssh\n");
    assert!(shared.log().contains(&warning), "{}", shared.log());
}

#[test]
fn add_run_show_and_rm_missing_something_say_what_and_show_the_command() {
    let home = Home::new();
    let folder = a_folder();
    home.ok(&["add", "photos", folder.path().to_str().unwrap(), "--local"]);
    for (args, said) in [
        (
            vec!["add", "--local"],
            "tooob: say the new Host's name and what it serves, a folder, a file or HOST:PORT:\n  \
             tooob add NAME TARGET --local\n",
        ),
        (
            vec!["add", "films", "--local"],
            "tooob: say what films serves, a folder, a file or HOST:PORT:\n  \
             tooob add films TARGET --local\n",
        ),
        (
            vec!["run"],
            "tooob: say which Host (Hosts here: photos):\n  tooob run NAME\n",
        ),
        (
            vec!["show"],
            "tooob: say which Host (Hosts here: photos):\n  tooob show NAME\n",
        ),
        (
            vec!["rm"],
            "tooob: say which Host (Hosts here: photos):\n  tooob rm NAME\n",
        ),
    ] {
        let out = home.tooob(&args);
        assert!(!out.status.success(), "{args:?}");
        assert_eq!(stderr(&out), said, "{args:?}");
    }
    assert_eq!(
        home.ok(&["ls"]).lines().count(),
        2,
        "nothing added or removed"
    );
}

#[test]
fn share_uploads_turns_uploads_on_in_its_settings_for_a_new_host_and_a_reused_one() {
    let home = Home::new();
    let folder = a_folder();
    let path = folder.path().to_str().unwrap();
    let shared = home.spawn(&["share", path, "--local", "--uploads", "--name", "new"]);
    shared.until_it_says("serving");
    shared.stop();
    let settings = std::fs::read_to_string(home.shown("new", "settings file")).unwrap();
    assert!(settings.contains("\nuploads = true\n"), "{settings}");

    let other = a_folder();
    let path = other.path().to_str().unwrap();
    home.ok(&["add", "old", path, "--local"]);
    let file = home.shown("old", "settings file");
    assert!(
        !std::fs::read_to_string(&file)
            .unwrap()
            .contains("uploads = true")
    );
    let shared = home.spawn(&["share", path, "--uploads"]);
    shared.until_it_says("serving");
    shared.stop();
    let settings = std::fs::read_to_string(&file).unwrap();
    assert_eq!(settings.matches("uploads = true").count(), 1, "{settings}");
    home.ok(&["show", "old"]);

    // Turned on, it stays on: share without --uploads leaves it, and never adds a second line.
    let shared = home.spawn(&["share", path, "--uploads"]);
    shared.until_it_says("serving");
    shared.stop();
    let shared = home.spawn(&["share", path]);
    shared.until_it_says("serving");
    shared.stop();
    let settings = std::fs::read_to_string(&file).unwrap();
    assert_eq!(settings.matches("uploads = true").count(), 1, "{settings}");
}

#[test]
fn share_with_no_invite_makes_none_and_serves() {
    let home = Home::new();
    let folder = a_folder();
    let relay = TestRelay::start();
    let path = folder.path().to_str().unwrap();
    let shared = home.spawn(&["share", path, "--relay", &relay.url]);
    shared.until_it_says("serving");
    let name = home
        .ok(&["ls"])
        .lines()
        .nth(1)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_string();
    assert!(home.ok(&["invites", &name]).is_empty(), "{}", shared.log());
    assert!(!shared.log().contains("#invite="), "{}", shared.log());
}

/// What `run -d` leaves under the test's HOME: entries for launchd or systemd.
fn entries(home: &Home) -> Vec<std::path::PathBuf> {
    ["Library/LaunchAgents", ".config/systemd/user"]
        .iter()
        .flat_map(|dir| {
            std::fs::read_dir(home.0.path().join(dir))
                .into_iter()
                .flatten()
        })
        .map(|e| e.unwrap().path())
        .collect()
}

#[test]
fn run_d_with_neither_systemd_nor_launchd_says_what_to_have_run_and_writes_nothing() {
    let home = Home::new();
    let folder = a_folder();
    home.ok(&["add", "blog", folder.path().to_str().unwrap(), "--local"]);
    home.install_tooob();

    let out = home.tooob(&["run", "blog", "-d"]);
    assert!(!out.status.success());
    assert_eq!(
        stderr(&out),
        "tooob: keeping blog running needs systemd or launchd, and this system has neither.\n  \
         Have your service manager run `~/.local/bin/tooob run blog` as you, restarting it on \
         failure.\n"
    );
    assert!(entries(&home).is_empty(), "{:?}", entries(&home));
    assert!(home.row("blog").contains(" no "), "{}", home.row("blog"));
}

/// Where `photos`' service manager puts what it prints itself: only its dying words, since it
/// writes its own log (ADR 0025).
fn crash_log(home: &Home) -> String {
    format!("{}/crash.log", home.shown("photos", "state folder"))
}

/// The crash log is made by `run -d`, so the service manager appends to a file only the Owner's
/// account reads.
fn assert_crash_log_is_private(crashes: &str) {
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(crashes).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600, "{crashes}");
}

/// This user's id, as launchd's `gui/UID` domain names it.
fn uid() -> u32 {
    unsafe { libc::getuid() }
}

#[test]
fn run_d_with_launchd_writes_a_launch_agent_running_the_installed_tooob_and_loads_it() {
    let home = Home::new();
    let folder = a_folder();
    home.ok(&["add", "photos", folder.path().to_str().unwrap(), "--local"]);
    let installed = home.install_tooob();
    home.stand_in("launchctl", "");
    let crashes = crash_log(&home);
    assert_eq!(home.shown("photos", "Autostart"), "no");

    let out = home.tooob(&["run", "photos", "-d"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(String::from_utf8_lossy(&out.stdout), "");
    let said = stderr(&out);
    assert!(
        said.contains("won't idle-sleep") && said.contains("lid"),
        "says a closed lid still sleeps: {said}"
    );
    assert!(
        said.starts_with(
            "tooob: photos is kept running now, and after logins, reboots and crashes: `tooob \
             logs photos -f` follows it, and `tooob stop photos` stops it.\n"
        ),
        "{said}"
    );

    let agent = home
        .0
        .path()
        .join("Library/LaunchAgents/net.tooobs.photos.plist");
    assert_eq!(entries(&home), std::slice::from_ref(&agent));
    assert_eq!(
        home.shown("photos", "Autostart"),
        format!("yes, {}", agent.display())
    );
    let plist = std::fs::read_to_string(&agent).unwrap();
    let tooob_home = home.0.path().to_str().unwrap();
    for (key, value) in [
        ("Label", "<string>net.tooobs.photos</string>".to_string()),
        (
            "ProgramArguments",
            format!(
                "<array>\n\t\t<string>{}</string>\n\t\t<string>run</string>\n\t\t\
                 <string>photos</string>\n\t</array>",
                installed.display()
            ),
        ),
        ("RunAtLoad", "<true/>".to_string()),
        (
            "KeepAlive",
            "<dict>\n\t\t<key>SuccessfulExit</key>\n\t\t<false/>\n\t</dict>".to_string(),
        ),
        ("StandardOutPath", format!("<string>{crashes}</string>")),
        ("StandardErrorPath", format!("<string>{crashes}</string>")),
    ] {
        let at = format!("<key>{key}</key>\n\t{value}");
        assert!(plist.contains(&at), "no {at}\nin:\n{plist}");
    }
    assert!(
        plist.contains(&format!(
            "<key>TOOOB_HOME</key>\n\t\t<string>{tooob_home}</string>"
        )),
        "a Host in a TOOOB_HOME of its own runs there: {plist}"
    );
    if cfg!(target_os = "macos") {
        let lint = std::process::Command::new("/usr/bin/plutil")
            .arg("-lint")
            .arg(&agent)
            .output()
            .unwrap();
        assert!(lint.status.success(), "{lint:?}");
    }
    assert_eq!(
        home.calls("launchctl"),
        [format!("bootstrap gui/{} {}", uid(), agent.display())]
    );
    assert_crash_log_is_private(&crashes);
    let row: Vec<String> = home
        .row("photos")
        .split_whitespace()
        .map(String::from)
        .collect();
    assert_eq!(row[1..3], ["stopped", "yes"], "{row:?}");
}

#[test]
fn run_d_with_systemd_writes_a_user_unit_running_the_installed_tooob_and_starts_it() {
    let home = Home::new();
    let folder = a_folder();
    home.ok(&["add", "photos", folder.path().to_str().unwrap(), "--local"]);
    let installed = home.install_tooob();
    home.stand_in("systemctl", "");
    let crashes = crash_log(&home);

    let out = home.tooob(&["run", "photos", "-d"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let said = stderr(&out);
    assert!(said.contains("loginctl enable-linger"), "{said}");
    assert!(said.contains("tooob logs photos -f"), "{said}");

    let unit = home
        .0
        .path()
        .join(".config/systemd/user/tooob@photos.service");
    assert_eq!(entries(&home), std::slice::from_ref(&unit));
    let text = std::fs::read_to_string(&unit).unwrap();
    for line in [
        format!("ExecStart=\"{}\" run photos", installed.display()),
        format!("Environment=\"TOOOB_HOME={}\"", home.0.path().display()),
        "Restart=on-failure".to_string(),
        format!("StandardOutput=append:{crashes}"),
        format!("StandardError=append:{crashes}"),
        "WantedBy=default.target".to_string(),
    ] {
        assert!(text.lines().any(|l| l == line), "no {line}\nin:\n{text}");
    }
    assert_eq!(
        home.calls("systemctl"),
        [
            "is-system-running",
            "--user daemon-reload",
            "--user enable tooob@photos.service",
            "--user restart tooob@photos.service",
        ]
    );
    assert_crash_log_is_private(&crashes);
    assert!(
        home.row("photos").contains(" yes "),
        "{}",
        home.row("photos")
    );
}

#[test]
fn run_d_leaves_nothing_behind_when_the_service_manager_refuses() {
    for (tool, refuses) in [("launchctl", "bootstrap"), ("systemctl", "restart")] {
        let home = Home::new();
        let folder = a_folder();
        home.ok(&["add", "photos", folder.path().to_str().unwrap(), "--local"]);
        home.install_tooob();
        home.stand_in(
            tool,
            &format!("case \"$*\" in *{refuses}*) echo 'no session' >&2; exit 5;; esac"),
        );
        let out = home.tooob(&["run", "photos", "-d"]);
        assert!(!out.status.success(), "{tool}");
        assert!(
            stderr(&out).contains("no session"),
            "{tool}: {}",
            stderr(&out)
        );
        assert!(entries(&home).is_empty(), "{tool}: {:?}", entries(&home));
        assert!(home.row("photos").contains(" no "), "{tool}");
    }
}

#[test]
fn run_d_needs_tooob_where_the_install_script_puts_it_and_refuses_a_host_run_by_hand() {
    let home = Home::new();
    let folder = a_folder();
    home.ok(&["add", "photos", folder.path().to_str().unwrap(), "--local"]);
    home.stand_in("launchctl", "");

    let out = home.tooob(&["run", "photos", "-d"]);
    assert!(!out.status.success());
    let tooob = home.0.path().join(".local/bin/tooob");
    assert_eq!(
        stderr(&out),
        format!(
            "tooob: photos would start from {}, where the install script puts tooob, and it \
             isn't there: install tooob, or put this one there:\n  mkdir -p {} && cp {} {}\n",
            tooob.display(),
            tooob.parent().unwrap().display(),
            env!("CARGO_BIN_EXE_tooob"),
            tooob.display(),
        )
    );

    home.install_tooob();
    let running = home.start("photos");
    let out = home.tooob(&["run", "photos", "-d"]);
    assert!(!out.status.success());
    let said = stderr(&out);
    assert!(
        said.contains(&running.child.id().to_string()) && said.contains("tooob stop photos"),
        "{said}"
    );
    assert!(entries(&home).is_empty(), "{:?}", entries(&home));
    assert!(home.calls("launchctl").is_empty());
    assert!(home.row("photos").contains("running"));
}

/// The environment `photos`' entry gives it, as its service manager would.
fn entry_env(home: &Home) -> Vec<(String, String)> {
    let entry = &entries(home)[0];
    let text = std::fs::read_to_string(entry).unwrap();
    let mut env = Vec::new();
    for line in text.lines() {
        if let Some(pair) = line.strip_prefix("Environment=\"") {
            let (k, v) = pair.trim_end_matches('"').split_once('=').unwrap();
            env.push((k.to_string(), v.to_string()));
        }
    }
    if let Some(at) = text.find("<key>EnvironmentVariables</key>") {
        let dict = &text[at..text[at..].find("</dict>").unwrap() + at];
        let inner = |l: &str, tag: &str| {
            l.trim()
                .strip_prefix(&format!("<{tag}>"))
                .and_then(|l| l.strip_suffix(&format!("</{tag}>")))
                .map(String::from)
        };
        let lines: Vec<&str> = dict.lines().collect();
        for pair in lines.windows(2) {
            if let (Some(k), Some(v)) = (inner(pair[0], "key"), inner(pair[1], "string")) {
                env.push((k, v));
            }
        }
    }
    env
}

/// `tooob run photos` as its entry starts it, once its log says it's serving: it prints nothing.
fn run_as_its_entry(home: &Home) -> support::home::Running {
    let env = entry_env(home);
    let env: Vec<(&str, &str)> = env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let running = home.spawn_with(&["run", "photos"], &env);
    home.until_logged("photos", "photos: serving ");
    running
}

#[test]
fn stop_on_a_host_kept_running_stops_it_and_keeps_it_stopped_until_start() {
    for (tool, stops, calls) in [
        (
            "launchctl",
            format!("bootout gui/{}/net.tooobs.photos", uid()),
            vec![format!("bootout gui/{}/net.tooobs.photos", uid())],
        ),
        (
            "systemctl",
            "--user stop tooob@photos.service".to_string(),
            vec![
                "--user disable tooob@photos.service".to_string(),
                "--user stop tooob@photos.service".to_string(),
                "--user daemon-reload".to_string(),
            ],
        ),
    ] {
        let home = Home::new();
        let folder = a_folder();
        home.ok(&["add", "photos", folder.path().to_str().unwrap(), "--local"]);
        home.install_tooob();
        let pid = home.stand_ins().join("pid");
        home.stand_in(
            tool,
            &format!(
                "case \"$*\" in '{stops}') read pid < '{}'; kill -TERM $pid;; esac",
                pid.display()
            ),
        );
        home.ok(&["run", "photos", "-d"]);
        let kept = home.calls(tool).len();
        let running = run_as_its_entry(&home);
        std::fs::write(&pid, running.child.id().to_string()).unwrap();
        assert!(home.row("photos").contains("running"), "{tool}");

        let out = home.tooob(&["stop", "photos"]);
        assert!(out.status.success(), "{tool}: {}", stderr(&out));
        assert_eq!(String::from_utf8_lossy(&out.stdout), "", "{tool}");
        assert_eq!(
            stderr(&out),
            "tooob: photos is stopped, and stays stopped across logins and reboots: `tooob \
             start photos` starts it again\n",
            "{tool}"
        );
        assert_eq!(home.calls(tool)[kept..], calls, "{tool}");
        // Nothing its service manager loads at login or boot.
        assert!(entries(&home).is_empty(), "{tool}: {:?}", entries(&home));
        let row = home.row("photos");
        assert!(autostart_column(&row) == "stopped", "{tool}: {row}");
        assert_eq!(
            home.shown("photos", "Autostart"),
            "no, kept stopped: `tooob start photos` starts it again",
            "{tool}"
        );
        home.until_logged("photos", "photos: stopped");

        home.ok(&["run", "photos", "-d"]);
        assert!(
            home.shown("photos", "Autostart").starts_with("yes, "),
            "{tool}"
        );
        assert_eq!(entries(&home).len(), 1, "{tool}: kept running again");
        assert!(home.row("photos").contains(" yes "), "{tool}");
    }
}

#[test]
fn start_after_stop_runs_it_again_the_way_it_ran_through_its_service_manager_and_keeps_it_running()
{
    for (tool, calls) in [
        (
            "launchctl",
            vec![format!(
                "bootstrap gui/{} {}/Library/LaunchAgents/net.tooobs.photos.plist",
                uid(),
                "HOME"
            )],
        ),
        (
            "systemctl",
            vec![
                "is-system-running".to_string(),
                "--user daemon-reload".to_string(),
                "--user enable tooob@photos.service".to_string(),
                "--user restart tooob@photos.service".to_string(),
            ],
        ),
    ] {
        let home = Home::new();
        let calls: Vec<String> = calls
            .iter()
            .map(|c| c.replace("HOME", home.0.path().to_str().unwrap()))
            .collect();
        let folder = a_folder();
        home.ok(&["add", "photos", folder.path().to_str().unwrap(), "--local"]);
        home.install_tooob();
        home.stand_in(tool, "");
        // Kept running with the release check off, which `start` keeps though it isn't typed
        // with it.
        let out = home.tooob_with(&["run", "photos", "-d"], &[("TOOOB_NO_UPDATE_CHECK", "1")]);
        assert!(out.status.success(), "{tool}: {}", stderr(&out));
        let entry = entries(&home)[0].clone();
        let written = std::fs::read_to_string(&entry).unwrap();
        home.ok(&["stop", "photos"]);
        assert!(entries(&home).is_empty(), "{tool}");
        let before = home.calls(tool).len();

        let out = home.tooob(&["start", "photos"]);
        assert!(out.status.success(), "{tool}: {}", stderr(&out));
        assert_eq!(String::from_utf8_lossy(&out.stdout), "", "{tool}");
        assert_eq!(
            stderr(&out),
            "tooob: photos is kept running now, and after logins, reboots and crashes: `tooob \
             logs photos -f` follows it, and `tooob stop photos` stops it.\n",
            "{tool}"
        );
        assert_eq!(home.calls(tool)[before..], calls, "{tool}");
        assert_eq!(entries(&home), std::slice::from_ref(&entry), "{tool}");
        assert_eq!(std::fs::read_to_string(&entry).unwrap(), written, "{tool}");
        assert!(home.row("photos").contains(" yes "), "{tool}");
        assert!(
            home.shown("photos", "Autostart").starts_with("yes, "),
            "{tool}"
        );

        // And `stop` keeps it stopped again, as many times as it's asked.
        home.ok(&["stop", "photos"]);
        assert!(entries(&home).is_empty(), "{tool}");
        home.ok(&["start", "photos"]);
        assert_eq!(entries(&home), std::slice::from_ref(&entry), "{tool}");
    }
}

#[test]
fn start_on_a_host_only_ever_run_in_a_terminal_says_what_to_type_and_starts_nothing() {
    for tool in ["launchctl", "systemctl", "neither"] {
        let home = Home::new();
        let folder = a_folder();
        home.ok(&["add", "blog", folder.path().to_str().unwrap(), "--local"]);
        home.install_tooob();
        home.stand_in(tool, "");
        // Run in a terminal once, and stopped there.
        home.start("blog").stop();

        let out = home.tooob(&["start", "blog"]);
        assert!(!out.status.success(), "{tool}");
        assert_eq!(
            stderr(&out),
            "tooob: blog doesn't start by itself; serve it here with `tooob run blog`, or keep \
             it running with `tooob run blog -d`\n",
            "{tool}"
        );
        assert!(
            home.calls(tool).is_empty(),
            "{tool}: {:?}",
            home.calls(tool)
        );
        assert!(entries(&home).is_empty(), "{tool}: {:?}", entries(&home));
        let row = home.row("blog");
        assert!(row.contains("stopped") && row.contains(" no "), "{tool}");
    }
}

#[test]
fn start_with_no_host_name_lists_the_hosts_kept_stopped() {
    let home = Home::new();
    let fails = |args: &[&str]| {
        let out = home.tooob(args);
        assert!(!out.status.success(), "{args:?}");
        stderr(&out)
    };
    home.install_tooob();
    home.stand_in("launchctl", "");
    let folder = a_folder();
    for name in ["blog", "films", "photos", "notes"] {
        let path = folder.path().join(name);
        std::fs::create_dir(&path).unwrap();
        home.ok(&["add", name, path.to_str().unwrap(), "--local"]);
    }
    assert_eq!(
        fails(&["start"]),
        "tooob: say which Host to start (none is kept stopped):\n  tooob start NAME\n"
    );
    // blog is only ever run in a terminal; films and photos are kept stopped; notes is kept
    // running, and running.
    for name in ["films", "photos", "notes"] {
        home.ok(&["run", name, "-d"]);
    }
    home.ok(&["stop", "films"]);
    home.ok(&["stop", "photos"]);
    let notes = entry_env(&home);
    let notes: Vec<(&str, &str)> = notes
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    let _notes = home.spawn_with(&["run", "notes"], &notes);
    home.until_logged("notes", "notes: serving ");
    assert_eq!(
        fails(&["start"]),
        "tooob: say which Host to start (kept stopped: films, photos):\n  tooob start NAME\n"
    );
}

#[test]
fn start_on_a_running_host_leaves_it_as_it_is_and_says_so() {
    let home = Home::new();
    let folder = a_folder();
    home.ok(&["add", "photos", folder.path().to_str().unwrap(), "--local"]);
    home.install_tooob();
    home.stand_in("launchctl", "");

    // In a terminal.
    let running = home.start("photos");
    let out = home.tooob(&["start", "photos"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(
        stderr(&out),
        format!(
            "tooob: photos is already running here (pid {}), started by hand: Ctrl-C where it \
             runs, or `tooob stop photos`, stops it\n",
            running.child.id()
        )
    );
    running.stop();

    // Kept running.
    home.ok(&["run", "photos", "-d"]);
    let before = home.calls("launchctl");
    let _running = run_as_its_entry(&home);
    let out = home.tooob(&["start", "photos"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(
        stderr(&out),
        "tooob: photos is already running in the background: `tooob logs photos -f` follows \
         it, and `tooob stop photos` stops it\n"
    );
    assert_eq!(home.calls("launchctl"), before, "nothing restarted");
    assert!(home.row("photos").contains("running"));
    drop(_running);

    // Kept stopped, but run in a terminal since.
    home.ok(&["stop", "photos"]);
    let _running = home.start("photos");
    let before = home.calls("launchctl");
    let out = home.tooob(&["start", "photos"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(
        stderr(&out),
        "tooob: photos is already running, but it's kept stopped: `tooob stop photos` stops \
         it, then `tooob start photos` keeps it running\n"
    );
    assert_eq!(home.calls("launchctl"), before, "nothing started");
    assert!(entries(&home).is_empty(), "{:?}", entries(&home));
}

#[test]
fn start_on_a_host_kept_running_that_isnt_running_now_starts_it_afresh_from_its_entry() {
    // Its service manager gave up on it after its crashes, say.
    let home = Home::new();
    let folder = a_folder();
    home.ok(&["add", "photos", folder.path().to_str().unwrap(), "--local"]);
    home.install_tooob();
    home.stand_in("launchctl", "");
    home.ok(&["run", "photos", "-d"]);
    let agent = entries(&home)[0].clone();
    let written = std::fs::read_to_string(&agent).unwrap();
    let before = home.calls("launchctl").len();

    let out = home.tooob(&["start", "photos"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("is kept running now"),
        "{}",
        stderr(&out)
    );
    assert_eq!(
        home.calls("launchctl")[before..],
        [
            format!("bootout gui/{}/net.tooobs.photos", uid()),
            format!("bootstrap gui/{} {}", uid(), agent.display()),
        ]
    );
    assert_eq!(std::fs::read_to_string(&agent).unwrap(), written);
}

#[test]
fn a_start_its_service_manager_refuses_leaves_the_host_kept_stopped_and_says_so() {
    for (tool, refuses) in [("launchctl", "bootstrap"), ("systemctl", "restart")] {
        let home = Home::new();
        let folder = a_folder();
        home.ok(&["add", "photos", folder.path().to_str().unwrap(), "--local"]);
        home.install_tooob();
        home.stand_in(tool, "");
        home.ok(&["run", "photos", "-d"]);
        home.ok(&["stop", "photos"]);
        home.stand_in(
            tool,
            &format!("case \"$*\" in *{refuses}*) echo 'no session' >&2; exit 5;; esac"),
        );

        let out = home.tooob(&["start", "photos"]);
        assert!(!out.status.success(), "{tool}");
        let said = stderr(&out);
        assert!(
            said.contains("photos is still stopped") && said.contains("no session"),
            "{tool}: {said}"
        );
        assert!(entries(&home).is_empty(), "{tool}: {:?}", entries(&home));
        assert!(autostart_column(&home.row("photos")) == "stopped", "{tool}");

        // It can still be started once its service manager will.
        home.stand_in(tool, "");
        home.ok(&["start", "photos"]);
        assert_eq!(entries(&home).len(), 1, "{tool}");
    }
}

/// The AUTOSTART column of a row `tooob ls` prints.
fn autostart_column(row: &str) -> &str {
    row.split_whitespace().nth(2).unwrap()
}

#[test]
fn ls_shows_autostart_yes_for_kept_running_stopped_for_kept_stopped_and_no_otherwise() {
    let home = Home::new();
    home.install_tooob();
    home.stand_in("launchctl", "");
    let folder = a_folder();
    for name in ["blog", "photos", "notes"] {
        let path = folder.path().join(name);
        std::fs::create_dir(&path).unwrap();
        home.ok(&["add", name, path.to_str().unwrap(), "--local"]);
    }
    home.ok(&["run", "photos", "-d"]);
    home.ok(&["run", "notes", "-d"]);
    home.ok(&["stop", "photos"]);

    for (name, shown, kept_stopped) in [
        ("blog", "no", false),
        ("photos", "stopped", true),
        ("notes", "yes", false),
    ] {
        assert_eq!(autostart_column(&home.row(name)), shown, "{name}");
        let json: serde_json::Value = serde_json::from_str(&home.ok(&["ls", "--json"])).unwrap();
        let row = json
            .as_array()
            .unwrap()
            .iter()
            .find(|h| h["name"] == name)
            .unwrap()
            .clone();
        assert_eq!(row["kept_stopped"], kept_stopped, "{name}");
        assert_eq!(row["autostart"], shown == "yes", "{name}");
    }
}

#[test]
fn start_writes_its_entry_afresh_for_where_the_host_is_now() {
    // Its Hosts' folder moved between `stop` and `start`: the entry put by names the old one.
    for tool in ["launchctl", "systemctl"] {
        let home = Home::new();
        home.install_tooob();
        home.stand_in(tool, "");
        let folder = a_folder();
        let old = home.0.path().join("old");
        let new = home.0.path().join("new");
        let run = |args: &[&str], dir: &std::path::Path, env: &[(&str, &str)]| {
            let env: Vec<(&str, &str)> = [("TOOOB_HOME", dir.to_str().unwrap())]
                .into_iter()
                .chain(env.iter().copied())
                .collect();
            let out = home.tooob_with(args, &env);
            assert!(out.status.success(), "{tool} {args:?}: {}", stderr(&out));
        };
        run(
            &["add", "photos", folder.path().to_str().unwrap(), "--local"],
            &old,
            &[],
        );
        run(
            &["run", "photos", "-d"],
            &old,
            &[("TOOOB_NO_UPDATE_CHECK", "1")],
        );
        run(&["stop", "photos"], &old, &[]);
        std::fs::rename(&old, &new).unwrap();

        run(&["start", "photos"], &new, &[]);
        let text = std::fs::read_to_string(&entries(&home)[0]).unwrap();
        assert!(
            !text.contains(old.to_str().unwrap()),
            "{tool}: names the old folder:\n{text}"
        );
        let env = entry_env(&home);
        for (k, v) in [
            ("TOOOB_HOME", new.to_str().unwrap()),
            // What `-d` was typed with that the Host as found now can't say.
            ("TOOOB_NO_UPDATE_CHECK", "1"),
        ] {
            assert!(
                env.contains(&(k.to_string(), v.to_string())),
                "{tool}: no {k}={v} in {env:?}"
            );
        }
        let put_by: Vec<_> = walkdir(&new)
            .into_iter()
            .filter(|f| {
                let f = f.to_string_lossy();
                f.ends_with(".stopped") || f.ends_with(".stopping")
            })
            .collect();
        assert!(put_by.is_empty(), "{tool}: {put_by:?}");
    }
}

/// Every file under `dir`.
fn walkdir(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    for e in std::fs::read_dir(dir).unwrap() {
        let path = e.unwrap().path();
        match path.is_dir() {
            true => files.extend(walkdir(&path)),
            false => files.push(path),
        }
    }
    files
}

#[test]
fn start_after_a_stop_cut_short_unloads_what_launchd_may_still_have_loaded_first() {
    let home = Home::new();
    let folder = a_folder();
    home.ok(&["add", "photos", folder.path().to_str().unwrap(), "--local"]);
    home.install_tooob();
    // The stop is cut short as it asks launchd to unload the agent, as a Ctrl-C would.
    let cut = home.stand_ins().join("cut");
    home.stand_in(
        "launchctl",
        &format!(
            "case \"$*\" in bootout*) if [ -e '{0}' ]; then /bin/rm '{0}'; kill -9 $PPID; fi;; \
             esac",
            cut.display()
        ),
    );
    home.ok(&["run", "photos", "-d"]);
    let agent = entries(&home)[0].clone();
    std::fs::write(&cut, "").unwrap();
    assert!(!home.tooob(&["stop", "photos"]).status.success());
    assert!(entries(&home).is_empty(), "{:?}", entries(&home));
    let before = home.calls("launchctl").len();

    home.ok(&["start", "photos"]);
    assert_eq!(
        home.calls("launchctl")[before..],
        [
            format!("bootout gui/{}/net.tooobs.photos", uid()),
            format!("bootstrap gui/{} {}", uid(), agent.display()),
        ]
    );
    assert_eq!(entries(&home), [agent]);
}

#[test]
fn a_stop_cut_short_leaves_nothing_to_start_at_login_and_stop_again_finishes_it() {
    for stops in [
        (
            "launchctl",
            format!("bootout gui/{}/net.tooobs.photos", uid()),
        ),
        ("systemctl", "--user stop tooob@photos.service".to_string()),
    ] {
        let (tool, stops) = stops;
        let home = Home::new();
        let folder = a_folder();
        home.ok(&["add", "photos", folder.path().to_str().unwrap(), "--local"]);
        home.install_tooob();
        let pid = home.stand_ins().join("pid");
        // The first stop is cut short just as it asks the service manager to stop the Host, as
        // a Ctrl-C would: the stand-in kills `tooob stop` itself.
        let cut = home.stand_ins().join("cut");
        home.stand_in(
            tool,
            &format!(
                "case \"$*\" in '{stops}') if [ -e '{cut}' ]; then /bin/rm '{cut}'; kill -9 $PPID; \
                 else read pid < '{pid}'; kill -TERM $pid; fi;; esac",
                cut = cut.display(),
                pid = pid.display()
            ),
        );
        home.ok(&["run", "photos", "-d"]);
        let running = run_as_its_entry(&home);
        std::fs::write(&pid, running.child.id().to_string()).unwrap();
        std::fs::write(&cut, "").unwrap();

        let out = home.tooob(&["stop", "photos"]);
        assert!(!out.status.success(), "{tool}: cut short");
        assert!(home.row("photos").contains("running"), "{tool}");
        assert!(
            entries(&home).is_empty(),
            "{tool}: nothing to start at login: {:?}",
            entries(&home)
        );

        let out = home.tooob(&["stop", "photos"]);
        assert!(out.status.success(), "{tool}: {}", stderr(&out));
        assert!(
            stderr(&out).contains("stays stopped"),
            "{tool}: {}",
            stderr(&out)
        );
        let row = home.row("photos");
        assert!(autostart_column(&row) == "stopped", "{tool}: {row}");
        home.ok(&["run", "photos", "-d"]);
        assert_eq!(entries(&home).len(), 1, "{tool}");
    }
}

#[test]
fn a_stop_its_service_manager_refuses_leaves_the_host_kept_running_and_says_so() {
    let home = Home::new();
    let folder = a_folder();
    home.ok(&["add", "photos", folder.path().to_str().unwrap(), "--local"]);
    home.install_tooob();
    home.stand_in(
        "launchctl",
        "case \"$*\" in bootout*) echo 'Boot-out failed: 1: Operation not permitted' >&2; exit 1;; \
         esac",
    );
    home.ok(&["run", "photos", "-d"]);
    let _running = run_as_its_entry(&home);

    let out = home.tooob(&["stop", "photos"]);
    assert!(!out.status.success());
    let said = stderr(&out);
    assert!(
        said.contains("photos didn't stop, so it's still kept running")
            && said.contains("Operation not permitted"),
        "{said}"
    );
    assert_eq!(entries(&home).len(), 1, "its entry is back");
    let row = home.row("photos");
    assert!(row.contains("running") && row.contains(" yes "), "{row}");
}

#[test]
fn stop_on_a_host_kept_running_that_isnt_running_now_keeps_it_stopped_too() {
    // Between its crashes, say, or kept running from outside a login session: launchd hasn't
    // loaded it, and says so.
    let home = Home::new();
    let folder = a_folder();
    home.ok(&["add", "photos", folder.path().to_str().unwrap(), "--local"]);
    home.install_tooob();
    home.stand_in(
        "launchctl",
        "case \"$*\" in bootout*) echo 'Boot-out failed: 3: No such process' >&2; exit 3;; esac",
    );
    home.ok(&["run", "photos", "-d"]);

    let out = home.tooob(&["stop", "photos"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stderr(&out).contains("stays stopped"), "{}", stderr(&out));
    assert!(entries(&home).is_empty(), "{:?}", entries(&home));
    assert!(autostart_column(&home.row("photos")) == "stopped");
}

#[test]
fn run_and_share_on_a_host_kept_running_never_start_a_second_copy_and_share_adds_its_invites() {
    let home = Home::new();
    let folder = a_folder();
    let path = folder.path().to_str().unwrap();
    let relay = TestRelay::start();
    home.ok(&["add", "photos", path, "--relay", &relay.url]);
    home.install_tooob();
    home.stand_in("launchctl", "");
    home.ok(&["run", "photos", "-d"]);
    let calls = home.calls("launchctl");
    let _running = run_as_its_entry(&home);

    let out = home.tooob(&["run", "photos"]);
    assert!(!out.status.success());
    assert_eq!(
        stderr(&out),
        "tooob: photos is kept running in the background, so its service manager runs it: \
         `tooob logs photos -f` follows it, and `tooob stop photos` stops it\n"
    );

    for (args, invited) in [
        (vec!["share", path, "--invite", "Ann"], "Ann"),
        (vec!["share", path, "--invite", "Bea", "-d"], "Bea"),
    ] {
        let out = home.tooob(&args);
        assert!(out.status.success(), "{args:?}: {}", stderr(&out));
        let said = stderr(&out);
        assert!(
            said.contains(&format!("\n    {invited} · works for 7 days\n"))
                && said.ends_with(
                    "tooob: photos is already running in the background: `tooob logs photos -f` \
                     follows it, and `tooob stop photos` stops it\n"
                ),
            "{args:?}: {said}"
        );
        assert!(
            String::from_utf8_lossy(&out.stdout).contains("/#invite="),
            "{args:?}"
        );
        assert!(
            home.ok(&["invites", "photos"]).contains(invited),
            "{args:?}"
        );
    }
    assert_eq!(
        home.calls("launchctl"),
        calls,
        "nothing started or restarted"
    );
    assert!(home.row("photos").contains("running"));
}

#[test]
fn share_d_adds_a_host_invites_and_keeps_it_running_and_again_once_its_stopped() {
    let home = Home::new();
    let folder = a_folder();
    let path = folder.path().to_str().unwrap();
    let relay = TestRelay::start();
    home.install_tooob();
    home.stand_in("launchctl", "");

    let out = home.tooob(&[
        "share", path, "--relay", &relay.url, "--invite", "Ann", "-d",
    ]);
    assert!(out.status.success(), "{}", stderr(&out));
    let said = stderr(&out);
    assert!(
        said.contains("\n    Ann · works for 7 days\n") && said.contains("is kept running now"),
        "{said}"
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("/#invite="));
    let name = home
        .ok(&["ls"])
        .lines()
        .nth(1)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_string();
    assert!(home.ok(&["invites", &name]).contains("Ann"));
    assert_eq!(entries(&home).len(), 1, "{:?}", entries(&home));
    let load = format!("bootstrap gui/{} {}", uid(), entries(&home)[0].display());
    assert_eq!(home.calls("launchctl"), std::slice::from_ref(&load));

    home.ok(&["stop", &name]);
    let out = home.tooob(&["share", path, "--invite", "Cy", "-d"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(home.ok(&["invites", &name]).contains("Cy"));
    assert_eq!(entries(&home).len(), 1, "kept running again");
    assert_eq!(home.calls("launchctl").last(), Some(&load));
}

#[test]
fn share_d_its_service_manager_refuses_adds_no_host_and_no_invites() {
    let home = Home::new();
    let relay = TestRelay::start();
    home.install_tooob();
    home.stand_in(
        "launchctl",
        "case \"$*\" in bootstrap*) echo 'no session' >&2; exit 5;; esac",
    );
    let new = a_folder();
    let out = home.tooob(&[
        "share",
        new.path().to_str().unwrap(),
        "--relay",
        &relay.url,
        "--invite",
        "Ann",
        "-d",
    ]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("no session"), "{}", stderr(&out));
    assert_eq!(home.ok(&["ls"]).lines().count(), 1, "no Host was added");
    assert!(!String::from_utf8_lossy(&out.stdout).contains("#invite="));

    let reused = a_folder();
    let path = reused.path().to_str().unwrap();
    home.ok(&["add", "photos", path, "--relay", &relay.url]);
    let out = home.tooob(&["share", path, "--invite", "Bea", "-d"]);
    assert!(!out.status.success());
    assert!(
        home.ok(&["invites", "photos"]).is_empty(),
        "no Invite was made"
    );
    assert!(entries(&home).is_empty(), "{:?}", entries(&home));
}

#[test]
fn share_d_that_cant_keep_a_host_running_adds_nothing() {
    let home = Home::new();
    let folder = a_folder();
    let relay = TestRelay::start();
    home.stand_in("launchctl", "");
    let path = folder.path().to_str().unwrap();
    let out = home.tooob(&[
        "share", path, "--relay", &relay.url, "--invite", "Ann", "-d",
    ]);
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("where the install script puts tooob"),
        "{}",
        stderr(&out)
    );
    assert_eq!(home.ok(&["ls"]).lines().count(), 1, "nothing was added");
    assert!(home.calls("launchctl").is_empty());
}

#[test]
fn rm_on_a_host_kept_running_stops_it_removes_its_entry_and_deletes_it() {
    for (tool, stops, calls) in [
        (
            "launchctl",
            format!("bootout gui/{}/net.tooobs.photos", uid()),
            vec![format!("bootout gui/{}/net.tooobs.photos", uid())],
        ),
        (
            "systemctl",
            "--user disable --now tooob@photos.service".to_string(),
            vec![
                "--user disable --now tooob@photos.service".to_string(),
                "--user daemon-reload".to_string(),
            ],
        ),
    ] {
        let home = Home::new();
        let folder = a_folder();
        home.ok(&["add", "photos", folder.path().to_str().unwrap(), "--local"]);
        home.install_tooob();
        let pid = home.stand_ins().join("pid");
        home.stand_in(
            tool,
            &format!(
                "case \"$*\" in '{stops}') read pid < '{}'; kill -TERM $pid;; esac",
                pid.display()
            ),
        );
        home.ok(&["run", "photos", "-d"]);
        let kept = home.calls(tool).len();
        let state = home.shown("photos", "state folder");
        let running = run_as_its_entry(&home);
        std::fs::write(&pid, running.child.id().to_string()).unwrap();

        let out = home.tooob(&["rm", "photos"]);
        assert!(out.status.success(), "{tool}: {}", stderr(&out));
        assert_eq!(String::from_utf8_lossy(&out.stdout), "", "{tool}");
        assert_eq!(home.calls(tool)[kept..], calls, "{tool}");
        assert!(entries(&home).is_empty(), "{tool}: {:?}", entries(&home));
        assert!(!home.ok(&["ls"]).contains("photos"), "{tool}");
        assert!(!std::path::Path::new(&state).exists(), "{tool}");
    }
}

#[test]
fn a_host_kept_running_by_0_2_0s_enable_stops_runs_d_and_rms_with_the_new_commands() {
    let home = Home::new();
    let folder = a_folder();
    home.ok(&["add", "photos", folder.path().to_str().unwrap(), "--local"]);
    home.install_tooob();
    let pid = home.stand_ins().join("pid");
    let unload = format!("bootout gui/{}/net.tooobs.photos", uid());
    home.stand_in(
        "launchctl",
        &format!(
            "case \"$*\" in '{unload}') read pid < '{}'; kill -TERM $pid;; esac",
            pid.display()
        ),
    );
    // The agent as 0.2.0's `tooob enable` wrote it: its output to host.log, and a comment
    // naming `tooob disable`.
    home.ok(&["run", "photos", "-d"]);
    let agent = entries(&home)[0].clone();
    let crashes = crash_log(&home);
    let log = format!("{}/host.log", home.shown("photos", "state folder"));
    let new = std::fs::read_to_string(&agent).unwrap();
    let old = new.replace(&crashes, &log).replace(
        "`tooob stop` stops it, and `tooob rm` removes it",
        "`tooob disable` removes it",
    );
    assert_ne!(old, new);
    std::fs::write(&agent, &old).unwrap();
    let running = run_as_its_entry(&home);
    std::fs::write(&pid, running.child.id().to_string()).unwrap();
    let calls = home.calls("launchctl").len();

    home.ok(&["stop", "photos"]);
    assert_eq!(home.calls("launchctl")[calls..], [unload.as_str()]);
    assert!(entries(&home).is_empty(), "{:?}", entries(&home));
    assert!(autostart_column(&home.row("photos")) == "stopped");

    home.ok(&["run", "photos", "-d"]);
    assert_eq!(
        std::fs::read_to_string(&agent).unwrap(),
        new,
        "written afresh, as this tooob writes it"
    );
    let running = run_as_its_entry(&home);
    std::fs::write(&pid, running.child.id().to_string()).unwrap();
    home.ok(&["rm", "photos"]);
    assert_eq!(home.calls("launchctl").last(), Some(&unload));
    assert!(entries(&home).is_empty(), "{:?}", entries(&home));
    assert!(!home.ok(&["ls"]).contains("photos"));
}

#[test]
fn rm_on_a_host_stop_keeps_stopped_deletes_it_and_what_was_put_by() {
    let home = Home::new();
    let folder = a_folder();
    home.ok(&["add", "photos", folder.path().to_str().unwrap(), "--local"]);
    home.install_tooob();
    home.stand_in("launchctl", "");
    home.ok(&["run", "photos", "-d"]);
    home.ok(&["stop", "photos"]);
    let state = home.shown("photos", "state folder");
    let calls = home.calls("launchctl").len();

    home.ok(&["rm", "photos"]);
    assert_eq!(home.calls("launchctl").len(), calls, "nothing to unload");
    assert!(!std::path::Path::new(&state).exists());
    assert!(entries(&home).is_empty(), "{:?}", entries(&home));
}

#[test]
fn run_d_on_a_host_kept_running_says_so_while_it_runs_and_starts_it_afresh_when_it_doesnt() {
    let home = Home::new();
    let folder = a_folder();
    home.ok(&["add", "photos", folder.path().to_str().unwrap(), "--local"]);
    home.install_tooob();
    let pid = home.stand_ins().join("pid");
    let unload = format!("bootout gui/{}/net.tooobs.photos", uid());
    home.stand_in(
        "launchctl",
        &format!(
            "case \"$*\" in '{unload}') read pid < '{}'; kill -TERM $pid;; esac",
            pid.display()
        ),
    );
    home.ok(&["run", "photos", "-d"]);
    let agent = entries(&home)[0].clone();
    let load = format!("bootstrap gui/{} {}", uid(), agent.display());
    let running = run_as_its_entry(&home);
    std::fs::write(&pid, running.child.id().to_string()).unwrap();

    let out = home.tooob(&["run", "photos", "-d"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(
        stderr(&out),
        "tooob: photos is already running in the background: `tooob logs photos -f` follows \
         it, and `tooob stop photos` stops it\n"
    );
    assert_eq!(
        home.calls("launchctl"),
        std::slice::from_ref(&load),
        "nothing restarted"
    );
    assert!(home.row("photos").contains("running"));

    // Its service manager gave up on it, say: started afresh, from its entry rewritten.
    drop(running);
    while home.row("photos").contains("running") {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let out = home.tooob(&["run", "photos", "-d"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("is kept running now"),
        "{}",
        stderr(&out)
    );
    assert_eq!(home.calls("launchctl"), [load.clone(), unload, load]);
}

#[test]
fn logs_with_no_host_name_lists_the_hosts() {
    let home = Home::new();
    let folder = a_folder();
    home.ok(&["add", "photos", folder.path().to_str().unwrap(), "--local"]);
    home.ok(&["add", "films", "127.0.0.1:8096", "--local"]);
    let out = home.tooob(&["logs"]);
    assert_eq!(
        stderr(&out),
        "tooob: say which Host's log (Hosts here: films, photos):\n  tooob logs NAME\n"
    );
}

/// The power assertions process `pid` holds, as `pmset` lists them.
#[cfg(target_os = "macos")]
fn assertions_of(pid: u32) -> Vec<String> {
    let out = std::process::Command::new("/usr/bin/pmset")
        .args(["-g", "assertions"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.trim_start().starts_with(&format!("pid {pid}(")))
        .map(String::from)
        .collect()
}

#[cfg(target_os = "macos")]
#[test]
fn a_host_keeps_the_mac_from_idle_sleeping_while_it_serves_and_lets_it_once_it_stops() {
    let home = Home::new();
    let folder = a_folder();
    home.ok(&["add", "photos", folder.path().to_str().unwrap(), "--local"]);
    let running = home.start("photos");
    let pid = running.child.id();
    let held = assertions_of(pid);
    assert!(
        held.len() == 1
            && held[0].contains("PreventUserIdleSystemSleep")
            && held[0].contains("named: \"tooob: photos is serving\""),
        "{held:?}"
    );
    assert!(
        running.log().contains("closing its lid"),
        "{}",
        running.log()
    );

    home.ok(&["stop", "photos"]);
    running.until_it_says("photos: stopped");
    assert_eq!(assertions_of(pid), Vec::<String>::new());
}

#[test]
fn run_d_with_systemctl_but_no_systemd_running_says_what_to_have_run_and_writes_nothing() {
    // A container or WSL: systemctl is there, but systemd isn't what booted the box.
    let home = Home::new();
    let folder = a_folder();
    home.ok(&["add", "blog", folder.path().to_str().unwrap(), "--local"]);
    home.install_tooob();
    home.stand_in(
        "systemctl",
        "case \"$*\" in is-system-running) echo offline; exit 1;; esac",
    );

    let out = home.tooob(&["run", "blog", "-d"]);
    assert!(!out.status.success());
    assert_eq!(
        stderr(&out),
        "tooob: keeping blog running needs systemd or launchd, and this system has neither.\n  \
         Have your service manager run `~/.local/bin/tooob run blog` as you, restarting it on \
         failure.\n"
    );
    assert_eq!(home.calls("systemctl"), ["is-system-running"]);
    assert!(entries(&home).is_empty(), "{:?}", entries(&home));
}

#[test]
fn run_d_makes_a_launch_agent_for_any_host_name() {
    let home = Home::new();
    home.install_tooob();
    home.stand_in("launchctl", "");
    for name in ["a--b", "b-", "-c", "_"] {
        home.ok(&["add", name, "127.0.0.1:8096", "--local"]);
        home.ok(&["run", name, "-d"]);
        let agent = home
            .0
            .path()
            .join(format!("Library/LaunchAgents/net.tooobs.{name}.plist"));
        let plist = std::fs::read_to_string(&agent).unwrap();
        let comments: String = plist
            .split("<!--")
            .skip(1)
            .map(|c| c.split("-->").next().unwrap())
            .collect();
        assert!(!comments.contains("--"), "{name}: {plist}");
        if cfg!(target_os = "macos") {
            let lint = std::process::Command::new("/usr/bin/plutil")
                .arg("-lint")
                .arg(&agent)
                .output()
                .unwrap();
            assert!(lint.status.success(), "{name}: {lint:?}");
        }
    }
}

/// Devices of the stopped Host `name`, which shares `folder`, that each used an Invite for `label`
/// with First-Visit Approval while it ran for a moment: their Visitor Keys. Each waits.
async fn waiting_visitors(
    home: &Home,
    name: &str,
    folder: &std::path::Path,
    label: &str,
    keys: &[[u8; 32]],
) -> Vec<String> {
    let host = Host::start(HostConfig {
        site: Site::SharedFolder(folder.into()),
        data_dir: home.shown(name, "state folder").into(),
        network: Network::LocalOnly,
    })
    .await
    .unwrap();
    for key in keys {
        let link = home.ok(&["invite", name, label, "--approve"]);
        let token = link.trim().rsplit("#invite=").next().unwrap().to_string();
        let visit = Visit::connect(host.address(), *key, Network::LocalOnly)
            .await
            .unwrap();
        visit.redeem(&token).await.unwrap();
    }
    host.shutdown().await;
    keys.iter()
        .map(|k| iroh::SecretKey::from_bytes(k).public().to_string())
        .collect()
}

fn new_key() -> [u8; 32] {
    let mut key = [0u8; 32];
    getrandom::fill(&mut key).unwrap();
    key
}

/// `tooob visitors NAME`'s row for the Visitor whose Visitor Key is `key`, its columns split.
fn visitor_row(home: &Home, name: &str, key: &str) -> Vec<String> {
    let shown = home.ok(&["visitors", name]);
    let row = shown
        .lines()
        .find(|l| l.starts_with(&key[..12]))
        .unwrap_or_else(|| panic!("{key} isn't listed:\n{shown}"));
    row.split("  ")
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .map(String::from)
        .collect()
}

/// What `tooob visitors NAME --json` says of the Visitor whose Visitor Key is `key`.
fn visitor_listed(home: &Home, name: &str, key: &str) -> serde_json::Value {
    let visitors: serde_json::Value =
        serde_json::from_str(&home.ok(&["visitors", name, "--json"])).unwrap();
    visitors
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["key"] == key)
        .unwrap()
        .clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_waiting_device_is_approved_by_its_device_code_spaced_or_not() {
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
    let keys = [new_key(), new_key(), new_key()];
    let waiting = waiting_visitors(&home, "photos", folder.path(), "aunt", &keys).await;
    let codes: Vec<String> = waiting
        .iter()
        .map(|k| tooob::access::device_code(k))
        .collect();

    let header = home.ok(&["visitors", "photos"]);
    let header: Vec<&str> = header
        .lines()
        .next()
        .unwrap()
        .split("  ")
        .map(str::trim)
        .collect();
    let header: Vec<&str> = header.into_iter().filter(|c| !c.is_empty()).collect();
    assert_eq!(
        header,
        [
            "KEY",
            "LABEL",
            "STATUS",
            "CODE",
            "LAST SEEN",
            "SENT",
            "RECEIVED"
        ]
    );
    let row = visitor_row(&home, "photos", &waiting[0]);
    assert_eq!(
        row[1..4],
        ["aunt", "waiting for approval", codes[0].as_str()],
        "{row:?}"
    );
    assert_eq!(
        visitor_listed(&home, "photos", &waiting[0])["device_code"],
        codes[0].as_str()
    );

    // As shown, in one word ('482 917'), in two (482 917), and without the space (482917).
    let (first, second) = codes[1].split_once(' ').unwrap();
    let joined = codes[2].replace(' ', "");
    for approve in [
        vec!["approve", "photos", &codes[0]],
        vec!["approve", "photos", first, second],
        vec!["approve", "photos", &joined],
    ] {
        let out = home.tooob(&approve);
        assert!(out.status.success(), "{approve:?}: {}", stderr(&out));
        assert_eq!(String::from_utf8_lossy(&out.stdout), "", "{approve:?}");
        assert_eq!(stderr(&out), "", "{approve:?}");
    }
    for key in &waiting {
        assert_eq!(visitor_listed(&home, "photos", key)["status"], "Allowed");
        assert_eq!(
            visitor_listed(&home, "photos", key)["device_code"],
            serde_json::Value::Null
        );
        let row = visitor_row(&home, "photos", key);
        assert_eq!(row[1..4], ["aunt", "allowed", "-"], "in: no code; {row:?}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn approving_a_device_code_no_waiting_device_has_says_so_and_approves_nothing() {
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
    let alice = a_visitor(&home, "photos", folder.path()).await;
    let waiting = waiting_visitors(&home, "photos", folder.path(), "aunt", &[new_key()]).await;
    let aunt = tooob::access::device_code(&waiting[0]);
    let _running = home.start("photos");

    // A code nobody has, and the code of a device that's already in: neither is waiting.
    let nobodys = if aunt == "000 000" {
        "000 001"
    } else {
        "000 000"
    };
    for code in [nobodys.to_string(), tooob::access::device_code(&alice)] {
        let out = home.tooob(&["approve", "photos", &code]);
        assert!(!out.status.success(), "{code}");
        assert_eq!(String::from_utf8_lossy(&out.stdout), "");
        assert!(
            stderr(&out).contains(&format!(
                "no device waiting for approval has Device Code {code}"
            )) && stderr(&out).contains("give 7 characters of it or more"),
            "{}",
            stderr(&out)
        );
    }
    assert_eq!(
        visitor_listed(&home, "photos", &waiting[0])["status"],
        "AwaitingApproval",
        "approved nothing"
    );
    // A key prefix still works.
    home.ok(&["approve", "photos", &waiting[0][..8]]);
    assert_eq!(
        visitor_listed(&home, "photos", &waiting[0])["status"],
        "Allowed"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn two_waiting_devices_with_one_device_code_are_approved_only_by_key() {
    // One in a million each, so a few thousand keys have two that share a code.
    let mut by_code = std::collections::HashMap::new();
    let twins = loop {
        let key = new_key();
        let public = iroh::SecretKey::from_bytes(&key).public().to_string();
        if let Some(twin) = by_code.insert(tooob::access::device_code(&public), key) {
            break [twin, key];
        }
    };
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
    let waiting = waiting_visitors(&home, "photos", folder.path(), "aunt", &twins).await;
    let code = tooob::access::device_code(&waiting[0]);

    let out = home.tooob(&["approve", "photos", &code]);
    assert!(!out.status.success());
    let said = stderr(&out);
    assert!(
        said.contains(&format!(
            "2 devices waiting for approval have Device Code {code}"
        )) && said.contains(&waiting[0])
            && said.contains(&waiting[1]),
        "{said}"
    );
    for key in &waiting {
        assert_eq!(
            visitor_listed(&home, "photos", key)["status"],
            "AwaitingApproval"
        );
    }
}
