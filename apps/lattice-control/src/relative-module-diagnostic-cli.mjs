import { open } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import path from 'node:path';
import { diagnoseRelativeModule, diagnoseRelativeModuleReceipt } from './relative-module-diagnostic.mjs';

// Local caller-supplied evidence only; this CLI is not an authority adapter.
try {
  const receiptMode = process.argv.length === 5 && process.argv[3] === '--receipt' && /^[a-f0-9]{64}$/u.test(process.argv[4]);
  if (process.argv.length !== 3 && !receiptMode) throw new Error('Usage: node --experimental-vm-modules relative-module-diagnostic-cli.mjs request.json [--receipt request-sha256]');
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
  if (receiptMode && createHash('sha256').update(input).digest('hex') !== process.argv[4]) throw new Error('Request digest mismatch');
  const request = JSON.parse(new TextDecoder('utf8', { fatal: true }).decode(input));
  if (!request || typeof request !== 'object' || Array.isArray(request)) throw new Error('Request must be an object');
  console.log(JSON.stringify(receiptMode
    ? await diagnoseRelativeModuleReceipt(request, path.resolve(process.argv[2]), process.argv[4])
    : await diagnoseRelativeModule(request), null, 2));
} catch (error) {
  console.error(JSON.stringify({ error: error.message }));
  process.exitCode = 1;
}
