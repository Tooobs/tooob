//! Shared Folders: a Host serving a folder (or a single file) itself, to Visitors over iroh on this
//! machine (no Relay). It browses like a web server's open directory (ADR 0012).
use std::path::Path;
use tooob::Network;
use tooob::access::InviteOptions;
use tooob::host::{Host, HostConfig, Site, Uploads};
use tooob::visit::{Response, Visit};

/// A Visitor let in to a Host sharing a folder or file; the Host lives as long as this does.
struct SharedFolderVisit {
    _host: Host,
    visit: Visit,
    _data: tempfile::TempDir,
}

/// A Host sharing `path`, and a Visitor it let in.
async fn share(path: &Path) -> SharedFolderVisit {
    share_with(path, None).await
}

/// A Host sharing `path` with uploads on (ADR 0026), and a Visitor it let in.
async fn share_uploads(path: &Path) -> SharedFolderVisit {
    share_with(path, Some(Uploads::default())).await
}

async fn share_with(path: &Path, uploads: Option<Uploads>) -> SharedFolderVisit {
    let data = tempfile::tempdir().unwrap();
    let config = HostConfig {
        site: Site::SharedFolder(path.into()),
        data_dir: data.path().into(),
        network: Network::LocalOnly,
    };
    let host = Host::start_with(config, uploads).await.unwrap();
    let mut key = [0u8; 32];
    getrandom::fill(&mut key).unwrap();
    let visit = Visit::connect(host.address(), key, Network::LocalOnly)
        .await
        .unwrap();
    let invite = host
        .control()
        .invite("friend", InviteOptions::default())
        .unwrap();
    visit.redeem(&invite.token).await.unwrap();
    SharedFolderVisit {
        _host: host,
        visit,
        _data: data,
    }
}

fn header<'a>(page: &'a Response, name: &str) -> Option<&'a str> {
    page.headers
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
}

fn write(path: impl AsRef<Path>, contents: impl AsRef<[u8]>) {
    let path = path.as_ref();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

#[tokio::test]
async fn a_visitor_downloads_a_file_from_a_shared_folder() {
    let folder = tempfile::tempdir().unwrap();
    write(folder.path().join("notes.txt"), "hello friends\n");
    let shared = share(folder.path()).await;

    let page = shared.visit.get("/notes.txt").await.unwrap();
    assert_eq!(page.status, 200);
    assert_eq!(page.body, b"hello friends\n");
    assert!(
        header(&page, "content-type")
            .unwrap()
            .starts_with("text/plain")
    );
}

/// Every link on a listing page, in order.
fn hrefs(page: &Response) -> Vec<String> {
    String::from_utf8_lossy(&page.body)
        .split("href=\"")
        .skip(1)
        .map(|s| s[..s.find('"').unwrap()].to_string())
        .collect()
}

/// The links on a listing page that open what they name, in order.
fn links(page: &Response) -> Vec<String> {
    hrefs(page)
        .into_iter()
        .filter(|l| !l.ends_with("?download"))
        .collect()
}

#[tokio::test]
async fn a_shared_folder_lists_folders_first_with_sizes_dates_and_dotfiles() {
    let folder = tempfile::tempdir().unwrap();
    write(folder.path().join("notes.txt"), "hello friends\n");
    write(folder.path().join("film.mp4"), vec![0u8; 1_500_000]);
    write(folder.path().join(".DS_Store"), "x");
    write(folder.path().join("Holiday photos/beach.png"), "png");
    let when = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_792_665_000); // 2026-10-22 10:30 UTC
    std::fs::File::options()
        .write(true)
        .open(folder.path().join("notes.txt"))
        .unwrap()
        .set_modified(when)
        .unwrap();
    let shared = share(folder.path()).await;

    let page = shared.visit.get("/").await.unwrap();
    assert_eq!(page.status, 200);
    assert!(
        header(&page, "content-type")
            .unwrap()
            .starts_with("text/html")
    );
    assert_eq!(
        links(&page),
        ["Holiday%20photos/", ".DS_Store", "film.mp4", "notes.txt"],
        "folders first, then by name; no way up from the top"
    );
    let html = String::from_utf8_lossy(&page.body);
    assert!(html.contains("14 B"), "{html}");
    assert!(html.contains("1.5 MB"), "{html}");
    assert!(html.contains("2026-10-22 10:30"), "{html}");
}

