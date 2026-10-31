//! A Host's log (ADR 0025): the Host writes it itself, in its state folder, readable only by the
//! Owner's account, however it was started. It holds a JSON object a line (`Line`: its time in
//! local time with its offset from UTC, the Host, what it says), shown as
//! `2026-10-31 14:02:11  photos: serving …`. A Host run by hand prints those lines to its terminal
//! too; one its service manager runs writes only the log, leaving its own output for its dying
//! words. Any process may write a Host's log: each line, and each roll-over, takes its lock.
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

/// The lock beside a Host's log that every process writing it takes, so lines and roll-overs
/// from a running Host and a command don't cross.
fn lock_file(state_dir: &Path) -> PathBuf {
    state_dir.join("host.log.lock")
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
/// crash log instead. Otherwise what it says as it dies would land untimed in whichever file its
/// service manager opened. Only once it holds its Host's lock: a refused second `run` says why
/// where it was started.
pub fn keep_output_off_the_log(state_dir: &Path) -> io::Result<()> {
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
        let crashes = open_private(&crash_file(state_dir))?;
        // SAFETY: both are open descriptors; dup2 points fd at what crashes is open on.
        if unsafe { libc::dup2(crashes.as_raw_fd(), fd) } < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

/// A Host's log, open for its lines.
pub struct Log {
    name: String,
    path: PathBuf,
    older: PathBuf,
    crashes: PathBuf,
    lock: PathBuf,
    /// How big it grows before it rolls over to `older`.
    limit: u64,
    /// Its lines go to stderr as well: a Host run by hand.
    echo: bool,
    open: Mutex<Open>,
}

/// What a `Log` has open: the log as last written, and its lock file.
#[derive(Default)]
struct Open {
    log: Option<File>,
    lock: Option<File>,
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
            lock: lock_file(state_dir),
            limit: limit(),
            echo,
            open: Mutex::new(Open::default()),
        }
    }

    /// Log `text`, now: a line for each of its lines.
    pub fn write(&self, text: &str) {
        let time = local_time(SystemTime::now());
        let lines: Vec<Line> = text
            .split('\n')
            .map(|text| Line::new(Some(time.clone()), &self.name, text))
            .collect();
        self.record(&lines);
    }

    /// Log an `event`, now: `text` for people, one line, and `fields` for `tooob logs --json`.
    pub fn event(&self, event: &str, fields: Fields, text: &str) {
        let time = local_time(SystemTime::now());
        let mut line = Line::new(Some(time), &self.name, &text.replace('\n', " "));
        line.event = Some(event.to_string());
        line.fields = (!fields.is_empty()).then_some(fields);
        self.record(&[line]);
    }

    /// Append `lines` to the log, and print them where they're printed.
    fn record(&self, lines: &[Line]) {
        let json: String = lines
            .iter()
            .filter_map(|line| serde_json::to_string(line).ok())
            .map(|line| line + "\n")
            .collect();
        let written = self.locked(|open| self.append(open, &json));
        // Never lost: a log that can't be written leaves its lines on stderr.
        if self.echo || written.is_err() {
            let shown: String = lines.iter().map(|line| format!("{line}\n")).collect();
            eprint!("{shown}");
        }
    }

    /// Move the Host's dying words, what its crash log holds since it last started, into its log,
    /// as of when they were written, and empty the crash log: they're said once. Under the log's
    /// lock, so no other tooob's are lost between reading them and emptying it.
    pub fn take_dying_words(&self) {
        let taken = self.locked(|_| {
            let mut crashes = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&self.crashes)?;
            let said = last_of(&mut crashes)?;
            let at = crashes.metadata()?.modified()?;
            crashes.set_len(0)?;
            crashes.set_permissions(std::fs::Permissions::from_mode(0o600))?;
            Ok((said, at))
        });
        if let Ok((said, at)) = taken
            && !said.is_empty()
        {
            let time = Some(local_time(at));
            self.record(&dying_words(&self.name, time, &said, TAKEN));
        }
    }

    /// Run `f` holding the log's lock, for this process's threads and every other process.
    fn locked<T>(&self, f: impl FnOnce(&mut Open) -> io::Result<T>) -> io::Result<T> {
        use std::os::fd::AsRawFd;
        let mut open = self.open.lock().unwrap_or_else(|e| e.into_inner());
        if open.lock.is_none() {
            open.lock = Some(open_private(&self.lock)?);
        }
        let fd = open.lock.as_ref().unwrap().as_raw_fd();
        // SAFETY: fd is the lock file's, open for as long as this Log is.
        while unsafe { libc::flock(fd, libc::LOCK_EX) } != 0 {
            let e = io::Error::last_os_error();
            if e.kind() != io::ErrorKind::Interrupted {
                return Err(e);
            }
        }
        let done = f(&mut open);
        // SAFETY: as above.
        unsafe { libc::flock(fd, libc::LOCK_UN) };
        done
    }

    /// Append `lines` to the log, holding its lock, rolling it over first when they'd take it past
    /// its limit. The log at its path is the one written: opened again if it has gone (rolled over
    /// by another process, removed), so its size is checked on the file that's there.
    fn append(&self, open: &mut Open, lines: &str) -> io::Result<()> {
        let here = std::fs::metadata(&self.path).ok();
        let gone = match (open.log.as_ref(), &here) {
            (Some(log), Some(here)) => log.metadata()?.ino() != here.ino(),
            _ => true,
        };
        if gone {
            open.log = Some(open_private(&self.path)?);
        }
        let size = open.log.as_ref().unwrap().metadata()?.len();
        if size > 0 && size + lines.len() as u64 > self.limit {
            std::fs::rename(&self.path, &self.older)?;
            open.log = Some(open_private(&self.path)?);
        }
        open.log.as_mut().unwrap().write_all(lines.as_bytes())
    }
}

