//! What a Host's log says of its Visitors (ADR 0025): each Visit, from getting in to leaving,
//! refusals, and the Owner's changes. Through the `tooob` command with real Hosts on a Relay of
//! their own, and Visitors connecting as a browser does. Never which pages or files they opened.
mod support;

use serde_json::Value;
use support::home::{Home, Running, TestRelay, a_folder};
use tooob::Network;
use tooob::access::Redeemed;
use tooob::visit::{Visit, VisitError};

/// A Visit ends a second after its Visitor's last connection, and a Visit with no Direct Path is
/// said to be relayed after a second: a minute and 10 s otherwise.
const QUICK: [(&str, &str); 2] = [("TOOOB_VISIT_GRACE", "1s"), ("TOOOB_DIRECT_WAIT", "1s")];

/// Host photos, sharing a folder, running on a Relay of its own.
struct Photos {
    home: Home,
    relay: TestRelay,
    running: Running,
    _folder: tempfile::TempDir,
}

impl Photos {
    fn start() -> Photos {
        Photos::start_with(&QUICK)
    }

    fn start_with(env: &[(&str, &str)]) -> Photos {
        let home = Home::new();
        let folder = a_folder();
        let relay = TestRelay::start();
        let path = folder.path().to_str().unwrap();
        home.ok(&["add", "photos", path, "--relay", &relay.url]);
        let running = home.start_with("photos", env);
        Photos {
            home,
            relay,
            running,
            _folder: folder,
        }
    }

    /// An Invite for `label`; its token.
    fn invite(&self, label: &str, flags: &[&str]) -> String {
        let link = self
            .home
            .ok(&[&["invite", "photos", label], flags].concat());
        link.trim().rsplit("#invite=").next().unwrap().to_string()
    }

    /// A connection from the device whose Visitor Key is `key`, as a browser tab makes one.
    async fn connect(&self, key: [u8; 32]) -> Visit {
        let host = iroh::PublicKey::from_z32(&self.home.shown("photos", "Host Key")).unwrap();
        let relay: iroh::RelayUrl = self.relay.url.parse().unwrap();
        Visit::connect(
            iroh::EndpointAddr::new(host).with_relay_url(relay.clone()),
            key,
            Network::Relay(relay),
        )
        .await
        .unwrap()
    }

    /// Wait until its log says `what`.
    fn until_logged(&self, what: &str) {
        self.home.until_logged("photos", what);
    }

    /// Its log, as `tooob logs` shows it, without times.
    fn said(&self) -> Vec<String> {
        let shown = self.home.ok(&["logs", "photos", "-n", "all"]);
        shown
            .lines()
            .map(|l| l.split_once("  photos: ").map_or(l, |(_, said)| said))
            .map(String::from)
            .collect()
    }

    /// Its log's lines that are `event`, as `tooob logs --json` gives them.
    fn events(&self, event: &str) -> Vec<Value> {
        self.home
            .ok(&["logs", "photos", "-n", "all", "--json"])
            .lines()
            .map(|l| serde_json::from_str::<Value>(l).unwrap())
            .filter(|l| l["event"] == event)
            .collect()
    }
}

fn new_key() -> [u8; 32] {
    let mut key = [0u8; 32];
    getrandom::fill(&mut key).unwrap();
    key
}

