// Shell page: opens an Invite Link, holds the iroh connection (WASM), and serves the Site's iframe
// over it. Link: https://<host key>.<relay domain>/#invite=<token>, the Host's Key Address: an
// origin of its own, so its storage and Visitor Key are walled off from every other Host's. The
// fragment never leaves the browser.
import init, { Tooob } from './pkg/web.js';

const enc = new TextEncoder();
const dec = new TextDecoder();
const statusEl = document.getElementById('status');
const stats = (window.__tooobStats = {});
const show = (s) => (statusEl.textContent = s);
const ms = (t) => Math.round(performance.now() - t);

const concat = (a, b) => {
  const out = new Uint8Array(a.length + b.length);
  out.set(a);
  out.set(b, a.length);
  return out;
};

// Read the JSON head line; returns [head, leftover bytes].
async function readHead(reader) {
  let buf = new Uint8Array(0);
  for (;;) {
    const c = await reader.read();
    if (!c) throw new Error('stream ended before response head');
    buf = concat(buf, c);
    const i = buf.indexOf(10);
    if (i >= 0) return [JSON.parse(dec.decode(buf.subarray(0, i))), buf.slice(i + 1)];
  }
}

// Lowercase hex: how iroh prints a key, so how the Host reads the Visitor Key in a hello.
const hex = (bytes) => [...bytes].map((b) => b.toString(16).padStart(2, '0')).join('');
const idbResult = (request) => new Promise((ok, fail) => ((request.onsuccess = () => ok(request.result)), (request.onerror = () => fail(request.error))));

// The Visitor Key: one per browser, reused on every visit, as {privateKey, public} (public in hex).
// A WebCrypto key that can't be exported, kept in IndexedDB (ADR 0011).
// ponytail: the Site's own scripts share this origin, so while they run they can sign hellos with
// it (for this Host only), just never read it out. Give the shell an origin of its own to stop that.
async function visitorKey() {
  const open = indexedDB.open('tooob');
  open.onupgradeneeded = () => open.result.createObjectStore('keys');
  const db = await idbResult(open);
  const keys = (mode) => db.transaction('keys', mode).objectStore('keys');
  try {
    let key = await idbResult(keys().get('visitor'));
    if (!key) {
      const old = localStorage.tooobVisitorKey;
      key = old ? await fromLocalStorage(old) : await newVisitorKey();
      // Another tab may have stored one first: then that one is this browser's.
      await idbResult(keys('readwrite').add(key, 'visitor')).catch(async (e) => {
        if (e?.name !== 'ConstraintError') throw e;
        key = await idbResult(keys().get('visitor'));
      });
    }
    localStorage.removeItem('tooobVisitorKey'); // only once the key store has it
    return key;
  } finally {
    db.close();
  }
}

async function newVisitorKey() {
  const pair = await crypto.subtle.generateKey('Ed25519', false, ['sign']);
  return { privateKey: pair.privateKey, public: hex(new Uint8Array(await crypto.subtle.exportKey('raw', pair.publicKey))) };
}

// Shells before the key store kept the Visitor Key's 32-byte secret in localStorage, as hex. Move
// it into the key store as is, so the Host still knows this browser. WebCrypto won't derive a public
// key, so import it once as exportable to read the public half, then keep a copy that isn't.
const ED25519_PKCS8_PREFIX = '302e020100300506032b657004220420'; // RFC 8410's wrapper around a 32-byte secret
async function fromLocalStorage(old) {
  const pkcs8 = Uint8Array.from((ED25519_PKCS8_PREFIX + old).match(/../g), (h) => parseInt(h, 16));
  const jwk = await crypto.subtle.exportKey('jwk', await crypto.subtle.importKey('pkcs8', pkcs8, 'Ed25519', true, ['sign']));
  const publicKey = Uint8Array.from(atob(jwk.x.replace(/-/g, '+').replace(/_/g, '/')), (c) => c.charCodeAt(0));
  return { privateKey: await crypto.subtle.importKey('jwk', jwk, 'Ed25519', false, ['sign']), public: hex(publicKey) };
}

