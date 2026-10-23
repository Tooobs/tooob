//! The Owner's operations over the running Host's local control socket (ADR 0010).
mod support;

use support::TestSite;
use tooob::Network;
use tooob::access::{InviteOptions, Redeemed};
use tooob::control::{Client, ControlError};
use tooob::host::{Host, HostConfig, Site};
use tooob::visit::{Visit, VisitError};

async fn start_host(site: &TestSite, dir: &tempfile::TempDir) -> Host {
    let config = HostConfig {
        site: Site::WebApp(site.addr),
        data_dir: dir.path().into(),
        network: Network::LocalOnly,
    };
    Host::start(config).await.unwrap()
}

fn new_visitor_key() -> [u8; 32] {
    let mut key = [0u8; 32];
    getrandom::fill(&mut key).unwrap();
    key
}

#[tokio::test]
async fn the_owner_can_invite_over_the_local_socket() {
    let site = TestSite::start().await;
    let dir = tempfile::tempdir().unwrap();
    let host = start_host(&site, &dir).await;

    let invite = Client::new(dir.path())
        .invite("mom", InviteOptions::default())
        .await
        .unwrap();
    let visit = Visit::connect(host.address(), new_visitor_key(), Network::LocalOnly)
        .await
        .unwrap();
    assert_eq!(visit.redeem(&invite.token).await, Ok(Redeemed::Admitted));
}

#[tokio::test]
async fn pairing_over_the_local_socket_makes_an_owner_device() {
    let site = TestSite::start().await;
    let dir = tempfile::tempdir().unwrap();
    let host = start_host(&site, &dir).await;
    let owner = Client::new(dir.path());

    let pairing = owner.pair("my phone").await.unwrap();
    let phone = Visit::connect(host.address(), new_visitor_key(), Network::LocalOnly)
        .await
        .unwrap();
    assert_eq!(phone.redeem(&pairing.token).await, Ok(Redeemed::Admitted));
    let visitors = owner.visitors().await.unwrap();
    assert_eq!(visitors.len(), 1);
    assert!(visitors[0].owner_device && visitors[0].label == "my phone");
}

#[tokio::test]
async fn the_owner_can_block_by_a_unique_key_prefix() {
    let site = TestSite::start().await;
    let dir = tempfile::tempdir().unwrap();
    let host = start_host(&site, &dir).await;
    let owner = Client::new(dir.path());
    let dave = Visit::connect(host.address(), new_visitor_key(), Network::LocalOnly)
        .await
        .unwrap();
    let mom = Visit::connect(host.address(), new_visitor_key(), Network::LocalOnly)
        .await
        .unwrap();
    dave.redeem(
        &owner
            .invite("dave", InviteOptions::default())
            .await
            .unwrap()
            .token,
    )
    .await
    .unwrap();
    mom.redeem(
        &owner
            .invite("mom", InviteOptions::default())
            .await
            .unwrap()
            .token,
    )
    .await
    .unwrap();
    let daves_key = owner
        .visitors()
        .await
        .unwrap()
        .into_iter()
        .find(|v| v.label == "dave")
        .unwrap()
        .key;

    owner.block(&daves_key[..8]).await.unwrap();
    assert_eq!(dave.get("/").await.unwrap_err(), VisitError::Denied);
    assert_eq!(mom.get("/").await.unwrap().status, 200);
    assert_eq!(
        owner.block("ffffffffffff").await,
        Err(ControlError::Refused("unknown_visitor".into()))
    );
    // An empty prefix names nobody; it must not quietly match the only Visitor.
    assert_eq!(
        owner.block("").await,
        Err(ControlError::Refused("key_required".into()))
    );
}

#[tokio::test]
async fn the_owner_can_approve_and_unblock_over_the_local_socket() {
    let site = TestSite::start().await;
    let dir = tempfile::tempdir().unwrap();
    let host = start_host(&site, &dir).await;
    let owner = Client::new(dir.path());
    let options = InviteOptions {
        first_visit_approval: true,
        ..Default::default()
    };
    let mom = Visit::connect(host.address(), new_visitor_key(), Network::LocalOnly)
        .await
        .unwrap();
    mom.redeem(&owner.invite("mom", options).await.unwrap().token)
        .await
        .unwrap();
    let key = owner.visitors().await.unwrap()[0].key.clone();

    owner.approve(&key[..8]).await.unwrap();
    assert_eq!(mom.get("/").await.unwrap().status, 200);
    owner.block(&key[..8]).await.unwrap();
    owner.unblock(&key[..8]).await.unwrap();
    assert_eq!(mom.get("/").await.unwrap().status, 200);
}

