// Shell page: holds the iroh connection (WASM) and serves the site iframe's requests over it,
// moving them to a direct WebRTC data channel when one can be set up.
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

// The visitor's identity: one key per browser, reused on every visit.
// ponytail: localStorage is readable by the site's own JS (same origin); move to a non-extractable store later.
function visitorKey() {
  let hex = localStorage.tooobVisitorKey;
  if (!hex) {
    hex = [...crypto.getRandomValues(new Uint8Array(32))].map((b) => b.toString(16).padStart(2, '0')).join('');
    localStorage.tooobVisitorKey = hex;
  }
  return Uint8Array.from(hex.match(/../g), (h) => parseInt(h, 16));
}

const host = location.hash.slice(1);
if (!host) {
  show('missing host id: open /#<endpoint-id>');
  throw new Error('no host');
}

let t = performance.now();
show('loading wasm…');
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
const tooob = await Tooob.connect(host, new URLSearchParams(location.search).get('relay') ?? undefined, visitorKey());
stats.visitor = tooob.visitor_id();
stats.connectMs = ms(t);
console.log('[tooob] paths', tooob.paths());

// ── Transports. A stream is {via, writer: {write, finish}, reader: {read, free}}. ──────────────
stats.bytes = { iroh: 0, direct: 0 };
const viaIroh = async () => {
  const bi = await tooob.open();
  return { via: 'iroh', writer: bi.writer(), reader: bi.reader() };
};
let direct = null; // set once the WebRTC path is up
const openStream = () => (direct ? direct().catch(viaIroh) : viaIroh());

// All direct streams share one data channel: several channels transferring at once stall for
// seconds (webrtc-rs + Chrome, spike 2). Message: [kind u8][stream id u32 BE][payload].
const DATA = 0, FIN = 1, RESET = 2;
const CHUNK = 32 * 1024;
function muxTransport(dc) {
  dc.binaryType = 'arraybuffer';
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
  dc.onclose = () => streams.forEach((st) => { st.ended = true; st.wake?.(); });
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
    const st = { queue: [], ended: false, wake: null };
    streams.set(id, st);
    return {
      via: 'direct',
      writer: {
        async write(bytes) { for (let i = 0; i < bytes.length; i += CHUNK) send(DATA, id, bytes.subarray(i, i + CHUNK)); },
        finish() { send(FIN, id); },
      },
      reader: {
        async read() {
          while (!st.queue.length && !st.ended) await new Promise((r) => (st.wake = r));
          if (st.queue.length) return st.queue.shift();
          streams.delete(id);
        },
        free() {
          if (streams.delete(id) && dc.readyState === 'open') send(RESET, id);
        },
      },
    };
  };
}