// The Host Key is the hostname's first label; the Relay is the rest of it, on the same port.
const [hostKey, ...domain] = location.hostname.split('.');
const relay = `${location.protocol}//${domain.join('.')}${location.port ? ':' + location.port : ''}/`;
const invite = new URLSearchParams(location.hash.slice(1)).get('invite');
if (!domain.length || !/^[ybndrfg8ejkmcpqxot1uwisza345h769]{52}$/.test(hostKey)) {
  show('This link is incomplete. Ask for a new Invite Link.');
  throw new Error('no Host Key');
}

let t = performance.now();
show('loading…');
await init();
stats.wasmMs = ms(t);

// Already controlled (any repeat visit): don't wait. register() on a warm reload took ~2 s in Chrome,
// so just kick off the update check in the background.
t = performance.now();
if (navigator.serviceWorker.controller) {
  navigator.serviceWorker.register('/tooob-sw.js');
} else {
  await navigator.serviceWorker.register('/tooob-sw.js');
  await navigator.serviceWorker.ready;
  if (!navigator.serviceWorker.controller) {
    await new Promise((r) => navigator.serviceWorker.addEventListener('controllerchange', r, { once: true }));
  }
}
stats.swMs = ms(t);

t = performance.now();
show('connecting…');
const UNREACHABLE = "Couldn't reach this site: its host may be offline. Reload this page to try again.";
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
// A browser without Ed25519 in WebCrypto or without IndexedDB (some private modes) can't keep one.
const visitor = await visitorKey().catch((e) => {
  show("This browser can't keep this device's key, so it can't open the site. Try an up-to-date browser, outside private browsing.");
  throw e;
});
let tooob;

/** One request/reply exchange: a JSON line each way. */
async function ask(message, conn = tooob) {
  const bi = await conn.open();
  const writer = bi.writer();
  await writer.write(enc.encode(JSON.stringify(message) + '\n'));
  writer.finish();
  const [reply] = await readHead(bi.reader());
  return reply;
}

/** Connect, then say hello: prove this browser's Visitor Key for the new connection (ADR 0011). */
async function connect() {
  const conn = await Tooob.connect(hostKey, relay);
  const sig = new Uint8Array(await crypto.subtle.sign('Ed25519', visitor.privateKey, conn.hello_message()));
  const hello = { t: 'hello', visitor: visitor.public, sig: Array.from(sig) };
  const reply = await ask(hello, conn).catch((e) => ({ error: String(e) }));
  if (reply.status !== 200) {
    conn.close();
    throw new Error(`hello refused: ${JSON.stringify(reply)}`);
  }
  return conn;
}

// A stream is {via, writer: {write, finish}, reader: {read, free}}, on the Relayed Path (the iroh
// connection; browsers are relay-only) or on the Direct Path once it's up.
const viaRelay = async () => {
  const bi = await openRelay();
  return { via: 'relay', writer: bi.writer(), reader: bi.reader() };
};

// The iroh connection is lost when the Host stops or restarts, or the device sleeps past iroh's
// idle timeout (~30 s). Then dial again and say hello, once for everything waiting on it: five
// tries, backing off, saying so on the pill. Streams wait for it; once it gives up they fail at
// once, and the pill offers a reload.
const BACKOFF = [0, 2, 4, 8, 16]; // seconds before each try
const TRY_MS = 5000; // a dial to a Host that's gone only fails after ~30 s
let reconnecting = null;
let lost = null; // while reconnecting: {n: the try, next: seconds until the one after}
let gaveUp = false; // until the page comes back into view: the network may be back by then
const cantReach = () => new Error("can't reach the site");

function reconnect(conn) {
  if (gaveUp) return Promise.reject(cantReach());
  if (tooob === conn) reconnecting ??= dialAgain(conn).finally(() => (reconnecting = null));
  return reconnecting;
}

