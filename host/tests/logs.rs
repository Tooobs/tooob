//! A Host's own log (ADR 0025): written by the Host itself, however it was started, a JSON object a
//! line with its time, bounded by rolling over; read with `tooob logs NAME`. Through the `tooob`
//! command, run as a script would.
mod support;

use serde_json::{Value, json};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use support::home::{Home, TestRelay, a_folder, version_json};

/// A time zone five hours behind UTC with no summer time, so a local time is told from UTC.
const TZ: (&str, &str) = ("TZ", "XYZ+5");

/// Now, to the minute, in `TZ`, as `date` says it in `format`.
fn local_minute(format: &str) -> String {
    let out = std::process::Command::new("date")
        .arg(format!("+{format}"))
        .env(TZ.0, TZ.1)
        .output()
        .unwrap();
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

/// Whether `line`, as `tooob logs` shows it, starts with a time, `2026-10-31 14:02:11` and two
/// spaces, in one of `minutes`.
fn shown_in(line: &str, minutes: &[String]) -> bool {
    let shape = line.len() > 21
        && line.char_indices().take(21).all(|(i, c)| match i {
            4 | 7 => c == '-',
            10 => c == ' ',
            13 | 16 => c == ':',
            19 | 20 => c == ' ',
            _ => c.is_ascii_digit(),
        });
    shape && minutes.iter().any(|m| line.starts_with(m.as_str()))
}

fn read(file: &str) -> String {
    std::fs::read_to_string(file).unwrap_or_default()
}

fn mode(file: impl AsRef<Path>) -> u32 {
    std::fs::metadata(file).unwrap().permissions().mode() & 0o777
}

fn stderr(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// A line of photos' log as it keeps it, at a time long gone.
fn old_line(text: &str) -> String {
    json!({"time": "2026-10-30T09:00:00-05:00", "host": "photos", "text": text}).to_string() + "\n"
}

/// That line as `tooob logs` shows it.
fn shown(text: &str) -> String {
    format!("2026-10-30 09:00:00  photos: {text}\n")
}

#[test]
fn a_host_run_by_hand_writes_its_own_log_each_line_in_local_time_and_prints_it_too() {
    let home = Home::new();
    let folder = a_folder();
    home.ok(&["add", "photos", folder.path().to_str().unwrap(), "--local"]);
    let log = home.shown("photos", "log");

    let before = (
        local_minute("%Y-%m-%dT%H:%M"),
        local_minute("%Y-%m-%d %H:%M"),
    );
    let mut running = home.start_with("photos", &[TZ]);
    assert!(running.stop_with(libc::SIGTERM).success());
    let after = (
        local_minute("%Y-%m-%dT%H:%M"),
        local_minute("%Y-%m-%d %H:%M"),
    );

    assert_eq!(mode(&log), 0o600, "only the Owner's account reads it");
    let kept = read(&log);
    for line in kept.lines() {
        let line: Value = serde_json::from_str(line).unwrap();
        let time = line["time"].as_str().unwrap();
        assert!(
            (time.starts_with(&before.0) || time.starts_with(&after.0))
                && time.ends_with("-05:00")
                && time.len() == 25,
            "local time with its offset from UTC: {line}"
        );
        assert_eq!(line["host"], "photos", "{line}");
    }

    let shown = home.ok(&["logs", "photos", "-n", "all"]);
    assert_eq!(shown.lines().count(), kept.lines().count());
    assert!(shown.contains("  photos: serving "), "{shown}");
    assert!(shown.ends_with("  photos: stopped\n"), "{shown}");
    for line in shown.lines() {
        assert!(
            shown_in(line, &[before.1.clone(), after.1.clone()]),
            "{line:?}"
        );
        assert!(
            running.log().contains(&format!("{line}\n")),
            "its terminal had each line: {line:?} in:\n{}",
            running.log()
        );
    }
}

#[test]
fn logs_shows_the_last_50_lines_of_any_hosts_log_or_n_of_them_or_all() {
    let home = Home::new();
    let folder = a_folder();
    home.ok(&["add", "photos", folder.path().to_str().unwrap(), "--local"]);
    home.ok(&["add", "films", "127.0.0.1:8096", "--local"]);
    let log = home.shown("photos", "log");
    let texts: Vec<String> = (0..120).map(|i| format!("line {i}")).collect();
    let kept: String = texts.iter().map(|t| old_line(t)).collect();
    std::fs::write(&log, kept).unwrap();
    let lines: Vec<String> = texts.iter().map(|t| shown(t)).collect();

    assert_eq!(home.ok(&["logs", "photos"]), lines[70..].concat());
    assert_eq!(
        home.ok(&["logs", "photos", "-n", "3"]),
        lines[117..].concat()
    );
    assert_eq!(home.ok(&["logs", "photos", "-n", "all"]), lines.concat());
    assert_eq!(home.ok(&["logs", "photos", "-n", "0"]), "");
    for bad in ["-3", "some", ""] {
        let out = home.tooob(&["logs", "photos", "-n", bad]);
        assert!(!out.status.success(), "-n {bad:?}");
        assert_eq!(
            stderr(&out),
            format!("tooob: -n takes a number of lines, or all: not {bad:?}\n")
        );
    }
    let out = home.tooob(&["logs", "photos", "-n"]);
    assert_eq!(stderr(&out), "tooob: -n needs a value\n");

    // Run by hand, never enabled: its lines are there too.
    let mut running = home.start("photos");
    assert!(running.stop_with(libc::SIGTERM).success());
    let last = home.ok(&["logs", "photos", "-n", "1"]);
    assert!(last.ends_with("  photos: stopped\n"), "{last}");

    let never = home.tooob(&["logs", "films"]);
    assert!(never.status.success(), "{}", stderr(&never));
    assert_eq!(String::from_utf8_lossy(&never.stdout), "");
    assert_eq!(
        stderr(&never),
        "tooob: films has no log yet: a Host starts one when it first runs\n"
    );
}

#[test]
fn a_log_past_its_limit_rolls_over_to_one_older_file_replacing_the_one_before() {
    let home = Home::new();
    let folder = a_folder();
    home.ok(&["add", "photos", folder.path().to_str().unwrap(), "--local"]);
    let log = home.shown("photos", "log");
    let older = format!("{log}.1");
    std::fs::write(&older, old_line("from long ago")).unwrap();
    let texts: Vec<String> = (0..20).map(|i| format!("line {i}")).collect();
    let old: String = texts.iter().map(|t| old_line(t)).collect();
    std::fs::write(&log, &old).unwrap();

    // Room for a few more bytes, not a line.
    let limit = (old.len() + 20).to_string();
    let mut running = home.start_with("photos", &[("TOOOB_LOG_LIMIT", &limit)]);
    assert!(running.stop_with(libc::SIGTERM).success());

    assert_eq!(read(&older), old, "the older file is the log as it was");
    assert_eq!(mode(&older), 0o600);
    let new = read(&log);
    assert!(
        new.contains("\"starting ") && new.contains("\"stopped\""),
        "{new}"
    );
    assert!(!new.contains("line 19"), "{new}");
    assert!(new.len() <= old.len() + 20, "{}", new.len());
    assert!(!Path::new(&format!("{log}.2")).exists());
    let all = home.ok(&["logs", "photos", "-n", "all"]);
    let before: String = texts.iter().map(|t| shown(t)).collect();
    assert!(all.starts_with(&before), "{all}");
    assert!(all.ends_with("  photos: stopped\n"), "{all}");
    assert_eq!(all.lines().count(), 20 + new.lines().count());
}

#[test]
fn logs_f_follows_what_it_writes_next_across_a_roll_over() {
    let home = Home::new();
    let folder = a_folder();
    home.ok(&["add", "photos", folder.path().to_str().unwrap(), "--local"]);
    let log = home.shown("photos", "log");
    let old: String = (0..30).map(|i| old_line(&format!("line {i}"))).collect();
    std::fs::write(&log, &old).unwrap();

    let following = home.spawn(&["logs", "photos", "-f", "-n", "1"]);
    following.until_it_says("line 29\n");
    let limit = (old.len() + 20).to_string();
    let mut running = home.start_with("photos", &[("TOOOB_LOG_LIMIT", &limit)]);
    assert!(running.stop_with(libc::SIGTERM).success());
    following.until_it_says("photos: stopped\n");
    assert_eq!(read(&format!("{log}.1")), old, "it rolled over");
    let followed = following.log();
    assert!(followed.starts_with(&shown("line 29")), "{followed}");
    assert!(followed.contains("  photos: starting "), "{followed}");
    assert_eq!(followed.lines().count(), 1 + read(&log).lines().count());
}

#[test]
fn logs_json_passes_each_line_through_and_gives_a_line_it_didnt_write_no_time() {
    let home = Home::new();
    let folder = a_folder();
    home.ok(&["add", "photos", folder.path().to_str().unwrap(), "--local"]);
    let log = home.shown("photos", "log");
    let older = format!("{log}.1");
    std::fs::write(&older, old_line("serving /photos")).unwrap();
    // One with a field a later tooob adds; and one it didn't write itself: 0.2.0's, or a crash's.
    let later = json!({
        "time": "2026-10-30T09:00:01-05:00", "host": "photos", "text": "jm got in",
        "event": "visit"
    });
    std::fs::write(
        &log,
        format!("{later}\nthread 'main' panicked at src/main.rs:1:1:\n"),
    )
    .unwrap();

    let out = home.ok(&["logs", "photos", "--json"]);
    let objects: Vec<Value> = out
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        objects,
        [
            json!({"time": "2026-10-30T09:00:00-05:00", "host": "photos", "text": "serving /photos"}),
            later,
            json!({
                "time": null, "host": "photos", "text": "thread 'main' panicked at src/main.rs:1:1:"
            }),
        ]
    );
    assert_eq!(
        home.ok(&["logs", "photos"]),
        shown("serving /photos")
            + "2026-10-30 09:00:01  photos: jm got in\n"
            + "thread 'main' panicked at src/main.rs:1:1:\n"
    );
}

/// What a Host's Autostart entry sets, as its service manager starts it.
const AS_ITS_ENTRY: (&str, &str) = ("TOOOB_AUTOSTART", "photos");

#[test]
fn a_host_its_service_manager_runs_writes_only_its_log_and_prints_nothing() {
    let home = Home::new();
    let folder = a_folder();
    home.ok(&["add", "photos", folder.path().to_str().unwrap(), "--local"]);

    let mut running = home.spawn_with(&["run", "photos"], &[AS_ITS_ENTRY]);
    home.until_logged("photos", "photos: serving ");
    assert!(running.stop_with(libc::SIGTERM).success());
    let shown = home.ok(&["logs", "photos"]);
    assert!(shown.ends_with("photos: stopped\n"), "{shown}");
    assert_eq!(
        running.log(),
        "",
        "its service manager's capture is for crashes"
    );
}

/// Write `said` to photos' crash log, as its service manager would as it died, dated then in
/// `TZ`.
fn crashed(home: &Home, said: &str) -> String {
    let crashes = format!("{}/crash.log", home.shown("photos", "state folder"));
    std::fs::write(&crashes, said).unwrap();
    let touched = std::process::Command::new("touch")
        .args(["-t", "202610300901.30", &crashes])
        .env(TZ.0, TZ.1)
        .status()
        .unwrap();
    assert!(touched.success());
    crashes
}

const PANIC: &str =
    "thread 'main' panicked at src/main.rs:1:1:\nsomething broke on 198.51.100.23:54321\n";

#[test]
fn a_hosts_dying_words_are_moved_into_its_log_when_it_next_starts() {
    let home = Home::new();
    let folder = a_folder();
    home.ok(&["add", "photos", folder.path().to_str().unwrap(), "--local"]);
    let log = home.shown("photos", "log");
    std::fs::write(&log, old_line("an older line")).unwrap();
    let crashes = crashed(&home, PANIC);

    let mut running = home.spawn_with(&["run", "photos"], &[AS_ITS_ENTRY, TZ]);
    home.until_logged("photos", "photos: serving ");
    assert!(running.stop_with(libc::SIGTERM).success());
    let shown_now = home.ok(&["logs", "photos", "-n", "all"]);
    let lines: Vec<&str> = shown_now.lines().collect();
    assert_eq!(
        lines[..4],
        [
            "2026-10-30 09:00:00  photos: an older line",
            "2026-10-30 09:01:30  photos: before it stopped last time, it said:",
            "2026-10-30 09:01:30  photos:   thread 'main' panicked at src/main.rs:1:1:",
            "2026-10-30 09:01:30  photos:   something broke on an address",
        ],
        "{lines:#?}"
    );
    assert!(lines[4].contains("photos: starting"), "{lines:#?}");
    assert_eq!(read(&crashes), "", "said once");
    assert_eq!(mode(&crashes), 0o600);
}

#[test]
fn logs_shows_what_a_host_said_as_it_stopped_that_isnt_in_its_log_yet() {
    // Its service manager gave up restarting it, so it never moved them into its log.
    let home = Home::new();
    let folder = a_folder();
    home.ok(&["add", "photos", folder.path().to_str().unwrap(), "--local"]);
    let log = home.shown("photos", "log");
    std::fs::write(&log, old_line("serving /photos")).unwrap();
    let crashes = crashed(&home, PANIC);

    let not_yet = "before it stopped last time, it said (not in its log yet):";
    let logs = |args: &[&str]| {
        let out = home.tooob_with(&[&["logs", "photos"], args].concat(), &[TZ]);
        assert!(out.status.success(), "{}", stderr(&out));
        String::from_utf8(out.stdout).unwrap()
    };
    assert_eq!(
        logs(&["-n", "1"]),
        shown("serving /photos")
            + "2026-10-30 09:01:30  photos: "
            + not_yet
            + "\n2026-10-30 09:01:30  photos:   thread 'main' panicked at src/main.rs:1:1:\n\
               2026-10-30 09:01:30  photos:   something broke on an address\n"
    );
    let objects: Vec<Value> = logs(&["--json"])
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        objects[1],
        json!({"time": "2026-10-30T09:01:30-05:00", "host": "photos", "text": not_yet})
    );

    let following = home.spawn_with(&["logs", "photos", "-f"], &[TZ]);
    following.until_it_says("  something broke on an address\n");
    {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&crashes)
            .unwrap();
        file.write_all(b"and then some\n").unwrap();
    }
    following.until_it_says("photos:   and then some\n");
    assert_eq!(following.log().matches(not_yet).count(), 1);
}

