// Spike criterion 5: Home Assistant's UI loads through tooob and updates live over the WebSocket shim.
// usage: node check-ha.mjs <ha-host-id or visit-url>   (run ha-setup.mjs once first)
import { readFileSync } from 'node:fs';
import puppeteer from 'puppeteer-core';

const [host] = process.argv.slice(2);
if (!host) throw new Error('usage: node check-ha.mjs <ha-host-id or visit-url>');
const ORIGIN = 'http://localhost:8088';
// Access tokens expire after 30 min; trade the stored refresh token for a fresh one each run.
const saved = JSON.parse(readFileSync('ha-tokens.json', 'utf8'));
const fresh = await fetch('http://127.0.0.1:8123/auth/token', {
  method: 'POST',
  body: new URLSearchParams({ grant_type: 'refresh_token', refresh_token: saved.refresh_token, client_id: ORIGIN + '/' }),
}).then((r) => r.json());
const tok = { ...saved, ...fresh };

const browser = await puppeteer.launch({ executablePath: '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome', headless: true });
const page = await browser.newPage();
page.on('console', (m) => process.env.VERBOSE && console.log('  console:', m.text()));
// Hand the HA frontend its tokens the way its own login would store them.
await page.evaluateOnNewDocument((t, origin) => {
  localStorage.hassTokens = JSON.stringify({
    access_token: t.access_token, refresh_token: t.refresh_token, token_type: 'Bearer',
    expires_in: t.expires_in, expires: Date.now() + t.expires_in * 1000, hassUrl: origin, clientId: origin + '/',
  });
}, tok, ORIGIN);

const t0 = Date.now();
await page.goto(host.startsWith("http") ? host : `${ORIGIN}/#${host}`);
const frame = await (await page.waitForSelector('iframe')).contentFrame();
let ok = true;
try {
  await frame.waitForFunction(() => document.querySelector('home-assistant')?.hass?.connected, { timeout: 60000 });
  console.log(`PASS  5a. HA connected over tooob WebSocket  ${Date.now() - t0} ms from shell load`);
  // The dashboard itself: main shell rendered and the default panel past its loading state.
  // HA removes its launch screen only once the first panel has rendered.
  await frame.waitForFunction(() => !document.getElementById('ha-launch-screen'), { timeout: 60000 });
  console.log(`PASS  5b. HA dashboard rendered  ${Date.now() - t0} ms from shell load`);
} catch (e) {
  console.log(`FAIL  5a/b. HA UI did not finish loading: ${e.message}`);
  ok = false;
}

if (ok) {
  // Change a state behind the UI's back and time how long until the UI shows it.
  const value = String(Date.now() % 100000);
  const t1 = Date.now();
  const r = await fetch('http://127.0.0.1:8123/api/states/sensor.tooob_live', {
    method: 'POST', headers: { authorization: `Bearer ${tok.access_token}`, 'content-type': 'application/json' },
    body: JSON.stringify({ state: value }),
  });
  if (!r.ok) throw new Error(`state update failed: ${r.status}`);
  try {
    await frame.waitForFunction((v) => document.querySelector('home-assistant').hass.states['sensor.tooob_live']?.state === v, { timeout: 10000 }, value);
    console.log(`PASS  5c. live state update reached the UI  ${Date.now() - t1} ms`);
  } catch {
    console.log('FAIL  5c. live state update never reached the UI');
    ok = false;
  }
}
if (ok) {
  // Second visit: hashed assets should come from the service worker's cache, not the tunnel.
  let tunneled = 0;
  page.on('console', (m) => /^\[tooob\] GET /.test(m.text()) && tunneled++);
  const t2 = Date.now();
  await page.reload();
  const f2 = await (await page.waitForSelector('iframe')).contentFrame();
  await f2.waitForFunction(() => document.querySelector('home-assistant')?.hass?.connected, { timeout: 60000 });
  const wsMs = Date.now() - t2;
  await f2.waitForFunction(() => !document.getElementById('ha-launch-screen'), { timeout: 60000 });
  const s = await page.evaluate(() => window.__tooobStats);
  console.log(`info  5d. warm visit: dashboard in ${Date.now() - t2} ms (shell: connect ${s.connectMs} ms, page ${s.pageMs} ms; HA WebSocket at ${wsMs} ms), ${tunneled} requests over tooob`);
}
await page.screenshot({ path: 'ha.png' });
await browser.close();
process.exit(ok ? 0 : 1);
