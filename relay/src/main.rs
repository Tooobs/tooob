//! `tooob-relay`: stock iroh-relay's command line (`--dev`, `--config-path`) and config file, plus
//! the Enrolled Keys (ADR 0030): `enroll`, `remove` and `list` edit their file, which a running
//! Relay watches.
use std::{path::PathBuf, process::ExitCode};

use clap::{Parser, Subcommand};
use iroh_base::EndpointId;
use tooob_relay::{List, Relay, config};
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

/// A key as a Host logs it (z-base-32, as in its Key Address), or as iroh writes it (hex).
fn parse_key(s: &str) -> Result<EndpointId, String> {
    EndpointId::from_z32(s)
        .or_else(|_| s.parse())
        .map_err(|_| format!("{s} isn't a key"))
}

/// Change the list in the file at `path` and save it, saying what changed.
fn edit(
    path: &std::path::Path,
    change: impl FnOnce(&mut List) -> Result<String, String>,
) -> Result<(), String> {
    let mut list = List::load(path)?;
    let said = change(&mut list)?;
    list.save(path)?;
    println!("{said} ({})", tooob_relay::access(&list));
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
