//! A Host's log (ADR 0025): the Host writes it itself, in its state folder, readable only by the
//! Owner's account, however it was started. Each line starts with its time in local time
//! (`2026-10-31 14:02:11  photos: serving …`). A Host run by hand prints the same lines to its
//! terminal; one its service manager runs writes only the log, leaving its own output for its dying
//! words.
use std::{
    fs::File,
    io::{self, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

/// A Host's log, in its state folder.
pub fn file(state_dir: &Path) -> PathBuf {
    state_dir.join("host.log")
}

/// The log a Host's log rolls over to, replacing the one before.
pub fn older_file(state_dir: &Path) -> PathBuf {
    state_dir.join("host.log.1")
}

/// Where a Host's service manager puts what the Host prints itself, which for a Host the service
/// manager runs is only its dying words (a panic, a refusal before its log is open): its next start
/// moves them into its log.
pub fn crash_file(state_dir: &Path) -> PathBuf {
    state_dir.join("crash.log")
}

/// Make a Host's crash log if it isn't there, readable only by the Owner's account, for its service
/// manager to append to.
pub fn make_crash_file(state_dir: &Path) -> io::Result<()> {
    open_private(&crash_file(state_dir)).map(drop)
}

/// Keep what this process prints off the Host's log: where its stdout or stderr is the log (or
/// the one it rolled over to), as an Autostart entry from tooob 0.2.0 has it, it goes to the
/// crash log instead. Otherwise each line it printed would be in its log twice, and what it says
/// as it dies, untimed, in whichever file its service manager opened.
pub fn keep_output_off_the_log(state_dir: &Path) {
    use std::os::fd::AsRawFd;
    let logs: Vec<(u64, u64)> = [file(state_dir), older_file(state_dir)]
        .iter()
        .filter_map(|log| std::fs::metadata(log).ok())
        .map(|log| (log.dev(), log.ino()))
        .collect();
    for fd in [libc::STDOUT_FILENO, libc::STDERR_FILENO] {
        // SAFETY: fd stays open, and is never closed through this File, which isn't dropped.
        let output = std::mem::ManuallyDrop::new(unsafe {
            <File as std::os::fd::FromRawFd>::from_raw_fd(fd)
        });
        let Ok(output) = output.metadata() else {
            continue;
        };
        if !logs.contains(&(output.dev(), output.ino())) {
            continue;
        }
        if let Ok(crashes) = open_private(&crash_file(state_dir)) {
            // SAFETY: both are open descriptors; dup2 points fd at what crashes is open on.
            unsafe { libc::dup2(crashes.as_raw_fd(), fd) };
        }
    }
}

/// A Host's log, open for its lines.
pub struct Log {
    name: String,
    path: PathBuf,
    older: PathBuf,
    crashes: PathBuf,
    /// How big it grows before it rolls over to `older`.
    limit: u64,
    /// Its lines go to stderr as well: a Host run by hand.
    echo: bool,
    file: Mutex<Option<File>>,
}

impl Log {
    /// The log of the Host named `name`, whose state folder is `state_dir`; `echo` prints each line
    /// to stderr too. Opened as it's first written.
    pub fn open(state_dir: &Path, name: &str, echo: bool) -> Log {
        Log {
            name: name.to_string(),
            path: file(state_dir),
            older: older_file(state_dir),
            crashes: crash_file(state_dir),
            limit: limit(),
            echo,
            file: Mutex::new(None),
        }
    }

    /// Log `text`, now: a line for each of its lines, each with the time and the Host's name.
    pub fn write(&self, text: &str) {
        self.write_at(&local_time(SystemTime::now()), text);
    }

    fn write_at(&self, time: &str, text: &str) {
        let lines: String = text
            .split('\n')
            .map(|text| {
                let line = Line {
                    time: Some(time.to_string()),
                    host: self.name.clone(),
                    text: text.to_string(),
                };
                format!("{line}\n")
            })
            .collect();
        let written = self.append(&lines);
        // Never lost: a log that can't be written leaves its lines on stderr.
        if self.echo || written.is_err() {
            eprint!("{lines}");
        }
    }

    /// Move the Host's dying words, what its crash log holds since it last started, into its log,
    /// as of when they were written, and empty the crash log: they're said once.
    pub fn take_dying_words(&self) {
        use std::io::{Read, Seek, SeekFrom};
        // Enough to say why: a Host failing as it starts, over and over, may have said it often.
        const ENOUGH: u64 = 64 * 1024;
        let Ok(mut crashes) = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&self.crashes)
        else {
            return;
        };
        let Ok(meta) = crashes.metadata() else { return };
        let mut said = Vec::new();
        let _ = crashes.seek(SeekFrom::Start(meta.len().saturating_sub(ENOUGH)));
        let _ = crashes.read_to_end(&mut said);
        let lines = split(&said);
        let lines = &lines[lines.len().saturating_sub(50)..];
        if !lines.is_empty() {
            let at = meta.modified().unwrap_or_else(|_| SystemTime::now());
            let text: Vec<String> = std::iter::once("it last stopped saying, in crash.log:".into())
                .chain(lines.iter().map(|line| format!("  {line}")))
                .collect();
            self.write_at(&local_time(at), &text.join("\n"));
        }
        let _ = crashes.set_len(0);
        let _ = crashes.set_permissions(std::fs::Permissions::from_mode(0o600));
    }

    /// Append `lines` to the log, rolling it over first when they'd take it past its limit. The
    /// log at its path is the one written: opened again if it has gone (rolled over, removed).
    fn append(&self, lines: &str) -> io::Result<()> {
        let mut file = self.file.lock().unwrap_or_else(|e| e.into_inner());
        let here = std::fs::metadata(&self.path).ok();
        let gone = match (file.as_ref(), &here) {
            (Some(open), Some(here)) => open.metadata()?.ino() != here.ino(),
            _ => true,
        };
        if gone {
            *file = Some(open_private(&self.path)?);
        }
        let size = file.as_ref().unwrap().metadata()?.len();
        if size > 0 && size + lines.len() as u64 > self.limit {
            std::fs::rename(&self.path, &self.older)?;
            *file = Some(open_private(&self.path)?);
        }
        file.as_mut().unwrap().write_all(lines.as_bytes())
    }
}

/// How big a log grows before it rolls over: about 1 MB, so a Host keeps at most about 2 MB of
/// log (ADR 0025); `TOOOB_LOG_LIMIT` (bytes) says otherwise, which tests use.
fn limit() -> u64 {
    std::env::var("TOOOB_LOG_LIMIT")
        .ok()
        .and_then(|limit| limit.parse().ok())
        .unwrap_or(1_000_000)
}

/// One line of a Host's log: when, which Host, and what it says. In the log, and printed,
/// `2026-10-31 14:02:11  photos: serving …`; with `tooob logs --json`, an object of these fields,
/// its time as `2026-10-31T14:02:11`.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Line {
    /// In local time, `2026-10-31 14:02:11`; None for a line the Host didn't write itself (a
    /// crash's, under an older tooob's Autostart).
    #[serde(serialize_with = "iso")]
    pub time: Option<String>,
    pub host: String,
    pub text: String,
}

