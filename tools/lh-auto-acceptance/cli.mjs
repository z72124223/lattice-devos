import { summarize } from './summary.mjs';

try {
  if (process.argv[2] === undefined) {
    throw new TypeError('Expected a JSON task array as argv[2].');
  }
  const result = summarize(JSON.parse(process.argv[2]));
  process.stdout.write(`${JSON.stringify(result)}\n`);
} catch (error) {
  process.stderr.write(`Invalid task input: ${error.message}\n`);
  process.exitCode = 1;
}
