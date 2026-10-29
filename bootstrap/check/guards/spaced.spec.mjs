import { spawn } from 'node:child_process';
import { mkdirSync, mkdtempSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { recordProcess, test } from '../guards.mjs';

// As rustup's cargo does: the program recorded hands over to one of the same name, by a path with a
// space in it (a home folder like /Users/John Smith).
test('starts a program that hands over to one in a folder with a space in its name, then hangs', async () => {
  const root = mkdtempSync(join(tmpdir(), 'spaced-'));
  const [spaced, bin] = [join(root, 'John Smith'), join(root, 'bin')];
  mkdirSync(spaced);
  mkdirSync(bin);
  symlinkSync(process.execPath, join(spaced, 'idler')); // node, by another name
  writeFileSync(join(bin, 'idler'), `#!/bin/sh\nexec "${join(spaced, 'idler')}" "$@"\n`, { mode: 0o755 });
  const child = spawn(join(bin, 'idler'), ['-e', 'setInterval(()=>{},1000)'], { stdio: 'ignore' });
  recordProcess(child);
  console.log(`started: ${child.pid}.`);
  await new Promise(() => {});
});
