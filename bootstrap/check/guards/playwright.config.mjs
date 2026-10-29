// The guards' own tests (../guards.test.mjs) run these specs, under the browser suite's guards.
import { guarded } from '../guards.mjs';

export default guarded({ testDir: '.' });
