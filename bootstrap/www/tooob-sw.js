// Service worker: every same-origin request is the site's (iframe, its subresources, its workers)
// and is carried over iroh by the shell page. Only the shell itself and /__tooob/ assets go to the
// network.
self.addEventListener('install', () => self.skipWaiting());
self.addEventListener('activate', (e) => e.waitUntil(self.clients.claim()));

self.addEventListener('fetch', (e) => {
  const url = new URL(e.request.url);
  if (url.origin !== location.origin || url.pathname.startsWith('/__tooob/') || url.pathname === '/tooob-sw.js') return;
  // A page load, at any of the Site's paths, is the shell, which opens that page (ADR 0023): fetched
  // as /, so the Relay never sees which page, and a Bundle upgrade arrives with the next load.
  if (e.request.destination === 'document') return e.respondWith(fetch('/'));
  e.respondWith(shellFor(e).then((shell) => tunnel(e.request, shell)));
});

// Several shell tabs can be open, each with a connection of its own, so every request goes to the
// shell whose iframe made it. A shell says when its iframe is about to load a document; that navigation's new
// client is then its, and so is everything that document fetches.
// When that's unknown, any ready shell tab gets the request (right with one tab open).
// ponytail: best effort with several tabs open. The map is in memory (lost when the browser stops
// an idle worker); tabs loading at the same instant can swap announcements; a Site's own nested
// iframes, workers, and navigations to other origins announce nothing or leave a stale one. Persist
// the map and tag requests with their tab if multi-tab use matters.
const shellOf = new Map(); // Site document's client id → its shell's client id
const loading = []; // shells about to (re)load their iframe, oldest first
const waiters = []; // navigations waiting for an announcement
self.addEventListener('message', (e) => {
  if (e.data !== 'loading-site') return;
  loading.push(e.source.id);
  waiters.splice(0).forEach((wake) => wake());
});

async function shellFor(e) {
  if (e.request.destination !== 'iframe') return shellOf.get(e.clientId);
  // The announcement often arrives just after the navigation it announces: wait for it briefly.
  if (!loading.length) await new Promise((r) => (waiters.push(r), setTimeout(r, 1000)));
  const shell = loading.shift();
  if (shell && e.resultingClientId) shellOf.set(e.resultingClientId, shell);
  return shell;
}

const NULL_BODY = [101, 204, 205, 304];
// Where the request's body would be: the shell asks for it a chunk at a time. A shell from before
// #1 expects the whole body here, and fails the request on this rather than sending it empty.
const STREAMED = -1;
const SHIM = '<script src="/__tooob/ws-shim.js"></script>';

// Freshness lifetime in seconds from Cache-Control, or 0 if we shouldn't cache.
function maxAge(status, headers) {
  const cc = (headers.get('cache-control') || '').toLowerCase();
  if (status !== 200 || /no-store|no-cache|private/.test(cc) || headers.has('vary')) return 0;
  return Number(/max-age=(\d+)/.exec(cc)?.[1] || 0);
}

// Put the shim first in <head> (not <header>), or right after the doctype when the optional <head>
// tag is left out; never before the doctype, which would switch the page to quirks mode.
function injectShim(html) {
  for (const tag of [/<head(\s[^>]*)?>/i, /<!doctype[^>]*>/i]) {
    const m = tag.exec(html);
    if (m) return html.slice(0, m.index + m[0].length) + SHIM + html.slice(m.index + m[0].length);
  }
  return SHIM + html;
}

/** The first shell tab to answer a ping: only tabs that are in (not waiting or refused) answer. */
async function anyReadyShell() {
  const tabs = (await self.clients.matchAll({ type: 'window' })).filter((c) => c.frameType === 'top-level');
  return new Promise((resolve) => {
    setTimeout(resolve, 1000);
    for (const tab of tabs) {
      const { port1, port2 } = new MessageChannel();
      port1.onmessage = () => resolve(tab);
      tab.postMessage('ping', [port2]);
    }
  });
}

