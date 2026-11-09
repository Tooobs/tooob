// New Relays start closed, and an open Relay closes without cutting anyone off (#134, ADR 0030).
// Enrolling closes a Relay, so it's only ever a first-time step the Relay Operator takes by
// hand: the VPS wizard's own stage, the Fly README's own step. A deploy (every upgrade, and the
// Ship step for r.tooobs.net) never enrolls a key, makes a code or a token, or closes the Relay.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const read = (path) => readFileSync(new URL(path, import.meta.url), 'utf8');
const code = (text) => text.split('\n').filter((l) => !l.trim().startsWith('#')).join('\n');
const enrolling = /tooob-relay\s+(enroll|code|token|remove)\b|enrolled\.toml|TOOOB_RELAY_ENROLLED/;

for (const path of ['../relay/deploy.sh', '../fly-relay/deploy.sh', '../relay/image/start.sh',
  '../relay/image/Dockerfile', '../relay/compose.yaml', '../fly-relay/fly.toml', '../bundle.sh',
  '../served.sh']) {
  test(`${path.slice(3)} never enrolls a key or closes the Relay`, () => {
    assert.doesNotMatch(code(read(path)), enrolling);
  });
}

test('Fly runs nothing of its own on a deploy (no release_command)', () => {
  assert.doesNotMatch(code(read('../fly-relay/fly.toml')), /release_command/);
});

test('the VPS wizard enrolls the first Host in a stage of its own, after the deploy', () => {
  const wizard = read('../relay/wizard.sh');
  const deploy = wizard.indexOf('./deploy.sh $RELEASE');
  const stage = wizard.indexOf('stage "Close the Relay');
  assert.ok(deploy > 0 && stage > deploy, 'a "Close the Relay" stage after the deploy');
  const body = wizard.slice(stage, wizard.indexOf('stage "', stage + 1));
  assert.match(body, /relay_cli\(\) \{ vps "cd tooob-relay && docker compose exec -T relay tooob-relay /);
  assert.match(body, /relay_cli enroll /);
  assert.match(body, /relay_cli code /);
});

test('the Fly README enrolls the first Host after Deploy, never inside it', () => {
  const readme = read('../fly-relay/README.md');
  const deploy = readme.indexOf('## Deploy');
  const close = readme.indexOf('## Close it');
  assert.ok(deploy > 0 && close > deploy, 'a "Close it" section after Deploy');
  assert.match(readme.slice(close), /fly ssh console .*tooob-relay enroll /);
  assert.match(readme.slice(close), /fly ssh console .*tooob-relay code /);
});

test('the deploy README says how to close an open Relay without cutting anyone off', () => {
  const readme = read('../README.md');
  assert.match(readme, /## Closing an open Relay/);
  assert.match(readme, /tooob show NAME/);
});

test("nothing says to restrict iroh-relay's own `access`: Enrolled Keys do that", () => {
  for (const path of ['../relay/image/iroh-relay.toml', '../relay/README.md', '../fly-relay/README.md']) {
    assert.doesNotMatch(read(path), /set `access`|anyone can use it: before you share it/, path);
  }
});
