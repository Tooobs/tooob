//! A Shared Folder: the Host serves a folder, or a single file, itself, with no web server
//! alongside and no local port (ADR 0012). It browses like a web server's open directory. It's
//! read-only, unless its Owner turns on uploads: then its Visitors add files, add-only (ADR 0026).
use crate::host::Uploads;
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
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tower::ServiceExt;
use tower_http::services::ServeFile;

/// A Shared Folder as one request finds it: where it is, and whether it takes uploads.
pub(crate) struct Folder<'a> {
    pub root: &'a Path,
    pub uploads: Option<&'a Uploads>,
}

/// Who is asking, for the log of a finished upload (ADR 0025 as amended by ADR 0026).
pub(crate) struct Uploader<'a> {
    pub label: &'a str,
    pub key: &'a str,
}

/// The least an upload may leave free on the Owner's disk.
const FREE_FLOOR: u64 = 1 << 30;

/// How often, in bytes written, an upload whose size wasn't said checks the free space again.
const CHECK_FREE_EVERY: u64 = 1 << 20;

/// What the name of every partial upload starts with: listings never show one, and no upload may
/// be named so.
const PARTIAL: &str = ".tooob-partial";

impl Folder<'_> {
    /// Answer one Visitor request. Only GET and HEAD, and no WebSockets; PUT too, adding a file,
    /// where uploads are on (ADR 0026). Never POST, and no CORS preflight: another Site can't make
    /// a Visitor's browser upload.
    pub(crate) async fn serve(
        &self,
        head: &Value,
        rest: Vec<u8>,
        recv: impl AsyncRead + Unpin + Send,
        send: &mut (impl AsyncWrite + Unpin + Send),
        who: Uploader<'_>,
    ) -> Result<()> {
        let root = self.root;
        let method = head["method"].as_str().unwrap_or("GET");
        let uploads = self.uploads.filter(|_| root.is_dir());
        if let (Some(uploads), "PUT") = (uploads, method)
            && head["t"] != "ws"
        {
            let path = head["path"].as_str().unwrap_or("/");
            let path = path.split(['?', '#']).next().unwrap_or_default();
            return upload(root, uploads, path, head, rest, recv, send, who).await;
        }
        if head["t"] == "ws" || !matches!(method, "GET" | "HEAD") {
            let allow = if uploads.is_some() {
                "GET, HEAD, PUT"
            } else {
                "GET, HEAD"
            };
            let reply = json!({"status": 405, "headers": [["allow", allow]]});
            return wire::write_line(send, &reply).await;
        }
        // An uploaded page must never run script as the Site (ADR 0026): every file in a folder
        // taking uploads is sandboxed. Its listings are the Host's own, and run the Upload button.
        let sandbox = Sandbox(self.uploads.is_some());
        serve(root, uploads.is_some(), sandbox, method, head, send).await
    }
}

/// Whether every file served is sent with `Content-Security-Policy: sandbox`.
#[derive(Clone, Copy)]
struct Sandbox(bool);

/// Answer a GET or HEAD from the Shared Folder at `root`; its listings offer uploads when
/// `uploads` is set.
async fn serve(
    root: &Path,
    uploads: bool,
    sandbox: Sandbox,
    method: &str,
    head: &Value,
    send: &mut (impl AsyncWrite + Unpin + Send),
) -> Result<()> {
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
        return single_file(root, req, sandbox, send).await;
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
                Some(page) => {
                    let resp = ServeFile::new(page).oneshot(req).await?;
                    reply(send, sandbox, resp).await
                }
                None => match listing(root, &folder, &path, uploads) {
                    Ok(page) => reply(send, Sandbox(false), page).await,
                    Err(_) => status(send, 404).await, // unreadable: as for a file
                },
            }
        }
        Some(file) => {
            // The name it was asked for, which a symlink's target may not share.
            let last = path
                .trim_end_matches('/')
                .rsplit('/')
                .next()
                .unwrap_or_default();
            let name = percent_decode_str(last).decode_utf8_lossy().into_owned();
            serve_file(&file, &name, req, sandbox, send).await
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
    sandbox: Sandbox,
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
    serve_file(file, &name, req, sandbox, send).await
}

