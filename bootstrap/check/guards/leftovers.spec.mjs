import { expect, test } from '../guards.mjs';

test('leaves tabs and contexts open', async ({ browser, page }) => {
  const other = await browser.newContext();
  await other.newPage();
  await other.newPage();
  await page.goto('about:blank');
});

test('finds none left', async ({ browser }) => {
  expect(browser.contexts()).toEqual([]);
  const cdp = await browser.newBrowserCDPSession();
  const { targetInfos } = await cdp.send('Target.getTargets');
  await cdp.detach();
  expect(targetInfos.filter((t) => t.type === 'page')).toEqual([]);
});