/// `tooob run photos` as an Autostart entry from tooob 0.2.0 starts it: its output appended to
/// its log, which the Host now writes itself.
fn run_as_a_0_2_0_entry(home: &Home, log: &str) -> std::process::Child {
    let output = || {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log)
            .unwrap()
    };
    home.command(&["run", "photos"])
        .env(AS_ITS_ENTRY.0, AS_ITS_ENTRY.1)
        .stdout(output())
        .stderr(output())
        .spawn()
        .unwrap()
}

/// What process `pid`'s file descriptor `fd` is open on.
fn open_on(pid: u32, fd: u32) -> String {
    if cfg!(target_os = "linux") {
        let link = std::fs::read_link(format!("/proc/{pid}/fd/{fd}")).unwrap();
        return link.display().to_string();
    }
    let out = std::process::Command::new("lsof")
        .args(["-a", "-p", &pid.to_string(), "-d", &fd.to_string(), "-Fn"])
        .output()
        .unwrap();
    let out = String::from_utf8_lossy(&out.stdout).into_owned();
    out.lines()
        .find_map(|l| l.strip_prefix('n'))
        .unwrap_or_else(|| panic!("no file for fd {fd}: {out}"))
        .to_string()
}

#[test]
fn a_host_whose_entry_points_its_output_at_its_log_moves_it_to_its_crash_log_once_it_runs() {
    let home = Home::new();
    let folder = a_folder();
    home.ok(&["add", "photos", folder.path().to_str().unwrap(), "--local"]);
    let log = home.shown("photos", "log");
    let crashes = format!("{}/crash.log", home.shown("photos", "state folder"));

    let mut first = run_as_a_0_2_0_entry(&home, &log);
    home.until_logged("photos", "photos: serving ");
    for fd in [1, 2] {
        let on = open_on(first.id(), fd);
        assert!(on.ends_with("/photos/crash.log"), "fd {fd}: {on}");
    }
    // A second one is refused before it takes the lock, so it says why where it was told to.
    let second = run_as_a_0_2_0_entry(&home, &log).wait().unwrap();
    assert!(!second.success());
    unsafe { libc::kill(first.id() as i32, libc::SIGTERM) };
    assert!(first.wait().unwrap().success());

    assert!(
        !read(&crashes).contains("already running"),
        "{}",
        read(&crashes)
    );
    let shown = home.ok(&["logs", "photos", "-n", "all"]);
    assert_eq!(shown.matches("photos: serving ").count(), 1, "{shown}");
    assert!(
        shown.contains("\ntooob: photos is already running"),
        "{shown}"
    );
    assert!(shown.ends_with("photos: stopped\n"), "{shown}");
}

