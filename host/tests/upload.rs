//! Uploads through a real Host to a stand-in Site, over iroh on this machine (no Relay): a request
//! body streams from the Visitor to the Site as it arrives, so its size has no cap and the Host
//! never holds it whole. A file of its own, so this process's memory is these uploads' alone.
mod support;

use sha2::Digest;
use std::pin::Pin;
use std::task::{Context, Poll};
use support::{TestSite, hex};
use tokio::io::{AsyncRead, ReadBuf};
use tooob::Network;
use tooob::access::InviteOptions;
use tooob::host::{Host, HostConfig, Site};
use tooob::visit::Visit;

const MIB: u64 = 1 << 20;

/// `size` bytes of a fixed pseudo-random pattern, made as they're read: never all in memory. After
/// `break_at` bytes, if given, the read fails, as an upload does when the Visitor's tab goes away.
struct Pattern {
    left: u64,
    state: u64,
    break_at: Option<u64>,
}

impl Pattern {
    fn new(size: u64) -> Pattern {
        Pattern {
            left: size,
            state: 0x9e37_79b9_7f4a_7c15,
            break_at: None,
        }
    }

    /// The SHA-256 of the bytes this pattern gives, read off a fresh copy of it.
    fn sha256(size: u64) -> String {
        let mut p = Pattern::new(size);
        let mut hash = sha2::Sha256::new();
        let mut chunk = vec![0u8; 64 * 1024];
        while p.left > 0 {
            let n = p.fill(&mut chunk);
            hash.update(&chunk[..n]);
        }
        hex(&hash.finalize())
    }

    fn fill(&mut self, out: &mut [u8]) -> usize {
        // Whole words only, so the bytes don't depend on how they're read (sizes are multiples of 8).
        let n = (out.len() as u64).min(self.left) as usize & !7;
        for word in out[..n].chunks_mut(8) {
            // xorshift64
            self.state ^= self.state << 13;
            self.state ^= self.state >> 7;
            self.state ^= self.state << 17;
            word.copy_from_slice(&self.state.to_le_bytes());
        }
        self.left -= n as u64;
        n
    }
}

impl AsyncRead for Pattern {
    fn poll_read(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let this = self.get_mut();
        if let Some(at) = this.break_at
            && this.left <= at
        {
            return Poll::Ready(Err(std::io::Error::other("the tab went away")));
        }
        let n = this.fill(buf.initialize_unfilled());
        buf.advance(n);
        Poll::Ready(Ok(()))
    }
}

/// The most memory this process has held at once, in bytes.
fn peak_memory() -> u64 {
    let mut usage = unsafe { std::mem::zeroed::<libc::rusage>() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
    let max = usage.ru_maxrss as u64;
    if cfg!(target_os = "macos") {
        max // bytes there, KiB on Linux
    } else {
        max * 1024
    }
}

async fn admitted_visitor(site: &TestSite, dir: &tempfile::TempDir) -> (Host, Visit) {
    let host = Host::start(HostConfig {
        site: Site::WebApp(site.addr),
        data_dir: dir.path().into(),
        network: Network::LocalOnly,
    })
    .await
    .unwrap();
    let mut key = [0u8; 32];
    getrandom::fill(&mut key).unwrap();
    let visit = Visit::connect(host.address(), key, Network::LocalOnly)
        .await
        .unwrap();
    let invite = host
        .control()
        .invite("mom", InviteOptions::default())
        .unwrap();
    visit.redeem(&invite.token).await.unwrap();
    (host, visit)
}

#[tokio::test(flavor = "multi_thread")]
async fn an_upload_far_over_64_mib_reaches_the_site_intact_without_the_host_holding_it() {
    let size = 320 * MIB;
    let expected = Pattern::sha256(size);
    let site = TestSite::start().await;
    let dir = tempfile::tempdir().unwrap();
    let (_host, visit) = admitted_visitor(&site, &dir).await;
    let before = peak_memory();

    let reply = visit
        .upload("POST", "/sha256", &[], Pattern::new(size))
        .await
        .unwrap();

    assert_eq!(reply.status, 200);
    assert_eq!(
        String::from_utf8(reply.body).unwrap(),
        format!("{size} {expected}")
    );
    let grew = peak_memory().saturating_sub(before);
    assert!(
        grew < 32 * MIB,
        "memory grew {} MiB for a {} MiB upload",
        grew / MIB,
        size / MIB
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_upload_that_breaks_off_midway_never_reaches_the_site_as_whole() {
    let site = TestSite::start().await;
    let dir = tempfile::tempdir().unwrap();
    let (_host, visit) = admitted_visitor(&site, &dir).await;

    let mut body = Pattern::new(16 * MIB);
    body.break_at = Some(8 * MIB); // the tab closes halfway
    let reply = visit.upload("POST", "/sha256", &[], body).await;

    assert!(reply.is_err(), "a broken-off upload got {reply:?}");
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    assert_eq!(site.uploads(), 0, "the Site took half an upload as whole");
    assert_eq!(
        visit.get("/").await.unwrap().status,
        200,
        "and the Visitor carries on"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_site_that_answers_before_taking_an_upload_has_its_answer_reach_the_visitor() {
    let site = TestSite::start().await;
    let dir = tempfile::tempdir().unwrap();
    let (_host, visit) = admitted_visitor(&site, &dir).await;

    // The stand-in Site answers every other path without reading the body, as a Site refusing an
    // upload too big for it does.
    let reply = visit
        .upload("POST", "/elsewhere", &[], Pattern::new(64 * MIB))
        .await
        .unwrap();

    assert_eq!(reply.status, 200);
    assert_eq!(
        String::from_utf8(reply.body).unwrap(),
        "POST /elsewhere visitor=mom"
    );
}
