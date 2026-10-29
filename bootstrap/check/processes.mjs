// Processes, as the guards (and their tests) see them.

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
