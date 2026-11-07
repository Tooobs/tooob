//! `tooob`: make Hosts (`tooob add`), run each in the foreground (`tooob run`), and manage them
//! from the same box. Each Host is its own process (ADR 0013); keeping one running after the
//! terminal closes is the operating system's job, never tooob's (ADR 0014). Owner commands talk to
//! a running Host over its control socket (ADR 0010).
use anyhow::{Context, Result, bail};
use std::{collections::HashMap, io::IsTerminal, path::Path, time::Duration};
use tokio::signal::unix::{SignalKind, signal};
use tooob::access::{InviteOptions, Status};
use tooob::autostart::{self, Keeper};
use tooob::awake::Awake;
use tooob::bootstrap::{self, BootstrapVersion, Problem, Watch};
use tooob::control::{Client, ControlError};
use tooob::host::{Host, HostConfig, Site};
use tooob::hosts::{
    Entry, Home, NO_RELAY, RUN_YOUR_OWN, check_relay, name_char, no_invite_links, no_key_address,
    quote,
};
use tooob::keys::{Key, Keys};
use tooob::log::{self, Log, Logged};
use tooob::upgrade::{self, Relays, ReleaseNumber, Releases};
use tooob::{Network, amount, lasting, seconds};

// musl's own allocator costs ~22% more CPU per byte served than glibc's (measured in #59), so
// Linux Releases, which are static musl builds, use mimalloc.
#[cfg(target_env = "musl")]
#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

const USAGE: &str = "usage:
  tooob add NAME TARGET (--relay URL | --local)
                                                  make a Host; TARGET is a folder or file to share
                                                  read-only, or HOST:PORT for a web app
  tooob run NAME [-d]                             serve it here, until Ctrl-C or q; d there, or
                                                  -d (--detach), keeps it running instead, now and
                                                  after logins, reboots and crashes: launchd or
                                                  systemd runs it
  tooob share PATH [--invite LABEL]... [--expires 7d] [--approve] [--name NAME]
                   [--relay URL | --local] [--uploads] [-d]
                                                  add a Host for PATH (or reuse its Host), invite
                                                  each LABEL, then run it (-d as for run); it
                                                  warns when PATH holds .git, .env or .ssh;
                                                  --uploads: its Visitors can add files to it,
                                                  never replacing one
  tooob ls [--json]                               every Host on this box, running or stopped
  tooob show NAME                                 its settings, Host Key and files
  tooob stop NAME                                 stop it; one kept running stays stopped, across
                                                  logins and reboots, until start
  tooob start NAME                                start one stop keeps stopped, the way it ran:
                                                  launchd or systemd runs it again, and keeps it
                                                  running
  tooob rm NAME                                   stop it and delete it, Host Key and all
  tooob logs NAME [-n N|all] [-f] [--json]       its log, the last 50 lines, N, or all of it;
                                                  -f follows it, --json gives an object a line
  tooob invite NAME LABEL [--approve] [--owner] [--expires 1h]
                                                  print an Invite Link; --approve: each device
                                                  it lets in waits for approve, --owner: to be
                                                  an Owner Device. Each waiting device shows a
                                                  Device Code: approve only when it's the code
                                                  tooob visitors shows
  tooob pair NAME LABEL                           Pairing Invite (this box only)
  tooob visitors NAME [--json]                    its Visitors: when each was last seen, and what
                                                  was sent to them and received from them
  tooob invites NAME [--json]                     its unused Invites
  tooob approve NAME CODE                         let in the waiting device showing Device Code
                                                  CODE (482 917), once the person who has it
                                                  reads you the same code; or by its KEY (a
                                                  prefix of six digits reads as a CODE: give 7)
  tooob block NAME KEY | unblock NAME KEY         KEY: a unique prefix is enough
  tooob revoke NAME INVITE-ID
  tooob upgrade [VERSION] [--check] [--yes] [--anyway]
                                                  install the newest Release, or VERSION (older
                                                  ones too), asking first: --check only looks,
                                                  --yes doesn't ask, --anyway even when a Host's
                                                  Relay isn't ready for it; Hosts kept running
                                                  restart on it
  tooob --version                                 the Release and Protocol Version
NAME is a Host Name, what the Host serves, or a Host Key prefix. Owner commands work whether the
Host is running or not. There's no default Relay: deploy/ shows how to run your own. Nothing is
asked but upgrade's one question: what's missing is an error that shows the command to type.
Running Hosts look for a newer Release once a day, and say when one is out.";

const VALUE_FLAGS: &[&str] = &["relay", "expires", "name", "lines"];
const BARE_FLAGS: &[&str] = &[
    "local", "json", "approve", "owner", "follow", "yes", "check", "anyway", "detach", "uploads",
];

/// Where an Owner learns to deploy a Relay.
const RUN_YOUR_OWN_RELAY: &str = "deploy/ shows how";

/// Commands for one Host, named first: `tooob VERB NAME …`. They work whether it's running or not
/// (ADR 0014).
const OWNER: &[&str] = &[
    "invite", "pair", "visitors", "invites", "block", "approve", "unblock", "revoke", "stop",
];

/// `--flag value` and bare `--flag` (None).
type Flags = HashMap<String, Option<String>>;

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        if !e.is::<Logged>() {
            eprintln!("tooob: {e:#}");
        }
        std::process::exit(1);
    }
}

/// The Release and the Protocol Version: `tooob 0.1.0 (tooob/0)`.
fn release_and_protocol_version() -> String {
    format!(
        "tooob {} ({})",
        env!("CARGO_PKG_VERSION"),
        tooob::PROTOCOL_VERSION
    )
}

