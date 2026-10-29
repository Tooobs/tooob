import { execFileSync } from 'node:child_process';
import { utimesSync } from 'node:fs';
import { test } from '../guards.mjs';
import * as h from '../harness.mjs';

test('builds tooob, then hangs', async () => {
  const lib = new URL('../../../host/src/lib.rs', import.meta.url).pathname;
  utimesSync(lib, new Date(), new Date()); // so there's something to build
  const building = h.buildHost();
  let cargo, compiling;
  for (let waited = 0; !compiling && waited < 3000; waited += 100) {
    await new Promise((r) => setTimeout(r, 100));
    cargo ??= execFileSync('pgrep', ['-P', String(process.pid), 'cargo'], { encoding: 'utf8' }).trim();
    try {
      compiling = execFileSync('pgrep', ['-P', cargo], { encoding: 'utf8' }).trim().split(/\s+/).join(',');
    } catch {} // none yet
  }
  console.log(`cargo: ${cargo}. compiling: ${compiling}.`);
  await building;
  await new Promise(() => {});
});
