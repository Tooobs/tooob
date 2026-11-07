//! A stand-in Site: a real HTTP server on 127.0.0.1 that counts requests and echoes what reached it.
#![allow(dead_code)] // shared by several test files; each uses part of it
pub mod github;
pub mod home;
use axum::{
    Router,
    extract::{
        Request,
        ws::{Message, WebSocketUpgrade},
    },
    routing::any,
};
use std::{
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

pub struct TestSite {
    pub addr: SocketAddr,
    requests: Arc<AtomicUsize>,
    uploads: Arc<AtomicUsize>,
    refused: Arc<AtomicUsize>,
    polls: Arc<AtomicUsize>,
}

/// One open long-poll, counted while its handler lives.
struct OpenPoll(Arc<AtomicUsize>);

impl OpenPoll {
    fn new(open: Arc<AtomicUsize>) -> OpenPoll {
        open.fetch_add(1, Ordering::SeqCst);
        OpenPoll(open)
    }
}

impl Drop for OpenPoll {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl TestSite {
    pub async fn start() -> TestSite {
        let requests = Arc::new(AtomicUsize::new(0));
        let counter = requests.clone();
        let hang_counter = requests.clone();
        let uploads = Arc::new(AtomicUsize::new(0));
        let upload_counter = uploads.clone();
        let refused = Arc::new(AtomicUsize::new(0));
        let refused_counter = refused.clone();
        let polls = Arc::new(AtomicUsize::new(0));
        let poll_counter = polls.clone();
        let app = Router::new()
            // A Site refusing an upload too big for it: it answers at once, then reads and drops
            // whatever more comes, as Node's servers and nginx do.
            .route(
                "/refuse",
                any(move |req: Request| async move {
                    tokio::spawn(async move {
                        use futures::StreamExt;
                        let mut body = req.into_body().into_data_stream();
                        while let Some(Ok(chunk)) = body.next().await {
                            refused_counter.fetch_add(chunk.len(), Ordering::SeqCst);
                        }
                    });
                    (axum::http::StatusCode::PAYLOAD_TOO_LARGE, "too big")
                }),
            )
            // An upload like Immich's: read as it arrives, never held whole, answered with its size
            // and SHA-256. Only a body that arrived to its end counts as an upload.
            .route(
                "/sha256",
                any(move |req: Request| async move {
                    use futures::StreamExt;
                    use sha2::Digest;
                    let mut body = req.into_body().into_data_stream();
                    let (mut size, mut hash) = (0usize, sha2::Sha256::new());
                    while let Some(chunk) = body.next().await {
                        let Ok(chunk) = chunk else {
                            return "broken off".to_string();
                        };
                        size += chunk.len();
                        hash.update(&chunk);
                    }
                    upload_counter.fetch_add(1, Ordering::SeqCst);
                    format!("{size} {}", hex(&hash.finalize()))
                }),
            )
            // A download of `size` bytes, like a photo or a video.
            .route(
                "/bytes/{size}",
                any(
                    |axum::extract::Path(size): axum::extract::Path<usize>| async move {
                        vec![7u8; size]
                    },
                ),
            )
            // A Site that has stopped answering: takes the request and never replies.
            .route(
                "/hang",
                any(move || {
                    hang_counter.fetch_add(1, Ordering::SeqCst);
                    std::future::pending::<&'static str>()
                }),
            )
            // A long-poll: open until the request is dropped, and never a byte meanwhile.
            .route(
                "/poll",
                any(move || {
                    let open = OpenPoll::new(poll_counter.clone());
                    async move {
                        let _open = open;
                        std::future::pending::<&'static str>().await
                    }
                }),
            )
            // A live connection like Home Assistant's: echoes each text message, tagged with the Visitor.
            .route(
                "/ws",
                any(
                    |ws: WebSocketUpgrade, headers: axum::http::HeaderMap| async move {
                        let who = headers
                            .get("x-tooob-label")
                            .and_then(|v| v.to_str().ok())
                            .unwrap_or("-")
                            .to_string();
                        ws.on_upgrade(move |mut socket| async move {
                            while let Some(Ok(Message::Text(t))) = socket.recv().await {
                                let _ = socket.send(Message::text(format!("{t} from {who}"))).await;
                            }
                        })
                    },
                ),
            )
            // A cookie login like Immich's or Nextcloud's: an HttpOnly session cookie.
            .route(
                "/login",
                any(|| async {
                    (
                        [("set-cookie", "sid=s3cret; HttpOnly; Path=/")],
                        "logged in",
                    )
                }),
            )
            .route(
                "/me",
                any(|req: Request| async move {
                    let cookie = req
                        .headers()
                        .get("cookie")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("");
                    if cookie.contains("sid=s3cret") {
                        "alice"
                    } else {
                        "nobody"
                    }
                }),
            )
            .fallback(any(move |req: Request| {
                let counter = counter.clone();
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                    // Every value, so a second (injected) Label would show up.
                    let labels: Vec<_> = req
                        .headers()
                        .get_all("x-tooob-label")
                        .iter()
                        .filter_map(|v| v.to_str().ok())
                        .collect();
                    let visitor = if labels.is_empty() {
                        "-".to_string()
                    } else {
                        labels.join(",")
                    };
                    format!("{} {} visitor={visitor}", req.method(), req.uri().path())
                }
            }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        TestSite {
            addr,
            requests,
            uploads,
            refused,
            polls,
        }
    }

    /// Long-polls to `/poll` the Site still has open.
    pub fn open_polls(&self) -> usize {
        self.polls.load(Ordering::SeqCst)
    }

    /// How much of the uploads it refused reached `/refuse` anyway, in bytes.
    pub fn refused_bytes(&self) -> usize {
        self.refused.load(Ordering::SeqCst)
    }

    /// Bodies that reached `/sha256` whole, to their end.
    pub fn uploads(&self) -> usize {
        self.uploads.load(Ordering::SeqCst)
    }

    pub fn requests(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
