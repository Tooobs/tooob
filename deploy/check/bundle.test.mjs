// deploy/bundle.sh (#67), which every Relay setup runs to get the Bootstrap Bundle it serves: a Release's,
// from a stand-in for GitHub's Releases (TOOOB_RELEASES), or bootstrap/www as built here.
import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync, spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { createServer } from 'node:http';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const DEPLOY = join(dirname(fileURLToPath(import.meta.url)), '..');
const scratch = mkdtempSync(join(tmpdir(), 'tooob-bundle-'));
after(() => rmSync(scratch, { recursive: true, force: true }));

/** A Release's Bundle, as bootstrap/package.sh packs one: its archive and the archive's .sha256. */
function aBundle(release) {
  const dir = mkdtempSync(join(scratch, 'pack-'));
  const manifest = JSON.stringify({ release, protocolVersion: 'tooob/0', files: {} }, null, 2) + '\n';
  mkdirSync(join(dir, 'tooob-bootstrap/__tooob'), { recursive: true });
  writeFileSync(join(dir, 'tooob-bootstrap/__tooob/version.json'), manifest);
  writeFileSync(join(dir, 'tooob-bootstrap/index.html'), `<p>Bootstrap ${release}</p>\n`);
  execFileSync('tar', ['-cJf', 'tooob-bootstrap.tar.xz', 'tooob-bootstrap'], { cwd: dir });
  const archive = readFileSync(join(dir, 'tooob-bootstrap.tar.xz'));
  const sum = `${createHash('sha256').update(archive).digest('hex')} *tooob-bootstrap.tar.xz\n`;
  return { archive, sum, hash: createHash('sha256').update(manifest).digest('hex') };
}

/** Releases laid out as GitHub does: /download/v<release>/<file>, and /latest/download/<file>. */
async function releases(byTag, latest) {
  const server = createServer((req, res) => {
    const m = /^\/(?:download\/(v[^/]+)|latest\/download)\/([^/]+)$/.exec(req.url);
    const bundle = m && byTag[m[1] ?? latest];
    const body = bundle && { 'tooob-bootstrap.tar.xz': bundle.archive, 'tooob-bootstrap.tar.xz.sha256': bundle.sum }[m[2]];
    if (!body) return res.writeHead(404).end();
    res.end(body);
  });
  await new Promise((r) => server.listen(0, '127.0.0.1', r));
  after(() => server.close());
  return `http://127.0.0.1:${server.address().port}`;
}

/** ./bundle.sh ARGS... into a fresh directory (or `{ dir }`, last): its exit status, output and that directory. */
function bundle(url, ...args) {
  const opts = typeof args.at(-1) === 'object' ? args.pop() : {};
  const dir = opts.dir ?? mkdtempSync(join(scratch, 'out-'));
  return new Promise((resolve) => {
    const env = { ...process.env, TOOOB_RELEASES: url, ...(opts.path && { PATH: opts.path }) };
    const child = spawn(join(DEPLOY, 'bundle.sh'), [dir, ...args], { env });
    let stdout = '', stderr = '';
    child.stdout.on('data', (d) => (stdout += d));
    child.stderr.on('data', (d) => (stderr += d));
    child.on('exit', (status) => resolve({ status, stdout, stderr, dir }));
  });
}

test("a named Release's Bundle lands in DIR/www, and its hash is printed", async () => {
  const b = aBundle('0.1.0');
  const url = await releases({ 'v0.1.0': b, 'v0.2.0': aBundle('0.2.0') }, 'v0.2.0');
  const out = await bundle(url, '0.1.0');
  assert.equal(out.status, 0, out.stderr);
  assert.equal(readFileSync(join(out.dir, 'www/index.html'), 'utf8'), '<p>Bootstrap 0.1.0</p>\n');
  assert.match(out.stdout, new RegExp(`Release 0\\.1\\.0.*Bundle hash ${b.hash}`));
});

test('with no Release named, the newest', async () => {
  const url = await releases({ 'v0.1.0': aBundle('0.1.0'), 'v0.2.0': aBundle('0.2.0') }, 'v0.2.0');
  const out = await bundle(url);
  assert.equal(out.status, 0, out.stderr);
  assert.equal(readFileSync(join(out.dir, 'www/index.html'), 'utf8'), '<p>Bootstrap 0.2.0</p>\n');
});

