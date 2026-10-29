import { execFileSync } from 'node:child_process';
import { test } from '../guards.mjs';
import * as h from '../harness.mjs';

test('starts a Relay and a Host, then hangs', async () => {
  h.buildHost();
  const relay = await h.startRelay();
  await h.startHost(await h.startSite(), relay);
  const children = execFileSync('ps', ['-o', 'pid=,comm=', '-p', execFileSync('pgrep', ['-P', String(process.pid)], { encoding: 'utf8' }).trim().split(/\s+/).join(',')], { encoding: 'utf8' });
  const started = children.split('\n').filter((line) => /tooob|iroh-relay/.test(line)).map((line) => line.trim().split(/\s+/)[0]);
  console.log(`started: ${started.join(',')}.`);
  await new Promise(() => {});
});
