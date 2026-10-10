import { createHash } from 'node:crypto';
import { lstat, mkdir, open, realpath, writeFile } from 'node:fs/promises';
import path from 'node:path';
import vm from 'node:vm';
import { pathToFileURL } from 'node:url';
import { canonicalizeProjectPath, normalizeRequestedProjectPath } from './project-inspector.mjs';
import { isExecutionDenied } from './execution-recovery.mjs';
import { createDiagnosticReceipt, diagnosticCommand, diagnosticReceiptCommand,
  failureEventDigest, validateDiagnosticBinding } from './relative-module-receipt.mjs';

const maximumBytes = 1024 * 1024;
const sha256 = bytes => createHash('sha256').update(bytes).digest('hex');
const digest = value => typeof value === 'string' && /^[a-f0-9]{64}$/u.test(value);
const text = value => typeof value === 'string' && value.length > 0 && value.length <= 8192 && !/[\0\r\n]/u.test(value);
const samePath = (a, b) => process.platform === 'win32'
  ? path.normalize(a).toLowerCase() === path.normalize(b).toLowerCase() : path.normalize(a) === path.normalize(b);
const inside = (root, value) => {
  const relative = path.relative(root, value);
  return relative === '' || (!path.isAbsolute(relative) && relative !== '..' && !relative.startsWith(`..${path.sep}`));
};
const fail = code => { throw Object.assign(new Error(code), { diagnosticCode: code }); };
const identity = (a, b) => a.dev === b.dev && a.ino === b.ino;
const unchanged = (a, b) => identity(a, b) && a.size === b.size && a.mtimeNs === b.mtimeNs && a.ctimeNs === b.ctimeNs;

// Forward traversal checks every existing ancestor before visiting its child,
// including parents of a missing target. Reject links even when they stay inside.
async function inspectPath(root, requested, allowMissing = false) {
  const absolute = normalizeRequestedProjectPath(requested);
  if (!inside(root, absolute)) fail('PATH_OUTSIDE_PROJECT');
  let cursor = root;
  const segments = path.relative(root, absolute).split(path.sep).filter(Boolean);
  let last = await lstat(root, { bigint: true });
  if (last.isSymbolicLink() || !samePath(await realpath(root), root)) fail('PATH_REDIRECTED');
  for (const [index, segment] of segments.entries()) {
    if (!last.isDirectory()) fail('PATH_PARENT_NOT_DIRECTORY');
    cursor = path.join(cursor, segment);
    try { last = await lstat(cursor, { bigint: true }); }
    catch (error) {
      if (allowMissing && error.code === 'ENOENT') return { exists: false, nearestExistingParent: path.dirname(cursor) };
      throw error;
    }
    if (last.isSymbolicLink()) fail('PATH_REDIRECTED');
    const canonical = await realpath(cursor);
    if (!inside(root, canonical) || !samePath(canonical, cursor)) fail('PATH_REDIRECTED');
    if (index < segments.length - 1 && !last.isDirectory()) fail('PATH_PARENT_NOT_DIRECTORY');
  }
  return { exists: true, details: last };
}

// Explicit caller-selected files only, with bounded reads and identity/content
// drift checks. These observations are not an atomic hostile-filesystem sandbox.
async function readCheckedFile(root, filename) {
  const before = await inspectPath(root, filename);
  if (!before.details.isFile()) fail('SOURCE_NOT_REGULAR_FILE');
  if (before.details.size > BigInt(maximumBytes)) fail('SOURCE_TOO_LARGE');
  const handle = await open(filename, 'r');
  try {
    const opened = await handle.stat({ bigint: true });
    if (!unchanged(before.details, opened)) fail('SOURCE_CHANGED');
    const buffer = Buffer.alloc(maximumBytes + 1);
    let total = 0;
    while (total < buffer.length) {
      const { bytesRead } = await handle.read(buffer, total, buffer.length - total, total);
      if (!bytesRead) break;
      total += bytesRead;
    }
    if (total > maximumBytes) fail('SOURCE_TOO_LARGE');
    const after = await handle.stat({ bigint: true });
    const current = await inspectPath(root, filename);
    if (!unchanged(opened, after) || !unchanged(after, current.details) || after.size !== BigInt(total)) fail('SOURCE_CHANGED');
    return buffer.subarray(0, total);
  } finally { await handle.close(); }
}