async function dialAgain(conn) {
  conn.close(); // nothing else should wait on it
  dropDirect(); // the Host closes a Direct Path with its connection
  for (let n = 1; ; n++) {
    lost = { n };
    pill();
    try {
      tooob = await dial(TRY_MS);
      watch(tooob);
      lost = null;
      pill();
      directRetries = 0;
      goDirect().catch(() => {});
      return;
    } catch {}
    if (n === BACKOFF.length) {
      lost = null;
      gaveUp = true;
      pill();
      throw cantReach();
    }
    for (let s = BACKOFF[n]; s > 0; s--) {
      lost = { n, next: s };
      pill();
      await sleep(1000);
    }
  }
}

// connect(), giving up after `ms`; a connection that comes later is closed.
function dial(ms) {
  let late = false;
  const attempt = connect().then((conn) => (late ? conn.close() : conn));
  return Promise.race([attempt, sleep(ms).then(() => ((late = true), Promise.reject(new Error('timed out'))))]);
}

// Hear at once when the Host closes the connection (it does when it stops).
const watch = (conn) => conn.closed().then(() => reconnect(conn)).catch(() => {});

// Waking from deep sleep (Firefox for Android in Doze), iroh's timers were frozen with the page: it
// still thinks the connection is up while the network has dropped it, so a request would wait out
// the idle timeout (~30 s). So on coming back into view, check the Host answers, and if it doesn't,
// drop the connection and dial again. Streams wait for the check. If the shell gave up while the
// page was out of view, it tries again.
let checking = null;
function checkOnWake() {
  if (document.visibilityState !== 'visible' || checking || reconnecting) return;
  const conn = tooob;
  if (gaveUp) {
    gaveUp = false;
    return reconnect(conn).catch(() => {});
  }
  checking = Promise.race([ask({ t: 'standing' }, conn).catch(() => undefined), sleep(4000)])
    .then((reply) => reply?.standing || reconnect(conn))
    .catch(() => {}) // gave up: the pill says so
    .finally(() => (checking = null));
}

async function openRelay() {
  await checking;
  await reconnecting;
  const conn = tooob;
  try {
    return await conn.open();
  } catch {
    await reconnect(conn);
    return tooob.open();
  }
}

let direct = null; // opens a stream on the Direct Path while it's up
const openStream = () => (direct ? direct().catch(viaRelay) : viaRelay());
stats.bytes = { relay: 0, direct: 0 };

// What the Host says when it refuses an Invite, in words for the person holding the link.
const REFUSED = {
  already_used: 'This invite has already been used. Ask for a new one.',
  expired: 'This invite has expired. Ask for a new one.',
  unknown_invite: "This invite isn't valid (it may have been revoked). Ask for a new one.",
  blocked: "This device doesn't have access.",
};

// Use the Invite (once), then drop the token from the address bar: the used link shouldn't linger in
// history or bookmarks, and the plain link keeps working for this device (its Visitor Key is kept).
// A refused Invite isn't the end: this device may already be in (Mom re-tapping her link), so the
// standing check below decides, and the refusal only explains a "no". If the Host can't be reached,
// the token stays, so a reload retries.
let refusal;
try {
  tooob = await connect();
  if (invite) {
    show('opening your invite…');
    const reply = await ask({ t: 'redeem', token: invite });
    if (!reply.redeemed && reply.error !== 'already_admitted') {
      refusal = REFUSED[reply.error] ?? "This invite can't be used right now. Try again later.";
    }
    history.replaceState(null, '', location.pathname + location.search);
  }
} catch (e) {
  show(UNREACHABLE);
  throw e;
}
stats.visitor = visitor.public;
stats.connectMs = ms(t);

// Where this device stands; wait (checking every 3 s) while the Owner hasn't approved it yet. The
// wait can be long, so a dropped connection (Host restarted, relay hiccup) or a reply without a
// Standing means reconnect and ask again, never a guess.
for (;;) {
  const reply = await Promise.race([ask({ t: 'standing' }), sleep(5000)]).catch(() => undefined);
  const standing = reply?.standing;
  if (standing === 'allowed') break;
  if (standing === 'denied') {
    show(refusal ?? "You're not invited to this site. Ask its owner for an Invite Link.");
    throw new Error('not invited');
  }
  if (standing === 'awaiting_approval') {
    show('Waiting for approval: the owner has to let this device in. This page will open by itself.');
  } else {
    tooob.close(); // or its Relay link lingers
    tooob = await connect().catch(() => tooob);
  }
  await sleep(3000);
}

