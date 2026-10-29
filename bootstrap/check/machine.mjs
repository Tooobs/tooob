// What the browser suite's runs on this machine share, as the guards (and their tests) see it.

/** The port a run's runner listens on while it goes: the one-run-at-a-time lock (guards.mjs). */
export const LOCK_PORT = 27182; // below every common ephemeral range, so no outgoing connection takes it

/** Whether process `pid` is still running. */
export function alive(pid) {
  const n = Number(pid);
  if (!Number.isInteger(n) || n <= 0) return false; // 0 and below would ask about process groups
  try {
    process.kill(n, 0);
    return true;
  } catch (e) {
    return e.code === 'EPERM';
  }
}
