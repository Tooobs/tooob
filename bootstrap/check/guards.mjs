// The browser suite's guards (#82), so a run can't take over the machine.
import { execFileSync } from 'node:child_process';
import { appendFileSync, existsSync, mkdirSync, readFileSync, rmSync } from 'node:fs';
import { connect, createServer } from 'node:net';
import { tmpdir } from 'node:os';
import { basename, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { defineConfig, test as base } from '@playwright/test';
import { LOCK_PORT, alive } from './machine.mjs';

export { expect } from '@playwright/test';
export { alive } from './machine.mjs';

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

/**
 * A Playwright config with the guards on; `config` says where the specs are. In the runner, it first
 * takes the one-run lock, or refuses: before Playwright clears the output folder, so a refused run
 * leaves a going run's results be. (Workers load the config too, under their runner's lock.)
 */
export async function guarded(config) {
  // A Playwright child (a worker, or the loader its UI, watch mode and test server fork) has an IPC
  // channel; the runner a shell, npx or a test starts has none.
  const runner = process.env.TEST_WORKER_INDEX === undefined && process.send === undefined;
  if (runner) await lock();
  const limit = limits();
  return defineConfig({
    testMatch: '*.spec.mjs',
    workers: 1,
    fullyParallel: false,
    timeout: limit.test,
    globalTimeout: limit.run,
    globalSetup: fileURLToPath(import.meta.url),
    reporter: 'list',
    // Headless unless HEADED=1, which opens windows you can watch; SLOWMO=ms pauses after every
    // action, for following along.
    use: { browserName: 'chromium', headless: process.env.HEADED !== '1', launchOptions: { slowMo: Number(process.env.SLOWMO) || 0 } },
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
function capTabs(browser) {
  const guard = (context) => {
    const newPage = context.newPage.bind(context);
    context.newPage = async () => {
      if (tabs(browser).length >= TAB_CAP) throw capError();
      return newPage();
    };
    context.on('page', (page) => {
      if (tabs(browser).length > TAB_CAP) {
        tabsPastCap.push(capError());
        page.close().catch(() => {});
      }
    });
    return context;
  };
  const newContext = browser.newContext.bind(browser);
  browser.newContext = async (options) => guard(await newContext(options));
}

const tabsPastCap = []; // the errors for tabs past the cap that pages opened themselves, in the test running now

export const test = base.extend({
  // The worker's one browser, with the tab cap.
  browser: [
    async ({ browser }, use) => {
      capTabs(browser);
      await use(browser);
    },
    { scope: 'worker' },
  ],
  // Every tab and context a test opened closes when it ends, however it ends; and a test whose page
  // opened a tab past the cap fails.
  closeTabsAndCheckCap: [
    async ({ browser }, use) => {
      await use();
      await Promise.all(browser.contexts().map((context) => context.close()));
      const errors = tabsPastCap.splice(0);
      if (errors.length) throw errors[0];
    },
    { auto: true },
  ],
});

// What runs on this machine keep between them: what each started, in the temp folder, so a run in
// another checkout of tooob sees it too.
const STATE = join(tmpdir(), 'tooob-browser-suite');
const RECORD = join(STATE, 'processes'); // a JSON line per process: { pid, argv, group }

/**
 * The lock that says a run is going: LOCK_PORT, listened on by the runner from loading its config
 * until its process exits, after its reporters have written .last-run.json. The operating system
 * frees it the moment a run ends or is killed, so it's never left behind, and two runs can't both
 * have it. Whoever connects is told the holder's process ID. In a long-lived runner (Playwright's
 * UI, watch mode) it's held for the whole session. On macOS something on the wildcard address
 * (*:port) goes unnoticed: harmless, since every run binds 127.0.0.1.
 */
async function lock() {
  // This runner's lock, once taken: its config can load again in the same process.
  if (globalThis.tooobSuiteLock) return;
  const server = createServer((socket) => {
    socket.unref(); // a prober that never closes keeps no run going
    socket.on('error', () => {}); // a prober's reset is its own business
    socket.end(String(process.pid));
  });
  const taken = await new Promise((resolve, reject) => {
    server.once('error', (e) => (e.code === 'EADDRINUSE' ? resolve(true) : reject(e)));
    server.listen(LOCK_PORT, '127.0.0.1', () => resolve(false));
  });
  if (!taken) {
    globalThis.tooobSuiteLock = server.unref(); // the process's exit lets it go
    return;
  }
  const holder = await new Promise((resolve) => {
    let said = '';
    connect(LOCK_PORT, '127.0.0.1')
      .setTimeout(2000, function () {
        this.destroy(); // whatever holds the port isn't saying
      })
      .on('data', (d) => (said += d))
      .on('close', () => resolve(said))
      .on('error', () => resolve(''));
  });
  throw new Error(
    /^\d+$/.test(holder)
      ? `another run of the browser suite is going (process ${holder}): one run at a time`
      : `port ${LOCK_PORT}, the browser suite's one-run-at-a-time lock, is taken by something else`,
  );
}

/**
 * Note a process the harness started (a Host, a Relay, a build), so no run leaves it running. With
 * `group`, it leads a process group of its own, and the whole group is stopped with it.
 */
export function recordProcess(child, { group = false } = {}) {
  mkdirSync(STATE, { recursive: true });
  appendFileSync(RECORD, JSON.stringify({ pid: child.pid, argv: child.spawnargs, group }) + '\n');
}

/** What the record holds, line by line. */
function recorded() {
  if (!existsSync(RECORD)) return [];
  return readFileSync(RECORD, 'utf8').split('\n').filter(Boolean).flatMap((line) => {
    try {
      return [JSON.parse(line)];
    } catch {
      return []; // a line from an older record, or cut short
    }
  });
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
 * Whether recorded process `p` is still running as recorded: its command line has the recorded
 * arguments, after a program of the same name. Only the name: rustup's cargo hands over to its
 * toolchain's, by a full path that may have spaces in it. A process ID alone isn't enough: IDs get
 * reused.
 */
function stillRunning(p) {
  const running = commandLine(p.pid);
  const args = p.argv.slice(1).map((a) => ' ' + a).join('');
  return running.endsWith(args) && basename(running.slice(0, running.length - args.length)) === basename(p.argv[0]);
}

/** Stop every process in the record that's still running as recorded, then clear the record. */
async function stopRecorded() {
  const processes = recorded().filter(stillRunning);
  const signal = (name) => processes.filter(stillRunning).forEach((p) => {
    try {
      process.kill(p.group ? -p.pid : p.pid, name);
    } catch {} // it ended meanwhile
  });
  signal('SIGTERM');
  for (let waited = 0; waited < 3000 && processes.some((p) => alive(p.pid)); waited += 100) await new Promise((r) => setTimeout(r, 100));
  signal('SIGKILL');
  rmSync(RECORD, { force: true });
}

/**
 * Playwright's global setup, before a run starts, under the lock: stop what a run killed before its
 * teardown left running. Returns the global teardown, which stops what this run left running.
 */
export default async function setup() {
  await stopRecorded();
  return stopRecorded;
}
