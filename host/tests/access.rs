//! Access rules for one Host: Invites, the Allowlist, Blocks and Owner Devices (see GLOSSARY.md).
use tooob::access::{
    Access, ChangeError, Decision, InviteInfo, InviteOptions, RedeemError, Redeemed, Status,
    VisitorInfo,
};

const MOM_PHONE: &str = "visitor-key-mom-phone";
const MOM_LAPTOP: &str = "visitor-key-mom-laptop";
const OWNER_LAPTOP: &str = "visitor-key-owner-laptop";
const OWNER_PHONE: &str = "visitor-key-owner-phone";
const STRANGER: &str = "visitor-key-stranger";
const NOW: u64 = 1_800_000_000; // unix seconds
const DAY: u64 = 24 * 60 * 60;

/// Invite and redeem in one step.
fn admit(access: &mut Access, label: &str, visitor_key: &str) {
    let invite = access.invite(label, InviteOptions::default(), NOW).unwrap();
    access.redeem(&invite.token, visitor_key, NOW).unwrap();
}

fn fresh() -> (tempfile::TempDir, Access) {
    let dir = tempfile::tempdir().unwrap();
    let access = Access::open(dir.path().join("access.json")).unwrap();
    (dir, access)
}

#[test]
fn unknown_visitor_is_denied() {
    let (_dir, access) = fresh();
    assert_eq!(access.admits(MOM_PHONE), Decision::Denied);
}

#[test]
fn redeeming_an_invite_admits_the_visitor_under_its_label() {
    let (_dir, mut access) = fresh();
    let invite = access.invite("mom", InviteOptions::default(), NOW).unwrap();
    assert_eq!(
        access.redeem(&invite.token, MOM_PHONE, NOW + 60),
        Ok(Redeemed::Admitted)
    );
    assert_eq!(
        access.admits(MOM_PHONE),
        Decision::Allowed {
            label: "mom".into(),
            owner_device: false
        }
    );
}

#[test]
fn an_invite_works_only_once() {
    let (_dir, mut access) = fresh();
    let invite = access.invite("mom", InviteOptions::default(), NOW).unwrap();
    access.redeem(&invite.token, MOM_PHONE, NOW).unwrap();
    // The link got forwarded: the second device is refused.
    assert_eq!(
        access.redeem(&invite.token, STRANGER, NOW),
        Err(RedeemError::AlreadyUsed)
    );
    assert_eq!(access.admits(STRANGER), Decision::Denied);
}

#[test]
fn an_invite_expires_after_seven_days_by_default() {
    let (_dir, mut access) = fresh();
    let early = access.invite("mom", InviteOptions::default(), NOW).unwrap();
    let late = access.invite("mom", InviteOptions::default(), NOW).unwrap();
    assert_eq!(
        access.redeem(&early.token, MOM_PHONE, NOW + 7 * DAY - 1),
        Ok(Redeemed::Admitted)
    );
    assert_eq!(
        access.redeem(&late.token, MOM_LAPTOP, NOW + 7 * DAY),
        Err(RedeemError::Expired)
    );
    assert_eq!(access.admits(MOM_LAPTOP), Decision::Denied);
}

#[test]
fn blocking_a_lost_phone_leaves_the_same_persons_laptop_working() {
    let (_dir, mut access) = fresh();
    admit(&mut access, "mom", MOM_PHONE);
    admit(&mut access, "mom", MOM_LAPTOP);
    access.block(MOM_PHONE).unwrap();
    assert_eq!(access.admits(MOM_PHONE), Decision::Denied);
    assert_eq!(
        access.admits(MOM_LAPTOP),
        Decision::Allowed {
            label: "mom".into(),
            owner_device: false
        }
    );
}

#[test]
fn a_blocked_visitor_stays_blocked_even_with_a_fresh_invite() {
    let (_dir, mut access) = fresh();
    admit(&mut access, "dave", STRANGER);
    access.block(STRANGER).unwrap();
    let forwarded = access.invite("mom", InviteOptions::default(), NOW).unwrap();
    assert_eq!(
        access.redeem(&forwarded.token, STRANGER, NOW),
        Err(RedeemError::Blocked)
    );
    assert_eq!(access.admits(STRANGER), Decision::Denied);
}