async fn run() -> Result<()> {
    if std::env::args().skip(1).eq(["--version"]) {
        println!("{}", release_and_protocol_version());
        return Ok(());
    }
    let typed = Typed::new(std::env::args().skip(1).collect());
    let (words, flags, invitees) = parse(typed.args.iter().cloned())?;
    let home = Home::from_env()?;
    // add, run, ls, show and rm take exactly what they say, nothing more: `run` in particular
    // takes no settings, so a Host runs the same however it's started (ADR 0014).
    let exactly = |n: usize, allowed: &[&str]| {
        if words.len() > n {
            bail!("too many arguments: {}\n{USAGE}", words[n..].join(" "));
        }
        if let Some(other) = flags.keys().find(|f| !allowed.contains(&f.as_str())) {
            bail!("{} takes no --{other}\n{USAGE}", words[0]);
        }
        if !invitees.is_empty() {
            bail!("{} takes no --invite\n{USAGE}", words[0]);
        }
        Ok(())
    };

    match words.first().map(String::as_str) {
        Some("add") => {
            exactly(3, &["relay", "local"])?;
            let (name, target) = match (words.get(1), words.get(2)) {
                (Some(name), Some(target)) => (name, target),
                (Some(name), None) => {
                    let what = format!("say what {name} serves, a folder, a file or HOST:PORT");
                    return Err(typed.missing(&what, "TARGET"));
                }
                (None, _) => {
                    let what = "say the new Host's name and what it serves, a folder, a file or \
                                HOST:PORT";
                    return Err(typed.missing(what, "NAME TARGET"));
                }
            };
            let relay = new_host_relay(&typed, &flags).await?;
            home.add(name, target, &relay)?;
        }
        Some("run") => {
            exactly(2, &["detach"])?;
            let host = named(&home, &typed, "run", words.get(1))?;
            match flags.contains_key("detach") {
                true => {
                    let keeper = autostart::require_keeping_running(&host.name)?;
                    say(&keeper.keep_running(&host)?);
                }
                false => serve(&host, home.state_dir(), Vec::new()).await?,
            }
        }
        Some("start") => {
            exactly(2, &[])?;
            start(&named(&home, &typed, "start", words.get(1))?)?;
        }
        Some("share") => share(&home, &typed, &words, &flags, &invitees).await?,
        Some("ls") => {
            exactly(1, &["json"])?;
            ls(&home, flags.contains_key("json")).await?;
        }
        Some("show") => {
            exactly(2, &[])?;
            show_host(&named(&home, &typed, "show", words.get(1))?).await;
        }
        // Gone (#96): each says, in a line, what replaced it.
        Some("enable") => bail!(
            "enable is now run -d: tooob run {} -d",
            words.get(1).map_or("NAME".into(), |w| quote(w))
        ),
        Some("disable") => bail!(
            "disable is now stop (or rm): tooob stop {}",
            words.get(1).map_or("NAME".into(), |w| quote(w))
        ),
        Some("logs") => {
            exactly(2, &["follow", "lines", "json"])?;
            let host = named(&home, &typed, "logs", words.get(1))?;
            logs(&host, &flags).await?;
        }
        Some("rm") => {
            exactly(2, &[])?;
            let host = named(&home, &typed, "rm", words.get(1))?;
            // Stopped first, however it runs, and never started again by its service manager.
            // A command holding its lock for a moment isn't a running Host: `remove` waits it out.
            autostart::remove(&host)?;
            match Client::new(&host.state_dir).stop().await {
                Ok(()) => host.until_stopped()?,
                Err(ControlError::NotRunning) => {}
                Err(e) => return Err(refused(&host)(e)),
            }
            host.remove()?;
        }
        Some("upgrade") => {
            exactly(2, &["yes", "check", "anyway"])?;
            upgrade(&home, &typed, words.get(1), &flags).await?;
        }
        Some(command) if OWNER.contains(&command) => {
            owner_command(&home, &typed, command, &words, &flags, &invitees).await?
        }
        _ => bail!("{USAGE}"),
    }
    Ok(())
}

/// The `relay` setting `--relay URL` or `--local` asks for; None if neither says (ADR 0015).
fn relay(flags: &Flags) -> Result<Option<String>> {
    match (
        flags.contains_key("local"),
        flags.get("relay").cloned().flatten(),
    ) {
        (true, Some(_)) => bail!("--relay or --local, not both"),
        (true, None) => Ok(Some(NO_RELAY.into())),
        (false, relay) => Ok(relay),
    }
}

/// A new Host's Relay, as `--relay URL` (or `--local`) says: never a guess (ADR 0015), and never
/// asked for (ADR 0022). A Relay whose Bootstrap won't do for this tooob is refused before the Host
/// is made (ADR 0020).
async fn new_host_relay(typed: &Typed, flags: &Flags) -> Result<String> {
    let Some(relay) = relay(flags)? else {
        return Err(typed.missing(
            &format!(
                "say which Relay its Visitors reach it through (or --local for this network \
                 only; {RUN_YOUR_OWN})"
            ),
            "--relay https://RELAY/",
        ));
    };
    check_relay(&relay)?;
    if relay != NO_RELAY
        && let Err(problem) = bootstrap::check(&relay).await
    {
        bail!("no Host added: {}", problem.at(&relay));
    }
    Ok(relay)
}

/// Someone `share` invites once it's serving, as `--invite LABEL`, `--expires` and `--approve`
/// say.
struct Invitee {
    label: String,
    expires_in: u64,
    approve: bool,
}

/// `tooob share`: add a Host for a folder or file, or reuse the one already serving it, then run
/// it here, making exactly the Invites `--invite` names. Everything is checked before a Host is
/// added, so a refusal adds nothing.
async fn share(
    home: &Home,
    typed: &Typed,
    words: &[String],
    flags: &Flags,
    invitees: &[String],
) -> Result<()> {
    let Some(given) = words.get(1) else {
        return Err(typed.missing("say what to share, a folder or a file", "PATH"));
    };
    let path = std::fs::canonicalize(given).with_context(|| format!("can't share {given}"))?;
    let serves = path.to_string_lossy();
    if flags.contains_key("uploads")
        && let Some(why) = uploads_off(&path)
    {
        bail!("{given} can't take uploads: {why}");
    }
    let expires_in = match flags.get("expires").cloned().flatten() {
        Some(expires) => seconds(&expires)?,
        None => 7 * 86_400,
    };
    let who: Vec<Invitee> = invitees
        .iter()
        .map(|label| Invitee {
            label: label.clone(),
            expires_in,
            approve: flags.contains_key("approve"),
        })
        .collect();
    // A reused Host keeps its own Relay; the flags are for a new one. Invites need a Key Address,
    // so without one, `--invite` is refused.
    let found = home
        .hosts()
        .into_iter()
        .find(|h| h.settings.serves == serves);
    let name = match (&found, flags.get("name").cloned().flatten()) {
        (Some(host), _) => host.name.clone(),
        (None, Some(name)) => name,
        (None, None) => free_name(home, &path),
    };
    let keeper = match flags.contains_key("detach") {
        true => Some(autostart::require_keeping_running(&name)?),
        false => None,
    };
    let (mut host, added) = match found {
        Some(host) => {
            if !who.is_empty() {
                host.key_address()?;
            }
            (host, false)
        }
        None => {
            let relay = new_host_relay(typed, flags).await?;
            if let Some(why) = no_key_address(&relay).filter(|_| !who.is_empty()) {
                bail!(no_invite_links("the new Host", why));
            }
            (home.add(&name, &serves, &relay)?, true)
        }
    };
    if flags.contains_key("uploads") {
        host.turn_on_uploads()?;
    }
    for warning in warnings(given, &path) {
        eprintln!("tooob: warning: {warning}");
    }
    // Served here, it says so as it starts, as `tooob run` does.
    if (keeper.is_some() || autostart::is_on(&host))
        && let Some(warning) = uploads_warning(&host)
    {
        eprintln!("tooob: warning: {warning}");
    }
    // Kept running before any Invite is made: when its service manager refuses, nothing is added.
    let said = match &keeper {
        Some(keeper) => match keeper.keep_running(&host) {
            Ok(said) => said,
            Err(e) if added => {
                host.remove()?;
                return Err(e);
            }
            Err(e) => return Err(e),
        },
        None if autostart::is_on(&host) => vec![in_the_background(&host)],
        None => return serve(&host, home.state_dir(), who).await,
    };
    // Its service manager runs it, or is starting it: the Invites are made here, through the Host
    // or in its state, and never by a second copy of it (#96).
    invite_given(&host, &host.client(), &who).await?;
    say(&said);
    Ok(())
}

