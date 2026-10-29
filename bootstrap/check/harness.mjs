// The real pieces on this machine, on free ports: a Relay (stock iroh-relay behind a front that also
// serves the Bootstrap, www/), a stand-in Site, and Hosts (`tooob add` + `tooob run`) from host/.
// Playwright's Chromium then opens real Invite Links, on Key Addresses: `<host key>.localhost`, which
// Chromium treats as a secure context and resolves to this machine. The Hosts and Relays started
// here go in the run's process record (guards.mjs), so none outlives a run.
import { execFileSync, spawn } from 'node:child_process';
import { existsSync, mkdtempSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { createServer, request } from 'node:http';
import { connect } from 'node:net';
import { tmpdir } from 'node:os';
import { extname, join } from 'node:path';
import { recordProcess } from './guards.mjs';

const ROOT = new URL('../..', import.meta.url).pathname;
// TOOOB_BIN and TOOOB_WWW swap in another Release's tooob or Bootstrap Bundle: release.sh's
// cross-Release check (ADR 0017).
const TOOOB = process.env.TOOOB_BIN || join(ROOT, 'target/debug/tooob');
const children = [];
const servers = [];
const sockets = []; // proxied connections: a server can't close while they're open

/** Build the tooob the Hosts run (unless TOOOB_BIN names one), without blocking: time limits stay live. */
export async function buildHost() {
  if (process.env.TOOOB_BIN) return;
  // In a process group of its own, so a run that ends mid-build stops it and the compilers it started,
  // or they keep the build folder locked.
  const cargo = spawn('cargo', ['build', '-q', '--manifest-path', join(ROOT, 'host/Cargo.toml')], { stdio: ['ignore', 'inherit', 'inherit'], detached: true });
  children.push(cargo);
  recordProcess(cargo, { group: true });
  const status = await new Promise((resolve, reject) => cargo.on('error', reject).on('exit', resolve));
  if (status !== 0) throw new Error(`cargo build failed (${status})`);
}

function listen(server) {
  servers.push(server);
  return new Promise((resolve) => server.listen(0, '127.0.0.1', () => resolve(server.address().port)));
}

async function freePort() {
  const server = createServer();
  const port = await new Promise((resolve) => server.listen(0, '127.0.0.1', () => resolve(server.address().port)));
  await new Promise((r) => server.close(r));
  return port;
}

async function until(check, what, ms = 15000) {
  const start = Date.now();
  for (;;) {
    if (await check().catch(() => false)) return;
    if (Date.now() - start > ms) throw new Error(`timed out waiting for ${what}`);
    await new Promise((r) => setTimeout(r, 100));
  }
}

function start(cmd, args, env = process.env) {
  const child = spawn(cmd, args, { stdio: ['ignore', 'pipe', 'pipe'], env });
  let out = '';
  child.stdout.on('data', (d) => (out += d));
  child.stderr.on('data', (d) => (out += d));
  children.push(child);
  recordProcess(child);
  return { child, output: () => out };
}

export async function startRelay() {
  const port = await freePort();
  const config = join(mkdtempSync(join(tmpdir(), 'relay-')), 'relay.toml');
  writeFileSync(config, `http_bind_addr = "127.0.0.1:${port}"\nenable_metrics = false\n`);
  start('iroh-relay', ['--dev', '--config-path', config]);
  const url = `http://127.0.0.1:${port}/`;
  await until(async () => (await fetch(url + 'healthz')).ok, 'the relay');
  return url;
}

const TYPES = { '.html': 'text/html', '.js': 'text/javascript', '.wasm': 'application/wasm', '.json': 'application/json' };
const IROH_PATHS = ['/relay', '/ping', '/generate_204', '/healthz'];

/**
 * A Relay as a Relay Operator runs one (ADR 0020): one web server that hands iroh's paths to
 * iroh-relay at `relayUrl` and serves the Bootstrap (www/) for everything else, on its own domain
 * (`domain`) and every Key Address under it. `url` is what Hosts name as their Relay. Chrome
 * won't set a cookie for all of `localhost`, so a test of cookies set for a whole Relay domain
 * needs one a label longer, such as `relay.localhost`.
 * `silence()` makes every Relay connection open so far go quiet both ways while staying open, as a
 * phone's network does to the flows it had when it comes out of deep sleep; connections made
 * afterwards work. `asked` is every path it was asked for other than iroh's, in order.
 */
export async function startRelayFront(relayUrl, domain = 'localhost') {
  const relay = new URL(relayUrl);
  const www = process.env.TOOOB_WWW || join(ROOT, 'bootstrap/www');
  const open = new Set();
  const asked = [];
  const server = createServer((req, res) => {
    // As the Relay image's front.caddy does (ADR 0021): no Cookie goes on, and the Bundle asks
    // browsers for a process of its own.
    delete req.headers.cookie;
    const path = new URL(req.url, 'http://x').pathname;
    if (IROH_PATHS.includes(path)) {
      const upstream = request({ host: relay.hostname, port: relay.port, path: req.url, method: req.method, headers: req.headers }, (r) => {
        res.writeHead(r.statusCode, r.headers);
        r.pipe(res);
      });
      upstream.on('error', () => res.destroy());
      return req.pipe(upstream);
    }
    asked.push(path);
    // Any other path is a page of the Site's, opened before the service worker is installed: the
    // shell, which opens it (ADR 0023). A missing Bundle file is still missing.
    const bundleFile = path.startsWith('/__tooob/') || path === '/tooob-sw.js';
    const file = path.endsWith('/') ? join(path, 'index.html') : path;
    const page = bundleFile ? file : existsSync(join(www, file)) && statSync(join(www, file)).isFile() ? file : '/index.html';
    try {
      const body = readFileSync(join(www, page));
      res.setHeader('content-type', TYPES[extname(page)] ?? 'text/html');
      res.setHeader('cache-control', 'no-store');
      res.setHeader('origin-agent-cluster', '?1');
      res.setHeader('cross-origin-opener-policy', 'same-origin-allow-popups');
      res.end(body);
    } catch {
      res.statusCode = 404;
      res.end();
    }
  });
  // iroh's Relay connection is a WebSocket: once upgraded, just bytes both ways.
  server.on('upgrade', (req, client, head) => {
    const upstream = connect(Number(relay.port), relay.hostname);
    const pair = { quiet: false };
    open.add(pair);
    sockets.push(client, upstream);
    const lines = [`${req.method} ${req.url} HTTP/1.1`];
    for (let i = 0; i < req.rawHeaders.length; i += 2) {
      if (req.rawHeaders[i].toLowerCase() !== 'cookie') lines.push(`${req.rawHeaders[i]}: ${req.rawHeaders[i + 1]}`);
    }
    upstream.write(lines.join('\r\n') + '\r\n\r\n');
    upstream.write(head);
    client.on('data', (d) => pair.quiet || upstream.write(d));
    upstream.on('data', (d) => pair.quiet || client.write(d));
    const end = () => (open.delete(pair), client.destroy(), upstream.destroy());
    client.on('close', end).on('error', end);
    upstream.on('close', end).on('error', end);
  });
  const port = await listen(server);
  return {
    url: `http://${domain}:${port}/`,
    port,
    asked,
    silence: () => open.forEach((pair) => (pair.quiet = true)),
  };
}

/**
 * Tell the shell its page was hidden and is visible again, as when a phone wakes. Only the news
 * is forged: whatever the shell does about it runs for real.
 */
export async function wake(page) {
  await page.evaluate(() => {
    for (const state of ['hidden', 'visible']) {
      Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => state });
      document.dispatchEvent(new Event('visibilitychange'));
    }
    delete document.visibilityState;
  });
}

