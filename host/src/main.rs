//! `tooob`: make Hosts (`tooob add`), run each in the foreground (`tooob run`), and manage them
//! from the same box. Each Host is its own process (ADR 0013); keeping one running after the
//! terminal closes is the operating system's job, never tooob's (ADR 0014). Owner commands talk to
//! a running Host over its control socket (ADR 0010).
use anyhow::{Context, Result, bail};
use std::{collections::HashMap, path::Path, time::Duration};
use tokio::signal::unix::{SignalKind, signal};
use tooob::Network;
use tooob::access::InviteOptions;
use tooob::autostart;
use tooob::awake::Awake;
use tooob::bootstrap::{self, BootstrapVersion, Problem, Watch};
use tooob::control::{Client, ControlError};
use tooob::host::{Host, HostConfig, Site};
use tooob::hosts::{
    Entry, Home, NO_RELAY, RUN_YOUR_OWN, check_relay, name_char, no_invite_links, no_key_address,
    quote,
};

// musl's own allocator costs ~22% more CPU per byte served than glibc's (measured in #59), so
// Linux Releases, which are static musl builds, use mimalloc.
#[cfg(target_env = "musl")]
#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

const USAGE: &str = "usage:
  tooob add NAME TARGET (--relay URL | --local)
                                                  make a Host; TARGET is a folder or file to share
                                                  read-only, or HOST:PORT for a web app
  tooob run NAME                                  serve it here, until Ctrl-C
  tooob share PATH [--invite LABEL]... [--expires 7d] [--approve] [--name NAME]
                   [--relay URL | --local]
                                                  add a Host for PATH (or reuse its Host), invite
                                                  each LABEL, then run it; it warns when PATH
                                                  holds .git, .env or .ssh
  tooob ls [--json]                               every Host on this box, running or stopped
  tooob show NAME                                 its settings, Host Key and files
  tooob rm NAME                                   delete it, Host Key and all
  tooob stop NAME                                 end a running Host
  tooob enable NAME                               start it by itself, now, at login and again after
                                                  a failure (launchd or systemd runs it)
  tooob disable NAME                              stop it, and stop starting it by itself
  tooob logs NAME [-f]                            the log of a Host that starts by itself; -f
                                                  follows it
  tooob invite NAME LABEL [--approve] [--owner] [--expires 1h]  print an Invite Link
  tooob pair NAME LABEL                           Pairing Invite (this box only)
  tooob visitors NAME [--json] | invites NAME [--json]
  tooob block NAME KEY | approve NAME KEY | unblock NAME KEY   KEY: a unique prefix is enough
  tooob revoke NAME INVITE-ID
  tooob --version                                 the Release and Protocol Version
NAME is a Host Name, what the Host serves, or a Host Key prefix. Owner commands work whether the
Host is running or not. There's no default Relay: deploy/ shows how to run your own. Nothing is
ever asked: what's missing is an error that shows the command to type.";

