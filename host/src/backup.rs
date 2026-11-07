//! A Backup (ADR 0029): one Host's Host Key, access state, record and settings, sealed under a
//! passphrase: argon2id stretches it, XChaCha20-Poly1305 seals the contents, behind a small
//! versioned header that is the cipher's associated data.
use anyhow::{Context, Result, bail};
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit, Payload},
};

/// What every Backup starts with.
const MAGIC: &[u8] = b"tooob-backup";
/// The format this tooob writes.
const VERSION: u8 = 1;
/// The magic, the version, three argon2id costs, the salt and the nonce.
const HEADER_LEN: usize = MAGIC.len() + 1 + 3 * 4 + SALT_LEN + NONCE_LEN;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 24;
/// The most a Backup's header may ask of argon2id: 1 GiB, 16 passes, 16 lanes, so a doctored file
/// can't exhaust or hang a small box before the cipher finds it changed.
const MAX: Cost = Cost {
    memory_kib: 1024 * 1024,
    passes: 16,
    lanes: 16,
};
/// The most of stdin read for a passphrase: its first line needs far less.
const MAX_PASSPHRASE: u64 = 4096;

/// How hard argon2id works on the passphrase.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cost {
    pub memory_kib: u32,
    pub passes: u32,
    pub lanes: u32,
}

impl Cost {
    /// 64 MiB, 3 passes, 1 lane: about a second on a Raspberry Pi. In a debug build only,
    /// `TOOOB_BACKUP_KIB` sets the memory instead, which tests use to stay fast: a Release ignores
    /// it, so an exported value never weakens a real Backup.
    pub fn from_env() -> Result<Cost> {
        let memory_kib = match std::env::var("TOOOB_BACKUP_KIB") {
            Ok(kib) if cfg!(debug_assertions) => kib
                .parse()
                .context("TOOOB_BACKUP_KIB takes a number of KiB")?,
            _ => 64 * 1024,
        };
        Ok(Cost {
            memory_kib,
            passes: 3,
            lanes: 1,
        })
    }
}

/// Why a Backup didn't open: the same for a wrong passphrase and a changed file, since the cipher
/// can't tell them apart.
#[derive(Debug)]
pub struct WontOpen;

impl std::fmt::Display for WontOpen {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("wrong passphrase, or not a tooob Backup")
    }
}

impl std::error::Error for WontOpen {}

fn stretch(passphrase: &[u8], salt: &[u8], cost: Cost) -> Result<[u8; 32]> {
    let params = argon2::Params::new(cost.memory_kib, cost.passes, cost.lanes, Some(32))
        .map_err(|e| anyhow::anyhow!("argon2id won't take {cost:?}: {e}"))?;
    let argon = argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
    let mut key = [0u8; 32];
    argon
        .hash_password_into(passphrase, salt, &mut key)
        .map_err(|e| anyhow::anyhow!("argon2id: {e}"))?;
    Ok(key)
}

/// `contents` sealed under `passphrase`, header first.
pub fn seal(passphrase: &[u8], contents: &[u8], cost: Cost) -> Result<Vec<u8>> {
    let mut salt = [0u8; SALT_LEN];
    let mut nonce = [0u8; NONCE_LEN];
    getrandom::fill(&mut salt).map_err(|e| anyhow::anyhow!("no randomness: {e}"))?;
    getrandom::fill(&mut nonce).map_err(|e| anyhow::anyhow!("no randomness: {e}"))?;
    let mut sealed = Vec::with_capacity(HEADER_LEN + contents.len() + 16);
    sealed.extend_from_slice(MAGIC);
    sealed.push(VERSION);
    for n in [cost.memory_kib, cost.passes, cost.lanes] {
        sealed.extend_from_slice(&n.to_le_bytes());
    }
    sealed.extend_from_slice(&salt);
    sealed.extend_from_slice(&nonce);
    let key = stretch(passphrase, &salt, cost)?;
    let cipher = XChaCha20Poly1305::new(&key.into());
    let payload = Payload {
        msg: contents,
        aad: &sealed,
    };
    let body = cipher
        .encrypt(&XNonce::from(nonce), payload)
        .map_err(|_| anyhow::anyhow!("couldn't seal the Backup"))?;
    sealed.extend_from_slice(&body);
    Ok(sealed)
}

/// The contents of `sealed`, if `passphrase` opens it. A Backup of a format this tooob doesn't
/// know says so; anything else that won't open is `WontOpen`.
pub fn open(passphrase: &[u8], sealed: &[u8]) -> Result<Vec<u8>> {
    if sealed.len() < HEADER_LEN || !sealed.starts_with(MAGIC) {
        return Err(WontOpen.into());
    }
    let (header, body) = sealed.split_at(HEADER_LEN);
    let version = header[MAGIC.len()];
    if version != VERSION {
        bail!("it's a Backup of format {version}, which this tooob can't read: upgrade tooob");
    }
    let at = MAGIC.len() + 1;
    let n = |i: usize| u32::from_le_bytes(header[at + 4 * i..at + 4 * i + 4].try_into().unwrap());
    let cost = Cost {
        memory_kib: n(0),
        passes: n(1),
        lanes: n(2),
    };
    if cost.memory_kib > MAX.memory_kib || cost.passes > MAX.passes || cost.lanes > MAX.lanes {
        return Err(WontOpen.into());
    }
    let salt = &header[at + 12..at + 12 + SALT_LEN];
    let nonce: [u8; NONCE_LEN] = header[at + 12 + SALT_LEN..].try_into().unwrap();
    let key = stretch(passphrase, salt, cost).map_err(|_| WontOpen)?;
    let cipher = XChaCha20Poly1305::new(&key.into());
    let payload = Payload {
        msg: body,
        aad: header,
    };
    cipher
        .decrypt(&XNonce::from(nonce), payload)
        .map_err(|_| WontOpen.into())
}