/** A stand-in Site: answers every path with what reached it, and counts requests. */
export async function startSite() {
  const site = { requests: 0 };
  const server = createServer((req, res) => {
    site.requests++;
    const size = /^\/bytes\/(\d+)/.exec(req.url)?.[1]; // a big response, like a photo
    if (size) return res.end(Buffer.alloc(Number(size), 7));
    if (req.method === 'POST') { // an upload: say how much arrived
      let received = 0;
      req.on('data', (c) => (received += c.length));
      return req.on('end', () => res.end(`received ${received}`));
    }
    res.setHeader('content-type', 'text/html');
    res.end(`<!doctype html><p id="site">site ${req.url} label=${req.headers['x-tooob-label'] ?? '-'}</p>`);
  });
  site.addr = `127.0.0.1:${await listen(server)}`;
  return site;
}

// Every Host made here lives under a TOOOB_HOME of its own, never the Owner's real one.
const HOME = mkdtempSync(join(tmpdir(), 'tooob-home-'));
const ENV = { ...process.env, TOOOB_HOME: HOME };
let hosts = 0;

/** A Host for a Site's web app, through a Relay, running; `name` runs one made earlier again. */
export async function startHost(site, relay, name) {
  if (!name) {
    name = `site-${++hosts}`;
    tooob(['add', name, site.addr, '--relay', relay]);
  }
  return runHost(name);
}