const VALUE_FLAGS: &[&str] = &["relay", "expires", "name"];
const BARE_FLAGS: &[&str] = &["local", "json", "approve", "owner", "follow"];

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
        eprintln!("tooob: {e:#}");
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
            exactly(2, &[])?;
            serve(&named(&home, &typed, "run", words.get(1))?, Vec::new()).await?;
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
        Some("enable") => {
            exactly(2, &[])?;
            let said = autostart::enable(&named(&home, &typed, "enable", words.get(1))?)?;
            for line in said {
                eprintln!("tooob: {line}");
            }
        }
        Some("disable") => {
            exactly(2, &[])?;
            autostart::disable(&named(&home, &typed, "disable", words.get(1))?)?;
        }
        Some("logs") => {
            exactly(2, &["follow"])?;
            let host = named(&home, &typed, "logs", words.get(1))?;
            logs(&host, flags.contains_key("follow")).await?;
        }
        Some("rm") => {
            exactly(2, &[])?;
            let host = named(&home, &typed, "rm", words.get(1))?;
            if autostart::is_on(&host) {
                bail!(
                    "{0} starts by itself: turn that off first with `tooob disable {0}`",
                    host.name
                );
            }
            host.remove()?;
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
    let host = match home
        .hosts()
        .into_iter()
        .find(|h| h.settings.serves == serves)
    {
        Some(host) => {
            if !who.is_empty() {
                host.key_address()?;
            }
            host
        }
        None => {
            let relay = new_host_relay(typed, flags).await?;
            if let Some(why) = no_key_address(&relay).filter(|_| !who.is_empty()) {
                bail!(no_invite_links("the new Host", why));
            }
            let name = match flags.get("name").cloned().flatten() {
                Some(name) => name,
                None => free_name(home, &path),
            };
            home.add(&name, &serves, &relay)?
        }
    };
    for warning in warnings(given, &path) {
        eprintln!("tooob: warning: {warning}");
    }
    serve(&host, who).await
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
        visitors: Option<usize>,
        /// Its Relay's Bootstrap won't do for this tooob (ADR 0020): only a running Host knows.
        bootstrap_mismatch: bool,
        serves: String,
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
            visitors: visitors.map(|v| v.len()),
            bootstrap_mismatch: bootstrap.is_some_and(|b| b.is_mismatch()),
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
        let autostart = if r.autostart { "yes" } else { "no" };
        println!(
            "{:<width$}  {:<states$}  {autostart:<9}  {visitors:>8}  {}",
            r.name,
            state(r),
            r.serves
        );
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

/// Whether a Host starts by itself, and where its entry is when it does.
fn autostart_shown(host: &Entry) -> String {
    match autostart::entry_file(host).filter(|file| file.exists()) {
        Some(file) => format!("yes, {}", file.display()),
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
            } else {
                for v in visitors {
                    let owner_device = if v.owner_device { "  owner device" } else { "" };
                    println!(
                        "{}  {:<20} {:?}{owner_device}",
                        &v.key[..12],
                        v.label,
                        v.status
                    );
                }
            }
        }
        "invites" => {
            let invites = client.invites().await.map_err(refused)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&invites)?);
            } else {
                for i in invites {
                    println!("{}  {:<20} expires {}", i.id, i.label, i.expires_at);
                }
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

/// The Host a command names, by its name, what it serves or a Host Key prefix. Never a
/// guess, and never asked for: without one, it says which Hosts there are (the running ones, to
/// stop one) and the command to type (ADR 0022).
fn named(home: &Home, typed: &Typed, command: &str, wanted: Option<&String>) -> Result<Entry> {
    if let Some(wanted) = wanted {
        return home.find(wanted);
    }
    let mut hosts = home.hosts();
    match command {
        "stop" => hosts.retain(|h| h.running().is_some()),
        "disable" => hosts.retain(autostart::is_on),
        _ => {}
    }
    let names: Vec<String> = hosts.into_iter().map(|h| h.name).collect();
    let which = match (command, names.as_slice()) {
        ("stop", []) => "none is running".to_string(),
        ("stop", names) => format!("running: {}", names.join(", ")),
        ("disable", []) => "none starts by itself".to_string(),
        ("disable", names) => format!("starting by itself: {}", names.join(", ")),
        (_, []) => "none yet: `tooob add NAME TARGET` makes one".to_string(),
        (_, names) => format!("Hosts here: {}", names.join(", ")),
    };
    let what = match command {
        "invite" => "say which Host the Invite is for",
        "pair" => "say which Host the Pairing Invite is for",
        "stop" => "say which Host to stop",
        "logs" => "say which Host's log",
        "enable" => "say which Host to start by itself",
        "disable" => "say which Host to stop starting by itself",
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
        "block" | "approve" | "unblock" => Some((
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

/// Make an Invite and print its link, alone on stdout so a script can take it. The Key Address
/// comes first: without one, no Invite is made that nobody would have a link for.
async fn invite(host: &Entry, client: &Client, label: &str, options: InviteOptions) -> Result<()> {
    host.key_address()?;
    let invite = client.invite(label, options).await.map_err(refused(host))?;
    println!("{}", host.invite_link(&invite.token)?);
    Ok(())
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

/// A duration in words: "7 days", "10 minutes".
fn lasting(duration: Duration) -> String {
    let (n, unit) = match duration.as_secs() {
        s if s % 86_400 == 0 => (s / 86_400, "day"),
        s if s % 3600 == 0 => (s / 3600, "hour"),
        s if s % 60 == 0 => (s / 60, "minute"),
        s => (s, "second"),
    };
    format!("{n} {unit}{}", if n == 1 { "" } else { "s" })
}

/// `tooob run`: serve a Host here, holding the lock on its state, with its settings as its file
/// says now. One process per Host: a second is refused, with the first one's pid.
async fn serve(host: &Entry, invitees: Vec<Invitee>) -> Result<()> {
    if autostart::is_on(host) && !autostart::started_by_its_entry(host) {
        bail!(
            "{0} starts by itself, so its service manager runs it: `tooob logs {0} -f` follows \
             it, and `tooob disable {0}` turns that off",
            host.name
        );
    }
    if !invitees.is_empty() {
        host.key_address()?; // say so now, not once it's serving
    }
    let _lock = host.lock()?;
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
            "{} won't start ({} names its Relay): {}",
            host.name,
            host.settings_file.display(),
            problem.at(&host.settings.relay)
        );
    }
    serve_here(host, site, network, bootstrap, invitees).await
}

/// How often a running Host checks its Relay's Bootstrap again: every 10 minutes, or as
/// `TOOOB_BOOTSTRAP_CHECK_EVERY` says (`1s`, `30m`), which tests use.
fn check_every() -> Result<Duration> {
    let Some(every) = std::env::var_os("TOOOB_BOOTSTRAP_CHECK_EVERY") else {
        return Ok(Duration::from_secs(10 * 60));
    };
    let every = every.to_string_lossy();
    match seconds(&every).context("in TOOOB_BOOTSTRAP_CHECK_EVERY")? {
        0 => bail!("TOOOB_BOOTSTRAP_CHECK_EVERY can't be 0"),
        secs => Ok(Duration::from_secs(secs)),
    }
}

/// Run the Host in this process until it's told to stop: Ctrl-C, SIGTERM from Docker or a
/// service manager, SIGHUP from a closing terminal, or the Owner (`tooob stop`). Stopping cleanly
/// closes every Visitor's connection, so their browsers know at once. Its log goes to stderr.
async fn serve_here(
    host: &Entry,
    site: Site,
    network: Network,
    bootstrap: Option<Result<BootstrapVersion, Problem>>,
    invitees: Vec<Invitee>,
) -> Result<()> {
    let check_every = check_every()?;
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
    let beside = match &bootstrap {
        None => String::new(),
        Some(Ok(version)) => format!(", Bootstrap {version}"),
        Some(Err(_)) => ", Bootstrap unknown".to_string(),
    };
    eprintln!(
        "tooob: {name}: starting {}{beside}",
        release_and_protocol_version()
    );
    if let Some(Err(problem)) = &bootstrap {
        eprintln!(
            "tooob: {name}: warning: {}; it's checked again every {}",
            problem.at(&host.settings.relay),
            lasting(check_every)
        );
    }
    let running = Host::start(HostConfig {
        site,
        data_dir: host.state_dir.clone(),
        network,
    })
    .await?;
    eprintln!(
        "tooob: {name}: Host {}, through {through}",
        running.address().id.to_z32()
    );
    // Until this returns: the Host stops, and its checks with it.
    let _checking = bootstrap.map(|first| {
        let watch = Watch::new(&host.settings.relay, &first);
        bootstrap::keep_checking(name, watch, running.control(), check_every)
    });
    // Browsers reach a Host only through its Relay, so until it's there, its links lead nowhere.
    let online = async {
        let wait = Duration::from_secs(15);
        if !local && tokio::time::timeout(wait, running.online()).await.is_err() {
            eprintln!(
                "tooob: {name}: warning: not on its Relay yet, so Visitors' browsers can't reach it for now"
            );
        }
    };
    let stopped_early = tokio::select! {
        _ = online => false,
        _ = &mut signals => true,
        _ = running.stop_requested() => true,
    };
    if !stopped_early {
        eprintln!("tooob: {name}: serving {serving}");
        // While it serves, and no longer: a Host started by hand behaves as one started by
        // itself (ADR 0014).
        let awake = Awake::hold(&format!("tooob: {name} is serving"));
        if awake.is_some() {
            eprintln!(
                "tooob: {name}: this Mac won't idle-sleep while it serves (closing its lid still \
                 sleeps it)"
            );
        }
        // The socket only: this process holds the lock, and taking it again here would release it.
        let client = Client::new(&host.state_dir);
        for who in &invitees {
            invite_given(host, &client, who).await?;
        }
        tokio::select! {
            _ = &mut signals => {}
            _ = running.stop_requested() => {}
        }
    }
    // Bounded, whatever a Visitor or the Site is doing: never hang on the way out.
    if tokio::time::timeout(Duration::from_secs(5), running.shutdown())
        .await
        .is_err()
    {
        eprintln!("tooob: {name}: warning: didn't stop cleanly within 5 s");
    }
    eprintln!("tooob: {name}: stopped");
    Ok(())
}

/// Make the Invite `share` was given, saying who it's for and for how long.
async fn invite_given(host: &Entry, client: &Client, who: &Invitee) -> Result<()> {
    let options = InviteOptions {
        first_visit_approval: who.approve,
        expires_in: Some(who.expires_in),
        ..Default::default()
    };
    eprintln!(
        "  Invite for {} to {}, {}:",
        who.label,
        host.name,
        lasts(&options)
    );
    invite(host, client, &who.label, options).await
}

/// Stop a running Host, and wait until it has: through its service manager when it starts by
/// itself, so it isn't started again straight away (ADR 0014); otherwise by asking it.
async fn stop(host: &Entry) -> Result<()> {
    if autostart::is_on(host) {
        if host.running().is_none() {
            bail!(
                "{0} isn't running (it starts by itself: `tooob enable {0}` starts it now)",
                host.name
            );
        }
        return autostart::stop(host);
    }
    let client = Client::new(&host.state_dir);
    client.stop().await.map_err(refused(host))?;
    host.until_stopped()
}

/// `tooob logs`: the log a Host's service manager writes, all of it; and with `-f`, what it writes
/// next, until Ctrl-C.
async fn logs(host: &Entry, follow: bool) -> Result<()> {
    use std::io::{Seek, Write};
    use std::os::unix::fs::MetadataExt;
    let file = host.log_file();
    if !file.exists() && !autostart::is_on(host) {
        bail!(
            "{0} has no log: a Host writes one when it starts by itself (`tooob enable {0}`); run \
             by hand, it logs where it runs",
            host.name
        );
    }
    let mut log: Option<std::fs::File> = None;
    loop {
        // A new file, or one emptied: read it from the start.
        if let Ok(here) = std::fs::metadata(&file) {
            let stale = match log.as_mut() {
                Some(open) => {
                    open.metadata()?.ino() != here.ino() || here.len() < open.stream_position()?
                }
                None => true,
            };
            if stale {
                log = std::fs::File::open(&file).ok();
            }
        }
        if let Some(open) = log.as_mut() {
            let mut out = std::io::stdout().lock();
            std::io::copy(open, &mut out)?;
            out.flush()?;
        }
        if !follow {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
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
            match args[i].strip_prefix("--") {
                _ if args[i] == "-f" => {}
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

/// Words, `--flags` (value flags take the next word), and every `--invite LABEL`: it can repeat.
fn parse(args: impl Iterator<Item = String>) -> Result<(Vec<String>, Flags, Vec<String>)> {
    let (mut words, mut flags, mut invitees) = (Vec::new(), HashMap::new(), Vec::new());
    let mut args = args.peekable();
    while let Some(a) = args.next() {
        if a == "-f" {
            flags.insert("follow".to_string(), None);
            continue;
        }
        match a.strip_prefix("--") {
            Some("invite") => invitees.push(args.next().context("--invite needs a Label")?),
            Some(name) if VALUE_FLAGS.contains(&name) => {
                let value = args
                    .next()
                    .with_context(|| format!("--{name} needs a value"))?;
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

/// "90s", "30m", "1h", "7d" → seconds.
pub(crate) fn seconds(text: &str) -> Result<u64> {
    let (number, unit) = text.split_at(text.len().saturating_sub(1));
    let n: u64 = number
        .parse()
        .with_context(|| format!("bad duration {text:?} (try 1h or 7d)"))?;
    let unit = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 3600,
        "d" => 86400,
        _ => bail!("bad duration {text:?} (try 1h or 7d)"),
    };
    n.checked_mul(unit)
        .with_context(|| format!("bad duration {text:?} (too long)"))
}