watch(tooob);
document.addEventListener('visibilitychange', checkOnWake);

// The Direct Path (ADR 0004): one WebRTC data channel, negotiated over the iroh connection. The
// offer, the answer (with the Host's DTLS fingerprint) and the ICE candidates all travel over iroh,
// so the Direct Path is tied to the Host Key: DTLS refuses any certificate that doesn't hash to it. Candidates trickle both ways as they're found (setup ≈
// one round trip plus the ICE checks). Only for a device that's in: the Host refuses anyone else.

// Every direct stream shares the one channel. Message: [kind u8][stream id u32 BE][payload].
const DATA = 0, FIN = 1, RESET = 2;
const CHUNK = 32 * 1024;
const SEND_QUEUE = 4 << 20; // well under Chrome's ~16 MB, past which send() throws
function muxTransport(dc) {
  dc.binaryType = 'arraybuffer';
  dc.bufferedAmountLowThreshold = SEND_QUEUE / 2;
  const drained = () => new Promise((r) => {
    dc.addEventListener('bufferedamountlow', r, { once: true });
    dc.addEventListener('close', r, { once: true });
  });
  const streams = new Map();
  let nextId = 1;
  dc.onmessage = ({ data }) => {
    const b = new Uint8Array(data);
    const st = streams.get(new DataView(data).getUint32(1));
    if (!st) return; // already finished or reset
    if (b[0] === DATA) st.queue.push(b.subarray(5));
    else st.ended = true;
    st.wake?.();
  };
  dc.onclose = () => streams.forEach((st) => { st.lost = true; st.wake?.(); });
  const send = (kind, id, bytes = new Uint8Array(0)) => {
    const m = new Uint8Array(5 + bytes.length);
    m[0] = kind;
    new DataView(m.buffer).setUint32(1, id);
    m.set(bytes, 5);
    dc.send(m);
  };
  return async () => {
    if (dc.readyState !== 'open') throw new Error('direct channel closed');
    const id = nextId++;
    const st = { queue: [], ended: false, lost: false, wake: null };
    streams.set(id, st);
    return {
      via: 'direct',
      writer: {
        async write(bytes) {
          for (let i = 0; i < bytes.length; i += CHUNK) {
            if (dc.bufferedAmount > SEND_QUEUE) await drained(); // a big upload
            send(DATA, id, bytes.subarray(i, i + CHUNK));
          }
        },
        finish() { send(FIN, id); },
      },
      reader: {
        async read() {
          while (!st.queue.length && !st.ended && !st.lost) await new Promise((r) => (st.wake = r));
          if (st.queue.length) return st.queue.shift();
          streams.delete(id);
          if (st.lost) throw new Error('the Direct Path closed mid-stream'); // not a whole reply
        },
        free() {
          if (streams.delete(id) && dc.readyState === 'open') send(RESET, id);
        },
      },
    };
  };
}

// Resolve true when cond() holds, false after `ms`.
function until(cond, target, event, ms) {
  return new Promise((resolve) => {
    if (cond()) return resolve(true);
    const done = (v) => { clearTimeout(timer); target.removeEventListener(event, check); resolve(v); };
    const check = () => cond() && done(true);
    const timer = setTimeout(() => done(cond()), ms);
    target.addEventListener(event, check);
  });
}

/**
 * One offer/answer over a new iroh stream, with ICE candidates trickled both ways as they're
 * found; whether `pc` is connected within 5 s. `restart` renegotiates ICE on the Host's existing
 * peer connection instead of setting up a new one. Throws if the Host won't answer.
 */
