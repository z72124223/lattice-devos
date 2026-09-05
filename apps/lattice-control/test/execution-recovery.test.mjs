import test from 'node:test';
import assert from 'node:assert/strict';
import { isExecutionDenied, deniedItemIds } from '../src/execution-recovery.mjs';

test('only native failed tool evidence triggers recovery, not quotations or successful reads', () => {
  const text = 'CreateProcess rejected: blocked by policy';
  for (const item of [
    { type: 'commandExecution', status: 'declined' },
    { type: 'commandExecution', status: 'failed', aggregatedOutput: text },
    { type: 'mcpToolCall', status: 'failed', error: { message: text } },
    { type: 'dynamicToolCall', status: 'completed', success: false, contentItems: [{ type: 'inputText', text }] },
  ]) assert.equal(isExecutionDenied({ id: 'denial-1', ...item }), true);
  for (const item of [
    { type: 'agentMessage', text },
    { type: 'userMessage', content: [{ type: 'text', text }] },
    { type: 'commandExecution', status: 'completed', aggregatedOutput: text },
    { type: 'commandExecution', status: 'failed', aggregatedOutput: 'connection refused' },
    { type: 'mcpToolCall', status: 'completed', result: { content: [{ type: 'text', text }] } },
    { type: 'dynamicToolCall', status: 'completed', success: true, contentItems: [{ type: 'inputText', text }] },
    { type: 'commandExecution', status: 'inProgress', aggregatedOutput: text },
  ]) assert.equal(isExecutionDenied({ id: 'other-1', ...item }), false);
  assert.equal(isExecutionDenied({ type: 'commandExecution', status: 'declined' }), false);
});

test('replayed native item identifiers count once', () => {
  const item = { type: 'commandExecution', id: 'same', status: 'declined' };
  assert.equal(deniedItemIds({ items: [item, { ...item }] }).size, 1);
});
