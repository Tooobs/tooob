// Service worker: every same-origin request is the site's (iframe, its subresources, its workers)
// and is carried over iroh by the shell page. Only the top-level shell page and /__tooob/ assets
// go to the network.
self.addEventListener('install', () => self.skipWaiting());
self.addEventListener('activate', (e) => e.waitUntil(self.clients.claim()));

self.addEventListener('fetch', (e) => {
  const url = new URL(e.request.url);
  if (url.origin !== location.origin || url.pathname.startsWith('/__tooob/') || url.pathname === '/tooob-sw.js') return;
  if (e.request.destination === 'document') return; // the shell itself
  e.respondWith(tunnel(e.request));
});

const NULL_BODY = [101, 204, 205, 304];
const SHIM = '<script src="/__tooob/ws-shim.js"></script>';

// Freshness lifetime in seconds from Cache-Control, or 0 if we shouldn't cache.
function maxAge(status, headers) {
  const cc = (headers.get('cache-control') || '').toLowerCase();
  if (status !== 200 || /no-store|no-cache|private/.test(cc) || headers.has('vary')) return 0;
  return Number(/max-age=(\d+)/.exec(cc)?.[1] || 0);
}

async function tunnel(req) {
  const shell = (await self.clients.matchAll({ type: 'window' })).find((c) => c.frameType === 'top-level');
  if (!shell) return new Response('tooob shell is not open', { status: 503 });
  // Tunneled responses never reach the HTTP cache, so keep fresh ones here, one cache per host.
  // ponytail: freshness only, no ETag revalidation and no eviction; add both if caches grow or go stale.
  const host = new URL(shell.url).hash.slice(1);
  const cache = host && req.method === 'GET' && !req.headers.has('range') ? await caches.open('tooob-' + host) : null;
  const hit = await cache?.match(req.url);
  if (hit && Date.now() < Number(hit.headers.get('x-tooob-expires'))) return hit;

  const url = new URL(req.url);
  const body = ['GET', 'HEAD'].includes(req.method) ? null : await req.arrayBuffer();
  const { port1, port2 } = new MessageChannel();
  shell.postMessage({ method: req.method, path: url.pathname + url.search, headers: [...req.headers], body }, [port2]);

  // First message is the response head, then {chunk}... {done} or {error}.
  let ctrl, resolveHead;
  const headP = new Promise((r) => (resolveHead = r));
  const stream = new ReadableStream({
    start: (c) => (ctrl = c),
    cancel: () => port1.postMessage({ cancel: true }),
  });
  port1.onmessage = ({ data: m }) => {
    if (resolveHead) { resolveHead(m); resolveHead = null; return; }
    if (m.chunk) ctrl.enqueue(new Uint8Array(m.chunk));
    else if (m.done) ctrl.close();
    else if (m.error) ctrl.error(new Error(m.error));
  };
  const head = await headP;
  if (head.error) return new Response(head.error, { status: 502 });

  const headers = new Headers(head.headers);
  const nullBody = NULL_BODY.includes(head.status) || req.method === 'HEAD';
  let respBody = nullBody ? null : stream;
  // Inject the WebSocket shim into HTML documents loaded in the iframe.
  if (!nullBody && req.destination === 'iframe' && (headers.get('content-type') || '').includes('text/html')) {
    const html = await new Response(stream).text();
    respBody = /<head[^>]*>/i.test(html) ? html.replace(/<head[^>]*>/i, (m) => m + SHIM) : SHIM + html;
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
