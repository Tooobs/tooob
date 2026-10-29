import { alive, expect, test } from '../guards.mjs';

test("finds the killed run's Hosts and Relays stopped", async () => {
  const leftovers = process.env.LEFTOVERS.split(',');
  expect(leftovers.filter(alive)).toEqual([]);
});
