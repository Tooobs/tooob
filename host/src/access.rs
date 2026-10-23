//! Who may reach one Host's Site: Invites, the Allowlist, Blocks and Owner Devices.
//! The Allowlist lives only here, on the Host (ADR 0002).
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, fs, io, path::PathBuf};

/// How long an Invite Link works if nobody uses it.
const INVITE_LIFETIME_SECS: u64 = 7 * 24 * 60 * 60;
/// Pairing Invites are the most powerful Invite and normally used on the spot.
const PAIRING_LIFETIME_SECS: u64 = 60 * 60;

#[derive(Debug, PartialEq, Eq)]
pub enum Decision {
    Allowed {
        label: String,
        owner_device: bool,
    },
    /// Redeemed an Invite that needs the Owner's approval first.
    AwaitingApproval {
        label: String,
    },
    Denied,
}

/// A Visitor as the Owner sees it in a listing.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisitorInfo {
    pub key: String,
    pub label: String,
    pub owner_device: bool,
    /// Used an Owner Invite; becomes an Owner Device once approved.
    pub awaiting_owner_approval: bool,
    pub status: Status,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status {
    Allowed,
    AwaitingApproval,
    Blocked,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)] // any option left out takes its default
pub struct InviteOptions {
    /// First-Visit Approval: admit only once the Owner approves the device that used the Invite.
    /// Invites an Agent proposes carry it once the Owner approves them (ADR 0006).
    pub first_visit_approval: bool,
    /// An Owner Invite: the device that uses it becomes an Owner Device once approved (no approval
    /// needed at First-Run Pairing).
    pub owner_invite: bool,
    /// Seconds until the Invite Link stops working; 7 days if unset.
    pub expires_in: Option<u64>,
}

/// An Invite as handed to the Owner, once: the token goes into the Invite Link; the id names it
/// later (listing, revoking), since the Host only keeps a hash of the token.
#[derive(Debug, Serialize, Deserialize)]
pub struct Invite {
    pub token: String,
    pub id: String,
}

/// An unused Invite as the Owner sees it in a listing. Never includes the link.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InviteInfo {
    pub id: String,
    pub label: String,
    pub expires_at: u64,
    pub owner_invite: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Redeemed {
    Admitted,
    AwaitingApproval,
}

#[derive(Debug, PartialEq, Eq)]
pub enum RedeemError {
    UnknownInvite,
    AlreadyUsed,
    Expired,
    Blocked,
    /// This device is already a Visitor; the Invite is left for the device it was meant for.
    AlreadyAdmitted,
    /// The change couldn't be saved; nothing was changed.
    Storage(String),
}

/// Why the Owner's change to a Visitor (approve, Block, unblock) didn't happen.
#[derive(Debug, PartialEq, Eq)]
pub enum ChangeError {
    /// No unused Invite has this id.
    UnknownInvite,
    /// No Visitor has this key (a typo shouldn't look like a successful Block).
    UnknownVisitor,
    /// A Block wins: a blocked Visitor can't be approved.
    Blocked,
    /// The change couldn't be saved; nothing was changed.
    Storage(String),
}

#[derive(Serialize, Deserialize)]
struct StoredInvite {
    label: String,
    used: bool,
    expires_at: u64,
    first_visit_approval: bool,
    owner_invite: bool,
    /// Made on the home box: becomes an Owner Device without approval (ADR 0002: the Host decides).
    #[serde(default)]
    pairing: bool,
}

#[derive(Clone, Serialize, Deserialize)]
struct Visitor {
    label: String,
    blocked: bool,
    approved: bool,
    owner_device: bool,
    /// Used an Owner Invite and awaits approval to become an Owner Device; keeps its access meanwhile.
    #[serde(default)]
    awaiting_owner_approval: bool,
}

/// Everything that must survive a restart.
#[derive(Default, Serialize, Deserialize)]
struct State {
    invites: HashMap<String, StoredInvite>,
    visitors: HashMap<String, Visitor>,
}

pub struct Access {
    path: PathBuf,
    state: State,
}

