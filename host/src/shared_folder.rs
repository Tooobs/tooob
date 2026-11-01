//! A Shared Folder: the Host serves a folder, or a single file, itself, read-only, with no web
//! server alongside and no local port (ADR 0012). It browses like a web server's open directory.
use crate::wire;
use anyhow::{Context, Result};
use http_body_util::{BodyExt, Full};
use percent_encoding::{AsciiSet, CONTROLS, percent_decode_str, utf8_percent_encode};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Component, Path, PathBuf},
    time::UNIX_EPOCH,
};
use tokio::io::{AsyncWrite, AsyncWriteExt};
use tower::ServiceExt;
use tower_http::services::ServeFile;

/// Answer one Visitor request from the Shared Folder at `root`. Read-only: only GET and HEAD,
/// and no WebSockets.
pub(crate) async fn serve(
    root: &Path,
    head: &Value,
    send: &mut (impl AsyncWrite + Unpin + Send),
) -> Result<()> {
    let method = head["method"].as_str().unwrap_or("GET");
    if head["t"] == "ws" || !matches!(method, "GET" | "HEAD") {
        let reply = json!({"status": 405, "headers": [["allow", "GET, HEAD"]]});
        return wire::write_line(send, &reply).await;
    }
    let mut req = http::Request::builder()
        .method(method)
        .uri(head["path"].as_str().unwrap_or("/"));
    for (k, v) in wire::request_headers(head) {
        req = req.header(k, v);
    }
    let Ok(req) = req.body(()) else {
        return status(send, 400).await; // a header or path that isn't HTTP
    };
    let path = req.uri().path().to_string();
    if root.is_file() {
        return single_file(root, req, send).await;
    }
    match locate(root, &path) {
        // As web servers do: the page's relative links then resolve inside the folder. One leading
        // slash, always: `//github.com/` would send the browser to github.com.
        Some(target) if target.is_dir() && !path.ends_with('/') => {
            let query = req
                .uri()
                .query()
                .map(|q| format!("?{q}"))
                .unwrap_or_default();
            let location = format!("/{}/{query}", path.trim_start_matches('/'));
            let headers = json!([["location", location]]);
            wire::write_line(send, &json!({"status": 301, "headers": headers})).await
        }
        Some(folder) if folder.is_dir() => {
            match inside(root, &folder.join("index.html")).filter(|p| p.is_file()) {
                Some(page) => reply(send, ServeFile::new(page).oneshot(req).await?).await,
                None => match listing(root, &folder, &path) {
                    Ok(page) => reply(send, page).await,
                    Err(_) => status(send, 404).await, // unreadable: as for a file
                },
            }
        }
        Some(file) => {
            let name = percent_decode_str(path.rsplit('/').next().unwrap_or_default())
                .decode_utf8_lossy()
                .into_owned();
            serve_file(&file, &name, req, send).await
        }
        None => status(send, 404).await,
    }
}

async fn status(send: &mut (impl AsyncWrite + Unpin + Send), code: u16) -> Result<()> {
    wire::write_line(send, &json!({"status": code})).await
}

/// A Shared Folder of one file: it lives at its own name, which `/` redirects to, so a download is
/// saved under that name. Nothing else is there.
async fn single_file(
    file: &Path,
    req: http::Request<()>,
    send: &mut (impl AsyncWrite + Unpin + Send),
) -> Result<()> {
    let name = file.file_name().context("no file name")?.to_string_lossy();
    let path = req.uri().path();
    if path == "/" {
        let own = format!("/{}", utf8_percent_encode(&name, SEGMENT));
        let headers = json!([["location", own]]);
        return wire::write_line(send, &json!({"status": 302, "headers": headers})).await;
    }
    if percent_decode_str(path).decode_utf8().ok().as_deref() != Some(&format!("/{name}")) {
        return status(send, 404).await;
    }
    serve_file(file, &name, req, send).await
}

/// A file, shown in the browser, or saved as `name` when asked for with `?download`: what a
/// listing's Download link and the shell's Download button ask for (#99).
async fn serve_file(
    file: &Path,
    name: &str,
    req: http::Request<()>,
    send: &mut (impl AsyncWrite + Unpin + Send),
) -> Result<()> {
    let download = req.uri().query().is_some_and(|q| {
        q.split('&')
            .any(|p| p == "download" || p.starts_with("download="))
    });
    let mut resp = ServeFile::new(file).oneshot(req).await?;
    if download {
        let value = http::HeaderValue::from_str(&attachment(name))?;
        resp.headers_mut()
            .insert(http::header::CONTENT_DISPOSITION, value);
    }
    reply(send, resp).await
}

/// A Content-Disposition that saves the file as `name`: the real name (RFC 6266's `filename*`),
/// and a stand-in of plain ASCII for a browser that only reads `filename`.
fn attachment(name: &str) -> String {
    let plain: String = name
        .chars()
        .map(|c| match c {
            ' '..='~' if c != '"' && c != '\\' => c,
            _ => '_',
        })
        .collect();
    format!(
        "attachment; filename=\"{plain}\"; filename*=UTF-8''{}",
        utf8_percent_encode(name, ATTR_CHAR)
    )
}

/// RFC 8187's attr-char: everything else in a `filename*` is percent-encoded.
const ATTR_CHAR: &AsciiSet = &percent_encoding::NON_ALPHANUMERIC
    .remove(b'!')
    .remove(b'#')
    .remove(b'$')
    .remove(b'&')
    .remove(b'+')
    .remove(b'-')
    .remove(b'.')
    .remove(b'^')
    .remove(b'_')
    .remove(b'`')
    .remove(b'|')
    .remove(b'~');

