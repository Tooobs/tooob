import { writeFileSync } from 'node:fs';
import { test } from '../guards.mjs';

test('hangs', async () => {
  writeFileSync(test.info().outputPath('result'), 'a result of the run'); // as a trace would be
  await new Promise(() => {});
});
