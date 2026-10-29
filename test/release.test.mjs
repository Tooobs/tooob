// ./release.sh, run for real in a throwaway clone that pushes to a throwaway bare repo, never to
// origin (#59). In the clone, check.sh, bootstrap/build.sh and the browser suite are stubs, so a
// test chooses what passes and fails without building or starting anything. The browser suite's
// stub runs on Playwright, from this repo's bootstrap/check/node_modules, with no browser.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync, spawnSync } from 'node:child_process';
import { copyFileSync, existsSync, mkdtempSync, readFileSync, readdirSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
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
const STUB_CONFIG = "export default { testDir: '.', testMatch: 'stub.spec.mjs', workers: 1 };\n";

function aClone(t, { changelog = UNRELEASED, browserTest = "test('stub', () => {});", browserConfig = STUB_CONFIG } = {}) {
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
  for (const f of readdirSync(check).filter((f) => f.endsWith('.spec.mjs'))) rmSync(join(check, f));
  writeFileSync(join(check, 'stub.spec.mjs'), `import { test } from '@playwright/test';\n${browserTest}\n`);
  // Not the suite's guarded config: its one-run-at-a-time lock would refuse these while a real run
  // goes. Only the stub: guards/ holds specs too, for the guards' own tests.
  writeFileSync(join(check, 'playwright.config.mjs'), browserConfig);
  const modules = join(REPO, 'bootstrap/check/node_modules');
  assert.ok(existsSync(modules), 'needs Playwright: cd bootstrap/check && npm ci');
  symlinkSync(modules, join(check, 'node_modules'));
  writeFileSync(join(work, 'CHANGELOG.md'), changelog);
  git(work, 'add', '-A');
  git(work, 'commit', '-q', '-m', 'test setup');
  git(work, 'push', '-q', 'origin', 'HEAD:main');
  return { work, bare };
}

/** ./release.sh VERSION in the clone. Without NODE_TEST_CONTEXT, which this test runner passes to
 *  its children: release.sh's own tools should see the environment a person's shell gives them. */
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

/** A browser suite whose test 'flaky' fails its first `times` runs, then passes, beside one that always
 *  passes: its `source`, and how many times the steady one ran. Runs are counted in files outside
 *  the clone. */
function aFlakyBrowserTest(t, times) {
  const dir = mkdtempSync(join(tmpdir(), 'tooob-flaky-'));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const [flaky, steady] = [join(dir, 'flaky'), join(dir, 'steady')];
  const source = `import { existsSync, readFileSync, writeFileSync } from 'node:fs';
const ran = (file) => {
  const runs = existsSync(file) ? Number(readFileSync(file, 'utf8')) : 0;
  writeFileSync(file, String(runs + 1));
  return runs;
};
test('flaky', () => {
  const runs = ran(${JSON.stringify(flaky)});
  if (runs < ${times}) throw new Error('failed, run ' + (runs + 1));
});
test('steady', () => {
  ran(${JSON.stringify(steady)});
});`;
  return { source, steadyRuns: () => (existsSync(steady) ? Number(readFileSync(steady, 'utf8')) : 0) };
}

test('a browser test that fails once is rerun, alone, and the Release goes ahead', (t) => {
  const suite = aFlakyBrowserTest(t, 1);
  const { work, bare } = aClone(t, { browserTest: suite.source });
  const result = release(work, '0.2.0');
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /rerunning the tests that failed, once/);
  assert.equal(suite.steadyRuns(), 1, 'the rerun ran a test that passed');
  assert.equal(git(bare, 'tag', '-l'), 'v0.2.0');
});

test('a browser test that fails twice stops the Release', (t) => {
  const { work, bare } = aClone(t, { browserTest: aFlakyBrowserTest(t, 2).source });
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

test("a Release is refused up front without Playwright's Chromium, naming it", (t) => {
  const { work, bare } = aClone(t);
  const none = mkdtempSync(join(tmpdir(), 'tooob-no-browsers-'));
  t.after(() => rmSync(none, { recursive: true, force: true }));
  const result = release(work, '0.2.0', { PLAYWRIGHT_BROWSERS_PATH: none });
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /needs Playwright's Chromium: .*npx playwright install chromium/);
  assert.doesNotMatch(result.stdout, /the browser suite/, 'the suite ran');
  assert.equal(git(bare, 'tag', '-l'), '');
});

test('a browser suite that fails before any test runs stops the Release, saying so', (t) => {
  const { work, bare } = aClone(t, {
    // As when another run of the suite is going: its global setup refuses.
    browserConfig: `export default { testDir: '.', testMatch: 'stub.spec.mjs', workers: 1, globalSetup: './refuse.mjs' };\n`,
  });
  writeFileSync(join(work, 'bootstrap/check/refuse.mjs'), "export default () => { throw new Error('another run of the browser suite is going'); };\n");
  git(work, 'add', '-A');
  git(work, 'commit', '-q', '-m', 'a suite that refuses');
  git(work, 'push', '-q', 'origin', 'HEAD:main');
  const result = release(work, '0.2.0');
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /the browser suite: it failed before any test ran/);
  assert.doesNotMatch(result.stdout, /rerunning/);
  assert.equal(git(bare, 'tag', '-l'), '');
});
