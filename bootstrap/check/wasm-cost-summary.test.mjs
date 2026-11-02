// wasm-cost-summary.mjs's tests: node --test wasm-cost-summary.test.mjs
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { phasesOf, table } from './wasm-cost-summary.mjs';

// A cold load as wasm-cost.mjs records it, in ms from the shell's navigation start.
const cold = {
  wasmStart: 120.4,
  wasmEnd: 3300.2,
  instantiateEnd: 3390.0,
  wasmReady: 3395.6,
  swReady: 3620.0,
  dialed: 3990.2,
  connected: 4150.9,
  siteFirstByte: 4990.3,
  siteLoaded: 5010.7,
};

test('a load splits into the WASM fetch, compile and instantiate after its last byte, the service worker, connect (dialling iroh, then the hello), and the Site', () => {
  assert.deepEqual(phasesOf(cold), {
    fetch: 3180,
    compile: 95,
    wasm: 3275,
    sw: 224,
    connect: 531,
    dial: 370,
    hello: 161,
    siteFirstByte: 839,
    total: 5011,
  });
});

/** A run of one case, with only the phases `table` reads. */
const run = (profile, visit, wasm, connect) => ({ profile, visit, phases: { fetch: wasm - 10, compile: 10, wasm, sw: 5, connect, dial: connect - 100, hello: 100, siteFirstByte: 800, total: wasm + connect + 805 } });

test("a case's phases, iroh's dial among them, are summed up as their median and spread, an even count's median halfway between its middle two", () => {
  const runs = [
    run('phone', 'cold', 3000, 500),
    run('phone', 'cold', 3400, 700),
    run('phone', 'cold', 2900, 450),
    run('laptop', 'return', 60, 520),
    run('laptop', 'return', 80, 480),
  ];
  assert.equal(
    table(runs),
    [
      '| Case | WASM (fetch + compile) | Fetch | Compile + instantiate | Service worker | Connect | iroh dial | Site first byte | Total |',
      '|---|---|---|---|---|---|---|---|---|',
      '| phone, cold | 3000 (2900–3400) | 2990 (2890–3390) | 10 (10–10) | 5 (5–5) | 500 (450–700) | 400 (350–600) | 800 (800–800) | 4305 (4155–4905) |',
      '| laptop, return | 70 (60–80) | 60 (50–70) | 10 (10–10) | 5 (5–5) | 500 (480–520) | 400 (380–420) | 800 (800–800) | 1375 (1365–1385) |',
    ].join('\n'),
  );
});
