// The Relay's admin page (#133) logs in with an Operator Token, and is never on the Relay's public
// address: tooob-relay serves it on port 8081 inside the container, which `fly proxy` reaches, and
// which a VPS publishes on its own loopback only (`ssh -L`). Never through the front, never public.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const read = (path) => readFileSync(new URL(path, import.meta.url), 'utf8');
const code = (text) => text.split('\n').filter((l) => !l.trim().startsWith('#')).join('\n');

test('the image starts tooob-relay with the admin page on port 8081', () => {
  assert.match(code(read('../relay/image/start.sh')), /^tooob-relay .*--admin-addr '\[::\]:8081'/m);
});

test("the front never routes to the admin page's port", () => {
  assert.doesNotMatch(code(read('../relay/image/front.caddy')), /8081/);
});

test('Fly publishes no service on the admin port', () => {
  assert.doesNotMatch(code(read('../fly-relay/fly.toml')), /8081/);
});

test("a VPS publishes the admin port on its own loopback only", () => {
  const ports = code(read('../relay/compose.yaml')).match(/^\s*- "[^"]*8081[^"]*"/gm) ?? [];
  assert.deepEqual(ports.map((p) => p.trim()), ['- "127.0.0.1:8081:8081"']);
});
