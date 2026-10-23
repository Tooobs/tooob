// Spike 2: the direct WebRTC path. Runs the page + video load in three modes and reports which
// path carried the bytes.  usage: node check-direct.mjs <visit-url>
import puppeteer from 'puppeteer-core';

const [base] = process.argv.slice(2);
if (!base) throw new Error('usage: node check-direct.mjs <visit-url>');
const withMode = (mode) => {
  const u = new URL(base);
  if (mode) u.searchParams.set('direct', mode);
  return u.href;
};

const browser = await puppeteer.launch({
  executablePath: '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
  headless: true,
  args: ['--autoplay-policy=no-user-gesture-required'],
});
let ok = true;
const report = (name, pass, detail = '') => {
  ok &&= pass;
  console.log(`${pass ? 'PASS' : 'FAIL'}  ${name}  ${detail}`);
};

async function run(mode) {
  const page = await browser.newPage();
  page.on('console', (m) => process.env.VERBOSE && console.log('  console:', m.text()));
  await page.goto(withMode(mode));
  // Wait for the path decision, then measure a fresh page load and some video on whatever path won.
  await page.waitForFunction(() => window.__tooobStats?.path && window.__tooobStats?.pageMs, { timeout: 30000 });
  const r = await page.evaluate(async () => {
    const s = window.__tooobStats;
    const before = { ...s.bytes };
    const f = document.querySelector('iframe');
    let t = performance.now();
    await new Promise((res) => { f.onload = res; f.contentWindow.location.href = '/?again=' + Date.now(); });
    const pageMs = Math.round(performance.now() - t);
    await new Promise((res) => { f.onload = res; f.contentWindow.location.href = '/video.html'; });
    const v = f.contentDocument.getElementById('v');
    v.muted = true;
    const time = (ev, fn) => new Promise((res, rej) => {
      const t0 = performance.now();
      const to = setTimeout(() => rej(new Error(`${ev} timeout`)), 15000);
      v.addEventListener(ev, () => { clearTimeout(to); res(Math.round(performance.now() - t0)); }, { once: true });
      fn();
    });
    await time('playing', () => v.play());
    const seeks = [];
    for (const at of [60, 15, 100]) seeks.push(await time('seeked', () => (v.currentTime = at)));
    return {
      path: s.path, directMs: s.directMs, fingerprintOk: s.fingerprintOk, pageMs, seeks,
      iroh: s.bytes.iroh - before.iroh, direct: s.bytes.direct - before.direct,
    };
  }).catch((e) => ({ error: String(e) }));
  await page.close();
  return r;
}

const mb = (n) => (n / 1048576).toFixed(1) + ' MB';

const d = await run(null);
if (d.error) report('direct mode', false, d.error);
else {
  report('1. direct path negotiated over iroh', d.path === 'direct', `path=${d.path}, set up in ${d.directMs} ms`);
  const share = d.direct / (d.direct + d.iroh || 1);
  report('2. traffic moved to it', share > 0.95, `direct ${mb(d.direct)} vs iroh ${mb(d.iroh)}; page ${d.pageMs} ms; seeks ${d.seeks.join(' / ')} ms`);
  report('4. DTLS cert matches the fingerprint sent over iroh', d.fingerprintOk !== false, `fingerprintOk=${d.fingerprintOk}`);
}

const f = await run('fail');
if (f.error) report('3. fallback when direct fails', false, f.error);
else report('3. fallback when direct fails', f.path === 'iroh (direct failed)' && f.direct === 0 && f.iroh > 0,
  `path=${f.path}; iroh ${mb(f.iroh)}; page ${f.pageMs} ms; seeks ${f.seeks.join(' / ')} ms`);

const o = await run('0');
if (!o.error) console.log(`info  relay-only baseline: page ${o.pageMs} ms; seeks ${o.seeks.join(' / ')} ms; iroh ${mb(o.iroh)}`);

await browser.close();
process.exit(ok ? 0 : 1);
