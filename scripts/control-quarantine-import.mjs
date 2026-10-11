import { importControlSqliteQuarantine } from '../apps/lattice-control/src/restore-quarantine.mjs';

try {
  const args = process.argv.slice(2), names = ['--source', '--destination', '--sha256'];
  if (args.length !== 6 || names.some((name, index) => args[index * 2] !== name)) {
    throw Object.assign(new Error('Use --source ABSOLUTE_CLOSED_BACKUP --destination ABSOLUTE_FRESH_DIRECTORY --sha256 SHA256'), { code: 'CONTROL_RESTORE_ARGUMENTS_REQUIRED' });
  }
  console.log(JSON.stringify(importControlSqliteQuarantine({ source: args[1], destination: args[3], expectedSha256: args[5] })));
} catch (error) {
  console.error(JSON.stringify({ status: 'BLOCKED', code: error.code ?? 'CONTROL_RESTORE_IMPORT_FAILED', assets: 'RETAINED' }));
  process.exitCode = 2;
}
