import { readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { previewProjectPurge, applyProjectPurge, statusProjectPurge } from './project-purge.mjs';

const [action, ...args] = process.argv.slice(2);
const value = key => { const index = args.indexOf(key); return index < 0 ? undefined : args[index + 1]; };
try {
  const planPath = path.resolve(value('--plan') ?? '');
  if (!value('--plan') || !['preview', 'apply', 'status'].includes(action)) throw new Error('Usage: project-purge-client.mjs preview --input config.json --plan plan.json | apply --plan plan.json --confirm DIGEST --maintenance-offline | status --plan plan.json');
  let result;
  if (action === 'preview') {
    if (!value('--input')) throw new Error('PURGE_INPUT_REQUIRED');
    const options = JSON.parse(await readFile(value('--input'), 'utf8'));
    result = await previewProjectPurge({ ...options, protectedRoots: [...(options.protectedRoots ?? []), planPath, path.resolve(value('--input'))] });
    await writeFile(planPath, JSON.stringify(result, null, 2), { flag: 'wx', mode: 0o600 });
  } else {
    const plan = JSON.parse(await readFile(planPath, 'utf8'));
    result = action === 'apply' ? await applyProjectPurge(plan, { confirmDigest: value('--confirm'), maintenanceOffline: args.includes('--maintenance-offline') }) : await statusProjectPurge(plan);
  }
  process.stdout.write(JSON.stringify(result, null, 2) + '\n');
} catch (error) { process.stderr.write(error.message + '\n'); process.exitCode = 1; }