/** A Host sharing PATH (a Shared Folder, or a single file), through a Relay, running. */
export async function startShare(path, relay) {
  const name = `share-${++hosts}`;
  tooob(['add', name, path, '--relay', relay]);
  return runHost(name);
}

/** `tooob run NAME`, once it's serving. */
async function runHost(name) {
  const host = start(TOOOB, ['run', name], ENV);
  await until(async () => host.output().includes('serving'), `tooob run ${name}`);
  return {
    name,
    stop: () => new Promise((r) => { host.child.once('exit', r); host.child.kill(); }),
    // Freeze the Host: nothing it holds answers, so the browser's ICE checks go unanswered.
    pause: () => host.child.kill('SIGSTOP'),
    resume: () => host.child.kill('SIGCONT'),
  };
}

export function tooob(args) {
  return execFileSync(TOOOB, args, { encoding: 'utf8', env: ENV }).trim();
}

/** A fresh browser profile: its own storage, so its own Visitor Key. */
export async function newVisitor(browser) {
  const context = await browser.newContext();
  return context.newPage();
}

/**
 * Downloads in `page`, from now on. `next()` waits for the next one to finish: the name the browser
 * saved it under, and its bytes.
 */
export function watchDownloads(page) {
  const started = [];
  page.on('download', (download) => started.push(download));
  const nextStarted = (ms) => (started.length ? Promise.resolve(started.shift()) : page.waitForEvent('download', { timeout: ms }).then(() => started.shift()));
  return {
    next: async (ms = 20000) => {
      const download = await nextStarted(ms);
      return { name: download.suggestedFilename(), bytes: readFileSync(await download.path()) };
    },
  };
}

/** The status line the shell shows the Visitor. */
export async function statusText(page) {
  return page.$eval('#status', (el) => el.textContent);
}

/** Wait until the status line matches `re`. */
export async function pillSays(page, re, ms) {
  await page.waitForFunction((src) => new RegExp(src).test(document.getElementById('status').textContent), re.source, { timeout: ms });
}

/** Wait until the shell says the Site comes by `path`: 'direct' (the Direct Path) or 'relay'. */
export async function pathIs(page, path, ms) {
  await page.waitForFunction((path) => window.__tooobStats.path === path, path, { timeout: ms });
}

/** Wait until the Site has loaded in the shell's iframe; return its text. */
export async function siteText(page, ms = 20000) {
  const handle = await page.waitForSelector('iframe', { timeout: ms });
  const frame = await handle.contentFrame();
  await frame.waitForSelector('#site', { timeout: ms }).catch(async (e) => {
    const inner = await frame.evaluate(() => document.documentElement.outerHTML.slice(0, 300)).catch((x) => String(x));
    console.log('the Site never loaded. shell:', await statusText(page), '| iframe:', inner);
    throw e;
  });
  return frame.$eval('#site', (el) => el.textContent);
}

/**
 * What a script of the Site's own can find in the storage it shares with the shell (same origin):
 * localStorage, cache names, the private keys in IndexedDB (by algorithm), and the formats any of
 * them exported to.
 */
export async function whatTheSiteCanReach(page) {
  const frame = await (await page.$('iframe')).contentFrame();
  return frame.evaluate(async () => {
    const records = [];
    for (const { name } of await indexedDB.databases()) {
      const db = await new Promise((ok, fail) => Object.assign(indexedDB.open(name), { onsuccess: (e) => ok(e.target.result), onerror: fail }));
      for (const store of db.objectStoreNames) {
        const all = db.transaction(store).objectStore(store).getAll();
        records.push(...(await new Promise((ok) => (all.onsuccess = () => ok(all.result)))));
      }
      db.close();
    }
    const keys = records.flatMap((r) => (r instanceof CryptoKey ? [r] : Object.values(r ?? {}).filter((v) => v instanceof CryptoKey)));
    const privateKeys = keys.filter((k) => k.type === 'private');
    const exported = [];
    for (const k of privateKeys) {
      for (const format of ['pkcs8', 'jwk', 'raw']) {
        await crypto.subtle.exportKey(format, k).then(() => exported.push(format), () => {});
      }
    }
    return { localStorage: { ...localStorage }, caches: await caches.keys(), privateKeys: privateKeys.map((k) => k.algorithm.name), exported };
  });
}

/**
 * The phone sleeps, as with its screen off: the page stops (Chrome's debugger pauses its scripts and
 * its Site's), the network drops its Relay connection's flows (`front`, the page's own Relay front,
 * goes quiet), and nothing the page holds answers. After `seconds` it carries on and hears it's in
 * view again. Only the page sleeps: nothing signals any browser process. (Chrome's own freeze,
 * Page.setWebLifecycleState, never freezes a page Playwright drives: Playwright keeps every page
 * visible, and Chrome freezes only hidden ones.)
 */
