//! A Relay forwards a packet only if its sender or its receiver is an Enrolled Key (ADR 0030), and
//! everything while it has none: through real relay clients, against a running `tooob-relay`.
use std::{path::Path, time::Duration};

use iroh_base::{EndpointId, RelayUrl, SecretKey};
use iroh_relay::{
    client::{Client, ClientBuilder},
    protos::relay::{ClientToRelayMsg, Datagrams, RelayToClientMsg},
    tls::{CaTlsConfig, default_provider},
};
use n0_future::{SinkExt, StreamExt};
use tooob_relay::{List, Relay};

/// A `tooob-relay` on this machine whose Enrolled Keys are in `list`.
async fn relay(list: &Path) -> Relay {
    Relay::spawn_dev(([127, 0, 0, 1], 0), list.to_path_buf())
        .await
        .unwrap()
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

/// Whether `to` gets a packet `from` sends it, trying for about a second.
async fn reaches(from: &mut Client, to: &mut Client, to_key: EndpointId) -> bool {
    for _ in 0..5 {
        from.send(ClientToRelayMsg::Datagrams {
            dst_endpoint_id: to_key,
            datagrams: Datagrams::from("hello"),
        })
        .await
        .unwrap();
        loop {
            match tokio::time::timeout(Duration::from_millis(200), to.next()).await {
                Err(_) => break,
                Ok(Some(Ok(RelayToClientMsg::Datagrams { .. }))) => return true,
                Ok(Some(Ok(_))) => continue,
                Ok(other) => panic!("relay client ended: {other:?}"),
            }
        }
    }
    false
}

fn enrolled(dir: &Path, keys: &[EndpointId]) -> std::path::PathBuf {
    let path = dir.join("enrolled.toml");
    let mut list = List::default();
    for key in keys {
        list.enroll(*key, "test");
    }
    list.save(&path).unwrap();
    path
}

#[tokio::test]
async fn strangers_get_nothing_to_each_other_once_a_key_is_enrolled() {
    let dir = tempfile::tempdir().unwrap();
    let host = SecretKey::from_bytes(&rand::random()).public();
    let relay = relay(&enrolled(dir.path(), &[host])).await;
    let (mut a, _) = client(&relay).await;
    let (mut b, b_key) = client(&relay).await;
    assert!(!reaches(&mut a, &mut b, b_key).await);
}

#[tokio::test]
async fn a_stranger_reaches_an_enrolled_key_and_gets_its_replies() {
    let dir = tempfile::tempdir().unwrap();
    let relay_list = dir.path().join("enrolled.toml");
    let relay = relay(&relay_list).await;
    let (mut visitor, visitor_key) = client(&relay).await;
    let (mut host, host_key) = client(&relay).await;
    enrolled(dir.path(), &[host_key]);
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(reaches(&mut visitor, &mut host, host_key).await);
    assert!(reaches(&mut host, &mut visitor, visitor_key).await);
}

#[tokio::test]
async fn with_no_enrolled_keys_everything_is_forwarded() {
    let dir = tempfile::tempdir().unwrap();
    let relay = relay(&dir.path().join("enrolled.toml")).await;
    let (mut a, _) = client(&relay).await;
    let (mut b, b_key) = client(&relay).await;
    assert!(reaches(&mut a, &mut b, b_key).await);
}

#[tokio::test]
async fn the_first_key_enrolled_closes_a_running_relay_within_a_second() {
    let dir = tempfile::tempdir().unwrap();
    let relay = relay(&dir.path().join("enrolled.toml")).await;
    let (mut a, _) = client(&relay).await;
    let (mut b, b_key) = client(&relay).await;
    enrolled(
        dir.path(),
        &[SecretKey::from_bytes(&rand::random()).public()],
    );
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(!reaches(&mut a, &mut b, b_key).await);
}

#[tokio::test]
async fn a_removed_key_is_disconnected_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let list = dir.path().join("enrolled.toml");
    let relay = relay(&list).await;
    let (mut host, host_key) = client(&relay).await;
    let keep = SecretKey::from_bytes(&rand::random()).public();
    enrolled(dir.path(), &[host_key, keep]);
    tokio::time::sleep(Duration::from_millis(600)).await;
    enrolled(dir.path(), &[keep]);
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
}
