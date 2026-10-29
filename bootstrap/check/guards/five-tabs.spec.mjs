import { test } from '../guards.mjs';

test('opens a fifth tab', async ({ browser }) => {
  const one = await browser.newContext();
  const two = await browser.newContext();
  await one.newPage();
  await one.newPage();
  await two.newPage();
  await browser.newPage(); // a context of its own
  await two.newPage(); // the fifth
  await new Promise((r) => setTimeout(r, 60000)); // only reached if the fifth went unnoticed
});