#[tokio::test]
async fn a_subfolder_lists_its_files_and_links_back_up() {
    let folder = tempfile::tempdir().unwrap();
    write(folder.path().join("Holiday photos/beach.png"), "png");
    let shared = share(folder.path()).await;

    let page = shared.visit.get("/Holiday%20photos/").await.unwrap();
    assert_eq!(page.status, 200);
    assert_eq!(links(&page), ["../", "beach.png"]);
    assert!(String::from_utf8_lossy(&page.body).contains("Holiday photos"));
}

#[tokio::test]
async fn a_folder_without_its_trailing_slash_redirects_to_it() {
    let folder = tempfile::tempdir().unwrap();
    write(folder.path().join("Holiday photos/beach.png"), "png");
    let shared = share(folder.path()).await;

    let page = shared
        .visit
        .get("/Holiday%20photos?sort=name")
        .await
        .unwrap();
    assert_eq!(page.status, 301);
    assert_eq!(
        header(&page, "location"),
        Some("/Holiday%20photos/?sort=name"),
        "so its listing's relative links resolve inside it"
    );
}

#[tokio::test]
async fn a_folder_with_index_html_is_served_as_a_website() {
    let folder = tempfile::tempdir().unwrap();
    write(
        folder.path().join("blog/index.html"),
        "<link rel=stylesheet href=style.css><h1>Our trip</h1>",
    );
    write(folder.path().join("blog/style.css"), "h1 { color: teal }");
    let shared = share(folder.path()).await;

    let page = shared.visit.get("/blog/").await.unwrap();
    assert_eq!(page.status, 200);
    assert!(
        header(&page, "content-type")
            .unwrap()
            .starts_with("text/html")
    );
    assert_eq!(
        String::from_utf8_lossy(&page.body),
        "<link rel=stylesheet href=style.css><h1>Our trip</h1>"
    );
    let css = shared.visit.get("/blog/style.css").await.unwrap();
    assert!(
        header(&css, "content-type")
            .unwrap()
            .starts_with("text/css")
    );
}

#[tokio::test]
async fn a_range_request_gets_just_those_bytes_so_videos_seek() {
    let folder = tempfile::tempdir().unwrap();
    let film: Vec<u8> = (0..3_000_000u32).map(|i| (i % 251) as u8).collect();
    write(folder.path().join("film.mp4"), &film);
    let shared = share(folder.path()).await;

    let part = shared
        .visit
        .request("GET", "/film.mp4", &[("Range", "bytes=1000-1009")], b"")
        .await
        .unwrap();
    assert_eq!(part.status, 206);
    assert_eq!(part.body, film[1000..1010]);
    assert_eq!(
        header(&part, "content-range"),
        Some("bytes 1000-1009/3000000")
    );
    assert!(
        header(&part, "content-type")
            .unwrap()
            .starts_with("video/mp4")
    );
}

#[tokio::test]
async fn nothing_outside_the_shared_folder_can_be_reached() {
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("shared");
    write(root.join("notes.txt"), "hello friends\n");
    write(parent.path().join("private/secret.txt"), "the PIN is 1234");
    std::os::unix::fs::symlink(
        parent.path().join("private/secret.txt"),
        root.join("escape.txt"),
    )
    .unwrap();
    std::os::unix::fs::symlink(parent.path().join("private"), root.join("elsewhere")).unwrap();
    std::os::unix::fs::symlink(root.join("notes.txt"), root.join("also notes.txt")).unwrap();
    let shared = share(&root).await;

    for path in [
        "/../private/secret.txt",
        "/%2e%2e/private/secret.txt",
        "/..%2fprivate%2fsecret.txt",
        "/escape.txt",
        "/elsewhere/",
        "/elsewhere/secret.txt",
    ] {
        let page = shared.visit.get(path).await.unwrap();
        assert_eq!(page.status, 404, "{path}");
        assert!(
            !String::from_utf8_lossy(&page.body).contains("1234"),
            "{path}"
        );
    }
    let listing = shared.visit.get("/").await.unwrap();
    assert_eq!(
        links(&listing),
        ["also%20notes.txt", "notes.txt"],
        "links that lead out aren't listed"
    );
    let inside = shared.visit.get("/also%20notes.txt").await.unwrap();
    assert_eq!(
        inside.body, b"hello friends\n",
        "a link within the folder works"
    );
}

