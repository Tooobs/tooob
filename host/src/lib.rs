//! tooob v1: a Host makes one Site reachable to the Visitors its Owner invites.
pub mod access;
pub mod autostart;
pub mod awake;
pub mod backup;
pub mod bootstrap;
pub mod control;
mod direct;
pub mod host;
pub mod hosts;
mod jars;
pub mod keys;
pub mod log;
mod shared_folder;
pub mod upgrade;
mod usage;
pub mod visit;
mod visit_log;
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
    count(n, unit)
}

/// "1 day", "6 days".
fn count(n: u64, unit: &str) -> String {
    format!("{n} {unit}{}", if n == 1 { "" } else { "s" })
}

/// When something expiring at `expires_at` (Unix seconds) expires, as seen at `now`, in words:
/// "in 40 minutes", "in 5 hours", "in 6 days", or "expired". Always rounded up, so it never says
/// less than is left, in minutes up to 59, then hours up to 23, then days: 59 m 1 s left is
/// "in 1 hour", never "in 60 minutes".
pub fn expires(expires_at: u64, now: u64) -> String {
    let (n, unit) = match expires_at.saturating_sub(now) {
        0 => return "expired".into(),
        s if s.div_ceil(60) < 60 => (s.div_ceil(60), "minute"),
        s if s.div_ceil(3600) < 24 => (s.div_ceil(3600), "hour"),
        s => (s.div_ceil(86_400), "day"),
    };
    format!("in {}", count(n, unit))
}

/// `0 B`, `512 B`, `4.1 KB`, `2.3 MB`, `456 MB`: in thousands, as file sizes are shown on a Mac.
pub fn amount(bytes: u64) -> String {
    if bytes < 1000 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut units = ["KB", "MB", "GB", "TB", "PB"].iter();
    let mut unit = "B";
    while value >= 999.5 {
        match units.next() {
            Some(next) => {
                value /= 1000.0;
                unit = next;
            }
            None => break,
        }
    }
    match value {
        v if v < 99.95 => format!("{v:.1} {unit}"),
        v => format!("{v:.0} {unit}"),
    }
}

#[cfg(test)]
mod tests {
    use super::{amount, expires};

    #[test]
    fn an_expiry_is_in_minutes_within_the_hour_then_hours_within_the_day_then_days() {
        let now = 1_800_000_000;
        for (left, said) in [
            (0, "expired"),
            (1, "in 1 minute"),
            (60, "in 1 minute"),
            (61, "in 2 minutes"),
            (40 * 60, "in 40 minutes"),
            (59 * 60, "in 59 minutes"),
            (59 * 60 + 1, "in 1 hour"),
            (3600, "in 1 hour"),
            (3601, "in 2 hours"),
            (3600 + 29 * 60, "in 2 hours"),
            (5 * 3600, "in 5 hours"),
            (23 * 3600, "in 23 hours"),
            (23 * 3600 + 1, "in 1 day"),
            (86_400, "in 1 day"),
            (86_401, "in 2 days"),
            (6 * 86_400, "in 6 days"),
            (7 * 86_400 - 1, "in 7 days"),
        ] {
            assert_eq!(expires(now + left, now), said, "{left} s left");
        }
        assert_eq!(expires(now - 10, now), "expired");
    }

    #[test]
    fn amounts_are_in_thousands_with_a_decimal_below_a_hundred() {
        for (bytes, said) in [
            (0, "0 B"),
            (999, "999 B"),
            (1000, "1.0 KB"),
            (4_130, "4.1 KB"),
            (99_949, "99.9 KB"),
            (450_312, "450 KB"),
            (999_400, "999 KB"),
            (999_600, "1.0 MB"),
            (2_300_000, "2.3 MB"),
            (456_000_000, "456 MB"),
            (1_200_000_000, "1.2 GB"),
        ] {
            assert_eq!(amount(bytes), said, "{bytes}");
        }
    }
}
