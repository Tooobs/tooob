// A Relay on your own network to try things on (#142): the same image, over plain http, closed,
// started and stopped by test-relay.sh. Plain http is an option off by default, so a deploy
// (r.tooobs.net's, a VPS's) serves https exactly as before. Read as config: the Relay itself is
// checked by running it.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const read = (path) => readFileSync(new URL(path, import.meta.url), 'utf8');
const code = (text) => text.split('\n').filter((l) => !l.trim().startsWith('#')).join('\n');

test('plain http is off unless RELAY_PLAIN_HTTP is set, and then serves front.caddy on one port', () => {
  const start = code(read('../relay/image/start.sh'));
  assert.match(start, /RELAY_PLAIN_HTTP/);
  assert.match(start, /\/etc\/caddy\/Caddyfile/, 'the https Caddyfile is still the default');
  const plain = code(read('../relay/image/plain-http.Caddyfile'));
  assert.match(plain, /auto_https off/);
  assert.match(plain, /^http:\/\/:8080 \{\n\timport front\.caddy\n\}/m);
  for (const path of ['../relay/compose.yaml', '../fly-relay/fly.toml', '../relay/deploy.sh', '../fly-relay/deploy.sh']) {
    assert.doesNotMatch(read(path), /RELAY_PLAIN_HTTP/, `${path} keeps https`);
  }
});

test('the image ships the plain-http Caddyfile and has Caddy parse it at build', () => {
  const dockerfile = read('../relay/image/Dockerfile');
  assert.match(dockerfile, /^COPY Caddyfile plain-http\.Caddyfile front\.caddy \/etc\/caddy\/$/m);
  assert.match(dockerfile, /caddy adapt --config \/etc\/caddy\/plain-http\.Caddyfile/);
});

test('test-relay.sh starts it with podman, its own volume, a Sign-up Page, and closes it', () => {
  const sh = code(read('../relay/test-relay.sh'));
  assert.match(sh, /podman run -d --name "\$name"/);
  assert.match(sh, /-e RELAY_PLAIN_HTTP=yes/);
  assert.match(sh, /-e TOOOB_RELAY_SIGN_UP_URL="\$sign_up"/);
  assert.match(sh, /-v "\$name-data:\/data"/);
  assert.match(sh, /-p 127\.0\.0\.1:8081:8081/, 'the admin page on loopback only');
  // Closed from the start (ADR 0030): a stand-in key enrolled while none is.
  assert.match(sh, /tooob-relay enroll "\$closer" --label/);
  assert.match(sh, /podman rm -f "\$name"/, 'stop');
  assert.doesNotMatch(sh, /fly|docker /i, 'podman only, no host platform');
});

test('the deploy README says how to run one', () => {
  const readme = read('../relay/README.md');
  assert.match(readme, /## A test Relay on your own network/);
  assert.match(readme, /\.\/test-relay\.sh start /);
  assert.match(readme, /\.\/test-relay\.sh stop/);
});