#[test]
fn unblocking_lets_a_visitor_back_in() {
    let (_dir, mut access) = fresh();
    admit(&mut access, "mom", MOM_PHONE);
    access.block(MOM_PHONE).unwrap();
    access.unblock(MOM_PHONE).unwrap();
    assert_eq!(
        access.admits(MOM_PHONE),
        Decision::Allowed {
            label: "mom".into(),
            owner_device: false
        }
    );
}

#[test]
fn an_approve_first_invite_waits_for_the_owner() {
    let (_dir, mut access) = fresh();
    let options = InviteOptions {
        first_visit_approval: true,
        ..Default::default()
    };
    let invite = access.invite("mom", options, NOW).unwrap();
    assert_eq!(
        access.redeem(&invite.token, MOM_PHONE, NOW),
        Ok(Redeemed::AwaitingApproval)
    );
    assert_eq!(
        access.admits(MOM_PHONE),
        Decision::AwaitingApproval {
            label: "mom".into()
        }
    );
    access.approve(MOM_PHONE).unwrap();
    assert_eq!(
        access.admits(MOM_PHONE),
        Decision::Allowed {
            label: "mom".into(),
            owner_device: false
        }
    );
}

#[test]
fn the_first_owner_invite_pairs_without_approval() {
    let (_dir, mut access) = fresh();
    let pairing = access
        .invite(
            "my laptop",
            InviteOptions {
                owner_invite: true,
                ..Default::default()
            },
            NOW,
        )
        .unwrap();
    assert_eq!(
        access.redeem(&pairing.token, OWNER_LAPTOP, NOW),
        Ok(Redeemed::Admitted)
    );
    assert_eq!(
        access.admits(OWNER_LAPTOP),
        Decision::Allowed {
            label: "my laptop".into(),
            owner_device: true
        }
    );
}

#[test]
fn later_owner_invites_need_approval() {
    let (_dir, mut access) = fresh();
    let pairing = access
        .invite(
            "my laptop",
            InviteOptions {
                owner_invite: true,
                ..Default::default()
            },
            NOW,
        )
        .unwrap();
    access.redeem(&pairing.token, OWNER_LAPTOP, NOW).unwrap();
    // Someone who got hold of a second owner Invite must not become an Owner Device on their own.
    let second = access
        .invite(
            "my phone",
            InviteOptions {
                owner_invite: true,
                ..Default::default()
            },
            NOW,
        )
        .unwrap();
    assert_eq!(
        access.redeem(&second.token, OWNER_PHONE, NOW),
        Ok(Redeemed::AwaitingApproval)
    );
    access.approve(OWNER_PHONE).unwrap();
    assert_eq!(
        access.admits(OWNER_PHONE),
        Decision::Allowed {
            label: "my phone".into(),
            owner_device: true
        }
    );
}

#[test]
fn the_owner_can_list_visitors_with_their_status() {
    let (_dir, mut access) = fresh();
    admit(&mut access, "mom", MOM_PHONE);
    admit(&mut access, "dave", STRANGER);
    access.block(STRANGER).unwrap();
    let waiting = access
        .invite(
            "mom",
            InviteOptions {
                first_visit_approval: true,
                ..Default::default()
            },
            NOW,
        )
        .unwrap();
    access.redeem(&waiting.token, MOM_LAPTOP, NOW).unwrap();

    let mut listed = access.visitors();
    listed.sort_by(|a, b| a.key.cmp(&b.key));
    let expected = vec![
        VisitorInfo {
            key: MOM_LAPTOP.into(),
            label: "mom".into(),
            owner_device: false,
            awaiting_owner_approval: false,
            status: Status::AwaitingApproval,
        },
        VisitorInfo {
            key: MOM_PHONE.into(),
            label: "mom".into(),
            owner_device: false,
            awaiting_owner_approval: false,
            status: Status::Allowed,
        },
        VisitorInfo {
            key: STRANGER.into(),
            label: "dave".into(),
            owner_device: false,
            awaiting_owner_approval: false,
            status: Status::Blocked,
        },
    ];
    assert_eq!(listed, expected);
}

