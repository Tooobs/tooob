// The agent eval's scaffold (#152): its runs must use the `tooob` built from main, put first on
// PATH (agent/README.md), never an installed Release.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { chmodSync, existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const scaffold = join(dirname(fileURLToPath(import.meta.url)), '../agent/evals/send-a-file/scaffold.sh');

/** Runs the scaffold in a fresh folder with only `binDir` (and the system's) on PATH. */
function run(t, binDir) {
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