impl Line {
    /// A line of `host`'s log, as it's written there.
    pub fn parse(host: &str, line: &str) -> Line {
        let timed = line.get(..21).filter(|head| {
            head.char_indices().all(|(i, c)| match i {
                4 | 7 => c == '-',
                10 | 19 | 20 => c == ' ',
                13 | 16 => c == ':',
                _ => c.is_ascii_digit(),
            })
        });
        let Some(head) = timed else {
            return Line {
                time: None,
                host: host.to_string(),
                text: line.to_string(),
            };
        };
        let rest = &line[21..];
        let text = rest
            .strip_prefix(host)
            .and_then(|rest| rest.strip_prefix(": "))
            .unwrap_or(rest);
        Line {
            time: Some(head[..19].to_string()),
            host: host.to_string(),
            text: text.to_string(),
        }
    }
}

impl std::fmt::Display for Line {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match &self.time {
            Some(time) => write!(f, "{time}  {}: {}", self.host, self.text),
            None => f.write_str(&self.text),
        }
    }
}

/// A log line's time for JSON: `2026-10-31T14:02:11`, ISO 8601's local time.
fn iso<S: serde::Serializer>(time: &Option<String>, to: S) -> Result<S::Ok, S::Error> {
    match time {
        Some(time) => to.serialize_str(&time.replacen(' ', "T", 1)),
        None => to.serialize_none(),
    }
}