#[test]
fn everything_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("access.json");
    let (unused, used);
    {
        let mut access = Access::open(&path).unwrap();
        let pairing = access
            .invite(
                "my laptop",
                InviteOptions {
                    owner_invite: true,
                    ..Default::default()
                },
                NOW,
            )
            .unwrap();
        access.redeem(&pairing.token, OWNER_LAPTOP, NOW).unwrap();
        admit(&mut access, "dave", STRANGER);
        access.block(STRANGER).unwrap();
        let waiting = access
            .invite(
                "mom",
                InviteOptions {
                    first_visit_approval: true,
                    ..Default::default()
                },
                NOW,
            )
            .unwrap();
        access.redeem(&waiting.token, MOM_LAPTOP, NOW).unwrap();
        used = waiting.token;
        unused = access
            .invite("mom", InviteOptions::default(), NOW)
            .unwrap()
            .token;
    }

    let mut access = Access::open(&path).unwrap();
    assert_eq!(
        access.admits(OWNER_LAPTOP),
        Decision::Allowed {
            label: "my laptop".into(),
            owner_device: true
        }
    );
    assert_eq!(access.admits(STRANGER), Decision::Denied);
    assert_eq!(
        access.admits(MOM_LAPTOP),
        Decision::AwaitingApproval {
            label: "mom".into()
        }
    );
    assert_eq!(
        access.redeem(&used, MOM_PHONE, NOW),
        Err(RedeemError::AlreadyUsed)
    );
    assert_eq!(
        access.redeem(&unused, MOM_PHONE, NOW),
        Ok(Redeemed::Admitted)
    );
}

#[test]
fn a_redeem_that_cannot_be_saved_changes_nothing() {
    use std::{fs, os::unix::fs::PermissionsExt};
    let dir = tempfile::tempdir().unwrap();
    let mut access = Access::open(dir.path().join("access.json")).unwrap();
    let invite = access.invite("mom", InviteOptions::default(), NOW).unwrap();

    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o500)).unwrap(); // disk refuses writes
    assert!(matches!(
        access.redeem(&invite.token, MOM_PHONE, NOW),
        Err(RedeemError::Storage(_))
    ));
    assert_eq!(access.admits(MOM_PHONE), Decision::Denied);

    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(
        access.redeem(&invite.token, MOM_PHONE, NOW),
        Ok(Redeemed::Admitted),
        "the Invite is still usable"
    );
}

#[test]
fn an_admitted_device_using_another_invite_is_refused_and_the_invite_stays_usable() {
    let (_dir, mut access) = fresh();
    admit(&mut access, "mom", MOM_PHONE);
    // The laptop's Invite gets tapped on the phone first.
    let for_laptop = access
        .invite(
            "mom",
            InviteOptions {
                first_visit_approval: true,
                ..Default::default()
            },
            NOW,
        )
        .unwrap();
    assert_eq!(
        access.redeem(&for_laptop.token, MOM_PHONE, NOW),
        Err(RedeemError::AlreadyAdmitted)
    );
    assert_eq!(
        access.admits(MOM_PHONE),
        Decision::Allowed {
            label: "mom".into(),
            owner_device: false
        }
    );
    assert_eq!(
        access.redeem(&for_laptop.token, MOM_LAPTOP, NOW),
        Ok(Redeemed::AwaitingApproval)
    );
}