/// A file, shown in the browser, or saved as `name` when asked for with `?download`: what a
/// listing's Download link and the shell's Download button ask for (#99).
async fn serve_file(
    file: &Path,
    name: &str,
    req: http::Request<()>,
    sandbox: Sandbox,
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
    reply(send, sandbox, resp).await
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
    sandbox: Sandbox,
    mut resp: http::Response<B>,
) -> Result<()>
where
    B: http_body::Body<Data = bytes::Bytes> + Unpin,
    B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    if sandbox.0 {
        resp.headers_mut().insert(
            http::header::CONTENT_SECURITY_POLICY,
            http::HeaderValue::from_static("sandbox"),
        );
    }
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
/// Never a partial upload, hidden files shown or not (#29). With `uploads`, it has an Upload button
/// and takes files dropped onto it (ADR 0026).
fn listing(
    root: &Path,
    folder: &Path,
    path: &str,
    uploads: bool,
) -> Result<http::Response<Full<bytes::Bytes>>> {
    let mut entries: Vec<Entry> = fs::read_dir(folder)?
        .filter_map(|e| {
            let e = e.ok()?;
            if e.file_name()
                .as_encoded_bytes()
                .starts_with(PARTIAL.as_bytes())
            {
                return None;
            }
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
{upload}<table>
<tr><th>Name</th><th>Size</th><th>Modified (UTC)</th><th></th></tr>
{rows}</table>
",
        upload = if uploads { UPLOAD } else { "" },
    );
    http::Response::builder()
        .header("content-type", "text/html; charset=utf-8")
        .header("cache-control", "no-cache")
        .body(Full::from(html))
        .context("listing response")
}

/// A listing's upload controls (ADR 0026): an Upload button for one or more files, and the whole
/// page takes files dropped onto it. Each file is its own PUT into this folder, its bytes as the
/// body, showing its progress; the listing refreshes once they're done, saying the name each was
/// saved under. Folders aren't uploaded.
const UPLOAD: &str = r#"<style>
#upload { margin: 0 0 12px } #uploads { margin: 0 0 12px; padding-left: 20px }
#uploads .failed { color: #b00020 } body.drop { outline: 3px dashed #0a58ca; outline-offset: -8px }
@media (prefers-color-scheme: dark) { #uploads .failed { color: #ff8a80 } }
</style>
<p id="upload"><button type="button" id="pick">⬆ Upload</button> or drop files here
<input type="file" id="files" multiple hidden></p>
<ul id="uploads"></ul>
<script>
(() => {
  const list = document.getElementById('uploads');
  const input = document.getElementById('files');
  document.getElementById('pick').onclick = () => input.click();
  input.onchange = () => { send([...input.files]); input.value = ''; };
  addEventListener('dragover', (e) => { e.preventDefault(); document.body.classList.add('drop'); });
  addEventListener('dragleave', (e) => { if (!e.relatedTarget) document.body.classList.remove('drop'); });
  addEventListener('drop', (e) => {
    e.preventDefault();
    document.body.classList.remove('drop');
    const items = [...e.dataTransfer.items];
    const files = [];
    [...e.dataTransfer.files].forEach((file, i) => {
      if (items[i]?.webkitGetAsEntry?.()?.isDirectory) say(row(file.name), 'not uploaded: folders can’t be uploaded', 'failed');
      else files.push(file);
    });
    send(files);
  });
  function row(name) {
    const li = document.createElement('li');
    li.dataset.name = name;
    list.append(li);
    return li;
  }
  function say(li, text, state) {
    li.textContent = `${li.dataset.name}: ${text}`;
    if (state) li.className = state;
  }
  async function send(files) {
    if (!files.length) return;
    await Promise.all(files.map(one));
    const fresh = await fetch(location.pathname, { cache: 'no-store' }).then((r) => r.text());
    const table = new DOMParser().parseFromString(fresh, 'text/html').querySelector('table');
    if (table) document.querySelector('table').replaceWith(table);
  }
  async function one(file) {
    const li = row(file.name);
    say(li, '0%');
    const url = encodeURIComponent(file.name);
    let reply;
    try {
      // Read as it's sent, so its progress is what the Host has taken.
      const reader = file.stream().getReader();
      let sent = 0;
      const body = new ReadableStream({
        async pull(c) {
          const { done, value } = await reader.read();
          if (done) return c.close();
          sent += value.byteLength;
          say(li, `${Math.floor((100 * sent) / (file.size || 1))}%`);
          c.enqueue(value);
        },
      });
      reply = await fetch(url, { method: 'PUT', body, duplex: 'half' }).catch((e) => {
        if (sent) throw e;
        return fetch(url, { method: 'PUT', body: file }); // a browser that can't stream a body
      });
    } catch (e) {
      return say(li, 'not uploaded: the connection broke off', 'failed');
    }
    if (reply.status === 201) {
      const { saved } = await reply.json();
      return say(li, saved === file.name ? 'uploaded' : `uploaded, saved as ${saved}`, 'done');
    }
    const why = reply.status === 507 ? 'the disk is full' : (await reply.text()) || `refused (${reply.status})`;
    say(li, `not uploaded: ${why}`, 'failed');
  }
})();
</script>
"#;

/// Why a Visitor's upload can't have `name`, if it can't (ADR 0026).
fn refused_name(name: &str) -> Option<&'static str> {
    if name.contains(['/', '\\']) {
        return Some("a name can't contain / or \\");
    }
    if name.chars().any(char::is_control) {
        return Some("a name can't contain a control character");
    }
    if matches!(name, "" | "." | "..") {
        return Some("no name: empty, . and .. aren't names");
    }
    if name.len() > 255 {
        return Some("a name can be at most 255 bytes long");
    }
    if name.starts_with(PARTIAL) {
        return Some("names starting .tooob-partial are tooob's own");
    }
    if matches!(name, "index.html" | "index.htm") {
        return Some("index.html and index.htm would turn the folder into a website");
    }
    None
}

/// `name`, or its `n`th stand-in when it's taken, as browsers name downloads: `photo (1).jpg`.
fn numbered(name: &str, n: u32) -> String {
    if n == 0 {
        return name.to_string();
    }
    match name.rfind('.').filter(|&i| i > 0) {
        Some(i) => format!("{} ({n}){}", &name[..i], &name[i..]),
        None => format!("{name} ({n})"),
    }
}

/// A partial upload's file, deleted unless it's been renamed into place: an upload cut short or
/// abandoned leaves nothing behind.
struct Partial(PathBuf);

impl Drop for Partial {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// A reply with `text` as its body, for the page to show.
async fn refuse(send: &mut (impl AsyncWrite + Unpin + Send), code: u16, text: &str) -> Result<()> {
    let headers = json!([["content-type", "text/plain; charset=utf-8"]]);
    wire::write_line(send, &json!({"status": code, "headers": headers})).await?;
    send.write_all(text.as_bytes()).await?;
    Ok(())
}

const DISK_FULL: &str = "the disk is full";

/// Add one file, a PUT to `path`, into the folder it names: whole or not at all, never over
/// another file, and never so the disk is left with less than 1 GiB free (ADR 0026).
#[allow(clippy::too_many_arguments)]
async fn upload(
    root: &Path,
    uploads: &Uploads,
    path: &str,
    head: &Value,
    rest: Vec<u8>,
    mut recv: impl AsyncRead + Unpin + Send,
    send: &mut (impl AsyncWrite + Unpin + Send),
    who: Uploader<'_>,
) -> Result<()> {
    let (parent, last) = path.rsplit_once('/').unwrap_or(("", path));
    let Ok(name) = percent_decode_str(last).decode_utf8() else {
        return refuse(send, 400, "a name must be UTF-8").await;
    };
    if let Some(why) = refused_name(&name) {
        return refuse(send, 400, why).await;
    }
    let Some(folder) = locate(root, &format!("{parent}/")).filter(|f| f.is_dir()) else {
        return refuse(send, 404, "no such folder").await;
    };
    if inside(root, &folder.join("index.html")).is_some_and(|p| p.is_file()) {
        return refuse(
            send,
            403,
            "this folder is a website: nothing can be uploaded to it",
        )
        .await;
    }
    let length: Option<u64> = wire::request_headers(head)
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.trim().parse().ok());
    let leaves_enough = |more: u64| {
        uploads
            .free(&folder)
            .is_ok_and(|free| free.saturating_sub(more) >= FREE_FLOOR)
    };
    if !leaves_enough(length.unwrap_or(0)) {
        return refuse(send, 507, DISK_FULL).await;
    }

    let mut id = [0u8; 8];
    getrandom::fill(&mut id)?;
    let id: String = id.iter().map(|b| format!("{b:02x}")).collect();
    let partial = Partial(folder.join(format!("{PARTIAL}-{id}")));
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&partial.0)
        .await?;
    let mut written = 0u64;
    let mut checked = 0u64;
    let mut chunk = rest;
    loop {
        if !chunk.is_empty() {
            file.write_all(&chunk).await?;
            written += chunk.len() as u64;
            chunk.clear();
            // A size that wasn't said, or was said wrong: the disk is checked as it fills.
            if written - checked >= CHECK_FREE_EVERY {
                checked = written;
                if !leaves_enough(0) {
                    return refuse(send, 507, DISK_FULL).await;
                }
            }
        }
        chunk.reserve(wire::BODY_CHUNK);
        // A stream that breaks off is an error here: the partial file goes with `partial`.
        if recv.read_buf(&mut chunk).await? == 0 {
            break;
        }
    }
    file.flush().await?;
    file.sync_all().await?;
    drop(file);
    if !leaves_enough(0) {
        return refuse(send, 507, DISK_FULL).await;
    }
    if length.is_some_and(|l| l != written) {
        return refuse(send, 400, "the upload ended short").await;
    }
    // A hard link never replaces what's there, even when two uploads of one name land at once.
    let saved = (0..)
        .map(|n| numbered(&name, n))
        .find_map(
            |saved| match fs::hard_link(&partial.0, folder.join(&saved)) {
                Ok(()) => Some(Ok(saved)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => None,
                Err(e) => Some(Err(e)),
            },
        )
        .context("no name left")??;
    drop(partial);

    let shown = folder
        .join(&saved)
        .strip_prefix(root.canonicalize()?)
        .map(|p| format!("/{}", p.display()))
        .unwrap_or_else(|_| saved.clone());
    let fields = crate::fields! {
        "label" => who.label,
        "key" => crate::visit_log::short(who.key),
        "path" => shown.as_str(),
        "size" => written,
    };
    crate::log::event(
        "uploaded",
        fields,
        &format!(
            "{} ({}) uploaded {shown} · {}",
            who.label,
            crate::visit_log::short(who.key),
            crate::amount(written)
        ),
    );
    let location = format!("{parent}/{}", utf8_percent_encode(&saved, SEGMENT));
    let headers = json!([["content-type", "application/json"], ["location", location]]);
    wire::write_line(send, &json!({"status": 201, "headers": headers})).await?;
    send.write_all(json!({"saved": saved}).to_string().as_bytes())
        .await?;
    Ok(())
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
