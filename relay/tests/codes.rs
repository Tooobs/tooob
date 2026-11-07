//! A Host redeems an Enrollment Code as it connects (ADR 0030, #129): through real relay clients
//! presenting it as iroh's auth token, against a running `tooob-relay`.
use std::path::{Path, PathBuf};

use iroh_base::{EndpointId, RelayUrl, SecretKey};
use iroh_relay::{
    client::{Client, ClientBuilder},
    tls::{CaTlsConfig, default_provider},
};
use tooob_relay::{List, Relay};

async fn relay(list: &Path) -> Relay {
    Relay::spawn_dev(([127, 0, 0, 1], 0), list.to_path_buf())
        .await
        .unwrap()
}

/// A client of `relay`'s with a fresh key, presenting `code` if given: the client, its key, or
/// why the Relay refused it.
async fn connect(relay: &Relay, code: Option<&str>) -> (Result<Client, String>, EndpointId) {
    let secret = SecretKey::from_bytes(&rand::random());
    let key = secret.public();
    (connect_as(relay, secret, code).await, key)
}

async fn connect_as(
    relay: &Relay,
    secret: SecretKey,
    code: Option<&str>,
) -> Result<Client, String> {
    let url: RelayUrl = format!("http://{}", relay.http_addr()).parse().unwrap();
    let tls = CaTlsConfig::default()
        .client_config(default_provider())
        .unwrap();
    let mut builder =
        ClientBuilder::new(url, secret, iroh_dns::dns::DnsResolver::new()).tls_client_config(tls);
    if let Some(code) = code {
        builder = builder.auth_token(code);
    }
    builder.connect().await.map_err(|e| format!("{e:#}"))
}

/// A list file with one code made for `label`, good for `uses` until `expires`: its path, code.
fn with_code(dir: &Path, label: &str, uses: u32, expires: u64) -> (PathBuf, String) {
    let path = dir.join("enrolled.toml");
    let mut list = List::load(&path).unwrap();
    let code = list.make_code(label, uses, expires);
    list.save(&path).unwrap();
    (path, format!("relay.example.com/{code}"))
}

/// Why the Relay refused a connection, which it should have.
fn refusal(connected: Result<Client, String>, otherwise: &str) -> String {
    match connected {
        Ok(_) => panic!("{otherwise}"),
        Err(reason) => reason,
    }
}

fn in_a_week() -> u64 {
    tooob_relay::now() + 7 * 86_400
}

fn enrolled(path: &Path) -> Vec<(EndpointId, String)> {
    List::load(path)
        .unwrap()
        .entries()
        .map(|(k, l)| (k, l.to_string()))
        .collect()
}

#[tokio::test]
async fn a_host_with_a_valid_code_is_enrolled_under_its_label_and_the_code_is_spent() {
    let dir = tempfile::tempdir().unwrap();
    let (path, code) = with_code(dir.path(), "sam", 1, in_a_week());
    let relay = relay(&path).await;
    let (host, key) = connect(&relay, Some(&code)).await;
    host.unwrap();
    assert_eq!(enrolled(&path), vec![(key, "sam".to_string())]);
    assert_eq!(
        List::load(&path)
            .unwrap()
            .live_codes(tooob_relay::now())
            .count(),
        0
    );
}

#[tokio::test]
async fn a_code_for_two_uses_enrolls_two_hosts_and_refuses_a_third() {
    let dir = tempfile::tempdir().unwrap();
    let (path, code) = with_code(dir.path(), "family", 2, in_a_week());
    let relay = relay(&path).await;
    let (first, _) = connect(&relay, Some(&code)).await;
    let (second, _) = connect(&relay, Some(&code)).await;
    let (third, third_key) = connect(&relay, Some(&code)).await;
    first.unwrap();
    second.unwrap();
    let refused = refusal(third, "a third Host was let in");
    assert!(refused.contains("used up"), "{refused}");
    let keys = enrolled(&path);
    assert_eq!(keys.len(), 2);
    assert!(keys.iter().all(|(k, l)| *k != third_key && l == "family"));
}

#[tokio::test]
async fn an_expired_cancelled_or_wrong_code_enrolls_nothing_and_says_which() {
    let dir = tempfile::tempdir().unwrap();
    let (path, expired) = with_code(dir.path(), "late", 1, tooob_relay::now() - 1);
    let (_, cancelled) = with_code(dir.path(), "gone", 1, in_a_week());
    let mut list = List::load(&path).unwrap();
    assert_eq!(list.cancel("gone", tooob_relay::now()), 1);
    list.save(&path).unwrap();
    let relay = relay(&path).await;
    for (code, says) in [
        (expired.as_str(), "expired"),
        (cancelled.as_str(), "cancelled"),
        ("relay.example.com/0000-0000-0000-0000", "wrong"),
    ] {
        let (refused, _) = connect(&relay, Some(code)).await;
        let refused = refusal(refused, "let in with a bad code");
        assert!(refused.contains(says), "{says}: {refused}");
    }
    assert!(enrolled(&path).is_empty());
}

#[tokio::test]
async fn after_several_wrong_codes_an_address_is_refused_even_a_good_one() {
    let dir = tempfile::tempdir().unwrap();
    let (path, good) = with_code(dir.path(), "sam", 1, in_a_week());
    let relay = relay(&path).await;
    for _ in 0..tooob_relay::WRONG_CODES_ALLOWED {
        let (refused, _) = connect(&relay, Some("relay.example.com/0000-0000-0000-0000")).await;
        assert!(refused.err().unwrap().contains("wrong"));
    }
    let (refused, _) = connect(&relay, Some(&good)).await;
    let refused = refusal(refused, "let in after too many wrong codes");
    assert!(refused.contains("too many wrong"), "{refused}");
    assert!(enrolled(&path).is_empty());
    // A connection with no code is still admitted, under the forwarding rule.
    connect(&relay, None).await.0.unwrap();
}

#[tokio::test]
async fn an_enrolled_host_reconnecting_with_its_spent_code_is_let_in() {
    let dir = tempfile::tempdir().unwrap();
    let (path, code) = with_code(dir.path(), "sam", 1, in_a_week());
    let relay = relay(&path).await;
    let secret = SecretKey::from_bytes(&rand::random());
    let first = connect_as(&relay, secret.clone(), Some(&code))
        .await
        .unwrap();
    drop(first);
    connect_as(&relay, secret, Some(&code)).await.unwrap();
}

#[tokio::test]
async fn a_host_with_no_code_is_admitted_and_enrolls_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (path, _) = with_code(dir.path(), "sam", 1, in_a_week());
    let relay = relay(&path).await;
    connect(&relay, None).await.0.unwrap();
    assert!(enrolled(&path).is_empty());
}
