//! Shared Folders: a Host serving a folder (or a single file) itself, to Visitors over iroh on this
//! machine (no Relay). It browses like a web server's open directory (ADR 0012).
use std::path::Path;
use tooob::Network;
use tooob::access::InviteOptions;
use tooob::host::{Host, HostConfig, Site};
use tooob::visit::{Response, Visit};

/// A Visitor let in to a Host sharing a folder or file; the Host lives as long as this does.
struct SharedFolderVisit {
    _host: Host,
    visit: Visit,
    _data: tempfile::TempDir,
}

/// A Host sharing `path`, and a Visitor it let in.
async fn share(path: &Path) -> SharedFolderVisit {
    let data = tempfile::tempdir().unwrap();
    let host = Host::start(HostConfig {
        site: Site::SharedFolder(path.into()),
        data_dir: data.path().into(),
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

/// The links on a listing page, in order.
fn links(page: &Response) -> Vec<String> {
    String::from_utf8_lossy(&page.body)
        .split("href=\"")
        .skip(1)
        .map(|s| s[..s.find('"').unwrap()].to_string())
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
