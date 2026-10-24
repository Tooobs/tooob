// The Bootstrap Bundle's manifest, __tooob/version.json: what bootstrap/manifest.mjs writes and
// checks (GLOSSARY: Bootstrap Bundle, ADR 0017).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync, spawnSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, readFileSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const BOOTSTRAP = join(dirname(fileURLToPath(import.meta.url)), '..');
const MANIFEST = join(BOOTSTRAP, 'manifest.mjs');
const HELLO = '5891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03'; // sha256("hello\n")
const EMPTY = 'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855'; // sha256("")

// One Release for everything: the Bundle's must be the Host's.
const HOST_RELEASE = readFileSync(join(BOOTSTRAP, '../host/Cargo.toml'), 'utf8').match(
  /^version = "(.+)"$/m,
)[1];

/** A small Bundle in a fresh folder: index.html, __tooob/shell.js, and a stray dotfile. */
function aBundle(t) {
  const dir = mkdtempSync(join(tmpdir(), 'tooob-bundle-'));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  mkdirSync(join(dir, '__tooob'));
  writeFileSync(join(dir, 'index.html'), 'hello\n');
  writeFileSync(join(dir, '__tooob/shell.js'), '');
  writeFileSync(join(dir, '.DS_Store'), 'junk');
  return dir;
}

const write = (dir) => execFileSync('node', [MANIFEST, dir]);
const check = (dir) => spawnSync('node', [MANIFEST, '--check', dir], { encoding: 'utf8' });
const read = (dir) => JSON.parse(readFileSync(join(dir, '__tooob/version.json'), 'utf8'));

test('the manifest names the Release and Protocol Version, and every file by its hash', (t) => {
  const dir = aBundle(t);
  write(dir);
  assert.deepEqual(read(dir), {
    release: HOST_RELEASE,
    protocolVersion: 'tooob/0',
    files: { '__tooob/shell.js': EMPTY, 'index.html': HELLO },
  });
});

test('the check passes on a Bundle as built, and names a file changed since', (t) => {
  const dir = aBundle(t);
  write(dir);
  const untouched = check(dir);
  assert.equal(untouched.status, 0, untouched.stderr);

  writeFileSync(join(dir, '__tooob/shell.js'), 'alert(1)');
  const changed = check(dir);
  assert.notEqual(changed.status, 0);
  assert.match(changed.stderr, /__tooob\/shell\.js/);
  assert.doesNotMatch(changed.stderr, /index\.html/);
});

// With TOOOB_WWW, the browser suite runs on another Bundle (a published one, or the previous Release's
// in release.sh's cross-Release check): this local build isn't what's under test.
const OTHER_BUNDLE = process.env.TOOOB_WWW
  ? `the browser suite is on another Bootstrap Bundle (TOOOB_WWW=${process.env.TOOOB_WWW})`
  : false;

test('build.sh leaves a Bootstrap Bundle whose manifest matches it', { skip: OTHER_BUNDLE }, () => {
  const www = join(BOOTSTRAP, 'www');
  const result = check(www);
  assert.equal(result.status, 0, `run bootstrap/build.sh first\n${result.stderr}`);
  const listed = Object.keys(read(www).files);
  for (const f of ['index.html', 'tooob-sw.js', '__tooob/shell.js', '__tooob/pkg/web_bg.wasm'])
    assert.ok(listed.includes(f), `${f} isn't in the manifest`);
});

test('checking a folder with no manifest says so, without a stack trace', (t) => {
  const result = check(aBundle(t));
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /no manifest/);
  assert.doesNotMatch(result.stderr, /\n\s+at /);
});

test('a Bundle with a symlink in it is refused, naming the link', (t) => {
  const dir = aBundle(t);
  symlinkSync('/etc/hosts', join(dir, '__tooob/hosts'));
  const result = spawnSync('node', [MANIFEST, dir], { encoding: 'utf8' });
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /__tooob\/hosts.*symlink/);
});

test('the check fails on a manifest naming another Release or Protocol Version', (t) => {
  const dir = aBundle(t);
  write(dir);
  const path = join(dir, '__tooob/version.json');
  writeFileSync(path, JSON.stringify({ ...read(dir), release: '0.0.1', protocolVersion: 'tooob/9' }));
  const result = check(dir);
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /release: 0\.0\.1/);
  assert.match(result.stderr, /protocolVersion: tooob\/9/);
});

test("a Bundle's hash is its manifest's SHA-256", (t) => {
  const dir = aBundle(t);
  write(dir);
  const hash = execFileSync('node', [MANIFEST, '--hash', dir], { encoding: 'utf8' }).trim();
  const shasum = execFileSync('shasum', ['-a', '256', join(dir, '__tooob/version.json')], {
    encoding: 'utf8',
  });
  assert.equal(hash, shasum.split(' ')[0]);
});