async function negotiate(pc, restart = false) {
  const s = await viaRelay();
  // Serialize writes so lines never interleave and the offer always goes first.
  let chain = Promise.resolve();
  const sendLine = (o) => (chain = chain.then(() => s.writer.write(enc.encode(JSON.stringify(o) + '\n'))));
  const early = [];
  let offered = false, done = false;
  pc.onicecandidate = ({ candidate }) => {
    if (!candidate?.candidate || done) return;
    offered ? sendLine({ candidate: candidate.toJSON() }) : early.push(candidate.toJSON());
  };
  await pc.setLocalDescription(await pc.createOffer({ iceRestart: restart }));
  sendLine({ t: 'rtc', sdp: pc.localDescription.sdp, ...(restart && { restart: true }) });
  offered = true;
  early.forEach((c) => sendLine({ candidate: c }));
  let [answer, buf] = await readHead(s.reader);
  if (!answer.sdp) throw new Error(answer.error || 'no answer');
  await pc.setRemoteDescription({ type: 'answer', sdp: answer.sdp });
  // The Host's candidates arrive as more lines on the same stream.
  (async () => {
    for (;;) {
      let i;
      while ((i = buf.indexOf(10)) >= 0) {
        const line = JSON.parse(dec.decode(buf.subarray(0, i)));
        buf = buf.slice(i + 1);
        if (line.candidate) pc.addIceCandidate(line.candidate).catch(() => {});
      }
      const c = await s.reader.read();
      if (!c) return;
      buf = concat(buf, c);
    }
  })().catch(() => {});
  const up = await until(() => pc.connectionState === 'connected', pc, 'connectionstatechange', 5000);
  done = true;
  chain.then(() => s.writer.finish()).catch(() => {}); // done signaling either way
  return up;
}

// The Direct Path being set up or in use. A newer one, or losing the iroh connection it was
// negotiated on, retires it.
let directPc = null;

async function goDirect() {
  const pc = (directPc = new RTCPeerConnection({ iceServers: [{ urls: 'stun:stun.l.google.com:19302' }] }));
  const dc = pc.createDataChannel('mux');
  const up = (await negotiate(pc).catch((e) => {
    console.warn('[tooob] direct path', e);
    return false;
  })) && (await until(() => dc.readyState === 'open', dc, 'open', 2000));
  if (directPc !== pc) return pc.close();
  if (!up) {
    pc.close();
    setPath('relay');
    retryDirect();
    return;
  }
  directRetries = 0; // a later new Direct Path gets retries of its own
  const viaDirect = muxTransport(dc);
  const resume = () => {
    direct = viaDirect;
    setPath('direct');
    directKind(pc); // it may have changed (Wi-Fi to 5G)
  };
  resume();

  // A Direct Path that fails mid-visit (the network changed under it: Wi-Fi to 5G and back) gets
  // an ICE restart over iroh: new candidates, same data channel, so streams on it carry on. Tried
  // again and again, backing off to every 30 s, while the Site carries on over the Relay. If the
  // Host no longer has it, or its channel is gone, start a new one.
  let restarting = false;
  const restart = async () => {
    if (restarting) return;
    restarting = true;
    try {
      for (let wait = 2000; ; wait = Math.min(wait * 2, 30000)) {
        if (await negotiate(pc, true)) {
          if (dc.readyState !== 'open') throw new Error('the Direct Path channel closed');
          return resume();
        }
        await sleep(wait);
        // Closed, or came back by itself (the state change resumed it).
        if (pc.connectionState === 'closed' || pc.connectionState === 'connected') return;
      }
    } catch (e) {
      console.warn('[tooob] direct path restart', e);
      pc.close();
      if (directPc === pc) goDirect().catch(() => {});
    } finally {
      restarting = false;
    }
  };

  // 'disconnected' is often temporary (phones suspend backgrounded tabs; WebRTC recovers when they
  // come back), so pause the Direct Path then and resume on 'connected'.
  pc.onconnectionstatechange = () => {
    if (directPc !== pc) return;
    const state = pc.connectionState;
    if (state === 'connected' && dc.readyState === 'open') {
      resume();
    } else if (state === 'disconnected') {
      direct = null;
      setPath('relay (direct paused)');
    } else if (state === 'failed' || state === 'closed') {
      direct = null;
      setPath('relay');
      if (state === 'failed') restart();
    }
  };
}

function dropDirect() {
  directPc?.close();
  directPc = direct = null;
  setPath('relay');
}

