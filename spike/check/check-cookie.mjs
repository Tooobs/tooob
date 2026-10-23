// Cookie login through tooob: form POST -> 303 + HttpOnly Set-Cookie -> logged in, survives a reload,
// and the session cookie never reaches the browser.  usage: node check-cookie.mjs <visit-url>
import puppeteer from 'puppeteer-core';

const [url] = process.argv.slice(2);
if (!url) throw new Error('usage: node check-cookie.mjs <visit-url of a host in front of cookie-app.mjs>');
const browser = await puppeteer.launch({ executablePath: '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome', headless: true });
const page = await browser.newPage();
let ok = true;
const report = (name, pass, detail = '') => {
  ok &&= pass;
  console.log(`${pass ? 'PASS' : 'FAIL'}  ${name}  ${detail}`);
};
const siteFrame = async () => (await page.waitForSelector('iframe')).contentFrame();

await page.goto(url);
let frame = await siteFrame();
await frame.waitForSelector('#go', { timeout: 30000 });
await Promise.all([frame.waitForNavigation(), frame.click('#go')]);
const who = await frame.$eval('#who', (e) => e.textContent).catch(() => null);
report('6a. form login (POST, 303, HttpOnly cookie) works', who === 'logged in as alice', String(who));
report('6b. session cookie stays on the host', !(await frame.evaluate(() => document.cookie)).includes('sid='));

await page.reload(); // new iroh connection, same visitor key
frame = await siteFrame();
const me = await frame.waitForSelector('#who', { timeout: 30000 }).then(() => frame.evaluate(() => fetch('/me').then((r) => r.json()))).catch((e) => ({ error: String(e) }));
report('6c. still logged in after reload', me.user === 'alice', JSON.stringify(me));

await browser.close();
process.exit(ok ? 0 : 1);
