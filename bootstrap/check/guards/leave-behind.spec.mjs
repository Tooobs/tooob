import { execFileSync } from 'node:child_process';
import { test } from '../guards.mjs';
import * as h from '../harness.mjs';

test('starts a Relay and a Host, then hangs', async () => {
  h.buildHost();
  const relay = await h.startRelay();
  await h.startHost(await h.startSite(), relay);
  // This worker's children, but for its browser: the Relay and the Host.
  const children = execFileSync('pgrep', ['-P', String(process.pid)], { encoding: 'utf8' }).trim().split(/\s+/);
  const listed = execFileSync('ps', ['-o', 'pid=,comm=', '-p', children.join(',')], { encoding: 'utf8' });
  const started = listed.split('\n').filter((line) => /tooob|iroh-relay/.test(line)).map((line) => line.trim().split(/\s+/)[0]);
  console.log(`started: ${started.join(',')}.`);
  await new Promise(() => {});
});
