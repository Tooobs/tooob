//! Upgrade (ADR 0024): `tooob upgrade` replaces the `tooob` command with another Release, only
//! when the Owner asks, through that Release's own installer; and running Hosts say when a newer
//! one is out. Run as a script would (no terminal), against a stand-in GitHub, stand-in
//! installers, stand-in Relays and stand-in service managers: never GitHub, the real launchd or
//! systemd, or the Owner's own `~/.local/bin/tooob`. The question itself is in tests/terminal.rs.
mod support;

use std::process::Output;
use support::github::TestGitHub;
use support::home::{Home, TestRelay, a_folder, version_json};

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// This build's Release.
const THIS: &str = env!("CARGO_PKG_VERSION");

/// A Release `n` minor Releases after this one: `0.1.0` → `0.2.0` for 1.
fn newer(n: u32) -> String {
    let mut parts = THIS.split(['.', '-']);
    let major: u32 = parts.next().unwrap().parse().unwrap();
    let minor: u32 = parts.next().unwrap().parse().unwrap();
    format!("{major}.{}.0", minor + n)
}

/// Where a Release's notes are on GitHub.
fn notes(release: &str) -> String {
    format!("https://github.com/Tooobs/tooob/releases/tag/v{release}")
}

/// The Releases each stand-in installer that ran installed, in order.
fn installed(home: &Home) -> Vec<String> {
    std::fs::read_to_string(home.0.path().join("installed"))
        .unwrap_or_default()
        .lines()
        .map(String::from)
        .collect()
}

/// A Home whose `tooob` finds its Releases at a stand-in GitHub, which publishes this Release,
/// then a newer one, then a test Release of the one after.
fn with_releases() -> (Home, TestGitHub) {
    let home = Home::new();
    let github = TestGitHub::start();
    github.publish(THIS, Some(tooob::PROTOCOL_VERSION));
    github.publish(&newer(1), Some(tooob::PROTOCOL_VERSION));
    github.publish(&format!("{}-rc.1", newer(2)), None);
    home.releases_at(&github.url);
    (home, github)
}