/// What `share` says of a Host its service manager keeps running: that it runs in the
/// background, or that it's stopped now and how to start it.
fn in_the_background(host: &Entry) -> String {
    match host.running() {
        Some(_) => autostart::already_running(&host.name),
        None => format!(
            "{0} is set to keep running, but stopped now: `tooob run {0} -d` starts it, and \
             `tooob logs {0}` says why it stopped",
            host.name
        ),
    }
}

/// Why the folder or file at `path` can't take uploads: it's the Owner's home folder, or holds it
/// (ADR 0026).
fn uploads_off(path: &Path) -> Option<String> {
    tooob::host::no_uploads_with(path, tooob::host::owners_home().as_deref())
}

/// What a Host with uploads on says as it starts: who can add files where. Never a question
/// (ADR 0022). A single file takes none, so it says nothing.
fn uploads_warning(host: &Entry) -> Option<String> {
    let serves = Path::new(&host.settings.serves);
    (host.settings.uploads && serves.is_dir() && uploads_off(serves).is_none()).then(|| {
        format!(
            "Visitors can add files to any folder in {}, except hidden ones",
            tooob::host::tilde(serves)
        )
    })
}

/// How many folders and files `share` looks through for what an Owner may not mean to hand out: a
/// whole disk would take ages.
const CHECK_LIMIT: usize = 20_000;

/// What everyone invited to the folder or file at `path` (as `given`) could read that its Owner may
/// not mean them to: the whole home folder, a repository's history, secrets. One line each, with
/// the first of each found, and a line saying so when there was too much to look through all of
/// it. Never a question (ADR 0022).
fn warnings(given: &str, path: &Path) -> Vec<String> {
    let could_read = "which everyone you invite could read";
    let mut warnings = Vec::new();
    let home = std::env::var_os("HOME").and_then(|home| std::fs::canonicalize(home).ok());
    if home.as_deref() == Some(path) {
        warnings.push(format!("{given} is your whole home folder, {could_read}"));
    }
    let mut found: [(&str, &str, Option<std::path::PathBuf>); 3] = [
        (".git", ", a repository's whole history,", None),
        (".env", ",", None),
        (".ssh", ",", None),
    ];
    let mut todo = std::collections::VecDeque::from([path.to_path_buf()]);
    let mut seen = 0;
    let mut finished = true;
    'looking: while let Some(dir) = todo.pop_front() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            seen += 1;
            if seen > CHECK_LIMIT {
                finished = false;
                break 'looking;
            }
            let name = entry.file_name();
            if let Some((_, _, first)) = found.iter_mut().find(|(n, _, _)| name == *n) {
                first.get_or_insert_with(|| entry.path().strip_prefix(path).unwrap().into());
                continue; // nothing more to find in it
            }
            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                todo.push_back(entry.path());
            }
        }
    }
    for (_, what, first) in found {
        if let Some(first) = first {
            warnings.push(format!(
                "{given} holds {}{what} {could_read}",
                first.display()
            ));
        }
    }
    if !finished {
        warnings.push(format!(
            "{given} is too big to check all of it for .git, .env or .ssh"
        ));
    }
    warnings
}

/// A Host Name for what's at `path`: its name, lowercased, with anything but letters, digits, `-`
/// and `_` turned into `-` (`My Photos` becomes `my-photos`), and `-2`, `-3`… if that's taken.
fn free_name(home: &Home, path: &Path) -> String {
    let given = path
        .file_name()
        .map_or(String::new(), |n| n.to_string_lossy().to_lowercase());
    let mut base = String::new();
    for c in given.chars() {
        let c = if name_char(c) { c } else { '-' };
        if !(c == '-' && (base.is_empty() || base.ends_with('-'))) {
            base.push(c);
        }
    }
    let base = match base.trim_end_matches('-') {
        "" => "shared".to_string(),
        base => base.to_string(),
    };
    let taken = home.names();
    (1..)
        .map(|n| match n {
            1 => base.clone(),
            n => format!("{base}-{n}"),
        })
        .find(|name| !taken.contains(name))
        .unwrap()
}

/// `tooob ls`: every Host, running or stopped.
async fn ls(home: &Home, json: bool) -> Result<()> {
    #[derive(serde::Serialize)]
    struct Row {
        name: String,
        state: &'static str,
        autostart: bool,
        /// `tooob stop` keeps it stopped, until `tooob start`.
        kept_stopped: bool,
        visitors: Option<usize>,
        /// Its Relay's Bootstrap won't do for this tooob (ADR 0020): only a running Host knows.
        bootstrap_mismatch: bool,
        serves: String,
        /// Why uploads its settings turn on are off: a Host set up before #121 on its home folder.
        uploads_off: Option<String>,
    }
    let mut rows = Vec::new();
    for name in home.names() {
        let host = match home.entry(&name) {
            Ok(host) => host,
            Err(e) => {
                eprintln!("tooob: {e:#}");
                continue;
            }
        };
        let running = host.running().is_some();
        let client = Client::new(&host.state_dir);
        let (visitors, bootstrap) = match running {
            true => (
                client.visitors().await.ok(),
                client.bootstrap_found().await.ok(),
            ),
            false => (None, None),
        };
        rows.push(Row {
            name,
            state: if running { "running" } else { "stopped" },
            autostart: autostart::is_on(&host),
            kept_stopped: !autostart::is_on(&host) && autostart::is_kept_stopped(&host),
            visitors: visitors.map(|v| v.len()),
            bootstrap_mismatch: bootstrap.is_some_and(|b| b.is_mismatch()),
            uploads_off: host
                .settings
                .uploads
                .then(|| uploads_off(Path::new(&host.settings.serves)))
                .flatten()
                .map(|why| format!("uploads are off: {why}")),
            serves: host.settings.serves,
        });
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }
    let state = |r: &Row| match r.bootstrap_mismatch {
        true => format!("{} (bootstrap mismatch)", r.state),
        false => r.state.to_string(),
    };
    let width = rows.iter().map(|r| r.name.len()).max().unwrap_or(0).max(4);
    let states = rows
        .iter()
        .map(|r| state(r).len())
        .max()
        .unwrap_or(0)
        .max(7);
    println!(
        "{:<width$}  {:<states$}  {:<9}  {:>8}  SERVES",
        "NAME", "STATE", "AUTOSTART", "VISITORS"
    );
    for r in &rows {
        let visitors = r.visitors.map_or("-".into(), |n| n.to_string());
        let autostart = match (r.autostart, r.kept_stopped) {
            (true, _) => "yes",
            (false, true) => "stopped",
            (false, false) => "no",
        };
        println!(
            "{:<width$}  {:<states$}  {autostart:<9}  {visitors:>8}  {}",
            r.name,
            state(r),
            r.serves
        );
    }
    let off: Vec<String> = rows
        .iter()
        .filter_map(|r| Some(format!("{}: {}", r.name, r.uploads_off.as_ref()?)))
        .collect();
    if !off.is_empty() {
        println!("\n{}", off.join("\n"));
    }
    if let Some(newer) = upgrade::newer_release_known(home.state_dir()) {
        println!("\n{}", upgrade::is_out(&newer));
    }
    Ok(())
}