async function tunnel(req, shellId) {
  const shell = (shellId && (await self.clients.get(shellId))) ?? (await anyReadyShell());
  if (!shell) return new Response('tooob shell is not open', { status: 503 });
  // Tunneled responses never reach the HTTP cache, so keep fresh ones here: one cache, since this
  // origin is one Host's Key Address.
  // ponytail: freshness only, no ETag revalidation and no eviction; add both if caches grow or go stale.
  const cache = req.method === 'GET' && !req.headers.has('range') ? await caches.open('tooob') : null;
  const hit = await cache?.match(req.url);
  if (hit && Date.now() < Number(hit.headers.get('x-tooob-expires'))) return hit;

  const url = new URL(req.url);
  // A request body goes to the shell a chunk at a time, as fast as the Host takes it, so an upload
  // of any size is never held whole (#1). Without Request.body (Firefox), a Blob: the browser's, which
  // can keep a big one on disk.
  const upload = ['GET', 'HEAD'].includes(req.method) ? null : (req.body ?? (await req.blob()).stream()).getReader();
  const { port1, port2 } = new MessageChannel();
  const body = upload ? STREAMED : null;
  shell.postMessage({ method: req.method, path: url.pathname + url.search, headers: [...req.headers], body }, [port2]);

  // The shell asks for the body with {pull}, each answered with {up: bytes}, {up: null} at its end,
  // or {upError} if it can't be read whole (the body's stream errors). Its first other message
  // is the response head, then {chunk}... {done} or {error}.
  const pull = async () => {
    // The page giving up must never pass for the body's end. Chrome doesn't say so here yet: the
    // upload carries on, whole (#86).
    const { done, value } = await upload.read();
    if (req.signal?.aborted) throw new Error('the page gave up on the request');
    if (done) return port1.postMessage({ up: null });
    const own = value.byteOffset === 0 && value.byteLength === value.buffer.byteLength ? value : value.slice();
    port1.postMessage({ up: own.buffer }, [own.buffer]);
  };
  let ctrl, resolveHead;
  const headP = new Promise((r) => (resolveHead = r));
  const stream = new ReadableStream({
    start: (c) => (ctrl = c),
    cancel: () => port1.postMessage({ cancel: true }),
  });
  port1.onmessage = ({ data: m }) => {
    if (m.pull) return void pull().catch((e) => port1.postMessage({ upError: String(e) }));
    if (resolveHead) { resolveHead(m); resolveHead = null; return; }
    if (m.chunk) ctrl.enqueue(new Uint8Array(m.chunk));
    else if (m.done) ctrl.close();
    else if (m.error) ctrl.error(new Error(m.error));
  };
  // The tab can close mid-request and would never answer: check it's still there while waiting.
  const gone = new Promise((resolve) => {
    const check = setInterval(async () => {
      if (!(await self.clients.get(shell.id))) resolve({ error: 'the tooob tab was closed' });
    }, 2000);
    headP.finally(() => clearInterval(check));
  });
  const head = await Promise.race([headP, gone]);
  if (head.error) return new Response(head.error, { status: 502 });

  const headers = new Headers(head.headers);
  const nullBody = NULL_BODY.includes(head.status) || req.method === 'HEAD';
  let respBody = nullBody ? null : stream;
  // Inject the WebSocket shim into HTML documents loaded in the iframe.
  if (!nullBody && req.destination === 'iframe' && (headers.get('content-type') || '').includes('text/html')) {
    const html = await new Response(stream).text();
    respBody = injectShim(html);
    headers.delete('content-length');
  }
  // Cookies are handled by the host's per-visitor jar; synthesized responses can't set them anyway.
  const status = head.status === 101 ? 200 : head.status;
  const ttl = cache && respBody instanceof ReadableStream ? maxAge(status, headers) : 0;
  if (ttl) {
    const [mine, stored] = respBody.tee();
    const keep = new Headers(headers);
    keep.set('x-tooob-expires', String(Date.now() + ttl * 1000));
    cache.put(req.url, new Response(stored, { status, headers: keep })).catch(() => {});
    respBody = mine;
  }
  return new Response(respBody, { status, headers });
}