#[tokio::test]
async fn a_single_shared_file_is_served_as_itself_under_its_own_name() {
    let folder = tempfile::tempdir().unwrap();
    write(folder.path().join("Trip photos.zip"), "PK zip");
    write(folder.path().join("other.txt"), "not shared");
    let shared = share(&folder.path().join("Trip photos.zip")).await;

    let start = shared.visit.get("/").await.unwrap();
    assert_eq!(start.status, 302);
    assert_eq!(
        header(&start, "location"),
        Some("/Trip%20photos.zip"),
        "so a download is saved under the file's name"
    );
    let file = shared.visit.get("/Trip%20photos.zip").await.unwrap();
    assert_eq!(file.status, 200);
    assert_eq!(file.body, b"PK zip");
    assert!(
        header(&file, "content-type")
            .unwrap()
            .starts_with("application/zip")
    );
    for path in ["/other.txt", "/Trip%20photos.zip/x", "/../other.txt"] {
        assert_eq!(shared.visit.get(path).await.unwrap().status, 404, "{path}");
    }
}

#[tokio::test]
async fn a_symlinked_index_html_that_leads_out_is_not_served() {
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("shared");
    write(root.join("blog/post.txt"), "hi");
    write(parent.path().join("secret.html"), "the PIN is 1234");
    std::os::unix::fs::symlink(
        parent.path().join("secret.html"),
        root.join("blog/index.html"),
    )
    .unwrap();
    let shared = share(&root).await;

    let page = shared.visit.get("/blog/").await.unwrap();
    assert!(!String::from_utf8_lossy(&page.body).contains("1234"));
}

#[tokio::test]
async fn a_folder_redirect_never_leads_off_the_site() {
    let folder = tempfile::tempdir().unwrap();
    write(folder.path().join("github.com/readme.txt"), "hi");
    let shared = share(folder.path()).await;

    let page = shared.visit.get("//github.com").await.unwrap();
    assert_eq!(page.status, 301);
    assert_eq!(
        header(&page, "location"),
        Some("/github.com/"),
        "`//github.com/` would send the browser to github.com"
    );
}

#[tokio::test]
async fn a_shared_folder_is_read_only_and_has_no_websockets() {
    let folder = tempfile::tempdir().unwrap();
    write(folder.path().join("notes.txt"), "hello friends\n");
    write(folder.path().join("sub/a.txt"), "a");
    let shared = share(folder.path()).await;

    for (method, path) in [
        ("POST", "/"),
        ("PUT", "/notes.txt"),
        ("DELETE", "/notes.txt"),
        ("POST", "/sub"),
    ] {
        let page = shared
            .visit
            .request(method, path, &[], b"overwritten")
            .await
            .unwrap();
        assert_eq!(page.status, 405, "{method} {path}");
    }
    assert_eq!(
        std::fs::read(folder.path().join("notes.txt")).unwrap(),
        b"hello friends\n"
    );
    assert!(shared.visit.websocket("/notes.txt").await.is_err());
}

#[tokio::test]
async fn an_unreadable_folder_gets_an_answer_not_a_dropped_stream() {
    use std::os::unix::fs::PermissionsExt;
    let folder = tempfile::tempdir().unwrap();
    write(folder.path().join("locked/a.txt"), "a");
    let locked = folder.path().join("locked");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    let shared = share(folder.path()).await;

    let page = shared.visit.get("/locked/").await;
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(page.unwrap().status, 404);
}

#[tokio::test]
async fn a_file_asked_for_with_download_is_an_attachment_under_its_own_name() {
    let folder = tempfile::tempdir().unwrap();
    write(folder.path().join("notes.md"), "# hello\n");
    write(folder.path().join("Trip/día uno.html"), "<p>day one</p>");
    let shared = share(folder.path()).await;

    let notes = shared.visit.get("/notes.md?download").await.unwrap();
    assert_eq!(notes.status, 200);
    assert_eq!(notes.body, b"# hello\n");
    assert_eq!(
        header(&notes, "content-disposition"),
        Some("attachment; filename=\"notes.md\"; filename*=UTF-8''notes.md")
    );
    let page = shared
        .visit
        .get("/Trip/d%C3%ADa%20uno.html?download")
        .await
        .unwrap();
    assert_eq!(page.body, b"<p>day one</p>");
    assert_eq!(
        header(&page, "content-disposition"),
        Some("attachment; filename=\"d_a uno.html\"; filename*=UTF-8''d%C3%ADa%20uno.html"),
        "an ASCII stand-in for old browsers, the real name for the rest"
    );
    let slashed = shared.visit.get("/notes.md/?download").await.unwrap();
    assert_eq!(
        header(&slashed, "content-disposition"),
        Some("attachment; filename=\"notes.md\"; filename*=UTF-8''notes.md"),
        "a trailing slash never leaves it without a name"
    );
    let shown = shared.visit.get("/notes.md").await.unwrap();
    assert_eq!(
        header(&shown, "content-disposition"),
        None,
        "shown, not saved"
    );
}