// Which path carries the Site's traffic: 'direct', 'relay', or 'relay (direct paused)'.
function setPath(path) {
  stats.path = path;
  if (path !== 'direct') stats.directKind = undefined;
  pill();
}

// What kind of Direct Path WebRTC picked: on the home network (LAN), or punched through NAT via
// STUN-discovered public addresses. Judged by the Host's end of the selected candidate pair. Only
// the kind is ever shown, never an address: the pill is on screen when people show the Site around.
async function directKind(pc) {
  stats.directKind = undefined;
  // Android Chrome may not have the selected pair in its stats yet right at 'connected': retry.
  for (let attempt = 0; attempt < 10 && stats.path === 'direct'; attempt++) {
    const kind = await classify(pc).catch(() => undefined);
    if (kind) {
      stats.directKind = kind;
      return pill();
    }
    await sleep(300);
  }
}

async function classify(pc) {
  const report = await pc.getStats();
  let pair;
  report.forEach((s) => {
    if (s.type === 'transport' && s.selectedCandidatePairId) pair = report.get(s.selectedCandidatePairId);
  });
  report.forEach((s) => {
    if (!pair && s.type === 'candidate-pair' && s.state === 'succeeded' && s.nominated) pair = s;
  });
  const hostEnd = pair && report.get(pair.remoteCandidateId);
  if (!hostEnd?.address) return;
  // ponytail: judged by the address alone, so a home network on global IPv6 shows "NAT punched", and
  // so does Tailscale's 100.64/10; compare the two ends' networks if that misleads people.
  const lan = /^(10\.|192\.168\.|172\.(1[6-9]|2\d|3[01])\.|169\.254\.|127\.|fe80:|fc|fd|::1$)/i.test(hostEnd.address);
  return lan ? 'LAN' : 'NAT punched';
}

// The status pill, once the Site is showing: timings, and which path carries the traffic. Or, with
// the connection lost, that the shell is reconnecting, then that it gave up.
function pill() {
  asButton(gaveUp);
  if (gaveUp) return show("Can't reach the site. Tap to reload.");
  if (lost) return show(`Reconnecting… (try ${lost.n} of ${BACKOFF.length}${lost.next ? `, next in ${lost.next} s` : ''})`);
  if (stats.pageMs === undefined) return; // still loading: the status says so
  const path = stats.path === 'direct' ? ['direct', stats.directKind].filter(Boolean).join(' · ') : 'relay';
  show(`connected in ${stats.connectMs} ms · page ${stats.pageMs} ms · ${path}`);
}

// Once the shell gives up, the pill is a button that reloads the page.
function asButton(on) {
  if (on) {
    statusEl.setAttribute('role', 'button');
    statusEl.tabIndex = 0;
  } else {
    statusEl.removeAttribute('role');
    statusEl.removeAttribute('tabindex');
  }
}
const reload = () => gaveUp && location.reload();
statusEl.onclick = reload;
statusEl.onkeydown = (e) => (e.key === 'Enter' || e.key === ' ') && reload();

// An attempt right after a network switch (e.g. turning Wi-Fi on) gathers half-stale addresses and
// fails; a few seconds later the same attempt works (seen on a phone). So retry quietly a few times
// while the Site keeps working over the Relay.
let directRetries = 0;
function retryDirect() {
  if (directRetries >= 3) return;
  directRetries++;
  const failed = directPc;
  setTimeout(() => directPc === failed && goDirect().catch(() => {}), 4000 * directRetries); // unless a newer one started
}

const directReady = goDirect().catch((e) => {
  console.warn('[tooob] direct path', e);
  setPath('relay');
});

