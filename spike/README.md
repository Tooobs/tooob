# tooob browser spike

Throwaway proof that an ordinary browser can reach a web server behind NAT through iroh, with no install. See `../DESIGN.md` for the pass bar.

```
browser: shell page (iroh WASM) ─┬─ iframe: the site
                                 ├─ service worker: site's fetches → shell → iroh stream
                                 └─ ws-shim.js: site's WebSockets → shell → iroh stream
        ⇅ WSS to an iroh relay (browsers are relay-only)
host:   target/release/host <port>  → reverse proxy to 127.0.0.1:<port>
```

- `web/`: Rust → WASM. An iroh endpoint plus "open a bidirectional stream". JS does all framing.
- `host/`: iroh endpoint that proxies each stream to the local HTTP server. Also proxies WebSockets. Wire format is at the top of `host/src/main.rs`.
- `www/`: the bootstrap: shell, service worker, WebSocket shim.
- `check/`: headless-Chrome pass/fail checks.

## Run

```sh
./build.sh                                              # needs: brew llvm, wasm-bindgen-cli 0.2.129, wasm32 target
npx http-server testsite -p 8000 -c-1 -s                # something to serve (needs Range support for video)
npx http-server www -p 8088 -c-1 -s                     # the bootstrap
docker run -d --name tooob-relay -p 3340:3340 n0computer/iroh-relay:v1.3.0 --dev   # optional local relay
./target/release/host 8000 [relay-url]                  # prints the visit URL; default relay is n0's
open "<visit URL>"

cd check && npm i
node check.mjs "<visit URL>" [--idle 600]               # criteria 1-4
docker run -d --name tooob-ha -p 8123:8123 ghcr.io/home-assistant/home-assistant:stable
node ha-setup.mjs                                       # once: throwaway HA user, tokens -> ha-tokens.json
(mkdir -p ../ha-host && cd ../ha-host && ../target/release/host 8123 [relay-url])
node check-ha.mjs "<HA visit URL>"                      # criterion 5
node cookie-app.mjs &                                   # cookie-login test app on :8010
(mkdir -p ../cookie-host && cd ../cookie-host && ../target/release/host 8010 [relay-url])
node check-cookie.mjs "<cookie visit URL>"              # cookie login
```

Each host keeps its key in `tooob-host.key` in its working directory, so run each from its own folder. To deploy a real relay, see `../deploy/relay/`.

The test site's images and video are generated, not committed: run `./gen-testsite.sh` (needs ffmpeg).

## Results (2026-10-18, M4 Mac, host and browser on the same machine, n0 community relay `usw1`)

Five runs of `check.mjs` and three of `check-ha.mjs`:

| # | Criterion | Result |
|---|---|---|
| 1 | 2.5 MB page < 3 s, bootstrap cached | **Marginal PASS**: 0.8–2.6 s on a quiet network (connect 0.2–0.7 s). One run took **15.6 s** while a 1.5 GB Docker pull saturated the uplink |
| 2 | Video seek ~2 s | **Mostly PASS**: most seeks 0.7–2.5 s through a 23 MB / 1.5 Mbit/s MP4. One seek in one run took 5.0 s |
| 3 | WASM < 5 MB | **PASS**: 3.0 MB (1.0 MB gzipped) with `opt-level="s"` + LTO. 5.0 MB without |
| 4 | Survives 10 min idle | **PASS**: a request after 600 s of silence returned in 52 ms |
| 5 | Home Assistant UI loads and updates live | **PASS**: WebSocket up in 1.2–1.4 s, dashboard rendered in 4.2–5.7 s cold, state changes reach the UI in < 55 ms |

**Round 2, self-hosted relay and fixes.** The same checks with a stock iroh-relay on the same machine, which strips out network distance and n0's throttling to show tooob's own overhead:

| Setup | Connect | 2.5 MB page | Video seeks | HA dashboard, cold |
|---|---|---|---|---|
| n0 relay, address lookup | 240–680 ms | 1.2–2.3 s | 1.3–1.5 s | 4.2–5.7 s |
| n0 relay, relay URL in invite | ~70 ms | 2.4–2.5 s | 1.0–2.5 s | – |
| **Self-hosted iroh-relay (local)** | **6 ms** | **113 ms** | **66–303 ms** | **1.5 s** |

