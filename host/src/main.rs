//! `tooob`: make Hosts (`tooob add`), run each in the foreground (`tooob run`), and manage them
//! from the same box. Each Host is its own process (ADR 0013); keeping one running after the
//! terminal closes is the operating system's job, never tooob's (ADR 0014). Owner commands talk to
//! a running Host over its control socket (ADR 0010).
use anyhow::{Context, Result, bail};
use std::{
    collections::HashMap,
    io::IsTerminal,
    path::Path,
    time::{Duration, Instant},
};
use tokio::signal::unix::{SignalKind, signal};
use tooob::Network;
use tooob::access::InviteOptions;
use tooob::control::{Client, ControlError};
use tooob::host::{Host, HostConfig, Site};
use tooob::hosts::{Entry, Home, NO_RELAY, RUN_YOUR_OWN, check_bootstrap, check_relay, name_char};

mod ask;

const USAGE: &str = "usage:
  tooob add NAME TARGET (--relay URL | --local) --bootstrap URL
                                                  make a Host; TARGET is a folder or file to share
                                                  read-only, or HOST:PORT for a web app
  tooob run NAME                                  serve it here, until Ctrl-C
  tooob share [PATH] [--name NAME] [--invite LABEL] [--relay URL | --local] [--bootstrap URL]
                                                  add a Host for PATH (or reuse its Host), then
                                                  run it; in a terminal, it asks what's missing
  tooob ls [--json]                               every Host on this box, running or stopped
  tooob show NAME                                 its settings, Host Key and files
  tooob rm NAME                                   delete it, Host Key and all
  tooob stop NAME                                 end a running Host
  tooob invite NAME LABEL [--approve] [--owner] [--expires 1h]  print an Invite Link
  tooob pair NAME LABEL                           Pairing Invite (this box only)
  tooob visitors NAME [--json] | invites NAME [--json]
  tooob block NAME KEY | approve NAME KEY | unblock NAME KEY   KEY: a unique prefix is enough
  tooob revoke NAME INVITE-ID
  tooob --version                                 the Release and Protocol Version
NAME is a Host Name, what the Host serves, or a Host Key prefix. Owner commands work whether the
Host is running or not. There's no default Relay or Bootstrap: deploy/ shows how to run your own.";

const VALUE_FLAGS: &[&str] = &["relay", "bootstrap", "expires", "name"];
const BARE_FLAGS: &[&str] = &["local", "json", "approve", "owner"];

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

