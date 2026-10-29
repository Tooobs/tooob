// The browser suite: npx playwright test (after ../build.sh), in windows you can watch. Under the
// guards in guards.mjs, so a run can't take over the machine (#82).
import { guarded } from './guards.mjs';

export default guarded({ testDir: '.', testIgnore: 'guards/**' });