#[tokio::test]
async fn a_single_shared_file_asked_for_with_download_is_an_attachment() {
    let folder = tempfile::tempdir().unwrap();
    write(folder.path().join("notes.md"), "# hello\n");
    let shared = share(&folder.path().join("notes.md")).await;

    let notes = shared.visit.get("/notes.md?download").await.unwrap();
    assert_eq!(notes.body, b"# hello\n");
    assert_eq!(
        header(&notes, "content-disposition"),
        Some("attachment; filename=\"notes.md\"; filename*=UTF-8''notes.md")
    );
}

#[tokio::test]
async fn a_listing_has_a_download_link_for_each_file_but_not_for_folders() {
    let folder = tempfile::tempdir().unwrap();
    write(folder.path().join("notes.md"), "# hello\n");
    write(folder.path().join("page.html"), "<p>hi</p>");
    write(folder.path().join("Holiday photos/beach.png"), "png");
    let shared = share(folder.path()).await;

    let page = shared.visit.get("/").await.unwrap();
    assert_eq!(
        hrefs(&page),
        [
            "Holiday%20photos/",
            "notes.md",
            "notes.md?download",
            "page.html",
            "page.html?download"
        ]
    );
    let html = String::from_utf8_lossy(&page.body);
    assert!(
        html.contains("<a href=\"notes.md?download\">⬇ Download</a>"),
        "{html}"
    );
}

#[tokio::test]
async fn a_visitor_who_is_in_is_told_the_site_is_a_shared_folder() {
    let folder = tempfile::tempdir().unwrap();
    let shared = share(folder.path()).await;
    // So the shell offers to save a file it shows (#99): a web app's addresses have no ?download.
    assert_eq!(
        shared.visit.site_kind().await.unwrap().as_deref(),
        Some("shared_folder")
    );
}

/// What a PUT of `body` to `path` got.
async fn put(shared: &SharedFolderVisit, path: &str, body: &[u8]) -> Response {
    shared.visit.request("PUT", path, &[], body).await.unwrap()
}

/// What's in `folder`, by name, sorted.
fn names(folder: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(folder)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[tokio::test]
async fn with_uploads_on_a_visitor_adds_files_to_the_folder_they_are_looking_at() {
    let folder = tempfile::tempdir().unwrap();
    write(folder.path().join("Holiday photos/beach.png"), "png");
    let shared = share_uploads(folder.path()).await;

    let listing = shared.visit.get("/Holiday%20photos/").await.unwrap();
    assert!(String::from_utf8_lossy(&listing.body).contains("Upload"));
    let sent = put(&shared, "/Holiday%20photos/caf%C3%A9.jpg", b"jpeg bytes").await;
    assert_eq!(sent.status, 201);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&sent.body).unwrap()["saved"],
        "café.jpg"
    );
    let dotfile = put(&shared, "/.notes", b"fine").await;
    assert_eq!(dotfile.status, 201, "other dotfiles are fine");

    assert_eq!(
        std::fs::read(folder.path().join("Holiday photos/café.jpg")).unwrap(),
        b"jpeg bytes"
    );
    let listing = shared.visit.get("/Holiday%20photos/").await.unwrap();
    assert_eq!(links(&listing), ["../", "beach.png", "caf%C3%A9.jpg"]);
}

#[tokio::test]
async fn without_uploads_a_listing_has_no_upload_button_and_a_put_writes_nothing() {
    let folder = tempfile::tempdir().unwrap();
    write(folder.path().join("notes.txt"), "hi");
    let shared = share(folder.path()).await;

    let listing = shared.visit.get("/").await.unwrap();
    assert!(!String::from_utf8_lossy(&listing.body).contains("Upload"));
    assert_eq!(put(&shared, "/new.txt", b"x").await.status, 405);
    assert_eq!(names(folder.path()), ["notes.txt"]);
}