#[test]
fn an_owner_invite_upgrades_an_admitted_device_after_approval_without_losing_access() {
    let (_dir, mut access) = fresh();
    let pairing = access
        .invite(
            "my laptop",
            InviteOptions {
                owner_invite: true,
                ..Default::default()
            },
            NOW,
        )
        .unwrap();
    access.redeem(&pairing.token, OWNER_LAPTOP, NOW).unwrap();
    admit(&mut access, "my phone", OWNER_PHONE); // started out as an ordinary Visitor

    let upgrade = access
        .invite(
            "my phone",
            InviteOptions {
                owner_invite: true,
                ..Default::default()
            },
            NOW,
        )
        .unwrap();
    assert_eq!(
        access.redeem(&upgrade.token, OWNER_PHONE, NOW),
        Ok(Redeemed::AwaitingApproval)
    );
    assert_eq!(
        access.admits(OWNER_PHONE),
        Decision::Allowed {
            label: "my phone".into(),
            owner_device: false
        }
    );
    access.approve(OWNER_PHONE).unwrap();
    assert_eq!(
        access.admits(OWNER_PHONE),
        Decision::Allowed {
            label: "my phone".into(),
            owner_device: true
        }
    );
}

#[test]
fn a_pending_owner_upgrade_shows_up_in_the_listing() {
    let (_dir, mut access) = fresh();
    let pairing = access
        .invite(
            "my laptop",
            InviteOptions {
                owner_invite: true,
                ..Default::default()
            },
            NOW,
        )
        .unwrap();
    access.redeem(&pairing.token, OWNER_LAPTOP, NOW).unwrap();
    admit(&mut access, "my phone", OWNER_PHONE);
    let upgrade = access
        .invite(
            "my phone",
            InviteOptions {
                owner_invite: true,
                ..Default::default()
            },
            NOW,
        )
        .unwrap();
    access.redeem(&upgrade.token, OWNER_PHONE, NOW).unwrap();

    let phone = access
        .visitors()
        .into_iter()
        .find(|v| v.key == OWNER_PHONE)
        .unwrap();
    assert_eq!(
        (
            phone.status,
            phone.owner_device,
            phone.awaiting_owner_approval
        ),
        (Status::Allowed, false, true)
    );
}

#[test]
fn an_owner_device_using_another_owner_invite_stays_an_owner_device() {
    let (_dir, mut access) = fresh();
    let pairing = access
        .invite(
            "my laptop",
            InviteOptions {
                owner_invite: true,
                ..Default::default()
            },
            NOW,
        )
        .unwrap();
    access.redeem(&pairing.token, OWNER_LAPTOP, NOW).unwrap();
    let for_phone = access
        .invite(
            "my phone",
            InviteOptions {
                owner_invite: true,
                ..Default::default()
            },
            NOW,
        )
        .unwrap();
    assert_eq!(
        access.redeem(&for_phone.token, OWNER_LAPTOP, NOW),
        Err(RedeemError::AlreadyAdmitted)
    );
    assert_eq!(
        access.admits(OWNER_LAPTOP),
        Decision::Allowed {
            label: "my laptop".into(),
            owner_device: true
        }
    );
    assert_eq!(
        access.redeem(&for_phone.token, OWNER_PHONE, NOW),
        Ok(Redeemed::AwaitingApproval),
        "still usable"
    );
}

#[test]
fn a_block_cancels_a_pending_owner_upgrade() {
    let (_dir, mut access) = fresh();
    let pairing = access
        .invite(
            "my laptop",
            InviteOptions {
                owner_invite: true,
                ..Default::default()
            },
            NOW,
        )
        .unwrap();
    access.redeem(&pairing.token, OWNER_LAPTOP, NOW).unwrap();
    admit(&mut access, "dave", STRANGER);
    let upgrade = access
        .invite(
            "dave",
            InviteOptions {
                owner_invite: true,
                ..Default::default()
            },
            NOW,
        )
        .unwrap();
    access.redeem(&upgrade.token, STRANGER, NOW).unwrap();

    access.block(STRANGER).unwrap();
    assert_eq!(
        access.approve(STRANGER),
        Err(ChangeError::Blocked),
        "a Block wins over approval"
    );
    access.unblock(STRANGER).unwrap();
    assert_eq!(
        access.admits(STRANGER),
        Decision::Allowed {
            label: "dave".into(),
            owner_device: false
        }
    );
}

