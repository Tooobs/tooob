// For the guards' own tests: says whether the run still holds its lock when the reporters end, the
// last thing a run does (Playwright writes .last-run.json then).
import { connect } from 'node:net';
import { LOCK_PORT } from '../machine.mjs';

export default class LockReporter {
  async onEnd() {
    const held = await new Promise((resolve) => {
      connect(LOCK_PORT, '127.0.0.1').on('connect', function () { this.destroy(); resolve(true); }).on('error', () => resolve(false));
    });
    console.log(`the lock at the reporters' end: ${held ? 'held' : 'free'}`);
  }

  printsToStdio() {
    return false;
  }
}