/** A PATH with the tools bundle.sh needs but no shasum, as on Arch (#142): sha256sum only, which
 * here is whichever SHA-256 tool this box has, so the test runs on macOS too. */
function pathWithoutShasum() {
  const bin = mkdtempSync(join(scratch, 'bin-'));
  const which = (tool) => execFileSync('/bin/sh', ['-c', `command -v ${tool}`], { encoding: 'utf8' }).trim();
  for (const tool of ['sh', 'curl', 'tar', 'xz', 'sed', 'cut', 'rm', 'mv', 'cp', 'dirname']) {
    symlinkSync(which(tool), join(bin, tool));
  }
  let real;
  try { real = which('sha256sum'); } catch { real = `${which('shasum')} -a 256`; }
  writeFileSync(join(bin, 'sha256sum'), `#!/bin/sh\nexec ${real} "$@"\n`, { mode: 0o755 });
  return bin;
}

test('with no shasum (Arch), sha256sum checks the Bundle and prints its hash', async () => {
  const b = aBundle('0.1.0');
  const url = await releases({ 'v0.1.0': b }, 'v0.1.0');
  const out = await bundle(url, '0.1.0', { path: pathWithoutShasum() });
  assert.equal(out.status, 0, out.stderr);
  assert.match(out.stdout, new RegExp(`Release 0\\.1\\.0.*Bundle hash ${b.hash}$`, 'm'));
  const bad = await releases({ 'v0.1.0': { ...b, archive: aBundle('0.1.0-evil').archive } }, 'v0.1.0');
  const refused = await bundle(bad, '0.1.0', { path: pathWithoutShasum() });
  assert.match(refused.stderr, /doesn't match its checksum/);
});

test("a Bundle that doesn't match its checksum is refused, and nothing is left in DIR/www", async () => {
  const b = aBundle('0.1.0');
  const url = await releases({ 'v0.1.0': { ...b, archive: aBundle('0.1.0-evil').archive } }, 'v0.1.0');
  const out = await bundle(url, '0.1.0');
  assert.notEqual(out.status, 0);
  assert.match(out.stderr, /doesn't match its checksum/);
  assert.equal(existsSync(join(out.dir, 'www')), false);
});

test("a Release that doesn't exist is refused, saying what couldn't be downloaded", async () => {
  const url = await releases({ 'v0.1.0': aBundle('0.1.0') }, 'v0.1.0');
  const out = await bundle(url, '9.9.9');
  assert.notEqual(out.status, 0);
  assert.match(out.stderr, /couldn't download .*v9\.9\.9\/tooob-bootstrap\.tar\.xz/);
});

test('--local: bootstrap/www as built here, with its hash', async (t) => {
  const manifest = join(DEPLOY, '../bootstrap/www/__tooob/version.json');
  if (!existsSync(manifest)) return t.skip('bootstrap/www not built: bootstrap/build.sh');
  const out = await bundle('http://127.0.0.1:9/nowhere', '--local');
  assert.equal(out.status, 0, out.stderr);
  assert.deepEqual(readFileSync(join(out.dir, 'www/__tooob/version.json')), readFileSync(manifest));
  const hash = createHash('sha256').update(readFileSync(manifest)).digest('hex');
  assert.match(out.stdout, new RegExp(`bootstrap/www as built here.*Bundle hash ${hash}`));
});

test('a DIR that already has a www/ is refused and left as it was, so no older Bundle mixes in', async () => {
  const url = await releases({ 'v0.1.0': aBundle('0.1.0') }, 'v0.1.0');
  for (const args of [['0.1.0'], ['--local']]) {
    const dir = mkdtempSync(join(scratch, 'out-'));
    mkdirSync(join(dir, 'www'));
    writeFileSync(join(dir, 'www/index.html'), 'older\n');
    const out = await bundle(url, ...args, { dir });
    assert.notEqual(out.status, 0, args[0]);
    assert.match(out.stderr, /already has a www/, args[0]);
    assert.equal(readFileSync(join(dir, 'www/index.html'), 'utf8'), 'older\n', args[0]);
  }
});