#[tokio::test]
async fn the_owner_can_list_and_revoke_invites_over_the_local_socket() {
    let site = TestSite::start().await;
    let dir = tempfile::tempdir().unwrap();
    let host = start_host(&site, &dir).await;
    let owner = Client::new(dir.path());
    let wrong_chat = owner
        .invite("dave", InviteOptions::default())
        .await
        .unwrap();
    owner.invite("mom", InviteOptions::default()).await.unwrap();
    assert_eq!(owner.invites().await.unwrap().len(), 2);

    owner.revoke(&wrong_chat.id).await.unwrap();
    let labels: Vec<_> = owner
        .invites()
        .await
        .unwrap()
        .into_iter()
        .map(|i| i.label)
        .collect();
    assert_eq!(labels, vec!["mom"]);
    let stranger = Visit::connect(host.address(), new_visitor_key(), Network::LocalOnly)
        .await
        .unwrap();
    assert_eq!(
        stranger.redeem(&wrong_chat.token).await,
        Err(VisitError::InviteRefused("unknown_invite".into()))
    );
}

#[tokio::test]
async fn only_the_owner_account_can_use_the_control_socket() {
    use std::os::unix::fs::PermissionsExt;
    let site = TestSite::start().await;
    let dir = tempfile::tempdir().unwrap();
    let _host = start_host(&site, &dir).await;
    let mode = std::fs::metadata(dir.path().join("control.sock"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
}

#[tokio::test]
async fn with_no_host_running_the_client_says_so() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        Client::new(dir.path()).visitors().await,
        Err(ControlError::NotRunning)
    );
}

#[tokio::test]
async fn the_host_key_and_data_directory_are_private_to_the_owner_account() {
    use std::os::unix::fs::PermissionsExt;
    let site = TestSite::start().await;
    let parent = tempfile::tempdir().unwrap();
    let data_dir = parent.path().join("tooob");
    let config = HostConfig {
        site: Site::WebApp(site.addr),
        data_dir: data_dir.clone(),
        network: Network::LocalOnly,
    };
    let _host = Host::start(config).await.unwrap();
    let mode = |p: std::path::PathBuf| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(
        mode(data_dir.join("host.key")),
        0o600,
        "the Host Key is the Host's identity"
    );
    assert_eq!(mode(data_dir.clone()), 0o700);
}

#[tokio::test]
async fn a_data_directory_with_a_long_path_still_gets_a_control_socket() {
    let site = TestSite::start().await;
    let parent = tempfile::tempdir().unwrap();
    // Unix socket paths are limited to ~104 bytes on macOS; real data directories can be deeper.
    let data_dir = parent
        .path()
        .join("a-rather-long-directory-name-for-a-home-server".repeat(3));
    let config = HostConfig {
        site: Site::WebApp(site.addr),
        data_dir: data_dir.clone(),
        network: Network::LocalOnly,
    };
    let _host = Host::start(config).await.unwrap();
    assert_eq!(Client::new(&data_dir).visitors().await, Ok(vec![]));
}

#[tokio::test]
async fn an_empty_key_never_acts_on_the_only_visitor() {
    let site = TestSite::start().await;
    let dir = tempfile::tempdir().unwrap();
    let host = start_host(&site, &dir).await;
    let owner = Client::new(dir.path());
    let mom = Visit::connect(host.address(), new_visitor_key(), Network::LocalOnly)
        .await
        .unwrap();
    mom.redeem(
        &owner
            .invite("mom", InviteOptions::default())
            .await
            .unwrap()
            .token,
    )
    .await
    .unwrap();

    assert_eq!(
        owner.block("").await,
        Err(ControlError::Refused("key_required".into()))
    );
    assert_eq!(mom.get("/").await.unwrap().status, 200);
}