/// What a Backup holds: everything a Host needs to run as it did, without what's rebuilt (its
/// cookie jars and usage).
#[derive(serde::Serialize, serde::Deserialize)]
pub struct Contents {
    /// Its Host Key's secret half, in hex.
    pub host_key: String,
    /// `access.json`: its Allowlist, Invites and Blocks; None before it has any.
    pub access: Option<String>,
    /// `host.json`: the Site and network it last ran with; None before it first ran.
    pub record: Option<String>,
    /// Its settings file, as the Owner left it.
    pub settings: String,
}

/// Everything but the Host Key's secret half, which never goes in a log or an error.
impl std::fmt::Debug for Contents {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.debug_struct("Contents")
            .field("host_key", &"(secret)")
            .field("access", &self.access)
            .field("record", &self.record)
            .field("settings", &self.settings)
            .finish()
    }
}

impl Contents {
    /// What a Backup holds of the Host Key `key` (`host.key`'s 32 bytes) and the rest.
    pub fn new(
        key: &[u8],
        access: Option<String>,
        record: Option<String>,
        settings: String,
    ) -> Self {
        Contents {
            host_key: crate::hex(key),
            access,
            record,
            settings,
        }
    }

    /// The Host Key's secret half, as `host.key` holds it.
    pub fn key_bytes(&self) -> Result<[u8; 32]> {
        crate::from_hex(&self.host_key).context("the Backup's Host Key is damaged")
    }
}

/// The passphrase in the file at `path`, or on stdin for `-`: its first line.
pub fn passphrase(path: &str) -> Result<Vec<u8>> {
    use std::io::Read;
    let read = match path {
        "-" => {
            let mut text = Vec::new();
            let stdin = std::io::stdin().take(MAX_PASSPHRASE);
            { stdin }.read_to_end(&mut text).map(|_| text)
        }
        path => std::fs::read(path),
    };
    let what = match path {
        "-" => "stdin".to_string(),
        path => path.to_string(),
    };
    let text = read.map_err(|e| {
        let why = e.to_string();
        let why = why.split(" (os error").next().unwrap_or_default();
        anyhow::anyhow!(
            "can't read the passphrase from {what} ({why}): say a file that holds it, or - for \
             stdin: `--passphrase-file PATH`"
        )
    })?;
    let line = text.split(|&b| b == b'\n').next().unwrap_or_default();
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    if line.is_empty() {
        bail!("{what} holds no passphrase: its first line is the passphrase");
    }
    Ok(line.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAST: Cost = Cost {
        memory_kib: 64,
        passes: 1,
        lanes: 1,
    };

    #[test]
    fn what_is_sealed_opens_with_its_passphrase() {
        let sealed = seal(b"open sesame", b"the Host Key", FAST).unwrap();
        assert_eq!(open(b"open sesame", &sealed).unwrap(), b"the Host Key");
        assert!(!sealed.windows(12).any(|w| w == b"the Host Key"));
    }

    #[test]
    fn a_wrong_passphrase_wont_open_it() {
        let sealed = seal(b"open sesame", b"secret", FAST).unwrap();
        let e = open(b"open sesame!", &sealed).unwrap_err();
        assert!(e.is::<WontOpen>(), "{e}");
    }

    #[test]
    fn a_changed_byte_anywhere_wont_open_it() {
        let sealed = seal(b"pass", b"secret", FAST).unwrap();
        for at in [
            MAGIC.len() + 1,
            MAGIC.len() + 13,
            HEADER_LEN - 1,
            sealed.len() - 1,
        ] {
            let mut changed = sealed.clone();
            changed[at] ^= 1;
            assert!(
                open(b"pass", &changed).unwrap_err().is::<WontOpen>(),
                "at {at}"
            );
        }
        assert!(
            open(b"pass", &sealed[..HEADER_LEN])
                .unwrap_err()
                .is::<WontOpen>()
        );
        assert!(open(b"pass", b"not a backup").unwrap_err().is::<WontOpen>());
    }

    #[test]
    fn a_header_asking_too_much_of_argon2id_is_refused_at_once() {
        let sealed = seal(b"pass", b"secret", FAST).unwrap();
        for (i, too_much) in [(0, MAX.memory_kib + 1), (1, u32::MAX), (2, MAX.lanes + 1)] {
            let mut changed = sealed.clone();
            let at = MAGIC.len() + 1 + 4 * i;
            changed[at..at + 4].copy_from_slice(&too_much.to_le_bytes());
            let started = std::time::Instant::now();
            assert!(open(b"pass", &changed).unwrap_err().is::<WontOpen>());
            assert!(started.elapsed() < std::time::Duration::from_secs(1));
        }
    }

    #[test]
    fn a_later_format_says_to_upgrade() {
        let mut sealed = seal(b"pass", b"secret", FAST).unwrap();
        sealed[MAGIC.len()] = 2;
        let e = open(b"pass", &sealed).unwrap_err().to_string();
        assert!(e.contains("format 2") && e.contains("upgrade"), "{e}");
    }

    #[test]
    fn the_default_cost_is_64_mib_and_3_passes() {
        // Unit tests don't set the hook (the CLI tests' `Home` does, for its `tooob`); if someone
        // runs them with it exported, there's no default to check.
        if std::env::var_os("TOOOB_BACKUP_KIB").is_none() {
            assert_eq!(
                Cost::from_env().unwrap(),
                Cost {
                    memory_kib: 65536,
                    passes: 3,
                    lanes: 1
                }
            );
        }
    }
}
