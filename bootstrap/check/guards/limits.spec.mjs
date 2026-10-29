import { test } from '../guards.mjs';

test('says its limits', async () => {
  console.log(`limits: test ${test.info().timeout} ms, run ${test.info().config.globalTimeout} ms`);
});