// Negotiate the direct path over iroh: the offer, the answer (with the host's DTLS fingerprint) and
// the ICE candidates all travel over the authenticated iroh connection, so the direct path is tied
// to the host's key. Candidates trickle both ways as they're found (setup ≈ one round trip + checks).
// ?direct=0 turns it off; ?direct=fail gathers no candidates so it can't connect (tests fallback).
async function goDirect() {
  const mode = new URLSearchParams(location.search).get('direct');
  if (mode === '0') return (stats.path = 'iroh (direct off)');
  const t0 = performance.now();
  const pc = new RTCPeerConnection(mode === 'fail'
    ? { iceTransportPolicy: 'relay', iceServers: [] }
    : { iceServers: [{ urls: 'stun:stun.l.google.com:19302' }] });
  const dc = pc.createDataChannel('mux');
  const s = await viaIroh();
  // Serialize writes so lines never interleave and the offer always goes first.
  let chain = Promise.resolve();
  const sendLine = (o) => (chain = chain.then(() => s.writer.write(enc.encode(JSON.stringify(o) + '\n'))));
  const early = [];
  let offered = false;
  pc.onicecandidate = ({ candidate }) => {
    if (!candidate?.candidate) return;
    offered ? sendLine({ candidate: candidate.toJSON() }) : early.push(candidate.toJSON());
  };
  await pc.setLocalDescription(await pc.createOffer());
  sendLine({ t: 'rtc', sdp: pc.localDescription.sdp });
  offered = true;
  early.forEach((c) => sendLine({ candidate: c }));
  const phases = (stats.directPhases = {});
  let [answer, buf] = await readHead(s.reader);
  if (!answer.sdp) throw new Error(answer.error || 'no answer');
  phases.answer = ms(t0);
  await pc.setRemoteDescription({ type: 'answer', sdp: answer.sdp });
  // Host candidates arrive as more lines on the same stream.
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
  chain.then(() => s.writer.finish()).catch(() => {}); // done signaling either way
  if (!up) {
    stats.directWhy = await whyNoDirect(pc); // DIAGNOSTIC: what was tried, shown in the pill
    pc.close();
    stats.path = 'iroh (direct failed)';
    retryDirect();
    return;
  }
  phases.connected = ms(t0);
  stats.fingerprintOk = await fingerprintMatches(pc, answer.sdp);
  stats.directMs = ms(t0);
  stats.path = 'direct';
  stats.directWhy = undefined; // an earlier attempt's failure no longer applies
  directKind(pc).then(pill);
  await until(() => dc.readyState === 'open', dc, 'open', 2000);
  const viaDirect = muxTransport(dc);
  direct = viaDirect;
  // 'disconnected' is often temporary (phones suspend backgrounded tabs; WebRTC recovers when they
  // come back), so pause the direct path then and resume on 'connected'. Only 'failed' is final.
  pc.onconnectionstatechange = () => {
    const state = pc.connectionState;
    if (state === 'connected' && dc.readyState === 'open') {
      direct = viaDirect;
      stats.path = 'direct';
      stats.directKind = undefined;
      directKind(pc).then(pill); // the path may have changed (e.g. Wi-Fi → cellular)
    } else if (state === 'disconnected') {
      direct = null;
      stats.path = 'iroh (direct paused)';
    } else if (state === 'failed' || state === 'closed') {
      direct = null;
      stats.path = 'iroh (direct lost)';
    }
    pill();
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

// The certificate the browser actually accepted must hash to the fingerprint the host sent over iroh.
async function fingerprintMatches(pc, sdp) {
  const want = /a=fingerprint:sha-256 ([0-9A-F:]+)/i.exec(sdp)?.[1]?.toUpperCase();
  const cert = pc.sctp?.transport?.getRemoteCertificates?.()?.[0];
  if (!want || !cert) return null; // browser doesn't expose it; DTLS still enforced the SDP fingerprint
  const got = [...new Uint8Array(await crypto.subtle.digest('SHA-256', cert))].map((b) => b.toString(16).padStart(2, '0').toUpperCase()).join(':');
  return got === want;
}

const directReady = goDirect().catch((e) => {
  console.warn('[tooob] direct path', e);
  stats.path = 'iroh (direct error)';
});

// A direct attempt right after a network switch (e.g. turning Wi-Fi on) gathers half-stale
// addresses and fails; a few seconds later the same attempt works (seen on a phone: fails, then LAN).
// So retry quietly a few times while the page keeps working over the relay.
let directRetries = 0;
function retryDirect() {
  if (directRetries >= 3 || new URLSearchParams(location.search).get('direct') === 'fail') return;
  directRetries++;
  setTimeout(() => goDirect().then(pill).catch(() => {}), 4000 * directRetries);
}

// HTTP: one stream per request.
navigator.serviceWorker.onmessage = async ({ data: req, ports: [port] }) => {
  // Browser cancelled (e.g. video seek): stop reading, then drop the stream so the host stops sending.
  let reader, cancelled = false;
  port.onmessage = ({ data }) => (cancelled = !!data.cancel);
  try {
    const tReq = performance.now();
    const s = await openStream();
    reader = s.reader;
    await s.writer.write(enc.encode(JSON.stringify({ t: 'http', method: req.method, path: req.path, headers: req.headers }) + '\n'));
    if (req.body) await s.writer.write(new Uint8Array(req.body));
    s.writer.finish();
    const [head, rest] = await readHead(reader);
    const tHead = Math.round(performance.now() - tReq);
    console.log('[tooob]', req.method, req.path, head.status, 'via', s.via);
    port.postMessage(head);
    stats.bytes[s.via] += rest.length;
    if (rest.length) port.postMessage({ chunk: rest.buffer }, [rest.buffer]);
    for (let c; !cancelled && (c = await reader.read()); ) {
      stats.bytes[s.via] += c.length;
      const own = c.byteOffset === 0 && c.byteLength === c.buffer.byteLength ? c : c.slice(); // data channel chunks are views
      port.postMessage({ chunk: own.buffer }, [own.buffer]);
    }
    port.postMessage({ done: true });
    console.log('[tooob] done', req.path, s.via, Math.round(performance.now() - tReq), 'ms', 'first byte', tHead, 'ms');
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

// The direct path usually connects within ~300 ms locally, ~650 ms from a phone network; give it up
// to 800 ms so the first page doesn't go through the relay. Otherwise start on iroh and switch later.
await Promise.race([directReady, new Promise((r) => setTimeout(r, 800))]);

// Load the site and time it.
t = performance.now();
show(`connected in ${stats.connectMs} ms · loading site…`);
const frameEl = document.createElement('iframe');
frameEl.onload = () => {
  stats.pageMs ??= ms(t);
  stats.totalMs ??= Math.round(performance.now()); // from navigation start: everything the visitor waits for
  pill();
};
// Status pill: timings plus which path carries traffic (handy when testing from a phone).
// DIAGNOSTIC: which page version the phone is running (caches made this ambiguous).
const BUILD = 'b4';

function pill() {
  const why = stats.directWhy ? ` · ${stats.directWhy}` : '';
  const kind = (stats.path === 'direct' && stats.directKind ? ` · ${stats.directKind}` : '') + why;
  show(`${BUILD} · connect ${stats.connectMs} ms · first page ${stats.pageMs ?? '…'} ms · ${stats.path ?? 'checking direct…'}${kind}${stats.directMs ? ` (${stats.directMs} ms)` : ''}`);
}

// DIAGNOSTIC: why a direct attempt failed, without addresses: candidate kinds each side offered
// ("phone h2 s1" = 2 host, 1 STUN-discovered) and how the tried pairs ended.
async function whyNoDirect(pc) {
  const report = await pc.getStats();
  const count = (type) => {
    const kinds = {};
    report.forEach((s) => s.type === type && (kinds[s.candidateType] = (kinds[s.candidateType] ?? 0) + 1));
    return Object.entries(kinds).map(([k, n]) => `${k[0]}${n}`).join(' ') || 'none';
  };
  const pairs = {};
  report.forEach((s) => s.type === 'candidate-pair' && (pairs[s.state] = (pairs[s.state] ?? 0) + 1));
  const tried = Object.entries(pairs).map(([state, n]) => `${state} ${n}`).join(', ') || 'no pairs';
  return `phone ${count('local-candidate')} · host ${count('remote-candidate')} · ${tried} · ice ${pc.iceConnectionState}`;
}

// Which kind of direct connection WebRTC picked: on the home network (LAN), or punched through NAT
// via STUN-discovered public addresses. Judged by the Host's end of the selected candidate pair;
// only the kind is shown, never the addresses (the pill is on screen during demos).
async function directKind(pc) {
  // Android Chrome may not have the selected pair in its stats yet right at 'connected': retry.
  for (let attempt = 0; attempt < 10 && !(await classify(pc)); attempt++) {
    await new Promise((r) => setTimeout(r, 300));
  }
  stats.directKind ??= 'kind unknown';
}

/** Set stats.directKind from the selected candidate pair; false if the stats aren't there yet. */
async function classify(pc) {
  const report = await pc.getStats();
  let pair;
  report.forEach((s) => {
    if (s.type === 'transport' && s.selectedCandidatePairId) pair = report.get(s.selectedCandidatePairId);
  });
  report.forEach((s) => {
    if (!pair && s.type === 'candidate-pair' && s.state === 'succeeded' && s.nominated) pair = s;
  });
  const remote = pair && report.get(pair.remoteCandidateId);
  const local = pair && report.get(pair.localCandidateId);
  if (!remote) return false;
  const isPrivate = /^(10\.|192\.168\.|172\.(1[6-9]|2\d|3[01])\.|169\.254\.|127\.|fe80:|fd|::1$)/i.test(remote.address ?? '');
  stats.directKind =
    remote.candidateType === 'relay' || local?.candidateType === 'relay' ? 'relayed (TURN)'
      : isPrivate ? 'LAN'
        : 'NAT punched';
  stats.directCandidates = `${local?.candidateType}/${remote.candidateType}`; // for checks, not shown
  return true;
}
directReady.then(pill);
frameEl.src = '/';
document.body.append(frameEl);
