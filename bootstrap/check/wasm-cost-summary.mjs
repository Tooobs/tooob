// What wasm-cost.mjs (#112) makes of its loads: each split into phases, and a case's medians and spread.

const ms = (n) => Math.round(n);

/**
 * A load's phases, in whole ms, from its timeline (ms from the shell's navigation start): the WASM
 * fetch (request to last byte, as Resource Timing has it); compile and instantiate, what's left of
 * init() after the last byte (streaming compiles while it downloads); the two together, which is what
 * the WASM adds; the service worker's registration; connect (iroh connected and the hello answered), split into
 * dial (the Visitor Key out of IndexedDB, then iroh's connection to the Host through the Relay)
 * and hello (signing and sending it, and, on a first visit, using the Invite);
 * the Site's first byte after that (the shell waits up to 800 ms for the Direct Path first); and the
 * whole load, navigation start to the Site's page loaded.
 */
export function phasesOf(t) {
  return {
    fetch: ms(t.wasmEnd - t.wasmStart),
    compile: ms(t.wasmReady - t.wasmEnd),
    wasm: ms(t.wasmReady - t.wasmStart),
    sw: ms(t.swReady - t.wasmReady),
    connect: ms(t.connected - t.swReady),
    dial: ms(t.dialed - t.swReady),
    hello: ms(t.connected - t.dialed),
    siteFirstByte: ms(t.siteFirstByte - t.connected),
    total: ms(t.siteLoaded),
  };
}

const COLUMNS = { wasm: 'WASM (fetch + compile)', fetch: 'Fetch', compile: 'Compile + instantiate', sw: 'Service worker', connect: 'Connect', dial: 'iroh dial', siteFirstByte: 'Site first byte', total: 'Total' };

function median(values) {
  const s = [...values].sort((a, b) => a - b);
  const mid = s.length >> 1;
  return s.length % 2 ? s[mid] : ms((s[mid - 1] + s[mid]) / 2);
}

/** A Markdown table of each case's phases (`runs`' `phases`), as "median (min–max)" ms, cases in the order they first ran. */
export function table(runs) {
  const cases = Map.groupBy(runs, (r) => `${r.profile}, ${r.visit}`);
  const cell = (rs, key) => {
    const values = rs.map((r) => r.phases[key]);
    return `${median(values)} (${Math.min(...values)}–${Math.max(...values)})`;
  };
  return [
    `| Case | ${Object.values(COLUMNS).join(' | ')} |`,
    `|${'---|'.repeat(Object.keys(COLUMNS).length + 1)}`,
    ...[...cases].map(([name, rs]) => `| ${name} | ${Object.keys(COLUMNS).map((key) => cell(rs, key)).join(' | ')} |`),
  ].join('\n');
}