/// What starts the dying words a Host's next start moves into its log.
const TAKEN: &str = "before it stopped last time, it said:";

/// What starts the dying words `tooob logs` shows that aren't in the log yet.
const NOT_TAKEN: &str = "before it stopped last time, it said (not in its log yet):";

/// The last of what a crash log holds, a line each: enough to say why, since a Host failing as it
/// starts, over and over, may have said it often.
fn last_of(crashes: &mut File) -> io::Result<Vec<String>> {
    use std::io::{Read, Seek, SeekFrom};
    const ENOUGH: u64 = 64 * 1024;
    let len = crashes.metadata()?.len();
    crashes.seek(SeekFrom::Start(len.saturating_sub(ENOUGH)))?;
    let mut said = Vec::new();
    crashes.read_to_end(&mut said)?;
    let lines = split(&said);
    Ok(lines[lines.len().saturating_sub(50)..].to_vec())
}

/// `said`, a Host's dying words, as lines of its log at `time`: `heading`, then each, indented.
fn dying_words(host: &str, time: Option<String>, said: &[String], heading: &str) -> Vec<Line> {
    std::iter::once(heading.to_string())
        .chain(said.iter().map(|line| format!("  {line}")))
        .map(|text| Line::new(time.clone(), host, &text))
        .collect()
}

/// How big a log grows before it rolls over: about 1 MB, so a Host keeps at most about 2 MB of
/// log (ADR 0025); `TOOOB_LOG_LIMIT` (bytes) says otherwise, which tests use.
fn limit() -> u64 {
    std::env::var("TOOOB_LOG_LIMIT")
        .ok()
        .and_then(|limit| limit.parse().ok())
        .unwrap_or(1_000_000)
}

