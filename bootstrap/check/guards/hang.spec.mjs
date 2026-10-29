import { test } from '../guards.mjs';

test('hangs', async () => {
  await new Promise(() => {});
});
