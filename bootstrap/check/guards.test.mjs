// The browser suite's guards (#82), through the runner itself: `npx playwright test` on the small
// specs in guards/, which use the same guards as the suite. Headed: a few windows open and close.
// Run: node --test guards.test.mjs
import assert from 'node:assert/strict';
import { execFileSync, spawn } from 'node:child_process';
import { createServer } from 'node:net';
import { test } from 'node:test';
import { alive } from './guards.mjs';

const CHECK = new URL('.', import.meta.url).pathname;


/**
 * `npx playwright test` on guards/SPEC, in a process group of its own: once it ends, its exit code,
 * its output, and how long it took. Meanwhile, `saw(re)` waits until its output matches, and
 * `group` is its process group.
 */
function run(spec, { env = {} } = {}) {
  const { NODE_TEST_CONTEXT, CI, ...rest } = process.env;
  const started = Date.now();
  const child = spawn('npx', ['playwright', 'test', '--config', 'guards/playwright.config.mjs', '--reporter=line', `/${spec}`], {
    cwd: CHECK,
    env: { ...rest, ...env },
    stdio: ['ignore', 'pipe', 'pipe'],
    detached: true,
  });
  let output = '';
  const waiting = [];
  const seen = (d) => {
    output += d;
    for (const w of waiting) if (w.re.test(output)) w.resolve(output);
  };
  child.stdout.on('data', seen);
  child.stderr.on('data', seen);
  const done = new Promise((resolve) => child.on('close', (status) => resolve({ status, output, ms: Date.now() - started })));
  const saw = (re) => new Promise((resolve, reject) => {
    if (re.test(output)) return resolve(output);
    waiting.push({ re, resolve });
    done.then(() => reject(new Error(`the run ended without saying ${re}:\n${output}`)));
  });
  return Object.assign(done, { saw, group: child.pid });
}

test('a test that opens a fifth tab fails at once, naming the cap', async () => {
  const { status, output, ms } = await run('five-tabs.spec.mjs');
  assert.notEqual(status, 0, output);
  assert.match(output, /at most 4 tabs/, output);
  assert.ok(ms < 30000, `took ${ms} ms: the test went on after its fifth tab`);
});

test('a page that opens a fifth tab itself fails its test, naming the cap', async () => {
  const { status, output } = await run('five-tabs-popup.spec.mjs');
  assert.notEqual(status, 0, output);
  assert.match(output, /at most 4 tabs/, output);
});

test("no tab or context outlives its test", async () => {
  const { status, output } = await run('leftovers.spec.mjs');
  assert.equal(status, 0, output);
});

test('a test gets 90 s and a run 15 minutes', async () => {
  const { status, output } = await run('limits.spec.mjs');
  assert.equal(status, 0, output);
  assert.match(output, /limits: test 90000 ms, run 900000 ms/, output);
});

// The limits, shrunk for these tests only: a test gets 3 s and a run 5 s.
const SHRUNK = { TOOOB_GUARDS_LIMITS_MS: '3000,5000' };

test('a test that hangs fails at its limit', async () => {
  const { status, output, ms } = await run('hang.spec.mjs', { env: SHRUNK });
  assert.notEqual(status, 0, output);
  assert.match(output, /Test timeout of 3000ms exceeded/, output);
  assert.ok(ms < 20000, `took ${ms} ms`);
});

test('a run that goes on too long ends at its limit', async () => {
  const { status, output, ms } = await run('slow-run.spec.mjs', { env: SHRUNK });
  assert.notEqual(status, 0, output);
  assert.match(output, /Timed out waiting 5s for the test suite to run/, output);
  assert.ok(ms < 20000, `took ${ms} ms`);
});

test("the limits can't be stretched", async () => {
  const { output } = await run('limits.spec.mjs', { env: { TOOOB_GUARDS_LIMITS_MS: '999999,99999999' } });
  assert.match(output, /limits: test 90000 ms, run 900000 ms/, output);
});

