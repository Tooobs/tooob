import { execFileSync } from 'node:child_process';
import { utimesSync } from 'node:fs';
import { test } from '../guards.mjs';
import * as h from '../harness.mjs';

test('builds tooob, then hangs', async () => {
  const lib = new URL('../../../host/src/lib.rs', import.meta.url).pathname;
  utimesSync(lib, new Date(), new Date()); // so there's something to build
  const building = h.buildHost();
  await new Promise((r) => setTimeout(r, 300));
  console.log(`cargo: ${execFileSync('pgrep', ['-P', String(process.pid), 'cargo'], { encoding: 'utf8' }).trim()}.`);
  await building;
  await new Promise(() => {});
});