#[test]
fn changing_an_unknown_visitor_is_an_error_not_a_silent_success() {
    let (_dir, mut access) = fresh();
    admit(&mut access, "mom", MOM_PHONE);
    let typo = "visitor-key-mom-phoen";
    assert_eq!(access.block(typo), Err(ChangeError::UnknownVisitor));
    assert_eq!(access.approve(typo), Err(ChangeError::UnknownVisitor));
    assert_eq!(access.unblock(typo), Err(ChangeError::UnknownVisitor));
}

#[test]
fn a_block_or_approval_that_cannot_be_saved_changes_nothing() {
    use std::{fs, os::unix::fs::PermissionsExt};
    let (dir, mut access) = fresh();
    admit(&mut access, "mom", MOM_PHONE);
    let waiting = access
        .invite(
            "mom",
            InviteOptions {
                first_visit_approval: true,
                ..Default::default()
            },
            NOW,
        )
        .unwrap();
    access.redeem(&waiting.token, MOM_LAPTOP, NOW).unwrap();

    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o500)).unwrap(); // disk refuses writes
    assert!(matches!(
        access.block(MOM_PHONE),
        Err(ChangeError::Storage(_))
    ));
    assert!(matches!(
        access.approve(MOM_LAPTOP),
        Err(ChangeError::Storage(_))
    ));
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();

    // Memory still matches disk: the Owner was told the Block failed, so it must not half-apply.
    assert_eq!(
        access.admits(MOM_PHONE),
        Decision::Allowed {
            label: "mom".into(),
            owner_device: false
        }
    );
    assert_eq!(
        access.admits(MOM_LAPTOP),
        Decision::AwaitingApproval {
            label: "mom".into()
        }
    );
}

#[test]
fn unblocking_only_lifts_the_block_a_waiting_device_waits_again() {
    let (_dir, mut access) = fresh();
    let waiting = access
        .invite(
            "mom",
            InviteOptions {
                first_visit_approval: true,
                ..Default::default()
            },
            NOW,
        )
        .unwrap();
    access.redeem(&waiting.token, MOM_LAPTOP, NOW).unwrap();
    access.block(MOM_LAPTOP).unwrap();
    access.unblock(MOM_LAPTOP).unwrap();
    assert_eq!(
        access.admits(MOM_LAPTOP),
        Decision::AwaitingApproval {
            label: "mom".into()
        }
    );
}

#[test]
fn first_run_pairing_admits_a_device_that_was_waiting() {
    let (_dir, mut access) = fresh();
    let waiting = access
        .invite(
            "laptop",
            InviteOptions {
                first_visit_approval: true,
                ..Default::default()
            },
            NOW,
        )
        .unwrap();
    access.redeem(&waiting.token, OWNER_LAPTOP, NOW).unwrap();
    let pairing = access
        .invite(
            "my laptop",
            InviteOptions {
                owner_invite: true,
                ..Default::default()
            },
            NOW,
        )
        .unwrap();
    assert_eq!(
        access.redeem(&pairing.token, OWNER_LAPTOP, NOW),
        Ok(Redeemed::Admitted)
    );
    assert_eq!(
        access.admits(OWNER_LAPTOP),
        Decision::Allowed {
            label: "laptop".into(),
            owner_device: true
        }
    );
}

#[test]
fn an_invite_can_be_given_a_shorter_life() {
    let (_dir, mut access) = fresh();
    let hour = || InviteOptions {
        expires_in: Some(60 * 60),
        ..Default::default()
    };
    let early = access.invite("mom", hour(), NOW).unwrap();
    let late = access.invite("mom", hour(), NOW).unwrap();
    assert_eq!(
        access.redeem(&early.token, MOM_PHONE, NOW + 60 * 60 - 1),
        Ok(Redeemed::Admitted)
    );
    assert_eq!(
        access.redeem(&late.token, MOM_LAPTOP, NOW + 60 * 60),
        Err(RedeemError::Expired)
    );
}

