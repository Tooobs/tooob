# tooob — design

_Status: decided 2026-10-18. Working title was relay4all; product, repo and CLI name is **tooob**._

## Mission

Turn home infrastructure into web infrastructure for normal people. Run something at home — Home Assistant, Jellyfin, Immich, a NAS, a static site on a Pi — and let the people you choose reach it from an ordinary browser. No port forwarding, no public URL, nothing the relay can read.

**And for their agents** _(added 2026-10-24)_. An AI agent runs a Host for its Owner, or reaches Sites as a Visitor: a status dashboard, a dev preview or an MCP server, shared with people (in a browser) and with other agents (through `tooob visit`, a local proxy). The access model is the same: every Host has a person as its Owner, and every new device an Agent invites waits for that person's approval (ADR 0018). tooob is only the transport: how agents tell each other a link exists (a chat, robofinger, anything) is theirs to choose (ADR 0019).

**Private, not anonymous.** Content is end-to-end encrypted and access is invite-only. The relay still sees which keys talk, when, how much, and from which IPs. We say so plainly and keep that metadata minimal.

## How it works

```
 visitor's browser                          relay (iroh-relay)              home box
 ┌──────────────────────────────┐          ┌────────────────┐          ┌──────────────────┐
 │ <key>.<bootstrap domain>     │          │ forwards        │          │ tooob host        │
 │  shell page: iroh (WASM)  ───┼── WSS ──▶│ encrypted QUIC  │◀── QUIC ─┤  (reverse proxy)  │
 │  iframe: the actual site     │          │ by endpoint key │          │        │          │
 │  service worker: fetch → shell│          └────────────────┘          │        ▼          │
 │  WebSocket shim → shell      │                                       │  localhost:8123   │
 │  WebRTC data channels  ══════╪═══════ direct, hole-punched ═════════▶│                   │
 └──────────────────────────────┘         (most traffic, when it works) └──────────────────┘
```

1. The Owner adds a Host (`tooob add`) and runs it (`tooob run`): an iroh endpoint that reverse-proxies to a local HTTP server, or serves a Shared Folder itself. Its endpoint key is its address.
2. A visitor opens an Invite Link, which starts with the Bootstrap the Host's settings name and carries the Host Key and the Invite in its fragment. The Bootstrap is only a static page; the fragment never leaves the browser.
3. The bootstrap loads iroh compiled to WASM, generates a visitor key (kept in browser storage), and dials the host through the relay.
4. A service worker intercepts the site's requests and hands them to the shell page, which carries them over iroh. Service workers can't intercept WebSockets, so an injected shim replaces `WebSocket` and tunnels it the same way.
5. The host checks the visitor key against its allowlist, then proxies to the local server.
6. Over the iroh connection, browser and host negotiate a **direct WebRTC data channel** (STUN finds addresses, ICE punches through NAT). When it connects, requests move to it; when it doesn't (about 10–20% of connections, more on mobile networks), they stay on iroh. The relay never sees content on either path.

## Decisions

### Product and model
- **v1 is tooob web**: a home box serving a site to invited visitors in ordinary browsers.
- **Built on iroh 1.x** (MIT/Apache, wire-stable across v1). Hole punching is the hardest part and iroh has it. We don't write our own transport.
- **Open source**, `MIT OR Apache-2.0` (ADR 0016).
- **Self-hosted by design**: tooob names no Relay or Bootstrap of its own, and every Host's settings name both (ADR 0015). That's stock iroh-relay plus the static Bootstrap; `deploy/` has both.
- **Addresses are key subdomains** (`<base32 key>.<bootstrap domain>`, 52 chars, fits a DNS label). Each site gets its own origin. **Friendly names** are a v2 feature. The Bootstrap's domain goes on the Public Suffix List.

### Access control
- **The allowlist lives only on the host.** Relays and Bootstraps never learn who your visitors are.
- **Invites**: single-use, accepted on first visit, expire after 7 days, one per device (`--approve` optional). Device linking later.
- **Blocking**: always enforced on the host. Optionally the host also pushes a signed deny upstream so the relay drops that key's traffic (flood protection). **The relay can only take access away, never grant it.**
- **Visitor identity is passed to the app** (`X-Tooob-Visitor: mom`) so apps can personalize without a login system.
- **Host key backup**: `tooob key export` / `tooob key import` in v1 (passphrase-encrypted, includes the visitor database). SD cards die.

### Abuse and legal
- No public ingress, no exit to the open internet, invite-only. Useless for phishing.
- **A Relay Operator's only lever** is refusing to relay for a host key. No content inspection is possible.

