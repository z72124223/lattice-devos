import { formalWorkError } from './formal-work-store.mjs';

export const elicitationMethod = 'mcpServer/elicitation/request';
const prompt = 'Allow the lattice MCP server to run tool "lattice_task_status"?';
const title = 'Read bounded LATTICE task status';
const description = 'Reads durable status for one validated task reference. General tasks need only task_ref; client_request_id remains optional for legacy canary compatibility.';
const object = value => value !== null && typeof value === 'object' && !Array.isArray(value);
const keys = (value, expected) => object(value) && Object.keys(value).sort().join(',') === [...expected].sort().join(',');
export const elicitationError = code => formalWorkError(code, 'MCP 一次性許可無法核對；未送出批准，請核對目前原生提問。');

// Only the observed empty-form task-status permission is supported. External
// message/_meta classify a request; they never supply a user's decision. Drop
// display text and persistence hints instead of retaining arbitrary tool data.
export function taskStatusElicitation(message, taskRef) {
  const p = message.params, meta = p?._meta;
  if (Buffer.byteLength(JSON.stringify(message)) > 8192
    || !keys(p, ['threadId', 'turnId', 'serverName', 'mode', '_meta', 'message', 'requestedSchema'])
    || p.serverName !== 'lattice' || p.mode !== 'form' || p.message !== prompt
    || !keys(p.requestedSchema, ['type', 'properties']) || p.requestedSchema.type !== 'object'
    || !keys(p.requestedSchema.properties, [])
    || !keys(meta, ['codex_approval_kind', 'persist', 'tool_title', 'tool_description', 'tool_params', 'tool_params_display'])
    || meta.codex_approval_kind !== 'mcp_tool_call' || meta.tool_title !== title || meta.tool_description !== description
    || JSON.stringify(meta.persist) !== '["session","always"]'
    || !keys(meta.tool_params, ['task_ref']) || meta.tool_params.task_ref !== taskRef
    || !/^[a-f0-9]{64}$/u.test(taskRef)
    || !Array.isArray(meta.tool_params_display) || meta.tool_params_display.length !== 1
    || !keys(meta.tool_params_display[0], ['name', 'value', 'display_name'])
    || meta.tool_params_display[0].name !== 'task_ref' || meta.tool_params_display[0].display_name !== 'task_ref'
    || meta.tool_params_display[0].value !== taskRef) throw elicitationError('CONTROL_ELICITATION_UNSUPPORTED');
  return { serverName: 'lattice', tool: 'lattice_task_status', task_ref: taskRef,
    actions: ['accept', 'decline', 'cancel'], scope: 'once' };
}

export function elicitationResponse(input) {
  if (!object(input) || Object.keys(input).some(key => !['projectId', 'questionId', 'action'].includes(key))
    || !['accept', 'decline', 'cancel'].includes(input.action)) throw elicitationError('CONTROL_ELICITATION_ANSWER_REJECTED');
  // Installed native protocol: action + content, never approval {decision}.
  // No _meta/persist is forwarded, so this grants no session/always permission.
  return { action: input.action, content: input.action === 'accept' ? {} : null };
}

export function elicitationDenied(_detail, claim) {
  // Runtime derives this from full relational facts before clipping previews.
  // An older/unknown Runtime cannot establish that the turn is free of denial.
  const state = claim?.mcp_permission;
  return !keys(state, ['version', 'denied']) || state.version !== 1 || state.denied !== false;
}
