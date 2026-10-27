// ./release.sh, run for real in a throwaway clone that pushes to a throwaway bare repo, never to
// origin (#59). In the clone, check.sh, bootstrap/build.sh and the browser suite are stubs, so a
// test chooses what passes and fails without building or starting anything.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync, spawnSync } from 'node:child_process';
import { copyFileSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const REPO = join(dirname(fileURLToPath(import.meta.url)), '..');
const UNRELEASED = '# Changelog\n\n## Unreleased\n\n### Added\n- Something new.\n';

/** git in `dir`, its output trimmed. */
const git = (dir, ...args) => execFileSync('git', ['-C', dir, ...args], { encoding: 'utf8' }).trim();

/**
 * A clone of this repo whose origin is a bare repo of its own, with the working tree's release.sh
 * and stubs for everything slow: check.sh and the browser suite pass unless a test says otherwise.
 */
function aClone(t, { changelog = UNRELEASED, browserTest = "test('stub', () => {});" } = {}) {
  const dir = mkdtempSync(join(tmpdir(), 'tooob-release-'));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const [bare, work] = [join(dir, 'origin.git'), join(dir, 'work')];
  execFileSync('git', ['clone', '-q', '--bare', '--no-local', '--no-tags', REPO, bare]); // this repo's Releases aren't the clone's
  execFileSync('git', ['clone', '-q', bare, work]);
  git(work, 'config', 'user.email', 'release-test@example.invalid');
  git(work, 'config', 'user.name', 'release test');
  copyFileSync(join(REPO, 'release.sh'), join(work, 'release.sh'));
  writeFileSync(join(work, 'check.sh'), '#!/bin/sh\necho "check.sh: stub"\n', { mode: 0o755 });
  const check = join(work, 'bootstrap/check');
  for (const f of readdirSync(check).filter((f) => f.endsWith('.test.mjs'))) rmSync(join(check, f));
  writeFileSync(join(check, 'stub.test.mjs'), `import { test } from 'node:test';\n${browserTest}\n`);
  writeFileSync(join(work, 'CHANGELOG.md'), changelog);
  git(work, 'add', '-A');
  git(work, 'commit', '-q', '-m', 'test setup');
  git(work, 'push', '-q', 'origin', 'HEAD:main');
  return { work, bare };
}

/** ./release.sh VERSION in the clone. Without NODE_TEST_CONTEXT, which this test runner passes
 *  to its children: the browser suite's own `node --test` would report to this runner instead. */
function release(work, version, extraEnv = {}) {
  const { NODE_TEST_CONTEXT, ...env } = process.env;
  return spawnSync('./release.sh', [version], { cwd: work, encoding: 'utf8', env: { ...env, ...extraEnv } });
}

/** A PATH whose `gh` says `tag` is a published GitHub Release and nothing else is: GitHub can't see
 *  the clone's throwaway origin. */
function ghPublishing(t, tag) {
  const bin = mkdtempSync(join(tmpdir(), 'tooob-gh-'));
  t.after(() => rmSync(bin, { recursive: true, force: true }));
  writeFileSync(join(bin, 'gh'), `#!/bin/sh\n[ "$1 $2 $3" = "release view ${tag}" ] && exit 0\nexit 1\n`, { mode: 0o755 });
  return { PATH: `${bin}:${process.env.PATH}` };
}

test('a dirty tree is refused', (t) => {
  const { work, bare } = aClone(t);
  writeFileSync(join(work, 'README.md'), 'changed\n');
  const result = release(work, '0.1.0');
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /uncommitted changes/);
  assert.equal(git(bare, 'tag', '-l'), '');
});

test('a main behind origin is refused', (t) => {
  const { work, bare } = aClone(t);
  writeFileSync(join(work, 'README.md'), 'changed\n');
  git(work, 'commit', '-q', '-am', 'a commit only origin has');
  git(work, 'push', '-q', 'origin', 'HEAD:main');
  git(work, 'reset', '-q', '--hard', 'HEAD~1');
  const result = release(work, '0.1.0');
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /behind origin/);
  assert.equal(git(bare, 'tag', '-l'), '');
});

