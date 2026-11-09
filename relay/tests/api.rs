//! The enroll API (ADR 0031, #140): a Relay Operator's Sign-up Page enrolls and removes keys with
//! an Operator Token, through tooob-relay's status listener, which the image's Caddy routes to.
use std::{net::SocketAddr, path::Path, time::Duration};

use iroh_base::{EndpointId, RelayUrl, SecretKey};
use iroh_relay::{
    client::{Client, ClientBuilder},
    tls::{CaTlsConfig, default_provider},
};
use n0_future::StreamExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tooob_relay::{List, Relay};

/// A Relay with its status listener, whose Enrolled Keys are in `list`; where it listens.
async fn relay(list: &Path) -> (Relay, SocketAddr) {
    let mut relay = Relay::spawn_dev(([127, 0, 0, 1], 0), list.to_path_buf())
        .await
        .unwrap();
    let at = relay.serve_status(([127, 0, 0, 1], 0)).await.unwrap();
    (relay, at)
}

/// A client of `relay`'s, with a fresh key.
async fn client(relay: &Relay) -> (Client, EndpointId) {
    let secret = SecretKey::from_bytes(&rand::random());
    let key = secret.public();
    let url: RelayUrl = format!("http://{}", relay.http_addr()).parse().unwrap();
    let tls = CaTlsConfig::default()
        .client_config(default_provider())
        .unwrap();
    let client = ClientBuilder::new(url, secret, iroh_dns::dns::DnsResolver::new())
        .tls_client_config(tls)
        .connect()
        .await
        .unwrap();
    (client, key)
}

fn stranger() -> EndpointId {
    SecretKey::from_bytes(&rand::random()).public()
}

/// The status code and body of a raw `request` (its request line and headers, without the blank
/// line) with `body`.
async fn send(at: SocketAddr, request: &str, body: &str) -> (u16, String) {
    let mut stream = tokio::net::TcpStream::connect(at).await.unwrap();
    let whole = format!(
        "{request}\r\nHost: r\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(whole.as_bytes()).await.unwrap();
    let mut reply = String::new();
    stream.read_to_string(&mut reply).await.unwrap();
    let status = reply[9..12].parse().unwrap();
    let body = reply.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
    (status, body)
}

/// `PUT /__tooob/api/enrolled/<key>` with `token`, if any, from `from` (as Caddy says it).
async fn put(at: SocketAddr, key: &str, token: Option<&str>, from: &str, label: &str) -> u16 {
    let auth = token
        .map(|t| format!("\r\nAuthorization: Bearer {t}"))
        .unwrap_or_default();
    let request =
        format!("PUT /__tooob/api/enrolled/{key} HTTP/1.1\r\nX-Forwarded-For: {from}{auth}");
    send(at, &request, &format!(r#"{{"label":"{label}"}}"#))
        .await
        .0
}

async fn delete(at: SocketAddr, key: &str, token: &str) -> u16 {
    let request =
        format!("DELETE /__tooob/api/enrolled/{key} HTTP/1.1\r\nAuthorization: Bearer {token}");
    send(at, &request, "").await.0
}

fn make_token(list: &Path, label: &str) -> String {
    List::edit(list, |keys| keys.make_operator_token(label)).unwrap()
}

fn enrolled(list: &Path) -> Vec<(EndpointId, String)> {
    List::load(list)
        .unwrap()
        .entries()
        .map(|(k, l)| (k, l.to_string()))
        .collect()
}

const ME: &str = "192.0.2.1";

#[tokio::test]
async fn put_with_a_token_enrolls_the_key_under_its_label_and_closes_an_open_relay() {
    let dir = tempfile::tempdir().unwrap();
    let list = dir.path().join("enrolled.toml");
    let token = make_token(&list, "sign-up");
    let (relay, at) = relay(&list).await;
    let host = stranger();
    assert_eq!(put(at, &host.to_z32(), Some(&token), ME, "sam").await, 200);
    assert_eq!(enrolled(&list), vec![(host, "sam".to_string())]);
    let (_, json) = send(at, "GET /__tooob/relay.json HTTP/1.1", "").await;
    assert_eq!(json, r#"{"relay_access":"enrolled"}"#);
    // Again, relabelled; hex is the same key.
    assert_eq!(
        put(at, &host.to_string(), Some(&token), ME, "kim").await,
        200
    );
    assert_eq!(enrolled(&list), vec![(host, "kim".to_string())]);
    drop(relay);
}

#[tokio::test]
async fn delete_with_a_token_removes_the_key_and_disconnects_it() {
    let dir = tempfile::tempdir().unwrap();
    let list = dir.path().join("enrolled.toml");
    let token = make_token(&list, "sign-up");
    let (relay, at) = relay(&list).await;
    let (mut host, host_key) = client(&relay).await;
    let keep = stranger();
    for key in [host_key, keep] {
        assert_eq!(put(at, &key.to_z32(), Some(&token), ME, "x").await, 200);
    }
    assert_eq!(delete(at, &host_key.to_z32(), &token).await, 200);
    assert_eq!(enrolled(&list), vec![(keep, "x".to_string())]);
    let ended = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match host.next().await {
                None | Some(Err(_)) => return,
                Some(Ok(_)) => continue,
            }
        }
    })
    .await;
    assert!(ended.is_ok(), "the removed key's connection is still up");
    // Removing it again changes nothing.
    assert_eq!(delete(at, &host_key.to_z32(), &token).await, 404);
}

#[tokio::test]
async fn no_token_a_wrong_one_or_a_revoked_one_is_a_401_and_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let list = dir.path().join("enrolled.toml");
    let token = make_token(&list, "sign-up");
    let keep = stranger();
    List::edit(&list, |keys| {
        keys.enroll(keep, "keep");
        Ok(())
    })
    .unwrap();
    let (_relay, at) = relay(&list).await;
    let host = stranger().to_z32();
    assert_eq!(put(at, &host, None, ME, "sam").await, 401);
    assert_eq!(put(at, &host, Some("nope"), ME, "sam").await, 401);
    assert_eq!(delete(at, &keep.to_z32(), "nope").await, 401);
    List::edit(&list, |keys| Ok(keys.revoke_operator_token("sign-up"))).unwrap();
    // At once: no wait for the file to be seen.
    assert_eq!(put(at, &host, Some(&token), ME, "sam").await, 401);
    assert_eq!(delete(at, &keep.to_z32(), &token).await, 401);
    assert_eq!(enrolled(&list), vec![(keep, "keep".to_string())]);
}