test('a second run started during a run refuses, and says so', async () => {
  const first = run('hang.spec.mjs', { env: { TOOOB_GUARDS_LIMITS_MS: '30000,30000' } });
  try {
    await first.saw(/hangs/);
    const second = await run('limits.spec.mjs');
    assert.notEqual(second.status, 0, second.output);
    assert.match(second.output, /another run of the browser suite is going/, second.output);
    assert.doesNotMatch(second.output, /limits:/, 'the second run ran its tests');
  } finally {
    process.kill(-first.group, 'SIGINT'); // its group: npm, the runner and its worker, never the browser
    await first;
  }
  const third = await run('limits.spec.mjs');
  assert.equal(third.status, 0, `a run after the first ended refused:\n${third.output}`);
});

/**
 * Kill a run hard, as a killed terminal or agent does: SIGKILL every process in its group but
 * `spared` (its Hosts and Relays). Those are npm, the runner and its worker: Playwright starts the
 * browser in a group of its own, so no signal reaches it here.
 */
async function killHard(killed, spared = []) {
  const group = execFileSync('ps', ['-ww', '-o', 'pid=,command=', '-g', String(killed.group)], { encoding: 'utf8' })
    .trim().split('\n').map((line) => line.trim().split(/\s+/));
  const doomed = group.filter(([pid]) => !spared.includes(pid));
  assert.deepEqual(doomed.filter(([, command]) => /chrom/i.test(command)), [], 'a browser process is in the run\'s group');
  for (const [pid] of doomed) process.kill(Number(pid), 'SIGKILL');
  await killed;
}

const both = (results) => results.map((r) => r.output).join('\n----\n');

test("a run refuses when something else holds its lock's port, and says so", async () => {
  const squatter = createServer((socket) => socket.end());
  await new Promise((r) => squatter.listen(39812, '127.0.0.1', r));
  try {
    const { status, output } = await run('limits.spec.mjs');
    assert.notEqual(status, 0, output);
    assert.match(output, /port 39812.*is taken by something else/, output);
  } finally {
    await new Promise((r) => squatter.close(r));
  }
});

test('after a run is killed, of two runs started at the same moment, one goes ahead and the other refuses', async () => {
  const killed = run('hang.spec.mjs', { env: { TOOOB_GUARDS_LIMITS_MS: '30000,30000' } });
  await killed.saw(/hangs/);
  await killHard(killed);
  const results = await Promise.all([0, 1].map(() => run('hang.spec.mjs', { env: { TOOOB_GUARDS_LIMITS_MS: '4000,30000' } })));
  assert.equal(results.filter((r) => /another run of the browser suite is going/.test(r.output)).length, 1, both(results));
  assert.equal(results.filter((r) => /Test timeout of 4000ms exceeded/.test(r.output)).length, 1, both(results));
});


test('a run killed hard leaves Hosts and Relays behind, and the next run stops them first', async () => {
  const killed = run('leave-behind.spec.mjs', { env: { TOOOB_GUARDS_LIMITS_MS: '120000,120000' } });
  const said = (await killed.saw(/browser: [\d,]+\./)).match(/started: ([\d,]+)\. browser: ([\d,]+)\./);
  const [pids, browser] = [said[1].split(','), said[2].split(',')];
  assert.equal(pids.length, 2, `expected a Relay and a Host: ${pids}`);
  try {
    await killHard(killed, pids);
    assert.deepEqual(pids.filter(alive), pids, 'the killed run took its Host and Relay with it: nothing to test');
    // The browser, never signalled, ends by itself once its worker is gone.
    for (let waited = 0; waited < 10000 && browser.some(alive); waited += 200) await new Promise((r) => setTimeout(r, 200));
    assert.deepEqual(browser.filter(alive), [], 'the browser outlived its killed worker');

    const next = await run('no-leftovers.spec.mjs', { env: { LEFTOVERS: pids.join(',') } });
    assert.equal(next.status, 0, next.output);
  } finally {
    for (const pid of pids.filter(alive)) process.kill(Number(pid), 'SIGKILL');
  }
});

test("a run that ends, even failing, leaves none of its Hosts and Relays running", async () => {
  const failing = run('leave-behind.spec.mjs', { env: { TOOOB_GUARDS_LIMITS_MS: '20000,60000' } });
  const pids = (await failing.saw(/started: [\d,]+\./)).match(/started: ([\d,]+)\./)[1].split(',');
  const { status, output } = await failing;
  assert.notEqual(status, 0, output);
  assert.deepEqual(pids.filter(alive), [], output);
});
