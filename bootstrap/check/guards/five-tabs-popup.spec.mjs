import { test } from '../guards.mjs';

test('a page opens a fifth tab itself', async ({ browser }) => {
  const context = await browser.newContext();
  const pages = [await context.newPage(), await context.newPage(), await context.newPage(), await context.newPage()];
  await pages[0].evaluate(() => window.open('about:blank'));
  await new Promise((r) => setTimeout(r, 1000));
});