impl std::fmt::Debug for Access {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Access")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl Access {
    /// Load this Host's access state from `path`, or start empty.
    pub fn open(path: impl Into<PathBuf>) -> io::Result<Access> {
        let path = path.into();
        let state = match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(io::Error::other)?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => State::default(),
            Err(e) => return Err(e),
        };
        Ok(Access { path, state })
    }

    /// Write the whole state, owner-only: it holds unused Invite tokens.
    // ponytail: rewrites everything on each change; fine for a household's Visitors, use SQLite past thousands.
    fn save(&self) -> io::Result<()> {
        let bytes = serde_json::to_vec_pretty(&self.state).map_err(io::Error::other)?;
        crate::save_privately(&self.path, &bytes)
    }

    pub fn invite(&mut self, label: &str, options: InviteOptions, now: u64) -> io::Result<Invite> {
        self.store_invite(
            StoredInvite {
                label: label.to_string(),
                used: false,
                expires_at: now.saturating_add(options.expires_in.unwrap_or(INVITE_LIFETIME_SECS)),
                first_visit_approval: options.first_visit_approval,
                owner_invite: options.owner_invite,
                pairing: false,
            },
            now,
        )
    }

    /// A Pairing Invite, for `tooob pair` on the home box only: the device that uses it becomes an
    /// Owner Device at once, even when others exist. Physical access counts as ownership; it's how an
    /// Owner recovers when every Owner Device is lost or blocked. Never expose it remotely (ADR 0006).
    pub fn pairing_invite(
        &mut self,
        label: &str,
        expires_in: Option<u64>,
        now: u64,
    ) -> io::Result<Invite> {
        self.store_invite(
            StoredInvite {
                label: label.to_string(),
                used: false,
                expires_at: now.saturating_add(expires_in.unwrap_or(PAIRING_LIFETIME_SECS)),
                first_visit_approval: false,
                owner_invite: true,
                pairing: true,
            },
            now,
        )
    }

    /// Save a new Invite, forgetting expired ones (used or not) so the file doesn't grow forever.
    fn store_invite(&mut self, invite: StoredInvite, now: u64) -> io::Result<Invite> {
        let token = new_token()?;
        let id = invite_id(&token);
        let short = short_id(&id);
        let expired: Vec<_> = self
            .state
            .invites
            .extract_if(|_, i| now >= i.expires_at)
            .collect();
        self.state.invites.insert(id.clone(), invite);
        if let Err(e) = self.save() {
            self.state.invites.remove(&id); // nobody has the link; don't keep a phantom Invite
            self.state.invites.extend(expired);
            return Err(e);
        }
        Ok(Invite { token, id: short })
    }

    pub fn redeem(
        &mut self,
        token: &str,
        visitor_key: &str,
        now: u64,
    ) -> Result<Redeemed, RedeemError> {
        let id = invite_id(token);
        let invite = self
            .state
            .invites
            .get(&id)
            .ok_or(RedeemError::UnknownInvite)?;
        if invite.used {
            return Err(RedeemError::AlreadyUsed);
        }
        if now >= invite.expires_at {
            return Err(RedeemError::Expired);
        }
        // First-Run Pairing (no Owner Device has existed; blocked ones count) or a Pairing Invite.
        let pairing = invite.owner_invite
            && (invite.pairing || !self.state.visitors.values().any(|v| v.owner_device));
        let existing = self.state.visitors.get(visitor_key);
        // A Block wins over any Invite. Refused Invites aren't used up: they may be meant for another device.
        let updated = match existing {
            Some(v) if v.blocked => return Err(RedeemError::Blocked),
            // An Owner Invite upgrades an ordinary admitted device; it keeps its access while that awaits
            // approval. A device that's already an Owner Device (or on its way) has nothing to upgrade.
            Some(v) if invite.owner_invite && !v.owner_device && !v.awaiting_owner_approval => {
                Visitor {
                    owner_device: pairing,
                    awaiting_owner_approval: !pairing,
                    // Pairing is the Owner admitting themselves: nothing left to approve.
                    approved: v.approved || pairing,
                    ..v.clone()
                }
            }
            Some(_) => return Err(RedeemError::AlreadyAdmitted),
            None => Visitor {
                label: invite.label.clone(),
                blocked: false,
                approved: pairing || !(invite.first_visit_approval || invite.owner_invite),
                owner_device: invite.owner_invite,
                awaiting_owner_approval: false,
            },
        };
        let outcome = if updated.approved && !updated.awaiting_owner_approval {
            Redeemed::Admitted
        } else {
            Redeemed::AwaitingApproval
        };

        let previous = self.state.visitors.insert(visitor_key.to_string(), updated);
        self.state.invites.get_mut(&id).expect("checked above").used = true;
        if let Err(e) = self.save() {
            // Undo, so memory matches disk: the Invite stays usable and the device keeps its old standing.
            match previous {
                Some(v) => self.state.visitors.insert(visitor_key.to_string(), v),
                None => self.state.visitors.remove(visitor_key),
            };
            self.state.invites.get_mut(&id).expect("checked above").used = false;
            return Err(RedeemError::Storage(e.to_string()));
        }
        Ok(outcome)
    }

    /// Unused Invites that still work.
    pub fn invites(&self, now: u64) -> Vec<InviteInfo> {
        self.state
            .invites
            .iter()
            .filter(|(_, i)| !i.used && now < i.expires_at)
            .map(|(id, i)| InviteInfo {
                id: short_id(id),
                label: i.label.clone(),
                expires_at: i.expires_at,
                owner_invite: i.owner_invite,
            })
            .collect()
    }

    /// Kill an Invite Link before anyone uses it (sent to the wrong chat). Takes access away, so an
    /// Agent may do it directly (ADR 0006).
    pub fn revoke(&mut self, id: &str) -> Result<(), ChangeError> {
        let key = self
            .state
            .invites
            .keys()
            .find(|k| short_id(k) == id)
            .cloned()
            .ok_or(ChangeError::UnknownInvite)?;
        let removed = self.state.invites.remove(&key).expect("just found");
        if let Err(e) = self.save() {
            self.state.invites.insert(key, removed);
            return Err(ChangeError::Storage(e.to_string()));
        }
        Ok(())
    }

    pub fn admits(&self, visitor_key: &str) -> Decision {
        match self.state.visitors.get(visitor_key) {
            Some(v) if v.blocked => Decision::Denied,
            Some(v) if !v.approved => Decision::AwaitingApproval {
                label: v.label.clone(),
            },
            Some(v) => Decision::Allowed {
                label: v.label.clone(),
                owner_device: v.owner_device,
            },
            None => Decision::Denied,
        }
    }

    pub fn visitors(&self) -> Vec<VisitorInfo> {
        self.state
            .visitors
            .iter()
            .map(|(key, v)| VisitorInfo {
                key: key.clone(),
                label: v.label.clone(),
                owner_device: v.owner_device,
                awaiting_owner_approval: v.awaiting_owner_approval,
                status: if v.blocked {
                    Status::Blocked
                } else if !v.approved {
                    Status::AwaitingApproval
                } else {
                    Status::Allowed
                },
            })
            .collect()
    }

    /// The Owner approves a Visitor that is awaiting approval (first visit, or becoming an Owner Device).
    pub fn approve(&mut self, visitor_key: &str) -> Result<(), ChangeError> {
        self.change(visitor_key, |v| {
            if v.blocked {
                return Err(ChangeError::Blocked);
            }
            v.approved = true;
            if v.awaiting_owner_approval {
                v.owner_device = true;
                v.awaiting_owner_approval = false;
            }
            Ok(())
        })
    }

    pub fn unblock(&mut self, visitor_key: &str) -> Result<(), ChangeError> {
        self.change(visitor_key, |v| {
            v.blocked = false;
            Ok(())
        })
    }

    /// Block one Visitor. Other Visitors with the same Label are unaffected. A pending upgrade to
    /// Owner Device is cancelled: a Block wins.
    pub fn block(&mut self, visitor_key: &str) -> Result<(), ChangeError> {
        self.change(visitor_key, |v| {
            v.blocked = true;
            v.awaiting_owner_approval = false;
            Ok(())
        })
    }

    /// Apply an Owner's change to one Visitor and save it, or change nothing: memory never gets ahead
    /// of disk (a Block the Owner was told failed must not half-apply, or quietly vanish on restart).
    fn change(
        &mut self,
        visitor_key: &str,
        edit: impl FnOnce(&mut Visitor) -> Result<(), ChangeError>,
    ) -> Result<(), ChangeError> {
        let v = self
            .state
            .visitors
            .get_mut(visitor_key)
            .ok_or(ChangeError::UnknownVisitor)?;
        let before = v.clone();
        let result =
            edit(v).and_then(|()| self.save().map_err(|e| ChangeError::Storage(e.to_string())));
        if result.is_err() {
            self.state.visitors.insert(visitor_key.to_string(), before);
        }
        result
    }
}

/// Invites are stored under a hash of their token, so the access file (and its backups) holds no
/// usable Invite Links: a link is shown once, when it's made.
fn invite_id(token: &str) -> String {
    Sha256::digest(token.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The short name the Owner sees for an Invite: enough to tell a household's Invites apart.
fn short_id(id: &str) -> String {
    id[..8].to_string()
}

/// 128 random bits, hex: unguessable, and fits in a URL fragment.
fn new_token() -> io::Result<String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(io::Error::other)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