/// A person is at the keyboard: questions and pickers are allowed. Scripts and Agents never get
/// one, and must say everything with flags.
fn terminal() -> bool {
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
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
    let (words, flags, invitees) = parse(std::env::args().skip(1))?;
    let home = Home::from_env()?;
    let arg = |i: usize| {
        words
            .get(i)
            .cloned()
            .with_context(|| format!("missing argument\n{USAGE}"))
    };
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
            exactly(3, &["relay", "local", "bootstrap"])?;
            let (name, target) = (arg(1)?, arg(2)?);
            let (relay, bootstrap) = new_host_settings(&flags, false)?;
            home.add(&name, &target, &relay, &bootstrap)?;
        }
        Some("run") => {
            exactly(2, &[])?;
            serve(&home.find(&arg(1)?)?, Vec::new()).await?;
        }
        Some("share") => share(&home, &words, &flags, &invitees).await?,
        Some("ls") => {
            exactly(1, &["json"])?;
            ls(&home, flags.contains_key("json")).await?;
        }
        Some("show") => {
            exactly(2, &[])?;
            show_host(&home.find(&arg(1)?)?);
        }
        Some("rm") => {
            exactly(2, &[])?;
            home.find(&arg(1)?)?.remove()?;
        }
        Some(command) if OWNER.contains(&command) => {
            owner_command(&home, command, &words, &flags, &invitees).await?
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

/// A new Host's Relay and Bootstrap, as `--relay URL` (or `--local`) and `--bootstrap URL` say:
/// never a guess (ADR 0015). With `ask`, a person at the keyboard is asked for what's missing;
/// otherwise it's an error that names all of it.
fn new_host_settings(flags: &Flags, ask: bool) -> Result<(String, String)> {
    let relay = relay(flags)?;
    let bootstrap = flags.get("bootstrap").cloned().flatten();
    if let Some(relay) = &relay {
        check_relay(relay)?;
    }
    if let Some(bootstrap) = &bootstrap {
        check_bootstrap(bootstrap)?;
    }
    if ask {
        let relay = relay.map_or_else(ask::relay, Ok)?;
        let bootstrap = bootstrap.map_or_else(ask::bootstrap, Ok)?;
        return Ok((relay, bootstrap));
    }
    let mut missing = Vec::new();
    if relay.is_none() {
        missing.push(
            "which Relay its Visitors reach it through (--relay URL, or --local for this network only)",
        );
    }
    if bootstrap.is_none() {
        missing.push("which Bootstrap its Invite Links open (--bootstrap URL)");
    }
    match (relay, bootstrap) {
        (Some(relay), Some(bootstrap)) => Ok((relay, bootstrap)),
        _ => bail!("say {}: {RUN_YOUR_OWN}\n{USAGE}", missing.join(", and ")),
    }
}

/// What `share` serves from: the Host already serving the path, or a new one, added only once the
/// Owner has answered everything, so backing out adds nothing.
enum Sharing {
    Reused(Entry),
    New { relay: String, bootstrap: String },
}

/// `tooob share`: add a Host for a folder or file, or reuse the one already serving it, then run
/// it here. A person at the keyboard is asked what and who first, so backing out adds nothing.
async fn share(home: &Home, words: &[String], flags: &Flags, invitees: &[String]) -> Result<()> {
    let path = match words.get(1) {
        Some(path) => std::fs::canonicalize(path).with_context(|| format!("can't share {path}"))?,
        None if terminal() => ask::what_to_share()?,
        None => bail!("say what to share: tooob share PATH\n{USAGE}"),
    };
    let serves = path.to_string_lossy();
    // A reused Host keeps its own Relay and Bootstrap; the flags are for a new one.
    let sharing = match home
        .hosts()
        .into_iter()
        .find(|h| h.settings.serves == serves)
    {
        Some(host) => {
            if terminal() || !invitees.is_empty() {
                host.bootstrap()?; // the questions are about Invites, which need it
            }
            Sharing::Reused(host)
        }
        None => {
            let (relay, bootstrap) = new_host_settings(flags, terminal())?;
            Sharing::New { relay, bootstrap }
        }
    };
    if terminal() {
        ask::confirm_contents(&path)?;
    }
    let mut who: Vec<ask::Invitee> = invitees
        .iter()
        .map(|label| ask::Invitee {
            label: label.clone(),
            expires: flags
                .get("expires")
                .cloned()
                .flatten()
                .unwrap_or_else(|| "7d".into()),
            approve: flags.contains_key("approve"),
        })
        .collect();
    if terminal() {
        while let Some(more) = ask::invitee(who.is_empty())? {
            who.push(more);
        }
    }
    let host = match sharing {
        Sharing::Reused(host) => host,
        Sharing::New { relay, bootstrap } => {
            let name = match flags.get("name").cloned().flatten() {
                Some(name) => name,
                None => free_name(home, &path),
            };
            home.add(&name, &serves, &relay, &bootstrap)?
        }
    };
    serve(&host, who).await
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
        enabled: bool,
        visitors: Option<usize>,
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
        let visitors = match running {
            true => Client::new(&host.state_dir).visitors().await.ok(),
            false => None,
        };
        rows.push(Row {
            name,
            state: if running { "running" } else { "stopped" },
            enabled: false, // until #40
            visitors: visitors.map(|v| v.len()),
            serves: host.settings.serves,
        });
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }
    let width = rows.iter().map(|r| r.name.len()).max().unwrap_or(0).max(4);
    println!(
        "{:<width$}  {:<7}  {:<7}  {:>8}  SERVES",
        "NAME", "STATE", "ENABLED", "VISITORS"
    );
    for r in rows {
        let visitors = r.visitors.map_or("-".into(), |n| n.to_string());
        let enabled = if r.enabled { "yes" } else { "no" };
        println!(
            "{:<width$}  {:<7}  {enabled:<7}  {visitors:>8}  {}",
            r.name, r.state, r.serves
        );
    }
    Ok(())
}

/// `tooob show`: one Host's settings, Host Key, and where its files are.
fn show_host(host: &Entry) {
    let state = match host.running() {
        Some(pid) => format!("running (pid {pid})"),
        None => "stopped".into(),
    };
    println!("{}: {state}", host.name);
    let key = host.host_key().unwrap_or_else(|| "none yet".into());
    for (label, value) in [
        ("serves", host.settings.serves.clone()),
        ("relay", host.settings.relay.clone()),
        (
            "bootstrap",
            host.settings
                .bootstrap
                .clone()
                .unwrap_or_else(|| "not set: Invite Links need one in the settings file".into()),
        ),
        ("Host Key", key),
        ("settings file", host.settings_file.display().to_string()),
        ("state folder", host.state_dir.display().to_string()),
        ("log", host.log_file().display().to_string()),
    ] {
        println!("  {label:<14} {value}");
    }
}