/// `tooob show`: one Host's settings, Host Key, and where its files are; and while it runs, its
/// Relay's Bootstrap.
async fn show_host(host: &Entry) {
    let state = match host.running() {
        Some(pid) => format!("running (pid {pid})"),
        None => "stopped".into(),
    };
    println!("{}: {state}", host.name);
    let key = host.host_key().unwrap_or_else(|| "none yet".into());
    let bootstrap = match host.running() {
        Some(_) => Client::new(&host.state_dir).bootstrap_found().await.ok(),
        None => None,
    };
    for (label, value) in [
        ("serves", host.settings.serves.clone()),
        ("relay", host.settings.relay.clone()),
    ]
    .into_iter()
    .chain(bootstrap.map(|b| ("Bootstrap", b.to_string())))
    .chain([
        ("Host Key", key),
        ("settings file", host.settings_file.display().to_string()),
        ("state folder", host.state_dir.display().to_string()),
        ("log", host.log_file().display().to_string()),
        ("Autostart", autostart_shown(host)),
    ]) {
        println!("  {label:<14} {value}");
    }
}

/// Whether a Host starts by itself, and where its entry is when it does; and when `tooob stop`
/// keeps it stopped, how to start it again.
fn autostart_shown(host: &Entry) -> String {
    match autostart::entry_file(host).filter(|file| file.exists()) {
        Some(file) => format!("yes, {}", file.display()),
        None if autostart::is_kept_stopped(host) => format!(
            "no, kept stopped: `tooob start {}` starts it again",
            host.name
        ),
        None => "no".into(),
    }
}

/// `tooob VERB NAME …`: one of the Owner's commands on one Host, running or not. A change prints
/// nothing when it works; `invite` and `pair` print just the link.
async fn owner_command(
    home: &Home,
    typed: &Typed,
    command: &str,
    words: &[String],
    flags: &Flags,
    invitees: &[String],
) -> Result<()> {
    let allowed: &[&str] = match command {
        "invite" => &["expires", "approve", "owner"],
        "visitors" | "invites" => &["json"],
        _ => &[],
    };
    // A Device Code typed as shown, unquoted (`approve NAME 482 917`), is one word (#110).
    let joined;
    let words = match words {
        [_, _, a, b] if command == "approve" && [a, b].iter().all(|w| three_digits(w)) => {
            joined = [words[0].clone(), words[1].clone(), format!("{a} {b}")];
            &joined[..]
        }
        _ => words,
    };
    let arity = if after_name(command).is_some() { 3 } else { 2 };
    if words.len() > arity {
        bail!("too many arguments: {}\n{USAGE}", words[arity..].join(" "));
    }
    if let Some(other) = flags.keys().find(|f| !allowed.contains(&f.as_str())) {
        bail!("{command} takes no --{other}\n{USAGE}");
    }
    if !invitees.is_empty() {
        bail!("{command} takes no --invite\n{USAGE}");
    }
    let host = named(home, typed, command, words.get(1))?;
    if command == "stop" {
        return stop(&host).await;
    }
    let client = host.client();
    if matches!(command, "invite" | "pair") {
        host.key_address()?; // before what's missing: a link needs it
    }
    // Only the commands that take something after the Host's name ask for it.
    let (what, gap) = after_name(command).unwrap_or_default();
    let said = || {
        words
            .get(2)
            .cloned()
            .ok_or_else(|| typed.missing(what, gap))
    };
    let refused = refused(&host);
    let json = flags.contains_key("json");
    match command {
        "invite" => {
            let options = invite_options(flags)?;
            invite(&host, &client, &said()?, options).await?
        }
        "pair" => {
            let invite = client.pair(&said()?).await.map_err(refused)?;
            println!("{}", host.invite_link(&invite.token)?);
        }
        "visitors" => {
            let visitors = client.visitors().await.map_err(refused)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&visitors)?);
            } else if !visitors.is_empty() {
                print_visitors(&visitors);
            }
        }
        "invites" => {
            let invites = client.invites().await.map_err(refused)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&invites)?);
            } else if !invites.is_empty() {
                print_invites(&invites);
            }
        }
        "block" => client.block(&said()?).await.map_err(refused)?,
        "approve" => client.approve(&said()?).await.map_err(refused)?,
        "unblock" => client.unblock(&said()?).await.map_err(refused)?,
        "revoke" => client.revoke(&said()?).await.map_err(refused)?,
        _ => unreachable!(),
    }
    Ok(())
}

/// Half a Device Code as shown: three digits.
fn three_digits(word: &str) -> bool {
    word.len() == 3 && word.bytes().all(|b| b.is_ascii_digit())
}

/// `tooob invites NAME`: each unused Invite, its id, who it's for and when it expires, under a
/// header, a column each.
fn print_invites(invites: &[tooob::access::InviteInfo]) {
    let rows: Vec<[String; 3]> = std::iter::once(["ID".into(), "LABEL".into(), "EXPIRES".into()])
        .chain(
            invites
                .iter()
                .map(|i| [i.id.clone(), i.label.clone(), i.expires_at.to_string()]),
        )
        .collect();
    // As wide as its widest, in characters: a Label can be "Mamá".
    let widest = |at: usize| rows.iter().map(|r| r[at].chars().count()).max();
    let (id, label) = (widest(0).unwrap_or(0), widest(1).unwrap_or(0));
    for [i, l, expires] in &rows {
        println!("{i:<id$}  {l:<label$}  {expires}");
    }
}

/// `tooob visitors NAME`: each Visitor, where they stand, and what they've used since they first
/// got in (ADR 0025).
fn print_visitors(visitors: &[tooob::access::VisitorInfo]) {
    /// A Visitor's row, as shown.
    struct Row {
        key: String,
        label: String,
        status: String,
        code: String,
        seen: String,
        sent: String,
        received: String,
        owner_device: bool,
        /// Used an Owner Invite: it's in, and waits for approval to become an Owner Device.
        awaiting_owner_approval: bool,
    }
    let now = tooob::control::now();
    let rows: Vec<Row> = visitors
        .iter()
        .map(|v| Row {
            key: v.key.chars().take(12).collect(),
            label: v.label.clone(),
            status: match v.status {
                Status::Allowed => "allowed",
                Status::AwaitingApproval => "waiting for approval",
                Status::Blocked => "blocked",
            }
            .into(),
            code: v.device_code.clone().unwrap_or_else(|| "-".into()),
            seen: v
                .last_seen
                .map_or("-".into(), |at| ago(now.saturating_sub(at))),
            sent: amount(v.sent),
            received: amount(v.received),
            owner_device: v.owner_device,
            awaiting_owner_approval: v.awaiting_owner_approval,
        })
        .collect();
    let header = Row {
        key: "KEY".into(),
        label: "LABEL".into(),
        status: "STATUS".into(),
        code: "CODE".into(),
        seen: "LAST SEEN".into(),
        sent: "SENT".into(),
        received: "RECEIVED".into(),
        owner_device: false,
        awaiting_owner_approval: false,
    };
    // Each column as wide as its widest, in characters: a Label can be "Mamá".
    let widest = |column: fn(&Row) -> &str| {
        std::iter::once(&header)
            .chain(&rows)
            .map(|r| column(r).chars().count())
            .max()
            .unwrap_or(0)
    };
    let widths = [
        widest(|r| &r.key),
        widest(|r| &r.label),
        widest(|r| &r.status),
        widest(|r| &r.code),
        widest(|r| &r.seen),
        widest(|r| &r.sent),
        widest(|r| &r.received),
    ];
    for r in std::iter::once(&header).chain(&rows) {
        let [key, label, status, code, seen, sent, received] = widths;
        let owner_device = match (r.owner_device, r.awaiting_owner_approval) {
            (true, _) => "  owner device",
            (_, true) => "  waiting to become an owner device",
            _ => "",
        };
        println!(
            "{:<key$}  {:<label$}  {:<status$}  {:<code$}  {:<seen$}  {:>sent$}  {:>received$}{owner_device}",
            r.key, r.label, r.status, r.code, r.seen, r.sent, r.received
        );
    }
}

