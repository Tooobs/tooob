# A Site's pages have real paths on their Key Address

A Site's page has the address `https://<host key>.<relay domain>/<path>`, its own path on its Key Address. The address bar follows the Site as the Visitor moves around it, and opening that address shows that page, so a reload, a bookmark or a shared link lands where it was. Before, the Site ran in the Bootstrap's iframe while the address stayed on the Key Address, so every reload landed on the front page.

We rejected putting the page in the fragment (`/#/<path>`), the way an Invite is. The Relay would never see a path, and neither it nor the service worker would change. But the addresses wouldn't look like the web's, and the Site's own `#anchors` would come after a second `#`. Real paths cost a little privacy instead: a browser that opens one before tooob's service worker is installed (a device's first visit, or after the browser cleared it) asks the Relay for that path, so the Relay Operator can see which page was opened, never what's on it. Once installed, the service worker serves the Bootstrap for every page load itself, fetched from the Relay as `/`, so the Relay sees only `/`, as before.

## Consequences

- A Relay serves the Bootstrap for any path that isn't iroh's or the Bundle's own: under `/__tooob/` and at `/tooob-sw.js`, a missing file is still a `404`. A Site's own pages at those paths stay unreachable.
- The address follows full page loads, `history.pushState` and the Site's own `#hash`. It never brings back an `#invite=`. A Site's hash that happens to hold `invite=` is tried as an Invite on reload, which is accepted: renaming it would break every Invite Link given out.
- Hard to undo: bookmarks and shared links will carry these paths.