/// `tooob VERB NAME …`: one of the Owner's commands on one Host, running or not. A change prints
/// nothing when it works; `invite` and `pair` print just the link.
async fn owner_command(
    home: &Home,
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
    let arity = match command {
        "visitors" | "invites" | "stop" => 2,
        _ => 3,
    };
    if words.len() > arity {
        bail!("too many arguments: {}\n{USAGE}", words[arity..].join(" "));
    }
    if let Some(other) = flags.keys().find(|f| !allowed.contains(&f.as_str())) {
        bail!("{command} takes no --{other}\n{USAGE}");
    }
    if !invitees.is_empty() {
        bail!("{command} takes no --invite\n{USAGE}");
    }
    let host = named(home, command, words.get(1)).await?;
    if command == "stop" {
        return stop(&host).await;
    }
    let client = host.client()?;
    if matches!(command, "invite" | "pair") {
        host.bootstrap()?; // before anything's asked: a link needs it
    }
    let said = |what: &str| {
        words.get(2).cloned().with_context(|| {
            format!(
                "say {what}: tooob {command} {} {}\n{USAGE}",
                host.name,
                what.to_uppercase()
            )
        })
    };
    let refused = refused(&host);
    let json = flags.contains_key("json");
    match command {
        "invite" if words.get(2).is_none() && terminal() => {
            if !ask_and_invite(&host, &client).await? {
                bail!("No Invite made.");
            }
        }
        "invite" => {
            let options = invite_options(flags)?;
            invite(&host, &client, &said("label")?, options).await?
        }
        "pair" => {
            let label = match words.get(2) {
                None if terminal() => ask::label()?,
                _ => said("label")?,
            };
            let bootstrap = host.bootstrap()?;
            let invite = client.pair(&label).await.map_err(refused)?;
            let address = client.address().await.map_err(refused)?;
            println!("{}", address.invite_link(&bootstrap, &invite.token));
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
        "block" => client.block(&said("key")?).await.map_err(refused)?,
        "approve" => client.approve(&said("key")?).await.map_err(refused)?,
        "unblock" => client.unblock(&said("key")?).await.map_err(refused)?,
        "revoke" => client.revoke(&said("invite-id")?).await.map_err(refused)?,
        _ => unreachable!(),
    }
    Ok(())
}

/// The Host an Owner command names, by its name, what it serves or a Host Key prefix. Without one,
/// a person at the keyboard picks it (from the running Hosts, to stop one); a script is told which
/// there are. Never a guess.
async fn named(home: &Home, command: &str, wanted: Option<&String>) -> Result<Entry> {
    if let Some(wanted) = wanted {
        return home.find(wanted);
    }
    let mut hosts = home.hosts();
    if command == "stop" {
        hosts.retain(|h| h.running().is_some());
    }
    if terminal() && !hosts.is_empty() {
        let (prompt, cancelled) = match command {
            "invite" | "pair" => ("Which Host is this Invite for?", "No Invite made."),
            "stop" => ("Which Host should stop?", "Nothing stopped."),
            _ => ("Which Host?", "Nothing changed."),
        };
        let items: Vec<String> = hosts.iter().map(row).collect();
        let Some(i) = ask::pick(prompt, &items)? else {
            bail!("{cancelled}");
        };
        return Ok(hosts.remove(i));
    }
    let names: Vec<String> = hosts.into_iter().map(|h| h.name).collect();
    let which = match (command, names.as_slice()) {
        ("stop", []) => " (none is running)".to_string(),
        (_, []) => " (none yet: `tooob add NAME TARGET` makes one)".to_string(),
        (_, names) => format!(" (Hosts here: {})", names.join(", ")),
    };
    bail!("say which Host: tooob {command} NAME …{which}\n{USAGE}")
}

/// One Host on one line: its name, what it serves, and its short Host Key.
fn row(host: &Entry) -> String {
    let id = host.host_key().unwrap_or_default();
    format!(
        "{:<20} {:<40} Host {}…",
        host.name,
        host.settings.serves,
        short(&id)
    )
}

/// The first few characters of a Host Key: enough to tell a box's Hosts apart.
fn short(id: &str) -> &str {
    &id[..id.len().min(8)]
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

/// Make an Invite and print its link, alone on stdout so a script can take it. The Bootstrap comes
/// first: without one, no Invite is made that nobody would have a link for.
async fn invite(host: &Entry, client: &Client, label: &str, options: InviteOptions) -> Result<()> {
    let bootstrap = host.bootstrap()?;
    let refused = refused(host);
    let invite = client.invite(label, options).await.map_err(refused)?;
    let address = client.address().await.map_err(refused)?;
    println!("{}", address.invite_link(&bootstrap, &invite.token));
    Ok(())
}

/// How long an Invite works, in words: "works for 7 days".
fn lasts(options: &InviteOptions) -> String {
    let secs = options.expires_in.unwrap_or(7 * 86_400);
    let (n, unit) = match secs {
        s if s % 86_400 == 0 => (s / 86_400, "day"),
        s if s % 3600 == 0 => (s / 3600, "hour"),
        s if s % 60 == 0 => (s / 60, "minute"),
        s => (s, "second"),
    };
    format!("works for {n} {unit}{}", if n == 1 { "" } else { "s" })
}

/// `tooob run`: serve a Host here, holding the lock on its state, with its settings as its file
/// says now. One process per Host: a second is refused, with the first one's pid.
async fn serve(host: &Entry, invitees: Vec<ask::Invitee>) -> Result<()> {
    if !invitees.is_empty() {
        host.bootstrap()?; // say so now, not once it's serving
    }
    let _lock = host.lock()?;
    let site = host.settings.site()?;
    let network = host
        .settings
        .network()
        .with_context(|| format!("in {}", host.settings_file.display()))?;
    serve_here(host, site, network, invitees).await
}

/// Run the Host in this process until it's told to stop: Ctrl-C, SIGTERM from Docker or a
/// service manager, SIGHUP from a closing terminal, or the Owner (`tooob stop`). Stopping cleanly
/// closes every Visitor's connection, so their browsers know at once. Its log goes to stderr.
async fn serve_here(
    host: &Entry,
    site: Site,
    network: Network,
    invitees: Vec<ask::Invitee>,
) -> Result<()> {
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
    eprintln!("tooob: {name}: starting {}", release_and_protocol_version());
    let running = Host::start(HostConfig {
        site,
        data_dir: host.state_dir.clone(),
        network,
    })
    .await?;
    eprintln!(
        "tooob: {name}: Host {}, through {through}",
        running.address().id
    );
    // Links made before the Host has a Relay would only work on this network.
    let online = async {
        let wait = Duration::from_secs(15);
        if !local && tokio::time::timeout(wait, running.online()).await.is_err() {
            eprintln!(
                "tooob: {name}: warning: no Relay yet; Invite Links will only work on this network for now"
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
        // The socket only: this process holds the lock, and taking it again here would release it.
        let client = Client::new(&host.state_dir);
        for who in &invitees {
            invite_as_answered(host, &client, who).await?;
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

/// Make the Invite someone answered for (or gave as flags), saying who it's for and for how long.
async fn invite_as_answered(host: &Entry, client: &Client, who: &ask::Invitee) -> Result<()> {
    let options = InviteOptions {
        first_visit_approval: who.approve,
        expires_in: Some(seconds(&who.expires)?),
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

/// Ask who to invite and how, make the Invite, and show how to type it next time (#34). False
/// if the Owner skipped it.
async fn ask_and_invite(host: &Entry, client: &Client) -> Result<bool> {
    let Some(who) = ask::invitee(true)? else {
        return Ok(false);
    };
    invite_as_answered(host, client, &who).await?;
    let approve = if who.approve { " --approve" } else { "" };
    println!("Next time, in one line:");
    println!(
        "  tooob invite {} {} --expires {}{approve}",
        quote(&host.name),
        quote(&who.label),
        who.expires
    );
    Ok(true)
}

/// A word as a shell would need it: unchanged when it's plain, single-quoted otherwise.
fn quote(word: &str) -> String {
    let plain = !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./:@%+=,".contains(c));
    if plain {
        word.to_string()
    } else {
        format!("'{}'", word.replace('\'', r"'\''"))
    }
}

/// Ask a running Host to stop, and wait until it has.
async fn stop(host: &Entry) -> Result<()> {
    let client = Client::new(&host.state_dir);
    client.stop().await.map_err(refused(host))?;
    let started = Instant::now();
    while host.running().is_some() {
        if started.elapsed() > Duration::from_secs(10) {
            bail!("{} didn't stop within 10 s", host.name);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Ok(())
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

/// Words, `--flags` (value flags take the next word), and every `--invite LABEL`: it can repeat.
fn parse(args: impl Iterator<Item = String>) -> Result<(Vec<String>, Flags, Vec<String>)> {
    let (mut words, mut flags, mut invitees) = (Vec::new(), HashMap::new(), Vec::new());
    let mut args = args.peekable();
    while let Some(a) = args.next() {
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