// HTTP: one stream per request.
navigator.serviceWorker.onmessage = async ({ data: req, ports: [port] }) => {
  if (req === 'ping') return port.postMessage('pong'); // the service worker asking who's ready
  // Browser cancelled (e.g. video seek): stop reading, then drop the stream so the host stops sending.
  // ponytail: only noticed when the next chunk arrives, so a quiet stream (long-poll, SSE) stays open
  // after a cancel; needs a stoppable read here and the Host watching for the stop.
  let reader, cancelled = false;
  port.onmessage = ({ data }) => (cancelled = !!data.cancel);
  try {
    const s = await openStream();
    reader = s.reader;
    await s.writer.write(enc.encode(JSON.stringify({ t: 'http', method: req.method, path: req.path, headers: req.headers }) + '\n'));
    if (req.body) await s.writer.write(new Uint8Array(req.body));
    s.writer.finish();
    const [head, rest] = await readHead(reader);
    port.postMessage(head);
    stats.bytes[s.via] += rest.length;
    if (rest.length) port.postMessage({ chunk: rest.buffer }, [rest.buffer]);
    for (let c; !cancelled && (c = await reader.read()); ) {
      stats.bytes[s.via] += c.length;
      const own = c.byteOffset === 0 && c.byteLength === c.buffer.byteLength ? c : c.slice(); // data channel chunks are views
      port.postMessage({ chunk: own.buffer }, [own.buffer]);
    }
    port.postMessage({ done: true });
  } catch (e) {
    port.postMessage({ error: String(e) });
  } finally {
    if (cancelled) reader?.free();
  }
};

// WebSocket: one stream per socket, framed as [kind u8][len u32 BE][payload].
function frame(kind, bytes) {
  const f = new Uint8Array(5 + bytes.length);
  f[0] = kind;
  new DataView(f.buffer).setUint32(1, bytes.length);
  f.set(bytes, 5);
  return f;
}

window.__tooob = {
  openWs(path, protocols, cb) {
    let opened;
    let chain = new Promise((r) => (opened = r)); // writes queue until the socket opens, then run in order
    let writer;
    (async () => {
      try {
        const s = await openStream();
        writer = s.writer;
        const reader = s.reader;
        await writer.write(enc.encode(JSON.stringify({ t: 'ws', path, protocols }) + '\n'));
        let [head, buf] = await readHead(reader);
        if (head.status !== 101) throw new Error(head.error || `status ${head.status}`);
        opened();
        cb.open(head.protocol || '');
        for (;;) {
          while (buf.length >= 5) {
            const len = new DataView(buf.buffer, buf.byteOffset).getUint32(1);
            if (buf.length < 5 + len) break;
            const kind = buf[0];
            const payload = buf.slice(5, 5 + len);
            buf = buf.slice(5 + len);
            if (kind === 2) return cb.close(1000, '');
            cb.message(kind, payload);
          }
          const c = await reader.read();
          if (!c) return cb.close(1006, '');
          buf = concat(buf, c);
        }
      } catch (e) {
        console.warn('[tooob] websocket', path, e);
        cb.error();
        cb.close(1006, String(e));
      }
    })();
    const send = (kind, bytes) => (chain = chain.then(() => writer.write(frame(kind, bytes))).catch(() => {}));
    return {
      send: (kind, bytes) => send(kind, bytes),
      close: () => send(2, new Uint8Array(0)).then(() => writer?.finish()),
    };
  },
};

// The Direct Path usually connects within ~300 ms locally, ~650 ms from a phone network: give it up
// to 800 ms so the first page doesn't go through the Relay. Otherwise start there and switch later.
t = performance.now();
await Promise.race([directReady, sleep(800)]);
stats.directWaitMs = ms(t);

// Load the Site and time it.
t = performance.now();
show(`connected in ${stats.connectMs} ms · loading…`);
const frameEl = document.createElement('iframe');
// Tell the service worker each document the iframe is about to load is this tab's (see tooob-sw.js).
// Not when this whole tab is going (reload, close): the iframe unloads too, but loads nothing here.
let leaving = false;
addEventListener('beforeunload', () => (leaving = true));
const loadingSite = () => leaving || navigator.serviceWorker.controller?.postMessage('loading-site');
frameEl.onload = () => {
  frameEl.contentWindow.addEventListener('beforeunload', loadingSite);
  stats.pageMs ??= ms(t);
  stats.totalMs ??= Math.round(performance.now()); // from navigation start: everything the visitor waits for
  pill();
};
loadingSite();
frameEl.src = '/';
document.body.append(frameEl);
