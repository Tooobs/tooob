import { test } from '../guards.mjs';

// Each test is inside its own limit; together they aren't inside the run's.
for (const n of [1, 2, 3, 4]) {
  test(`takes a while, ${n}`, async () => {
    await new Promise((r) => setTimeout(r, 2000));
  });
}
