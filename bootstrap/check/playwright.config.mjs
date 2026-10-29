// The browser suite: npx playwright test (after ../build.sh), in windows you can watch. Under the
// guards in guards.mjs, so a run can't take over the machine (#82).
import { guarded } from './guards.mjs';

// Its results, and the .last-run.json release.sh reads, go in test-results/ here.
export default await guarded({ testDir: '.', testIgnore: 'guards/**', outputDir: new URL('test-results', import.meta.url).pathname });