/// How long ago, `secs` seconds: `now` within the minute, `5 min ago`, `3 h ago`, `2 days ago`.
fn ago(secs: u64) -> String {
    match secs {
        s if s < 60 => "now".into(),
        s if s < 3600 => format!("{} min ago", s / 60),
        s if s < 86_400 => format!("{} h ago", s / 3600),
        s if s < 2 * 86_400 => "1 day ago".into(),
        s => format!("{} days ago", s / 86_400),
    }
}

/// The Host a command names, by its name, what it serves or a Host Key prefix. Never a
/// guess, and never asked for: without one, it says which Hosts there are (the running ones, to
/// stop one, and those `stop` keeps stopped, to start one) and the command to type (ADR 0022).
fn named(home: &Home, typed: &Typed, command: &str, wanted: Option<&String>) -> Result<Entry> {
    if let Some(wanted) = wanted {
        return home.find(wanted);
    }
    let mut hosts = home.hosts();
    match command {
        "stop" => hosts.retain(|h| h.running().is_some()),
        "start" => hosts.retain(|h| {
            h.running().is_none() && !autostart::is_on(h) && autostart::is_kept_stopped(h)
        }),
        _ => {}
    }
    let names: Vec<String> = hosts.into_iter().map(|h| h.name).collect();
    let which = match (command, names.as_slice()) {
        ("stop", []) => "none is running".to_string(),
        ("stop", names) => format!("running: {}", names.join(", ")),
        ("start", []) => "none is kept stopped".to_string(),
        ("start", names) => format!("kept stopped: {}", names.join(", ")),
        (_, []) => "none yet: `tooob add NAME TARGET` makes one".to_string(),
        (_, names) => format!("Hosts here: {}", names.join(", ")),
    };
    let what = match command {
        "invite" => "say which Host the Invite is for",
        "pair" => "say which Host the Pairing Invite is for",
        "stop" => "say which Host to stop",
        "start" => "say which Host to start",
        "logs" => "say which Host's log",
        _ => "say which Host",
    };
    let gap = match after_name(command) {
        Some((_, after)) => format!("NAME {after}"),
        None => "NAME".to_string(),
    };
    Err(typed.missing(&format!("{what} ({which})"), &gap))
}

/// What an Owner command takes after the Host's NAME, if anything: what to say when it's missing,
/// and its placeholder.
fn after_name(command: &str) -> Option<(&'static str, &'static str)> {
    match command {
        "invite" => Some(("say who the Invite is for", "LABEL")),
        "pair" => Some(("say which device the Pairing Invite is for", "LABEL")),
        "approve" => Some((
            "say which device, by the Device Code it shows (`tooob visitors` lists them) or its \
             Visitor Key (a unique prefix is enough)",
            "CODE",
        )),
        "block" | "unblock" => Some((
            "say which Visitor, by their Visitor Key (a unique prefix is enough)",
            "KEY",
        )),
        "revoke" => Some((
            "say which Invite, by its id (`tooob invites` lists them)",
            "INVITE-ID",
        )),
        _ => None,
    }
}

fn invite_options(flags: &Flags) -> Result<InviteOptions> {
    Ok(InviteOptions {
        first_visit_approval: flags.contains_key("approve"),
        owner_invite: flags.contains_key("owner"),
        expires_in: flags
            .get("expires")
            .cloned()
            .flatten()
            .map(|e| seconds(&e))
            .transpose()?,
    })
}

/// Make an Invite and print its link, alone on stdout so a script can take it.
async fn invite(host: &Entry, client: &Client, label: &str, options: InviteOptions) -> Result<()> {
    println!("{}", invite_link(host, client, label, options).await?);
    Ok(())
}

/// Make an Invite; its link. The Key Address comes first: without one, no Invite is made that
/// nobody would have a link for.
async fn invite_link(
    host: &Entry,
    client: &Client,
    label: &str,
    options: InviteOptions,
) -> Result<String> {
    host.key_address()?;
    let invite = client.invite(label, options).await.map_err(refused(host))?;
    host.invite_link(&invite.token)
}

/// How long an Invite works, in words: "works for 7 days".
fn lasts(options: &InviteOptions) -> String {
    format!(
        "works for {}",
        lasting(Duration::from_secs(
            options.expires_in.unwrap_or(7 * 86_400)
        ))
    )
}

/// `tooob run`: serve a Host here, holding the lock on its state, with its settings as its file
/// says now. One process per Host: a second is refused, with the first one's pid.
/// `state` is its Home's state folder, where the box's release checks are kept.
///
/// `d` in its terminal hands it to its service manager, as `run -d` does, once it has stopped
/// serving here: its lock is let go first, for the service manager's to take. Should the service
/// manager refuse it then, it serves here again rather than not at all, saying why.
async fn serve(host: &Entry, state: &Path, mut invitees: Vec<Invitee>) -> Result<()> {
    loop {
        let Some(keeper) = serve_once(host, state, std::mem::take(&mut invitees)).await? else {
            return Ok(());
        };
        // What `run -d` would say besides is said in one line, its log's.
        match keeper.keep_running(host) {
            Ok(_) => {
                let name = &host.name;
                let manager = keeper.service_manager();
                log::event(
                    "detached",
                    tooob::fields! {"service_manager" => manager},
                    &format!(
                        "detached: {manager} keeps it running now, and after logins, reboots and \
                         crashes: `tooob logs {name} -f` follows it, and `tooob stop {name}` stops it"
                    ),
                );
                return Ok(());
            }
            Err(e) => log::warning(&format!(
                "it wasn't kept running ({e:#}), so it serves here again"
            )),
        }
    }
}

/// `tooob run`, once: serve until it stops, or until `d` detaches it, when it's what keeps it
/// running that's returned, with its lock let go.
async fn serve_once(host: &Entry, state: &Path, invitees: Vec<Invitee>) -> Result<Option<Keeper>> {
    if autostart::is_on(host) && !autostart::started_by_its_entry(host) {
        bail!(
            "{0} is kept running in the background, so its service manager runs it: `tooob logs \
             {0} -f` follows it, and `tooob stop {0}` stops it",
            host.name
        );
    }
    if !invitees.is_empty() {
        host.key_address()?; // say so now, not once it's serving
    }
    let _lock = host.lock()?;
    // From here on, what it says is its log's: however it was started (ADR 0025).
    let log = log::keep(Log::open(
        &host.state_dir,
        &host.name,
        !autostart::started_by_its_entry(host),
    ));
    if let Err(e) = log::keep_output_off_the_log(&host.state_dir) {
        log.event(
            "warning",
            Default::default(),
            &format!("warning: couldn't keep what it prints out of its log: {e}"),
        );
    }
    log.take_dying_words();
    match serve_locked(host, state, invitees).await {
        Ok(detach) => Ok(detach),
        Err(e) => {
            log.event("error", Default::default(), &format!("{e:#}"));
            Err(Logged.into())
        }
    }
}

