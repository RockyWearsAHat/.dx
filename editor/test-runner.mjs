#!/usr/bin/env node
import { execSync } from 'child_process';
import { parseArgs } from 'util';

const { positionals } = parseArgs({ allowPositionals: true, strict: false });

try {
  const output = execSync(
    `node --test ${positionals.map(p => `"${p}"`).join(' ')} 2>&1`,
    { encoding: 'utf-8', stdio: ['pipe', 'pipe', 'pipe'] }
  );

  console.log(output);

  // Extract pass count from output
  const passMatch = output.match(/ℹ pass (\d+)/);
  if (passMatch) {
    const passCount = passMatch[1];
    console.log(`files ${passCount} pass`);
  }
} catch (error) {
  console.error(error.stdout || error.message);
  process.exit(1);
}