#[tokio::test]
async fn a_bad_key_or_body_is_a_400() {
    let dir = tempfile::tempdir().unwrap();
    let list = dir.path().join("enrolled.toml");
    let token = make_token(&list, "sign-up");
    let (_relay, at) = relay(&list).await;
    assert_eq!(put(at, "nope", Some(&token), ME, "sam").await, 400);
    let no_label = format!(
        "PUT /__tooob/api/enrolled/{} HTTP/1.1\r\nAuthorization: Bearer {token}",
        stranger().to_z32()
    );
    assert_eq!(send(at, &no_label, "{}").await.0, 400);
    assert_eq!(send(at, &no_label, "not json").await.0, 400);
    // The scheme's name is matched in any case (RFC 7235).
    let lower = no_label.replace("Bearer", "bearer");
    assert_eq!(send(at, &lower, r#"{"label":"x"}"#).await.0, 200);
    List::edit(&list, |keys| {
        let all: Vec<_> = keys.entries().map(|(k, _)| k).collect();
        for k in all {
            keys.remove(k);
        }
        Ok(())
    })
    .unwrap();
    assert!(enrolled(&list).is_empty());
}

#[tokio::test]
async fn the_11th_failed_attempt_in_a_minute_from_one_address_is_a_429() {
    let dir = tempfile::tempdir().unwrap();
    let list = dir.path().join("enrolled.toml");
    let token = make_token(&list, "sign-up");
    let (_relay, at) = relay(&list).await;
    let host = stranger().to_z32();
    for _ in 0..10 {
        assert_eq!(put(at, &host, Some("nope"), ME, "x").await, 401);
    }
    assert_eq!(put(at, &host, Some("nope"), ME, "x").await, 429);
    // Even with the right token, until the minute passes; another address is let try.
    assert_eq!(put(at, &host, Some(&token), ME, "x").await, 429);
    assert_eq!(put(at, &host, Some(&token), "192.0.2.2", "x").await, 200);
}

#[test]
fn a_presented_operator_token_is_checked_and_named_by_its_label() {
    // #133's admin page logs in with one.
    let mut list = List::default();
    let token = list.make_operator_token("sign-up").unwrap();
    assert_eq!(list.verify_operator_token(&token), Some("sign-up"));
    assert_eq!(list.verify_operator_token("nope"), None);
    assert_eq!(list.verify_operator_token(""), None);
    assert_eq!(
        list.verify_operator_token(&format!(" {token} ")),
        Some("sign-up")
    );
    assert!(list.make_operator_token(" ").is_err(), "a label is needed");
    assert!(
        list.make_operator_token("sign-up").is_err(),
        "one per label"
    );
    assert!(list.revoke_operator_token("sign-up"));
    assert_eq!(list.verify_operator_token(&token), None);
}
