//! `tooob-relay`: stock iroh-relay's command line (`--dev`, `--config-path`) and config file, plus
//! the Enrolled Keys (ADR 0030): `enroll`, `remove` and `list` edit their file, which a running
//! Relay watches.
use std::{path::PathBuf, process::ExitCode};

use clap::{Parser, Subcommand};
use iroh_base::EndpointId;
use tooob_relay::{List, Relay, config, parse_key};
use tracing_subscriber::{EnvFilter, prelude::*};

#[cfg(target_env = "musl")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// tooob's Relay: iroh-relay, forwarding only traffic to or from an Enrolled Key, and everyone's
/// while there are none.
#[derive(Parser, Debug)]
#[clap(version, about, long_about = None)]
struct Cli {
    /// Run in localhost development mode over plain HTTP, on port 3340 unless the config says.
    #[clap(long, default_value_t = false)]
    dev: bool,
    /// Path to the configuration file (stock iroh-relay's).
    #[clap(long, short)]
    config_path: Option<PathBuf>,
    /// The file of Enrolled Keys.
    #[clap(long, env = "TOOOB_RELAY_ENROLLED", default_value = tooob_relay::DEFAULT_PATH)]
    enrolled: PathBuf,
    /// Where to answer `GET /__tooob/relay.json` (open or enrolled), if anywhere.
    #[clap(long, env = "TOOOB_RELAY_STATUS_ADDR")]
    status_addr: Option<std::net::SocketAddr>,
    /// This Relay's domain, which an Enrollment Code names (the Relay image's RELAY_DOMAIN).
    #[clap(long, env = "RELAY_DOMAIN")]
    domain: Option<String>,
    #[clap(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Enroll a key (a Host Key), or relabel one: the running Relay forwards its traffic.
    Enroll {
        /// The key: as the Host logs it ("Host <key>"), or in hex.
        #[clap(value_parser = parse_key)]
        key: EndpointId,
        /// What it is, for you: a Host's name, or whose it is.
        #[clap(long)]
        label: String,
    },
    /// Remove an Enrolled Key: the running Relay disconnects it at once.
    Remove {
        #[clap(value_parser = parse_key)]
        key: EndpointId,
    },
    /// List the Enrolled Keys, with their labels.
    List,
    /// Make an Enrollment Code: a Host presenting it is enrolled under its label.
    Code {
        /// Whose it is: the Host Key it enrolls gets this label.
        #[clap(long)]
        label: String,
        /// How many Hosts it can enroll.
        #[clap(long, default_value_t = 1)]
        uses: u32,
        /// How long it stays good unused: such as 7d, 12h or 30m.
        #[clap(long, default_value = "7d", value_parser = parse_duration)]
        expires: u64,
    },
    /// List the Enrollment Codes that can still enroll a Host.
    Codes,
    /// Cancel an Enrollment Code, given as itself or by its label.
    Cancel { code_or_label: String },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let done = match cli.command {
        Some(Command::Enroll { key, label }) => edit(&cli.enrolled, |list| {
            list.enroll(key, &label);
            Ok(format!("enrolled {key} as {label}"))
        }),
        Some(Command::Remove { key }) => edit(&cli.enrolled, |list| match list.remove(key) {
            true => Ok(format!("removed {key}")),
            false => Err(format!("{key} isn't enrolled")),
        }),
        Some(Command::List) => List::load(&cli.enrolled).map(|list| {
            if list.is_empty() {
                println!("no Enrolled Keys: this Relay is open, and forwards everyone's traffic");
            }
            for (key, label) in list.entries() {
                println!("{key}  {label}");
            }
        }),
        Some(Command::Code {
            label,
            uses,
            expires,
        }) => match cli.domain {
            None => Err("no domain to name in the code: give --domain or set RELAY_DOMAIN".into()),
            Some(domain) => edit(&cli.enrolled, |list| {
                let until = tooob_relay::now() + expires;
                let secret = list.make_code(&label, uses, until);
                Ok(format!(
                    "{domain}/{secret}  for {label}: {}, {}",
                    uses_text(uses),
                    expiry_text(until)
                ))
            }),
        },
        Some(Command::Codes) => List::load(&cli.enrolled).map(|list| {
            let now = tooob_relay::now();
            let mut none = true;
            for code in list.live_codes(now) {
                none = false;
                println!(
                    "{}  {}, {}",
                    code.label,
                    uses_text(code.uses_left),
                    expiry_text(code.expires)
                );
            }
            if none {
                println!("no unused Enrollment Codes");
            }
        }),
        Some(Command::Cancel { code_or_label }) => edit(&cli.enrolled, |list| {
            match list.cancel(&code_or_label, tooob_relay::now()) {
                0 => Err(format!("no unused Enrollment Code is {code_or_label}")),
                1 => Ok("cancelled 1 Enrollment Code".to_string()),
                n => Ok(format!("cancelled {n} Enrollment Codes")),
            }
        }),
        None => serve(cli.dev, cli.config_path, cli.enrolled, cli.status_addr),
    };
    match done {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("tooob-relay: {e}");
            ExitCode::FAILURE
        }
    }
}

/// A length of time such as 7d, 12h, 30m or 90s, in seconds.
fn parse_duration(s: &str) -> Result<u64, String> {
    let bad = || format!("{s} isn't a length of time such as 7d, 12h or 30m");
    let (n, unit) = s.split_at(s.len().saturating_sub(1));
    let n: u64 = n.parse().map_err(|_| bad())?;
    let per = match unit {
        "d" => 86_400,
        "h" => 3_600,
        "m" => 60,
        "s" => 1,
        _ => return Err(bad()),
    };
    n.checked_mul(per).ok_or_else(bad)
}

fn uses_text(uses: u32) -> String {
    match uses {
        1 => "1 use left".to_string(),
        n => format!("{n} uses left"),
    }
}

fn expiry_text(expires: u64) -> String {
    let left = expires.saturating_sub(tooob_relay::now());
    match left {
        0 => "expired".to_string(),
        s if s >= 86_400 => format!("expires in {} days", s.div_ceil(86_400)),
        s if s >= 3_600 => format!("expires in {} hours", s.div_ceil(3_600)),
        s => format!("expires in {} minutes", s.div_ceil(60)),
    }
}

/// Change the list in the file at `path` and save it, saying what changed.
fn edit(
    path: &std::path::Path,
    change: impl FnOnce(&mut List) -> Result<String, String>,
) -> Result<(), String> {
    let said = List::edit(path, |list| {
        change(list).map(|said| format!("{said} ({})", tooob_relay::access(list)))
    })?;
    println!("{said}");
    Ok(())
}

/// Run the Relay until it stops or is interrupted.
fn serve(
    dev: bool,
    config_path: Option<PathBuf>,
    enrolled: PathBuf,
    status_addr: Option<std::net::SocketAddr>,
) -> Result<(), String> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .with(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("warn,tooob_relay=info")),
        )
        .init();
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("failed to set default crypto provider");
    let runtime = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
    runtime.block_on(async {
        let config = config::server_config(config_path.as_deref(), dev)
            .await
            .map_err(|e| format!("{e:#}"))?;
        let mut relay = Relay::spawn(config, enrolled).await?;
        if let Some(addr) = status_addr {
            let at = relay.serve_status(addr).await?;
            tracing::info!("tooob-relay: /__tooob/relay.json at http://{at}");
        }
        tokio::select! {
            biased;
            _ = tokio::signal::ctrl_c() => (),
            _ = relay.join() => (),
        }
        relay.shutdown().await;
        Ok(())
    })
}
