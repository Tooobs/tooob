// The browser side of an Invite Link, end to end. Run: npx playwright test (after ../build.sh).
import assert from 'node:assert/strict';
import { createHash, randomBytes } from 'node:crypto';
import { closeSync, mkdirSync, mkdtempSync, openSync, readFileSync, rmSync, writeFileSync, writeSync } from 'node:fs';
import { createServer, get } from 'node:http';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { test as base } from './guards.mjs';
import * as h from './harness.mjs';

// The tooob the Hosts run, built once per worker with time of its own: a cold build isn't the first
// test's to wait out. Then iroh-relay itself, and the Relay Hosts name: a front that hands iroh's
// paths to it and serves the Bootstrap for everything else, as a Relay Operator runs one (ADR 0020).
const test = base.extend({
  tooob: [
    async ({}, use) => {
      await h.buildHost();
      await use();
    },
    { scope: 'worker', timeout: 10 * 60_000 },
  ],
  irohRelay: [
    async ({ tooob }, use) => {
      await use(await h.startRelay());
      h.stopAll();
    },
    { scope: 'worker' },
  ],
  relay: [async ({ irohRelay }, use) => use((await h.startRelayFront(irohRelay)).url), { scope: 'worker' }],
});

async function hostWithSite(relay) {
  const site = await h.startSite();
  let host = await h.startHost(site, relay);
  const { name } = host;
  const invite = (label, ...flags) => h.tooob(['invite', name, label, ...flags]);
  const start = async () => (host = await h.startHost(site, relay, name)); // the same Host: same Host Key and Allowlist
  const restart = async () => {
    await host.stop();
    await start();
  };
  return { site, name, invite, start, restart, pid: () => host.pid, stop: () => host.stop(), pause: () => host.pause(), resume: () => host.resume() };
}

test('an Invite Link loads the Site and drops the token from the address bar', async ({ browser, relay }) => {
  const { invite } = await hostWithSite(relay);
  const page = await h.newVisitor(browser);
  await page.goto(invite('mom'));
  assert.match(await h.siteText(page), /site \/ label=mom/);
  assert.ok(!page.url().includes('invite='), `token still in ${page.url()}`);
});

/** `link`'s Key Address at `path` instead: the address of one of the Site's pages. */
const pageAddress = (link, path) => new URL(link).origin + path;

/** Wait until the address bar is `path` on `page`'s Key Address. */
const addressIs = (page, path) => page.waitForFunction((path) => location.pathname + location.search + location.hash === path, path, { timeout: 10000 });

/** Wait until the tab's title is `title`. */
const titleIs = (page, title) => page.waitForFunction((title) => document.title === title, title, { timeout: 10000 });

/** The Site's page in the shell's iframe. */
const siteFrame = async (page) => (await page.$('iframe')).contentFrame();

test("a Visitor opening a page's own address sees that page, with the service worker installed and without", async ({ browser, relay }) => {
  const { invite } = await hostWithSite(relay);
  const link = invite('mom');
  const first = await h.newVisitor(browser);
  await first.goto(link);
  await h.siteText(first);

  const again = await first.context().newPage(); // the same device, its service worker installed
  await again.goto(pageAddress(link, '/subfolder/page.html?q=1'));
  assert.match(await h.siteText(again), /site \/subfolder\/page\.html\?q=1 label=mom/);

  // The browser cleared the service worker, and kept the Visitor Key.
  const cdp = await again.context().newCDPSession(again);
  await cdp.send('Storage.clearDataForOrigin', { origin: new URL(link).origin, storageTypes: 'service_workers' });
  await cdp.detach();
  const cleared = await first.context().newPage();
  await cleared.goto(pageAddress(link, '/subfolder/page.html'));
  assert.match(await h.siteText(cleared), /site \/subfolder\/page\.html label=mom/);
});

test("an Invite Link at a page's own address opens that page, and drops the token", async ({ browser, relay }) => {
  const { invite } = await hostWithSite(relay);
  const link = new URL(invite('mom'));
  const page = await h.newVisitor(browser);
  await page.goto(pageAddress(link, '/subfolder/page.html' + link.hash));
  assert.match(await h.siteText(page), /site \/subfolder\/page\.html label=mom/);
  assert.equal(page.url(), pageAddress(link, '/subfolder/page.html'));
});

test("a path that starts with // or /\\ is the Site's, and never frames another website", async ({ browser, relay }) => {
  // //example.invalid/x, as a URL of its own, would be another website's.
  const { invite } = await hostWithSite(relay);
  const link = new URL(invite('mom'));
  const page = await h.newVisitor(browser);
  await page.goto(pageAddress(link, '//example.invalid/x' + link.hash)); // a first visit, before the service worker
  assert.match(await h.siteText(page), /site \/\/example\.invalid\/x label=mom/);
  assert.equal(new URL((await siteFrame(page)).url()).origin, link.origin);
  assert.equal(page.url(), pageAddress(link, '//example.invalid/x'));

  const again = await page.context().newPage(); // its service worker installed
  await again.goto(pageAddress(link, '/\\example.invalid/y')); // the browser reads /\ as //
  assert.match(await h.siteText(again), /site \/\/example\.invalid\/y label=mom/);
  assert.equal(new URL((await siteFrame(again)).url()).origin, link.origin);
});

test("the Site moving to a path that starts with // puts the address bar there, on the Key Address", async ({ browser, relay }) => {
  const { invite } = await hostWithSite(relay);
  const link = new URL(invite('mom'));
  const page = await h.newVisitor(browser);
  await page.goto(link.href);
  await h.siteText(page);
  await (await siteFrame(page)).evaluate(() => history.pushState({}, '', location.origin + '//example.invalid/z'));
  await addressIs(page, '//example.invalid/z');
  assert.equal(new URL(page.url()).origin, link.origin);
  await page.reload();
  assert.match(await h.siteText(page), /site \/\/example\.invalid\/z label=mom/);
});

test("once the service worker is installed, the Relay never sees a page's path", async ({ browser, relay, irohRelay }) => {
  // It serves the shell for every page load itself, fetched from the Relay as / (ADR 0023).
  const { invite } = await hostWithSite(relay);
  const front = await h.startRelayFront(irohRelay); // the browser's alone, so what it's asked is the browser's
  const link = new URL(invite('mom'));
  link.port = front.port;
  const first = await h.newVisitor(browser);
  await first.goto(link.href);
  await h.siteText(first);

  const asked = front.asked.length;
  const again = await first.context().newPage();
  await again.goto(pageAddress(link, '/subfolder/page.html'));
  assert.match(await h.siteText(again), /site \/subfolder\/page\.html label=mom/);
  const pages = front.asked.slice(asked).filter((path) => !h.isBundlePath(path));
  assert.deepEqual(pages, ['/']);
});

test("the Site's pushState and #anchors put the address bar on the page shown, and a reload stays there", async ({ browser, relay }) => {
  const { invite } = await hostWithSite(relay);
  const page = await h.newVisitor(browser);
  await page.goto(invite('mom'));
  await h.siteText(page);
  const site = await siteFrame(page);

  await site.evaluate(() => history.pushState({}, '', '/albums/2026?sort=new'));
  await addressIs(page, '/albums/2026?sort=new');
  await site.evaluate(() => (location.hash = 'beach'));
  await addressIs(page, '/albums/2026?sort=new#beach');
  await site.evaluate(() => history.back()); // to before the anchor, within the Site's page
  await addressIs(page, '/albums/2026?sort=new');
  await site.evaluate(() => History.prototype.replaceState.call(history, {}, '', '/albums/2026?sort=old#beach'));
  await addressIs(page, '/albums/2026?sort=old#beach');

  await page.reload();
  assert.match(await h.siteText(page), /site \/albums\/2026\?sort=old label=mom/);
  assert.equal(await (await siteFrame(page)).evaluate(() => location.hash), '#beach');
  assert.equal(new URL(page.url()).hash, '#beach');
});