/// `tooob run`, holding the lock on its state, with its log kept; what keeps it running, once
/// `d` detaches it.
async fn serve_locked(
    host: &Entry,
    state: &Path,
    invitees: Vec<Invitee>,
) -> Result<Option<Keeper>> {
    let site = host.settings.site()?;
    let network = host
        .settings
        .network()
        .with_context(|| format!("in {}", host.settings_file.display()))?;
    // A Relay it can't reach now may be back soon (a box starting before its network): the Host
    // starts, and its checks settle it. Anything else won't do (ADR 0020).
    let bootstrap = match network {
        Network::LocalOnly => None,
        _ => Some(bootstrap::check(&host.settings.relay).await),
    };
    if let Some(Err(problem)) = &bootstrap
        && !matches!(problem, Problem::Unreachable(_))
    {
        bail!(
            "won't start ({} names its Relay): {}",
            host.settings_file.display(),
            problem.at(&host.settings.relay)
        );
    }
    serve_here(host, state, site, network, bootstrap, invitees).await
}

/// How often a running Host checks its Relay's Bootstrap again: every 10 minutes, or as
/// `TOOOB_BOOTSTRAP_CHECK_EVERY` says (`1s`, `30m`), which tests use.
fn check_every() -> Result<Duration> {
    every("TOOOB_BOOTSTRAP_CHECK_EVERY", Duration::from_secs(10 * 60))
}

/// How often a running Host's release check looks for a newer Release: once a day for the whole
/// box, or as `TOOOB_UPDATE_CHECK_EVERY` says (`1s`), which tests use.
fn release_check_every() -> Result<Duration> {
    every("TOOOB_UPDATE_CHECK_EVERY", Duration::from_secs(86_400))
}

/// The duration the environment variable `var` says (`1s`, `30m`), or `default`.
fn every(var: &str, default: Duration) -> Result<Duration> {
    let Some(every) = std::env::var_os(var) else {
        return Ok(default);
    };
    let every = every.to_string_lossy();
    match seconds(&every).with_context(|| format!("in {var}"))? {
        0 => bail!("{var} can't be 0"),
        secs => Ok(Duration::from_secs(secs)),
    }
}

/// Run the Host in this process until it's told to stop: Ctrl-C, SIGTERM from Docker or a
/// service manager, SIGHUP from a closing terminal, the Owner (`tooob stop`), or `q` in its
/// terminal; or until `d` there detaches it, when it's what keeps it running that's returned.
/// Stopping cleanly closes every Visitor's connection, so their browsers know at once. What it
/// says goes in its log.
async fn serve_here(
    host: &Entry,
    state: &Path,
    site: Site,
    network: Network,
    bootstrap: Option<Result<BootstrapVersion, Problem>>,
    invitees: Vec<Invitee>,
) -> Result<Option<Keeper>> {
    let check_every = check_every()?;
    let release_check_every = release_check_every()?;
    // Listening from before the Host starts, so a stop while it's still coming up is clean too.
    let mut interrupt = signal(SignalKind::interrupt())?;
    let mut terminate = signal(SignalKind::terminate())?;
    let mut hangup = signal(SignalKind::hangup())?;
    let signals = async {
        tokio::select! {
            _ = interrupt.recv() => {}
            _ = terminate.recv() => {}
            _ = hangup.recv() => {}
        }
    };
    tokio::pin!(signals);
    let name = &host.name;
    let serving = site.to_string();
    let local = matches!(network, Network::LocalOnly);
    let through = match &network {
        Network::LocalOnly => "no Relay: this network only".to_string(),
        Network::Relay(url) => format!("Relay {url}"),
        Network::Default => "n0's Relays".to_string(),
    };
    let relay = match &network {
        Network::Relay(url) => Some(url.to_string()),
        _ => None,
    };
    let beside = match &bootstrap {
        None => String::new(),
        Some(Ok(version)) => format!(", Bootstrap {version}"),
        Some(Err(_)) => ", Bootstrap unknown".to_string(),
    };
    let fields = tooob::fields! {
        "release" => env!("CARGO_PKG_VERSION"),
        "protocol_version" => tooob::PROTOCOL_VERSION,
    };
    let starting = format!("starting {}{beside}", release_and_protocol_version());
    log::event("starting", fields, &starting);
    if let Some(Err(problem)) = &bootstrap {
        log::warning(&format!(
            "{}; it's checked again every {}",
            problem.at(&host.settings.relay),
            lasting(check_every)
        ));
    }
    let uploads = host.settings.uploads.then(tooob::host::Uploads::default);
    if let Some(warning) = uploads_warning(host) {
        log::warning(&warning);
    }
    let config = HostConfig {
        site,
        data_dir: host.state_dir.clone(),
        network,
    };
    let running = Host::start_with(config, uploads).await?;
    let key = running.address().id.to_z32();
    let fields = tooob::fields! {"host_key" => key, "relay" => relay, "local" => local};
    log::event("host", fields, &format!("Host {key}, through {through}"));
    // Until this returns: the Host stops, and its checks with it.
    let _checking = bootstrap.map(|first| {
        let watch = Watch::new(&host.settings.relay, &first);
        bootstrap::keep_checking(watch, running.control(), check_every)
    });
    let _release_checks = (!upgrade::release_check_off())
        .then(|| upgrade::keep_checking_releases(state, release_check_every));
    // Browsers reach a Host only through its Relay, so until it's there, its links lead nowhere.
    let online = async {
        let wait = Duration::from_secs(15);
        if !local && tokio::time::timeout(wait, running.online()).await.is_err() {
            log::warning("not on its Relay yet, so Visitors' browsers can't reach it for now");
        }
    };
    let mut detach = None;
    let stopped_early = tokio::select! {
        _ = online => false,
        _ = &mut signals => true,
        _ = running.stop_requested() => true,
    };
    if !stopped_early {
        log::event("serving", Default::default(), &format!("serving {serving}"));
        // While it serves, and no longer: a Host started by hand behaves as one started by
        // itself (ADR 0014).
        let awake = Awake::hold(&format!("tooob: {name} is serving"));
        if awake.is_some() {
            log::event(
                "awake",
                Default::default(),
                "this Mac won't idle-sleep while it serves (closing its lid still sleeps it)",
            );
        }
        // The socket only: this process holds the lock, and taking it again here would release it.
        let client = Client::new(&host.state_dir);
        invite_given(host, &client, &invitees).await?;
        detach = until_stopped(host, &running, &mut signals).await;
    }
    // Bounded, whatever a Visitor or the Site is doing: never hang on the way out.
    if tokio::time::timeout(Duration::from_secs(5), running.shutdown())
        .await
        .is_err()
    {
        log::warning("didn't stop cleanly within 5 s");
    }
    // Detached, it isn't stopped: it says so once its service manager runs it (`serve`).
    if detach.is_none() {
        log::event("stopped", Default::default(), "stopped");
    }
    Ok(detach)
}

