// How long the Bootstrap's WASM (web_bg.wasm) costs a Visitor (#112): a measurement, not a test,
// outside the browser suite (not a *.spec.mjs). Hosts run here through our own Relay
// (https://r.tooobs.net/, whose Bundle the browser loads), and Chromium opens their Invite Links:
// a cold first visit in a fresh profile, then, after the browser closes, a return visit to the Key
// Address with the service worker installed, in that same profile. Each in two profiles: a laptop
// (unthrottled) and phone-like (CPU throttled ×4, DevTools' "Fast 4G" network). Findings:
// docs/research/bootstrap-wasm-cost.md.
//
// Run: cd bootstrap/check && node wasm-cost.mjs   (RUNS=5 a profile; RELAY=https://r.tooobs.net/;
// HEADED=1 to watch; TRACE=1 also adds up Chrome's trace events for compiling WASM and its code
// cache.) Prints a table of medians and spread and writes every load to test-results/wasm-cost.json.
//
// Under the browser suite's guards: it takes their one-run lock, so it never runs beside the suite;
// it opens one tab at a time; every Host it starts goes in their process record and is stopped when
// it ends, however it ends; a load may take 60 s and the whole run 15 minutes.
import { chromium } from '@playwright/test';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { createServer } from 'node:net';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import stopLeftovers from './guards.mjs';
import * as h from './harness.mjs';
import { LOCK_PORT } from './machine.mjs';
import { phasesOf, table } from './wasm-cost-summary.mjs';

const RELAY = process.env.RELAY || 'https://r.tooobs.net/';
const RUNS = Number(process.env.RUNS) || 5;
const TRACE = process.env.TRACE === '1';
const LOAD_MS = 60_000;
const RUN_MS = 15 * 60_000;

// DevTools' "Fast 4G" preset (Chrome's front_end/core/sdk/NetworkManager.ts), and its ×4 CPU.
const PHONE = {
  cpu: 4,
  network: { offline: false, latency: 60 * 2.75, downloadThroughput: ((9 * 1000 * 1000) / 8) * 0.9, uploadThroughput: ((1.5 * 1000 * 1000) / 8) * 0.9 },
};
const PROFILES = { laptop: null, phone: PHONE };

/** The browser suite's one-run lock (guards.mjs), held until this process exits. */
async function lock() {
  const server = createServer((socket) => socket.on('error', () => {}).end(String(process.pid)));
  await new Promise((resolve, reject) => {
    server.once('error', (e) => reject(e.code === 'EADDRINUSE' ? new Error('a browser suite run (or another measurement) is going: one run at a time') : e));
    server.listen(LOCK_PORT, '127.0.0.1', resolve);
  });
  server.unref();
}

/**
 * Runs in every frame before its own scripts; only the shell's (the top frame) is measured. Marks
 * when WebAssembly.instantiateStreaming / instantiate start and end (shell.js's glue calls them from
 * the page itself: no worker and not the service worker), and when the shell first records each of
 * its own timings in window.__tooobStats: wasmMs (init() done), swMs, connectMs (iroh connected and
 * the hello answered), pageMs; and when it first signs (the hello, once iroh has connected).
 */
function instrument() {
  if (window !== window.top) return;
  const mark = (name) => performance.mark('wasm-cost:' + name);
  for (const name of ['instantiateStreaming', 'instantiate']) {
    const real = WebAssembly[name];
    WebAssembly[name] = async function (...args) {
      mark(name + ':start');
      try {
        return await real.apply(this, args);
      } finally {
        mark(name + ':end');
      }
    };
  }
  // The hello is the first thing the shell signs, straight after iroh connects to the Host.
  const sign = SubtleCrypto.prototype.sign;
  SubtleCrypto.prototype.sign = function (...args) {
    if (!performance.getEntriesByName('wasm-cost:dialed').length) mark('dialed');
    return sign.apply(this, args);
  };
  let stats;
  Object.defineProperty(window, '__tooobStats', {
    configurable: true,
    get: () => stats,
    set(value) {
      stats = value;
      for (const key of ['wasmMs', 'swMs', 'connectMs', 'pageMs']) {
        let v;
        Object.defineProperty(value, key, {
          configurable: true,
          enumerable: true,
          get: () => v,
          set(x) {
            if (v === undefined) mark(key);
            v = x;
          },
        });
      }
    },
  });
}

/** Where everything happened, in ms from the shell's navigation start, once the Site has loaded. */
function timeline() {
  const mark = (name) => performance.getEntriesByName('wasm-cost:' + name)[0]?.startTime ?? null;
  const resource = (end) => performance.getEntriesByType('resource').find((e) => new URL(e.name).pathname.endsWith(end));
  const wasm = resource('/web_bg.wasm');
  const glue = resource('/pkg/web.js'); // wasm-bindgen's glue, which shell.js imports and which fetches the WASM
  const nav = performance.getEntriesByType('navigation')[0];
  const site = document.querySelector('iframe').contentWindow.performance;
  const siteNav = site.getEntriesByType('navigation')[0];
  const instantiate = mark('instantiateStreaming:end') === null ? 'instantiate' : 'instantiateStreaming';
  return {
    shellFirstByte: nav.responseStart,
    glueStart: glue.startTime,
    glueEnd: glue.responseEnd,
    wasmStart: wasm.startTime,
    wasmFirstByte: wasm.responseStart,
    wasmEnd: wasm.responseEnd,
    wasmTransferSize: wasm.transferSize, // 0: from the HTTP cache unasked; a few hundred bytes: a 304
    wasmBodySize: wasm.encodedBodySize,
    wasmDeliveryType: wasm.deliveryType ?? null,
    wasmViaServiceWorker: wasm.workerStart > 0,
    instantiateCalled: instantiate,
    instantiateStart: mark(instantiate + ':start'),
    instantiateEnd: mark(instantiate + ':end'),
    wasmReady: mark('wasmMs'),
    swReady: mark('swMs'),
    dialed: mark('dialed'),
    connected: mark('connectMs'),
    siteFirstByte: site.timeOrigin + siteNav.responseStart - performance.timeOrigin,
    siteLoaded: mark('pageMs'),
    path: window.__tooobStats.path ?? null,
    shell: { ...window.__tooobStats, bytes: undefined, sent: undefined },
  };
}

