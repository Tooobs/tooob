# What the Bootstrap's WASM costs a Visitor

Measured 2026-11-02 for [#112](https://github.com/Tooobs/tooob/issues/112): how long `web_bg.wasm` (iroh in the browser) takes to fetch, compile and instantiate on a cold first visit and on a return visit, on a laptop and on a phone-like browser. It decides whether "the browser's own WebRTC plus TURN in place of iroh" is worth grilling (the table in #112 has the trade-offs).

**Verdict: the rule fires, on the cold case.** On a phone-like browser the WASM adds **3.3 s** to a cold first visit (the line is 1 s), which settles it: the rule is an "or". A return visit sits **on** its 200 ms line (203 ms here, 199 ms in a [re-run](#re-run)), so it doesn't settle anything by itself. On the laptop it adds 260 ms and 57 ms. Most of that is the network rather than WASM work: the Relay sends the WASM uncompressed, its fetch starts late, and every return visit waits for a revalidation round trip. Compiling and instantiating takes 3–21 ms. The [cheaper fixes](#cheaper-fixes-first) would probably bring both phone numbers under their lines, so they come before any swap.

## Setup

- **Hosts:** `target/debug/tooob` from `main` (0.3.3), on this laptop, Relay `https://r.tooobs.net/` (our own, on Fly; about 42 ms TCP connect from here). The stand-in Site from `harness.mjs`. The browser loads the Bootstrap Bundle that Relay serves: Release 0.3.3, `web_bg.wasm` 3,126,387 bytes.
- **Browser:** Playwright's Chromium 156.0.8078.4 (`channel: 'chromium'`, the full browser headless), on an Apple M4 (10 cores). Each run uses a **fresh persistent profile**, so the disk HTTP cache and V8's code cache work as in a normal Chrome profile, which an incognito-like Playwright context wouldn't give.
- **Cold first visit:** the Invite Link in that fresh profile, then 3 s for caches to be written.
- **Return visit:** the browser closes and relaunches on the same profile, then opens the Key Address with no Invite. The service worker is installed and the Visitor Key is kept. Relaunching keeps V8's in-process module cache out of it, like a phone reopening the link later.
- **Laptop:** no throttling.
- **Phone-like:** CPU ×4 (`Emulation.setCPUThrottlingRate`) and DevTools' "Fast 4G" (`Network.emulateNetworkConditions`: 9 Mbit/s down, 1.5 up, 165 ms latency, with DevTools' 0.9 factor on throughput).
- **5 runs per case**, each run doing a cold and then a return visit in its own profile. Medians, with min–max in brackets.

### What each phase is

All times come from the shell's own page, in ms. Instrumented from an init script, so `shell.js` is unchanged:

| Phase | From → to |
|---|---|
| **Fetch** | Resource Timing for `web_bg.wasm`: request start → last byte |
| **Compile + instantiate** | last byte → `init()` resolved (`__tooobStats.wasmMs` set). Streaming compilation overlaps the download, so this is what's left after it. See below for why not the `instantiateStreaming` marks |
| **WASM** | fetch + compile + instantiate: what the decision rule measures |
| **Service worker** | `init()` done → SW registered and controlling (`swMs`) |
| **Connect** | SW ready → iroh connected and the hello answered (`connectMs`). On a cold visit it includes using the Invite |
| **iroh dial** | the part of connect before the hello is signed: the Visitor Key out of IndexedDB, then `Tooob.connect` through the Relay |
| **Site first byte** | connected → the iframe's document `responseStart`. Includes the shell's wait of up to 800 ms for the Direct Path |
| **Total** | navigation start → the Site's page loaded (`totalMs`) |

**Where `instantiate` runs:** in the shell page itself. `shell.js` imports wasm-bindgen's glue (`pkg/web.js`), whose `init()` calls `WebAssembly.instantiateStreaming` (`application/wasm`, so never the `instantiate` fallback) on the page's main thread. There is no dedicated worker, and the service worker (`tooob-sw.js`) loads no WASM. It doesn't intercept `/__tooob/` either: on a return visit the WASM request passes through it to the network (Resource Timing's `workerStart` > 0) and is answered by the HTTP cache.

**Why compile + instantiate ends at `init()`, not at the `instantiateStreaming` marks** #112 named: the script records both. `instantiateStreaming` starts as soon as the glue asks for the WASM, before its first byte, so start → end spans the whole download (laptop cold 209 ms, phone-like cold 3,105 ms) and isn't a compile time. Its end is what the marks add, and `init()` resolves 0–1 ms after it, so either endpoint gives the same number. Last byte → `instantiateStreaming` resolved, from the same runs: laptop cold 5 (3–8), return 3 (2–4); phone-like cold 16 (15–21), return 18 (18–20). `init()` is used because it's what the shell waits on, and what `__tooobStats.wasmMs` on a real phone reports.

## Results

| Case | WASM (fetch + compile) | Fetch | Compile + instantiate | Service worker | Connect | iroh dial | Site first byte | Total |
|---|---|---|---|---|---|---|---|---|
| laptop, cold | 260 (253–264) | 252 (248–261) | 5 (3–8) | 77 (70–104) | 626 (622–669) | 354 (345–392) | 280 (257–444) | 1631 (1625–1795) |
| laptop, return | 57 (51–59) | 54 (49–55) | 3 (2–4) | 0 (0–0) | 488 (479–494) | 342 (335–350) | 321 (318–325) | 1247 (1230–1289) |
| phone, cold | 3274 (3271–3336) | 3257 (3255–3315) | 17 (16–21) | 87 (82–96) | 836 (819–856) | 536 (529–547) | 1085 (1076–1088) | 6212 (6183–6263) |
| phone, return | 203 (200–204) | 184 (182–185) | 18 (18–21) | 0 (0–1) | 697 (686–726) | 528 (525–551) | 1106 (1104–1121) | 2884 (2873–2910) |

### Re-run

The same script, run again the same day by the reviewer (`node wasm-cost.mjs`, 5 runs):

| Case | WASM (fetch + compile) | Fetch | Compile + instantiate | Service worker | Connect | iroh dial | Site first byte | Total |
|---|---|---|---|---|---|---|---|---|
| laptop, cold | 263 (259–272) | 261 (250–267) | 6 (2–11) | 74 (71–114) | 622 (609–654) | 345 (336–380) | 449 (274–504) | 1805 (1607–1962) |
| laptop, return | 58 (53–62) | 53 (50–59) | 3 (3–6) | 0 (0–0) | 495 (493–507) | 348 (346–358) | 333 (315–483) | 1296 (1231–2360) |
| phone, cold | 3281 (3272–3485) | 3258 (3254–3470) | 17 (15–25) | 88 (85–98) | 843 (832–2128) | 534 (528–1617) | 1074 (1069–1084) | 6213 (6186–7933) |
| phone, return | 199 (196–202) | 179 (177–180) | 20 (18–23) | 1 (0–1) | 702 (694–709) | 531 (521–542) | 1103 (1095–1161) | 2921 (2866–3750) |

Within a few ms of the first, except one phone-like cold load whose iroh dial took 1,617 ms (an outlier, like one in an earlier traced run).

What the rows say:

- **The fetch is the cost, and it's uncompressed.** A cold visit transfers 3,126,687 bytes: `front.caddy` has no `encode`, so the Relay sends the WASM as is, not as the 1.0 MB gzip `spike/README.md` and #112 assume. On Fast 4G that is 3.1 s by itself.
- **The fetch starts late.** The WASM request starts only after `index.html` → `shell.js` → `web.js` arrive. That's at 738 ms on the phone-like browser (median, with the shell's first byte at 155 ms), and 340 ms on the laptop.
- **A return visit revalidates every Bootstrap Bundle file, one after another.** The Relay serves it with `Cache-Control: no-cache`, on purpose so upgrades reach phones at once. So every return visit gets a 304 for each file (transfer size 300 bytes, `deliveryType: "cache"`), each costing a round trip. The WASM's 304 is the 184 ms of the phone's return fetch, and `web.js`'s 304 comes before it (175 ms). Counted from `web.js`'s request, the glue and the WASM together take 382 ms on a phone-like return (106 ms on the laptop). A WebRTC-only shell would drop both files.
- **Compiling is cheap, because V8 compiles lazily.** Streaming compilation only validates and sets up: 3–21 ms after the last byte. Each function compiles when it's first called (`wasm.CompileLazy`, about 1,717 functions per load), on the main thread, during connect and the Site's load. A traced run (`TRACE=1`, 3 runs) adds that up to **31–52 ms on the laptop and 91–98 ms phone-like**, cold and return alike. Count it, and the WASM costs about 100 ms on a laptop return and about 300 ms on a phone-like return.
- **V8's code cache never kicks in.** No code cache hit or write (`v8.wasm.cachedModule`, `moduleCacheHit`, `compiledModule`) shows in any traced load, and a return visit compiles the same ~1,717 functions lazily again. Chrome caches a WASM module's code only after V8 has optimised (tiered up) enough of it, and a short visit tiers up 3–5 functions. A return visit gets the bytes from the HTTP cache but compiles them again. With lazy compilation, that recompile is the ~95 ms (phone-like) above.
- **The Direct Path doesn't come up in 800 ms phone-like.** Under CPU ×4 the shell waits its full 800 ms, then loads the Site through the Relay. That's why the phone's Site first byte is ~1.1 s against ~0.3 s on the laptop. This isn't a WASM cost (WebRTC-only would wait the same), but it's a third of a phone-like return visit.

