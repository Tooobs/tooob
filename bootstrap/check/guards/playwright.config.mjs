// The guards' own tests (../guards.test.mjs) run these specs, under the browser suite's guards.
import { guarded } from '../guards.mjs';

// Its own output folder, so these runs leave the suite's results (and release.sh's .last-run.json) be.
export default await guarded({ testDir: '.', outputDir: new URL('test-results', import.meta.url).pathname });
