//! `/__tooob/relay.json`: whether a Relay is open or carries only Enrolled Keys' traffic (ADR 0030).
use std::{net::SocketAddr, path::Path, time::Duration};

use iroh_base::SecretKey;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tooob_relay::{List, Relay};

async fn relay_json(at: SocketAddr) -> String {
    ask(at, "GET /__tooob/relay.json").await
}

/// The body of what `request` ("METHOD PATH") gets, which must be a 200.
async fn ask(at: SocketAddr, request: &str) -> String {
    let mut stream = tokio::net::TcpStream::connect(at).await.unwrap();
    stream
        .write_all(format!("{request} HTTP/1.1\r\nHost: r\r\nConnection: close\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let mut reply = String::new();
    stream.read_to_string(&mut reply).await.unwrap();
    assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
    reply.split("\r\n\r\n").nth(1).unwrap().to_string()
}

fn enroll_one(list: &Path) {
    let mut keys = List::default();
    keys.enroll(SecretKey::from_bytes(&rand::random()).public(), "test");
    keys.save(list).unwrap();
}

#[tokio::test]
async fn relay_json_says_open_then_enrolled() {
    let dir = tempfile::tempdir().unwrap();
    let list = dir.path().join("enrolled.toml");
    let mut relay = Relay::spawn_dev(([127, 0, 0, 1], 0), list.clone())
        .await
        .unwrap();
    let at = relay.serve_status(([127, 0, 0, 1], 0)).await.unwrap();
    assert_eq!(relay_json(at).await, r#"{"relay_access":"open"}"#);
    enroll_one(&list);
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(relay_json(at).await, r#"{"relay_access":"enrolled"}"#);
}

#[tokio::test]
async fn enrolled_says_whether_a_key_is_enrolled_and_whether_the_relay_is_open() {
    // #130: a Host asks this, to tell its Owner when its Relay doesn't carry it yet.
    let dir = tempfile::tempdir().unwrap();
    let list = dir.path().join("enrolled.toml");
    let mut relay = Relay::spawn_dev(([127, 0, 0, 1], 0), list.clone())
        .await
        .unwrap();
    let at = relay.serve_status(([127, 0, 0, 1], 0)).await.unwrap();
    let host = SecretKey::from_bytes(&rand::random()).public();
    let z32 = format!("GET /__tooob/enrolled/{}", host.to_z32());
    let open = r#"{"enrolled":false,"relay_access":"open"}"#;
    assert_eq!(ask(at, &z32).await, open);
    enroll_one(&list);
    tokio::time::sleep(Duration::from_secs(1)).await;
    let not = r#"{"enrolled":false,"relay_access":"enrolled"}"#;
    assert_eq!(ask(at, &z32).await, not);
    let mut keys = List::load(&list).unwrap();
    keys.enroll(host, "home box");
    keys.save(&list).unwrap();
    tokio::time::sleep(Duration::from_secs(1)).await;
    let yes = r#"{"enrolled":true,"relay_access":"enrolled"}"#;
    assert_eq!(ask(at, &z32).await, yes);
    // Hex, as iroh writes a key, is the same key.
    assert_eq!(ask(at, &format!("GET /__tooob/enrolled/{host}")).await, yes);
}

#[tokio::test]
async fn enrolled_is_a_404_for_what_isnt_a_key() {
    let dir = tempfile::tempdir().unwrap();
    let mut relay = Relay::spawn_dev(([127, 0, 0, 1], 0), dir.path().join("enrolled.toml"))
        .await
        .unwrap();
    let at = relay.serve_status(([127, 0, 0, 1], 0)).await.unwrap();
    let mut stream = tokio::net::TcpStream::connect(at).await.unwrap();
    stream
        .write_all(b"GET /__tooob/enrolled/nope HTTP/1.1\r\nHost: r\r\n\r\n")
        .await
        .unwrap();
    let mut reply = String::new();
    stream.read_to_string(&mut reply).await.unwrap();
    assert!(reply.starts_with("HTTP/1.1 404"), "{reply}");
}

#[tokio::test]
async fn relay_json_answers_a_cache_busting_query_and_a_head() {
    let dir = tempfile::tempdir().unwrap();
    let mut relay = Relay::spawn_dev(([127, 0, 0, 1], 0), dir.path().join("enrolled.toml"))
        .await
        .unwrap();
    let at = relay.serve_status(([127, 0, 0, 1], 0)).await.unwrap();
    let busted = ask(at, "GET /__tooob/relay.json?t=1").await;
    assert_eq!(busted, r#"{"relay_access":"open"}"#);
    assert_eq!(ask(at, "HEAD /__tooob/relay.json").await, "");
}
