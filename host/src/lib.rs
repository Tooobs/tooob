//! tooob v1: a Host makes one Site reachable to the Visitors its Owner invites.
pub mod access;
pub mod autostart;
pub mod awake;
pub mod bootstrap;
pub mod control;
mod direct;
pub mod host;
pub mod hosts;
mod jars;
pub mod log;
mod shared_folder;
pub mod upgrade;
pub mod visit;
mod visits;
mod wire;

pub use wire::PROTOCOL_VERSION;

use anyhow::{Context, Result, bail};
use iroh::{Endpoint, RelayMode, RelayUrl, SecretKey, endpoint::presets};
use std::{
    fs, io,
    io::Write,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

/// Which Relays an endpoint uses.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Network {
    /// n0's public Relays and address lookup.
    Default,
    /// One specific Relay, run by the Owner or by someone they trust.
    Relay(RelayUrl),
    /// No Relay and no lookup: this machine and its network only (`--local`).
    LocalOnly,
}

/// An iroh endpoint for a Host or a Visitor on the given Network.
pub(crate) async fn endpoint(network: &Network, key: SecretKey) -> Result<Endpoint> {
    let builder = match network {
        Network::Default => Endpoint::builder(presets::N0),
        Network::Relay(url) => {
            Endpoint::builder(presets::N0).relay_mode(RelayMode::custom([url.clone()]))
        }
        Network::LocalOnly => Endpoint::builder(presets::Minimal).relay_mode(RelayMode::Disabled),
    };
    Ok(builder.secret_key(key).bind().await?)
}

/// Write a file only the Owner's account can read: temp file, then rename, so a crash never
/// leaves a half-written file.
pub(crate) fn save_privately(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = temp_path(path);
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    fs::rename(&tmp, path)
}

/// Where `save_privately` writes before the rename; a crash can leave it behind.
pub(crate) fn temp_path(path: &Path) -> PathBuf {
    path.with_extension("tmp")
}

/// "90s", "30m", "1h", "7d" → seconds.
pub fn seconds(text: &str) -> Result<u64> {
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

/// A duration in words: "7 days", "10 minutes".
pub fn lasting(duration: std::time::Duration) -> String {
    let (n, unit) = match duration.as_secs() {
        s if s % 86_400 == 0 => (s / 86_400, "day"),
        s if s % 3600 == 0 => (s / 3600, "hour"),
        s if s % 60 == 0 => (s / 60, "minute"),
        s => (s, "second"),
    };
    format!("{n} {unit}{}", if n == 1 { "" } else { "s" })
}
