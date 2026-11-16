// served.sh: whether a Relay serves a Bootstrap Bundle on its domain and on a Key Address under
// it. With a Bundle it waits for that Bundle (each deploy.sh's last step); with only a domain it
// checks that both serve the same one, the one the domain serves now, and names it (a live check
// after a deploy). A stand-in curl on PATH answers for each host from a folder, so no network.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { chmodSync, mkdirSync, mkdtempSync, writeFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const served = new URL('../served.sh', import.meta.url).pathname;
const sha256 = (text) => createHash('sha256').update(text).digest('hex');

// A folder with a stand-in curl that gives <answers>/<host> for https://<host>/… (to stdout, or
// to the file -o names), or fails as curl -f does when there's no such file.
function relay(answers) {
  const dir = mkdtempSync(join(tmpdir(), 'served-'));
  mkdirSync(join(dir, 'bin'));
  mkdirSync(join(dir, 'answers'));
  for (const [host, body] of Object.entries(answers)) writeFileSync(join(dir, 'answers', host), body);
  const curl = join(dir, 'bin', 'curl');
  writeFileSync(curl, `#!/bin/sh
out=
while [ $# -gt 0 ]; do
  case $1 in -o) out=$2; shift ;; *) url=$1 ;; esac
  shift
done
host=\${url#https://}; host=\${host%%/*}
[ -f "${dir}/answers/$host" ] || exit 22
if [ -n "$out" ]; then cat "${dir}/answers/$host" > "$out"; else cat "${dir}/answers/$host"; fi
`);
  chmodSync(curl, 0o755);
  return dir;
}

function run(dir, args) {
  return spawnSync('sh', [served, ...args], {
    encoding: 'utf8',
    env: { ...process.env, PATH: `${join(dir, 'bin')}:${process.env.PATH}`, SERVED_TRIES: '2', SERVED_EVERY: '0' },
  });
}

function bundle(dir, version) {
  const www = join(dir, 'www');
  mkdirSync(join(www, '__tooob'), { recursive: true });
  writeFileSync(join(www, '__tooob', 'version.json'), version);
  return www;
}

test('with no domain it says how to run it, and fails', () => {
  const result = run(relay({}), []);
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /usage: .*served\.sh RELAY_DOMAIN \[BUNDLE\]/);
});

test('with only a domain it names the Bundle the domain and a Key Address both serve', () => {
  const version = '{"release":"0.3.6"}\n';
  const dir = relay({ 'r.example': version, 'check.r.example': version });
  const result = run(dir, ['r.example']);
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, new RegExp(`https://r\\.example/ serves the Bundle, hash ${sha256(version)}`));
});

test('with only a domain it fails when a Key Address serves another Bundle', () => {
  const dir = relay({ 'r.example': '{"release":"0.3.6"}\n', 'check.r.example': '{"release":"0.3.5"}\n' });
  const result = run(dir, ['r.example']);
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /https:\/\/check\.r\.example\/__tooob\/version\.json is still not serving this Bundle/);
});

test('with only a domain it fails when the domain serves no Bundle', () => {
  const result = run(relay({}), ['r.example']);
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /https:\/\/r\.example\/__tooob\/version\.json serves no Bundle/);
});

test('with a Bundle it waits for that Bundle, as each deploy.sh runs it', () => {
  const version = '{"release":"0.3.6"}\n';
  const dir = relay({ 'r.example': version, 'check.r.example': version });
  const result = run(dir, ['r.example', bundle(dir, version)]);
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, new RegExp(`serves the Bundle, hash ${sha256(version)}`));
});

test('with a Bundle it fails when the Relay serves another one', () => {
  const dir = relay({ 'r.example': '{"release":"0.3.5"}\n', 'check.r.example': '{"release":"0.3.5"}\n' });
  const result = run(dir, ['r.example', bundle(dir, '{"release":"0.3.6"}\n')]);
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /https:\/\/r\.example\/__tooob\/version\.json is still not serving this Bundle/);
});