/// One line of a Host's log, as it's kept there: a JSON object of when, which Host, and what it
/// says. Fields a later tooob adds are kept by `tooob logs --json`, which passes each line through.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Line {
    /// In local time, with its offset from UTC: `2026-10-31T14:02:11-07:00`. None for a line the
    /// Host didn't write itself (one 0.2.0 wrote, or a crash's).
    pub time: Option<String>,
    pub host: String,
    pub text: String,
    /// What happened, for a line that's one of the Host's events (`got_in`, `blocked`…).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
    /// The event's particulars, so `tooob logs --json` needn't be read from its text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fields: Option<Fields>,
}

/// An event's particulars: `{"label": "dave", "key": "3f2a91c0", …}`.
pub type Fields = serde_json::Map<String, serde_json::Value>;

/// `fields!{"label" => label, "key" => key}`: an event's particulars.
#[macro_export]
macro_rules! fields {
    ($($name:expr => $value:expr),* $(,)?) => {{
        #[allow(unused_mut)]
        let mut fields = $crate::log::Fields::new();
        $(fields.insert($name.to_string(), serde_json::json!($value));)*
        fields
    }};
}

impl Line {
    fn new(time: Option<String>, host: &str, text: &str) -> Line {
        Line {
            time,
            host: host.to_string(),
            text: text.to_string(),
            event: None,
            fields: None,
        }
    }

    /// A line of `host`'s log as it's kept there: its object, or for one that isn't, the line as
    /// it is, with no time.
    pub fn parse(host: &str, kept: &str) -> Line {
        serde_json::from_str(kept).unwrap_or_else(|_| Line::new(None, host, kept))
    }

    /// Whether the Host wrote this line itself, with its time.
    pub fn is_timed(&self) -> bool {
        self.time.is_some()
    }
}

/// `2026-10-31 14:02:11  photos: serving …`: its local time to the second, without its offset.
impl std::fmt::Display for Line {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let Some(time) = &self.time else {
            return f.write_str(&self.text);
        };
        match (time.get(..10), time.get(11..19)) {
            (Some(day), Some(clock)) => write!(f, "{day} {clock}  {}: {}", self.host, self.text),
            _ => write!(f, "{time}  {}: {}", self.host, self.text),
        }
    }
}

/// A line of `host`'s log as `tooob logs --json` shows it: as it's kept, when it's an object; an
/// object made for it when it isn't.
pub fn as_json(host: &str, kept: &str) -> String {
    match serde_json::from_str::<Line>(kept) {
        Ok(_) => kept.to_string(),
        Err(_) => serde_json::to_string(&Line::new(None, host, kept)).unwrap_or_default(),
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

/// `at` in local time, to the second, with its offset from UTC: `2026-10-31T14:02:11-07:00`. In
/// UTC (`+00:00`) if the local time can't be told.
pub fn local_time(at: SystemTime) -> String {
    let secs = at.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()) as libc::time_t;
    // SAFETY: tzset reads TZ and the zone files into libc's own state, which a setenv at the same
    // time could change under it: tooob never calls setenv. localtime_r and gmtime_r write only
    // the tm they're given, which is ours and big enough.
    let tm = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        tzset();
        if libc::localtime_r(&secs, &mut tm).is_null() {
            tm = std::mem::zeroed();
            if libc::gmtime_r(&secs, &mut tm).is_null() {
                return "1970-01-01T00:00:00+00:00".to_string();
            }
            tm.tm_gmtoff = 0;
        }
        tm
    };
    let east = tm.tm_gmtoff;
    let (sign, off) = if east < 0 { ('-', -east) } else { ('+', east) };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}{sign}{:02}:{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec,
        off / 3600,
        off % 3600 / 60,
    )
}