/// `path`, made if it isn't there, for appending; readable only by the Owner's account, as an
/// older one is made too.
fn open_private(path: &Path) -> io::Result<File> {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)?;
    if file.metadata()?.mode() & 0o077 != 0 {
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(file)
}

/// `at` in local time, to the second: `2026-10-31 14:02:11`.
pub fn local_time(at: SystemTime) -> String {
    let secs = at.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()) as libc::time_t;
    // SAFETY: tzset reads TZ and the zone files into libc's own state; localtime_r writes only the
    // tm it's given, which is ours and big enough.
    let tm = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        tzset();
        libc::localtime_r(&secs, &mut tm);
        tm
    };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec
    )
}

unsafe extern "C" {
    /// POSIX's: reads TZ, which localtime_r needn't.
    fn tzset();
}

/// This process's Host's log, once it's kept: one Host to a process (ADR 0013).
static KEPT: OnceLock<Log> = OnceLock::new();

/// Keep `log` as this process's Host's log: where `line` writes from now on.
pub fn keep(log: Log) -> &'static Log {
    KEPT.get_or_init(|| log)
}

/// Log `text` in this process's Host's log; on stderr, as it always was, without one.
pub fn line(text: &str) {
    match KEPT.get() {
        Some(log) => log.write(text),
        None => eprintln!("tooob: {text}"),
    }
}

/// An error already in the Host's log, and printed with it where it's printed: nothing more to say.
#[derive(Debug)]
pub struct Logged;

impl std::fmt::Display for Logged {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("logged")
    }
}

impl std::error::Error for Logged {}

/// A Host's log as it's read: its older file's lines, then its own, and with `more`, the lines
/// written since, across a roll-over.
pub struct Reader {
    path: PathBuf,
    older: PathBuf,
    /// The log as it was last read, there.
    current: Option<File>,
    /// The end of a line still being written.
    partial: Vec<u8>,
}

impl Reader {
    /// The log of the Host whose state folder is `state_dir`.
    pub fn open(state_dir: &Path) -> Reader {
        Reader {
            path: file(state_dir),
            older: older_file(state_dir),
            current: None,
            partial: Vec::new(),
        }
    }

    /// Whether there's a log at all: a Host starts one when it first runs.
    pub fn exists(&self) -> bool {
        self.path.exists() || self.older.exists()
    }

    /// Every line in the log so far, oldest first, without their newlines.
    pub fn so_far(&mut self) -> io::Result<Vec<String>> {
        let mut lines = match std::fs::read(&self.older) {
            Ok(older) => split(&older),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e),
        };
        lines.extend(self.more()?);
        Ok(lines)
    }

    /// The whole lines written since the last read: what's left of the file read before, then,
    /// when it has been rolled over, removed or emptied, the new one from its start.
    pub fn more(&mut self) -> io::Result<Vec<String>> {
        use std::io::{Read, Seek};
        let here = std::fs::metadata(&self.path).ok();
        let mut read = std::mem::take(&mut self.partial);
        if let Some(open) = self.current.as_mut() {
            open.read_to_end(&mut read)?;
        }
        let afresh = match (self.current.as_mut(), &here) {
            (_, None) => false,
            (None, Some(_)) => true,
            (Some(open), Some(here)) => {
                open.metadata()?.ino() != here.ino() || here.len() < open.stream_position()?
            }
        };
        if afresh {
            if !read.is_empty() && !read.ends_with(b"\n") {
                read.push(b'\n');
            }
            self.current = File::open(&self.path).ok();
            if let Some(open) = self.current.as_mut() {
                open.read_to_end(&mut read)?;
            }
        }
        let whole = read
            .iter()
            .rposition(|&b| b == b'\n')
            .map_or(0, |at| at + 1);
        self.partial = read.split_off(whole);
        Ok(split(&read))
    }
}

/// `text`'s lines, without their newlines.
fn split(text: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(text)
        .lines()
        .map(String::from)
        .collect()
}