#[test]
fn a_pairing_invite_makes_an_owner_device_without_approval_even_after_first_run() {
    let (_dir, mut access) = fresh();
    let first = access
        .invite(
            "my laptop",
            InviteOptions {
                owner_invite: true,
                ..Default::default()
            },
            NOW,
        )
        .unwrap();
    access.redeem(&first.token, OWNER_LAPTOP, NOW).unwrap();
    access.block(OWNER_LAPTOP).unwrap(); // the only Owner Device was stolen

    let pairing = access.pairing_invite("new phone", None, NOW).unwrap();
    assert_eq!(
        access.redeem(&pairing.token, OWNER_PHONE, NOW),
        Ok(Redeemed::Admitted)
    );
    assert_eq!(
        access.admits(OWNER_PHONE),
        Decision::Allowed {
            label: "new phone".into(),
            owner_device: true
        }
    );
}

#[test]
fn a_pairing_invite_lasts_an_hour_by_default() {
    let (_dir, mut access) = fresh();
    let early = access.pairing_invite("my phone", None, NOW).unwrap();
    let late = access.pairing_invite("my tablet", None, NOW).unwrap();
    assert_eq!(
        access.redeem(&early.token, OWNER_PHONE, NOW + 60 * 60 - 1),
        Ok(Redeemed::Admitted)
    );
    assert_eq!(
        access.redeem(&late.token, OWNER_LAPTOP, NOW + 60 * 60),
        Err(RedeemError::Expired)
    );
}

#[test]
fn invite_tokens_are_never_stored_in_the_clear() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("access.json");
    let invite = Access::open(&path)
        .unwrap()
        .invite("mom", InviteOptions::default(), NOW)
        .unwrap();
    // What's on disk is the requirement: a backup or stolen copy of the file holds no usable link.
    assert!(
        !std::fs::read_to_string(&path)
            .unwrap()
            .contains(&invite.token)
    );
    let mut reopened = Access::open(&path).unwrap();
    assert_eq!(
        reopened.redeem(&invite.token, MOM_PHONE, NOW),
        Ok(Redeemed::Admitted)
    );
}

#[test]
fn the_owner_can_list_unused_invites_and_revoke_one() {
    let (_dir, mut access) = fresh();
    let wrong_chat = access
        .invite("dave", InviteOptions::default(), NOW)
        .unwrap();
    let for_mom = access
        .invite(
            "mom",
            InviteOptions {
                expires_in: Some(DAY),
                ..Default::default()
            },
            NOW,
        )
        .unwrap();
    admit(&mut access, "used already", MOM_LAPTOP);

    let mut listed = access.invites(NOW);
    listed.sort_by(|a, b| a.label.cmp(&b.label));
    assert_eq!(
        listed,
        vec![
            InviteInfo {
                id: wrong_chat.id.clone(),
                label: "dave".into(),
                expires_at: NOW + 7 * DAY,
                owner_invite: false
            },
            InviteInfo {
                id: for_mom.id.clone(),
                label: "mom".into(),
                expires_at: NOW + DAY,
                owner_invite: false
            },
        ]
    );
    access.revoke(&wrong_chat.id).unwrap();
    assert_eq!(
        access.redeem(&wrong_chat.token, STRANGER, NOW),
        Err(RedeemError::UnknownInvite)
    );
    assert_eq!(access.invites(NOW).len(), 1);
}

#[test]
fn expired_invites_are_forgotten_when_the_next_invite_is_made() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("access.json");
    let mut access = Access::open(&path).unwrap();
    let old = access.invite("mom", InviteOptions::default(), NOW).unwrap();
    access.redeem(&old.token, MOM_PHONE, NOW).unwrap(); // used ones go too
    let unused = access
        .invite("dave", InviteOptions::default(), NOW)
        .unwrap();
    let size_before = std::fs::metadata(&path).unwrap().len();

    access
        .invite("mom", InviteOptions::default(), NOW + 8 * DAY)
        .unwrap();
    assert_eq!(
        access.redeem(&unused.token, STRANGER, NOW + 8 * DAY),
        Err(RedeemError::UnknownInvite)
    );
    assert_eq!(
        access.redeem(&old.token, STRANGER, NOW + 8 * DAY),
        Err(RedeemError::UnknownInvite)
    );
    assert!(
        std::fs::metadata(&path).unwrap().len() < size_before,
        "the file stops growing"
    );
}