#[test]
fn check_shows_the_newest_final_release_and_its_notes_and_installs_nothing() {
    let (home, _github) = with_releases();
    home.install_tooob();

    let out = home.tooob(&["upgrade", "--check"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let said = stdout(&out);
    assert_eq!(
        said,
        format!(
            "tooob {} is out (this is {THIS}): {}\n",
            newer(1),
            notes(&newer(1))
        )
    );
    assert!(!said.contains(&newer(2)), "a test Release is never offered");
    assert!(installed(&home).is_empty());
}

/// What `~/.local/bin/tooob --version` says.
fn installed_version(home: &Home) -> String {
    let out = std::process::Command::new(home.0.path().join(".local/bin/tooob"))
        .arg("--version")
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[test]
fn yes_installs_the_newest_final_release_through_its_own_installer_fetched_by_its_tag() {
    let (home, github) = with_releases();
    home.install_tooob();

    let out = home.tooob(&["upgrade", "--yes"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(installed(&home), [newer(1)]);
    let tag = format!("v{}", newer(1));
    assert!(
        github.asked().contains(&format!(
            "/Tooobs/tooob/releases/download/{tag}/tooob-installer.sh"
        )),
        "{:?}",
        github.asked()
    );
    assert_eq!(
        installed_version(&home),
        format!("tooob {} ({})", newer(1), tooob::PROTOCOL_VERSION)
    );
    assert!(
        stderr(&out).contains(&format!("tooob {} is installed", newer(1))),
        "{}",
        stderr(&out)
    );
}

#[test]
fn without_a_terminal_or_yes_it_refuses_at_once_with_the_command_to_type() {
    let (home, _github) = with_releases();
    home.install_tooob();

    for (args, typed) in [
        (vec!["upgrade"], "tooob upgrade --yes"),
        (vec!["upgrade", "--anyway"], "tooob upgrade --yes --anyway"),
    ] {
        let out = home.tooob(&args);
        assert!(!out.status.success(), "{args:?}");
        assert_eq!(
            stderr(&out),
            format!(
                "tooob: there's no terminal here to ask in, so say --yes to install tooob {}:\n  \
                 {typed}\n",
                newer(1)
            ),
            "{args:?}"
        );
        assert!(installed(&home).is_empty(), "{args:?}");
    }
}

/// A Release before this one: `0.1.0` → `0.0.9`.
fn older() -> String {
    let mut parts = THIS.split(['.', '-']);
    let major: u32 = parts.next().unwrap().parse().unwrap();
    let minor: u32 = parts.next().unwrap().parse().unwrap();
    format!("{major}.{}.9", minor - 1)
}

#[test]
fn upgrade_version_installs_that_exact_release_an_older_one_or_a_test_release_too() {
    let (home, github) = with_releases();
    github.publish(&older(), Some(tooob::PROTOCOL_VERSION));
    home.install_tooob();
    let rc = format!("{}-rc.1", newer(2));

    for (wanted, release) in [
        (older(), older()),
        (format!("v{}", newer(1)), newer(1)),
        (rc.clone(), rc.clone()),
    ] {
        std::fs::remove_file(home.0.path().join(".local/bin/tooob")).unwrap();
        home.install_tooob();
        let out = home.tooob(&["upgrade", &wanted, "--yes"]);
        assert!(out.status.success(), "{wanted}: {}", stderr(&out));
        assert_eq!(installed(&home).last(), Some(&release), "{wanted}");
        assert!(installed_version(&home).starts_with(&format!("tooob {release} ")));
        assert_eq!(
            stdout(&out),
            format!("tooob {release} (this is {THIS}): {}\n", notes(&release)),
            "{wanted}"
        );
    }
}

#[test]
fn a_release_that_isnt_published_is_refused_and_this_one_needs_nothing() {
    let (home, _github) = with_releases();
    home.install_tooob();

    let out = home.tooob(&["upgrade", "7.7.7", "--yes"]);
    assert!(!out.status.success());
    assert_eq!(
        stderr(&out),
        "tooob: there's no Release 7.7.7: https://github.com/Tooobs/tooob/releases lists them\n"
    );
    let out = home.tooob(&["upgrade", "seven", "--yes"]);
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("\"seven\" isn't a Release"),
        "{}",
        stderr(&out)
    );

    let out = home.tooob(&["upgrade", THIS, "--yes"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out), format!("this is tooob {THIS} already\n"));
    assert!(installed(&home).is_empty());
}

#[test]
fn on_the_newest_release_it_says_so_and_installs_nothing() {
    let home = Home::new();
    let github = TestGitHub::start();
    github.publish(THIS, Some(tooob::PROTOCOL_VERSION));
    github.publish(&format!("{}-rc.1", newer(1)), None);
    home.releases_at(&github.url);
    home.install_tooob();

    for args in [
        vec!["upgrade"],
        vec!["upgrade", "--check"],
        vec!["upgrade", "--yes"],
    ] {
        let out = home.tooob(&args);
        assert!(out.status.success(), "{args:?}: {}", stderr(&out));
        assert_eq!(
            stdout(&out),
            format!("tooob {THIS} is the newest Release\n"),
            "{args:?}"
        );
    }
    assert!(installed(&home).is_empty());
}

#[test]
fn a_tooob_that_isnt_the_installed_one_is_refused_with_the_installer_to_run_by_hand() {
    let (home, github) = with_releases();
    let build = env!("CARGO_BIN_EXE_tooob");
    let installer = format!(
        "curl --proto '=https' --tlsv1.2 -LsSf {}Tooobs/tooob/releases/download/v{}/tooob-installer.sh | sh",
        github.url,
        newer(1)
    );
    let refusal = format!(
        "tooob: this tooob is {build}, not ~/.local/bin/tooob where the installer puts it, so \
         upgrade won't replace it: install tooob {} by hand with\n  {installer}\n",
        newer(1)
    );

    // None installed, then another tooob there: a copy.
    for installed_one in [false, true] {
        if installed_one {
            let bin = home.0.path().join(".local/bin");
            std::fs::create_dir_all(&bin).unwrap();
            std::fs::copy(build, bin.join("tooob")).unwrap();
        }
        let out = home.tooob(&["upgrade", "--yes"]);
        assert!(!out.status.success(), "{installed_one}");
        assert_eq!(stderr(&out), refusal, "{installed_one}");
        assert!(installed(&home).is_empty());
        let out = home.tooob(&["upgrade", "--check"]);
        assert!(out.status.success(), "--check only looks: {}", stderr(&out));
    }
}

#[test]
fn a_tooob_in_a_container_is_refused_and_told_to_pull_the_new_image() {
    let (home, _github) = with_releases();
    home.install_tooob();
    let out = home.tooob_with(&["upgrade", "--yes"], &[("TOOOB_IN_CONTAINER", "1")]);
    assert!(!out.status.success());
    assert_eq!(
        stderr(&out),
        format!(
            "tooob: this tooob runs in a container, and upgrade replaces only \
             ~/.local/bin/tooob: pull tooob {}'s image instead\n",
            newer(1)
        )
    );
    assert!(installed(&home).is_empty());
}

#[test]
fn an_upgrade_that_would_leave_hosts_on_a_bootstrap_mismatch_is_refused_unless_anyway() {
    let home = Home::new();
    let github = TestGitHub::start();
    github.publish(THIS, Some(tooob::PROTOCOL_VERSION));
    github.publish(&newer(1), Some("tooob/7"));
    home.releases_at(&github.url);
    home.install_tooob();
    let (relay, other) = (TestRelay::bootstrap_only(), TestRelay::bootstrap_only());
    let (photos, blog, films) = (a_folder(), a_folder(), a_folder());
    home.ok(&[
        "add",
        "photos",
        photos.path().to_str().unwrap(),
        "--relay",
        &relay.url,
    ]);
    home.ok(&[
        "add",
        "blog",
        blog.path().to_str().unwrap(),
        "--relay",
        &relay.url,
    ]);
    home.ok(&[
        "add",
        "films",
        films.path().to_str().unwrap(),
        "--relay",
        &other.url,
    ]);
    home.ok(&["add", "notes", "127.0.0.1:8096", "--local"]);
    other.serve_bootstrap(Some(version_json(&newer(1), "tooob/7")));

    let out = home.tooob(&["upgrade", "--yes"]);
    assert!(!out.status.success());
    assert_eq!(
        stderr(&out),
        format!(
            "tooob: tooob {new} speaks tooob/7, and these Hosts' Relays serve a Bootstrap that \
             speaks another Protocol Version, so they'd be left on a Bootstrap Mismatch:\n  \
             blog, photos: Relay {url} serves Bootstrap {THIS} ({pv})\nDeploy each Relay with \
             tooob {new} first (deploy/ shows how), or upgrade regardless:\n  \
             tooob upgrade --anyway --yes\n",
            new = newer(1),
            url = relay.url,
            pv = tooob::PROTOCOL_VERSION,
        )
    );
    assert!(installed(&home).is_empty());

    let out = home.tooob(&["upgrade", "--yes", "--anyway"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(installed(&home), [newer(1)]);
}

#[test]
fn once_each_relay_serves_the_new_bootstrap_or_cant_be_reached_the_upgrade_goes_ahead() {
    let home = Home::new();
    let github = TestGitHub::start();
    github.publish(THIS, Some(tooob::PROTOCOL_VERSION));
    github.publish(&newer(1), Some("tooob/7"));
    home.releases_at(&github.url);
    home.install_tooob();
    let relay = TestRelay::bootstrap_only();
    let (photos, blog) = (a_folder(), a_folder());
    home.ok(&[
        "add",
        "photos",
        photos.path().to_str().unwrap(),
        "--relay",
        &relay.url,
    ]);
    let gone = {
        let gone = TestRelay::bootstrap_only();
        home.ok(&[
            "add",
            "blog",
            blog.path().to_str().unwrap(),
            "--relay",
            &gone.url,
        ]);
        gone.url.clone()
    };
    relay.serve_bootstrap(Some(version_json(&newer(1), "tooob/7")));

    let out = home.tooob(&["upgrade", "--yes"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(
        stderr(&out).contains(&format!(
            "tooob: warning: can't check blog's Relay {gone}, so can't tell whether tooob {} \
             works with its Bootstrap (",
            newer(1)
        )),
        "{}",
        stderr(&out)
    );
    assert_eq!(installed(&home), [newer(1)]);
}

#[test]
fn a_release_whose_protocol_version_cant_be_told_is_refused_unless_anyway() {
    let (home, _github) = with_releases();
    home.install_tooob();
    let relay = TestRelay::bootstrap_only();
    let photos = a_folder();
    home.ok(&[
        "add",
        "photos",
        photos.path().to_str().unwrap(),
        "--relay",
        &relay.url,
    ]);
    let rc = format!("{}-rc.1", newer(2));

    let out = home.tooob(&["upgrade", &rc, "--yes"]);
    assert!(!out.status.success());
    assert_eq!(
        stderr(&out),
        format!(
            "tooob: tooob {rc}'s notes don't say which Protocol Version it speaks, so whether \
             your Hosts' Relays serve a Bootstrap it works with can't be checked; upgrade \
             regardless:\n  tooob upgrade {rc} --anyway --yes\n"
        )
    );
    assert!(installed(&home).is_empty());
    home.ok(&["upgrade", &rc, "--yes", "--anyway"]);
    assert_eq!(installed(&home), [rc]);
}

/// This user's id, as launchd's `gui/UID` domain names it.
fn uid() -> u32 {
    unsafe { libc::getuid() }
}

/// The pid in `file`, once something has written one there.
fn pid_in(file: &std::path::Path) -> u32 {
    let started = std::time::Instant::now();
    loop {
        if let Some(pid) = std::fs::read_to_string(file)
            .ok()
            .and_then(|p| p.trim().parse().ok())
        {
            return pid;
        }
        assert!(
            started.elapsed().as_secs() < 10,
            "nothing wrote {}",
            file.display()
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

#[test]
fn after_an_upgrade_enabled_hosts_run_the_new_release_and_hosts_run_by_hand_are_named() {
    for (tool, stop, start) in [
        (
            "launchctl",
            format!("kill SIGTERM gui/{}/net.tooobs.photos", uid()),
            format!("kickstart gui/{}/net.tooobs.photos", uid()),
        ),
        (
            "systemctl",
            "--user restart tooob@photos.service".to_string(),
            "--user restart tooob@photos.service".to_string(),
        ),
    ] {
        let (home, _github) = with_releases();
        home.install_tooob();
        for name in ["photos", "blog", "films"] {
            let folder = a_folder();
            home.ok(&["add", name, folder.path().to_str().unwrap(), "--local"]);
            std::mem::forget(folder);
        }
        // The stand-in service manager stops the Host in `pid` and starts the one installed,
        // as the Host's entry says to, writing its pid in `new-pid`.
        let (pid, new_pid) = (
            home.stand_ins().join("pid"),
            home.stand_ins().join("new-pid"),
        );
        let log = home.0.path().join("restarted.log");
        let stops = format!("read pid < '{}'; kill -TERM $pid", pid.display());
        let starts = format!(
            "TOOOB_AUTOSTART=photos \"$HOME/.local/bin/tooob\" run photos > '{}' 2>&1 < /dev/null \
             & echo $! > '{}'",
            log.display(),
            new_pid.display()
        );
        let then = match stop == start {
            true => format!("case \"$*\" in '{stop}') {stops}; {starts};; esac"),
            false => format!("case \"$*\" in '{stop}') {stops};; '{start}') {starts};; esac"),
        };
        // Only once it's enabled: enabling it with systemd restarts it too.
        let armed = home.stand_ins().join("armed");
        home.stand_in(
            tool,
            &format!("[ -e '{}' ] || exit 0\n{then}", armed.display()),
        );
        home.ok(&["enable", "photos"]);
        std::fs::write(&armed, "").unwrap();
        let enabled = home.calls(tool).len();
        let photos = home.spawn_with(&["run", "photos"], &[("TOOOB_AUTOSTART", "photos")]);
        photos.until_it_says("serving");
        std::fs::write(&pid, photos.child.id().to_string()).unwrap();
        let _blog = home.start("blog");

        let out = home.tooob(&["upgrade", "--yes"]);
        assert!(out.status.success(), "{tool}: {}", stderr(&out));
        let restarted = pid_in(&new_pid);
        let _cleanup = Killed(restarted);
        let mut calls = vec![stop.clone(), start.clone()];
        calls.dedup();
        assert_eq!(home.calls(tool)[enabled..], calls, "{tool}");
        photos.until_it_says("photos: stopped");
        let said = stderr(&out);
        assert!(
            said.contains(&format!("tooob: restarted on tooob {}: photos\n", newer(1))),
            "{tool}: {said}"
        );
        assert!(
            said.contains(&format!("tooob: still on {THIS} until restarted: blog\n")),
            "{tool}: {said}"
        );
        assert!(!said.contains("films"), "{tool}: a stopped Host: {said}");
        let ran = std::fs::read_to_string(home.0.path().join(format!("ran-{}", newer(1))));
        assert_eq!(
            ran.unwrap(),
            "run photos\n",
            "{tool}: the new Release runs it"
        );
        assert!(home.row("photos").contains("running"), "{tool}");
    }
}

/// Kills a process when dropped.
struct Killed(u32);

impl Drop for Killed {
    fn drop(&mut self) {
        unsafe { libc::kill(self.0 as i32, libc::SIGKILL) };
    }
}

/// What a Host and `tooob ls` say when a newer Release is out.
fn out_line(release: &str) -> String {
    format!("tooob {release} is out: tooob upgrade")
}

#[test]
fn a_running_host_says_once_that_a_newer_release_is_out_and_ls_says_it_without_asking_github() {
    let (home, github) = with_releases();
    let (photos, blog) = (a_folder(), a_folder());
    home.ok(&["add", "photos", photos.path().to_str().unwrap(), "--local"]);
    home.ok(&["add", "blog", blog.path().to_str().unwrap(), "--local"]);
    assert!(
        !home.ok(&["ls"]).contains("is out"),
        "nothing looked up yet"
    );

    let photos = home.start("photos");
    photos.until_it_says(&format!("photos: {}\n", out_line(&newer(1))));
    let blog = home.start("blog");
    blog.until_it_says(&format!("blog: {}\n", out_line(&newer(1))));
    assert_eq!(
        github.lookups(),
        1,
        "once a day for the whole box: {:?}",
        github.asked()
    );

    let asked = github.asked().len();
    let listed = home.ok(&["ls"]);
    assert!(
        listed.ends_with(&format!("\n\n{}\n", out_line(&newer(1)))),
        "under its table: {listed}"
    );
    assert_eq!(github.asked().len(), asked, "ls never asks GitHub");
    assert_eq!(
        photos.log().matches("is out").count(),
        1,
        "{}",
        photos.log()
    );
}

#[test]
fn a_running_host_looks_again_each_interval_but_says_a_release_is_out_only_once() {
    let (home, github) = with_releases();
    let photos = a_folder();
    home.ok(&["add", "photos", photos.path().to_str().unwrap(), "--local"]);

    let photos = home.start_with("photos", &[("TOOOB_UPDATE_CHECK_EVERY", "1s")]);
    let started = std::time::Instant::now();
    while github.lookups() < 3 {
        assert!(started.elapsed().as_secs() < 10, "{:?}", github.asked());
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert_eq!(
        photos.log().matches("is out").count(),
        1,
        "{}",
        photos.log()
    );
}

#[test]
fn no_update_check_stops_the_lookup_and_on_the_newest_release_nothing_is_said() {
    let (home, github) = with_releases();
    let photos = a_folder();
    home.ok(&["add", "photos", photos.path().to_str().unwrap(), "--local"]);
    let running = home.start_with(
        "photos",
        &[
            ("TOOOB_NO_UPDATE_CHECK", "1"),
            ("TOOOB_UPDATE_CHECK_EVERY", "1s"),
        ],
    );
    std::thread::sleep(std::time::Duration::from_secs(2));
    assert_eq!(github.lookups(), 0);
    assert!(!running.log().contains("is out"), "{}", running.log());
    running.stop();

    let home = Home::new();
    let github = TestGitHub::start();
    github.publish(THIS, Some(tooob::PROTOCOL_VERSION));
    github.publish(&format!("{}-rc.1", newer(1)), None);
    home.releases_at(&github.url);
    let photos = a_folder();
    home.ok(&["add", "photos", photos.path().to_str().unwrap(), "--local"]);
    let running = home.start_with("photos", &[("TOOOB_UPDATE_CHECK_EVERY", "1s")]);
    let started = std::time::Instant::now();
    while github.lookups() < 2 {
        assert!(started.elapsed().as_secs() < 10, "{:?}", github.asked());
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(!running.log().contains("is out"), "{}", running.log());
    assert!(!home.ok(&["ls"]).contains("is out"));
}

#[test]
fn a_host_enabled_with_no_update_check_set_keeps_it_off_when_its_service_manager_runs_it() {
    for (tool, entry) in [
        ("launchctl", "Library/LaunchAgents/net.tooobs.photos.plist"),
        ("systemctl", ".config/systemd/user/tooob@photos.service"),
    ] {
        let home = Home::new();
        let photos = a_folder();
        home.ok(&["add", "photos", photos.path().to_str().unwrap(), "--local"]);
        home.install_tooob();
        home.stand_in(tool, "");
        let out = home.tooob_with(&["enable", "photos"], &[("TOOOB_NO_UPDATE_CHECK", "1")]);
        assert!(out.status.success(), "{tool}: {}", stderr(&out));
        let text = std::fs::read_to_string(home.0.path().join(entry)).unwrap();
        assert!(
            text.contains("<key>TOOOB_NO_UPDATE_CHECK</key>\n\t\t<string>1</string>")
                || text.contains("Environment=\"TOOOB_NO_UPDATE_CHECK=1\""),
            "{tool}: {text}"
        );
    }
}