function contextExclusion(context, now) {
  if (context?.authority === 'denied') return 'DENIED';
  if (context?.circuitOpen === true) return 'CIRCUIT_OPEN';
  if (!text(context?.projectId) || !text(context?.taskId) || context?.projectScope !== 'matched'
      || context?.authority !== 'authorized') return 'IDENTITY_OR_AUTHORITY_UNKNOWN';
  if (context.lifecycle !== 'active') return 'LIFECYCLE_INAPPLICABLE';
  if (context.freshness !== 'current' || !Number.isFinite(Date.parse(context.validUntil))
      || Date.parse(context.validUntil) <= now) return 'STALE_OR_UNKNOWN';
  if (context.circuitOpen !== false) return 'CIRCUIT_STATE_UNKNOWN';
  return null;
}

function inspectFailure(event, importer) {
  if (!text(event?.id)) fail('INCOMPLETE_EVENT');
  if (isExecutionDenied(event)) return { excluded: 'DENIED' };
  if (event.type !== 'commandExecution' || event.status !== 'failed') return { excluded: 'NOT_FAILED_COMMAND' };
  if (!text(event.command) || typeof event.aggregatedOutput !== 'string'
      || event.aggregatedOutput.length > 64 * 1024) fail('INCOMPLETE_EVENT');
  if (!Number.isSafeInteger(event.exitCode) || event.exitCode === 0) fail('INCOMPLETE_FAILURE');
  if (/Cannot find package\b/u.test(event.aggregatedOutput)) fail('THIRD_PARTY_PACKAGE');
  if (!event.aggregatedOutput.includes('ERR_MODULE_NOT_FOUND')) fail('UNSUPPORTED_FAILURE');
  const matches = [...event.aggregatedOutput.matchAll(/^Error \[ERR_MODULE_NOT_FOUND\]: Cannot find module '([^'\r\n]+)' imported from ([^\r\n]+)$/gmu)];
  if (matches.length !== 1) fail('INSUFFICIENT_FAILURE_DETAILS');
  // Reported paths are compared only; they never authorize filesystem access.
  const [, reportedTarget, reportedImporter] = matches[0];
  if (!path.isAbsolute(reportedTarget) || !path.isAbsolute(reportedImporter)
      || !samePath(reportedImporter, importer)) fail('FAILURE_IMPORTER_MISMATCH');
  return { reportedTarget };
}

/**
 * Caller/adapter boundary: source JSON and hashes establish byte consistency,
 * not native provenance or current permission. The caller must independently
 * verify its task identity, original event and live read authority. JSON cannot
 * turn those assertions into verified authority; every output says so explicitly.
 * This module never links/evaluates project code or executes a recovery action.
 */
