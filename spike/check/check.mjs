// Spike pass/fail check: drives headless Chrome through the tooob shell and measures the DESIGN.md bar.
// usage: node check.mjs <host-id or visit-url> [--idle <seconds>] [--ws]
import puppeteer from 'puppeteer-core';

const [host] = process.argv.slice(2);
const arg = (name) => process.argv.indexOf(name);
const idleSec = arg('--idle') > 0 ? Number(process.argv[arg('--idle') + 1]) : 0;
// Accepts a bare host id or the full visit URL the host prints (which carries ?relay=).
const SHELL = host?.startsWith("http") ? host : `http://localhost:8088/#${host}`;
if (!host) throw new Error('usage: node check.mjs <host-id or visit-url> [--idle <seconds>] [--ws]');

const browser = await puppeteer.launch({
  executablePath: '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
  headless: true,
  args: ['--autoplay-policy=no-user-gesture-required'],
});
const page = await browser.newPage();
page.on('console', (m) => process.env.VERBOSE && console.log('  console:', m.text()));
const results = [];
const report = (name, ok, detail) => {
  results.push(ok);
  console.log(`${ok ? 'PASS' : 'FAIL'}  ${name}  ${detail}`);
};
const stats = () => page.waitForFunction(() => window.__tooobStats?.pageMs, { timeout: 30000 }).then((h) => h.jsonValue()).then(() => page.evaluate(() => window.__tooobStats));

// Cold load, then a reload with the bootstrap cached (the "first visit" the bar is about).
await page.goto(SHELL);
const cold = await stats();
console.log(`info  cold load: wasm ${cold.wasmMs} ms, connect ${cold.connectMs} ms, page ${cold.pageMs} ms`);
await page.reload();
const warm = await stats();
report("1. 2 MB page < 3 s (bootstrap cached)", warm.totalMs < 3000, `${warm.totalMs} ms from navigation (sw ${warm.swMs}, connect ${warm.connectMs}, page ${warm.pageMs})`);

// Video: start, then seek around.
const video = await page.evaluate(async () => {
  const f = document.querySelector('iframe');
  await new Promise((r) => { f.onload = r; f.contentWindow.location.href = '/video.html'; });
  const v = f.contentDocument.getElementById('v');
  v.muted = true;
  const time = (ev, fn) => new Promise((r, j) => {
    const t = performance.now();
    const to = setTimeout(() => j(new Error(`${ev} timeout`)), 15000);
    v.addEventListener(ev, () => { clearTimeout(to); r(Math.round(performance.now() - t)); }, { once: true });
    fn();
  });
  const startMs = await time('playing', () => v.play());
  const seeks = [];
  for (const s of [60, 15, 100, 40]) seeks.push(await time('seeked', () => (v.currentTime = s)));
  return { startMs, seeks };
}).catch((e) => ({ error: String(e) }));
if (video.error) report('2. video seek ~2 s', false, video.error);
else report('2. video seek ~2 s', Math.max(...video.seeks) <= 2500, `start ${video.startMs} ms, seeks ${video.seeks.join(' / ')} ms`);

// WASM size is checked by build.sh; repeat it here so one run shows the whole bar.
const wasm = await page.evaluate(async () => (await (await fetch('/__tooob/pkg/web_bg.wasm')).arrayBuffer()).byteLength);
report('3. WASM < 5 MB', wasm < 5 * 1024 * 1024, `${(wasm / 1048576).toFixed(2)} MB uncompressed`);

if (idleSec) {
  console.log(`info  idling ${idleSec} s…`);
  await new Promise((r) => setTimeout(r, idleSec * 1000));
  const afterIdle = await page.evaluate(async () => {
    const t = performance.now();
    const r = await document.querySelector('iframe').contentWindow.fetch('/index.html', { cache: 'no-store' });
    await r.text();
    return { status: r.status, ms: Math.round(performance.now() - t) };
  }).catch((e) => ({ error: String(e) }));
  report(`4. survives ${idleSec} s idle`, afterIdle.status === 200, JSON.stringify(afterIdle));
}

await browser.close();
process.exit(results.every(Boolean) ? 0 : 1);
