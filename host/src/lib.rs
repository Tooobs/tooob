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
mod shared_folder;
pub mod visit;
mod wire;

pub use wire::PROTOCOL_VERSION;

use anyhow::Result;
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