test("after the Site's pushState, a reload stays on that page", async ({ browser, relay }) => {
  const { invite } = await hostWithSite(relay);
  const page = await h.newVisitor(browser);
  await page.goto(invite('mom'));
  await h.siteText(page);
  await (await siteFrame(page)).evaluate(() => history.pushState({}, '', '/albums/2026?sort=new'));
  await addressIs(page, '/albums/2026?sort=new');
  await page.reload();
  assert.match(await h.siteText(page), /site \/albums\/2026\?sort=new label=mom/);
  assert.equal(new URL(page.url()).pathname + new URL(page.url()).search, '/albums/2026?sort=new');
});

test("after the Site's #anchor, a reload stays on it", async ({ browser, relay }) => {
  const { invite } = await hostWithSite(relay);
  const page = await h.newVisitor(browser);
  await page.goto(invite('mom'));
  await h.siteText(page);
  await (await siteFrame(page)).evaluate(() => (location.hash = 'beach'));
  await addressIs(page, '/#beach');
  await page.reload();
  assert.match(await h.siteText(page), /site \/ label=mom/);
  assert.equal(await (await siteFrame(page)).evaluate(() => location.hash), '#beach');
  assert.equal(new URL(page.url()).hash, '#beach');
});

test("the Site's pushState still works when the browser refuses to move the address bar", async ({ browser, relay }) => {
  // As Safari does past 100 history calls in 30 s: it throws, and the shell carries on.
  const { invite } = await hostWithSite(relay);
  const page = await h.newVisitor(browser);
  await page.goto(invite('mom'));
  await h.siteText(page);
  const shown = page.url();
  await page.evaluate(() => {
    history.replaceState = () => {
      throw new DOMException('Attempt to use history.replaceState() more than 100 times per 30 seconds', 'SecurityError');
    };
  });
  const site = await siteFrame(page);
  assert.equal(await site.evaluate(() => (history.pushState({}, '', '/albums/2026'), location.pathname)), '/albums/2026');
  assert.equal(page.url(), shown, 'the address bar moved anyway');
});

test('reopening a used Invite Link in another browser says so and never reaches the Site', async ({ browser, relay }) => {
  const { site, invite } = await hostWithSite(relay);
  const link = invite('mom');
  const first = await h.newVisitor(browser);
  await first.goto(link);
  await h.siteText(first);
  const requestsBefore = site.requests;

  const second = await h.newVisitor(browser);
  await second.goto(link);
  await h.pillSays(second, /already been used/, 20000);
  assert.equal(site.requests, requestsBefore, 'the second browser reached the Site');
});

test('an approve-first Invite waits, then opens by itself once the owner approves', async ({ browser, relay }) => {
  const { name, invite } = await hostWithSite(relay);
  const page = await h.newVisitor(browser);
  await page.goto(invite('mom', '--approve'));
  await h.pillSays(page, /Waiting for approval/, 20000);

  const key = h.tooob(['visitors', name]).split(/\s+/)[0];
  h.tooob(['approve', name, key]);
  assert.match(await h.siteText(page), /label=mom/, 'the page opened by itself');
});

test('after redeeming, a reload still gets in (the device is remembered)', async ({ browser, relay }) => {
  const { invite } = await hostWithSite(relay);
  const page = await h.newVisitor(browser);
  await page.goto(invite('mom'));
  await h.siteText(page);
  await page.reload();
  assert.match(await h.siteText(page), /label=mom/);
});

test('a browser with a Visitor Key from before the key store keeps its identity', async ({ browser, relay }) => {
  const { name, invite } = await hostWithSite(relay);
  const link = invite('mom');
  const page = await h.newVisitor(browser);
  await page.goto(new URL('/__tooob/version.json', link).href); // its origin, without starting the shell
  // the old shell kept the Visitor Key's secret in localStorage, as hex
  // RFC 8032's first Ed25519 test vector: this secret's public key is d75a9801…
  await page.evaluate(() => (localStorage.tooobVisitorKey = '9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60'));
  await page.goto(link);
  await h.siteText(page);
  const visitors = h.tooob(['visitors', name]);
  assert.match(visitors, /^d75a980182b1\s/m, visitors); // the CLI shows the first 12 hex digits
  const found = await h.whatTheSiteCanReach(page);
  assert.deepEqual(found.localStorage, {}, 'the old key is still in localStorage');
  assert.deepEqual(found.privateKeys, ['Ed25519'], 'expected the moved Visitor Key in the key store');
  assert.deepEqual(found.exported, [], 'the Site exported the moved Visitor Key');
});

test("the Site's own scripts can't read the Visitor Key", async ({ browser, relay }) => {
  const { invite } = await hostWithSite(relay);
  const page = await h.newVisitor(browser);
  await page.goto(invite('mom'));
  await h.siteText(page);
  const found = await h.whatTheSiteCanReach(page);
  assert.deepEqual(found.localStorage, {}, 'the shell left something in localStorage');
  assert.deepEqual(found.privateKeys, ['Ed25519'], 'expected the Visitor Key in the key store');
  assert.deepEqual(found.exported, [], 'the Site exported the Visitor Key');
});