test('an empty Unreleased section is refused', (t) => {
  const { work, bare } = aClone(t, { changelog: '# Changelog\n\n## Unreleased\n\n## 0.0.1\n\n- Old.\n' });
  const result = release(work, '0.1.0');
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /Unreleased/);
  assert.equal(git(bare, 'tag', '-l'), '');
});

test('a patch Release that changes the Protocol Version is refused', (t) => {
  const { work, bare } = aClone(t);
  git(work, 'tag', 'v0.1.0');
  const wire = join(work, 'host/src/wire.rs');
  writeFileSync(wire, readFileSync(wire, 'utf8').replace('"tooob/0"', '"tooob/1"'));
  git(work, 'commit', '-q', '-am', 'a new Protocol Version');
  git(work, 'push', '-q', 'origin', 'HEAD:main', 'v0.1.0');
  const result = release(work, '0.1.1');
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /Protocol Version.*tooob\/0.*tooob\/1/);
  assert.match(result.stderr, /0\.2\.0/);
  assert.equal(git(bare, 'tag', '-l'), 'v0.1.0');
});

/** The first `version = "…"` line of a Cargo.toml in the clone: its package's version. */
const packageVersion = (work, path) => readFileSync(join(work, path), 'utf8').match(/^version = "(.+)"$/m)[1];

/** The version Cargo.lock records for `name`. */
const lockedVersion = (work, path, name) =>
  readFileSync(join(work, path), 'utf8').match(new RegExp(`name = "${name}"\\nversion = "(.+)"`))[1];

test('a Release bumps every version, stamps the changelog, and pushes a commit and its tag', (t) => {
  const { work, bare } = aClone(t);
  const result = release(work, '0.2.0');
  assert.equal(result.status, 0, result.stderr);

  assert.equal(packageVersion(work, 'host/Cargo.toml'), '0.2.0');
  assert.equal(packageVersion(work, 'bootstrap/web/Cargo.toml'), '0.2.0');
  assert.equal(lockedVersion(work, 'Cargo.lock', 'tooob'), '0.2.0');
  assert.equal(lockedVersion(work, 'bootstrap/Cargo.lock', 'web'), '0.2.0');

  const today = new Date().toISOString().slice(0, 10);
  const changelog = readFileSync(join(work, 'CHANGELOG.md'), 'utf8');
  assert.match(
    changelog,
    new RegExp(
      `^# Changelog\\n\\n## Unreleased\\n\\n## 0\\.2\\.0 — ${today}\\n\\n` +
        'Protocol Version: `tooob/0`\\. Tested with iroh-relay 1\\.3\\.0\\.\\n\\n' +
        '### Added\\n- Something new\\.\\n$',
    ),
  );

  assert.equal(git(work, 'status', '--porcelain'), '');
  assert.equal(git(work, 'log', '-1', '--format=%s'), 'Release 0.2.0');
  assert.equal(git(bare, 'rev-parse', 'main'), git(work, 'rev-parse', 'HEAD'));
  assert.equal(git(bare, 'rev-parse', 'v0.2.0^{commit}'), git(work, 'rev-parse', 'HEAD'));
  assert.match(result.stdout, /cross-Release check: skipped, the first Release/);
  assert.match(result.stdout, /never move/);
});

