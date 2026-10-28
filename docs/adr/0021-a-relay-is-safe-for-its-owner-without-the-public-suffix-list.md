# A Relay is safe for its Owner without the Public Suffix List

Amends ADR 0008 and ADR 0020, which put every Relay's domain on the Public Suffix List. Getting listed takes weeks, through volunteers who may say no, and months more to reach the browsers people run, so it can't be on the way to running a Relay. Each Key Address is already an origin of its own (ADR 0020): storage, caches, service worker and Visitor Key are walled off without the list. What the list adds is making each Key Address a *site* of its own. Without it, tooob closes the gaps it can itself:

- **Cookies set for the whole Relay domain.** The script injected into Site pages drops `Domain=` from any cookie a Site sets, through `document.cookie` or the Cookie Store, so it stays with that Site. The shell deletes any relay-wide cookie it finds when it starts. tooob itself uses no browser cookies: the Visitor Key is in IndexedDB, a Site's cookies are in its Host's jar (ADR 0005), and the tunnel never carries browser cookies.
- **Cookie bombs.** A Relay strips the `Cookie` header from every request. It never needs one, so a Site can't make other Hosts' pages fail to load by filling the domain with cookies.
- **Process isolation.** A Relay sends `Origin-Agent-Cluster: ?1` and `Cross-Origin-Opener-Policy: same-origin-allow-popups` with everything in the Bundle. We left out `Cross-Origin-Embedder-Policy`, which isolates further but breaks Sites that load fonts or scripts from elsewhere, and COOP `same-origin`, which breaks login popups.

## Consequences

- The injected script can't reach everywhere a Site's script can run: SVG documents, `about:blank` frames, a Site's own service worker, and top-level tabs once #77 lands. A relay-wide cookie set there lasts until the shell's next start.
- **A Relay shared with people who don't trust each other must still be on the list.** Password managers match logins by base domain, so without it one Host's page can be offered another Host's passwords, and only the list stops that. The docs say so; a Relay for its Owner alone doesn't need it.
