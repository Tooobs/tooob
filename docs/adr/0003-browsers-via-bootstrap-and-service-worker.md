# Browsers reach Hosts through a Bootstrap page, a service worker and an iframe

Visitors use ordinary browsers with nothing installed. The Bootstrap loads iroh compiled to WASM into a long-lived page, shows the Site in an iframe, and a service worker carries the iframe's requests to that page, which sends them to the Host. WebSockets can't be intercepted by service workers, so an injected shim replaces `WebSocket` in the Site. We rejected requiring a browser extension or native app (too much friction for the people Owners invite) and a public HTTPS URL per Site, as Cloudflare Tunnel and Funnel do (public ingress invites abuse and breaks "private").

## Consequences

- Visitors trust whoever serves the Bootstrap; self-hosting it removes tooob from that trust.
- Responses built by a service worker can't set cookies, hence the Host's cookie jar (ADR 0005).
- A Site's own service worker can't register; offline PWA features are lost.
