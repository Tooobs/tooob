// The browser side of an Invite Link, end to end. Run: node --test (after ../build.sh).
import assert from 'node:assert/strict';
import { mkdirSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { createServer, get } from 'node:http';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { after, before, test } from 'node:test';
import * as h from './harness.mjs';

// iroh-relay itself, and the Relay Hosts name: a front that hands iroh's paths to it and serves the
// Bootstrap for everything else, as a Relay Operator runs one (ADR 0020).
let irohRelay, relay, browser;

before(async () => {
  h.buildHost();
  [irohRelay, browser] = await Promise.all([h.startRelay(), h.launchBrowser()]);
  relay = (await h.startRelayFront(irohRelay)).url;
});

after(async () => {
  await browser?.close();
  h.stopAll();
});

async function hostWithSite() {
  const site = await h.startSite();
  let host = await h.startHost(site, relay);
  const { name } = host;
  const invite = (label, ...flags) => h.tooob(['invite', name, label, ...flags]);
  const start = async () => (host = await h.startHost(site, relay, name)); // the same Host: same Host Key and Allowlist
  const restart = async () => {
    await host.stop();
    await start();
  };
  return { site, name, invite, start, restart, stop: () => host.stop(), pause: () => host.pause(), resume: () => host.resume() };
}

test('an Invite Link loads the Site and drops the token from the address bar', async () => {
  const { invite } = await hostWithSite();
  const page = await h.newVisitor(browser);
  await page.goto(invite('mom'));
  assert.match(await h.siteText(page), /site \/ label=mom/);
  assert.ok(!page.url().includes('invite='), `token still in ${page.url()}`);
});

test('reopening a used Invite Link in another browser says so and never reaches the Site', async () => {
  const { site, invite } = await hostWithSite();
  const link = invite('mom');
  const first = await h.newVisitor(browser);
  await first.goto(link);
  await h.siteText(first);
  const requestsBefore = site.requests;

  const second = await h.newVisitor(browser);
  await second.goto(link);
  await second.waitForFunction(() => /already been used/.test(document.getElementById('status').textContent), { timeout: 20000 });
  assert.equal(site.requests, requestsBefore, 'the second browser reached the Site');
});

test('an approve-first Invite waits, then opens by itself once the owner approves', async () => {
  const { name, invite } = await hostWithSite();
  const page = await h.newVisitor(browser);
  await page.goto(invite('mom', '--approve'));
  await page.waitForFunction(() => /Waiting for approval/.test(document.getElementById('status').textContent), { timeout: 20000 });

  const key = h.tooob(['visitors', name]).split(/\s+/)[0];
  h.tooob(['approve', name, key]);
  assert.match(await h.siteText(page), /label=mom/, 'the page opened by itself');
});

test('after redeeming, a reload still gets in (the device is remembered)', async () => {
  const { invite } = await hostWithSite();
  const page = await h.newVisitor(browser);
  await page.goto(invite('mom'));
  await h.siteText(page);
  await page.reload();
  assert.match(await h.siteText(page), /label=mom/);
});

test('a browser with a Visitor Key from before the key store keeps its identity', async () => {
  const { name, invite } = await hostWithSite();
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

test("the Site's own scripts can't read the Visitor Key", async () => {
  const { invite } = await hostWithSite();
  const page = await h.newVisitor(browser);
  await page.goto(invite('mom'));
  await h.siteText(page);
  const found = await h.whatTheSiteCanReach(page);
  assert.deepEqual(found.localStorage, {}, 'the shell left something in localStorage');
  assert.deepEqual(found.privateKeys, ['Ed25519'], 'expected the Visitor Key in the key store');
  assert.deepEqual(found.exported, [], 'the Site exported the Visitor Key');
});

test("a link without an Invite tells a stranger they're not invited", async () => {
  const { site, invite } = await hostWithSite();
  const plain = invite('mom').replace(/#invite=.*/, ''); // the Host's Key Address, minus the Invite
  const page = await h.newVisitor(browser);
  await page.goto(plain);
  await page.waitForFunction(() => /not invited/.test(document.getElementById('status').textContent), { timeout: 20000 });
  assert.equal(site.requests, 0);
});

test('re-tapping your own Invite Link later still opens the Site', async () => {
  const { invite } = await hostWithSite();
  const link = invite('mom');
  const page = await h.newVisitor(browser);
  await page.goto(link);
  await h.siteText(page);
  await page.goto('about:blank');
  await page.goto(link); // from her email again, token and all
  assert.match(await h.siteText(page), /label=mom/);
});

test('the Invite token never ends up in browser storage', async () => {
  const { invite } = await hostWithSite();
  const page = await h.newVisitor(browser);
  await page.goto(invite('mom'));
  await h.siteText(page);
  const caches = await page.evaluate(() => caches.keys());
  assert.ok(caches.length > 0, 'expected the shell to have opened its cache');
  assert.ok(caches.every((name) => !name.includes('invite')), `cache names: ${caches}`);
});

test("the injected WebSocket shim keeps the Site's page in standards mode", async () => {
  const { invite } = await hostWithSite(); // the stand-in Site serves <!doctype html> with no <head>
  const page = await h.newVisitor(browser);
  await page.goto(invite('mom'));
  await h.siteText(page);
  const frame = await (await page.$('iframe')).contentFrame();
  assert.equal(await frame.evaluate(() => document.compatMode), 'CSS1Compat', 'quirks mode: shim landed before the doctype');
  assert.equal(await frame.evaluate(() => typeof window.WebSocket.prototype.send), 'function');
});

test('a page waiting for approval survives the Host restarting', async () => {
  const { name, invite, restart } = await hostWithSite();
  const page = await h.newVisitor(browser);
  await page.goto(invite('mom', '--approve'));
  await page.waitForFunction(() => /Waiting for approval/.test(document.getElementById('status').textContent), { timeout: 20000 });

  await restart(); // the page's connection drops while it's waiting
  const key = h.tooob(['visitors', name]).split(/\s+/)[0];
  h.tooob(['approve', name, key]);
  assert.match(await h.siteText(page, 40000), /label=mom/, 'the page reconnected and opened by itself');
});

test('an Invite Link is its Host\'s Key Address, on its Relay\'s domain', async () => {
  const { name, invite } = await hostWithSite();
  const key = h.tooob(['show', name]).match(/Host Key\s+(\S+)/)[1];
  const link = new URL(invite('mom'));
  assert.equal(link.origin, `http://${key}.localhost:${new URL(relay).port}`);
  assert.match(link.hash, /^#invite=[^&]+$/);
  assert.equal(link.search, '', 'no ?relay=: the Relay is the link\'s own domain');
});

test("two Sites through one Relay are two origins: neither can reach the other's storage or Visitor Key", async () => {
  const x = await hostWithSite();
  const y = await hostWithSite();
  const tabX = await h.newVisitor(browser);
  const tabY = await tabX.browserContext().newPage(); // the same browser, so the same profile
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

test("a cookie a Site sets for the whole Relay domain stays with that Site", async () => {
  // Without the Public Suffix List, the browser lets a Key Address set a cookie for its whole Relay
  // domain, which every other Host's pages would then see (ADR 0021).
  const shared = (await h.startRelayFront(irohRelay, 'relay.localhost')).url;
  const a = await h.startHost(await h.startSite(), shared);
  const b = await h.startHost(await h.startSite(), shared);
  const tabA = await h.newVisitor(browser);
  const tabB = await tabA.browserContext().newPage(); // the same browser, so the same cookies
  await tabA.goto(h.tooob(['invite', a.name, 'mom']));
  await h.siteText(tabA);
  const siteA = await (await tabA.$('iframe')).contentFrame();
  await siteA.evaluate(async () => {
    document.cookie = 'viaDocument=a; Domain=relay.localhost; Path=/';
    await cookieStore.set({ name: 'viaCookieStore', value: 'a', domain: 'relay.localhost', path: '/' });
  });
  // Where the browser keeps them, before any shell of Site B's could clean up after Site A.
  const kept = (await tabA.browserContext().cookies()).map((c) => `${c.name} for ${c.domain}`).sort();
  const keyAddress = new URL(tabA.url()).hostname;
  assert.deepEqual(kept, [`viaCookieStore for ${keyAddress}`, `viaDocument for ${keyAddress}`]);

  await tabB.goto(h.tooob(['invite', b.name, 'dad']));
  await h.siteText(tabB);
  const siteB = await (await tabB.$('iframe')).contentFrame();
  assert.equal(await siteB.evaluate(() => document.cookie), '', "Site B read Site A's cookies");
});

test('a cookie for the whole Relay domain is gone once a shell has started, and the Site keeps its own', async () => {
  // Set where the injected script doesn't reach, such as a Site's own service worker (ADR 0021).
  const shared = (await h.startRelayFront(irohRelay, 'relay.localhost')).url;
  const host = await h.startHost(await h.startSite(), shared);
  const link = h.tooob(['invite', host.name, 'mom']);
  const tab = await h.newVisitor(browser);
  const jar = tab.browserContext();
  await jar.setCookie(
    { name: 'stray', value: 'relay-wide', domain: '.relay.localhost', path: '/' },
    { name: '__Secure-stray', value: 'relay-wide', domain: '.relay.localhost', path: '/', secure: true }, // deleted only by a Secure deletion
    { name: 'stray', value: 'mine', domain: new URL(link).hostname, path: '/' },
  );
  await tab.goto(link);
  await h.siteText(tab);
  const left = (await jar.cookies()).map((c) => `${c.name}=${c.value} for ${c.domain}`);
  assert.deepEqual(left, [`stray=mine for ${new URL(link).hostname}`]);
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
    const ask = (path, headers) => new Promise((resolve) => {
      get({ host: '127.0.0.1', port: front.port, path, headers: { cookie: 'bomb=x', ...headers } }, (res) => res.resume().on('end', () => resolve(res)))
        .on('upgrade', (res, socket) => (socket.destroy(), resolve(res)));
    });
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

test('two tabs on two different Sites each get their own Site', async () => {
  const x = await hostWithSite();
  const y = await hostWithSite();
  const tabX = await h.newVisitor(browser);
  const tabY = await tabX.browserContext().newPage(); // same browser, so same service worker
  await tabX.goto(x.invite('mom'));
  assert.match(await h.siteText(tabX), /label=mom/);
  await tabY.goto(y.invite('dad'));
  assert.match(await h.siteText(tabY), /label=dad/, "tab Y's requests went to tab X's Site");
  await tabX.reload();
  assert.match(await h.siteText(tabX), /label=mom/);
});

test('two tabs on the same Site both keep working', async () => {
  const { invite } = await hostWithSite();
  const first = await h.newVisitor(browser);
  await first.goto(invite('mom'));
  await h.siteText(first);
  const second = await first.browserContext().newPage();
  await second.goto(first.url()); // the plain link, as a second tab would open it
  assert.match(await h.siteText(second), /label=mom/);
  await first.reload(); // the first tab still gets through
  assert.match(await h.siteText(first), /label=mom/);
});

test("a tab still waiting for approval never swallows another tab's requests", async () => {
  const x = await hostWithSite();
  const y = await hostWithSite();
  const openWaiting = async (page) => {
    await page.goto(x.invite('mom', '--approve'));
    await page.waitForFunction(() => /Waiting for approval/.test(document.getElementById('status').textContent), { timeout: 20000 });
  };
  // Waiting tabs on both sides of the allowed one, so tab order can't hide a wrong pick.
  const first = await h.newVisitor(browser);
  await openWaiting(first);
  const allowed = await first.browserContext().newPage();
  await allowed.goto(y.invite('dad'));
  await h.siteText(allowed);
  await openWaiting(await first.browserContext().newPage());

  // The browser stops idle service workers, forgetting which tab is whose.
  const cdp = await allowed.createCDPSession();
  await cdp.send('ServiceWorker.enable');
  await cdp.send('ServiceWorker.stopAllWorkers');
  const frame = await (await allowed.$('iframe')).contentFrame();
  const text = await frame.evaluate(() => Promise.race([
    fetch('/again').then((r) => r.text()),
    new Promise((r) => setTimeout(() => r('hung'), 10000)),
  ]));
  assert.match(text, /site \/again label=dad/);
});

test("the Site's traffic goes over the Direct Path, not the Relay", async () => {
  const { invite } = await hostWithSite();
  const page = await h.newVisitor(browser);
  await page.goto(invite('mom'));
  await h.siteText(page);
  await page.waitForFunction(() => window.__tooobStats.path === 'direct', { timeout: 20000 });
  const bytes = await h.bytesForReload(page);
  assert.ok(bytes.direct > 0, `nothing came direct: ${JSON.stringify(bytes)}`);
  assert.equal(bytes.relay, 0, `the Relay carried Site bytes: ${JSON.stringify(bytes)}`);
});

test('after the phone sleeps, the Site still loads and goes back onto the Direct Path', async () => {
  const { invite } = await hostWithSite();
  const page = await h.newVisitor(browser);
  await page.goto(invite('mom'));
  await h.siteText(page);
  await page.waitForFunction(() => window.__tooobStats.path === 'direct', { timeout: 10000 });

  await h.sleepBrowser(browser, 35); // past iroh's 30 s idle timeout: the Host drops the connection
  assert.equal(await h.siteFetch(page, '/bytes/20000000'), '200 20000000', 'a download right after waking');
  await page.waitForFunction(() => window.__tooobStats.path === 'direct', { timeout: 20000 });
  await page.browserContext().close();
});

test("after a phone's deep sleep silences its Relay connection, the Site loads as soon as it wakes", async () => {
  // Firefox for Android in Doze: the page and iroh's timers freeze, the network drops the old
  // flows, and on waking iroh still thinks its connection is up, so a request waits ~30 s for
  // its idle timeout. Relay only, so nothing can come by the Direct Path instead.
  const { invite } = await hostWithSite();
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

test('a Direct Path that drops and comes back carries the Site again, without a reload', async () => {
  const { invite, pause, resume } = await hostWithSite();
  const page = await h.newVisitor(browser);
  await page.goto(invite('mom'));
  await h.siteText(page);
  await page.waitForFunction(() => window.__tooobStats.path === 'direct', { timeout: 20000 });

  pause(); // the browser's ICE checks go unanswered, as when a phone backgrounds the tab
  try {
    await page.waitForFunction(() => /paused/.test(window.__tooobStats.path), { timeout: 20000 });
  } finally {
    resume();
  }
  await page.waitForFunction(() => window.__tooobStats.path === 'direct', { timeout: 20000 });
  const bytes = await h.bytesForReload(page);
  assert.ok(bytes.direct > 0, `nothing came direct: ${JSON.stringify(bytes)}`);
  assert.equal(bytes.relay, 0, `the Relay carried Site bytes: ${JSON.stringify(bytes)}`);
});

test('a Direct Path that fails mid-visit is renegotiated each time, and a download in flight survives', async () => {
  const { invite } = await hostWithSite();
  const page = await h.newVisitor(browser);
  await h.watchDirectPath(page);
  await page.goto(invite('mom'));
  await h.siteText(page);
  await page.waitForFunction(() => window.__tooobStats.path === 'direct', { timeout: 20000 });
  const frame = await (await page.$('iframe')).contentFrame();
  const download = frame.evaluate(() => fetch('/bytes/20971520').then((r) => r.arrayBuffer()).then((b) => b.byteLength));

  for (let i = 0; i < 5; i++) { // more failures than any retry budget
    assert.notEqual(await h.failDirectPath(page), 'direct', `failure ${i + 1} went unnoticed`);
    await page.waitForFunction(() => window.__tooobStats.path === 'direct', { timeout: 3000 });
  }
  assert.equal(await download, 20971520);
  const bytes = await h.bytesForReload(page);
  assert.ok(bytes.direct > 0, `nothing came direct: ${JSON.stringify(bytes)}`);
  assert.equal(bytes.relay, 0, `the Relay carried Site bytes: ${JSON.stringify(bytes)}`);
});

test('the pill says the Direct Path is on the home network', async () => {
  const { invite } = await hostWithSite();
  const page = await h.newVisitor(browser);
  await page.goto(invite('mom'));
  await h.siteText(page);
  await page.waitForFunction(() => / · direct · LAN$/.test(document.getElementById('status').textContent), { timeout: 20000 });
  assert.doesNotMatch(await h.statusText(page), /\d+\.\d+\.\d+\.\d+|::/, 'the pill shows an address');
});

test('the pill says a Direct Path from elsewhere was punched through NAT, without its address', async () => {
  const { invite } = await hostWithSite();
  const page = await h.newVisitor(browser);
  await h.forgeNatPunched(page);
  await page.goto(invite('mom'));
  await h.siteText(page);
  await page.waitForFunction(() => / · direct · NAT punched$/.test(document.getElementById('status').textContent), { timeout: 20000 });
  assert.doesNotMatch(await h.statusText(page), /203\.0\.113/);
});

test('the pill says the Site is coming over the Relay when the Direct Path fails', async () => {
  const { invite } = await hostWithSite();
  const page = await h.newVisitor(browser);
  await h.breakDirectPath(page);
  await page.goto(invite('mom'));
  await h.siteText(page);
  await page.waitForFunction(() => / · relay$/.test(document.getElementById('status').textContent), { timeout: 20000 });
});

test('when the Direct Path fails, the Site still loads over the Relay', async () => {
  const { invite } = await hostWithSite();
  const page = await h.newVisitor(browser);
  await h.breakDirectPath(page);
  await page.goto(invite('mom'));
  assert.match(await h.siteText(page), /label=mom/);
  await page.waitForFunction(() => window.__tooobStats.path === 'relay', { timeout: 20000 });
  const bytes = await h.bytesForReload(page);
  assert.ok(bytes.relay > 0, `nothing came over the Relay: ${JSON.stringify(bytes)}`);
  assert.equal(bytes.direct, 0);
});

test('a Direct Path attempt that fails is retried, and the Site moves onto it', async () => {
  const { invite } = await hostWithSite();
  const page = await h.newVisitor(browser);
  await h.breakDirectPath(page, 1); // like an attempt right after a network switch
  await page.goto(invite('mom'));
  assert.match(await h.siteText(page), /label=mom/);
  await page.waitForFunction(() => window.__tooobStats.path === 'direct', { timeout: 30000 });
  const bytes = await h.bytesForReload(page);
  assert.ok(bytes.direct > 0, `nothing came direct: ${JSON.stringify(bytes)}`);
  assert.equal(bytes.relay, 0, `the Relay carried Site bytes: ${JSON.stringify(bytes)}`);
});

test('the first page waits at most ~800 ms for a Direct Path that never comes', async () => {
  const { invite } = await hostWithSite();
  const page = await h.newVisitor(browser);
  await h.breakDirectPath(page); // it never connects, so only the cap ends the wait
  await page.goto(invite('mom'));
  await h.siteText(page);
  const waited = await page.evaluate(() => window.__tooobStats.directWaitMs);
  assert.ok(waited >= 700 && waited <= 1000, `waited ${waited} ms`);
});

test("the browser refuses a Direct Path whose certificate isn't the one the Host sent over iroh", async () => {
  const { invite } = await hostWithSite();
  const page = await h.newVisitor(browser);
  await h.forgeDirectFingerprint(page);
  await page.goto(invite('mom'));
  assert.match(await h.siteText(page), /label=mom/);
  await page.waitForFunction(() => window.__tooobStats.path === 'relay', { timeout: 20000 });
  const bytes = await h.bytesForReload(page);
  assert.equal(bytes.direct, 0, 'Site bytes went over the forged Direct Path');
});

test('bursts over the Direct Path never stall (SCTP loses a burst and waits >= 1 s to resend)', async () => {
  const { invite } = await hostWithSite();
  const page = await h.newVisitor(browser);
  await page.goto(invite('mom'));
  await h.siteText(page);
  await page.waitForFunction(() => window.__tooobStats.path === 'direct', { timeout: 20000 });
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

test('a refused upload leaves the Direct Path carrying the next request', async () => {
  const { name, invite } = await hostWithSite();
  const page = await h.newVisitor(browser);
  await page.goto(invite('mom'));
  await h.siteText(page);
  await page.waitForFunction(() => window.__tooobStats.path === 'direct', { timeout: 20000 });
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

test('a 20 MB upload goes through on the Direct Path', async () => {
  const { invite } = await hostWithSite();
  const page = await h.newVisitor(browser);
  await page.goto(invite('mom'));
  await h.siteText(page);
  await page.waitForFunction(() => window.__tooobStats.path === 'direct', { timeout: 20000 });
  const before = await page.evaluate(() => window.__tooobStats.bytes.direct);
  const frame = await (await page.$('iframe')).contentFrame();
  const reply = await frame.evaluate(() => fetch('/upload', { method: 'POST', body: new Uint8Array(20 << 20) }).then((r) => r.text()));
  assert.match(reply, /received 20971520/);
  assert.ok((await page.evaluate(() => window.__tooobStats.bytes.direct)) > before, 'the upload left the Direct Path');
});

test("when the Host can't be reached the page says so and keeps the Invite for a retry", async () => {
  const { invite, stop } = await hostWithSite();
  const link = invite('mom');
  await stop();
  const page = await h.newVisitor(browser);
  await page.goto(link);
  await page.waitForFunction(() => /Couldn't reach/.test(document.getElementById('status').textContent), { timeout: 60000 });
  assert.ok(page.url().includes('invite='), 'the Invite should stay so a reload can retry');
});


test('when the Host goes away the pill says it is reconnecting, and counts the tries', async () => {
  const { invite, stop } = await hostWithSite();
  const page = await h.newVisitor(browser);
  await page.goto(invite('mom'));
  await h.siteText(page);

  const stopping = stop();
  await h.pillSays(page, /^Reconnecting… \(try 1 of 5/, 3000); // heard at once: the Host closes the connection
  await stopping;
  await h.pillSays(page, /^Reconnecting… \(try 2 of 5, next in 4 s\)$/, 20000);
  await h.pillSays(page, /^Reconnecting… \(try 2 of 5, next in 3 s\)$/, 2000);
  await h.pillSays(page, /^Reconnecting… \(try 3 of 5/, 5000);
  await page.browserContext().close(); // stop it trying
});

test('when the Host comes back during the tries, the Site loads again without a reload', async () => {
  const { invite, stop, start } = await hostWithSite();
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
  await page.waitForFunction(() => window.__tooobStats.path === 'direct', { timeout: 20000 }); // and the Direct Path is back
});

test("when the Host stays away the pill gives up and offers a reload, and tapping it reloads", async () => {
  const { invite, stop } = await hostWithSite();
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

test('a phone that wakes after the shell gave up tries again by itself', async () => {
  const { invite, stop, start } = await hostWithSite();
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

async function visitSharedFolder(path) {
  const host = await h.startShare(path, relay);
  const page = await h.newVisitor(browser);
  const downloads = await h.watchDownloads(page);
  await page.goto(h.tooob(['invite', host.name, 'friend']));
  const frame = await (await page.waitForSelector('iframe', { timeout: 20000 })).contentFrame();
  return { page, frame, downloads };
}

test('a Visitor browses a Shared Folder into a subfolder and opens a photo', async () => {
  const { frame } = await visitSharedFolder(aFolder());
  await frame.waitForSelector('a[href="Holiday%20photos/"]', { timeout: 20000 });
  await Promise.all([frame.waitForNavigation(), frame.click('a[href="Holiday%20photos/"]')]);
  await Promise.all([frame.waitForNavigation(), frame.click('a[href="beach.png"]')]);
  assert.equal(await frame.evaluate(() => document.contentType), 'image/png', 'the browser shows the photo');
});

test("a file in a Shared Folder that the browser can't show downloads under its own name", async () => {
  const { frame, downloads } = await visitSharedFolder(aFolder());
  await frame.waitForSelector('a[href="Trip%20photos.zip"]', { timeout: 20000 });
  const download = downloads.next();
  await frame.click('a[href="Trip%20photos.zip"]');
  const { name, bytes } = await download;
  assert.equal(name, 'Trip photos.zip');
  assert.equal(bytes.toString(), 'PK zip bytes');
});

test('a single shared photo opens in the browser', async () => {
  const { frame } = await visitSharedFolder(join(aFolder(), 'Holiday photos', 'beach.png'));
  await frame.waitForFunction(() => document.contentType === 'image/png', { timeout: 20000 });
});

test('a single shared zip downloads under its own name', async () => {
  const { downloads } = await visitSharedFolder(join(aFolder(), 'Trip photos.zip'));
  const { name, bytes } = await downloads.next();
  assert.equal(name, 'Trip photos.zip');
  assert.equal(bytes.toString(), 'PK zip bytes');
});
