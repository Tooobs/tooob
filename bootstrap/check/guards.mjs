// The browser suite's guards (#82), so a run can't take over the machine.
import { execFileSync } from 'node:child_process';
import { appendFileSync, existsSync, linkSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { defineConfig, test as base } from '@playwright/test';

export { expect } from '@playwright/test';

/** The most tabs open at once, across every context: no test needs more than 3. */
export const TAB_CAP = 4;

/**
 * How long a test, and a whole run, may take. TOOOB_GUARDS_LIMITS_MS="TEST,RUN" shrinks them, for
 * the guards' own tests; it can't stretch them.
 */
function limits() {
  const [test, run] = (process.env.TOOOB_GUARDS_LIMITS_MS ?? '').split(',').map(Number);
  const shrunk = (limit, to) => (to > 0 ? Math.min(limit, to) : limit);
  return { test: shrunk(90_000, test), run: shrunk(15 * 60_000, run) };
}

/** A Playwright config with the guards on; `config` says where the specs are. */
export function guarded(config) {
  const limit = limits();
  return defineConfig({
    testMatch: '*.spec.mjs',
    workers: 1,
    fullyParallel: false,
    timeout: limit.test,
    globalTimeout: limit.run,
    globalSetup: fileURLToPath(import.meta.url),
    reporter: 'list',
    // Headed, always; SLOWMO=ms pauses after every action, for following along.
    use: { browserName: 'chromium', headless: false, launchOptions: { slowMo: Number(process.env.SLOWMO) || 0 } },
    ...config,
  });
}

const capError = () => new Error(`a test opened a tab past the cap: at most ${TAB_CAP} tabs may be open at once`);

/** Every tab open in `browser`, across its contexts. */
const tabs = (browser) => browser.contexts().flatMap((context) => context.pages());

/**
 * Keep `browser` to TAB_CAP tabs: opening one more through newPage throws at once. A tab the page
 * opens itself (a popup) can't be refused, so it's closed, and the test fails when it ends.
 */
function capTabs(browser, popped) {
  const guard = (context) => {
    const newPage = context.newPage.bind(context);
    context.newPage = async () => {
      if (tabs(browser).length >= TAB_CAP) throw capError();
      return newPage();
    };
    context.on('page', (page) => {
      if (tabs(browser).length > TAB_CAP) {
        popped.push(capError());
        page.close().catch(() => {});
      }
    });
    return context;
  };
  const newContext = browser.newContext.bind(browser);
  browser.newContext = async (options) => guard(await newContext(options));
}

const popups = []; // tabs past the cap that pages opened themselves, in the test running now

export const test = base.extend({
  // The worker's one browser, with the tab cap.
  browser: [
    async ({ browser }, use) => {
      capTabs(browser, popups);
      await use(browser);
    },
    { scope: 'worker' },
  ],
  // Every tab and context a test opened closes when it ends, however it ends.
  closeTabs: [
    async ({ browser }, use) => {
      await use();
      await Promise.all(browser.contexts().map((context) => context.close()));
      const popped = popups.splice(0);
      if (popped.length) throw popped[0];
    },
    { auto: true },
  ],
});

// What runs on this machine keep between them: which one is going, and what it started. In the
// temp folder, so a run in another checkout of tooob sees them too.
const STATE = join(tmpdir(), 'tooob-browser-suite');
const LOCK = join(STATE, 'running');
const RECORD = join(STATE, 'processes'); // a line per Host or Relay: its process ID, then its command line

const alive = (pid) => {
  try {
    process.kill(pid, 0);
    return true;
  } catch (e) {
    return e.code === 'EPERM';
  }
};

/** Take the lock that says a run is going, or refuse if another run has it. */
function lock() {
  mkdirSync(STATE, { recursive: true });
  const mine = `${LOCK}.${process.pid}`; // written whole, then linked: the lock is never half-written
  writeFileSync(mine, String(process.pid));
  try {
    for (;;) {
      try {
        return linkSync(mine, LOCK);
      } catch (e) {
        if (e.code !== 'EEXIST') throw e;
      }
      const holder = Number(readFileSync(LOCK, 'utf8'));
      if (alive(holder) && commandLine(holder).includes('playwright')) { // not some later process with its ID
        throw new Error(`another run of the browser suite is going (process ${holder}): one run at a time`);
      }
      rmSync(LOCK, { force: true }); // its run was killed
    }
  } finally {
    rmSync(mine, { force: true });
  }
}

/** Note a Host or Relay the harness started, so no run leaves it running. */
export function recordProcess(child) {
  mkdirSync(STATE, { recursive: true });
  appendFileSync(RECORD, `${child.pid} ${child.spawnargs.join(' ')}\n`);
}

/** The command line of process `pid`, or nothing if it's gone. */
function commandLine(pid) {
  try {
    return execFileSync('ps', ['-ww', '-o', 'command=', '-p', String(pid)], { encoding: 'utf8' }).trim();
  } catch {
    return '';
  }
}

/**
 * Stop every Host and Relay in the record that's still running, then clear it. A process ID only
 * counts while its command line is still the one recorded: IDs get reused.
 */
async function stopRecorded() {
  if (!existsSync(RECORD)) return;
  const running = readFileSync(RECORD, 'utf8').split('\n').filter(Boolean)
    .map((line) => [line.slice(0, line.indexOf(' ')), line.slice(line.indexOf(' ') + 1)])
    .filter(([pid, command]) => commandLine(pid) === command)
    .map(([pid]) => Number(pid));
  const signal = (name) => running.filter(alive).forEach((pid) => {
    try {
      process.kill(pid, name);
    } catch {} // it ended meanwhile
  });
  signal('SIGTERM');
  for (let waited = 0; waited < 3000 && running.some(alive); waited += 100) await new Promise((r) => setTimeout(r, 100));
  signal('SIGKILL');
  rmSync(RECORD, { force: true });
}

/**
 * Playwright's global setup, before a run starts: take the lock, and stop what a run killed before
 * its teardown left running. Returns the global teardown, which stops what this run left running.
 */
export default async function setup() {
  lock();
  await stopRecorded();
  return async () => {
    await stopRecorded();
    rmSync(LOCK, { force: true });
  };
}