/**
 * Chrome's trace events about compiling WASM, by name: how many of each and their time together, in
 * ms (wasm.CompileLazy: functions compiled on their first call, on the page's main thread; a code
 * cache hit or write would show as v8.wasm.cachedModule / moduleCacheHit / compiledModule).
 */
async function wasmTraceEvents(cdp) {
  const done = new Promise((r) => cdp.once('Tracing.tracingComplete', r));
  await cdp.send('Tracing.end');
  const { stream } = await done;
  let json = '';
  for (;;) {
    const { data, eof, base64Encoded } = await cdp.send('IO.read', { handle: stream });
    json += base64Encoded ? Buffer.from(data, 'base64').toString() : data;
    if (eof) break;
  }
  await cdp.send('IO.close', { handle: stream });
  const counts = {};
  for (const e of JSON.parse(json).traceEvents ?? []) {
    if (!/wasm/i.test(e.name) || !/cache|compil|deserial|stream/i.test(e.name)) continue;
    const c = (counts[e.name] ??= { n: 0, us: 0 });
    c.n++;
    c.us += e.dur ?? 0;
  }
  return Object.fromEntries(Object.entries(counts).map(([name, c]) => [name, { n: c.n, ms: Math.round(c.us / 1000) }]));
}

/** Open `url` in a browser on `profileDir` (as `profile` throttles it) and time it to the Site. */
async function load(profileDir, url, profile) {
  const context = await chromium.launchPersistentContext(profileDir, { channel: 'chromium', headless: process.env.HEADED !== '1' });
  try {
    await context.addInitScript(instrument);
    const page = context.pages()[0] ?? (await context.newPage());
    const cdp = await context.newCDPSession(page);
    if (profile) {
      await cdp.send('Emulation.setCPUThrottlingRate', { rate: profile.cpu });
      await cdp.send('Network.enable');
      await cdp.send('Network.emulateNetworkConditions', profile.network);
    }
    if (TRACE) await cdp.send('Tracing.start', { traceConfig: { includedCategories: ['v8.wasm', 'disabled-by-default-v8.wasm.detailed', 'devtools.timeline', 'disabled-by-default-devtools.timeline'] }, transferMode: 'ReturnAsStream' });
    const measured = (async () => {
      await page.goto(url);
      await h.siteText(page, LOAD_MS);
      const t = await page.evaluate(timeline);
      if (TRACE) t.trace = await wasmTraceEvents(cdp);
      // Give Chrome a moment to write its caches (and its WASM code cache) before the browser closes.
      await page.waitForTimeout(3000);
      return t;
    })();
    let timer;
    const late = new Promise((_, fail) => (timer = setTimeout(() => fail(new Error(`no Site within ${LOAD_MS / 1000} s: ${url}`)), LOAD_MS + 5000)));
    return await Promise.race([measured, late]).finally(() => clearTimeout(timer));
  } finally {
    await context.close();
  }
}

async function main() {
  await lock();
  const stopRecorded = await stopLeftovers(); // what a killed run left, then what this one starts
  const giveUp = setTimeout(() => {
    console.error(`wasm-cost: past ${RUN_MS / 60000} minutes, stopping`);
    finish(1);
  }, RUN_MS);
  let finishing = false;
  async function finish(code) {
    if (finishing) return;
    finishing = true;
    clearTimeout(giveUp);
    h.stopAll();
    await stopRecorded();
    process.exit(code);
  }
  process.on('SIGINT', () => finish(130)).on('SIGTERM', () => finish(143));
  try {
    await h.buildHost();
    const site = await h.startSite();
    const host = await h.startHost(site, RELAY);
    const runs = [];
    for (let i = 1; i <= RUNS; i++) {
      for (const [name, profile] of Object.entries(PROFILES)) {
        const dir = mkdtempSync(join(tmpdir(), 'wasm-cost-profile-'));
        try {
          const link = h.tooob(['invite', host.name, `${name}-${i}`]);
          const cold = await load(dir, link, profile);
          const back = await load(dir, new URL(link).origin + '/', profile);
          for (const [visit, t] of [['cold', cold], ['return', back]]) {
            const p = phasesOf(t);
            runs.push({ profile: name, visit, run: i, timeline: t, phases: p });
            console.log(`${name} ${visit} #${i}: wasm ${p.wasm} ms (fetch ${p.fetch}, compile+instantiate ${p.compile}) · connect ${p.connect} (dial ${p.dial}) · site first byte ${p.siteFirstByte} · ${t.path}${t.trace ? ' · ' + JSON.stringify(t.trace) : ''}`);
          }
        } finally {
          rmSync(dir, { recursive: true, force: true });
        }
      }
    }
    mkdirSync(new URL('test-results', import.meta.url), { recursive: true });
    writeFileSync(new URL('test-results/wasm-cost.json', import.meta.url), JSON.stringify({ relay: RELAY, phone: PHONE, runs }, null, 2));
    console.log('\n' + table(runs));
    await finish(0);
  } catch (e) {
    console.error(e);
    await finish(1);
  }
}

await main();
