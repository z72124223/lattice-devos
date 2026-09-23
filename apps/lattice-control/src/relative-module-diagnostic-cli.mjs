import { open } from 'node:fs/promises';
import { diagnoseRelativeModule } from './relative-module-diagnostic.mjs';

// Local caller-supplied evidence only; this CLI is not an authority adapter.
try {
  if (process.argv.length !== 3) throw new Error('Usage: node --experimental-vm-modules relative-module-diagnostic-cli.mjs request.json');
  const handle = await open(process.argv[2], 'r');
  const buffer = Buffer.alloc(1024 * 1024 + 1);
  let total = 0;
  try {
    while (total < buffer.length) {
      const { bytesRead } = await handle.read(buffer, total, buffer.length - total, total);
      if (!bytesRead) break;
      total += bytesRead;
    }
  } finally { await handle.close(); }
  if (total > 1024 * 1024) throw new Error('Request exceeds 1 MiB');
  const input = buffer.subarray(0, total);
  const request = JSON.parse(new TextDecoder('utf8', { fatal: true }).decode(input));
  if (!request || typeof request !== 'object' || Array.isArray(request)) throw new Error('Request must be an object');
  console.log(JSON.stringify(await diagnoseRelativeModule(request), null, 2));
} catch (error) {
  console.error(JSON.stringify({ error: error.message }));
  process.exitCode = 1;
}