### Connect times against `spike/README.md`

The spike measured "iroh connect via the Fly relay" at 503 ms (phone, probably on home Wi-Fi) and 546 ms (phone on cellular). That span is `Tooob.connect` alone (`spike/www/__tooob/shell.js`, `stats.connectMs`): no hello, no Invite. The like-for-like span here is the **iroh dial**, `Tooob.connect` plus reading the Visitor Key from IndexedDB: **342–354 ms on the laptop, about 30% under the spike's 503 ms**, and 528–536 ms phone-like, about the spike's 546 ms on cellular. So the spike's **~500 ms via Fly holds, and on the laptop today's dial is faster.** (The whole connect, with the hello, is 488 ms on a return visit and 626 ms on a cold one, which also uses the Invite: one more round trip and an Allowlist write.) The phone-like figure is only partly a phone's. CDP's network throttling covers the WebSocket's opening, but a real cellular path to Fly is what the Owner card is for.

### What the emulation doesn't cover

- CPU throttling slows only the page's main thread. V8's background threads (streaming validation, top-tier compiles) run at full laptop speed, so compile + instantiate is a floor. The lazy compiles run on the main thread, so they are throttled.
- The network profile covers the page's own requests: the Bootstrap Bundle and the Relay WebSocket's opening. It doesn't cover the service worker's own fetch of the shell on a return visit (one ~1 KB request), or WebRTC. The Direct Path here is to a Host on the same machine.
- An M4 slowed ×4 is still quicker than many phones. The [phone card](#a-real-phone) checks the real thing.

## The decision rule, applied

> The WASM (fetch + compile + instantiate) adds > 1 s to a cold phone-like first visit, or > 200 ms to a return visit: open an idea issue … Otherwise: "measured, not worth it".

- **Cold, phone-like: 3,274 ms** (3,271–3,336), more than 1 s. **Fires.**
- **Return, phone-like: 203 ms** (200–204; the re-run's median is 199, 196–202). **On the line, not clearly over it.** Measured as the rule words it (the WASM file's fetch + compile + instantiate), the return visit is within noise of 200 ms either way. It's clearly over only on a wider measure: counting the glue's own serial 304 before it (382 ms from `web.js`'s request), or the lazy compiles (~300 ms). On the laptop, return is 57 ms (~100 ms with lazy compiles), well under.
- The rule is an "or", so **the verdict rests on the cold case**, which is over its line three times.

So, per the rule: open the idea issue "the browser's own WebRTC plus TURN in place of iroh", with these numbers. But most of both costs is how the Bootstrap Bundle is served, not the work of running the WASM. Sending it uncompressed costs about 2.3 s of the cold 3.3 s, and the revalidation round trip is 184 ms of the return's 203 ms. The fixes below come first and are much smaller than swapping protocols. The idea issue should be grilled against the numbers after them.

## Cheaper fixes first

Estimates from the numbers above, at Fast 4G's 1,012,500 bytes/s. None is built or measured here.

1. **Compress the Bootstrap Bundle on the Relay.** `web_bg.wasm` is 3,126,387 bytes: 1,049,060 gzipped (`-9`), 843,169 with zstd (`-19`), 794,877 with brotli (`-q 11`). Caddy's `encode zstd gzip` in `front.caddy` compresses on the fly. Shipping `.br`/`.gz` files in the Bootstrap Bundle and serving them with `file_server { precompressed br gzip }` gets brotli's size with no CPU on the Relay. Cold phone-like fetch: **3.26 s → ~0.95 s (brotli) or ~1.2 s (gzip)**. Filed as [#113](https://github.com/Tooobs/tooob/issues/113).
2. **Preload the WASM and its glue from `index.html`.** `<link rel="modulepreload" href="/__tooob/shell.js">`, the same for `pkg/web.js`, and `<link rel="preload" href="/__tooob/pkg/web_bg.wasm" as="fetch" type="application/wasm" crossorigin>`. That starts the fetch at the shell's first byte (~155 ms) instead of ~738 ms on a cold phone (**−~580 ms**), and on a return visit makes the three revalidations parallel instead of serial (**−~350 ms**).
3. **Stop revalidating on every return visit.** Either content-hashed Bootstrap Bundle file names with `Cache-Control: immutable`, or the service worker serving `/__tooob/` from Cache Storage and checking `version.json` in the background. Either one keeps the instant upgrades `front.caddy` wants `no-cache` for. Return phone-like fetch: **184 ms → ~0**, which leaves the WASM at ~20 ms plus ~95 ms of lazy compiles, about 115 ms: under the line.
4. **A smaller build** (`opt-level = "z"`, `wasm-opt -Oz`): fewer bytes and fewer functions to compile lazily. Not measured. `wasm-opt` isn't installed here.

With 1 and 2, a cold phone-like visit would wait about 1 s for the WASM, mostly overlapped with loading `shell.js`. With 3, a return visit would wait about 0.1 s.

## Unverified

- **The emulation's gaps**, listed [above](#what-the-emulation-doesnt-cover): background compile threads at full speed, the service worker's own fetch and WebRTC unthrottled, an M4 ×4 quicker than many phones. None of the phone-like rows is a real phone's; the Owner card below is.
- **Every saving under [Cheaper fixes first](#cheaper-fixes-first)** is an estimate from sizes and round trips, not built or measured: brotli's ~0.95 s, preloading's −580 ms and −350 ms, the ~115 ms return visit without revalidation, and whatever a smaller build saves.
- **Why the code cache never kicks in** (too little tier-up in a short visit) is Chrome's documented heuristic as we understand it, not something traced to the line in V8.

## A real phone

The Owner card "#112 · the Bootstrap's WASM on a phone, over cellular" (`docs/agents/owner-tests.md`): open an Invite Link cold on a phone over cellular, then reopen the Key Address from a bookmark, noting tap-to-Site and the pill's numbers. Compare with the phone-like rows above.

## Re-running it

```sh
cd bootstrap/check
node --test wasm-cost-summary.test.mjs   # the summary's own tests
node wasm-cost.mjs                       # ~2.5 min: 5 runs × (laptop, phone-like) × (cold, return)
TRACE=1 RUNS=3 node wasm-cost.mjs        # also counts Chrome's WASM compile and code-cache trace events
```

- `RUNS` sets the runs per profile. `RELAY` sets the Relay (default `https://r.tooobs.net/`). `HEADED=1` opens a window you can watch.
- It prints a line per load and the table above, and writes every load's timeline to `test-results/wasm-cost.json`.
- It runs under the browser suite's guards: it takes their one-run lock (so never beside the suite), opens one tab at a time, records the Host it starts so nothing outlives it, and gives up on a load after 60 s and on the whole run after 15 minutes.
- It is not a `*.spec.mjs`, so the browser suite never runs it.
- The numbers move with the Relay's Bootstrap Bundle. After compression, preloading or a smaller build ships to r.tooobs.net, run it again and add the new table here.
