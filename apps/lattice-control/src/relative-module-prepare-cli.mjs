import { prepareRelativeModule, readRelativeModulePreparationInput } from './relative-module-diagnostic.mjs';

// No subprocess, network, database, automatic binding discovery, or command execution.
try {
  if (process.argv.length !== 3) throw new Error('INVALID_ARGUMENTS');
  const result = await prepareRelativeModule(await readRelativeModulePreparationInput(process.argv[2]));
  console.log(JSON.stringify(result, null, 2));
  if (result.decision !== 'prepared') process.exitCode = 1;
} catch {
  console.error(JSON.stringify({ error: 'INVALID_PREPARATION_INPUT' }));
  process.exitCode = 1;
}
