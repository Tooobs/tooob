//! Each Visitor's cookie jar for the Site, kept on disk beside the Allowlist so a Host restart
//! doesn't log Visitors out of the Site (ADR 0005).
use anyhow::Result;
use reqwest::{Url, header::HeaderValue};
use std::{
    collections::HashMap,
    fs, io,
    os::unix::fs::DirBuilderExt,
    path::PathBuf,
    sync::{Arc, Mutex},
};

/// Every Visitor's jar: one owner-only file per Visitor Key.
#[derive(Debug)]
pub(crate) struct Jars {
    dir: PathBuf,
    /// The jars in use, by Visitor Key. Its lock also orders handing out a jar against a Block
    /// forgetting it.
    clients: Mutex<HashMap<String, SiteClient>>,
}

/// One Visitor's HTTP client for the Site, and the cookie jar it fills.
#[derive(Debug, Clone)]
pub(crate) struct SiteClient {
    pub(crate) http: reqwest::Client,
    pub(crate) jar: Arc<Jar>,
}

impl Jars {
    pub(crate) fn open(dir: impl Into<PathBuf>) -> io::Result<Jars> {
        let dir = dir.into();
        if !dir.exists() {
            fs::DirBuilder::new().mode(0o700).create(&dir)?;
        }
        Ok(Jars {
            dir,
            clients: Default::default(),
        })
    }

    /// The Visitor's own HTTP client, holding their Site cookies: `Set-Cookie` stops here, so
    /// HttpOnly session cookies never reach the browser (ADR 0005). None if `still_admitted` says
    /// they were blocked since their request was let in: their jar mustn't come back.
    pub(crate) fn client_for(
        &self,
        visitor_key: &str,
        still_admitted: impl FnOnce() -> bool,
    ) -> Result<Option<SiteClient>> {
        let mut clients = self.clients.lock().unwrap();
        if !still_admitted() {
            return Ok(None);
        }
        if let Some(client) = clients.get(visitor_key) {
            return Ok(Some(client.clone()));
        }
        let jar = Arc::new(Jar::load(self.path_for(visitor_key)));
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .cookie_provider(jar.clone())
            .build()?;
        let client = SiteClient { http, jar };
        clients.insert(visitor_key.to_string(), client.clone());
        Ok(Some(client))
    }

    /// Throw the Visitor's jar away, in memory and on disk: their Site logins go with a Block.
    pub(crate) fn forget(&self, visitor_key: &str) {
        let mut clients = self.clients.lock().unwrap();
        match clients.remove(visitor_key) {
            Some(client) => client.jar.forget(),
            None => remove(&self.path_for(visitor_key)),
        }
    }

    /// Visitor Keys are the hex public keys a hello proved, so they're safe as file names.
    fn path_for(&self, visitor_key: &str) -> PathBuf {
        self.dir.join(format!("{visitor_key}.json"))
    }
}

/// A cookie jar that saves itself after every change.
#[derive(Debug)]
pub(crate) struct Jar {
    path: PathBuf,
    /// None once forgotten: a reply already on its way can't bring the jar back.
    store: Mutex<Option<cookie_store::CookieStore>>,
}

impl Jar {
    /// The jar saved at `path`, or an empty one. A jar that won't load only costs a login.
    fn load(path: PathBuf) -> Jar {
        let store = match fs::File::open(&path) {
            Ok(f) => cookie_store::serde::json::load(io::BufReader::new(f)).unwrap_or_else(|e| {
                eprintln!("tooob: warning: starting {} afresh: {e}", path.display());
                Default::default()
            }),
            Err(_) => Default::default(),
        };
        Jar {
            path,
            store: Mutex::new(Some(store)),
        }
    }

    fn forget(&self) {
        let mut store = self.store.lock().unwrap();
        *store = None;
        remove(&self.path);
    }
}

/// Delete a saved jar, and the temp file a crash mid-save may have left beside it.
fn remove(path: &std::path::Path) {
    for path in [path.to_path_buf(), crate::temp_path(path)] {
        if let Err(e) = fs::remove_file(&path)
            && e.kind() != io::ErrorKind::NotFound
        {
            eprintln!("tooob: warning: couldn't remove {}: {e}", path.display());
        }
    }
}

impl reqwest::cookie::CookieStore for Jar {
    fn set_cookies(&self, headers: &mut dyn Iterator<Item = &HeaderValue>, url: &Url) {
        let cookies: Vec<_> = headers
            .filter_map(|v| v.to_str().ok())
            .filter_map(|v| cookie::Cookie::parse(v.to_string()).ok())
            .collect();
        if cookies.is_empty() {
            return;
        }
        let mut store = self.store.lock().unwrap();
        let Some(store) = store.as_mut() else { return };
        store.store_response_cookies(cookies.into_iter(), url);
        // Session cookies (no Expires) are kept too: a browser would drop them when it closes, but
        // a Host restart isn't the Visitor leaving.
        let mut saved = Vec::new();
        let result =
            cookie_store::serde::json::save_incl_expired_and_nonpersistent(store, &mut saved)
                .map_err(io::Error::other)
                .and_then(|()| crate::save_privately(&self.path, &saved));
        if let Err(e) = result {
            eprintln!("tooob: warning: couldn't save {}: {e}", self.path.display());
        }
    }

    fn cookies(&self, url: &Url) -> Option<HeaderValue> {
        let header = self
            .store
            .lock()
            .unwrap()
            .as_ref()?
            .get_request_values(url)
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("; ");
        if header.is_empty() {
            None
        } else {
            HeaderValue::from_str(&header).ok()
        }
    }
}