#[tokio::test]
async fn an_upload_whose_name_is_taken_gets_a_number_and_nothing_is_overwritten() {
    let folder = tempfile::tempdir().unwrap();
    write(folder.path().join("photo.jpg"), "original");
    write(folder.path().join("README"), "readme");
    let shared = share_uploads(folder.path()).await;

    let saved = |r: Response| {
        serde_json::from_slice::<serde_json::Value>(&r.body).unwrap()["saved"]
            .as_str()
            .unwrap()
            .to_string()
    };
    assert_eq!(
        saved(put(&shared, "/photo.jpg", b"one").await),
        "photo (1).jpg"
    );
    assert_eq!(
        saved(put(&shared, "/photo.jpg", b"two").await),
        "photo (2).jpg"
    );
    assert_eq!(saved(put(&shared, "/README", b"r").await), "README (1)");
    // Two at once, of one name: each lands under a name of its own.
    let (a, b) = tokio::join!(
        put(&shared, "/same.txt", b"a"),
        put(&shared, "/same.txt", b"b")
    );
    let mut both = [saved(a), saved(b)];
    both.sort();
    assert_eq!(both, ["same (1).txt", "same.txt"]);

    assert_eq!(
        std::fs::read(folder.path().join("photo.jpg")).unwrap(),
        b"original"
    );
    assert_eq!(
        std::fs::read(folder.path().join("photo (1).jpg")).unwrap(),
        b"one"
    );
    assert_eq!(
        std::fs::read(folder.path().join("photo (2).jpg")).unwrap(),
        b"two"
    );
}

#[tokio::test]
async fn each_refused_name_is_refused_with_its_reason_and_nothing_is_written() {
    let folder = tempfile::tempdir().unwrap();
    write(folder.path().join("sub/a.txt"), "a");
    let shared = share_uploads(folder.path()).await;
    let long = "x".repeat(256);

    for (path, why) in [
        ("/a%2Fb.txt", "/"),
        ("/a%5Cb.txt", "\\"),
        ("/a%00b.txt", "control"),
        ("/a%0Ab.txt", "control"),
        ("/", "no name"),
        ("/sub/", "no name"),
        ("/.", "no name"),
        ("/sub/..", "no name"),
        (&format!("/{long}"), "255"),
        ("/.tooob-partial-x", ".tooob-partial"),
        ("/index.html", "index.html"),
        ("/sub/index.htm", "index.htm"),
    ] {
        let reply = put(&shared, path, b"x").await;
        assert_eq!(reply.status, 400, "{path}");
        let said = String::from_utf8_lossy(&reply.body);
        assert!(said.contains(why), "{path}: {said}");
    }
    assert_eq!(names(folder.path()), ["sub"]);
    assert_eq!(names(&folder.path().join("sub")), ["a.txt"]);
}

#[tokio::test]
async fn no_upload_lands_in_a_folder_serving_index_html_or_outside_the_share() {
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("shared");
    write(root.join("blog/index.html"), "<h1>hi</h1>");
    write(parent.path().join("private/secret.txt"), "pin");
    std::os::unix::fs::symlink(parent.path().join("private"), root.join("elsewhere")).unwrap();
    let shared = share_uploads(&root).await;

    let blog = put(&shared, "/blog/new.txt", b"x").await;
    assert_eq!(blog.status, 403);
    let listing = shared.visit.get("/blog/").await.unwrap();
    assert!(!String::from_utf8_lossy(&listing.body).contains("Upload"));
    for path in [
        "/../new.txt",
        "/%2e%2e/new.txt",
        "/elsewhere/new.txt",
        "/nope/new.txt",
    ] {
        assert_eq!(put(&shared, path, b"x").await.status, 404, "{path}");
    }
    assert_eq!(names(&root.join("blog")), ["index.html"]);
    assert_eq!(names(&parent.path().join("private")), ["secret.txt"]);
    assert_eq!(names(parent.path()), ["private", "shared"]);
}

#[tokio::test]
async fn a_post_upload_and_a_cors_preflight_are_refused() {
    let folder = tempfile::tempdir().unwrap();
    let shared = share_uploads(folder.path()).await;

    for method in ["POST", "OPTIONS"] {
        let reply = shared
            .visit
            .request(
                method,
                "/new.txt",
                &[
                    ("Origin", "https://other.example"),
                    ("Access-Control-Request-Method", "PUT"),
                ],
                b"x",
            )
            .await
            .unwrap();
        assert_eq!(reply.status, 405, "{method}");
        assert_eq!(header(&reply, "access-control-allow-origin"), None);
        assert_eq!(header(&reply, "access-control-allow-methods"), None);
    }
    assert!(names(folder.path()).is_empty());
}