Tooob's own overhead is small. The slowness above came from the network path and n0's shared relay (about 8 Mbit/s). A real VPS relay will land in between: tooob's overhead plus round-trip time and the VPS's bandwidth. **Next measurement:** deploy `../deploy/relay` and run the host at home with a visitor on a phone network.

**Round 3, caching and repeat visits.** The service worker now caches fresh `200` GET responses per host, following the app's `Cache-Control: max-age`. Home Assistant's hashed bundles are `max-age=31d`, so a repeat visit sends 1 request over tooob (the page itself) instead of about 100. Measuring that turned up a bigger problem: **on every repeat visit, the shell waited ~1.9 s on `navigator.serviceWorker.register()`** even though the page was already controlled. Rounds 1 and 2 missed it because criterion 1 summed the shell's own stages instead of timing from navigation start, so their page numbers are up to ~2 s too low. Both are fixed: registration no longer blocks a controlled page, and criterion 1 now measures from navigation start.

| After round 3 | 2.5 MB page, from navigation | HA dashboard, cold → repeat visit |
|---|---|---|
| Self-hosted relay (local) | 128–135 ms | 0.86 s → **0.40 s** (HA direct without tooob: 0.65 → 0.39 s) |
| n0 relay, relay URL in invite | 1.9–2.6 s (images aren't cacheable: `no-cache`) | – |

A repeat visit to Home Assistant through tooob is now as fast as visiting it directly.

**Verdict: the browser path works.** Every mechanism held up: WASM iroh, service worker, WebSocket shim, range requests, idle survival, and a real app (Home Assistant). Throughput was noisy, though: the 2.5 MB page took 0.4–15 s, roughly 1–50 Mbit/s, through n0's shared, rate-limited community relay. The page and seek bars pass with little margin, so judge speed again on a self-hosted relay before reading much into these numbers.

All traffic went through the relay, both ends were in the same house, and the relay was in US-West. Real visitors elsewhere add their own round-trip time.

## Findings that change the plan

1. ~~Cookies don't work.~~ **Fixed:** the host keeps a cookie jar per visitor key and strips `Set-Cookie`, so `HttpOnly` session cookies never reach the browser. The browser keeps its visitor key in localStorage, so a login survives reloads (`check-cookie.mjs` passes: form POST → 303 → logged in → reload → still logged in). Limits: the jars are in memory (a host restart logs everyone out), cookies set by page JavaScript don't reach the host, server cookies aren't readable from page JavaScript, and `Secure` cookies aren't sent over the host's plain-HTTP hop to the app.
2. **The site's own service worker can't register.** HA tried to register `/sw-modern.js` and got a 404. HA works without it, but PWAs lose offline support. Probably acceptable.
3. ~~No HTTP caching for tunneled responses.~~ **Fixed** (round 3): a per-host Cache Storage that honors `max-age`. It has no ETag revalidation and no eviction yet. Also fixed: the hidden ~2 s `register()` stall on repeat visits.
4. ~~Connect time is the biggest share of a warm load.~~ **Fixed:** the host prints its relay URL in the visit link (`?relay=`), which skips the pkarr lookup. Connect went from 240–680 ms to about 70 ms on n0, and 6 ms on a local relay.
5. **Chrome won't load media in hidden tabs.** That's why the first video test hung under automation. It's a test-harness artifact, not a tooob bug.
6. ~~These ran on n0's community relays.~~ The host now takes a relay URL, a stock iroh-relay works unchanged (round 2), and `../deploy/relay/` is ready for a VPS.

## Not tested

- A real network split (host on a home connection, visitor on a phone network), and Safari/Firefox. Do these before calling the browser path done.
- Uploads larger than 64 MB (the host buffers request bodies). Many visitors at once.

## Spike 2: direct WebRTC path (2026-10-18)

The shell now negotiates a WebRTC data channel with the host, carrying the offer, answer and trickled ICE candidates over the iroh connection, and moves requests onto it. Host side: `host/src/rtc.rs` (webrtc-rs 0.21). Check: `node check-direct.mjs "<visit URL>"`. Test switches: `?direct=0` (off) and `?direct=fail` (no candidates, so it can't connect).

Host and browser on the same Mac, iroh via n0's US-West relay:

| | Relay only | Direct path |
|---|---|---|
| Direct-path setup | – | **~100 ms** with trickle ICE (5.2 s when waiting for full candidate gathering) |
| 2.5 MB page | 2.5–2.6 s | **57–72 ms** |
| Video seeks | 1.3–1.5 s | **21–241 ms** |
| Bytes over the relay after setup | everything | **0** |

Pass bar: 1 ✅ negotiated over iroh only · 2 ✅ traffic moved (0 bytes on iroh) · 3 ✅ forced failure falls back to iroh, checks still pass · 4 ✅ the DTLS certificate Chrome accepted hashes to the fingerprint the host sent over iroh · 5 ⚠️ cookies pass, but see below. Home Assistant wasn't re-run because Docker is off.

**Finding: bursts into Chrome stall native senders. Fixed by pacing.** Worked through in order:

| Sender | Stalled loads (2.5 MB burst) | Stall length |
|---|---|---|
| webrtc-rs, one channel per stream | ~1/3 | 1–13 s |
| webrtc-rs, one multiplexed channel | 6/10 | 1–13 s; retransmissions fail repeatedly (1, 2, 4… s waits) |
| str0m (`TOOOB_RTC=str0m`), multiplexed | 5/10 | ~1.0–1.4 s; recovers after one 1 s retransmission timeout |
| webrtc-rs paced to 200 / **100** / 50 Mbit/s | 3/10 / **0/10** / 0/10 | – |
| Chrome → Chrome, same burst (control) | 0/10 | – (same browser process, so its sender is naturally paced) |

Conclusions:
1. **Root trigger:** a native sender bursting faster than ~100–200 Mbit/s into Chrome loses packets on loopback. Two independent SCTP stacks (webrtc-rs and str0m) both do it, so it's not one library's bug. Ruled out: packet size (webrtc-rs already caps at 1,191 bytes, unlike the pion report), number of data channels, the send-buffer limit.
2. **Why a loss costs seconds:** both Rust stacks use a ≥1 s retransmission timeout (`RTO_MIN = 1000` is hardcoded in webrtc-rs's `rtc-sctp`).
3. **A webrtc-rs-specific problem on top:** its retransmissions keep failing, so the waits double to 3, 7 or 13 s. str0m recovers after one wait. This is worth reporting upstream.

**Fix in the code:** the host paces direct-path sending to **80 Mbit/s by default** (`TOOOB_PACE_MBPS`, 0 = off; one cap shared by all visitors for now). Result: 0/10 stalls, page 260–340 ms, seeks 18–155 ms. Home upload speeds usually stay under the cap anyway; it matters for fast fiber or a visitor on the same network. Also: the shell holds the first page up to 500 ms for the direct path (it usually connects in 80–300 ms), so a first visit dropped from 2.5 s (via relay) to **0.4–0.6 s**.

**Real network, 2026-10-18.** Relay on Fly (sjc, stock iroh-relay behind Fly's TLS; see `../deploy/fly-relay/`), bootstrap on Fly (`../deploy/fly-bootstrap/`), host on the Mac at home, visitor on an Android phone:

| | Phone, probably on home Wi-Fi | Phone on cellular |
|---|---|---|
| iroh connect via the Fly relay | 503 ms | 546 ms |
| Direct path up (hole-punched) | 486 ms | **658 ms**, through the carrier's NAT |
| First page, 2.5 MB | 701 ms, direct | 1,546 ms via relay (direct came up after the then-500 ms wait) |
| Video + seeking | – | **direct, seeking felt instant** |

Through the relay alone, the Fly relay beat n0's community relay by a wide margin: first visit 1.3 s vs 2.5 s, repeat page 0.4 s vs 2.5 s, seeks 0.11–0.29 s vs 1.3–1.5 s.

Fixed after the phone test: (1) the first page now waits up to **800 ms** for the direct path (cellular took 658 ms); (2) WebRTC's `disconnected` state, which phones hit whenever the browser goes to the background, now **pauses** the direct path and resumes it on `connected`. Only `failed` is final. Before, every app switch dropped the visitor back to the relay for good.

Caveats: one phone, one carrier, and the Wi-Fi/VPN state of the first run wasn't confirmed. Symmetric NATs (some carriers and corporate networks) will still fall back to the relay. That's what the fallback path is for.

**Still open:** more networks and carriers (Fly relay, host at home, visitor on a phone network); report the webrtc-rs retransmission behaviour upstream; decide between webrtc-rs (has STUN) and str0m (recovers better, but needs our own STUN for public addresses).
