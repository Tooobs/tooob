//! A Host's own log (ADR 0025): written by the Host itself, however it was started, each line with
//! its time, bounded by rolling over; read with `tooob logs NAME`. Through the `tooob` command, run
//! as a script would.
mod support;

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use support::home::{Home, a_folder};

/// A time zone five hours behind UTC with no summer time, so a local time is told from UTC.
const TZ: (&str, &str) = ("TZ", "XYZ+5");

/// Now, to the minute, in `TZ`, as `date` says it.
fn local_minute() -> String {
    let out = std::process::Command::new("date")
        .arg("+%Y-%m-%d %H:%M")
        .env(TZ.0, TZ.1)
        .output()
        .unwrap();
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

/// Whether `line` starts with a time, `2026-10-31 14:02:11` and two spaces, in `minutes`.
fn timed_in(line: &str, minutes: &[String]) -> bool {
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

#[test]
fn a_host_run_by_hand_writes_its_own_log_each_line_in_local_time_and_prints_it_too() {
    let home = Home::new();
    let folder = a_folder();
    home.ok(&["add", "photos", folder.path().to_str().unwrap(), "--local"]);
    let log = home.shown("photos", "log");

    let before = local_minute();
    let mut running = home.start_with("photos", &[TZ]);
    assert!(running.stop_with(libc::SIGTERM).success());
    let minutes = [before, local_minute()];

    let written = read(&log);
    assert_eq!(mode(&log), 0o600, "only the Owner's account reads it");
    assert!(written.contains("  photos: serving "), "{written}");
    assert!(written.ends_with("  photos: stopped\n"), "{written}");
    for line in written.lines() {
        assert!(timed_in(line, &minutes), "{line:?} in:\n{written}");
        assert!(
            running.log().contains(&format!("{line}\n")),
            "its terminal had each line: {line:?} in:\n{}",
            running.log()
        );
    }
}

fn stderr(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// A log line as a Host writes it, at a time long gone.
fn old_line(text: &str) -> String {
    format!("2026-10-30 09:00:00  photos: {text}\n")
}

#[test]
fn logs_shows_the_last_50_lines_of_any_hosts_log_or_n_of_them_or_all() {
    let home = Home::new();
    let folder = a_folder();
    home.ok(&["add", "photos", folder.path().to_str().unwrap(), "--local"]);
    home.ok(&["add", "films", "127.0.0.1:8096", "--local"]);
    let log = home.shown("photos", "log");
    let lines: Vec<String> = (0..120).map(|i| old_line(&format!("line {i}"))).collect();
    std::fs::write(&log, lines.concat()).unwrap();

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
    let old: String = (0..20).map(|i| old_line(&format!("line {i}"))).collect();
    std::fs::write(&log, &old).unwrap();

    // Room for a few more bytes, not a line.
    let limit = (old.len() + 20).to_string();
    let mut running = home.start_with("photos", &[("TOOOB_LOG_LIMIT", &limit)]);
    assert!(running.stop_with(libc::SIGTERM).success());

    assert_eq!(read(&older), old, "the older file is the log as it was");
    assert_eq!(mode(&older), 0o600);
    let new = read(&log);
    assert!(new.contains("photos: starting") && new.ends_with("photos: stopped\n"));
    assert!(!new.contains("line 19"), "{new}");
    assert!(new.len() <= old.len() + 20, "{}", new.len());
    assert!(!Path::new(&format!("{log}.2")).exists());
    assert_eq!(home.ok(&["logs", "photos", "-n", "all"]), old + &new);
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
    assert_eq!(following.log(), old_line("line 29") + &read(&log));
}

#[test]
fn logs_json_gives_an_object_a_line_with_its_time_its_host_and_what_it_says() {
    let home = Home::new();
    let folder = a_folder();
    home.ok(&["add", "photos", folder.path().to_str().unwrap(), "--local"]);
    let log = home.shown("photos", "log");
    let older = format!("{log}.1");
    std::fs::write(&older, old_line("serving /photos")).unwrap();
    // A line it didn't write itself: a crash's, under 0.2.0's Autostart.
    std::fs::write(
        &log,
        old_line("a Visitor's: \"quoted\"") + "thread 'main' panicked at src/main.rs:1:1:\n",
    )
    .unwrap();

    let json = home.ok(&["logs", "photos", "--json"]);
    let objects: Vec<serde_json::Value> = json
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        objects,
        [
            serde_json::json!({
                "time": "2026-10-30T09:00:00", "host": "photos", "text": "serving /photos"
            }),
            serde_json::json!({
                "time": "2026-10-30T09:00:00", "host": "photos", "text": "a Visitor's: \"quoted\""
            }),
            serde_json::json!({
                "time": null, "host": "photos", "text": "thread 'main' panicked at src/main.rs:1:1:"
            }),
        ]
    );
    assert_eq!(
        home.ok(&["logs", "photos", "--json", "-n", "1"]),
        json.lines().last().unwrap().to_string() + "\n"
    );
}

/// What a Host's Autostart entry sets, as its service manager starts it.
const AS_ITS_ENTRY: (&str, &str) = ("TOOOB_AUTOSTART", "photos");

#[test]
fn a_host_its_service_manager_runs_writes_only_its_log_and_prints_nothing() {
    let home = Home::new();
    let folder = a_folder();
    home.ok(&["add", "photos", folder.path().to_str().unwrap(), "--local"]);
    let log = home.shown("photos", "log");

    let mut running = home.spawn_with(&["run", "photos"], &[AS_ITS_ENTRY]);
    home.until_logged("photos", "photos: serving ");
    assert!(running.stop_with(libc::SIGTERM).success());
    assert!(read(&log).ends_with("photos: stopped\n"), "{}", read(&log));
    assert_eq!(
        running.log(),
        "",
        "its service manager's capture is for crashes"
    );
}

#[test]
fn a_hosts_dying_words_are_moved_into_its_log_when_it_next_starts() {
    let home = Home::new();
    let folder = a_folder();
    home.ok(&["add", "photos", folder.path().to_str().unwrap(), "--local"]);
    let log = home.shown("photos", "log");
    let crashes = format!("{}/crash.log", home.shown("photos", "state folder"));
    std::fs::write(&log, old_line("an older line")).unwrap();
    std::fs::write(
        &crashes,
        "thread 'main' panicked at src/main.rs:1:1:\nsomething broke\n",
    )
    .unwrap();
    // As it was when the Host died.
    let touched = std::process::Command::new("touch")
        .args(["-t", "202610300901.30", &crashes])
        .status()
        .unwrap();
    assert!(touched.success());

    let mut running = home.spawn_with(&["run", "photos"], &[AS_ITS_ENTRY]);
    home.until_logged("photos", "photos: serving ");
    assert!(running.stop_with(libc::SIGTERM).success());
    let lines: Vec<String> = read(&log).lines().map(String::from).collect();
    assert_eq!(
        lines[..4],
        [
            "2026-10-30 09:00:00  photos: an older line",
            "2026-10-30 09:01:30  photos: it last stopped saying, in crash.log:",
            "2026-10-30 09:01:30  photos:   thread 'main' panicked at src/main.rs:1:1:",
            "2026-10-30 09:01:30  photos:   something broke",
        ],
        "{lines:#?}"
    );
    assert!(lines[4].contains("photos: starting"), "{lines:#?}");
    assert_eq!(read(&crashes), "", "said once");
    assert_eq!(mode(&crashes), 0o600);
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

#[test]
fn a_host_whose_entry_points_its_output_at_its_log_keeps_its_dying_words_out_of_it() {
    let home = Home::new();
    let folder = a_folder();
    home.ok(&["add", "photos", folder.path().to_str().unwrap(), "--local"]);
    let log = home.shown("photos", "log");
    let crashes = format!("{}/crash.log", home.shown("photos", "state folder"));

    let mut first = run_as_a_0_2_0_entry(&home, &log);
    home.until_logged("photos", "photos: serving ");
    // A second one, refused before it opens its log: what it says is a crash's.
    let second = run_as_a_0_2_0_entry(&home, &log).wait().unwrap();
    assert!(!second.success());
    unsafe { libc::kill(first.id() as i32, libc::SIGTERM) };
    assert!(first.wait().unwrap().success());

    let written = read(&log);
    for line in written.lines() {
        assert!(
            line.starts_with("20") && line.contains("  photos: "),
            "only its own lines, each once: {line:?} in:\n{written}"
        );
    }
    assert_eq!(written.matches("photos: serving ").count(), 1, "{written}");
    assert!(
        read(&crashes).contains("already running"),
        "{}",
        read(&crashes)
    );
}
