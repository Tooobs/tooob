import { expect, test } from '../guards.mjs';

const alive = (pid) => {
  try {
    return process.kill(Number(pid), 0);
  } catch {
    return false;
  }
};

test("finds the killed run's Hosts and Relays stopped", async () => {
  const leftovers = process.env.LEFTOVERS.split(',');
  expect(leftovers.filter(alive)).toEqual([]);
});
