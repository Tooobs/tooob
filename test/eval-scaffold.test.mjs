// The agent eval's scaffold (#152): its runs must use the `tooob` built from main, put first on
// PATH (agent/README.md), never an installed Release.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const evals = join(dirname(fileURLToPath(import.meta.url)), '../agent/evals');

/** Runs case `name`'s scaffold in a fresh folder with only `binDir` (and the system's) on PATH. */
function run(t, binDir, name = 'send-a-file') {
  const scaffold = join(evals, name, 'scaffold.sh');
  const root = mkdtempSync(join(tmpdir(), 'eval-scaffold-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const bin = join(root, binDir);
  mkdirSync(bin, { recursive: true });
  writeFileSync(join(bin, 'tooob'), '#!/bin/sh\necho "tooob 0.0.0 (tooob/0)"\n');
  chmodSync(join(bin, 'tooob'), 0o755);
  const cwd = join(root, 'run');
  mkdirSync(cwd);
  const r = spawnSync('sh', [scaffold], { cwd, encoding: 'utf8', env: { PATH: `${bin}:/usr/bin:/bin` } });
  return { ...r, cwd };
}

test("a run whose tooob is main's debug build gets its report.pdf", (t) => {
  const r = run(t, 'repo/target/debug');
  assert.equal(r.status, 0, r.stderr);
  assert.ok(existsSync(join(r.cwd, 'report.pdf')));
});

test('a run whose tooob is an installed Release is refused, saying how to run it', (t) => {
  const r = run(t, 'home/.local/bin');
  assert.notEqual(r.status, 0);
  assert.match(r.stderr, /target\/debug/);
  assert.match(r.stderr, /agent\/README\.md/);
});

test('an-upload-arrived leaves an upload whose content, not its name, holds the marker', (t) => {
  const r = run(t, 'repo/target/debug', 'an-upload-arrived');
  assert.equal(r.status, 0, r.stderr);
  const files = readdirSync(join(r.cwd, 'from-ana'));
  assert.equal(files.length, 1);
  assert.doesNotMatch(files[0], /PELICAN|marker/);
  assert.match(readFileSync(join(r.cwd, 'from-ana', files[0]), 'utf8'), /PELICAN-7731/);
});

test('an-upload-arrived refuses an installed Release too', (t) => {
  const r = run(t, 'home/.local/bin', 'an-upload-arrived');
  assert.notEqual(r.status, 0);
  assert.match(r.stderr, /target\/debug/);
});