/// What a Host run in a terminal shows when it first reads keys: under what it printed as it
/// started, or once it's brought to the foreground.
const KEYS: &str = "(q) quit · (d) detach: keep it running in the background";

/// Serve until the Host is told to stop: by a signal, the Owner (`tooob stop`), or in its
/// terminal, `q`; or until `d` there detaches it, when it can be kept running: then what keeps it
/// running. Keys are read only while it's its terminal's foreground job, and never for its service
/// manager's.
async fn until_stopped(
    host: &Entry,
    running: &Host,
    signals: &mut std::pin::Pin<&mut impl std::future::Future<Output = ()>>,
) -> Option<Keeper> {
    let mut keys = match autostart::started_by_its_entry(host) {
        true => None,
        false => Keys::read(KEYS),
    };
    let stop = running.stop_requested();
    tokio::pin!(stop);
    loop {
        let key = async {
            match keys.as_mut() {
                Some(keys) => keys.next().await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            _ = &mut *signals => return None,
            _ = &mut stop => return None,
            key = key => match key {
                Key::Quit => return None,
                // Checked before it stops serving here: when it can't be kept running, it serves
                // on, saying why.
                Key::Detach => match autostart::require_keeping_running(&host.name) {
                    Ok(keeper) => return Some(keeper),
                    Err(e) => eprintln!("tooob: {e:#}"),
                },
            },
        }
    }
}

/// Make the Invites `share` was given, then print them in a block of their own, a blank line
/// before and after: who each is for and for how long, then its link on a line of its own, alone
/// on stdout so a script can take it. Nothing when there are none.
async fn invite_given(host: &Entry, client: &Client, invitees: &[Invitee]) -> Result<()> {
    let mut made = Vec::new();
    for invitee in invitees {
        let options = InviteOptions {
            first_visit_approval: invitee.approve,
            expires_in: Some(invitee.expires_in),
            ..Default::default()
        };
        let works_for = lasts(&options);
        let link = invite_link(host, client, &invitee.label, options).await?;
        made.push((&invitee.label, works_for, link));
    }
    if made.is_empty() {
        return Ok(());
    }
    eprintln!("\n  Invites · each opens in one browser");
    for (label, works_for, link) in made {
        eprintln!("\n    {label} · {works_for}");
        eprint!("    ");
        println!("{link}");
    }
    eprintln!();
    Ok(())
}

/// Tell the Owner `lines`, a line each.
fn say(lines: &[String]) {
    for line in lines {
        eprintln!("tooob: {line}");
    }
}

/// Stop a running Host, and wait until it has: through its service manager when it's kept
/// running, which then keeps it stopped across logins and reboots (ADR 0014); otherwise by asking
/// it.
async fn stop(host: &Entry) -> Result<()> {
    if autostart::stop(host)? {
        eprintln!(
            "tooob: {0} is stopped, and stays stopped across logins and reboots: `tooob start \
             {0}` starts it again",
            host.name
        );
        return Ok(());
    }
    let client = Client::new(&host.state_dir);
    client.stop().await.map_err(refused(host))?;
    host.until_stopped()
}

/// `tooob start`: a running Host is left as it is, saying how it runs; any other is started the
/// way it ran before, through its service manager, if it can be (`autostart::start`). Never in
/// the background by tooob itself (ADR 0014).
fn start(host: &Entry) -> Result<()> {
    let name = &host.name;
    if let Some(pid) = host.running() {
        eprintln!(
            "tooob: {}",
            match autostart::is_on(host) {
                true => autostart::already_running(name),
                // Run by hand since `stop`, or a `stop` of it cut short.
                false if autostart::is_kept_stopped(host) => format!(
                    "{name} is already running, but it's kept stopped: `tooob stop {name}` \
                     stops it, then `tooob start {name}` keeps it running"
                ),
                false => format!(
                    "{name} is already running here (pid {pid}), started by hand: Ctrl-C where \
                     it runs, or `tooob stop {name}`, stops it"
                ),
            }
        );
        return Ok(());
    }
    say(&autostart::start(host)?);
    Ok(())
}

/// `tooob logs`: a Host's log, the last 50 lines of it or as many as `-n` says (`all`); and with
/// `-f`, what it writes next, until Ctrl-C. With `--json`, an object a line. Last, what it said as
/// it stopped that isn't in its log yet.
async fn logs(host: &Entry, flags: &Flags) -> Result<()> {
    use std::io::Write;
    let last = match flags.get("lines").cloned().flatten() {
        None => Some(50),
        Some(all) if all == "all" => None,
        Some(n) => match n.parse::<usize>() {
            Ok(n) => Some(n),
            _ => bail!("-n takes a number of lines, or all: not {n:?}"),
        },
    };
    let mut log = log::Reader::open(&host.state_dir);
    if !log.exists() {
        eprintln!(
            "tooob: {} has no log yet: a Host starts one when it first runs",
            host.name
        );
    }
    let lines = log.so_far()?;
    let from = last.map_or(0, |n| lines.len().saturating_sub(n));
    let json = flags.contains_key("json");
    let show = |kept: &[String], dying: &[log::Line]| -> Result<()> {
        let mut out = std::io::stdout().lock();
        for line in kept {
            match json {
                true => writeln!(out, "{}", log::as_json(&host.name, line))?,
                false => writeln!(out, "{}", log::Line::parse(&host.name, line))?,
            }
        }
        for line in dying {
            match json {
                true => writeln!(out, "{}", serde_json::to_string(line)?)?,
                false => writeln!(out, "{line}")?,
            }
        }
        Ok(out.flush()?)
    };
    show(&lines[from..], &log.dying_words(&host.name)?)?;
    while flags.contains_key("follow") {
        tokio::time::sleep(Duration::from_millis(200)).await;
        show(&log.more()?, &log.dying_words(&host.name)?)?;
    }
    Ok(())
}

/// `tooob upgrade [VERSION]`: replace this box's `tooob` with the newest final Release, or the one
/// VERSION names, through that Release's own installer, only when the Owner says so (ADR 0024).
async fn upgrade(home: &Home, typed: &Typed, wanted: Option<&String>, flags: &Flags) -> Result<()> {
    let releases = Releases::from_env()?;
    let this = ReleaseNumber::this();
    let target = match wanted {
        None => {
            let newest = releases.newest().await?;
            if newest.number <= this {
                println!("tooob {this} is the newest Release");
                return Ok(());
            }
            println!(
                "tooob {} is out (this is {this}): {}",
                newest.number, newest.notes
            );
            newest
        }
        Some(wanted) => {
            let Some(version) = ReleaseNumber::parse(wanted) else {
                bail!("{wanted:?} isn't a Release: say one like 0.2.0");
            };
            if version == this {
                println!("this is tooob {this} already");
                return Ok(());
            }
            let Some(release) = releases.exact(&version).await? else {
                bail!(
                    "there's no Release {version}: https://github.com/Tooobs/tooob/releases lists \
                     them"
                );
            };
            println!("tooob {version} (this is {this}): {}", release.notes);
            release
        }
    };
    if flags.contains_key("check") {
        return Ok(());
    }
    upgrade::replaceable(&releases, &target)?;
    let version = &target.number;
    let yes = flags.contains_key("yes");
    if !yes && !std::io::stdin().is_terminal() {
        return Err(typed.missing(
            &format!("there's no terminal here to ask in, so say --yes to install tooob {version}"),
            "--yes",
        ));
    }
    let checked = match flags.contains_key("anyway") {
        true => None,
        false => Some(upgrade::check_relays(home, &target).await),
    };
    let warnings = match checked {
        None => Vec::new(),
        Some(Relays::Fine { warnings }) => warnings,
        Some(Relays::Mismatch {
            speaks,
            stranded,
            warnings,
        }) => {
            for warning in warnings {
                eprintln!("tooob: warning: {warning}");
            }
            return Err(typed.missing(
                &format!(
                    "tooob {version} speaks {speaks}, and these Hosts' Relays serve a Bootstrap \
                     that speaks another Protocol Version, so they'd be left on a Bootstrap \
                     Mismatch:\n  {}\nDeploy each Relay with tooob {version} first ({RUN_YOUR_OWN_RELAY}), \
                     or upgrade regardless",
                    stranded.join("\n  ")
                ),
                "--anyway",
            ));
        }
        Some(Relays::CantTell) => {
            return Err(typed.missing(
                &format!(
                    "tooob {version}'s notes don't say which Protocol Version it speaks, so \
                     whether your Hosts' Relays serve a Bootstrap it works with can't be checked; \
                     upgrade regardless"
                ),
                "--anyway",
            ));
        }
    };
    for warning in warnings {
        eprintln!("tooob: warning: {warning}");
    }
    if !yes && !asked_yes(version)? {
        eprintln!("tooob: nothing installed");
        return Ok(());
    }
    upgrade::install(&releases, &target).await?;
    eprintln!("tooob: tooob {version} is installed");
    restart_after_upgrade(home, version);
    Ok(())
}

/// After an upgrade to `version`: restart each running Host that starts by itself, through its
/// service manager, so it runs `version`, and name the Hosts run by hand, which keep the Release
/// they started with until they're restarted.
fn restart_after_upgrade(home: &Home, version: &ReleaseNumber) {
    let (mut restarted, mut by_hand) = (Vec::new(), Vec::new());
    for host in home.hosts() {
        let Some(pid) = host.running() else { continue };
        if !autostart::is_on(&host) {
            by_hand.push(host.name);
            continue;
        }
        match autostart::restart(&host, pid) {
            Ok(()) => restarted.push(host.name),
            Err(e) => eprintln!(
                "tooob: warning: {0} wasn't restarted ({e:#}): `tooob logs {0}` says what it \
                 did, and `tooob stop {0}` then `tooob run {0} -d` starts it afresh",
                host.name
            ),
        }
    }
    if !restarted.is_empty() {
        eprintln!(
            "tooob: restarted on tooob {version}: {}",
            restarted.join(", ")
        );
    }
    if !by_hand.is_empty() {
        eprintln!(
            "tooob: still on {} until restarted: {}",
            ReleaseNumber::this(),
            by_hand.join(", ")
        );
    }
}

/// Ask the Owner, at a terminal, whether to install `version`: `Install 0.2.0? [y/N]`, the one
/// question tooob asks (ADR 0022). Only y installs.
fn asked_yes(version: &ReleaseNumber) -> Result<bool> {
    use std::io::{BufRead, Write};
    eprint!("Install {version}? [y/N] ");
    std::io::stderr().flush()?;
    let mut answer = String::new();
    std::io::stdin().lock().read_line(&mut answer)?;
    Ok(matches!(answer.trim().to_lowercase().as_str(), "y" | "yes"))
}

/// Turn a control-socket error into a message that says what to do.
fn refused(host: &Entry) -> impl Fn(ControlError) -> anyhow::Error + Copy + '_ {
    move |e| match e {
        ControlError::NotRunning => anyhow::anyhow!(
            "{0} isn't running (start it with `tooob run {0}`)",
            host.name
        ),
        ControlError::Refused(why) => anyhow::anyhow!("{why}"),
    }
}

