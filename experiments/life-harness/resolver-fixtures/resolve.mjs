// Each invocation is a separate native command. Only resolver arguments are restricted;
// process.env and installed tools are never modified. Exceptions are not rewritten.
import assert from 'node:assert/strict';
import path from 'node:path';
import { resolveGitExecutable } from '../../../apps/lattice-control/src/project-inspector.mjs';
import { resolveWindowsCodexRuntime } from '../../../apps/lattice-control/src/codex-runtime-resolution.mjs';

const [tool, mode] = process.argv.slice(2);
assert.equal(process.argv.length, 4);
assert.ok(['git', 'codex'].includes(tool));
assert.ok(['missing', 'success'].includes(mode));
assert.equal(process.platform, 'win32');
console.log(JSON.stringify({ tool, mode, process_env_mutated: false,
  resolver_options: mode === 'missing'
    ? tool === 'git' ? { pathValue: '' } : { env: {} }
    : { inherited: tool === 'git' ? ['PATH'] : ['LOCALAPPDATA', 'APPDATA'] } }));

const command = tool === 'git'
  ? await resolveGitExecutable({ cwd: process.cwd(), ...(mode === 'missing' ? { pathValue: '' } : {}) })
  : (await resolveWindowsCodexRuntime({ env: mode === 'missing' ? {} : {
    LOCALAPPDATA: process.env.LOCALAPPDATA, APPDATA: process.env.APPDATA,
  } })).command;
// Do not execute the returned command or App Server arguments. This is resolution only.
console.log(JSON.stringify({ result: 'resolved', tool, command_basename: path.basename(command) }));