export async function sleepPage(page, front, seconds) {
  const cdp = await page.context().newCDPSession(page);
  await cdp.send('Debugger.enable');
  await cdp.send('Debugger.pause');
  front.silence();
  try {
    await new Promise((r) => setTimeout(r, seconds * 1000));
  } finally {
    await cdp.detach(); // and the page carries on
  }
  await wake(page);
}

/** Fetch `path` from inside the Site's iframe, as a link on the Site would: "<status> <bytes>". */
export async function siteFetch(page, path) {
  const frame = await (await page.$('iframe')).contentFrame();
  return frame.evaluate(async (path) => {
    const r = await fetch(path);
    return `${r.status} ${(await r.arrayBuffer()).byteLength}`;
  }, path);
}

/**
 * Make the page's first `attempts` Direct Path attempts fail (all of them by default): the browser
 * gathers no ICE candidates.
 */
export async function breakDirectPath(page, attempts = Infinity) {
  await page.addInitScript((attempts) => {
    const Real = RTCPeerConnection;
    let made = 0;
    window.RTCPeerConnection = function (config) {
      return attempts === null || made++ < attempts ? new Real({ iceTransportPolicy: 'relay', iceServers: [] }) : new Real(config);
    };
  }, Number.isFinite(attempts) ? attempts : null); // Infinity doesn't survive the trip into the page
}

/** Keep hold of the page's peer connections, so failDirectPath can reach them. */
export async function watchDirectPath(page) {
  await page.addInitScript(() => {
    const Real = RTCPeerConnection;
    window.__peers = [];
    window.RTCPeerConnection = function (config) {
      const pc = new Real(config);
      window.__peers.push(pc);
      return pc;
    };
  });
}

/**
 * Tell the shell its Direct Path failed, as the browser does when the network changes under it.
 * Only the news is forged: the connection itself stays up, so whatever the shell does next runs
 * for real against the Host. Returns the path the shell says it's on straight after hearing the
 * news: read any later and its ICE restart, one round trip to the Host on a connection that never
 * really failed, may already have put it back on 'direct' (#30).
 */
export async function failDirectPath(page) {
  return page.evaluate(() => {
    const pc = window.__peers.at(-1);
    Object.defineProperty(pc, 'connectionState', { configurable: true, get: () => 'failed' });
    pc.dispatchEvent(new Event('connectionstatechange'));
    delete pc.connectionState;
    return window.__tooobStats.path;
  });
}

/**
 * Someone else answering on the Direct Path: the certificate it presents won't hash to the
 * fingerprint the Host sent over iroh. Simulated by changing that fingerprint as the browser
 * applies the answer.
 */
export async function forgeDirectFingerprint(page) {
  await page.addInitScript(() => {
    const real = RTCPeerConnection.prototype.setRemoteDescription;
    RTCPeerConnection.prototype.setRemoteDescription = function (desc) {
      const sdp = desc.sdp.replace(/(a=fingerprint:sha-256 )([0-9A-F]{2})/gi, (_, k, b) => k + (b === '00' ? '01' : '00'));
      return real.call(this, { type: desc.type, sdp });
    };
  });
}

/**
 * A Direct Path punched through NAT, as from a phone on 5G: the browser's stats say the Host's end
 * of every candidate pair is a STUN-discovered public address (a documentation one).
 */
export async function forgeNatPunched(page) {
  await page.addInitScript(() => {
    const real = RTCPeerConnection.prototype.getStats;
    RTCPeerConnection.prototype.getStats = async function (...args) {
      const report = await real.apply(this, args);
      const forged = (s) => (s.type === 'remote-candidate' ? { ...s, candidateType: 'srflx', address: '203.0.113.7' } : s);
      return new Map([...report].map(([id, s]) => [id, forged(s)]));
    };
  });
}

/** Reload the Site in the shell's iframe; the Site bytes each path carried meanwhile. */
export async function bytesForReload(page) {
  return page.evaluate(async () => {
    const s = window.__tooobStats;
    const before = { ...s.bytes };
    const f = document.querySelector('iframe');
    await new Promise((res) => { f.onload = res; f.contentWindow.location.href = '/?again=' + Date.now(); });
    return { direct: s.bytes.direct - before.direct, relay: s.bytes.relay - before.relay };
  });
}

export function stopAll() {
  for (const child of children) child.kill();
  for (const socket of sockets) socket.destroy();
  for (const server of servers) {
    server.close();
    server.closeAllConnections?.(); // a Relay front's keep-alive connections
  }
}