/// The short Visitor Key the log names a device by.
fn short(key: &[u8; 32]) -> String {
    iroh::SecretKey::from_bytes(key).public().to_string()[..8].to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_visitors_first_visit_is_logged_when_they_get_in_and_when_they_leave() {
    let photos = Photos::start();
    let dave = new_key();
    let token = photos.invite("dave", &[]);
    let tab = photos.connect(dave).await;
    assert_eq!(tab.redeem(&token).await, Ok(Redeemed::Admitted));
    let got_in = format!("dave ({}) got in · first visit · relay", short(&dave));
    photos.until_logged(&got_in);
    tab.close().await;
    let left = format!("dave ({}) left after ", short(&dave));
    photos.until_logged(&left);

    let said = photos.said();
    let at = said.iter().position(|l| *l == got_in).unwrap();
    assert!(said[at + 1].starts_with(&left), "{said:#?}");
    assert!(said[at + 1].ends_with(" s"), "{said:#?}");
    let got_in = &photos.events("got_in")[0];
    assert_eq!(got_in["fields"]["label"], "dave");
    assert_eq!(got_in["fields"]["key"], short(&dave));
    assert_eq!(got_in["fields"]["visit"], "first");
    assert_eq!(got_in["fields"]["path"], "relay");
    let left = &photos.events("left")[0];
    assert_eq!(left["fields"]["key"], short(&dave));
    assert!(left["fields"]["seconds"].as_u64().unwrap() < 30, "{left}");
}

#[tokio::test(flavor = "multi_thread")]
async fn reloads_and_more_tabs_within_the_grace_are_one_visit_and_coming_back_after_is_a_return() {
    let photos = Photos::start_with(&[("TOOOB_VISIT_GRACE", "3s"), ("TOOOB_DIRECT_WAIT", "1s")]);
    let dave = new_key();
    let token = photos.invite("dave", &[]);
    let first = photos.connect(dave).await;
    assert_eq!(first.redeem(&token).await, Ok(Redeemed::Admitted));
    photos.until_logged("got in · first visit");
    let second = photos.connect(dave).await; // another tab
    second.get("/notes.txt").await.unwrap();
    first.close().await;
    second.close().await;
    let reloaded = photos.connect(dave).await; // a reload, well within the grace
    reloaded.get("/notes.txt").await.unwrap();
    reloaded.close().await;
    photos.until_logged("left after");
    assert_eq!(photos.events("got_in").len(), 1, "{:#?}", photos.said());

    let back = photos.connect(dave).await;
    back.get("/notes.txt").await.unwrap();
    let returned = format!("dave ({}) got in · return · relay", short(&dave));
    photos.until_logged(&returned);
    assert_eq!(photos.events("got_in")[1]["fields"]["visit"], "return");
    assert_eq!(photos.events("left").len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_visit_going_on_when_the_host_stops_ends_with_it() {
    // A minute's grace and 10 s for a Direct Path, as for real: the Host stopping says it all.
    let mut photos = Photos::start_with(&[]);
    let dave = new_key();
    let token = photos.invite("dave", &[]);
    let tab = photos.connect(dave).await;
    assert_eq!(tab.redeem(&token).await, Ok(Redeemed::Admitted));
    assert!(photos.running.stop_with(libc::SIGTERM).success());

    let said = photos.said();
    let short = short(&dave);
    let end = &said[said.len() - 3..];
    assert_eq!(
        end[0],
        format!("dave ({short}) got in · first visit · relay")
    );
    assert!(
        end[1].starts_with(&format!("dave ({short}) left after ")),
        "{said:#?}"
    );
    assert_eq!(end[2], "stopped");
}

#[tokio::test(flavor = "multi_thread")]
async fn refusals_are_logged_once_a_device_with_why() {
    let photos = Photos::start();
    let stranger = new_key();
    let tab = photos.connect(stranger).await;
    assert_eq!(tab.get("/notes.txt").await, Err(VisitError::Denied));
    assert_eq!(tab.get("/notes.txt").await, Err(VisitError::Denied)); // asking again
    let not_invited = format!("refused a device ({}) · not invited", short(&stranger));
    photos.until_logged(&not_invited);

    // An Invite past its time, and one used already by another device.
    let late = photos.invite("eve", &["--expires", "1s"]);
    let used = photos.invite("frank", &[]);
    let frank = photos.connect(new_key()).await;
    assert_eq!(frank.redeem(&used).await, Ok(Redeemed::Admitted));
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    let (eve, someone) = (new_key(), new_key());
    let tab = photos.connect(eve).await;
    assert!(tab.redeem(&late).await.is_err());
    assert_eq!(tab.get("/").await, Err(VisitError::Denied));
    let tab = photos.connect(someone).await;
    assert!(tab.redeem(&used).await.is_err());
    let tab = photos.connect(new_key()).await;
    assert!(
        tab.redeem("0123456789abcdef0123456789abcdef")
            .await
            .is_err()
    );
    // frank tapping his link again: he's in, so nothing's refused.
    assert!(frank.redeem(&used).await.is_err());
    frank.get("/").await.unwrap();

    // A device waiting for approval, asking again and again, and a Blocked one.
    let approve = photos.invite("gran", &["--approve"]);
    let gran = new_key();
    let tab = photos.connect(gran).await;
    assert_eq!(tab.redeem(&approve).await, Ok(Redeemed::AwaitingApproval));
    for _ in 0..3 {
        assert_eq!(tab.get("/").await, Err(VisitError::AwaitingApproval));
    }
    let mallory = new_key();
    let token = photos.invite("mallory", &[]);
    let tab = photos.connect(mallory).await;
    assert_eq!(tab.redeem(&token).await, Ok(Redeemed::Admitted));
    photos.home.ok(&["block", "photos", &short(&mallory)]);
    assert_eq!(tab.get("/").await, Err(VisitError::Denied));

    let blocked = format!("refused mallory ({}) · blocked", short(&mallory));
    photos.until_logged(&blocked);
    let refused = photos.events("refused");
    let reasons: Vec<(&str, &str)> = refused
        .iter()
        .map(|r| {
            let f = &r["fields"];
            (f["reason"].as_str().unwrap(), f["key"].as_str().unwrap())
        })
        .collect();
    let eve_short = short(&eve);
    let someone_short = short(&someone);
    let gran_short = short(&gran);
    let mallory_short = short(&mallory);
    let stranger_short = short(&stranger);
    assert_eq!(
        reasons,
        [
            ("unknown_device", stranger_short.as_str()),
            ("expired", eve_short.as_str()),
            ("already_used", someone_short.as_str()),
            ("unknown_invite", reasons[3].1),
            ("awaiting_approval", gran_short.as_str()),
            ("blocked", mallory_short.as_str()),
        ],
        "{:#?}",
        photos.said()
    );
    let said = photos.said();
    let eve_id = &refused[1]["fields"]["invite"];
    assert_eq!(refused[1]["fields"]["label"], "eve");
    assert!(
        said.contains(&format!(
            "refused a device ({eve_short}) · Invite {} for eve · expired",
            eve_id.as_str().unwrap()
        )),
        "{said:#?}"
    );
    assert!(
        said.iter().any(|l| l
            .starts_with(&format!("refused a device ({someone_short}) · Invite "))
            && l.ends_with(" for frank · already used")),
        "{said:#?}"
    );
    assert!(said.contains(&format!("gran ({gran_short}) is waiting for approval")));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_owners_changes_are_logged_with_where_they_came_from_running_or_stopped() {
    let mut photos = Photos::start();
    let (dave, gran) = (new_key(), new_key());
    let tab = photos.connect(dave).await;
    assert_eq!(
        tab.redeem(&photos.invite("dave", &[])).await,
        Ok(Redeemed::Admitted)
    );
    let token = photos.invite("gran", &["--approve", "--expires", "2h"]);
    let tab = photos.connect(gran).await;
    assert_eq!(tab.redeem(&token).await, Ok(Redeemed::AwaitingApproval));
    photos.home.ok(&["block", "photos", &short(&dave)]);
    photos.until_logged("blocked dave");

    assert!(photos.running.stop_with(libc::SIGTERM).success());
    // Commands made while it's stopped write its log themselves.
    photos.home.ok(&["unblock", "photos", &short(&dave)]);
    photos.home.ok(&["approve", "photos", &short(&gran)]);
    photos.invite("sam", &["--owner"]);
    let invites = photos.home.ok(&["invites", "photos", "--json"]);
    let invites: Value = serde_json::from_str(&invites).unwrap();
    let sams = invites
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["label"] == "sam")
        .unwrap();
    let sams = sams["id"].as_str().unwrap();
    photos.home.ok(&["revoke", "photos", sams]);
    photos.home.ok(&["pair", "photos", "my phone"]);

    let said = photos.said();
    let (dave, gran) = (short(&dave), short(&gran));
    let gran_invite = photos.events("invite_made")[1]["fields"]["invite"].clone();
    let gran_invite = gran_invite.as_str().unwrap();
    for line in [
        format!(
            "made Invite {gran_invite} for gran · approve first · works for 2 hours · from the CLI"
        ),
        format!("blocked dave ({dave}) · from the CLI"),
        format!("unblocked dave ({dave}) · from the CLI"),
        format!("approved gran ({gran}) · from the CLI"),
        format!("made Invite {sams} for sam · Owner Invite · works for 7 days · from the CLI"),
        format!("revoked Invite {sams} for sam · from the CLI"),
    ] {
        assert!(said.contains(&line), "{line:?} in {said:#?}");
    }
    assert!(
        said.last().unwrap().starts_with("made Pairing Invite ")
            && said
                .last()
                .unwrap()
                .ends_with(" for my phone · works for 1 hour · from the CLI"),
        "{said:#?}"
    );
    let blocked = &photos.events("blocked")[0]["fields"];
    assert_eq!(blocked["label"], "dave");
    assert_eq!(blocked["key"], dave);
    assert_eq!(blocked["from"], "cli");
    let made = &photos.events("invite_made")[1]["fields"];
    assert_eq!(made["label"], "gran");
    assert_eq!(made["first_visit_approval"], true);
    assert_eq!(made["expires_in"], 7200);
    assert_eq!(made["from"], "cli");
    assert_eq!(photos.events("invite_made")[3]["fields"]["pairing"], true);
    assert_eq!(photos.events("invite_revoked")[0]["fields"]["invite"], sams);
    assert_eq!(photos.events("approved")[0]["fields"]["key"], gran);
    assert_eq!(photos.events("unblocked")[0]["fields"]["from"], "cli");
}
