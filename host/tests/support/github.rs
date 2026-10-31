//! A stand-in for GitHub's Releases of tooob: its Releases API and its downloads, on this machine,
//! which a test's `tooob` reaches through `TOOOB_RELEASES_URL` (`Home::releases_at`). Each Release's
//! `tooob-installer.sh` is a stand-in too: it records that it ran, in `<HOME>/installed`, and puts
//! a stand-in `tooob` at `~/.local/bin/tooob` that says it's that Release and otherwise runs this
//! build, noting each run in `<HOME>/ran-<release>`.
use axum::{
    Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use std::sync::{Arc, Mutex};

/// One Release, as the stand-in publishes it.
#[derive(Clone)]
struct Published {
    release: String,
    /// What its notes say it speaks; None for notes that don't say (a test Release's).
    protocol_version: Option<String>,
}

#[derive(Clone, Default)]
struct Shared {
    releases: Arc<Mutex<Vec<Published>>>,
    /// Every path asked for, in order.
    asked: Arc<Mutex<Vec<String>>>,
    /// Whether its API refuses everyone, as GitHub's does past its rate limit.
    refusing: Arc<Mutex<bool>>,
}

/// The stand-in GitHub; stopped when dropped.
pub struct TestGitHub {
    pub url: String,
    shared: Shared,
    _stop: tokio::sync::oneshot::Sender<()>,
}

impl TestGitHub {
    /// GitHub with no Releases yet.
    pub fn start() -> TestGitHub {
        let shared = Shared::default();
        let app = Router::new()
            .route("/repos/Tooobs/tooob/releases", get(list))
            .route("/repos/Tooobs/tooob/releases/tags/{tag}", get(one))
            .route(
                "/Tooobs/tooob/releases/download/{tag}/{file}",
                get(download),
            )
            .with_state(shared.clone());
        let (port_tx, port_rx) = std::sync::mpsc::channel();
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async {
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                port_tx.send(listener.local_addr().unwrap().port()).unwrap();
                tokio::select! {
                    _ = axum::serve(listener, app) => {}
                    _ = stopped => {}
                }
            });
        });
        let port = port_rx.recv().unwrap();
        TestGitHub {
            url: format!("http://127.0.0.1:{port}/"),
            shared,
            _stop: stop,
        }
    }

    /// Publish `release` (`0.2.0`, or `0.2.0-rc.1` for a test Release), its notes saying it speaks
    /// `protocol_version`, as release.sh's do, or saying nothing of it (None).
    pub fn publish(&self, release: &str, protocol_version: Option<&str>) {
        self.shared.releases.lock().unwrap().push(Published {
            release: release.to_string(),
            protocol_version: protocol_version.map(String::from),
        });
    }

    /// From now on, its API answers 403 Forbidden, as GitHub's does past its rate limit.
    pub fn refuse(&self) {
        *self.shared.refusing.lock().unwrap() = true;
    }

    /// Every path asked for so far, in order.
    pub fn asked(&self) -> Vec<String> {
        self.shared.asked.lock().unwrap().clone()
    }

    /// How many times the Releases were looked up (listed).
    pub fn lookups(&self) -> usize {
        self.asked()
            .iter()
            .filter(|p| {
                p.starts_with("/repos/Tooobs/tooob/releases?")
                    || *p == "/repos/Tooobs/tooob/releases"
            })
            .count()
    }
}

/// A Release as GitHub's API has it: only the fields tooob reads, and a few it doesn't.
fn json(p: &Published) -> serde_json::Value {
    let tag = format!("v{}", p.release);
    let body = match &p.protocol_version {
        Some(v) => format!(
            "## Release Notes\n\nProtocol Version: `{v}`. Tested with iroh-relay 1.3.0.\n\n### Added\n- things\n"
        ),
        None => "## Release Notes\n\n### Added\n- things\n".to_string(),
    };
    serde_json::json!({
        "tag_name": tag,
        "name": p.release,
        "draft": false,
        "prerelease": p.release.contains('-'),
        "html_url": format!("https://github.com/Tooobs/tooob/releases/tag/{tag}"),
        "body": body,
    })
}

fn note(shared: &Shared, path: String) {
    shared.asked.lock().unwrap().push(path);
}

async fn list(State(shared): State<Shared>, uri: axum::http::Uri) -> Response {
    note(&shared, uri.to_string());
    if *shared.refusing.lock().unwrap() {
        return (StatusCode::FORBIDDEN, r#"{"message": "rate limited"}"#).into_response();
    }
    // Newest first, as GitHub lists them, with a draft nobody should see.
    let mut all: Vec<serde_json::Value> =
        shared.releases.lock().unwrap().iter().map(json).collect();
    all.reverse();
    all.push(serde_json::json!({
        "tag_name": "v9.9.9", "draft": true, "prerelease": false,
        "html_url": "https://github.com/Tooobs/tooob/releases/tag/untagged", "body": "",
    }));
    axum::Json(all).into_response()
}

fn find(shared: &Shared, tag: &str) -> Option<Published> {
    let release = tag.strip_prefix('v')?;
    shared
        .releases
        .lock()
        .unwrap()
        .iter()
        .find(|p| p.release == release)
        .cloned()
}

async fn one(State(shared): State<Shared>, Path(tag): Path<String>) -> Response {
    note(&shared, format!("/repos/Tooobs/tooob/releases/tags/{tag}"));
    match find(&shared, &tag) {
        Some(p) => axum::Json(json(&p)).into_response(),
        None => (StatusCode::NOT_FOUND, r#"{"message": "Not Found"}"#).into_response(),
    }
}

async fn download(
    State(shared): State<Shared>,
    Path((tag, file)): Path<(String, String)>,
) -> Response {
    note(
        &shared,
        format!("/Tooobs/tooob/releases/download/{tag}/{file}"),
    );
    match (find(&shared, &tag), file.as_str()) {
        (Some(p), "tooob-installer.sh") => installer(&p).into_response(),
        _ => (StatusCode::NOT_FOUND, "Not Found").into_response(),
    }
}

/// A stand-in for a Release's `tooob-installer.sh`: what dist's does, in short (it moves the new
/// `tooob` into `~/.local/bin`, replacing whatever is there), recording that it ran.
fn installer(p: &Published) -> String {
    let release = &p.release;
    let speaks = p
        .protocol_version
        .as_deref()
        .unwrap_or(tooob::PROTOCOL_VERSION);
    let build = env!("CARGO_BIN_EXE_tooob");
    format!(
        r#"#!/bin/sh
PATH=/bin:/usr/bin
set -eu
echo "{release}" >> "$HOME/installed"
echo "installing to $HOME/.local/bin"
mkdir -p "$HOME/.local/bin"
cat > "$HOME/.local/bin/tooob.new" <<'TOOOB'
#!/bin/sh
if [ "$*" = --version ]; then echo "tooob {release} ({speaks})"; exit 0; fi
echo "$*" >> "$HOME/ran-{release}"
exec '{build}' "$@"
TOOOB
chmod +x "$HOME/.local/bin/tooob.new"
mv "$HOME/.local/bin/tooob.new" "$HOME/.local/bin/tooob"
echo "tooob installed"
"#
    )
}