test("a Release keeps the earlier Releases' sections below its own", (t) => {
  const { work } = aClone(t, { changelog: `${UNRELEASED}\n## 0.1.0 — 2026-01-01\n\n- Old.\n` });
  const result = release(work, '0.2.0');
  assert.equal(result.status, 0, result.stderr);
  const changelog = readFileSync(join(work, 'CHANGELOG.md'), 'utf8');
  assert.match(changelog, /- Something new\.\n\n## 0\.1\.0 — 2026-01-01\n\n- Old\.\n$/);
});

/** The source of a browser test that fails its first `times` runs, then passes: it counts runs in a
 *  file outside the clone. */
function aFlakyBrowserTest(t, times) {
  const count = join(mkdtempSync(join(tmpdir(), 'tooob-flaky-')), 'runs');
  t.after(() => rmSync(dirname(count), { recursive: true, force: true }));
  return `import { existsSync, readFileSync, writeFileSync } from 'node:fs';
test('flaky', () => {
  const runs = existsSync(${JSON.stringify(count)}) ? Number(readFileSync(${JSON.stringify(count)}, 'utf8')) : 0;
  writeFileSync(${JSON.stringify(count)}, String(runs + 1));
  if (runs < ${times}) throw new Error('failed, run ' + (runs + 1));
});
test('steady', () => {});`;
}

test('a browser test that fails once is rerun, and the Release goes ahead', (t) => {
  const { work, bare } = aClone(t, { browserTest: aFlakyBrowserTest(t, 1) });
  const result = release(work, '0.2.0');
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /rerunning the tests that failed, once/);
  assert.equal(git(bare, 'tag', '-l'), 'v0.2.0');
});

test('a browser test that fails twice stops the Release', (t) => {
  const { work, bare } = aClone(t, { browserTest: aFlakyBrowserTest(t, 2) });
  const result = release(work, '0.2.0');
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /failed twice/);
  assert.equal(git(bare, 'tag', '-l'), '');
  assert.equal(git(work, 'status', '--porcelain'), '');
});

test('a test Release leaves the changelog for the Release it tests', (t) => {
  const { work, bare } = aClone(t);
  const result = release(work, '0.2.0-rc.1');
  assert.equal(result.status, 0, result.stderr);
  assert.equal(readFileSync(join(work, 'CHANGELOG.md'), 'utf8'), UNRELEASED);
  assert.equal(packageVersion(work, 'host/Cargo.toml'), '0.2.0-rc.1');
  assert.equal(git(bare, 'tag', '-l'), 'v0.2.0-rc.1');
});

test("a tag whose Release was never published doesn't block the next Release", (t) => {
  const { work, bare } = aClone(t);
  git(work, 'tag', 'v0.1.0'); // its release workflow failed: no GitHub Release
  git(work, 'push', '-q', 'origin', 'v0.1.0');
  const result = release(work, '0.1.1');
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /cross-Release check: skipped, no earlier Release is published/);
  assert.equal(git(bare, 'tag', '-l').split('\n').sort().join(' '), 'v0.1.0 v0.1.1');
});

test("a Release isn't checked against a published test Release of its own version", (t) => {
  const { work, bare } = aClone(t);
  git(work, 'tag', 'v0.1.0-rc.1');
  git(work, 'push', '-q', 'origin', 'v0.1.0-rc.1');
  const result = release(work, '0.1.0-rc.2', ghPublishing(t, 'v0.1.0-rc.1'));
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /cross-Release check: skipped, 0\.1\.0-rc\.1 is a test Release of 0\.1\.0/);
  assert.equal(git(bare, 'tag', '-l').split('\n').sort().join(' '), 'v0.1.0-rc.1 v0.1.0-rc.2');
});

test("an earlier version's test Release still gets the cross-Release check", (t) => {
  const { work } = aClone(t);
  git(work, 'tag', 'v0.1.0-rc.1');
  git(work, 'push', '-q', 'origin', 'v0.1.0-rc.1');
  const result = release(work, '0.2.0-rc.1', ghPublishing(t, 'v0.1.0-rc.1'));
  assert.equal(result.status, 0, result.stderr);
  assert.doesNotMatch(result.stdout, /is a test Release of/);
  assert.match(result.stdout, /cross-Release check: skipped, 0\.1\.0-rc\.1 has no tooob for this machine/); // the stand-in gh lists no assets
});