#[tokio::test]
async fn a_second_host_cannot_take_over_a_running_hosts_data_directory() {
    let site = TestSite::start().await;
    let dir = tempfile::tempdir().unwrap();
    let first = start_host(&site, &dir).await;
    let config = HostConfig {
        site: Site::WebApp(site.addr),
        data_dir: dir.path().into(),
        network: Network::LocalOnly,
    };
    let second = Host::start(config).await;
    assert!(
        second.is_err(),
        "a second Host started on a live data directory"
    );

    // The Owner's commands still reach the Host that's actually serving.
    let owner = Client::new(dir.path());
    let mom = Visit::connect(first.address(), new_visitor_key(), Network::LocalOnly)
        .await
        .unwrap();
    mom.redeem(
        &owner
            .invite("mom", InviteOptions::default())
            .await
            .unwrap()
            .token,
    )
    .await
    .unwrap();
    assert_eq!(mom.get("/").await.unwrap().status, 200);
}

#[tokio::test]
async fn shutting_down_stops_the_control_socket_too() {
    let site = TestSite::start().await;
    let dir = tempfile::tempdir().unwrap();
    start_host(&site, &dir).await.shutdown().await;
    assert_eq!(
        Client::new(dir.path()).visitors().await,
        Err(ControlError::NotRunning)
    );
    let _restarted = start_host(&site, &dir).await; // and the directory is free for the next Host
}

#[tokio::test]
async fn the_cli_finds_the_host_however_the_data_directory_is_spelled() {
    let site = TestSite::start().await;
    let parent = tempfile::tempdir().unwrap();
    let long = parent
        .path()
        .join("a-rather-long-directory-name-for-a-home-server".repeat(3));
    let config = HostConfig {
        site: Site::WebApp(site.addr),
        data_dir: long.clone(),
        network: Network::LocalOnly,
    };
    let _host = Host::start(config).await.unwrap();

    let alias = parent.path().join("short"); // e.g. a relative path or a symlink: short when spelled
    std::os::unix::fs::symlink(&long, &alias).unwrap();
    assert_eq!(Client::new(&alias).visitors().await, Ok(vec![]));
}

#[tokio::test]
async fn a_long_path_hosts_socket_lives_in_a_private_directory_of_ours() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let site = TestSite::start().await;
    let parent = tempfile::tempdir().unwrap();
    let long = parent
        .path()
        .join("a-rather-long-directory-name-for-a-home-server".repeat(3));
    let config = HostConfig {
        site: Site::WebApp(site.addr),
        data_dir: long,
        network: Network::LocalOnly,
    };
    let _host = Host::start(config).await.unwrap();

    let me = std::fs::metadata(parent.path()).unwrap().uid();
    let private = std::env::temp_dir().join(format!("tooob-{me}"));
    let meta = std::fs::symlink_metadata(&private).unwrap();
    assert!(meta.is_dir() && meta.uid() == me);
    assert_eq!(
        meta.permissions().mode() & 0o777,
        0o700,
        "nobody else can reach or replace the socket"
    );
}

/// One raw request on the control socket, the way another client might send it.
async fn raw(dir: &std::path::Path, request: &str) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::UnixStream::connect(dir.join("control.sock"))
        .await
        .unwrap();
    stream
        .write_all(format!("{request}\n").as_bytes())
        .await
        .unwrap();
    let mut reply = String::new();
    stream.read_to_string(&mut reply).await.unwrap();
    reply
}

#[tokio::test]
async fn partial_or_malformed_requests_get_an_answer_not_a_hangup() {
    let site = TestSite::start().await;
    let dir = tempfile::tempdir().unwrap();
    let _host = start_host(&site, &dir).await;
    let partial = raw(dir.path(), r#"{"op":"invite","label":"mom"}"#).await;
    assert!(
        partial.contains(r#""ok""#),
        "missing options mean defaults: {partial}"
    );
    let malformed = raw(
        dir.path(),
        r#"{"op":"invite","label":"mom","options":"soon"}"#,
    )
    .await;
    assert!(malformed.contains(r#""error""#), "{malformed}");
}

#[tokio::test]
async fn an_existing_data_directory_keeps_its_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let site = TestSite::start().await;
    let dir = tempfile::tempdir().unwrap(); // already exists, like a NAS mount or a Docker volume
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    let _host = start_host(&site, &dir).await;
    let mode = std::fs::metadata(dir.path()).unwrap().permissions().mode() & 0o777;
    assert_eq!(
        mode, 0o755,
        "tooob warns about an open directory instead of changing it"
    );
    let key_mode = std::fs::metadata(dir.path().join("host.key"))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(key_mode, 0o600, "the Host Key itself is still private");
}