unsafe extern "C" {
    /// POSIX's, which the libc crate doesn't have: reads TZ, which localtime_r needn't.
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

/// Log an `event` in this process's Host's log (`Log::event`); its text on stderr without one.
pub fn event(event: &str, fields: Fields, text: &str) {
    match KEPT.get() {
        Some(log) => log.event(event, fields, text),
        None => eprintln!("tooob: {text}"),
    }
}

/// Log `text`, a line about a Visitor's connection that may carry what it or its browser sent (an
/// error from WebRTC, iroh or a bad request), with any address in it taken out: where Visitors
/// are is never logged (ADR 0025).
pub fn about_a_visitor(text: &str) {
    line(&without_addresses(text));
}

/// `text` with each IP address in it (with a port or not, IPv6 in brackets or not) and each mDNS
/// name a browser gives its own (`….local`) said as `an address`.
pub fn without_addresses(text: &str) -> String {
    let part =
        |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | ':' | '[' | ']' | '%' | '-');
    let mut said = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(part) {
        said.push_str(&rest[..start]);
        let len = rest[start..]
            .find(|c| !part(c))
            .unwrap_or(rest.len() - start);
        let word = &rest[start..start + len];
        // A sentence's own punctuation after it isn't part of it.
        let bare = word.trim_end_matches(['.', ':', '-']);
        if is_an_address(bare) {
            said.push_str("an address");
            said.push_str(&word[bare.len()..]);
        } else {
            said.push_str(word);
        }
        rest = &rest[start + len..];
    }
    said.push_str(rest);
    said
}

fn is_an_address(word: &str) -> bool {
    use std::net::{IpAddr, SocketAddr};
    let unzoned = |w: &str| w.split('%').next().unwrap_or(w).to_string();
    let plain = unzoned(word.trim_matches(['[', ']']));
    word.parse::<SocketAddr>().is_ok()
        || plain.parse::<IpAddr>().is_ok()
        || word.rsplit_once(':').is_some_and(|(ip, port)| {
            port.parse::<u16>().is_ok()
                && unzoned(ip.trim_matches(['[', ']']))
                    .parse::<IpAddr>()
                    .is_ok()
        })
        || (word.len() > ".local".len() && word.ends_with(".local"))
}

/// Log `warning: text` in this process's Host's log, as a `warning` event.
pub fn warning(text: &str) {
    event("warning", Fields::new(), &format!("warning: {text}"));
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
/// written since, across a roll-over. Lines as they're kept: `Line::parse` reads one.
pub struct Reader {
    path: PathBuf,
    older: PathBuf,
    crashes: PathBuf,
    /// How much of the crash log has been shown.
    crashes_shown: u64,
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
            crashes: crash_file(state_dir),
            crashes_shown: 0,
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

impl Reader {
    /// What `host` said as it last stopped that isn't in its log yet (its next start moves it
    /// there), since this was last asked: a line saying so, then each. A Host its service manager
    /// gave up restarting would otherwise never say why.
    pub fn dying_words(&mut self, host: &str) -> io::Result<Vec<Line>> {
        use std::io::{Read, Seek, SeekFrom};
        let Ok(mut crashes) = File::open(&self.crashes) else {
            self.crashes_shown = 0;
            return Ok(Vec::new());
        };
        let meta = crashes.metadata()?;
        if meta.len() < self.crashes_shown {
            self.crashes_shown = 0; // moved into the log, and more said since
        }
        let first = self.crashes_shown == 0;
        crashes.seek(SeekFrom::Start(self.crashes_shown))?;
        let mut said = Vec::new();
        crashes.read_to_end(&mut said)?;
        let whole = said
            .iter()
            .rposition(|&b| b == b'\n')
            .map_or(0, |at| at + 1);
        self.crashes_shown += whole as u64;
        let said = split(&said[..whole]);
        if said.is_empty() {
            return Ok(Vec::new());
        }
        let time = Some(local_time(meta.modified()?));
        Ok(match first {
            true => dying_words(host, time, &said, NOT_TAKEN),
            false => said
                .iter()
                .map(|line| Line::new(time.clone(), host, &format!("  {line}")))
                .collect(),
        })
    }
}

/// `text`'s lines, without their newlines.
fn split(text: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(text)
        .lines()
        .map(String::from)
        .collect()
}