async function diagnose(request, { now = Date.now() } = {}, capture = null, preparedEvidenceBytes = null) {
  const output = { schema: 'lattice.relative-module-diagnostic.v1', decision: 'abstain', reason: null,
    advisoryOnly: true, adopted: false, location: null, hints: [],
    trust: { source: 'caller_supplied_snapshot', context: 'caller_asserted_current',
      sourceBytesVerified: false, importerBytesVerified: false,
      nativeProvenanceVerified: false, authorizationVerified: false } };
  const finish = (decision, reason) => ({ ...output, decision, reason });
  try {
    if (!request || typeof request !== 'object' || Array.isArray(request) || !Number.isFinite(now)) fail('INVALID_REQUEST');
    const excluded = contextExclusion(request.context, now);
    if (excluded) return finish('excluded', excluded);
    const { context } = request;
    const projectRoot = normalizeRequestedProjectPath(request.projectRoot);
    const importer = normalizeRequestedProjectPath(request.importer);
    const evidenceFile = normalizeRequestedProjectPath(request.evidenceFile);
    if (!digest(request.evidenceSha256)) fail('SOURCE_DIGEST_REQUIRED');
    // An archive may be outside the project only because the caller explicitly
    // named it; no path parsed from a log ever reaches a filesystem API.
    const evidenceBytes = preparedEvidenceBytes ?? await readCheckedFile(
      await canonicalizeProjectPath(path.dirname(evidenceFile)), evidenceFile);
    if (sha256(evidenceBytes) !== request.evidenceSha256) fail('SOURCE_INTEGRITY_MISMATCH');
    const evidence = JSON.parse(new TextDecoder('utf8', { fatal: true }).decode(evidenceBytes));
    const { subject, event } = evidence;
    if (evidence.schema !== 'lattice.relative-module-evidence.v1' || !subject || !event) fail('INVALID_SOURCE');
    output.trust.sourceBytesVerified = true;
    if (capture) capture.evidence = evidence;
    if (subject.projectId !== context.projectId || subject.taskId !== context.taskId
        || !text(subject.projectRoot) || !text(subject.importer)
        || !samePath(subject.projectRoot, projectRoot) || !samePath(subject.importer, importer)) {
      return finish('excluded', 'SUBJECT_MISMATCH');
    }
    const { excluded: failureExcluded, reportedTarget } = inspectFailure(event, importer);
    if (failureExcluded) return finish('excluded', failureExcluded);
    const root = await canonicalizeProjectPath(projectRoot);
    if (!inside(root, importer)) fail('PATH_OUTSIDE_PROJECT');
    if (!['.mjs', '.js'].includes(path.extname(importer))) fail('UNSUPPORTED_SOURCE_TYPE');
    if (!digest(subject.importerSha256)) fail('IMPORTER_DIGEST_REQUIRED');
    const bytes = await readCheckedFile(root, importer);
    if (sha256(bytes) !== subject.importerSha256) fail('IMPORTER_INTEGRITY_MISMATCH');
    output.trust.importerBytesVerified = true;
    if (typeof vm.SourceTextModule !== 'function') fail('PARSER_UNAVAILABLE');
    let module;
    try {
      module = new vm.SourceTextModule(new TextDecoder('utf8', { fatal: true }).decode(bytes),
        { identifier: pathToFileURL(importer).href, context: vm.createContext(Object.create(null)) });
    } catch { fail('UNSUPPORTED_SYNTAX'); }
    if (!Array.isArray(module.moduleRequests) || module.status !== 'unlinked') fail('PARSER_UNAVAILABLE');
    const candidates = [];
    for (const item of module.moduleRequests) {
      if (item.phase !== 'evaluation' || Object.keys(item.attributes).length) fail('UNSUPPORTED_IMPORT_ATTRIBUTES');
      if (!item.specifier.startsWith('./') && !item.specifier.startsWith('../')) continue;
      if (!item.specifier.isWellFormed() || /[\\%?#:\u0000-\u001f\u007f]/u.test(item.specifier)) fail('UNSUPPORTED_SPECIFIER');
      const target = path.resolve(path.dirname(importer), item.specifier);
      if (!inside(root, target)) fail('PATH_OUTSIDE_PROJECT');
      if (samePath(target, reportedTarget)) candidates.push({ specifier: item.specifier, target });
    }
    if (candidates.length !== 1) fail('NO_UNIQUE_STATIC_RELATIVE_DEPENDENCY');
    const { specifier, target } = candidates[0];
    const inspected = await inspectPath(root, target, true);
    if (inspected.exists) fail('TARGET_EXISTS_NOW');
    // Re-read source and paths after parsing; a changed observation is an abstention.
    if (!samePath(await canonicalizeProjectPath(projectRoot), root)
        || sha256(await readCheckedFile(root, importer)) !== subject.importerSha256) fail('SOURCE_CHANGED');
    if ((await inspectPath(root, target, true)).exists) fail('TARGET_CHANGED');
    output.location = { projectRoot: root, importer, specifier, target,
      nearestExistingParent: inspected.nearestExistingParent, targetExistsNow: false };
    output.hints = ['核對相對模組名稱、大小寫、副檔名及是否漏存或改名。',
      '相對路徑以匯入檔所在目錄解析；命令的工作目錄不能取代此基準。',
      '本結果僅為唯讀檢查提示，未驗證正式授權，不執行安裝、修復或重試。'];
    return finish('selected', null);
  } catch (error) {
    return finish('abstain', error.diagnosticCode ?? error.code ?? 'INVALID_SOURCE');
  }
}

export async function diagnoseRelativeModule(request, options) {
  return diagnose(request, options);
}

// Only the explicitly invoked CLI uses this mode; Control never calls it.
export async function diagnoseRelativeModuleReceipt(request, requestPath, requestSha256) {
  const capture = {};
  const result = await diagnose(request, undefined, capture);
  return createDiagnosticReceipt(request, capture.evidence, result, requestPath, requestSha256);
}

// The CLI's explicitly named configuration is the only unavoidable read before
// the context gate. Event/importer reads and all writes happen in prepare below.
export async function readRelativeModulePreparationInput(filename) {
  const absolute = normalizeRequestedProjectPath(filename);
  const bytes = await readCheckedFile(await canonicalizeProjectPath(path.dirname(absolute)), absolute);
  return JSON.parse(new TextDecoder('utf8', { fatal: true }).decode(bytes));
}

/** Prepare local artifacts only; never execute the returned command or event.
 * Explicit paths/context remain caller assertions. As with readCheckedFile, the
 * repeated filesystem observations do not form an atomic hostile-FS sandbox.
 * On any failure keep partial output for inspection; never overwrite or clean it.
 */
export async function prepareRelativeModule(input, { now = Date.now } = {}) {
  const output = { schema: 'lattice.relative-module-preparation.v1', decision: 'abstain', reason: null,
    advisoryOnly: true, adopted: false, retainedOutputDirectory: null, artifacts: null, command: null,
    trust: { source: 'caller_supplied_snapshot', context: 'caller_asserted_current',
      sourceBytesVerified: false, importerBytesVerified: false,
      nativeProvenanceVerified: false, authorizationVerified: false } };
  const finish = (decision, reason) => ({ ...output, decision, reason });
  try {
    // Capture caller data before the first await; never mutate it or extend expiry.
    input = structuredClone(input);
    if (!input || typeof input !== 'object' || Array.isArray(input) || !Number.isFinite(now())) fail('INVALID_REQUEST');
    const gate = () => contextExclusion(input.context, now());
    const excluded = gate();
    if (excluded) return finish('excluded', excluded);
    const mode = input.mode ?? 'diagnostic';
    if (!['diagnostic', 'receipt'].includes(mode)) fail('INVALID_MODE');
    if (mode === 'receipt') {
      validateDiagnosticBinding(input.receiptBinding, input.context);
      if (Date.parse(input.context.validUntil) > now() + 300000) fail('RECEIPT_EXPIRY_OUT_OF_RANGE');
    } else if (input.receiptBinding !== undefined) fail('RECEIPT_MODE_REQUIRED');
    const projectRoot = normalizeRequestedProjectPath(input.projectRoot);
    const importer = normalizeRequestedProjectPath(input.importer);
    const failureFile = normalizeRequestedProjectPath(input.failureFile);
    const outputDirectory = normalizeRequestedProjectPath(input.outputDirectory);
    if (!inside(projectRoot, importer)) fail('PATH_OUTSIDE_PROJECT');
    if (!digest(input.failureSha256)) fail('SOURCE_DIGEST_REQUIRED');
    const evidenceFile = path.join(outputDirectory, 'evidence.json'), requestPath = path.join(outputDirectory, 'request.json');
    diagnosticCommand(requestPath); // Reject unsafe shell literals before any I/O.
    const archiveRoot = await canonicalizeProjectPath(path.dirname(failureFile));
    const sourceBytes = await readCheckedFile(archiveRoot, failureFile);
    if (sha256(sourceBytes) !== input.failureSha256) fail('SOURCE_INTEGRITY_MISMATCH');
    const original = JSON.parse(new TextDecoder('utf8', { fatal: true }).decode(sourceBytes));
    const { excluded: failureExcluded } = inspectFailure(original, importer);
    if (failureExcluded) return finish('excluded', failureExcluded);
    // Preserve the original full shell wrapper, but do not copy unrelated fields.
    failureEventDigest(original);
    const event = Object.fromEntries(['id', 'type', 'status', 'command', 'aggregatedOutput', 'exitCode'].map(key => [key, original[key]]));
    if (mode === 'receipt' && input.receiptBinding.failureItemId !== event.id) fail('BINDING_REJECTED');
    const root = await canonicalizeProjectPath(projectRoot);
    const importerBytes = await readCheckedFile(root, importer);
    const evidenceBytes = Buffer.from(JSON.stringify({ schema: 'lattice.relative-module-evidence.v1',
      subject: { projectId: input.context.projectId, taskId: input.context.taskId,
        projectRoot, importer, importerSha256: sha256(importerBytes) }, event }));
    const request = { projectRoot, importer, evidenceFile, evidenceSha256: sha256(evidenceBytes), context: input.context,
      ...(mode === 'receipt' ? { receiptBinding: input.receiptBinding } : {}) };
    const requestBytes = Buffer.from(JSON.stringify(request));
    if (evidenceBytes.length > maximumBytes || requestBytes.length > maximumBytes) fail('SOURCE_TOO_LARGE');
    // Reuse the actual diagnostic (parse only, no evaluation) before creating output.
    // This private bytes path cannot be selected by the existing diagnostic API/CLI.
    const diagnostic = await diagnose(request, { now: now() }, null, evidenceBytes);
    if (diagnostic.decision !== 'selected') return finish(diagnostic.decision, diagnostic.reason);
    if (inside(outputDirectory, diagnostic.location.target) || inside(diagnostic.location.target, outputDirectory)) fail('OUTPUT_TARGET_COLLISION');
    const command = mode === 'receipt' ? diagnosticReceiptCommand(requestPath, sha256(requestBytes)) : diagnosticCommand(requestPath);
    const parent = await canonicalizeProjectPath(path.dirname(outputDirectory));
    const parentIdentity = await lstat(parent, { bigint: true });
    if ((await inspectPath(parent, outputDirectory, true)).exists) fail('OUTPUT_EXISTS');
    const checkSources = async () => {
      if (gate()) fail('CONTEXT_EXPIRED_OR_CHANGED');
      if (!samePath(await canonicalizeProjectPath(projectRoot), root)
          || !samePath(await canonicalizeProjectPath(path.dirname(failureFile)), archiveRoot)
          || sha256(await readCheckedFile(archiveRoot, failureFile)) !== input.failureSha256
          || sha256(await readCheckedFile(root, importer)) !== sha256(importerBytes)) fail('SOURCE_CHANGED');
      if ((await inspectPath(root, diagnostic.location.target, true)).exists) fail('TARGET_CHANGED');
      if (gate()) fail('CONTEXT_EXPIRED_OR_CHANGED');
    };
    await checkSources();
    await mkdir(outputDirectory); // No recursive parents, no reuse of an existing directory.
    output.retainedOutputDirectory = outputDirectory;
    const directoryIdentity = (await inspectPath(parent, outputDirectory)).details;
    const checkOutput = async () => {
      await canonicalizeProjectPath(parent);
      if (!identity(parentIdentity, await lstat(parent, { bigint: true }))
          || !identity(directoryIdentity, (await inspectPath(parent, outputDirectory)).details)) fail('OUTPUT_CHANGED');
    };
    for (const [filename, bytes] of [[evidenceFile, evidenceBytes], [requestPath, requestBytes]]) {
      if (gate()) fail('CONTEXT_EXPIRED_OR_CHANGED');
      await checkOutput();
      await writeFile(filename, bytes, { flag: 'wx', mode: 0o600 });
      await checkOutput();
      if (sha256(await readCheckedFile(outputDirectory, filename)) !== sha256(bytes)) fail('OUTPUT_CHANGED');
    }
    await checkSources();
    await checkOutput();
    if (gate()) fail('CONTEXT_EXPIRED_OR_CHANGED');
    output.trust.sourceBytesVerified = true;
    output.trust.importerBytesVerified = true;
    output.artifacts = { evidenceFile, evidenceSha256: sha256(evidenceBytes), requestPath, requestSha256: sha256(requestBytes) };
    output.command = command;
    return finish('prepared', null);
  } catch (error) {
    const code = error.diagnosticCode ?? error.code;
    return finish('abstain', /^[A-Z][A-Z0-9_]{0,127}$/u.test(code ?? '') ? code : 'INVALID_PREPARATION_INPUT');
  }
}