test("a link without an Invite tells a stranger they're not invited", async ({ browser, relay }) => {
  const { site, invite } = await hostWithSite(relay);
  const plain = invite('mom').replace(/#invite=.*/, ''); // the Host's Key Address, minus the Invite
  const page = await h.newVisitor(browser);
  await page.goto(plain);
  await h.pillSays(page, /not invited/, 20000);
  assert.equal(site.requests, 0);
});

test("a stranger at a page's own address is told the same, and the Host is never asked for the page", async ({ browser, relay }) => {
  const { site, invite } = await hostWithSite(relay);
  const page = await h.newVisitor(browser);
  await page.goto(pageAddress(invite('mom'), '/subfolder/page.html'));
  await h.pillSays(page, /not invited/, 20000);
  assert.equal(await h.statusText(page), "You're not invited to this site. Ask its owner for an Invite Link.");
  assert.equal(site.requests, 0);
});

test('re-tapping your own Invite Link later still opens the Site', async ({ browser, relay }) => {
  const { invite } = await hostWithSite(relay);
  const link = invite('mom');
  const page = await h.newVisitor(browser);
  await page.goto(link);
  await h.siteText(page);
  await page.goto('about:blank');
  await page.goto(link); // from her email again, token and all
  assert.match(await h.siteText(page), /label=mom/);
});

test('the Invite token never ends up in browser storage', async ({ browser, relay }) => {
  const { invite } = await hostWithSite(relay);
  const page = await h.newVisitor(browser);
  await page.goto(invite('mom'));
  await h.siteText(page);
  const caches = await page.evaluate(() => caches.keys());
  assert.ok(caches.length > 0, 'expected the shell to have opened its cache');
  assert.ok(caches.every((name) => !name.includes('invite')), `cache names: ${caches}`);
});

test("the injected WebSocket shim keeps the Site's page in standards mode", async ({ browser, relay }) => {
  const { invite } = await hostWithSite(relay); // the stand-in Site serves <!doctype html> with no <head>
  const page = await h.newVisitor(browser);
  await page.goto(invite('mom'));
  await h.siteText(page);
  const frame = await (await page.$('iframe')).contentFrame();
  assert.equal(await frame.evaluate(() => document.compatMode), 'CSS1Compat', 'quirks mode: shim landed before the doctype');
  assert.equal(await frame.evaluate(() => typeof window.WebSocket.prototype.send), 'function');
});

test('a page waiting for approval survives the Host restarting', async ({ browser, relay }) => {
  const { name, invite, restart } = await hostWithSite(relay);
  const page = await h.newVisitor(browser);
  await page.goto(invite('mom', '--approve'));
  await h.pillSays(page, /Waiting for approval/, 20000);

  await restart(); // the page's connection drops while it's waiting
  const key = h.tooob(['visitors', name]).split(/\s+/)[0];
  h.tooob(['approve', name, key]);
  assert.match(await h.siteText(page, 40000), /label=mom/, 'the page reconnected and opened by itself');
});

test('an Invite Link is its Host\'s Key Address, on its Relay\'s domain', async ({ relay }) => {
  const { name, invite } = await hostWithSite(relay);
  const key = h.tooob(['show', name]).match(/Host Key\s+(\S+)/)[1];
  const link = new URL(invite('mom'));
  assert.equal(link.origin, `http://${key}.localhost:${new URL(relay).port}`);
  assert.match(link.hash, /^#invite=[^&]+$/);
  assert.equal(link.search, '', 'no ?relay=: the Relay is the link\'s own domain');
});

test("two Sites through one Relay are two origins: neither can reach the other's storage or Visitor Key", async ({ browser, relay }) => {
  const x = await hostWithSite(relay);
  const y = await hostWithSite(relay);
  const tabX = await h.newVisitor(browser);
  const tabY = await tabX.context().newPage(); // the same browser, so the same profile
  await tabX.goto(x.invite('mom'));
  await h.siteText(tabX);
  const siteX = await (await tabX.$('iframe')).contentFrame();
  await siteX.evaluate(async () => {
    localStorage.mine = 'x';
    await (await caches.open('mine')).put('/mine', new Response('x'));
  });

  await tabY.goto(y.invite('dad'));
  await h.siteText(tabY);
  const found = await h.whatTheSiteCanReach(tabY);
  assert.deepEqual(found.localStorage, {}, "Site Y read Site X's localStorage");
  assert.ok(!found.caches.includes('mine'), `Site Y saw Site X's caches: ${found.caches}`);
  assert.deepEqual(found.privateKeys, ['Ed25519'], 'only its own Visitor Key');
  const visitorKey = (host) => h.tooob(['visitors', host.name]).split(/\s+/)[0];
  assert.notEqual(visitorKey(x), visitorKey(y), 'one Visitor Key for both Sites');
});

test("a cookie a Site sets for the whole Relay domain stays with that Site", async ({ browser, irohRelay }) => {
  // Without the Public Suffix List, the browser lets a Key Address set a cookie for its whole Relay
  // domain, which every other Host's pages would then see (ADR 0021).
  const shared = (await h.startRelayFront(irohRelay, 'relay.localhost')).url;
  const a = await h.startHost(await h.startSite(), shared);
  const b = await h.startHost(await h.startSite(), shared);
  const tabA = await h.newVisitor(browser);
  const tabB = await tabA.context().newPage(); // the same browser, so the same cookies
  await tabA.goto(h.tooob(['invite', a.name, 'mom']));
  await h.siteText(tabA);
  const siteA = await (await tabA.$('iframe')).contentFrame();
  await siteA.evaluate(async () => {
    document.cookie = 'viaDocument=a; Domain=relay.localhost; Path=/';
    await cookieStore.set({ name: 'viaCookieStore', value: 'a', domain: 'relay.localhost', path: '/' });
  });
  // Where the browser keeps them, before any shell of Site B's could clean up after Site A.
  const kept = (await tabA.context().cookies()).map((c) => `${c.name} for ${c.domain}`).sort();
  const keyAddress = new URL(tabA.url()).hostname;
  assert.deepEqual(kept, [`viaCookieStore for ${keyAddress}`, `viaDocument for ${keyAddress}`]);

  await tabB.goto(h.tooob(['invite', b.name, 'dad']));
  await h.siteText(tabB);
  const siteB = await (await tabB.$('iframe')).contentFrame();
  assert.equal(await siteB.evaluate(() => document.cookie), '', "Site B read Site A's cookies");
});

test('a cookie for the whole Relay domain is gone once a shell has started, and the Site keeps its own', async ({ browser, irohRelay }) => {
  // Set where the injected script doesn't reach, such as a Site's own service worker (ADR 0021).
  const shared = (await h.startRelayFront(irohRelay, 'relay.localhost')).url;
  const host = await h.startHost(await h.startSite(), shared);
  const link = h.tooob(['invite', host.name, 'mom']);
  const tab = await h.newVisitor(browser);
  const jar = tab.context();
  await jar.addCookies([
    { name: 'stray', value: 'relay-wide', domain: '.relay.localhost', path: '/' },
    { name: '__Secure-stray', value: 'relay-wide', domain: '.relay.localhost', path: '/', secure: true }, // deleted only by a Secure deletion
    { name: 'stray', value: 'mine', domain: new URL(link).hostname, path: '/' },
  ]);
  await tab.goto(link);
  await h.siteText(tab);
  const left = (await jar.cookies()).map((c) => `${c.name}=${c.value} for ${c.domain}`);
  assert.deepEqual(left, [`stray=mine for ${new URL(link).hostname}`]);
});

/** GET `path` from a Relay front, as a browser would, with `headers`: its response, with `body`. */
const askFront = (front, path, headers = {}) => new Promise((resolve, reject) => {
  get({ host: '127.0.0.1', port: front.port, path, headers }, (res) => {
    let body = '';
    res.setEncoding('utf8').on('data', (d) => (body += d)).on('end', () => resolve(Object.assign(res, { body })));
  })
    .on('upgrade', (res, socket) => (socket.destroy(), resolve(res)))
    .on('error', reject);
});

test('the Relay serves the Bundle for process isolation, and passes no Cookie on', async () => {
  // In step with the Relay image's front.caddy (ADR 0021): what iroh-relay is sent here stands in
  // for what reaches it there.
  const seen = [];
  const upstream = createServer((req, res) => (seen.push(req.headers), res.end()));
  upstream.on('upgrade', (req, socket) => (seen.push(req.headers), socket.end('HTTP/1.1 404 Not Found\r\n\r\n')));
  await new Promise((r) => upstream.listen(0, '127.0.0.1', r));
  try {
    const front = await h.startRelayFront(`http://127.0.0.1:${upstream.address().port}/`);
    const ask = (path, headers) => askFront(front, path, { cookie: 'bomb=x', ...headers });
    const bundle = JSON.parse(readFileSync(new URL('../www/__tooob/version.json', import.meta.url), 'utf8'));
    for (const path of ['/', '/__tooob/version.json', ...Object.keys(bundle.files).map((f) => '/' + f)]) {
      const res = await ask(path);
      assert.equal(res.statusCode, 200, path);
      assert.equal(res.headers['origin-agent-cluster'], '?1', path);
      assert.equal(res.headers['cross-origin-opener-policy'], 'same-origin-allow-popups', path);
      assert.equal(res.headers['cross-origin-embedder-policy'], undefined, path);
    }
    await ask('/healthz');
    await ask('/relay', { connection: 'Upgrade', upgrade: 'websocket' });
    assert.equal(seen.length, 2);
    assert.deepEqual(seen.map((headers) => headers.cookie), [undefined, undefined]);
  } finally {
    upstream.close();
  }
});

test("the Relay serves the shell at any page's path, and a missing Bundle file is still a 404", async ({ irohRelay }) => {
  // In step with the Relay image's front.caddy (ADR 0023): for a first visit, before the service
  // worker is installed.
  const front = await h.startRelayFront(irohRelay);
  const shell = readFileSync(new URL('../www/index.html', import.meta.url), 'utf8');
  for (const path of ['/subfolder/', '/subfolder/page.html', '/photo.png?size=big', '/a/b/c', '/tooob-sw.js/x', '//example.invalid/x']) {
    const res = await askFront(front, path);
    assert.equal(res.statusCode, 200, path);
    assert.equal(res.body, shell, path);
    assert.match(res.headers['content-type'], /^text\/html/, path);
  }
  for (const path of ['/__tooob/missing.js', '/__tooob/', '/__tooob/pkg/missing.wasm']) {
    assert.equal((await askFront(front, path)).statusCode, 404, path);
  }
});

test('two tabs on two different Sites each get their own Site', async ({ browser, relay }) => {
  const x = await hostWithSite(relay);
  const y = await hostWithSite(relay);
  const tabX = await h.newVisitor(browser);
  const tabY = await tabX.context().newPage(); // same browser, so same service worker
  await tabX.goto(x.invite('mom'));
  assert.match(await h.siteText(tabX), /label=mom/);
  await tabY.goto(y.invite('dad'));
  assert.match(await h.siteText(tabY), /label=dad/, "tab Y's requests went to tab X's Site");
  await tabX.reload();
  assert.match(await h.siteText(tabX), /label=mom/);
});

test('two tabs on the same Site both keep working', async ({ browser, relay }) => {
  const { invite } = await hostWithSite(relay);
  const first = await h.newVisitor(browser);
  await first.goto(invite('mom'));
  await h.siteText(first);
  const second = await first.context().newPage();
  await second.goto(first.url()); // the plain link, as a second tab would open it
  assert.match(await h.siteText(second), /label=mom/);
  await first.reload(); // the first tab still gets through
  assert.match(await h.siteText(first), /label=mom/);
});

test("a tab still waiting for approval never swallows another tab's requests", async ({ browser, relay }) => {
  const x = await hostWithSite(relay);
  const y = await hostWithSite(relay);
  const openWaiting = async (page) => {
    await page.goto(x.invite('mom', '--approve'));
    await h.pillSays(page, /Waiting for approval/, 20000);
  };
  // Waiting tabs on both sides of the allowed one, so tab order can't hide a wrong pick.
  const first = await h.newVisitor(browser);
  await openWaiting(first);
  const allowed = await first.context().newPage();
  await allowed.goto(y.invite('dad'));
  await h.siteText(allowed);
  await openWaiting(await first.context().newPage());

  // The browser stops idle service workers, forgetting which tab is whose.
  const cdp = await allowed.context().newCDPSession(allowed);
  await cdp.send('ServiceWorker.enable');
  await cdp.send('ServiceWorker.stopAllWorkers');
  const frame = await (await allowed.$('iframe')).contentFrame();
  const text = await frame.evaluate(() => Promise.race([
    fetch('/again').then((r) => r.text()),
    new Promise((r) => setTimeout(() => r('hung'), 10000)),
  ]));
  assert.match(text, /site \/again label=dad/);
});

test("the Site's traffic goes over the Direct Path, not the Relay", async ({ browser, relay }) => {
  const { invite } = await hostWithSite(relay);
  const page = await h.newVisitor(browser);
  await page.goto(invite('mom'));
  await h.siteText(page);
  await h.pathIs(page, 'direct', 20000);
  const bytes = await h.bytesForReload(page);
  assert.ok(bytes.direct > 0, `nothing came direct: ${JSON.stringify(bytes)}`);
  assert.equal(bytes.relay, 0, `the Relay carried Site bytes: ${JSON.stringify(bytes)}`);
});

test('after the phone sleeps, the Site still loads and goes back onto the Direct Path', async ({ browser, relay, irohRelay }) => {
  const { invite } = await hostWithSite(relay);
  const front = await h.startRelayFront(irohRelay); // the browser's alone, so silencing it leaves the Host's be
  const page = await h.newVisitor(browser);
  const link = new URL(invite('mom'));
  link.port = front.port;
  await page.goto(link.href);
  await h.siteText(page);
  await h.pathIs(page, 'direct', 10000);

  await h.sleepPage(page, front, 35); // past iroh's 30 s idle timeout: the Host drops the connection
  assert.equal(await h.siteFetch(page, '/bytes/20000000'), '200 20000000', 'a download right after waking');
  await h.pathIs(page, 'direct', 20000);
});

test("after a phone's deep sleep silences its Relay connection, the Site loads as soon as it wakes", async ({ browser, relay, irohRelay }) => {
  // Firefox for Android in Doze: the page and iroh's timers freeze, the network drops the old
  // flows, and on waking iroh still thinks its connection is up, so a request waits ~30 s for
  // its idle timeout. Relay only, so nothing can come by the Direct Path instead.
  const { invite } = await hostWithSite(relay);
  const front = await h.startRelayFront(irohRelay); // the browser's alone, so silencing it leaves the Host's be
  const page = await h.newVisitor(browser);
  await h.breakDirectPath(page);
  const link = new URL(invite('mom'));
  link.port = front.port;
  await page.goto(link.href);
  await h.siteText(page);

  front.silence();
  await h.wake(page);
  const download = h.siteFetch(page, '/bytes/20000000');
  const late = new Promise((r) => setTimeout(() => r('no answer in 10 s'), 10000));
  assert.equal(await Promise.race([download, late]), '200 20000000');
});

test('a Direct Path that drops and comes back carries the Site again, without a reload', async ({ browser, relay }) => {
  const { invite, pause, resume } = await hostWithSite(relay);
  const page = await h.newVisitor(browser);
  await page.goto(invite('mom'));
  await h.siteText(page);
  await h.pathIs(page, 'direct', 20000);

  pause(); // the browser's ICE checks go unanswered, as when a phone backgrounds the tab
  try {
    await page.waitForFunction(() => /paused/.test(window.__tooobStats.path), null, { timeout: 20000 });
  } finally {
    resume();
  }
  await h.pathIs(page, 'direct', 20000);
  const bytes = await h.bytesForReload(page);
  assert.ok(bytes.direct > 0, `nothing came direct: ${JSON.stringify(bytes)}`);
  assert.equal(bytes.relay, 0, `the Relay carried Site bytes: ${JSON.stringify(bytes)}`);
});

test('a Direct Path that fails mid-visit is renegotiated each time, and a download in flight survives', async ({ browser, relay }) => {
  const { invite } = await hostWithSite(relay);
  const page = await h.newVisitor(browser);
  await h.watchDirectPath(page);
  await page.goto(invite('mom'));
  await h.siteText(page);
  await h.pathIs(page, 'direct', 20000);
  const frame = await (await page.$('iframe')).contentFrame();
  const download = frame.evaluate(() => fetch('/bytes/20971520').then((r) => r.arrayBuffer()).then((b) => b.byteLength));

  for (let i = 0; i < 5; i++) { // more failures than any retry budget
    assert.notEqual(await h.failDirectPath(page), 'direct', `failure ${i + 1} went unnoticed`);
    await h.pathIs(page, 'direct', 3000);
  }
  assert.equal(await download, 20971520);
  const bytes = await h.bytesForReload(page);
  assert.ok(bytes.direct > 0, `nothing came direct: ${JSON.stringify(bytes)}`);
  assert.equal(bytes.relay, 0, `the Relay carried Site bytes: ${JSON.stringify(bytes)}`);
});

test('the pill says the Direct Path is on the home network', async ({ browser, relay }) => {
  const { invite } = await hostWithSite(relay);
  const page = await h.newVisitor(browser);
  await page.goto(invite('mom'));
  await h.siteText(page);
  await h.pillSays(page, / · direct · LAN$/, 20000);
  assert.doesNotMatch(await h.statusText(page), /\d+\.\d+\.\d+\.\d+|::/, 'the pill shows an address');
});

test('the pill says a Direct Path from elsewhere was punched through NAT, without its address', async ({ browser, relay }) => {
  const { invite } = await hostWithSite(relay);
  const page = await h.newVisitor(browser);
  await h.forgeNatPunched(page);
  await page.goto(invite('mom'));
  await h.siteText(page);
  await h.pillSays(page, / · direct · NAT punched$/, 20000);
  assert.doesNotMatch(await h.statusText(page), /203\.0\.113/);
});

test('the pill says the Site is coming over the Relay when the Direct Path fails', async ({ browser, relay }) => {
  const { invite } = await hostWithSite(relay);
  const page = await h.newVisitor(browser);
  await h.breakDirectPath(page);
  await page.goto(invite('mom'));
  await h.siteText(page);
  await h.pillSays(page, / · relay$/, 20000);
});

test('when the Direct Path fails, the Site still loads over the Relay', async ({ browser, relay }) => {
  const { invite } = await hostWithSite(relay);
  const page = await h.newVisitor(browser);
  await h.breakDirectPath(page);
  await page.goto(invite('mom'));
  assert.match(await h.siteText(page), /label=mom/);
  await h.pathIs(page, 'relay', 20000);
  const bytes = await h.bytesForReload(page);
  assert.ok(bytes.relay > 0, `nothing came over the Relay: ${JSON.stringify(bytes)}`);
  assert.equal(bytes.direct, 0);
});

test('a Direct Path attempt that fails is retried, and the Site moves onto it', async ({ browser, relay }) => {
  const { invite } = await hostWithSite(relay);
  const page = await h.newVisitor(browser);
  await h.breakDirectPath(page, 1); // like an attempt right after a network switch
  await page.goto(invite('mom'));
  assert.match(await h.siteText(page), /label=mom/);
  await h.pathIs(page, 'direct', 30000);
  const bytes = await h.bytesForReload(page);
  assert.ok(bytes.direct > 0, `nothing came direct: ${JSON.stringify(bytes)}`);
  assert.equal(bytes.relay, 0, `the Relay carried Site bytes: ${JSON.stringify(bytes)}`);
});

test('the first page waits at most ~800 ms for a Direct Path that never comes', async ({ browser, relay }) => {
  const { invite } = await hostWithSite(relay);
  const page = await h.newVisitor(browser);
  await h.breakDirectPath(page); // it never connects, so only the cap ends the wait
  await page.goto(invite('mom'));
  await h.siteText(page);
  const waited = await page.evaluate(() => window.__tooobStats.directWaitMs);
  assert.ok(waited >= 700 && waited <= 1000, `waited ${waited} ms`);
});

test("the browser refuses a Direct Path whose certificate isn't the one the Host sent over iroh", async ({ browser, relay }) => {
  const { invite } = await hostWithSite(relay);
  const page = await h.newVisitor(browser);
  await h.forgeDirectFingerprint(page);
  await page.goto(invite('mom'));
  assert.match(await h.siteText(page), /label=mom/);
  await h.pathIs(page, 'relay', 20000);
  const bytes = await h.bytesForReload(page);
  assert.equal(bytes.direct, 0, 'Site bytes went over the forged Direct Path');
});

test('bursts over the Direct Path never stall (SCTP loses a burst and waits >= 1 s to resend)', async ({ browser, relay }) => {
  const { invite } = await hostWithSite(relay);
  const page = await h.newVisitor(browser);
  await page.goto(invite('mom'));
  await h.siteText(page);
  await h.pathIs(page, 'direct', 20000);
  const frame = await (await page.$('iframe')).contentFrame();
  // Like a page of photos: 8 × 320 KB at once, ten times over.
  const rounds = await frame.evaluate(async () => {
    const times = [];
    for (let round = 0; round < 10; round++) {
      const t = performance.now();
      await Promise.all([...Array(8)].map((_, i) => fetch(`/bytes/327680?r=${round}&i=${i}`).then((r) => r.arrayBuffer())));
      times.push(Math.round(performance.now() - t));
    }
    return times;
  });
  const stats = await page.evaluate(() => window.__tooobStats);
  assert.equal(stats.path, 'direct', 'the Direct Path dropped');
  assert.ok(rounds.every((ms) => ms < 1000), `rounds took ${rounds.join(' / ')} ms`);
});

test('a refused upload leaves the Direct Path carrying the next request', async ({ browser, relay }) => {
  const { name, invite } = await hostWithSite(relay);
  const page = await h.newVisitor(browser);
  await page.goto(invite('mom'));
  await h.siteText(page);
  await h.pathIs(page, 'direct', 20000);
  const key = h.tooob(['visitors', name]).split(/\s+/)[0];
  const frame = await (await page.$('iframe')).contentFrame();

  h.tooob(['block', name, key]); // refused before the Host reads the body
  const upload = await frame.evaluate(() => fetch('/upload', { method: 'POST', body: new Uint8Array(1 << 20) }).then((r) => r.status));
  assert.equal(upload, 403);
  h.tooob(['unblock', name, key]);
  const before = await page.evaluate(() => window.__tooobStats.bytes.direct);
  const text = await frame.evaluate(() => Promise.race([
    fetch('/after').then((r) => r.text()),
    new Promise((r) => setTimeout(() => r('hung'), 5000)),
  ]));
  assert.match(text, /site \/after label=mom/);
  assert.ok((await page.evaluate(() => window.__tooobStats.bytes.direct)) > before, 'the next request left the Direct Path');
});

test('a 20 MB upload goes through on the Direct Path', async ({ browser, relay }) => {
  const { invite } = await hostWithSite(relay);
  const page = await h.newVisitor(browser);
  await page.goto(invite('mom'));
  await h.siteText(page);
  await h.pathIs(page, 'direct', 20000);
  const before = await page.evaluate(() => window.__tooobStats.bytes.direct);
  const frame = await (await page.$('iframe')).contentFrame();
  const reply = await frame.evaluate(() => fetch('/upload', { method: 'POST', body: new Uint8Array(20 << 20) }).then((r) => r.text()));
  assert.match(reply, /received 20971520/);
  assert.ok((await page.evaluate(() => window.__tooobStats.bytes.direct)) > before, 'the upload left the Direct Path');
});

/** A file of `size` random bytes in a folder of its own, made now (never committed): its path and SHA-256. */
function bigFile(size) {
  const path = join(mkdtempSync(join(tmpdir(), 'tooob-upload-')), 'video.mp4');
  const fd = openSync(path, 'w');
  const hash = createHash('sha256');
  for (let left = size; left > 0; left -= 1 << 20) {
    const chunk = randomBytes(Math.min(left, 1 << 20));
    hash.update(chunk);
    writeSync(fd, chunk);
  }
  closeSync(fd);
  return { path, sha256: hash.digest('hex'), remove: () => rmSync(dirname(path), { recursive: true }) };
}

/** A Visitor in on a new Host's Site, its traffic on `path` ('direct' or 'relay'): the Site's frame. */
async function visitOver(browser, relay, path) {
  const host = await hostWithSite(relay);
  const page = await h.newVisitor(browser);
  if (path === 'relay') await h.breakDirectPath(page);
  await page.goto(host.invite('mom'));
  await h.siteText(page);
  await h.pathIs(page, path, 20000);
  return { ...host, page, frame: await (await page.$('iframe')).contentFrame() };
}

for (const path of ['direct', 'relay']) {
  test(`an upload far over 64 MiB reaches the Site intact over the ${path} path, and neither the browser nor the Host holds it`, async ({ browser, relay }) => {
    const size = 300 << 20;
    const file = bigFile(size);
    try {
      const { pid, frame } = await visitOver(browser, relay, path);
      // As a Site's upload form does: a file the Visitor picks, sent as it is on disk.
      await frame.evaluate(() => document.body.insertAdjacentHTML('beforeend', '<input type="file" id="pick">'));
      await frame.setInputFiles('#pick', file.path);
      const [host, browserBefore] = [h.memoryOf(pid()), h.browserMemory()];
      let [hostPeak, browserPeak] = [host, browserBefore];
      const sampling = setInterval(() => {
        hostPeak = Math.max(hostPeak, h.memoryOf(pid()));
        browserPeak = Math.max(browserPeak, h.browserMemory());
      }, 200);
      const t = Date.now();
      const reply = await frame.evaluate(() => fetch('/sha256', { method: 'POST', body: document.getElementById('pick').files[0] }).then((r) => r.text()));
      clearInterval(sampling);
      console.log(`${path}: ${size >> 20} MiB in ${Date.now() - t} ms; Host ${host >> 20} → ${hostPeak >> 20} MiB, browser ${browserBefore >> 20} → ${browserPeak >> 20} MiB`);
      assert.equal(reply, `${size} ${file.sha256}`);
      assert.ok(hostPeak - host < 64 << 20, `the Host's memory grew ${(hostPeak - host) >> 20} MiB`);
      assert.ok(browserPeak - browserBefore < 200 << 20, `the browser's memory grew ${(browserPeak - browserBefore) >> 20} MiB`);
    } finally {
      file.remove();
    }
  });
}

for (const path of ['direct', 'relay']) {
  test(`an upload that breaks off midway over the ${path} path never reaches the Site as whole`, async ({ browser, relay }) => {
    const { site, frame } = await visitOver(browser, relay, path);
    // A body the Site's page makes as it goes, and can't finish: 32 MiB, then an error.
    const outcome = await frame.evaluate(() => {
      let sent = 0;
      const body = new ReadableStream({
        pull(c) {
          if (sent >= 32 << 20) return c.error(new Error('the camera roll went away'));
          sent += 1 << 20;
          c.enqueue(new Uint8Array(1 << 20).fill(7));
        },
      });
      return fetch('/sha256', { method: 'POST', body, duplex: 'half' }).then((r) => `status ${r.status}`, (e) => e.name);
    });
    assert.doesNotMatch(outcome, /status 200/);
    await new Promise((r) => setTimeout(r, 2000));
    assert.equal(site.uploads, 0, 'the Site took part of an upload for all of it');
    const next = await frame.evaluate(() => fetch('/after').then((r) => r.text()));
    assert.match(next, /site \/after label=mom/);
  });
}

test('a Site that refuses an upload early gets its answer to the page, and the browser stops sending the rest', async ({ browser, relay }) => {
  const file = bigFile(200 << 20);
  try {
    const { site, page, frame } = await visitOver(browser, relay, 'direct');
    await frame.evaluate(() => document.body.insertAdjacentHTML('beforeend', '<input type="file" id="pick">'));
    await frame.setInputFiles('#pick', file.path);
    const status = await frame.evaluate(() => fetch('/refuse', { method: 'POST', body: document.getElementById('pick').files[0] }).then((r) => r.status));
    assert.equal(status, 413);
    await new Promise((r) => setTimeout(r, 2000)); // time enough to send the rest, on this machine
    const sent = await page.evaluate(() => window.__tooobStats.sent.direct);
    assert.ok(sent < 32 << 20, `the browser sent ${sent >> 20} MiB of a refused 200 MiB upload`);
    assert.ok(site.refused < 32 << 20, `the Host passed ${site.refused >> 20} MiB of it to the Site`);
  } finally {
    file.remove();
  }
});

test("an upload to a Site that takes it slowly doesn't hold up the Visitor's other requests on the Direct Path", async ({ browser, relay }) => {
  const { frame } = await visitOver(browser, relay, 'direct');
  await frame.evaluate(() => void fetch('/slow', { method: 'POST', body: new Uint8Array(32 << 20) }).catch(() => {}));
  await new Promise((r) => setTimeout(r, 2000)); // the upload has filled every buffer on its way
  const ms = await frame.evaluate(async () => {
    const t = performance.now();
    await Promise.race([fetch('/after').then((r) => r.text()), new Promise((r) => setTimeout(r, 10000))]);
    return performance.now() - t;
  });
  assert.ok(ms < 1000, `another request took ${Math.round(ms)} ms during the slow upload`);
});

test("when the Host can't be reached the page says so and keeps the Invite for a retry", async ({ browser, relay }) => {
  const { invite, stop } = await hostWithSite(relay);
  const link = invite('mom');
  await stop();
  const page = await h.newVisitor(browser);
  await page.goto(link);
  await h.pillSays(page, /Couldn't reach/, 60000);
  assert.ok(page.url().includes('invite='), 'the Invite should stay so a reload can retry');
});


test('when the Host goes away the pill says it is reconnecting, and counts the tries', async ({ browser, relay }) => {
  const { invite, stop } = await hostWithSite(relay);
  const page = await h.newVisitor(browser);
  await page.goto(invite('mom'));
  await h.siteText(page);

  const stopping = stop();
  await h.pillSays(page, /^Reconnecting… \(try 1 of 5/, 3000); // heard at once: the Host closes the connection
  await stopping;
  await h.pillSays(page, /^Reconnecting… \(try 2 of 5, next in 4 s\)$/, 20000);
  await h.pillSays(page, /^Reconnecting… \(try 2 of 5, next in 3 s\)$/, 2000);
  await h.pillSays(page, /^Reconnecting… \(try 3 of 5/, 5000);
  await page.context().close(); // stop it trying
});

test('when the Host comes back during the tries, the Site loads again without a reload', async ({ browser, relay }) => {
  const { invite, stop, start } = await hostWithSite(relay);
  const page = await h.newVisitor(browser);
  await page.goto(invite('mom'));
  await h.siteText(page);
  await page.evaluate(() => (window.notReloaded = true));

  await stop();
  await h.pillSays(page, /^Reconnecting…/, 3000);
  await start();
  await h.pillSays(page, /^connected in \d+ ms · page \d+ ms · (relay|direct)/, 20000);
  assert.match(await h.siteFetch(page, '/bytes/1000'), /^200 1000$/);
  assert.ok(await page.evaluate(() => window.notReloaded), 'the shell reloaded');
  await h.pathIs(page, 'direct', 20000); // and the Direct Path is back
});

test("when the Host stays away the pill gives up and offers a reload, and tapping it reloads", async ({ browser, relay }) => {
  const { invite, stop } = await hostWithSite(relay);
  const page = await h.newVisitor(browser);
  await page.goto(invite('mom'));
  await h.siteText(page);

  await stop();
  await h.pillSays(page, /^Reconnecting… \(try 5 of 5\)$/, 60000); // 2 + 4 + 8 + 16 s apart, each try up to 5 s
  await h.pillSays(page, /^Can't reach the site\. Tap to reload\.$/, 10000);
  const frame = await (await page.$('iframe')).contentFrame();
  const failed = await frame.evaluate(async () => {
    const t = performance.now();
    const r = await fetch('/again');
    return { status: r.status, ms: Math.round(performance.now() - t) };
  });
  assert.ok(failed.status === 502 && failed.ms < 1000, `a request after giving up: ${JSON.stringify(failed)}`);
  await Promise.all([page.waitForNavigation(), page.click('#status')]);
});

test('a phone that wakes after the shell gave up tries again by itself', async ({ browser, relay }) => {
  const { invite, stop, start } = await hostWithSite(relay);
  const page = await h.newVisitor(browser);
  await page.goto(invite('mom'));
  await h.siteText(page);
  await page.evaluate(() => (window.notReloaded = true));

  await stop(); // the tries run out while the phone is asleep
  await h.pillSays(page, /^Can't reach the site\. Tap to reload\.$/, 70000);
  await start();
  await h.wake(page);
  await h.pillSays(page, /^connected in \d+ ms · page \d+ ms · (relay|direct)/, 20000);
  assert.equal(await page.$eval('#status', (el) => el.getAttribute('role')), null, 'the pill is still a button');
  assert.match(await h.siteFetch(page, '/bytes/1000'), /^200 1000$/);
  assert.ok(await page.evaluate(() => window.notReloaded), 'the shell reloaded');
});

/** A folder to share: a subfolder with a photo, a zip, and notes. */
function aFolder() {
  const root = mkdtempSync(join(tmpdir(), 'shared-'));
  mkdirSync(join(root, 'Holiday photos'));
  const png = 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==';
  writeFileSync(join(root, 'Holiday photos', 'beach.png'), Buffer.from(png, 'base64'));
  writeFileSync(join(root, 'Trip photos.zip'), 'PK zip bytes');
  writeFileSync(join(root, 'notes.txt'), 'hello friends');
  return root;
}

async function visitSharedFolder(path, { browser, relay }) {
  const host = await h.startShare(path, relay);
  const page = await h.newVisitor(browser);
  const downloads = h.watchDownloads(page);
  await page.goto(h.tooob(['invite', host.name, 'friend']));
  const frame = await (await page.waitForSelector('iframe', { timeout: 20000 })).contentFrame();
  return { page, frame, downloads };
}

/** A folder of web pages to share: a front page linking to a page in a subfolder, and one with no title. */
function aWebsite() {
  const root = mkdtempSync(join(tmpdir(), 'website-'));
  mkdirSync(join(root, 'trip'));
  writeFileSync(join(root, 'index.html'), '<!doctype html><title>Our holiday</title><p id="site">front page</p><a id="next" href="trip/day-one.html">Day one</a>');
  writeFileSync(join(root, 'trip', 'day-one.html'), '<!doctype html><title>Day one</title><p id="site">day one</p><a id="next" href="../untitled.html">More</a>');
  writeFileSync(join(root, 'untitled.html'), '<!doctype html><p id="site">no title</p>');
  return root;
}

test('a link in the Site puts the address bar on the page shown, a reload stays there, and Back goes back', async ({ browser, relay }) => {
  const { page, frame } = await visitSharedFolder(aWebsite(), { browser, relay });
  await frame.waitForSelector('#next', { timeout: 20000 });
  await Promise.all([frame.waitForNavigation(), frame.click('#next')]);
  await addressIs(page, '/trip/day-one.html');

  await page.reload();
  assert.equal(await h.siteText(page), 'day one');
  assert.equal(new URL(page.url()).pathname, '/trip/day-one.html');

  const site = await siteFrame(page);
  await Promise.all([site.waitForNavigation(), site.click('#next')]);
  await addressIs(page, '/untitled.html');
  await Promise.all([site.waitForNavigation(), site.evaluate(() => history.back())]);
  await addressIs(page, '/trip/day-one.html');
});

test("the tab's title follows the Site's, and is tooob before the Site has one", async ({ browser, relay }) => {
  const host = await h.startShare(aWebsite(), relay);
  const page = await h.newVisitor(browser);
  await page.goto(h.tooob(['invite', host.name, 'friend', '--approve']));
  await h.pillSays(page, /Waiting for approval/, 20000);
  assert.equal(await page.title(), 'tooob', 'while waiting for approval');

  h.tooob(['approve', host.name, h.tooob(['visitors', host.name]).split(/\s+/)[0]]);
  assert.equal(await h.siteText(page), 'front page');
  await titleIs(page, 'Our holiday');
  const site = await siteFrame(page);
  await Promise.all([site.waitForNavigation(), site.click('#next')]);
  await titleIs(page, 'Day one');
  await Promise.all([site.waitForNavigation(), site.click('#next')]);
  await titleIs(page, 'tooob'); // a page with no <title>
});

test("the tab's title follows a title the Site changes without moving", async ({ browser, relay }) => {
  const { page, frame } = await visitSharedFolder(aWebsite(), { browser, relay });
  await frame.waitForSelector('#next', { timeout: 20000 });
  await titleIs(page, 'Our holiday');
  await frame.evaluate(() => (document.title = '(3) Our holiday')); // an unread count
  await titleIs(page, '(3) Our holiday');
  await frame.evaluate(() => (document.querySelector('title').firstChild.data = 'Our holiday, seen'));
  await titleIs(page, 'Our holiday, seen');
  await frame.evaluate(() => document.querySelector('title').remove());
  await titleIs(page, 'tooob');
});

test("a Site page rewritten with document.open() is still followed: its title, its #anchors and Back", async ({ browser, relay }) => {
  const { page, frame } = await visitSharedFolder(aWebsite(), { browser, relay });
  await frame.waitForSelector('#next', { timeout: 20000 });
  await titleIs(page, 'Our holiday');
  await frame.evaluate(() => {
    document.open();
    document.write('<!doctype html><title>Rewritten</title><p id="site">rewritten</p>');
    document.close();
  });
  await titleIs(page, 'Rewritten');
  const rewritten = await siteFrame(page);
  await rewritten.evaluate(() => (document.title = '(1) Rewritten'));
  await titleIs(page, '(1) Rewritten');
  // document.open() erased every listener on the page and its window.
  await rewritten.evaluate(() => (location.hash = 'beach'));
  await addressIs(page, '/#beach');
  await rewritten.evaluate(() => history.back());
  await addressIs(page, '/');
});

test("the tab's title is written only when the Site's changes, not on every change to its <head>", async ({ browser, relay }) => {
  // CSS-in-JS adds and changes <style>s in <head> all the time.
  const { page, frame } = await visitSharedFolder(aWebsite(), { browser, relay });
  await frame.waitForSelector('#next', { timeout: 20000 });
  await titleIs(page, 'Our holiday');
  await page.evaluate(() => {
    const real = Object.getOwnPropertyDescriptor(Document.prototype, 'title');
    window.titleWrites = 0;
    Object.defineProperty(document, 'title', { get: () => real.get.call(document), set: (v) => (window.titleWrites++, real.set.call(document, v)) });
  });
  await frame.evaluate(async () => {
    for (let i = 0; i < 20; i++) {
      document.head.append(Object.assign(document.createElement('style'), { textContent: `.c${i} { color: red }` }));
      await new Promise((r) => setTimeout(r));
    }
  });
  await frame.evaluate(() => (document.title = 'Our holiday, seen'));
  await titleIs(page, 'Our holiday, seen');
  assert.equal(await page.evaluate(() => window.titleWrites), 1);
});

test("a Site whose history instrumentation wraps pushState before it loads still moves the address bar", async ({ browser, relay }) => {
  // As Sentry, GTM and single-spa do: history.pushState, wrapped early, as the page's own.
  const site = aWebsite();
  writeFileSync(join(site, 'app.html'), '<!doctype html><head><title>App</title><script>const real = history.pushState; history.pushState = function (...args) { return real.apply(this, args); };</script></head><p id="site">app</p>');
  const { page, frame } = await visitSharedFolder(site, { browser, relay });
  await frame.waitForSelector('#next', { timeout: 20000 });
  await Promise.all([frame.waitForNavigation(), frame.evaluate(() => (location.href = '/app.html'))]);
  await addressIs(page, '/app.html');
  await (await siteFrame(page)).evaluate(() => history.pushState({}, '', '/app/inbox'));
  await addressIs(page, '/app/inbox');
});

test("a Site page that never finishes loading is followed all the same, and announces the page after it", async ({ browser, relay }) => {
  const never = createServer(() => {}); // an image that never comes
  await new Promise((r) => never.listen(0, '127.0.0.1', r));
  try {
    const site = aWebsite();
    writeFileSync(join(site, 'live.html'), `<!doctype html><title>Live</title><p id="site">live</p><img src="http://127.0.0.1:${never.address().port}/photo.png">`);
    const { page, frame } = await visitSharedFolder(site, { browser, relay });
    await frame.waitForSelector('#next', { timeout: 20000 });
    await frame.evaluate(() => (location.href = '/live.html'));
    await page.waitForFunction(() => document.querySelector('iframe').contentDocument?.getElementById('site')?.textContent === 'live', null, { timeout: 20000 });
    await addressIs(page, '/live.html');
    await titleIs(page, 'Live');
    await (await siteFrame(page)).evaluate(() => history.pushState({}, '', '/live.html#now'));
    await addressIs(page, '/live.html#now');

    // Leaving it, the shell tells the service worker the next page is this tab's (#11).
    await page.evaluate(() => {
      const sw = navigator.serviceWorker.controller;
      const real = sw.postMessage.bind(sw);
      window.announced = 0;
      sw.postMessage = (message, ...rest) => (message === 'loading-site' && window.announced++, real(message, ...rest));
    });
    await (await siteFrame(page)).evaluate(() => (location.href = '/trip/day-one.html'));
    await addressIs(page, '/trip/day-one.html');
    assert.equal(await page.evaluate(() => window.announced), 1);
  } finally {
    never.closeAllConnections();
    never.close();
  }
});

test("a blob: page in the Site's frame isn't one of its pages: the address and the title stay on the last", async ({ browser, relay }) => {
  const { page, frame } = await visitSharedFolder(aWebsite(), { browser, relay });
  await frame.waitForSelector('#next', { timeout: 20000 });
  await titleIs(page, 'Our holiday');
  await frame.evaluate(() => (location.href = URL.createObjectURL(new Blob(['<title>A blob</title><p>made here</p>'], { type: 'text/html' }))));
  // Once the blob page has loaded, and the shell has heard so.
  await page.waitForFunction(() => {
    const site = document.querySelector('iframe').contentWindow;
    return site.location.protocol === 'blob:' && site.document.readyState === 'complete';
  });
  await page.evaluate(() => new Promise((r) => setTimeout(r, 200)));
  assert.equal(new URL(page.url()).pathname, '/');
  assert.equal(await page.title(), 'Our holiday');
});

test("a Bundle file isn't a page of the Site's: the address and the title stay on the last", async ({ browser, relay }) => {
  // ADR 0023: /__tooob/… and /tooob-sw.js are the Bundle's, and a reload there would show the file.
  const { page, frame } = await visitSharedFolder(aWebsite(), { browser, relay });
  await frame.waitForSelector('#next', { timeout: 20000 });
  await titleIs(page, 'Our holiday');
  await frame.evaluate(() => history.pushState({}, '', '/tooob-sw.js'));
  await frame.evaluate(() => (location.href = '/__tooob/version.json'));
  // Once the file has loaded, and the shell has heard so.
  await page.waitForFunction(() => {
    const site = document.querySelector('iframe').contentWindow;
    return site.location.pathname === '/__tooob/version.json' && site.document.readyState === 'complete';
  });
  await page.evaluate(() => new Promise((r) => setTimeout(r, 200)));
  assert.equal(new URL(page.url()).pathname, '/');
  assert.equal(await page.title(), 'Our holiday');
});

test('a Visitor browses a Shared Folder into a subfolder and opens a photo', async ({ browser, relay }) => {
  const { frame } = await visitSharedFolder(aFolder(), { browser, relay });
  await frame.waitForSelector('a[href="Holiday%20photos/"]', { timeout: 20000 });
  await Promise.all([frame.waitForNavigation(), frame.click('a[href="Holiday%20photos/"]')]);
  await Promise.all([frame.waitForNavigation(), frame.click('a[href="beach.png"]')]);
  assert.equal(await frame.evaluate(() => document.contentType), 'image/png', 'the browser shows the photo');
});

test("a file in a Shared Folder that the browser can't show downloads under its own name", async ({ browser, relay }) => {
  const { frame, downloads } = await visitSharedFolder(aFolder(), { browser, relay });
  await frame.waitForSelector('a[href="Trip%20photos.zip"]', { timeout: 20000 });
  const download = downloads.next();
  await frame.click('a[href="Trip%20photos.zip"]');
  const { name, bytes } = await download;
  assert.equal(name, 'Trip photos.zip');
  assert.equal(bytes.toString(), 'PK zip bytes');
});

test('a single shared photo opens in the browser', async ({ browser, relay }) => {
  const { frame } = await visitSharedFolder(join(aFolder(), 'Holiday photos', 'beach.png'), { browser, relay });
  await frame.waitForFunction(() => document.contentType === 'image/png', null, { timeout: 20000 });
});

test('a single shared zip downloads under its own name', async ({ browser, relay }) => {
  const { downloads } = await visitSharedFolder(join(aFolder(), 'Trip photos.zip'), { browser, relay });
  const { name, bytes } = await downloads.next();
  assert.equal(name, 'Trip photos.zip');
  assert.equal(bytes.toString(), 'PK zip bytes');
});
