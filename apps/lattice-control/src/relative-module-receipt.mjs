// Pure data checks only: Control must not open paths or run the producer command.
import { createHash } from 'node:crypto';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { isExecutionDenied } from './execution-recovery.mjs';

export const receiptSchema = 'lattice.relative-module-receipt.v1';
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
const digest = value => typeof value === 'string' && /^[a-f0-9]{64}$/u.test(value);
const text = (value, max = 4096) => typeof value === 'string' && value.length > 0
  && Buffer.byteLength(value) <= max && !/[\u0000-\u001f\u007f]/u.test(value);
const keys = (value, expected) => value && typeof value === 'object' && !Array.isArray(value)
  && Object.keys(value).sort().join(',') === [...expected].sort().join(',');
const bindingKeys = ['projectId', 'taskRef', 'claimId', 'threadId', 'turnId', 'inputId', 'failureItemId'];
const inside = (root, value) => {
  if (!text(root) || !text(value) || !path.isAbsolute(root) || !path.isAbsolute(value)) return false;
  const relative = path.relative(root, value);
  return relative === '' || (!path.isAbsolute(relative) && relative !== '..' && !relative.startsWith(`..${path.sep}`));
};
const samePath = (a, b) => inside(a, b) && inside(b, a);
// Keep both PowerShell literals and the native shlex display in one quote form.
// Paths needing mixed shlex chunks or PowerShell smart-quote handling fail closed.
const unsupportedQuote = /['"\u2018-\u201f$`!^]/u;
export function rejectDiagnostic(reason) {
  throw Object.assign(new Error(`唯讀診斷結果無法核對：${reason}`), { code: `CONTROL_DIAGNOSTIC_${reason}`, status: 409 });
}
export function failureEventDigest(event) {
  if (!event || !text(event.id, 256) || event.type !== 'commandExecution' || !text(event.command, 8192)
      || typeof event.aggregatedOutput !== 'string' || Buffer.byteLength(event.aggregatedOutput) > 65536
      || event.status !== 'failed' || !Number.isSafeInteger(event.exitCode) || event.exitCode === 0
      || isExecutionDenied(event)) rejectDiagnostic('FAILURE_REJECTED');
  return hash(JSON.stringify({ id: event.id, type: event.type, status: event.status,
    command: event.command, aggregatedOutput: event.aggregatedOutput, exitCode: event.exitCode }));
}
export function diagnosticReceiptCommand(requestPath, requestSha256) {
  if (!text(requestPath) || !path.isAbsolute(requestPath) || unsupportedQuote.test(requestPath)
      || !digest(requestSha256)) rejectDiagnostic('REQUEST_REJECTED');
  const cli = fileURLToPath(new URL('./relative-module-diagnostic-cli.mjs', import.meta.url));
  if (!text(cli) || unsupportedQuote.test(cli)) rejectDiagnostic('REQUEST_REJECTED');
  return `node --experimental-vm-modules --disable-warning=ExperimentalWarning '${cli}' '${requestPath}' --receipt ${requestSha256}`;
}
function matchesNativeCommand(item, inner) {
  if (!text(item.command, 16384) || !Array.isArray(item.commandActions) || item.commandActions.length !== 1
      || !keys(item.commandActions[0], ['type', 'command']) || item.commandActions[0].type !== 'unknown'
      || item.commandActions[0].command !== inner) return false;
  // Native v2 uses shlex::try_join(argv), not PowerShell command-line quoting.
  // Pinned upstream: comex/rust-shlex 4a0724b0/src/bytes.rs (1.3.0),
  // used by openai/codex 851d9e95/codex-rs/shell-command/src/parse_command.rs.
  // On this restricted alphabet both executable and inner script use one double
  // quoted token, escaping only backslash and double quote, exactly as JSON does.
  // Decode only the executable token; never parse/evaluate the supplied script.
  const prefix = /^("[^"]+") -Command /u.exec(item.command);
  if (!prefix) return false;
  let shell;
  try { shell = JSON.parse(prefix[1]); } catch { return false; }
  if (!/^[A-Za-z]:\\(?:[\p{L}\p{N} ._()-]+\\)*(?:pwsh|powershell)\.exe$/iu.test(shell)) return false;
  return item.command === `${JSON.stringify(shell)} -Command ${JSON.stringify(inner)}`;
}
export function createDiagnosticReceipt(request, evidence, result, requestPath, requestSha256) {
  const binding = request.receiptBinding;
  if (!keys(binding, bindingKeys) || !bindingKeys.every(key => text(binding[key], 256))
      || !digest(binding.taskRef) || binding.projectId !== request.context?.projectId
      || binding.taskRef !== request.context?.taskId || binding.failureItemId !== evidence?.event?.id
      || evidence.subject?.projectId !== binding.projectId || evidence.subject?.taskId !== binding.taskRef
      || !samePath(evidence.subject?.projectRoot, request.projectRoot) || !samePath(evidence.subject?.importer, request.importer)
      || !digest(request.evidenceSha256) || !digest(evidence?.subject?.importerSha256)) rejectDiagnostic('BINDING_REJECTED');
  diagnosticReceiptCommand(requestPath, requestSha256);
  return { schema: receiptSchema, binding, requestPath, requestSha256,
    evidenceSha256: request.evidenceSha256, failureEventSha256: failureEventDigest(evidence.event),
    validUntil: request.context.validUntil,
    subject: { projectRoot: request.projectRoot, importer: request.importer, importerSha256: evidence.subject.importerSha256 }, result };
}

export function verifyDiagnosticReceipt(item, failure, binding, workspace, now = Date.now()) {
  if (item?.type !== 'commandExecution' || item.status !== 'completed' || item.exitCode !== 0
      || typeof item.aggregatedOutput !== 'string' || Buffer.byteLength(item.aggregatedOutput) > 32768) rejectDiagnostic('RESULT_REJECTED');
  let receipt;
  try { receipt = JSON.parse(item.aggregatedOutput); } catch { rejectDiagnostic('FORMAT_REJECTED'); }
  if (!keys(receipt, ['schema', 'binding', 'requestPath', 'requestSha256', 'evidenceSha256',
    'failureEventSha256', 'validUntil', 'subject', 'result']) || receipt.schema !== receiptSchema
      || !keys(receipt.binding, bindingKeys) || !bindingKeys.every(key => receipt.binding[key] === binding[key])
      || !digest(receipt.evidenceSha256) || receipt.failureEventSha256 !== failureEventDigest(failure)) rejectDiagnostic('BINDING_REJECTED');
  if (!matchesNativeCommand(item, diagnosticReceiptCommand(receipt.requestPath, receipt.requestSha256))) rejectDiagnostic('COMMAND_REJECTED');
  const expires = Date.parse(receipt.validUntil);
  if (typeof receipt.validUntil !== 'string' || !Number.isFinite(expires) || expires <= now
      || expires > now + 300000) rejectDiagnostic('EXPIRED');
  if (!keys(receipt.subject, ['projectRoot', 'importer', 'importerSha256'])
      || !samePath(workspace, receipt.subject.projectRoot) || !inside(workspace, receipt.subject.importer)
      || !digest(receipt.subject.importerSha256)) rejectDiagnostic('PATH_REJECTED');
  const result = receipt.result;
  if (!keys(result, ['schema', 'decision', 'reason', 'advisoryOnly', 'adopted', 'location', 'hints', 'trust'])
      || result.schema !== 'lattice.relative-module-diagnostic.v1'
      || !['selected', 'abstain', 'excluded'].includes(result.decision)
      || result.advisoryOnly !== true || result.adopted !== false
      || !keys(result.trust, ['source', 'context', 'sourceBytesVerified', 'importerBytesVerified',
        'nativeProvenanceVerified', 'authorizationVerified'])
      || result.trust.source !== 'caller_supplied_snapshot' || result.trust.context !== 'caller_asserted_current'
      || typeof result.trust.sourceBytesVerified !== 'boolean' || typeof result.trust.importerBytesVerified !== 'boolean'
      || result.trust.nativeProvenanceVerified !== false || result.trust.authorizationVerified !== false
      || !Array.isArray(result.hints) || result.hints.length > 3 || !result.hints.every(hint => text(hint, 512))) rejectDiagnostic('FORMAT_REJECTED');
  if (result.decision === 'selected') {
    const location = result.location;
    if (result.reason !== null || !result.trust.sourceBytesVerified || !result.trust.importerBytesVerified
        || !keys(location, ['projectRoot', 'importer', 'specifier', 'target', 'nearestExistingParent', 'targetExistsNow'])
        || !samePath(location.projectRoot, workspace) || !samePath(location.importer, receipt.subject.importer)
        || !text(location.specifier, 8192) || !/^(?:\.\/|\.\.\/)/u.test(location.specifier)
        || !location.specifier.isWellFormed() || /[\\%?#:]/u.test(location.specifier)
        || !inside(workspace, location.target) || !inside(workspace, location.nearestExistingParent)
        || !samePath(location.target, path.resolve(path.dirname(location.importer), location.specifier))
        || !inside(location.nearestExistingParent, location.target)
        || location.targetExistsNow !== false) rejectDiagnostic('PATH_REJECTED');
  } else if (result.location !== null || !/^[A-Z][A-Z0-9_]{0,127}$/u.test(result.reason ?? '')) rejectDiagnostic('FORMAT_REJECTED');
  return receipt;
}