#[test]
fn two_writers_of_one_log_never_cross_lines_or_roll_over_twice() {
    // A running Host and a command made while it's stopped (#23) may write the same log: here, two
    // of tooob's own writers, each with its own lock file open, as two processes would have.
    let state = tempfile::tempdir().unwrap();
    let text = "x".repeat(200);
    std::thread::scope(|s| {
        for name in ["photos", "other"] {
            let (dir, text) = (state.path(), &text);
            s.spawn(move || {
                let log = tooob::log::Log::open(dir, name, false);
                for _ in 0..6000 {
                    log.write(text);
                }
            });
        }
    });
    let log = state.path().join("host.log");
    let older = state.path().join("host.log.1");
    for file in [&log, &older] {
        let kept = std::fs::read_to_string(file).unwrap();
        for line in kept.lines() {
            let line: Value = serde_json::from_str(line).unwrap_or_else(|e| panic!("{e}: {line}"));
            assert_eq!(line["text"], text.as_str());
        }
        assert!(
            kept.len() <= 1_000_000,
            "{}: {}",
            file.display(),
            kept.len()
        );
    }
    let older = std::fs::metadata(&older).unwrap().len();
    assert!(older > 999_000, "rolled over full, not twice: {older}");
}

/// Host photos' log's lines, as `tooob logs --json` gives them.
fn events(home: &Home) -> Vec<Value> {
    home.ok(&["logs", "photos", "-n", "all", "--json"])
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

#[test]
fn a_hosts_own_events_are_named_in_its_log_starting_serving_warnings_a_mismatch_and_stopping() {
    let home = Home::new();
    let folder = a_folder();
    let relay = TestRelay::start();
    let path = folder.path().to_str().unwrap();
    home.ok(&["add", "photos", path, "--relay", &relay.url]);
    let every = ("TOOOB_BOOTSTRAP_CHECK_EVERY", "1s");
    let mut running = home.start_with("photos", &[every]);
    relay.serve_bootstrap(Some(version_json("0.9.0", "tooob/9")));
    home.until_logged("photos", "tooob/9");
    relay.serve_bootstrap(None);
    home.until_logged("photos", "serves no Bootstrap");
    assert!(running.stop_with(libc::SIGTERM).success());

    let lines = events(&home);
    let named =
        |event: &str| -> Vec<&Value> { lines.iter().filter(|l| l["event"] == event).collect() };
    let starting = named("starting");
    assert_eq!(starting.len(), 1, "{lines:#?}");
    assert_eq!(starting[0]["fields"]["release"], env!("CARGO_PKG_VERSION"));
    assert_eq!(
        starting[0]["fields"]["protocol_version"],
        tooob::PROTOCOL_VERSION
    );
    assert_eq!(named("serving").len(), 1, "{lines:#?}");
    let mismatches = named("bootstrap_mismatch");
    assert_eq!(mismatches.len(), 2, "{lines:#?}");
    assert!(
        mismatches[0]["text"]
            .as_str()
            .unwrap()
            .starts_with("warning: ")
    );
    assert_eq!(mismatches[0]["fields"]["protocol_version"], "tooob/9");
    assert_eq!(lines.last().unwrap()["event"], "stopped", "{lines:#?}");
}