### Infrastructure
- **Browsers go direct when they can.** iroh in the browser is relay-only (pages can't open raw UDP), so the shell also negotiates a **WebRTC data channel** with the host. The signaling runs over the already-authenticated iroh connection, so there's no separate signaling server. The DTLS fingerprint arrives over that channel too, which binds the direct path to the host's key. **iroh's relay is the TURN fallback**, so there's no TURN server to run. Only connections that can't hole-punch cost relay bandwidth. _(Revised 2026-10-18; the first plan was relay-only browsers in v1.)_ Longer term, WebRTC as an iroh custom transport would move the path switching into iroh itself. That API is unstable, and the one existing attempt (`iroh-webrtc-transport`) targets pre-1.0 iroh.
- **Relays are stock iroh-relay.** tooob adds nothing to them, so any iroh-relay of the right version will do.

### Scope
- **v1 users**: technical early adopters. **Agents** is the next milestone, then **Normal people**; every v1 choice is checked against Normal people.
- **v1 protocol scope**: HTTP request/response, streaming bodies, range requests (video seeking), WebSockets via shim.
- **Host platforms**: Linux (arm64, x86_64) and macOS. **Packaging**: Docker image plus Home Assistant add-on first. Docker covers Umbrel, CasaOS, Unraid and Synology. Bare CLI for tinkerers. Desktop tray app later (laptops sleep).
- **Management**: one **admin API** on the host. The **web UI** (v1, minimal: invite, list visitors, block, bandwidth) is served as an owner-only tooob site, and first run pairs the owner's browser from a LAN setup page. The CLI is for scripting.
- **Agents** _(milestone after v1, before Normal people)_: Agents host with the CLI, from an Agent Account a person names on a Host they made (ADR 0018), and visit with `tooob visit LINK [--as NAME] [--listen 127.0.0.1:PORT | unix:PATH]`. First use case: an agent's status dashboard, opened by people in a browser and by other agents through `tooob visit`.
- **AI front door**: an **MCP server** that is a thin client of the admin API. Local first (beside Claude Desktop/Code, reaching the host over tooob with an owner key). Remote MCP with OAuth at the normal-people milestone. The agent may read, block and revoke directly, and make invites that need approval on first visit (ADR 0018). Unblocks, allowlist changes and invites without that approval become **pending requests the owner approves** in the web UI. The web UI stays the baseline.
- **v1 also**: the direct WebRTC path for browsers (above), proven first by spike 2.
- **v2**: native visitor app or extension (iroh's own hole punching, no WebRTC needed), friendly names, device linking.

## Risks and how we handle them

| Risk | Compared to the open web | Mitigation |
|---|---|---|
| Visitors trust bootstrap JS from whoever serves the Bootstrap | Same as any web app; better than a TLS-terminating CDN | Versioned, reproducible bootstrap; the Owner chooses (or runs) it; v2 extension |
| Isolation between sites | Same as github.io | Per-key subdomain plus Public Suffix List |
| Home IP visible on direct paths | Same as self-hosting | A "relay only" option (skip WebRTC and native hole punching) for hosts who want their IP hidden |
| Service worker / WASM / WebSocket-shim plumbing | New | **The spike** |
| Prompt injection via the AI front door, or via what an agent reads | New | An agent can't let anyone in unseen: its invites need the owner's approval on first visit, enforced through Agent Accounts (ADR 0018) |

## First step: the spike

A throwaway proof that the browser path works. Lives in `spike/`. **Nothing else gets built until it passes.**

Pass bar, with the relay on the same continent:
1. A 2 MB page with images loads in **under 3 s** on first visit (bootstrap cached).
2. **Video seeking** works within about 2 s.
3. WASM bundle **under 5 MB**.
4. The connection **survives 10 minutes idle**.
5. **Home Assistant's UI loads and updates live** (WebSocket shim).

If any of these fail badly, revisit the browser approach before going further.

**Status:** passed (see `spike/README.md`).

## Spike 2: the direct browser path

Proves that browsers can skip the relay. Pass bar:
1. Browser and host open a **WebRTC data channel negotiated only over the iroh connection** (no other signaling server).
2. **Requests move to it**: after setup, page loads and video go over the direct channel, and the host's per-path byte counts show the relay carrying almost nothing.
3. **Fallback is invisible**: when the direct path can't connect (forced in the test), everything keeps working over iroh with the existing checks still passing.
4. **The direct path is tied to the host's key**: the DTLS fingerprint the browser accepts is the one the host sent over iroh.
5. All earlier checks (page, video, Home Assistant, cookies) still pass.
