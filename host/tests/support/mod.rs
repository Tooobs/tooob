//! A stand-in Site: a real HTTP server on 127.0.0.1 that counts requests and echoes what reached it.
#![allow(dead_code)] // shared by several test files; each uses part of it
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
}

impl TestSite {
    pub async fn start() -> TestSite {
        let requests = Arc::new(AtomicUsize::new(0));
        let counter = requests.clone();
        let hang_counter = requests.clone();
        let uploads = Arc::new(AtomicUsize::new(0));
        let upload_counter = uploads.clone();
        let app = Router::new()
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
            // A Site that has stopped answering: takes the request and never replies.
            .route(
                "/hang",
                any(move || {
                    hang_counter.fetch_add(1, Ordering::SeqCst);
                    std::future::pending::<&'static str>()
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
        }
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