#[tokio::test]
async fn a_partial_upload_never_shows_in_a_listing() {
    let folder = tempfile::tempdir().unwrap();
    write(folder.path().join(".tooob-partial-abc"), "half");
    write(folder.path().join("notes.txt"), "hi");
    let shared = share_uploads(folder.path()).await;

    let listing = shared.visit.get("/").await.unwrap();
    assert_eq!(links(&listing), ["notes.txt"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_upload_cut_short_leaves_no_file_and_no_partial_file() {
    let folder = tempfile::tempdir().unwrap();
    let shared = share_uploads(folder.path()).await;

    // Half a body, then the stream breaks off, as when the Visitor's page closes.
    struct Breaks(usize);
    impl tokio::io::AsyncRead for Breaks {
        fn poll_read(
            mut self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
            buf: &mut tokio::io::ReadBuf<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            if self.0 == 0 {
                return std::task::Poll::Ready(Err(std::io::Error::other("tab closed")));
            }
            let n = self.0.min(buf.remaining()).min(64 * 1024);
            buf.put_slice(&vec![7u8; n]);
            self.0 -= n;
            std::task::Poll::Ready(Ok(()))
        }
    }
    let reply = shared
        .visit
        .upload("PUT", "/big.bin", &[], Breaks(4 << 20))
        .await;
    assert!(reply.is_err(), "{reply:?}");
    let started = std::time::Instant::now();
    while !names(folder.path()).is_empty() && started.elapsed().as_secs() < 5 {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(
        names(folder.path()).is_empty(),
        "{:?}",
        names(folder.path())
    );
}

#[tokio::test]
async fn an_upload_that_would_leave_under_1_gib_free_is_refused_with_507() {
    const GIB: u64 = 1 << 30;
    let folder = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let config = |root: &Path| HostConfig {
        site: Site::SharedFolder(root.into()),
        data_dir: data.path().into(),
        network: Network::LocalOnly,
    };
    // A disk with 1 GiB and 100 bytes free: a 200-byte upload whose size is known is refused at
    // the start, and one whose size isn't said is refused once it's written past the floor.
    let uploads = Uploads::with_free_space(|folder: &Path| {
        let written: u64 = std::fs::read_dir(folder)
            .map(|d| {
                d.flatten()
                    .filter_map(|e| e.metadata().ok())
                    .map(|m| m.len())
                    .sum()
            })
            .unwrap_or(0);
        Ok((GIB + 100).saturating_sub(written))
    });
    let host = Host::start_with(config(folder.path()), Some(uploads))
        .await
        .unwrap();
    let mut key = [0u8; 32];
    getrandom::fill(&mut key).unwrap();
    let visit = Visit::connect(host.address(), key, Network::LocalOnly)
        .await
        .unwrap();
    let invite = host
        .control()
        .invite("friend", InviteOptions::default())
        .unwrap();
    visit.redeem(&invite.token).await.unwrap();

    let body = vec![1u8; 200];
    let known = visit
        .request("PUT", "/known.bin", &[("Content-Length", "200")], &body)
        .await
        .unwrap();
    assert_eq!(known.status, 507);
    assert!(String::from_utf8_lossy(&known.body).contains("full"));
    let big = vec![1u8; 4 << 20];
    let unknown = visit
        .request("PUT", "/unknown.bin", &[], &big)
        .await
        .unwrap();
    assert_eq!(unknown.status, 507);
    assert!(
        names(folder.path()).is_empty(),
        "{:?}",
        names(folder.path())
    );
    let small = visit
        .request("PUT", "/small.txt", &[], b"ok")
        .await
        .unwrap();
    assert_eq!(small.status, 201, "what fits still lands");
}

#[tokio::test]
async fn with_uploads_on_every_file_is_served_sandboxed() {
    let folder = tempfile::tempdir().unwrap();
    write(folder.path().join("page.html"), "<script>alert(1)</script>");
    write(folder.path().join("blog/index.html"), "<h1>hi</h1>");
    let shared = share_uploads(folder.path()).await;

    for path in ["/page.html", "/page.html?download", "/blog/"] {
        let page = shared.visit.get(path).await.unwrap();
        assert_eq!(page.status, 200, "{path}");
        assert_eq!(
            header(&page, "content-security-policy"),
            Some("sandbox"),
            "{path}"
        );
    }
    let plain = share(folder.path()).await;
    let page = plain.visit.get("/page.html").await.unwrap();
    assert_eq!(
        header(&page, "content-security-policy"),
        None,
        "only with uploads on"
    );
}