/// The command line as typed, to show back with a placeholder in the gap where something's missing
/// (ADR 0022): tooob never asks for it.
struct Typed {
    args: Vec<String>,
    /// Where each word (not a `--flag` or its value) is in `args`.
    words_at: Vec<usize>,
}

impl Typed {
    fn new(args: Vec<String>) -> Typed {
        let mut words_at = Vec::new();
        let mut i = 0;
        while i < args.len() {
            match flag_name(&args[i]) {
                Some(name) if name == "invite" || VALUE_FLAGS.contains(&name) => i += 1,
                Some(_) => {}
                None => words_at.push(i),
            }
            i += 1;
        }
        Typed { args, words_at }
    }

    /// Say what's missing, and under it the command as typed with `gap` after its last word,
    /// quoted so it can be pasted back: `tooob share PATH --invite sam`.
    fn missing(&self, what: &str, gap: &str) -> anyhow::Error {
        let at = self.words_at.last().map_or(0, |&i| i + 1);
        let mut shown: Vec<String> = self.args.iter().map(|a| quote(a)).collect();
        shown.insert(at.min(shown.len()), gap.to_string());
        anyhow::anyhow!("{what}:\n  tooob {}", shown.join(" "))
    }
}

/// The flag `arg` is, without its dashes; `-f` is `--follow`, `-n` is `--lines`, `-d` is
/// `--detach`. None for a word.
fn flag_name(arg: &str) -> Option<&str> {
    match arg {
        "-f" => Some("follow"),
        "-d" => Some("detach"),
        "-n" => Some("lines"),
        _ => arg.strip_prefix("--"),
    }
}

/// Words, `--flags` (value flags take the next word), and every `--invite LABEL`: it can repeat.
fn parse(args: impl Iterator<Item = String>) -> Result<(Vec<String>, Flags, Vec<String>)> {
    let (mut words, mut flags, mut invitees) = (Vec::new(), HashMap::new(), Vec::new());
    let mut args = args.peekable();
    while let Some(a) = args.next() {
        match flag_name(&a) {
            Some("invite") => invitees.push(args.next().context("--invite needs a Label")?),
            Some(name) if VALUE_FLAGS.contains(&name) => {
                let value = args.next().with_context(|| format!("{a} needs a value"))?;
                flags.insert(name.to_string(), Some(value));
            }
            Some(name) if BARE_FLAGS.contains(&name) => {
                flags.insert(name.to_string(), None);
            }
            Some("bootstrap") => bail!(
                "--bootstrap is gone: a Relay serves its Bootstrap, so a Host names only its \
                 Relay (--relay URL)\n{USAGE}"
            ),
            Some(name) => bail!("unknown option --{name}\n{USAGE}"),
            None => words.push(a),
        }
    }
    Ok((words, flags, invitees))
}
