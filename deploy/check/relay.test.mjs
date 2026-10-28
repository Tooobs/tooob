// The Relay image (deploy/relay/image, #67) that both Relay setups run, built and run in Docker here: its
// iroh-relay and its Caddy, serving the Bootstrap as built here (bundle.sh --local). Its Caddyfile
// gets certificates through a DNS challenge, which needs a real domain, so the container runs its
// front (front.caddy) on plain HTTP instead, at localhost and every <host key>.localhost: a real
// Host and headless Chrome then open an Invite Link through it. Needs Docker and Chrome. Run:
// node --test relay.test.mjs, after bootstrap/build.sh and npm install in bootstrap/check. Building
// the image the first time takes a few minutes.
import { test, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync, spawnSync } from 'node:child_process';
import { cpSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { get } from 'node:http';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import * as h from '../../bootstrap/check/harness.mjs';

const DEPLOY = join(dirname(fileURLToPath(import.meta.url)), '..');
const IMAGE = 'tooob-relay-check';
const scratch = mkdtempSync(join(tmpdir(), 'tooob-relay-image-'));
const docker = (...args) => execFileSync('docker', args, { encoding: 'utf8' }).trim();
let container, relay, port, browser;

before(async () => {
  h.buildHost();
  const ctx = join(scratch, 'image');
  cpSync(join(DEPLOY, 'relay/image'), ctx, { recursive: true });
  execFileSync(join(DEPLOY, 'bundle.sh'), [ctx, '--local']);
  execFileSync('docker', ['build', '-q', '-t', IMAGE, ctx], { stdio: ['ignore', 'ignore', 'inherit'] });
  writeFileSync(join(scratch, 'Caddyfile'), '{\n\tadmin off\n}\n\n:8080 {\n\timport /etc/caddy/front.caddy\n}\n');
  container = docker('run', '-d', '--rm', '-p', '127.0.0.1::8080', '-v', `${join(scratch, 'Caddyfile')}:/etc/caddy/Caddyfile:ro`, IMAGE);
  port = docker('port', container, '8080').split('\n')[0].split(':').pop();
  relay = `http://localhost:${port}/`;
  for (let i = 0; ; i++) {
    if ((await fetchVia('localhost', '/healthz').catch(() => ({}))).status === 200) break;
    if (i > 100) throw new Error(`the Relay never came up:\n${docker('logs', container)}`);
    await new Promise((r) => setTimeout(r, 100));
  }
  browser = await h.launchBrowser();
});

after(async () => {
  await browser?.close();
  h.stopAll();
  if (container) docker('rm', '-f', container);
  rmSync(scratch, { recursive: true, force: true });
});

/** GET `path` from the container, as a browser on `host` would ask for it. */
function fetchVia(host, path, headers = {}) {
  return new Promise((resolve, reject) => {
    get({ host: '127.0.0.1', port, path, headers: { host: `${host}:${port}`, ...headers } }, (res) => {
      let body = '';
      res.on('data', (d) => (body += d));
      res.on('end', () => resolve({ status: res.statusCode, body, headers: res.headers }));
    }).on('error', reject);
  });
}

test("the shipped Caddyfile is valid, with the DNS provider's module built in", () => {
  // Shaped like a Cloudflare API token, which the module checks for.
  const out = spawnSync('docker', ['run', '--rm', '-e', 'RELAY_DOMAIN=relay.example.com', '-e', `DNS_API_TOKEN=${'x'.repeat(40)}`,
    '-e', 'ACME_EMAIL=operator@example.com', IMAGE, 'caddy', 'validate', '--config', '/etc/caddy/Caddyfile'], { encoding: 'utf8' });
  assert.equal(out.status, 0, out.stderr);
  assert.match(out.stdout + out.stderr, /Valid configuration/);
});

test("the Relay's domain and every Key Address serve the Bundle, which browsers recheck on every load", async () => {
  const manifest = readFileSync(join(DEPLOY, '../bootstrap/www/__tooob/version.json'), 'utf8');
  for (const host of ['localhost', 'ybndrfdyb.localhost']) {
    const res = await fetchVia(host, '/__tooob/version.json');
    assert.equal(res.status, 200, host);
    assert.equal(res.body, manifest, host);
    assert.equal(res.headers['cache-control'], 'no-cache', host);
  }
});

test('every Bundle file asks browsers for a process of its own (ADR 0021)', async () => {
  const bundle = JSON.parse(readFileSync(join(DEPLOY, '../bootstrap/www/__tooob/version.json'), 'utf8'));
  for (const host of ['localhost', 'ybndrfdyb.localhost']) {
    for (const path of ['/', '/__tooob/version.json', ...Object.keys(bundle.files).map((f) => '/' + f)]) {
      const res = await fetchVia(host, path);
      assert.equal(res.status, 200, host + path);
      assert.equal(res.headers['origin-agent-cluster'], '?1', host + path);
      assert.equal(res.headers['cross-origin-opener-policy'], 'same-origin-allow-popups', host + path);
      assert.equal(res.headers['cross-origin-embedder-policy'], undefined, host + path);
    }
  }
});

test("a Site that fills the Relay's domain with cookies can't stop other Hosts' Visitors connecting (ADR 0021)", async () => {
  // As many as Chrome keeps for one domain, each as big as it allows: every request to the Relay
  // would carry all of them.
  const cookie = Array.from({ length: 180 }, (_, i) => `c${i}=${'x'.repeat(4090)}`).join('; ');
  assert.equal((await fetchVia('ybndrfdyb.localhost', '/healthz', { cookie })).status, 200);
  assert.equal((await fetchVia('ybndrfdyb.localhost', '/__tooob/version.json', { cookie })).status, 200);
});

test("iroh's paths go to iroh-relay", async () => {
  assert.equal((await fetchVia('localhost', '/healthz')).status, 200);
  assert.equal((await fetchVia('ybndrfdyb.localhost', '/generate_204')).status, 204);
});

test('a Host using the Relay is reached from a browser through an Invite Link', async () => {
  const site = await h.startSite();
  const host = await h.startHost(site, relay);
  const link = h.tooob(['invite', host.name, 'mom']);
  assert.ok(link.startsWith(`http://`) && link.includes(`.localhost:${port}/#invite=`), link);
  const page = await h.newVisitor(browser);
  await page.goto(link);
  assert.match(await h.siteText(page), /site \/ label=mom/);
});
