import { execFileSync } from 'node:child_process';
import { test } from '../guards.mjs';
import * as h from '../harness.mjs';

test('starts a Relay and a Host, then hangs', async () => {
  await h.buildHost();
  const relay = await h.startRelay();
  await h.startHost(await h.startSite(), relay);
  // This worker's children: the Relay and the Host it started, and its browser.
  const children = execFileSync('pgrep', ['-P', String(process.pid)], { encoding: 'utf8' }).trim().split(/\s+/);
  const listed = execFileSync('ps', ['-o', 'pid=,comm=', '-p', children.join(',')], { encoding: 'utf8' })
    .trim().split('\n').map((line) => line.trim().split(/\s+/));
  const pids = (re) => listed.filter(([, command]) => re.test(command)).map(([pid]) => pid).join(',');
  console.log(`started: ${pids(/tooob|iroh-relay/)}. browser: ${pids(/chrom/i)}.`);
  await new Promise(() => {});
});
