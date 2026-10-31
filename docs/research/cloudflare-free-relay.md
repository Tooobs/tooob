# Can a Relay run on a free Cloudflare account?

Checked 2026-10-30. Every Cloudflare figure comes from developers.cloudflare.com or cloudflare.com as served that day, and the "last updated" date is the page's own. The iroh facts come from the source at the [`v1.3.0` tag](https://github.com/n0-computer/iroh/tree/v1.3.0) (commit `0072d7d`, the same commit the published `iroh-relay` 1.3.0 crate was built from) and from n0's own docs. Prices are in the provider's currency, excluding VAT unless a row says otherwise. Anything not confirmed on a primary page is in [Unverified](#unverified).

This feeds [#73](https://github.com/Tooobs/tooob/issues/73), the research ticket on what a Relay costs by provider and by what it carries. The [cost comparison](#cost-comparison-for-73) is written in that ticket's terms.

## What the Relay does today, and what Cloudflare would have to stand in for

- One container (`deploy/relay/image/`) runs Caddy and stock iroh-relay 1.3.0. Caddy gets a certificate for `<relay domain>` and `*.<relay domain>` through a DNS challenge, sends `/relay`, `/ping`, `/generate_204` and `/healthz` to iroh-relay on `127.0.0.1:3340` (plain HTTP, no `[tls]`, so no QUIC address discovery), and serves the Bootstrap Bundle for every other path. Every path that isn't a Bundle file gets the shell (`index.html`), except `/__tooob/*` and `/tooob-sw.js`, which stay 404. It strips `Cookie` and sends `Cache-Control: no-cache`, `Origin-Agent-Cluster: ?1` and `Cross-Origin-Opener-Policy: same-origin-allow-popups` (`front.caddy`, ADR 0021).
- **The Bootstrap connects to the Relay's own domain, not to its Key Address.** `shell.js` takes the first label of `location.hostname` as the Host Key and the rest as the Relay: `relay = https://<relay domain>/` (`bootstrap/www/__tooob/shell.js:80-82`). It passes that to `Tooob.connect`, which sets `RelayMode::custom([url])` and dials the Host Key at that relay URL (`bootstrap/web/src/lib.rs:32-45`). Each tab gets a fresh key (`SecretKey::generate()`).
- **The Host connects to the relay URL in its settings** (`Network::Relay(RelayUrl)` → `RelayMode::custom`, `host/src/lib.rs:34-46`). That is `https://<relay domain>/` today.
- So every relay WebSocket today, from Hosts and from browsers, goes to the apex `<relay domain>/relay`. Key Addresses (`<host key>.<relay domain>`) only ever serve the Bundle.
- The Direct Path uses Google's STUN server: `stun:stun.l.google.com:19302` (`host/src/direct.rs:43`, `bootstrap/www/__tooob/shell.js:458`). ADR 0004: no TURN server; a STUN server is still needed.
- The Bundle is 3.1 MB, nearly all of it `__tooob/pkg/web_bg.wasm` (3,122,831 B; 1,053,871 B gzipped, measured locally). `front.caddy` has no `encode` directive, so today's Relay serves it uncompressed.

## 1. Workers Free plan

| | Free | Source |
|---|---|---|
| Worker requests | 100,000 a day, reset at midnight UTC. Past the limit: Error 1027, or the Worker is bypassed if the route fails open | [Workers limits](https://developers.cloudflare.com/workers/platform/limits/) (updated Oct 29, 2026) |
| CPU per HTTP request | 10 ms (Paid: 30 s by default, up to 5 min) | same |
| Duration of an HTTP request | no limit | same |
| Memory | 128 MB per isolate | same |
| Subrequests | 50 per request | same |
| Request body | set by the zone's plan: 100 MB on Free | same |
| Workers per account | 100 | same |
| Static assets | 20,000 files per Worker version, 25 MiB per file | same |

**WebSockets.** Workers accept incoming WebSockets (`WebSocketPair`, `accept()`) and open outbound ones (`fetch()` with `Upgrade: websocket`, or `new WebSocket(url)`). Incoming messages are limited to 32 MiB; a bigger one closes the socket with 1009 ([Workers WebSockets](https://developers.cloudflare.com/workers/runtime-apis/websockets/), updated Oct 12, 2026). A WebSocket through a Worker is billed as one request, the upgrade: "WebSocket messages routed through a Worker do not count as requests" ([Workers pricing](https://developers.cloudflare.com/workers/platform/pricing/), updated Oct 2, 2026). The proxy itself supports WebSockets "on all Cloudflare plans", closes a connection after an unstated idle period with no data in either direction, and "may restart servers, which terminates WebSockets connections" ([Network: WebSockets](https://developers.cloudflare.com/network/websockets/), updated Aug 14, 2026). iroh sends a relay Ping every 15 s on a connection that has received nothing ([actor.rs:74](https://github.com/n0-computer/iroh/blob/v1.3.0/iroh/src/socket/transports/relay/actor.rs#L74)), so the idle cutoff shouldn't matter. Request headers are limited to 128 KB in total ([Connection limits](https://developers.cloudflare.com/fundamentals/reference/connection-limits/), updated Oct 22, 2026).

**Static assets.** "Requests to static assets are free and unlimited", and storing them costs nothing extra. A request that reaches the Worker script is billed as a Worker request. If the free request limit runs out, requests matching `run_worker_first` get a 429 instead of falling back to assets ([billing and limitations](https://developers.cloudflare.com/workers/static-assets/billing-and-limitations/), updated Apr 23, 2026). The Paid plan also has "no additional charges for data transfer (egress) or throughput (bandwidth)" ([Workers pricing](https://developers.cloudflare.com/workers/platform/pricing/)).

- **The shell at every path.** With `not_found_handling = "single-page-application"`, an unmatched request gets `/index.html` with 200. With no Worker script that holds for every request, so a missing `/__tooob/*` file would return the shell, not a 404 as Caddy gives today. With a script, navigation requests (`Sec-Fetch-Mode: navigate`) go to assets and the rest invoke the script, unless `run_worker_first` patterns decide instead ([SPA routing](https://developers.cloudflare.com/workers/static-assets/routing/single-page-application/), updated Oct 12, 2026).
- **`_headers`.** A `_headers` file in the assets directory sets response headers: up to 100 rules, 2,000 characters a line, one splat per URL, and hostname placeholders that stop at a dot (e.g. `https://:sub.example.com/*`). It does not apply to responses the Worker script makes ([Headers](https://developers.cloudflare.com/workers/static-assets/headers/), updated Sep 22, 2026). `Cache-Control`, `Origin-Agent-Cluster` and `Cross-Origin-Opener-Policy` on `/*` fit in one rule.
- **Pages** doesn't support wildcard custom domains ([wildcard DNS records](https://developers.cloudflare.com/dns/manage-dns-records/reference/wildcard-dns-records/)), so for Key Addresses it's Workers static assets on a route, not Pages.

## 2. Durable Objects on the Free plan

From [Durable Objects pricing](https://developers.cloudflare.com/durable-objects/platform/pricing/) (updated Oct 18, 2026) unless noted:

- **Availability.** The Free plan has only SQLite-backed Durable Objects. Free limits reset at 00:00 UTC, and "operations fail once a limit is exceeded" (paraphrased by the fetch tool; the gist is that a Relay on Free stops for the rest of the UTC day).
- **Requests.** Free: 100,000 a day. Paid: 1 million a month, then $0.15 per million. Billed requests: "Includes HTTP requests, RPC sessions, WebSocket messages, and alarm invocations." Each RPC session, which means each method call on a stub, is billed as one request, so calls from a Worker or from another Durable Object count. The pricing examples "exclude the costs for the Workers calling the Durable Objects", which are billed as Workers requests.
- **WebSocket messages.** Opening a WebSocket is a request. "There is no charge for outgoing WebSocket messages, nor for incoming WebSocket protocol pings". "For compute requests billing-only, a 20:1 ratio is applied to incoming WebSocket messages to factor in smaller messages for real-time communication": 100 incoming messages bill as 5 requests.
- **Duration.** Free: 13,000 GB-s a day. Paid: 400,000 GB-s a month, then $12.50 per million GB-s. "Duration billing charges for the 128 MB of memory your Durable Object is allocated, regardless of actual usage." "Durable Objects that are idle and eligible for hibernation are not billed for duration" (even before they're actually hibernated). Calling `ws.accept()` (the standard API) bills duration for the whole time the socket is open. The Hibernation API avoids that.
- **Storage (Free).** 5 GB in total, 5 million rows read and 100,000 rows written a day. A Relay stores nothing, so this doesn't matter.
- **Limits** ([Durable Objects limits](https://developers.cloudflare.com/durable-objects/platform/limits/), updated Oct 29, 2026): "An individual Object has a soft limit of 1,000 requests per second". CPU is 30 s per request or WebSocket message by default, configurable to 5 min, and each message resets the budget. Received WebSocket messages are limited to 32 MiB. There's no cap on the number of objects. Free accounts get 100 classes.
- **Hibernation** ([Durable Objects WebSockets](https://developers.cloudflare.com/durable-objects/best-practices/websockets/), updated Oct 30, 2026; [lifecycle](https://developers.cloudflare.com/durable-objects/concepts/durable-object-lifecycle/), updated Oct 30, 2026; [state API](https://developers.cloudflare.com/durable-objects/api/state/), updated Sep 30, 2026):
  - `ctx.acceptWebSocket(server)` with `webSocketMessage` and `webSocketClose` handlers lets the object hibernate "after 10 seconds of no incoming request or event". Waking re-runs the constructor, so per-socket state (e.g. the client's key) goes in `serializeAttachment`, at most 16,384 bytes.
  - Hibernation is "only supported when a Durable Object acts as a WebSocket server". Outbound WebSockets, `setTimeout` and `setInterval`, and pending I/O all prevent it.
  - The runtime answers protocol-level ping frames without waking the object. `setWebSocketAutoResponse` maps a fixed request string to a fixed response (at most 2,048 characters each). **That can't answer iroh's relay Ping, which carries 8 random bytes the Pong must echo, so every iroh keepalive is an incoming message that wakes the object** (derived from both sources).
  - At most 32,768 hibernatable WebSockets per object. `webSocketMessage` gets binary frames as `ArrayBuffer`.

## 3. Wildcard subdomains on a free zone

- **Wildcard DNS.** "Customers on all plans can create and proxy wildcard DNS records." A wildcard works only in the first label, and `*.r` under `example.com` is valid ([wildcard DNS records](https://developers.cloudflare.com/dns/manage-dns-records/reference/wildcard-dns-records/), updated Apr 16, 2026).
- **Universal SSL is one level deep.** It covers the "root domain (for example, `example.com`) and first-level subdomains". Deeper names such as `dev.www.example.com` "are **not** covered and will not serve a valid certificate" ([Universal SSL limitations](https://developers.cloudflare.com/ssl/edge-certificates/universal-ssl/limitations/), updated Oct 9, 2026). **So on a free zone, the Relay's domain must be the zone apex** (e.g. `tooobrelay.net`, with Key Addresses at `<host key>.tooobrelay.net`). `<host key>.r.example.com` gets no certificate.
- **A subdomain as its own zone** (subdomain setup) is Enterprise only ([subdomain setup](https://developers.cloudflare.com/dns/zone-setups/subdomain-setup/), updated Aug 14, 2026).
- **Advanced Certificate Manager** adds certificates that "cover more than one level of subdomain" (one wildcard per level, e.g. `*.staging.example.com`), up to 50 hostnames per certificate including the apex, and Total TLS, which covers every proxied hostname automatically. It's a paid add-on on Free ([ACM](https://developers.cloudflare.com/ssl/edge-certificates/advanced-certificate-manager/), updated Aug 14, 2026). Price: "Starting at $10 per month" ([cloudflare.com/personal](https://www.cloudflare.com/en-ca/personal/)). The docs pages give no price.
- **Routing a Worker on a wildcard.** Routes take a leading-wildcard hostname: `*.example.com/*` matches subdomains only, so pair it with `example.com/*`. The most specific pattern wins, and a route with no Worker excludes a path from a broader one. Every hostname needs a proxied DNS record ([Routes](https://developers.cloudflare.com/workers/configuration/routing/routes/), updated Oct 18, 2026). Routes per zone are capped at 1,000. "Custom Domains do not support wildcard DNS records", though a Custom Domain does get its own Advanced Certificate without ACM, even several levels deep ([Custom Domains](https://developers.cloudflare.com/workers/configuration/routing/custom-domains/), updated Sep 29, 2026). That covers only named hosts, never every Key Address.
- **Header rules.** Transform Rules can remove the `Cookie` request header ("you cannot set or modify the value of `cookie` … but you can remove these headers"; [request header modification](https://developers.cloudflare.com/rules/transform/request-header-modification/)). Free zones get 10 active Transform Rules ([Transform Rules](https://developers.cloudflare.com/rules/transform/), updated Aug 14, 2026).

## 4. Cloudflare Tunnel on Free

- "Available on all plans" ([Tunnel](https://developers.cloudflare.com/tunnel/), updated Sep 11, 2026). "Every account includes 1,000 tunnels at no cost"; more cost $4.50 per tunnel per month ([Tunnels for Platforms](https://developers.cloudflare.com/tunnel/features/tunnels-for-platforms/), updated Oct 16, 2026).
- WebSockets: "Yes. Cloudflare Tunnel has full support for Websockets" ([Tunnels FAQ](https://developers.cloudflare.com/cloudflare-one/faq/cloudflare-tunnels-faq/), updated Sep 16, 2026).
- Limits: 1,000 tunnels per account, 25 replicas per tunnel, and published applications limited only by a 1 MB configuration. **No bandwidth or connection limits are listed** ([account limits](https://developers.cloudflare.com/cloudflare-one/account-limits/), updated Oct 16, 2026).
- Wildcards: a locally managed `cloudflared` config takes `hostname: *.example.com` (not `test.*.example.com`), path regexes, and must end with a catch-all rule ([configuration file](https://developers.cloudflare.com/cloudflare-one/networks/connectors/cloudflare-tunnel/do-more-with-tunnels/local-management/configuration-file/), updated Oct 22, 2026). The dashboard routing docs say nothing on wildcards and say to create "a CNAME entry for each hostname" ([Tunnel routing](https://developers.cloudflare.com/tunnel/routing/), updated Oct 16, 2026). Today's Bootstrap only reaches iroh-relay on the apex, so option A needs no wildcard in the tunnel.
- Terms: the FAQ says public-hostname routes fall under the CDN terms' video and large-file clause (§5).

## 5. Terms on video, large files, and proxying

- **CDN (Free, Pro, Business)** ([Service-Specific Terms: Application Services](https://www.cloudflare.com/service-specific-terms-application-services/), updated Sep 28, 2026). The CDN "can be used to cache and serve web pages and websites." "Unless you are an Enterprise customer, Cloudflare offers specific Paid Services (e.g., the Developer Platform, Images, and Stream) that you must use in order to serve video and other large files via the CDN." "Cloudflare reserves the right to disable or limit your access to or use of the CDN, or to limit your End Users' access to certain of your resources through the CDN, if you use or are suspected of using the CDN without such Paid Services to serve video or a disproportionate percentage of pictures, audio files, or other large files." "We will use reasonable efforts to provide you with notice of such action."
- **Tunnel.** On Free, Pro and Business, the terms "require you to use a specific paid service to serve video and other large files". This applies to public hostname routes, not private network routes ([Tunnels FAQ](https://developers.cloudflare.com/cloudflare-one/faq/cloudflare-tunnels-faq/)).
- **Developer Platform** ([Service-Specific Terms: Developer Platform](https://www.cloudflare.com/service-specific-terms-developer-platform/), updated Sep 28, 2026). Cloudflare may limit storage or requests that "put an undue burden on the Cloudflare network". Nothing says whether Workers or Durable Objects carrying video is allowed or barred. The only video clause is about Stream.
- **Every service** ([Self-Serve Subscription Agreement](https://www.cloudflare.com/terms/), updated Sep 12, 2025). §2.2.1: "Unless otherwise expressly permitted in writing by Cloudflare, you will not and you have no right to: … (j) use the Services to provide a virtual private network or other similar proxy services." Also (b) no "undue burden on the Services", and (c) no circumventing "Service-specific usage limits". §2.6 lets Cloudflare end a Free Service "in our sole discretion". §8 lets it suspend "at any time, with or without notice for any reason or no reason at all".

**How this applies to a Relay (a judgement, not a ruling).**

- *Option A (CDN plus Tunnel).* Relayed traffic is QUIC inside a binary WebSocket, and Cloudflare can't see that it's video. The clause is "use or are suspected of using … to serve video or a disproportionate percentage of … large files", though, and a Relay for a Site with video does serve video. The Free plan names no Paid Service that would cover it.
- *Option B (Workers and Durable Objects).* The CDN clause names "the Developer Platform" as one of the Paid Services that makes video allowed. Whether Workers Free counts as a "Paid Service" isn't said.
- *Both.* §2.2.1(j) is the broader risk. A Relay forwards traffic between parties it can't read, which a plain reading could call "similar proxy services". Cloudflare also sells relaying as a product (Realtime TURN, metered per GB), which suggests it sees relaying as something to charge for (an inference). Enforcement is discretionary. The realistic failure is a Relay switched off without warning, which takes every Host on it offline. Hosts don't fall back to another Relay (ADR 0015, ADR 0020).
- **Trust (ADR 0007, 0020).** Behind Cloudflare's TLS-terminating proxy, Cloudflare sees what a Relay Operator sees: which keys talk (relay frames carry the source and destination Endpoint IDs in clear), when, how much, and Visitors' IPs. It also serves, and could change, the Bootstrap code. ADR 0020 accepts the Relay Operator as the party Visitors trust. With Cloudflare, that party is the Relay Operator and Cloudflare together.

## 6. The iroh-relay 1.3.0 server protocol

Links are to the [`v1.3.0` tag](https://github.com/n0-computer/iroh/tree/v1.3.0/iroh-relay/src).

**HTTP endpoints** ([server.rs](https://github.com/n0-computer/iroh/blob/v1.3.0/iroh-relay/src/server.rs)):

| Path | What it does |
|---|---|
| `GET /relay` | WebSocket upgrade, the relay itself ([http.rs:13](https://github.com/n0-computer/iroh/blob/v1.3.0/iroh-relay/src/http.rs#L13)) |
| `GET /ping` | 200, empty, `Access-Control-Allow-Origin: *`, for latency probes ([server.rs:1030](https://github.com/n0-computer/iroh/blob/v1.3.0/iroh-relay/src/server.rs#L1030)) |
| `GET /generate_204` | captive-portal check: 204, and if `X-Iroh-Challenge` (1–63 chars of `[A-Za-z0-9._-]`) is sent, `X-Iroh-Response: response <challenge>` ([server.rs:1053](https://github.com/n0-computer/iroh/blob/v1.3.0/iroh-relay/src/server.rs#L1053)). Without `[tls]` it's served on the same HTTP listener ([server.rs:820-826](https://github.com/n0-computer/iroh/blob/v1.3.0/iroh-relay/src/server.rs#L820)). It's meant for plain HTTP, which is why today's Caddyfile exempts it from the HTTPS redirect |
| `GET /healthz` | 200, JSON `{"status":"ok","version":"1.3.0","git_hash":"unknown"}` ([server.rs:1094](https://github.com/n0-computer/iroh/blob/v1.3.0/iroh-relay/src/server.rs#L1094)) |
| `GET /`, `/index.html`, `/robots.txt` | a small HTML page and `Disallow: /`. Today Caddy sends these to the Bundle |

**The upgrade** ([http_server.rs:573-670](https://github.com/n0-computer/iroh/blob/v1.3.0/iroh-relay/src/server/http_server.rs#L573)):

- The request needs `Upgrade: websocket`, `Sec-WebSocket-Version: 13`, a `Sec-WebSocket-Key` and `Sec-WebSocket-Protocol` listing `iroh-relay-v2` and/or `iroh-relay-v1`.
- The server picks the newest one it supports and echoes it. A missing or unsupported subprotocol is a 400.
- An optional `x-iroh-relay-client-auth-v1` header carries the TLS-exporter auth (below). In browsers, an access token goes in the `?token=` query parameter ([http.rs](https://github.com/n0-computer/iroh/blob/v1.3.0/iroh-relay/src/http.rs)).
- After that it's RFC 6455 binary messages. The server caps payloads at 1 MiB (`MAX_FRAME_SIZE`), and each relay frame is one binary message.
- Upgrade and handshake must finish within 30 s (`ESTABLISH_TIMEOUT`).

**Handshake** ([handshake.rs](https://github.com/n0-computer/iroh/blob/v1.3.0/iroh-relay/src/protos/handshake.rs)). Frames are a QUIC varint frame type followed by a postcard-encoded body.

1. If the auth header is present and verifies, the server skips straight to step 3. The header is base64url (no padding) of postcard `{public_key: 32 B, signature: 64 B, key_material_suffix: 16 B}`. The client signs the first 16 bytes of a TLS exporter (RFC 5705) with label `"iroh-relay handshake v1"` and context = its public key. The module docs say this "might break when there's an HTTPS proxy that doesn't properly deal with this TLS feature", and is "not available in browsers". **Behind any TLS-terminating proxy (Caddy today, Cloudflare in either option) the exporters differ, verification fails, and the server falls back to the challenge.** That's "part of normal operation", at the cost of one extra round trip ([handshake.rs:421-470](https://github.com/n0-computer/iroh/blob/v1.3.0/iroh-relay/src/protos/handshake.rs#L421)).
2. The server sends `ServerChallenge` (type 0): 16 random bytes. The client answers `ClientAuth` (type 1): its 32-byte public key and an Ed25519 signature over `blake3::derive_key("iroh-relay handshake v1 challenge signature", challenge)` ([handshake.rs:52, 210](https://github.com/n0-computer/iroh/blob/v1.3.0/iroh-relay/src/protos/handshake.rs#L210)).
3. The server runs access control, then sends `ServerConfirmsAuth` (type 2, empty body) or `ServerDeniesAuth` (type 3, a reason string).

**Frames after the handshake** ([common.rs:18](https://github.com/n0-computer/iroh/blob/v1.3.0/iroh-relay/src/protos/common.rs#L18), [relay.rs](https://github.com/n0-computer/iroh/blob/v1.3.0/iroh-relay/src/protos/relay.rs)):

| Type | Direction | Body |
|---|---|---|
| 4 `ClientToRelayDatagram` | client→relay | 32 B destination key, 1 B ECN, one datagram |
| 5 `ClientToRelayDatagramBatch` | client→relay | 32 B destination, 1 B ECN, u16 BE segment size, the datagrams |
| 6 / 7 `RelayToClientDatagram(Batch)` | relay→client | the same, with the **source** key in place of the destination |
| 8 `EndpointGone` | relay→client | 32 B key of a peer that sent to you and has disconnected |
| 9 `Ping` / 10 `Pong` | both | 8 B. Either side may ping, and the other echoes the payload |
| 11 `Health` | relay→client | UTF-8 text. v1 only, and an error on v2 |
| 12 `Restarting` | relay→client | two u32 BE milliseconds: reconnect in, try for |
| 13 `Status` | relay→client | 1 B: 0 healthy, 1 same endpoint ID connected elsewhere, 2 rate-limited. v2 only |

- A frame's content is limited to 64 KiB (`MAX_PACKET_SIZE`, [relay.rs:23](https://github.com/n0-computer/iroh/blob/v1.3.0/iroh-relay/src/protos/relay.rs#L23)).
- The server pings every 15 s, and a Pong that doesn't come within 5 s drops the client ([relay.rs:36](https://github.com/n0-computer/iroh/blob/v1.3.0/iroh-relay/src/protos/relay.rs#L36), [ping_tracker.rs](https://github.com/n0-computer/iroh/blob/v1.3.0/iroh-relay/src/ping_tracker.rs)). The iroh client sends its own Ping after 15 s without receiving anything ([actor.rs:74, 617](https://github.com/n0-computer/iroh/blob/v1.3.0/iroh/src/socket/transports/relay/actor.rs#L74)).
- Routing ([clients.rs](https://github.com/n0-computer/iroh/blob/v1.3.0/iroh-relay/src/server/clients.rs)):
  - A datagram for a key with no connection is dropped silently.
  - A second connection with the same key becomes active. The old one gets `Status(SameEndpointIdConnected)`, and gets `Status(Healthy)` back if the new one leaves.
  - When a key's last connection goes, every peer it had sent to gets `EndpointGone`.
  - Each client has a 512-frame send queue, and frames are dropped when it's full. Writes time out after 2 s.
- **Batching.** In the browser, iroh sends **one datagram per frame**: with no custom transports, `max_transmit_segments` is 1 under `wasm_browser` ([transports.rs:428-434](https://github.com/n0-computer/iroh/blob/v1.3.0/iroh/src/socket/transports.rs#L428)). A native Host batches only as far as its OS's segmentation offload allows.
- **What a reimplementation must match:**
  - the upgrade, including subprotocol negotiation and echo;
  - the challenge handshake (ignore the auth header);
  - postcard encodings: fixed arrays as raw bytes, `serde_bytes` signatures and strings with a varint length;
  - Ed25519 verification and BLAKE3 `derive_key`;
  - frames 4–10 and 13, rewriting 4/5 into 6/7;
  - Ping/Pong echo, `EndpointGone` and same-key replacement;
  - `/ping` with CORS, `/generate_204`, and optionally `/healthz`.

  `Restarting` and rate limiting are optional.
- **Stability.**
  - "Iroh version 1.0 asserts stability for both the wire protocol and language APIs … Any change that affects the wire stability of iroh will always coincide with a major release", and "Wire-breaking relay changes will get new URLs so older clients keep working" ([iroh 1.0 announcement](https://www.iroh.computer/blog/v1), n0's own, 2026-06-15).
  - The [release policy](https://docs.iroh.computer/about/release-policy): "The wire protocol must also remain compatible across minor versions within the same major series." Across majors it stays compatible with the previous major only. The policy doesn't mention the relay protocol by name.
  - History: `iroh-relay-v2` came in iroh 0.98.0 ([release notes](https://github.com/n0-computer/iroh/releases/tag/v0.98.0), 2026-04-17). It removed `Health` and added `Status`, negotiated by subprotocol, and `v1` is still accepted. 1.0.0 shipped 2026-06-15 and 1.3.0 on 2026-09-28. No 1.x release note marks a relay wire change.
  - A copy would track this protocol by hand, and new frame types are possible within 1.x, since `FrameType` is `#[non_exhaustive]`.

## Assessment

### (A) The Bundle on Workers static assets, stock iroh-relay behind a Tunnel

**Setup.**
- The Relay's domain is a zone apex (`relay.net`). Proxied records go at the apex (a CNAME to the tunnel) and at `*`.
- The Worker with assets takes routes `relay.net/*` and `*.relay.net/*`, plus no-Worker routes for `relay.net/relay`, `/ping`, `/generate_204` and `/healthz`, which fall through to the tunnel and on to iroh-relay as it runs today. Alternatively the Worker can `fetch()` a tunnel-only hostname for those paths: one Worker request per connection, and messages aren't counted.
- `_headers` carries the three response headers, and one Transform Rule strips `Cookie`.
- A variant, A′, sends everything through the tunnel to today's image with Caddy on plain HTTP. That keeps Caddy's routing exactly, but the Bundle then crosses the tunnel too.

**What works on Free.** Everything listed: Universal SSL covers `relay.net` and `*.relay.net`, the Tunnel is included at no cost with WebSockets, static assets are free and unlimited, and no Bootstrap or Host code changes. iroh-relay needs a machine. On the Owner's own box, next to the Host, relayed bytes leave the home connection once, the same as on the Direct Path. A Relay for one Owner then shares its fate with that Owner's Host, which goes down with it anyway.

**Capacity.** Cloudflare lists no Tunnel or CDN bandwidth limit (§4). The ceiling is the machine's uplink and the terms (§5), not a quota.

**Domain and certificate.** The apex of a zone of its own, or ACM at $10/month for a deeper name such as `r.example.com`.

**Terms risk.** The highest of the two options for video: CDN plus Tunnel is exactly what the CDN clause and the Tunnel FAQ name, and §2.2.1(j) applies as well.

**Open questions.**
- Do Workers static assets work on a wildcard route?
- Does `_headers` apply to the SPA fallback?
- Is a missing `/__tooob/*` returning the shell acceptable?
- Is `/generate_204` over plain HTTP still reachable (needs "Always Use HTTPS" off)?
- What does the Tunnel do to WebSocket throughput and latency? Not measured.

### (B) A Worker and Durable Objects reimplementation, one object per Host

**Setup.**
- A Worker on `*.relay.net/relay` reads the Host Key from the hostname's first label and forwards the upgrade to `idFromName(hostKey)`.
- The object runs the handshake and frame routing from §6, with `acceptWebSocket` so it hibernates.
- The Bundle stays on static assets, as in A.
- Both ends must reach the same object, so **Hosts and Bootstraps must connect at the Key Address**. `shell.js` would pass `location.origin + '/'`, not the parent domain, and a Host's relay URL would become `https://<its host key>.<relay domain>/`. That changes how ADR 0020's Key Address is derived from the Relay, and touches the Relay's Bootstrap check at start. Universal SSL covers it.
- The object can also refuse frames that aren't to or from its Host, which stock iroh-relay doesn't.

**What works on Free.** SQLite-backed objects, incoming WebSockets with hibernation, Ed25519 in WebCrypto, and no egress charge. The iroh client's own Ping every 15 s of silence wakes the object, which hibernation's fixed auto-response can't answer (§2). With nothing else arriving, a connected Host sends 4 pings a minute, 5,760 a day, which bills as 288 requests a day. Duration is billed only while the object can't hibernate.

**Capacity, worked out** (a model, not a measurement):
- *Assumptions:*
  - Relayed traffic is mostly Host → Visitor.
  - QUIC packets are about 1,200 B.
  - The Visitor ACKs every second packet (RFC 9000's default).
  - The browser sends one datagram per message (§6).
  - A macOS Host doesn't batch. A Linux Host may batch about 10 segments a frame.
- *Unbatched:* 1 GB is 1e9 / 1,200 = 833,333 data messages from the Host plus 416,667 ACK messages from the Visitor = 1,250,000 incoming messages. At 20:1 that bills as **62,500 requests per GB**.
- *Host batching 10:* 83,333 + 416,667 = 500,000 messages = **25,000 requests per GB**.
- *Free cap:* 100,000 requests a day ⇒ **about 1.6 GB a day unbatched, about 4 GB a day batched**. That is ≈ 48–120 GB a month only if every day is identical, because the cap resets daily and a busy day stops the Relay until 00:00 UTC.
- *Duration on Free:* 13,000 GB-s ÷ 0.125 GB = 104,000 object-seconds a day (≈ 29 h). Even busy all day at 10 Mbit/s (1.6 GB in 1,280 s = 160 GB-s), duration doesn't bind first.
- *Throughput:* if the 1,000-requests-a-second soft limit counts WebSocket messages, one Host's object tops out near 1,000 × 1,200 B × ⅔ (data share) ≈ 0.8 MB/s ≈ 6–7 Mbit/s unbatched. That is unverified, and it's the main open question for video.

**Domain and certificate.** The same as A: a zone apex, or ACM.

**Terms risk.** Somewhat lower on the video clause, since the CDN clause names the Developer Platform among the Paid Services that make video allowed, though it's unclear whether Free counts. §2.2.1(j) is unchanged.

**Open questions.**
- Does the soft limit count messages?
- The Worker's 101 response must echo `Sec-WebSocket-Protocol`. Unverified in the docs.
- BLAKE3 in a Worker means bundling a JS or WASM implementation.
- The object's location: it's created near the first request, normally the Host's.
- Upkeep: keeping a protocol copy in step with iroh 1.x.
- Is the ADR 0020 change acceptable?

## Cost comparison (for #73)

"Relayed traffic" here is the bytes a Relay forwards out per month. **Most of a Visit goes over the Direct Path and never touches the Relay.** Per visit, the Relay carries only the hello, the WebRTC offer and answer with their ICE candidates, and the requests made before the Direct Path is up (about 300 ms on Wi-Fi, about 650 ms on cellular; the shell holds the first page up to 800 ms for it, `spike/README.md:122-135`). That's a few KB. So **light (5 GB/month) is the expected case.** **Heavy (500 GB/month)** is a Visitor behind a NAT that can't hole-punch (symmetric NAT on some carriers and corporate networks, `spike/README.md:137`). With no TURN server (ADR 0004), everything they load is relayed, video included. The Bootstrap Bundle (3.1 MB per first load and per Release, then cached, with cheap 304 rechecks) is counted separately: free as Cloudflare static assets, and egress elsewhere (e.g. 100 first loads ≈ 0.31 GB ≈ $0.006 on Fly in North America). Domain registration is left out everywhere.

**Unit prices, checked 2026-10-30:**

| Provider | Base | Transfer | Source |
|---|---|---|---|
| Cloudflare Free, A | $0 (Tunnel included at no cost, static assets free) | none listed | §1, §4 |
| Cloudflare Free, B | $0 | none, until the daily request cap: ~1.6–4 GB a day | §2, Assessment B |
| Workers Paid | $5/month minimum; Workers: 10M requests and 30M CPU-ms included, then $0.30/M and $0.02/M CPU-ms; DO: 1M requests and 400,000 GB-s included, then $0.15/M and $12.50/M GB-s | "no additional charges for data transfer (egress) or throughput" | [Workers pricing](https://developers.cloudflare.com/workers/platform/pricing/), [DO pricing](https://developers.cloudflare.com/durable-objects/platform/pricing/) |
| ACM (if the Relay isn't a zone apex) | from $10/month | n/a | [cloudflare.com/personal](https://www.cloudflare.com/en-ca/personal/) |
| Fly.io as `deploy/fly-relay/` runs it | `shared-cpu-1x` 256 MB always on $2.19 + 1 GB volume $0.15 + dedicated IPv4 $2.00 = **$4.34/month**; no free tier for new organizations | outbound $0.02/GB in North America and Europe (our `sjc`), $0.04 in Asia Pacific, Oceania and South America, $0.12 in Africa and India; inbound free | [Fly pricing](https://docs.fly.io/about/pricing) (page undated) |
| DigitalOcean | $4/month: 512 MiB, 1 vCPU, 500 GiB transfer | $0.01/GiB over, pooled per team; inbound free | [Droplet pricing](https://www.digitalocean.com/pricing/droplets), [bandwidth](https://docs.digitalocean.com/platform/billing/bandwidth/) (verified Oct 26, 2026) |
| Hetzner, EU | CX23 €5.99 (2 vCPU, 4 GB; "currently unavailable") or CPX02 €6.49 (1 vCPU, 1 GB), IPv4 included | 20 TB included, then €1.00 per TB (the page says "per hour", apparently a typo) | [cost-optimized](https://www.hetzner.com/cloud/cost-optimized/), [regular performance](https://www.hetzner.com/cloud/regular-performance/) |
| Hetzner, US | CPX11 €17.99 (2 vCPU, 2 GB) | 1 TB included, then €1.00/TB | [regular performance](https://www.hetzner.com/cloud/regular-performance/) |
| OVHcloud | VPS-1 from $4.54/month (2 vCPU, 4 GB, 500 Mbit/s), possibly on 12-month terms | "Unlimited traffic" | [OVHcloud VPS](https://www.ovhcloud.com/en/vps/) |

**Monthly cost by relayed traffic.** B on Paid uses the unbatched 62,500 requests per GB.

| | Light, 5 GB | Medium, 50 GB | Heavy, 500 GB |
|---|---|---|---|
| Cloudflare Free, A | $0 | $0 | $0 (no quota; terms risk grows with video) |
| Cloudflare Free, B | $0 (312,500 requests/month ≈ 10,400/day, 10% of the cap) | $0 only with batching (unbatched 3.125M/month ≈ 104,000/day, **over the cap on an average day**; batched ≈ 41,700/day) | **runs out**: ≈ 1.04M requests/day, 10× the cap |
| Workers Paid, B | $5.00 (0.31M requests ≤ 1M included) | $5 + (3.125M − 1M) × $0.15/M = **$5.32** | $5 + (31.25M − 1M) × $0.15/M = **$9.54**; duration at 10 Mbit/s is 400,000 s × 0.125 = 50,000 GB-s, inside the 400,000 included |
| Workers Paid, A | $5 buys nothing A needs: **$0** | $0 | $0 |
| Fly.io, NA/EU | $4.34 + 5 × $0.02 = **$4.44** | $4.34 + $1.00 = **$5.34** | $4.34 + $10 = **$14.34** |
| Fly.io, APAC | $4.54 | $6.34 | $24.34 |
| Fly.io, Africa/India | $4.94 | $10.34 | $64.34 |
| DigitalOcean $4 | $4.00 | $4.00 | $4.00 (500 GB = 465.7 GiB < 500 GiB) |
| Hetzner EU | €5.99–6.49 | €5.99–6.49 | €5.99–6.49 (20 TB included) |
| Hetzner US | €17.99 | €17.99 | €17.99 (1 TB included) |
| OVHcloud VPS-1 | $4.54 | $4.54 | $4.54 |

Add $10/month to either Cloudflare option if the Relay's domain isn't a zone apex. Where the free tier runs out: B on Free stops for the day at about 1.6–4 GB relayed in a UTC day, around medium. A has no quota, only the terms.

**STUN.** tooob uses Google's `stun.l.google.com:19302` today. Cloudflare documents its own: "Cloudflare's STUN service at `stun.cloudflare.com` is free and unlimited" (UDP 3478; [Realtime TURN FAQ](https://developers.cloudflare.com/realtime/turn/faq/), updated Jul 14, 2026). The page doesn't say whether an account is needed.

## Unverified

- Whether a Worker with static assets serves on a wildcard **route** (`*.relay.net/*`). The docs cover routes and assets separately but never together.
- Whether `_headers` rules apply to the SPA-fallback `index.html` served for an unmatched path.
- Whether the Durable Objects soft limit of 1,000 requests a second counts WebSocket messages. That would cap one Host's relayed throughput at a few Mbit/s.
- Whether an incoming WebSocket message is the "event" that wakes a hibernated object (implied, not stated), and the exact wording of "operations fail" after the Free limits (paraphrased by the fetch tool).
- Whether a Worker's 101 response can set `Sec-WebSocket-Protocol`, which iroh's server sends back and its clients may check.
- That Cloudflare doesn't meter CDN or Tunnel bandwidth on Free. No primary page prices it, and none lists a limit.
- ACM's price. Only the marketing page's "Starting at $10 per month"; the docs give none.
- The packet model behind B's capacity: 1,200-byte packets, ACKs every second packet, about 10 segments a frame from a Linux Host. Modelled, not measured.
- How the terms read for a Relay: whether Workers Free counts as a CDN-terms "Paid Service", and whether §2.2.1(j) "similar proxy services" covers a Relay. Only Cloudflare can say.
- The postcard byte layout of `ClientAuth`'s signature (`serde_bytes` as a varint-length-prefixed byte string). Read from source, not checked against a test vector.
- Whether the browser's iroh client probes `/ping` or `/generate_204`, and what fails if a Cloudflare setup can't serve `/generate_204` over plain HTTP.
- Hetzner's overage wording ("€1.00 per hour" per extra TB) and its IPv4 price. DigitalOcean's $4 Droplet including an IPv4. OVH's tax basis and commitment. The Fly pricing page carries no date.
- Whether `stun.cloudflare.com` needs a Cloudflare account. The terms for Google's STUN, which tooob uses today, weren't checked.
- The Durable Objects limits page gives two figures for per-object storage on Free (10 GB and 1 GB). Irrelevant to a Relay.