/// The file or folder a request path names, percent-decoded, or None when it doesn't exist or
/// lies outside the Shared Folder (`..` in any spelling, or a symlink that leads out).
fn locate(root: &Path, path: &str) -> Option<PathBuf> {
    let path = percent_decode_str(path).decode_utf8().ok()?;
    let mut target = root.to_path_buf();
    for part in Path::new(path.trim_start_matches('/')).components() {
        match part {
            Component::Normal(name) => target.push(name),
            Component::CurDir => {}
            _ => return None,
        }
    }
    inside(root, &target)
}

/// Where `path` really is, following symlinks, if that's still within `root`.
fn inside(root: &Path, path: &Path) -> Option<PathBuf> {
    let real = path.canonicalize().ok()?;
    real.starts_with(root.canonicalize().ok()?).then_some(real)
}

/// Send a response as the proxy does: its JSON line, then the body.
async fn reply<B>(
    send: &mut (impl AsyncWrite + Unpin + Send),
    resp: http::Response<B>,
) -> Result<()>
where
    B: http_body::Body<Data = bytes::Bytes> + Unpin,
    B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    wire::write_line(
        send,
        &wire::response_head(resp.status(), resp.headers(), |_| true),
    )
    .await?;
    let mut body = resp.into_body();
    while let Some(frame) = body.frame().await {
        if let Ok(data) = frame.map_err(|e| anyhow::anyhow!(e.into()))?.into_data() {
            send.write_all(&data).await?;
        }
    }
    Ok(())
}

/// One path segment in a link: anything that would end it, start a query or fragment, look like
/// a scheme (`a:b`) or break out of the attribute is escaped.
const SEGMENT: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'&')
    .add(b'\'')
    .add(b'/')
    .add(b':')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'\\')
    .add(b'`')
    .add(b'{')
    .add(b'}');

struct Entry {
    name: String,
    folder: bool,
    size: u64,
    modified: Option<u64>,
}

/// A folder's open-directory page: folders first, then files, by name; size and date for each.
/// Only what a click could open: a symlink leading out of the Shared Folder isn't listed.
fn listing(root: &Path, folder: &Path, path: &str) -> Result<http::Response<Full<bytes::Bytes>>> {
    let mut entries: Vec<Entry> = fs::read_dir(folder)?
        .filter_map(|e| {
            let e = e.ok()?;
            let meta = fs::metadata(inside(root, &e.path())?).ok()?;
            Some(Entry {
                name: e.file_name().to_string_lossy().into_owned(),
                folder: meta.is_dir(),
                size: meta.len(),
                modified: meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_secs()),
            })
        })
        .collect();
    entries.sort_by_key(|e| (!e.folder, e.name.to_lowercase()));

    let title = format!(
        "Index of {}",
        escape(&percent_decode_str(path).decode_utf8_lossy())
    );
    let mut rows = String::new();
    if path != "/" {
        rows.push_str("<tr><td><a href=\"../\">../</a></td><td></td><td></td><td></td></tr>\n");
    }
    for e in &entries {
        let link = utf8_percent_encode(&e.name, SEGMENT);
        let slash = if e.folder { "/" } else { "" };
        // A file's own link shows it; its Download link saves it under its name, through the page,
        // whatever it is: the browser's "Save link as" can't reach the Host (#99). Not a `download`
        // link: Chromium fetches those around the service worker, as it does "Save link as".
        let (bytes, download) = if e.folder {
            (String::new(), String::new())
        } else {
            let download = format!("<a href=\"{link}?download\">⬇ Download</a>");
            (size(e.size), download)
        };
        rows.push_str(&format!(
            "<tr><td><a href=\"{link}{slash}\">{}{slash}</a></td><td class=n>{bytes}</td><td>{}</td><td>{download}</td></tr>\n",
            escape(&e.name),
            e.modified.map(date).unwrap_or_default(),
        ));
    }
    let html = format!(
        "<!doctype html>
<meta charset=\"utf-8\">
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">
<title>{title}</title>
<style>
body {{ font: 15px/1.5 system-ui, sans-serif; margin: 16px; color: #222; background: #fff }}
table {{ border-collapse: collapse; width: 100% }}
th, td {{ padding: 4px 8px; text-align: left; white-space: nowrap }}
td:first-child {{ white-space: normal; word-break: break-word; width: 100% }}
td.n {{ text-align: right }}
tr:nth-child(even) td {{ background: #f4f4f4 }}
a {{ color: #0a58ca; text-decoration: none }}
@media (prefers-color-scheme: dark) {{
  body {{ color: #ddd; background: #111 }}
  tr:nth-child(even) td {{ background: #1b1b1b }}
  a {{ color: #8ab4f8 }}
}}
</style>
<h1>{title}</h1>
<table>
<tr><th>Name</th><th>Size</th><th>Modified (UTC)</th><th></th></tr>
{rows}</table>
"
    );
    http::Response::builder()
        .header("content-type", "text/html; charset=utf-8")
        .header("cache-control", "no-cache")
        .body(Full::from(html))
        .context("listing response")
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// A size the way Finder shows it: 14 B, 1.5 MB.
fn size(bytes: u64) -> String {
    if bytes < 1000 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    for unit in ["KB", "MB", "GB", "TB"] {
        value /= 1000.0;
        if value < 1000.0 {
            return format!("{value:.1} {unit}");
        }
    }
    format!("{value:.1} PB")
}

/// Seconds since 1970 as `2026-10-22 10:30`, in UTC.
fn date(secs: u64) -> String {
    // Howard Hinnant's days-to-civil algorithm, for days since 1970-01-01.
    let days = (secs / 86_400) as i64 + 719_468;
    let era = days.div_euclid(146_097);
    let doe = days.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    let (hour, minute) = (secs % 86_400 / 3600, secs % 3600 / 60);
    format!("{year}-{month:02}-{day:02} {hour:02}:{minute:02}")
}
