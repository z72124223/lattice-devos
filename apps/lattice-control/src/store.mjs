import { createHash, randomUUID } from "node:crypto";
import { mkdirSync } from "node:fs";
import path from "node:path";
import { DatabaseSync } from "node:sqlite";
import { controlStoreSchemaVersion } from "./database-path.mjs";

const priorities = new Set(["low", "normal", "high", "urgent"]);
const statuses = new Set([
  "draft",
  "starting",
  "running",
  "waiting_approval",
  "codex_done",
  "verified",
  "failed",
  "archived",
]);
const workItemChangeFields = new Set([
  "status",
  "codex_thread_id",
  "codex_turn_id",
  "progress",
  "approval_json",
  "final_response",
  "verification_notes",
  "failure_summary",
  "archived_at",
]);
const installationReceiptSchemaVersion = "lattice.control.installation-receipt.v1";
const installationObservationKind = "OBSERVED_AFTER_INSTALL";
const installationReceiptAuthority = "NON_AUTHORITATIVE";
const developmentRadarSchemaVersion = "lattice.control.development-radar.v1";
const developmentRadarObservationKind = "UPSTREAM_DEVELOPMENT_RADAR";
const developmentRadarAuthority = "NON_AUTHORITATIVE";
const developmentRadarActions = new Set([
  "IGNORE",
  "WATCH",
  "WRAP_OFFICIAL",
  "ADOPT_OSS",
  "FREEZE_LATTICE",
]);
const controlSchemaVersion = controlStoreSchemaVersion;
const primaryConversationIdempotentKinds = [
  "codex_disconnected",
  "conversation_message_failed",
  "conversation_turn_dispatch_retry_intended",
  "conversation_turn_dispatch_not_sent",
  "conversation_unbound_claim_restarted",
  "codex_started",
  "conversation_reconnected",
  "conversation_reconnect_failed",
  "codex_thread_started",
  "codex_retry_started",
  "codex_reconciled",
  "conversation_server_response_failed",
  "conversation_terminal_missing_reply",
  "turn_terminal_conflict_ignored",
  "turn_completed",
  "mcp_server_startup_status_updated",
  "conversation_approval_declined",
  "conversation_notification_failed",
];
const primaryConversationIdempotentKindSet = new Set(primaryConversationIdempotentKinds);
const primaryConversationIdempotentKindsSql = primaryConversationIdempotentKinds
  .map((kind) => `'${kind}'`).join(", ");
const conversationEventReadIndexesSql = `
  CREATE INDEX IF NOT EXISTS work_events_work_item_kind_id
  ON work_events(work_item_id, kind, id DESC);

  CREATE INDEX IF NOT EXISTS work_events_client_message_lookup
  ON work_events(
    work_item_id,
    kind,
    json_extract(payload_json, '$.clientMessageId'),
    id DESC
  );

  CREATE INDEX IF NOT EXISTS work_events_thread_turn_lookup
  ON work_events(
    work_item_id,
    kind,
    json_extract(payload_json, '$.threadId'),
    json_extract(payload_json, '$.turnId'),
    id DESC
  );

  CREATE INDEX IF NOT EXISTS work_events_message_lookup
  ON work_events(
    work_item_id,
    kind,
    json_extract(payload_json, '$.messageId'),
    id DESC
  );

  CREATE INDEX IF NOT EXISTS work_events_idempotent_payload_lookup
  ON work_events(work_item_id, kind, payload_json)
  WHERE kind IN (${primaryConversationIdempotentKindsSql})
    AND length(CAST(payload_json AS BLOB)) <= 16384;
`;
const workCoreSchemaSql = `
  ${conversationEventReadIndexesSql}

  CREATE INDEX IF NOT EXISTS work_items_project_id_id
  ON work_items(project_id, id);

  CREATE TABLE IF NOT EXISTS work_item_relations (
    work_item_id TEXT PRIMARY KEY REFERENCES work_items(id) ON DELETE CASCADE,
    parent_work_item_id TEXT REFERENCES work_items(id) ON DELETE RESTRICT,
    blocker_status TEXT NOT NULL CHECK (blocker_status IN ('clear', 'blocked')),
    blocker_reason TEXT,
    CHECK (parent_work_item_id IS NULL OR parent_work_item_id <> work_item_id),
    CHECK (
      (blocker_status = 'clear' AND blocker_reason IS NULL)
      OR (blocker_status = 'blocked' AND length(blocker_reason) BETWEEN 1 AND 2048)
    )
  );

  CREATE INDEX IF NOT EXISTS work_item_relations_parent
  ON work_item_relations(parent_work_item_id);

  CREATE TABLE IF NOT EXISTS work_item_dependencies (
    work_item_id TEXT NOT NULL REFERENCES work_items(id) ON DELETE CASCADE,
    depends_on_work_item_id TEXT NOT NULL REFERENCES work_items(id) ON DELETE RESTRICT,
    created_at TEXT NOT NULL,
    PRIMARY KEY (work_item_id, depends_on_work_item_id),
    CHECK (work_item_id <> depends_on_work_item_id)
  );

  CREATE INDEX IF NOT EXISTS work_item_dependencies_reverse
  ON work_item_dependencies(depends_on_work_item_id);
`;
const decisionCoreSchemaSql = `
  CREATE TABLE IF NOT EXISTS decisions (
    id TEXT PRIMARY KEY,
    scope TEXT NOT NULL CHECK (length(scope) BETWEEN 1 AND 128),
    subject TEXT NOT NULL CHECK (length(subject) BETWEEN 1 AND 256),
    content TEXT NOT NULL CHECK (length(content) BETWEEN 1 AND 4096),
    rationale TEXT NOT NULL CHECK (length(rationale) BETWEEN 1 AND 4096),
    source_kind TEXT NOT NULL
      CHECK (source_kind IN ('user_confirmation', 'approved_document')),
    source_reference TEXT NOT NULL CHECK (length(source_reference) BETWEEN 1 AND 512),
    status TEXT NOT NULL CHECK (status IN ('current', 'superseded')),
    supersedes_decision_id TEXT,
    client_request_id TEXT NOT NULL UNIQUE
      CHECK (length(client_request_id) BETWEEN 1 AND 128),
    request_digest TEXT NOT NULL
      CHECK (length(request_digest) = 64 AND request_digest NOT GLOB '*[^0-9a-f]*'),
    created_at TEXT NOT NULL CHECK (length(created_at) BETWEEN 20 AND 32),
    UNIQUE (scope, subject, id),
    CHECK (supersedes_decision_id IS NULL OR supersedes_decision_id <> id),
    FOREIGN KEY (scope, subject, supersedes_decision_id)
      REFERENCES decisions(scope, subject, id) ON DELETE RESTRICT
  );

  CREATE UNIQUE INDEX IF NOT EXISTS decisions_current_scope_subject
  ON decisions(scope, subject) WHERE status = 'current';

  CREATE UNIQUE INDEX IF NOT EXISTS decisions_unique_successor
  ON decisions(supersedes_decision_id) WHERE supersedes_decision_id IS NOT NULL;

  CREATE INDEX IF NOT EXISTS decisions_scope_created_at
  ON decisions(scope, created_at DESC, id DESC);

  CREATE TRIGGER IF NOT EXISTS decisions_no_delete
  BEFORE DELETE ON decisions
  BEGIN
    SELECT RAISE(ABORT, 'decisions are retained permanently');
  END;

  CREATE TRIGGER IF NOT EXISTS decisions_immutable_update
  BEFORE UPDATE ON decisions
  WHEN
    NEW.id IS NOT OLD.id
    OR NEW.scope IS NOT OLD.scope
    OR NEW.subject IS NOT OLD.subject
    OR NEW.content IS NOT OLD.content
    OR NEW.rationale IS NOT OLD.rationale
    OR NEW.source_kind IS NOT OLD.source_kind
    OR NEW.source_reference IS NOT OLD.source_reference
    OR NEW.supersedes_decision_id IS NOT OLD.supersedes_decision_id
    OR NEW.client_request_id IS NOT OLD.client_request_id
    OR NEW.request_digest IS NOT OLD.request_digest
    OR NEW.created_at IS NOT OLD.created_at
    OR OLD.status <> 'current'
    OR NEW.status <> 'superseded'
  BEGIN
    SELECT RAISE(ABORT, 'decision history is immutable');
  END;

  CREATE TABLE IF NOT EXISTS decision_state (
    slot TEXT PRIMARY KEY CHECK (slot = 'current'),
    schema_version TEXT NOT NULL
      CHECK (schema_version = 'lattice.control.decision-state.v1'),
    revision INTEGER NOT NULL CHECK (revision >= 0),
    digest TEXT NOT NULL
      CHECK (length(digest) = 64 AND digest NOT GLOB '*[^0-9a-f]*'),
    updated_at TEXT NOT NULL CHECK (length(updated_at) BETWEEN 20 AND 32)
  );

  CREATE TRIGGER IF NOT EXISTS decision_state_no_delete
  BEFORE DELETE ON decision_state
  BEGIN
    SELECT RAISE(ABORT, 'decision state cannot be deleted');
  END;

  CREATE TRIGGER IF NOT EXISTS decision_state_revision_guard
  BEFORE UPDATE ON decision_state
  WHEN
    NEW.slot IS NOT OLD.slot
    OR NEW.schema_version IS NOT OLD.schema_version
    OR NEW.revision <> OLD.revision + 1
  BEGIN
    SELECT RAISE(ABORT, 'decision revision must advance exactly once');
  END;
`;
const decisionStateSchemaVersion = "lattice.control.decision-state.v1";
const decisionMutationSchemaVersion = "lattice.control.decision-mutation.v1";
const currentDecisionsPacketSchemaVersion = "lattice.control.current-decisions-packet.v1";
const decisionReadSchemaVersion = "lattice.control.decision-read.v1";
const decisionSearchSchemaVersion = "lattice.control.decision-search.v1";
const decisionSourceKinds = new Set([
  "user_confirmation",
  "approved_document",
]);
const decisionSourcePrefixes = new Map([
  ["user_confirmation", ["thread:"]],
  ["approved_document", ["file:", "document:"]],
]);
const decisionMaximumRows = 10_000;
const decisionMaximumCurrentResults = 32;
const decisionMaximumLineageDepth = 64;
const decisionMaximumSearchResults = 20;
const decisionStoreSource = Object.freeze({
  kind: "CONTROL_SQLITE_DECISIONS",
  authority: "CONTROL_LOCAL_PRODUCT_STATE",
});
const projectCatalogSchemaVersion = "lattice.control.project-catalog.v1";
const projectCatalogRecordKind = "CONTROL_LOCAL_CATALOG";
const legacyProjectRecordKind = "LEGACY_CONTROL_PROJECT";
const observationStatuses = new Set(["complete", "partial", "failed"]);
const safeRemoteProtocols = new Set(["file:", "git:", "http:", "https:", "ssh:"]);
const primaryConversationId = "primary";

export class ProjectRegistrationSupersededError extends Error {
  constructor() {
    super("project registration was superseded by a newer request");
    this.name = "ProjectRegistrationSupersededError";
    this.code = "PROJECT_REGISTRATION_SUPERSEDED";
    this.status = 409;
  }
}

export class ProjectRefreshSupersededError extends Error {
  constructor() {
    super("project refresh was superseded by a newer request");
    this.name = "ProjectRefreshSupersededError";
    this.code = "PROJECT_REFRESH_SUPERSEDED";
    this.status = 409;
  }
}

function now() {
  return new Date().toISOString();
}

function requireText(value, label) {
  if (typeof value !== "string" || !value.trim()) {
    throw new TypeError(`${label} is required`);
  }
  return value.trim();
}

function boundedText(value, label, maximumLength) {
  const text = requireText(value, label);
  if (text.length > maximumLength || /[\u0000-\u001f\u007f-\u009f]/u.test(text)) {
    throw new TypeError(`${label} is too long or contains unsafe control characters`);
  }
  return text;
}

function boundedConversationText(value) {
  if (typeof value !== "string" || !value.trim()) {
    throw new TypeError("conversation message is required");
  }
  const text = value.trim();
  if (
    Buffer.byteLength(text, "utf8") > 16_384
    || /[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f-\u009f]/u.test(text)
  ) {
    throw new TypeError("conversation message is too long or contains unsafe control characters");
  }
  return text;
}

function boundedConversationReply(value) {
  if (typeof value !== "string" || !value.trim()) {
    throw new TypeError("Codex reply is required");
  }
  const text = value.trim();
  if (
    Buffer.byteLength(text, "utf8") > 262_144
    || /[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f-\u009f]/u.test(text)
  ) {
    throw new TypeError("Codex reply is too long or contains unsafe control characters");
  }
  return text;
}

function normalizeClientMessageId(value) {
  const id = requireText(value, "client message ID");
  if (id.length > 128 || !/^[A-Za-z0-9._:-]+$/u.test(id)) {
    throw new TypeError("client message ID must contain 1-128 safe ASCII characters");
  }
  return id;
}

function conversationMessageDigest(text) {
  return createHash("sha256").update(text, "utf8").digest("hex");
}

function normalizeConversationLeaseOwner(value) {
  const owner = requireText(value, "conversation lease owner");
  if (owner.length > 128 || !/^[A-Za-z0-9._:-]+$/u.test(owner)) {
    throw new TypeError("conversation lease owner must contain 1-128 safe ASCII characters");
  }
  return owner;
}

function normalizeConversationLeasePid(value) {
  if (!Number.isSafeInteger(value) || value <= 0) {
    throw new TypeError("conversation lease PID must be a positive safe integer");
  }
  return value;
}

function normalizeConversationLeaseTtl(value) {
  if (!Number.isSafeInteger(value) || value < 3_000 || value > 300_000) {
    throw new TypeError("conversation lease TTL must be between 3000 and 300000 milliseconds");
  }
  return value;
}

function normalizeConversationLeaseGeneration(value) {
  if (!Number.isSafeInteger(value) || value <= 0) {
    throw new TypeError("conversation lease generation must be a positive safe integer");
  }
  return value;
}

function normalizeConversationFence(fence) {
  if (!fence || typeof fence !== "object") {
    throw new TypeError("primary conversation mutation requires a writer fence");
  }
  return {
    ownerId: normalizeConversationLeaseOwner(fence.ownerId),
    generation: normalizeConversationLeaseGeneration(fence.generation),
  };
}

function assertPrimaryConversationFence(database, fence, timestamp = now()) {
  const normalized = normalizeConversationFence(fence);
  const lease = database.prepare(`
    SELECT owner_id, generation, lease_expires_at
    FROM conversation_writer_leases WHERE conversation_id = ?
  `).get(primaryConversationId);
  if (
    lease?.owner_id !== normalized.ownerId
    || lease?.generation !== normalized.generation
    || lease.lease_expires_at <= timestamp
  ) {
    const error = new Error("LATTICE Control 已失去這條對話的寫入權；本次不會繼續送出");
    error.code = "CONVERSATION_WRITER_LOST";
    error.status = 409;
    throw error;
  }
  return lease;
}

export function normalizeProjectDisplayName(value) {
  return boundedText(value, "project name", 256);
}

function optionalBoundedText(value, label, maximumLength) {
  if (value == null) return null;
  return boundedText(value, label, maximumLength);
}

function observationTime(value, label = "observation time") {
  const text = boundedText(value, label, 64);
  const parsed = new Date(text);
  if (!Number.isFinite(parsed.getTime())) throw new TypeError(`${label} must be an ISO timestamp`);
  return parsed.toISOString();
}

function normalizedHttpsUrl(value, label) {
  const text = boundedText(value, label, 2_048);
  let parsed;
  try {
    parsed = new URL(text);
  } catch {
    throw new TypeError(`${label} must be a valid HTTPS URL`);
  }
  if (parsed.protocol !== "https:" || parsed.username || parsed.password) {
    throw new TypeError(`${label} must be a credential-free HTTPS URL`);
  }
  return boundedText(parsed.href, label, 2_048);
}

function normalizedDevelopmentRadar(input) {
  if (!input || typeof input !== "object" || Array.isArray(input)) {
    throw new TypeError("development radar snapshot is required");
  }
  const observedAt = observationTime(input.observed_at, "radar observation time");
  const expiresAt = observationTime(input.expires_at, "radar expiry time");
  if (expiresAt <= observedAt) {
    throw new TypeError("radar expiry time must be later than its observation time");
  }
  if (!Array.isArray(input.decisions) || input.decisions.length > 32) {
    throw new TypeError("radar decisions must be an array of at most 32 entries");
  }
  const decisions = input.decisions.map((decision) => {
    if (!decision || typeof decision !== "object" || Array.isArray(decision)) {
      throw new TypeError("radar decision must be an object");
    }
    const action = boundedText(decision.action, "radar decision action", 32);
    if (!developmentRadarActions.has(action)) {
      throw new TypeError("radar decision action is invalid");
    }
    return {
      action,
      subject: boundedText(decision.subject, "radar decision subject", 256),
      source_url: normalizedHttpsUrl(decision.source_url, "radar decision source URL"),
      version_or_date: optionalBoundedText(
        decision.version_or_date,
        "radar decision version or date",
        128,
      ),
      impact: optionalBoundedText(decision.impact, "radar decision impact", 2_048),
    };
  });
  return {
    observed_at: observedAt,
    expires_at: expiresAt,
    summary: boundedText(input.summary, "radar summary", 4_096),
    decisions,
  };
}

function nullableBoolean(value, label) {
  if (value == null || typeof value === "boolean") return value;
  throw new TypeError(`${label} must be true, false, or null`);
}

function nullableCount(value, label) {
  if (value == null) return null;
  if (!Number.isSafeInteger(value) || value < 0) {
    throw new TypeError(`${label} must be a non-negative safe integer or null`);
  }
  return value;
}

function normalizedAbsolutePath(value, label) {
  const candidate = boundedText(value, label, 32_767);
  if (!path.isAbsolute(candidate)) throw new TypeError(`${label} must be absolute`);
  return path.normalize(candidate);
}

function normalizedRelativePath(value, label) {
  const candidate = boundedText(value, label, 2_048).replaceAll("\\", "/");
  const normalized = path.posix.normalize(candidate);
  if (
    path.posix.isAbsolute(normalized)
    || normalized === ".."
    || normalized.startsWith("../")
  ) {
    throw new TypeError(`${label} must stay within the project root`);
  }
  return normalized;
}

function pathContains(parent, candidate) {
  const relative = path.relative(parent, candidate);
  return relative === "" || (!relative.startsWith("..") && !path.isAbsolute(relative));
}

function validateSanitizedRemoteUrl(value) {
  const url = boundedText(value, "sanitized Git remote URL", 4_096);
  const isWindowsPath = /^[a-zA-Z]:[\\/]/u.test(url) || url.startsWith("\\\\");
  if (isWindowsPath || path.isAbsolute(url) || /^[.]{1,2}[\\/]/u.test(url)) return url;
  if (!url.includes("://")) {
    if (/^[a-zA-Z0-9.-]+:[a-zA-Z0-9._~%+/-]+$/u.test(url)) return url;
    throw new TypeError("sanitized Git remote URL is invalid");
  }
  let parsed;
  try {
    parsed = new URL(url);
  } catch {
    throw new TypeError("sanitized Git remote URL is invalid");
  }
  if (
    !safeRemoteProtocols.has(parsed.protocol)
    || parsed.username
    || parsed.password
    || parsed.search
    || parsed.hash
    || (parsed.protocol !== "file:" && !parsed.hostname)
  ) {
    throw new TypeError("sanitized Git remote URL is invalid");
  }
  return url;
}

function normalizedFailures(value, stage) {
  if (!Array.isArray(value) || value.length > 256) {
    throw new TypeError(`${stage} failures must be a bounded array`);
  }
  return value.map((failure) => {
    if (!failure || typeof failure !== "object" || Array.isArray(failure)) {
      throw new TypeError(`${stage} failure must be an object`);
    }
    const normalized = {
      stage: boundedText(failure.stage ?? stage, `${stage} failure stage`, 64),
      code: boundedText(failure.code, `${stage} failure code`, 128),
      message: boundedText(failure.message, `${stage} failure message`, 512),
    };
    if (failure.relative_path != null) {
      normalized.relative_path = normalizedRelativePath(
        failure.relative_path,
        `${stage} failure relative path`,
      );
    }
    if (failure.remote != null) {
      normalized.remote = boundedText(failure.remote, `${stage} failure remote`, 256);
    }
    if (failure.direction != null) {
      const direction = boundedText(failure.direction, `${stage} failure direction`, 16);
      if (!["fetch", "push"].includes(direction)) {
        throw new TypeError(`${stage} failure direction is invalid`);
      }
      normalized.direction = direction;
    }
    return normalized;
  });
}

function normalizedInspection(value) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new TypeError("project inspection is required");
  }
  const git = value.git;
  const rules = value.rules;
  if (!git || typeof git !== "object" || !rules || typeof rules !== "object") {
    throw new TypeError("project inspection must include Git and rule observations");
  }
  if (!observationStatuses.has(git.status) || !observationStatuses.has(rules.status)) {
    throw new TypeError("project observation status is invalid");
  }
  const gitObservedAt = observationTime(git.observed_at, "Git observation time");
  const rulesObservedAt = observationTime(rules.observed_at, "rule observation time");
  const headSha = optionalBoundedText(git.head_sha, "Git HEAD SHA", 64);
  if (headSha && !/^[a-f0-9]{40,64}$/u.test(headSha)) {
    throw new TypeError("Git HEAD SHA must contain 40 to 64 lowercase hexadecimal characters");
  }
  if (!Array.isArray(git.remotes) || git.remotes.length > 2_048) {
    throw new TypeError("Git remotes must be a bounded array");
  }
  const remotes = git.remotes.map((remote) => {
    if (!remote || typeof remote !== "object") throw new TypeError("Git remote is invalid");
    const direction = boundedText(remote.direction, "Git remote direction", 16);
    if (!["fetch", "push"].includes(direction)) throw new TypeError("Git remote direction is invalid");
    const url = validateSanitizedRemoteUrl(remote.url);
    return {
      name: boundedText(remote.name, "Git remote name", 256),
      direction,
      url,
      credentials_redacted: Boolean(remote.credentials_redacted),
    };
  });
  if (!Array.isArray(rules.documents) || rules.documents.length > 4_096) {
    throw new TypeError("rule documents must be a bounded array");
  }
  const documents = rules.documents.map((document) => {
    if (!document || typeof document !== "object") throw new TypeError("rule document is invalid");
    const sha256 = boundedText(document.sha256, "rule document SHA-256", 64);
    if (!/^[a-f0-9]{64}$/u.test(sha256)) {
      throw new TypeError("rule document SHA-256 must be lowercase hexadecimal");
    }
    return {
      relative_path: normalizedRelativePath(document.relative_path, "rule document path"),
      sha256,
      purpose: boundedText(document.purpose, "rule document purpose", 256),
      observed_at: observationTime(document.observed_at, "rule document observation time"),
    };
  });
  if (
    !Array.isArray(rules.missing_standard_documents)
    || rules.missing_standard_documents.length > 64
  ) {
    throw new TypeError("missing standard rule documents must be a bounded array");
  }
  const canonicalPath = normalizedAbsolutePath(value.canonical_path, "canonical project path");
  const repositoryRoot = value.repo_root == null
      ? null
      : normalizedAbsolutePath(value.repo_root, "repository root");
  if (repositoryRoot && !pathContains(repositoryRoot, canonicalPath)) {
    throw new TypeError("repository root must contain the canonical project path");
  }
  if (git.status === "complete" && git.is_repository === true && !repositoryRoot) {
    throw new TypeError("Git repository observations require a repository root");
  }
  if (git.is_repository !== true && repositoryRoot) {
    throw new TypeError("repository root requires a confirmed Git repository observation");
  }
  return {
    canonical_path: canonicalPath,
    repo_root: repositoryRoot,
    git: {
      status: git.status,
      is_repository: nullableBoolean(git.is_repository, "Git repository state"),
      branch: optionalBoundedText(git.branch, "Git branch", 1_024),
      detached: nullableBoolean(git.detached, "Git detached state"),
      head_sha: headSha,
      dirty: nullableBoolean(git.dirty, "Git dirty state"),
      upstream: optionalBoundedText(git.upstream, "Git upstream", 1_024),
      ahead: nullableCount(git.ahead, "Git ahead count"),
      behind: nullableCount(git.behind, "Git behind count"),
      remotes,
      observed_at: gitObservedAt,
      failures: normalizedFailures(git.failures, "git"),
    },
    rules: {
      status: rules.status,
      documents,
      missing_standard_documents: rules.missing_standard_documents.map((document) => (
        normalizedRelativePath(document, "missing standard rule document")
      )),
      observed_at: rulesObservedAt,
      failures: normalizedFailures(rules.failures, "rules"),
    },
  };
}

function repositoryRootWasObserved(inspection) {
  return inspection.repo_root != null
    || (inspection.git.status === "complete" && inspection.git.is_repository === false);
}

function refreshGeneration(value) {
  if (!Number.isSafeInteger(value) || value < 1) {
    throw new TypeError("project refresh generation must be a positive safe integer");
  }
  return value;
}

function newerRecordedTime(latestObservation, lastFailureAt, incomingTime) {
  return [latestObservation?.git_observed_at, lastFailureAt]
    .some((recordedTime) => recordedTime != null && recordedTime > incomingTime);
}

function decodeNullableBoolean(value) {
  return value == null ? null : value === 1;
}

function parseJsonArray(value) {
  const parsed = JSON.parse(value);
  if (!Array.isArray(parsed)) throw new Error("stored project observation array is invalid");
  return parsed;
}

function decisionError(code, message, status = 409) {
  const error = new Error(message);
  error.code = code;
  error.status = status;
  return error;
}

function exactDecisionObject(value, requiredKeys, optionalKeys = []) {
  if (!value || typeof value !== "object" || Array.isArray(value)) return false;
  const allowed = new Set([...requiredKeys, ...optionalKeys]);
  const keys = Object.keys(value);
  return requiredKeys.every((key) => Object.hasOwn(value, key))
    && keys.every((key) => allowed.has(key));
}

function normalizedDecisionIdentifier(value, label, maximumLength) {
  if (
    typeof value !== "string"
    || value.length < 1
    || value.length > maximumLength
    || !/^[A-Za-z0-9][A-Za-z0-9._:/-]*$/u.test(value)
  ) {
    throw decisionError(
      "DECISION_INPUT_REJECTED",
      `${label} must be a bounded stable identifier`,
      400,
    );
  }
  assertDecisionTextIsSafe(value, label);
  return value;
}

function assertDecisionTextIsSafe(value, label) {
  const secretLikePatterns = [
    /-----BEGIN [A-Z ]*PRIVATE KEY-----/u,
    /\bsk-(?:proj-)?[A-Za-z0-9_-]{12,}\b/u,
    /\bsk_[A-Za-z0-9_-]{12,}\b/u,
    /\b(?:gh[pousr]_[A-Za-z0-9_]{12,}|github_pat_[A-Za-z0-9_]{12,})\b/u,
    /\bAKIA[0-9A-Z]{16}\b/u,
    /\bxox[baprs]-[A-Za-z0-9-]{12,}\b/u,
    /\beyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\b/u,
    /\bBearer\s+[A-Za-z0-9._~-]{12,}\b/iu,
    /\b[A-Za-z][A-Za-z0-9+.-]*:\/\/[^/\s:@]+:[^/\s@]+@/u,
    /(?:^|[^A-Za-z0-9])(?:(?:access|refresh|service|client|session|api)[_-]?)?(?:token|secret|key|password|passwd|pwd)\s*[:=]\s*\S+/iu,
    /(?:^|[^A-Za-z0-9])(?:otp|one[- ]time[- ]password)(?:\s*[:=]\s*|\s+)[A-Za-z0-9-]{4,}/iu,
    /(?:^|[\s,;])(?:[A-Z][A-Z0-9_]{2,}|[A-Za-z][A-Za-z0-9]*_[A-Za-z0-9_]+)\s*=\s*[^\s]+/mu,
    /(?:^|\s)(?:user|assistant|system|developer)\s*:[\s\S]*?(?:^|\s)(?:user|assistant|system|developer)\s*:/iu,
    /(?:hidden reasoning|chain[- ]of[- ]thought|<analysis>)/iu,
    /\b(?:model|assistant)?\s*internal\s+(?:reasoning|deliberation|thoughts?)\b/iu,
  ];
  if (secretLikePatterns.some((pattern) => pattern.test(value))) {
    throw decisionError(
      "DECISION_SENSITIVE_CONTENT_REJECTED",
      `${label} looks like secret, environment, or hidden-reasoning material`,
      400,
    );
  }
}

function normalizedDecisionText(value, label, maximumBytes) {
  if (typeof value !== "string" || !value.trim()) {
    throw decisionError("DECISION_INPUT_REJECTED", `${label} is required`, 400);
  }
  const text = value.trim();
  if (
    Buffer.byteLength(text, "utf8") > maximumBytes
    || /[\u0000-\u001f\u007f-\u009f]/u.test(text)
  ) {
    throw decisionError(
      "DECISION_INPUT_REJECTED",
      `${label} is too long or contains unsafe control characters`,
      400,
    );
  }
  assertDecisionTextIsSafe(text, label);
  return text;
}

function normalizedDecisionSource(value) {
  if (!exactDecisionObject(value, ["kind", "reference"])) {
    throw decisionError(
      "DECISION_SOURCE_REJECTED",
      "decision source must contain only kind and reference",
      400,
    );
  }
  if (!decisionSourceKinds.has(value.kind)) {
    throw decisionError("DECISION_SOURCE_REJECTED", "decision source kind is invalid", 400);
  }
  if (
    typeof value.reference !== "string"
    || value.reference.length < 1
    || value.reference.length > 512
    || !/^[A-Za-z0-9][A-Za-z0-9._:/#@+-]*$/u.test(value.reference)
  ) {
    throw decisionError(
      "DECISION_SOURCE_REJECTED",
      "decision source reference must be a bounded credential-free reference",
      400,
    );
  }
  const preciseReference = value.kind === "user_confirmation"
    ? /^thread:[A-Za-z0-9][A-Za-z0-9._-]{0,127}\/(?:turn|delegation):[A-Za-z0-9][A-Za-z0-9._:-]{0,127}(?:#[A-Za-z0-9][A-Za-z0-9._:-]{0,127})?$/u.test(value.reference)
    : /^(?:file:[A-Za-z0-9][A-Za-z0-9._/-]{0,383}|document:[A-Za-z0-9][A-Za-z0-9._:/-]{0,383})#[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$/u.test(value.reference);
  if (
    !decisionSourcePrefixes.get(value.kind).some((prefix) => value.reference.startsWith(prefix))
    || !preciseReference
  ) {
    throw decisionError(
      "DECISION_SOURCE_REJECTED",
      "decision source reference does not identify an explicit confirmation or approved document",
      400,
    );
  }
  assertDecisionTextIsSafe(value.reference, "decision source reference");
  return { kind: value.kind, reference: value.reference };
}

function normalizedDecisionRequestId(value) {
  if (
    typeof value !== "string"
    || value.length < 1
    || value.length > 128
    || !/^[A-Za-z0-9._:-]+$/u.test(value)
  ) {
    throw decisionError(
      "DECISION_INPUT_REJECTED",
      "client request ID must contain 1-128 safe ASCII characters",
      400,
    );
  }
  assertDecisionTextIsSafe(value, "client request ID");
  return value;
}

function normalizedDecisionRevision(value) {
  if (!Number.isSafeInteger(value) || value < 0) {
    throw decisionError(
      "DECISION_INPUT_REJECTED",
      "decision revision must be a non-negative safe integer",
      400,
    );
  }
  return value;
}

function normalizedDecisionDigest(value, label = "decision digest") {
  if (typeof value !== "string" || !/^[a-f0-9]{64}$/u.test(value)) {
    throw decisionError("DECISION_INPUT_REJECTED", `${label} must be a lowercase SHA-256 digest`, 400);
  }
  return value;
}

function normalizedDecisionRecordInput(input) {
  if (!exactDecisionObject(
    input,
    [
      "scope",
      "subject",
      "content",
      "rationale",
      "source",
      "clientRequestId",
      "expectedRevision",
      "expectedDigest",
    ],
    ["supersedesDecisionId"],
  )) {
    throw decisionError(
      "DECISION_INPUT_REJECTED",
      "decision record input has missing or unsupported fields",
      400,
    );
  }
  return {
    scope: normalizedDecisionIdentifier(input.scope, "decision scope", 128),
    subject: normalizedDecisionIdentifier(input.subject, "decision subject", 256),
    content: normalizedDecisionText(input.content, "decision content", 4_096),
    rationale: normalizedDecisionText(input.rationale, "decision rationale", 4_096),
    source: normalizedDecisionSource(input.source),
    clientRequestId: normalizedDecisionRequestId(input.clientRequestId),
    expectedRevision: normalizedDecisionRevision(input.expectedRevision),
    expectedDigest: normalizedDecisionDigest(input.expectedDigest),
    supersedesDecisionId: input.supersedesDecisionId == null
      ? null
      : normalizedDecisionIdentifier(input.supersedesDecisionId, "superseded decision ID", 128),
  };
}

function normalizedCurrentDecisionQuery(input) {
  if (!exactDecisionObject(input, ["scope", "limit"], ["subject"])) {
    throw decisionError(
      "DECISION_QUERY_REJECTED",
      "current decision query requires bounded scope and limit",
      400,
    );
  }
  if (
    !Number.isSafeInteger(input.limit)
    || input.limit < 1
    || input.limit > decisionMaximumCurrentResults
  ) {
    throw decisionError(
      "DECISION_QUERY_REJECTED",
      `current decision limit must be between 1 and ${decisionMaximumCurrentResults}`,
      400,
    );
  }
  return {
    scope: normalizedDecisionIdentifier(input.scope, "decision scope", 128),
    subject: input.subject == null
      ? null
      : normalizedDecisionIdentifier(input.subject, "decision subject", 256),
    limit: input.limit,
  };
}

function normalizedDecisionReadQuery(input) {
  if (!exactDecisionObject(
    input,
    ["decisionId", "maxDepth", "expectedRevision", "expectedDigest"],
  )) {
    throw decisionError(
      "DECISION_QUERY_REJECTED",
      "decision read requires identity, bounded depth, revision, and digest",
      400,
    );
  }
  if (
    !Number.isSafeInteger(input.maxDepth)
    || input.maxDepth < 1
    || input.maxDepth > decisionMaximumLineageDepth
  ) {
    throw decisionError(
      "DECISION_QUERY_REJECTED",
      `decision lineage depth must be between 1 and ${decisionMaximumLineageDepth}`,
      400,
    );
  }
  return {
    decisionId: normalizedDecisionIdentifier(input.decisionId, "decision ID", 128),
    maxDepth: input.maxDepth,
    expectedRevision: normalizedDecisionRevision(input.expectedRevision),
    expectedDigest: normalizedDecisionDigest(input.expectedDigest),
  };
}

function normalizedDecisionSearchQuery(input) {
  if (!exactDecisionObject(
    input,
    ["scope", "query", "limit", "expectedRevision", "expectedDigest"],
  )) {
    throw decisionError(
      "DECISION_QUERY_REJECTED",
      "decision search requires bounded scope, query, limit, revision, and digest",
      400,
    );
  }
  if (
    typeof input.query !== "string"
    || !input.query.trim()
    || Buffer.byteLength(input.query.trim(), "utf8") > 128
    || /[\u0000-\u001f\u007f-\u009f]/u.test(input.query)
  ) {
    throw decisionError(
      "DECISION_QUERY_REJECTED",
      "decision search query must contain 1-128 bounded UTF-8 bytes",
      400,
    );
  }
  if (
    !Number.isSafeInteger(input.limit)
    || input.limit < 1
    || input.limit > decisionMaximumSearchResults
  ) {
    throw decisionError(
      "DECISION_QUERY_REJECTED",
      `decision search limit must be between 1 and ${decisionMaximumSearchResults}`,
      400,
    );
  }
  return {
    scope: normalizedDecisionIdentifier(input.scope, "decision scope", 128),
    query: input.query.trim(),
    limit: input.limit,
    expectedRevision: normalizedDecisionRevision(input.expectedRevision),
    expectedDigest: normalizedDecisionDigest(input.expectedDigest),
  };
}

function decisionRequestDigest(input) {
  return createHash("sha256").update(JSON.stringify({
    operation: input.supersedesDecisionId == null ? "record" : "supersede",
    scope: input.scope,
    subject: input.subject,
    content: input.content,
    rationale: input.rationale,
    source: input.source,
    supersedes_decision_id: input.supersedesDecisionId,
    expected_revision: input.expectedRevision,
    expected_digest: input.expectedDigest,
  }), "utf8").digest("hex");
}

function normalizedDecisionTimestamp(value, label) {
  if (typeof value !== "string" || value.length > 32) {
    throw decisionError("DECISION_STATE_CORRUPT", `${label} is invalid`);
  }
  const parsed = new Date(value);
  if (
    !Number.isFinite(parsed.getTime())
    || parsed.getUTCFullYear() < 2000
    || parsed.toISOString() !== value
    || parsed.getTime() > Date.now() + 300_000
  ) {
    throw decisionError("DECISION_STATE_CORRUPT", `${label} is invalid`);
  }
  return value;
}

function decisionRows(database) {
  return database.prepare(`
    SELECT
      id, scope, subject, content, rationale, source_kind, source_reference,
      status, supersedes_decision_id, client_request_id, request_digest, created_at
    FROM decisions
    ORDER BY scope ASC, subject ASC, created_at ASC, id ASC
    LIMIT ?
  `).all(decisionMaximumRows + 1);
}

function decisionRowsDigest(rows) {
  return createHash("sha256").update(JSON.stringify(rows.map((row) => [
    row.id,
    row.scope,
    row.subject,
    row.content,
    row.rationale,
    row.source_kind,
    row.source_reference,
    row.status,
    row.supersedes_decision_id,
    row.client_request_id,
    row.request_digest,
    row.created_at,
  ])), "utf8").digest("hex");
}

function decodeDecision(row) {
  if (!row) return null;
  return {
    id: row.id,
    scope: row.scope,
    subject: row.subject,
    content: row.content,
    rationale: row.rationale,
    source: { kind: row.source_kind, reference: row.source_reference },
    status: row.status,
    supersedes_decision_id: row.supersedes_decision_id,
    created_at: row.created_at,
  };
}

function initializeDecisionState(database) {
  const rows = decisionRows(database);
  if (rows.length > decisionMaximumRows) {
    throw decisionError("DECISION_STORE_LIMIT_EXCEEDED", "decision store row limit exceeded");
  }
  database.prepare(`
    INSERT OR IGNORE INTO decision_state (
      slot, schema_version, revision, digest, updated_at
    ) VALUES ('current', ?, ?, ?, ?)
  `).run(decisionStateSchemaVersion, rows.length, decisionRowsDigest(rows), now());
  return assertDecisionStateIntegrity(database);
}

function decisionStateMetadata(database) {
  const states = database.prepare(`
    SELECT slot, schema_version, revision, digest, updated_at
    FROM decision_state ORDER BY slot
  `).all();
  if (states.length !== 1) {
    throw decisionError("DECISION_STATE_CORRUPT", "decision state singleton is missing or duplicated");
  }
  const state = states[0];
  if (
    state.slot !== "current"
    || state.schema_version !== decisionStateSchemaVersion
    || !Number.isSafeInteger(state.revision)
    || state.revision < 0
    || !/^[a-f0-9]{64}$/u.test(state.digest)
  ) {
    throw decisionError("DECISION_STATE_CORRUPT", "decision state metadata is invalid");
  }
  normalizedDecisionTimestamp(state.updated_at, "decision state timestamp");
  return state;
}

function assertDecisionStateIntegrity(database) {
  const state = decisionStateMetadata(database);
  const rows = decisionRows(database);
  if (rows.length > decisionMaximumRows) {
    throw decisionError("DECISION_STORE_LIMIT_EXCEEDED", "decision store row limit exceeded");
  }
  if (state.revision !== rows.length) {
    throw decisionError(
      "DECISION_STATE_REVISION_MISMATCH",
      "decision revision does not match retained history",
    );
  }
  const digest = decisionRowsDigest(rows);
  if (state.digest !== digest) {
    throw decisionError(
      "DECISION_STATE_DIGEST_MISMATCH",
      "decision digest does not match retained history",
    );
  }

  const byId = new Map();
  const groups = new Map();
  const childByParent = new Map();
  for (const row of rows) {
    normalizedDecisionIdentifier(row.id, "stored decision ID", 128);
    normalizedDecisionIdentifier(row.scope, "stored decision scope", 128);
    normalizedDecisionIdentifier(row.subject, "stored decision subject", 256);
    normalizedDecisionText(row.content, "stored decision content", 4_096);
    normalizedDecisionText(row.rationale, "stored decision rationale", 4_096);
    normalizedDecisionSource({ kind: row.source_kind, reference: row.source_reference });
    normalizedDecisionRequestId(row.client_request_id);
    normalizedDecisionDigest(row.request_digest, "stored request digest");
    normalizedDecisionTimestamp(row.created_at, "stored decision timestamp");
    if (!new Set(["current", "superseded"]).has(row.status)) {
      throw decisionError("DECISION_STATE_CORRUPT", "stored decision status is invalid");
    }
    if (byId.has(row.id)) {
      throw decisionError("DECISION_STATE_CORRUPT", "duplicate stored decision identity");
    }
    byId.set(row.id, row);
    const key = `${row.scope}\u0000${row.subject}`;
    const group = groups.get(key) ?? [];
    group.push(row);
    groups.set(key, group);
    if (row.supersedes_decision_id != null) {
      if (childByParent.has(row.supersedes_decision_id)) {
        throw decisionError("DECISION_LINEAGE_BRANCH_REJECTED", "decision lineage branches");
      }
      childByParent.set(row.supersedes_decision_id, row);
    }
  }
  for (const group of groups.values()) {
    const current = group.filter(({ status }) => status === "current");
    if (current.length !== 1) {
      throw decisionError(
        "DECISION_CURRENT_INVARIANT_VIOLATED",
        "each decision scope and subject must have exactly one current decision",
      );
    }
    for (const row of group) {
      const child = childByParent.get(row.id) ?? null;
      if ((row.status === "current") !== (child === null)) {
        throw decisionError(
          "DECISION_LINEAGE_STATUS_MISMATCH",
          "decision current and superseded status does not match lineage",
        );
      }
      if (row.supersedes_decision_id != null) {
        const parent = byId.get(row.supersedes_decision_id);
        if (!parent) {
          throw decisionError("DECISION_LINEAGE_DANGLING", "decision lineage target is missing");
        }
        if (parent.scope !== row.scope || parent.subject !== row.subject) {
          throw decisionError("DECISION_CROSS_SCOPE_SUPERSESSION_REJECTED", "decision lineage crosses scope or subject");
        }
        if (row.created_at < parent.created_at) {
          throw decisionError("DECISION_LINEAGE_TIME_INVALID", "decision lineage time moves backwards");
        }
      }
    }
    const visited = new Set();
    let cursor = current[0];
    while (cursor) {
      if (visited.has(cursor.id)) {
        throw decisionError("DECISION_LINEAGE_CYCLE_REJECTED", "decision lineage contains a cycle");
      }
      visited.add(cursor.id);
      cursor = cursor.supersedes_decision_id == null
        ? null
        : byId.get(cursor.supersedes_decision_id);
    }
    if (visited.size !== group.length) {
      throw decisionError("DECISION_LINEAGE_DISCONNECTED", "decision lineage is disconnected");
    }
  }
  return { state, rows, byId, childByParent };
}

function assertExpectedDecisionState(state, expectedRevision, expectedDigest) {
  if (state.revision !== expectedRevision || state.digest !== expectedDigest) {
    throw decisionError(
      "DECISION_REVISION_MISMATCH",
      "decision state changed before the requested operation",
    );
  }
}

function advanceDecisionState(database, previousState, timestamp) {
  const rows = decisionRows(database);
  if (rows.length > decisionMaximumRows) {
    throw decisionError("DECISION_STORE_LIMIT_EXCEEDED", "decision store row limit exceeded");
  }
  const result = database.prepare(`
    UPDATE decision_state
    SET revision = ?, digest = ?, updated_at = ?
    WHERE slot = 'current' AND revision = ? AND digest = ?
  `).run(
    previousState.revision + 1,
    decisionRowsDigest(rows),
    timestamp,
    previousState.revision,
    previousState.digest,
  );
  if (result.changes !== 1) {
    throw decisionError("DECISION_REVISION_MISMATCH", "decision state changed during mutation");
  }
  return assertDecisionStateIntegrity(database);
}

const schemaColumns = new Map([
  ["projects", ["id", "name", "root_path", "created_at", "updated_at"]],
  ["work_items", [
    "id", "project_id", "title", "objective", "priority", "status", "codex_thread_id",
    "codex_turn_id", "progress", "approval_json", "final_response", "verification_notes",
    "failure_summary", "archived_at", "created_at", "updated_at",
  ]],
  ["work_events", ["id", "work_item_id", "kind", "payload_json", "created_at"]],
  ["work_item_relations", [
    "work_item_id", "parent_work_item_id", "blocker_status", "blocker_reason",
  ]],
  ["work_item_dependencies", [
    "work_item_id", "depends_on_work_item_id", "created_at",
  ]],
  ["conversation_writer_leases", [
    "conversation_id", "owner_id", "owner_pid", "lease_expires_at", "updated_at", "generation",
  ]],
  ["installation_receipts", [
    "id", "schema_version", "observation_kind", "authority", "project_id", "component",
    "source_commit_sha", "artifact_path", "artifact_sha256", "receipt_digest", "recorded_at",
  ]],
  ["development_radar", [
    "slot", "schema_version", "observation_kind", "authority", "observed_at",
    "expires_at", "summary", "decisions_json", "updated_at",
  ]],
  ["project_registration_details", [
    "project_id", "canonical_path", "repo_root_path", "repo_root_observed_at", "registered_at",
    "refreshed_at", "refresh_generation", "last_refresh_failure_code",
    "last_refresh_failure_message", "last_refresh_failed_at",
  ]],
  ["project_registration_claims", ["canonical_path", "generation", "claimed_at"]],
  ["project_observations", [
    "id", "project_id", "git_status", "is_git_repository", "branch", "detached", "head_sha",
    "dirty", "upstream", "ahead", "behind", "git_observed_at", "git_failures_json",
    "rule_status", "rules_observed_at", "missing_rules_json", "rule_failures_json",
  ]],
  ["project_git_remotes", [
    "observation_id", "name", "direction", "url_sanitized", "credentials_redacted",
  ]],
  ["project_rule_documents", [
    "observation_id", "relative_path", "sha256", "purpose", "observed_at",
  ]],
  ["decisions", [
    "id", "scope", "subject", "content", "rationale", "source_kind", "source_reference",
    "status", "supersedes_decision_id", "client_request_id", "request_digest", "created_at",
  ]],
  ["decision_state", ["slot", "schema_version", "revision", "digest", "updated_at"]],
]);

const schemaForeignKeys = new Map([
  ["work_items", [["project_id", "projects", "id", "CASCADE"]]],
  ["work_events", [["work_item_id", "work_items", "id", "CASCADE"]]],
  ["work_item_relations", [
    ["parent_work_item_id", "work_items", "id", "RESTRICT"],
    ["work_item_id", "work_items", "id", "CASCADE"],
  ]],
  ["work_item_dependencies", [
    ["depends_on_work_item_id", "work_items", "id", "RESTRICT"],
    ["work_item_id", "work_items", "id", "CASCADE"],
  ]],
  ["conversation_writer_leases", [["conversation_id", "work_items", "id", "CASCADE"]]],
  ["installation_receipts", [["project_id", "projects", "id", "NO ACTION"]]],
  ["project_registration_details", [["project_id", "projects", "id", "CASCADE"]]],
  ["project_observations", [["project_id", "projects", "id", "CASCADE"]]],
  ["project_git_remotes", [["observation_id", "project_observations", "id", "CASCADE"]]],
  ["project_rule_documents", [["observation_id", "project_observations", "id", "CASCADE"]]],
  ["decisions", [
    ["scope", "decisions", "scope", "RESTRICT"],
    ["subject", "decisions", "subject", "RESTRICT"],
    ["supersedes_decision_id", "decisions", "id", "RESTRICT"],
  ]],
]);

function quotedIdentifier(value) {
  return `"${value.replaceAll('"', '""')}"`;
}

function schemaProfileFailure(detail) {
  throw new Error(`Control database schema profile mismatch: ${detail}`);
}

function validateLegacyWorkCoreMigrationBase(database) {
  const requiredTables = ["projects", "work_events", "work_items"];
  const actualTables = database.prepare(`
    SELECT name FROM sqlite_master
    WHERE type = 'table' AND name IN (?, ?, ?)
    ORDER BY name
  `).all(...requiredTables).map(({ name }) => name);
  if (JSON.stringify(actualTables) !== JSON.stringify(requiredTables)) {
    schemaProfileFailure("legacy base tables");
  }
  const workItemColumns = new Set(database.prepare("PRAGMA table_info(work_items)")
    .all().map(({ name }) => name));
  if (!workItemColumns.has("id") || !workItemColumns.has("project_id")) {
    schemaProfileFailure("legacy work_items columns");
  }
}

const decisionCoreOwnedObjectNames = new Set([
  "decisions",
  "decision_state",
  "decisions_current_scope_subject",
  "decisions_unique_successor",
  "decisions_scope_created_at",
  "decisions_no_delete",
  "decisions_immutable_update",
  "decision_state_no_delete",
  "decision_state_revision_guard",
]);

function validateLegacyDecisionCoreAbsent(database) {
  const collisions = database.prepare(`
    SELECT name FROM sqlite_master
    WHERE name NOT LIKE 'sqlite_%'
    ORDER BY name
  `).all()
    .map(({ name }) => name)
    .filter((name) => decisionCoreOwnedObjectNames.has(name));
  if (collisions.length > 0) {
    schemaProfileFailure(`legacy decision core objects: ${collisions.join(", ")}`);
  }
}

function normalizeSchemaSql(value) {
  const quoted = [];
  const protectedSql = value.replace(
    /'(?:''|[^'])*'|"(?:""|[^"])*"|`(?:``|[^`])*`|\[[^\]]*\]/gu,
    (literal) => {
      const marker = `\u0001${quoted.length}\u0002`;
      quoted.push(literal);
      return marker;
    },
  );
  const normalized = protectedSql
    .replace(/\bIF\s+NOT\s+EXISTS\b/giu, "")
    .replace(/\s+/gu, " ")
    .replace(/\s*([(),;])\s*/gu, "$1")
    .trim()
    .toLowerCase();
  return normalized.replace(/\u0001(\d+)\u0002/gu, (_marker, index) => quoted[Number(index)]);
}

function controlSchemaManifest(database) {
  return database.prepare(`
    SELECT type, name, sql
    FROM sqlite_master
    WHERE type IN ('table', 'index', 'trigger')
      AND name NOT LIKE 'sqlite_%'
      AND sql IS NOT NULL
    ORDER BY type, name
  `).all().map((entry) => ({
    type: entry.type,
    name: entry.name,
    sql_sha256: createHash("sha256").update(normalizeSchemaSql(entry.sql)).digest("hex"),
  }));
}

let expectedSchemaManifest = null;

function referenceControlSchemaManifest() {
  if (expectedSchemaManifest) return expectedSchemaManifest;
  const reference = new DatabaseSync(":memory:");
  try {
    initializeControlDatabase(reference, { validateProfile: false });
    expectedSchemaManifest = controlSchemaManifest(reference);
    return expectedSchemaManifest;
  } finally {
    reference.close();
  }
}

function validateControlSchemaProfile(database) {
  if (JSON.stringify(controlSchemaManifest(database)) !== JSON.stringify(referenceControlSchemaManifest())) {
    schemaProfileFailure("exact SQL manifest");
  }
  for (const [table, expectedColumns] of schemaColumns) {
    const actualColumns = database.prepare(
      `PRAGMA table_info(${quotedIdentifier(table)})`,
    ).all().map((column) => column.name);
    if (JSON.stringify(actualColumns) !== JSON.stringify(expectedColumns)) {
      schemaProfileFailure(`table ${table} columns`);
    }
  }
  for (const [table, expectedForeignKeys] of schemaForeignKeys) {
    const actualForeignKeys = database.prepare(
      `PRAGMA foreign_key_list(${quotedIdentifier(table)})`,
    ).all().map((foreignKey) => [
      foreignKey.from,
      foreignKey.table,
      foreignKey.to,
      foreignKey.on_delete,
    ]).sort();
    if (JSON.stringify(actualForeignKeys) !== JSON.stringify([...expectedForeignKeys].sort())) {
      schemaProfileFailure(`table ${table} foreign keys`);
    }
  }
  const observationIndex = database.prepare(
    "PRAGMA index_info(project_observations_project_id)",
  ).all().map((column) => column.name);
  if (JSON.stringify(observationIndex) !== JSON.stringify(["project_id"])) {
    schemaProfileFailure("project observation index");
  }
  const registrationIndexes = database.prepare(
    "PRAGMA index_list(project_registration_details)",
  ).all();
  const uniqueCanonicalPath = registrationIndexes.some((index) => (
    index.unique === 1
    && JSON.stringify(database.prepare(
      `PRAGMA index_info(${quotedIdentifier(index.name)})`,
    ).all().map((column) => column.name)) === JSON.stringify(["canonical_path"])
  ));
  if (!uniqueCanonicalPath) schemaProfileFailure("canonical project path uniqueness");
  const triggers = database.prepare(`
    SELECT name FROM sqlite_master
    WHERE type = 'trigger' AND name IN (?, ?)
    ORDER BY name
  `).all("installation_receipts_no_delete", "installation_receipts_no_update")
    .map((trigger) => trigger.name);
  if (JSON.stringify(triggers) !== JSON.stringify([
    "installation_receipts_no_delete",
    "installation_receipts_no_update",
  ])) {
    schemaProfileFailure("installation receipt append-only triggers");
  }
  const foreignKeyFailures = database.prepare("PRAGMA foreign_key_check").all();
  if (foreignKeyFailures.length > 0) schemaProfileFailure("foreign key integrity");
  const integrity = database.prepare("PRAGMA quick_check").all();
  if (integrity.length !== 1 || integrity[0].quick_check !== "ok") {
    schemaProfileFailure("SQLite quick check");
  }
}

function initializeControlDatabase(database, { validateProfile = true } = {}) {
  database.exec("BEGIN IMMEDIATE;");
  try {
    validateLegacyDecisionCoreAbsent(database);
    database.exec(`
      CREATE TABLE IF NOT EXISTS projects (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        root_path TEXT NOT NULL,
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL
      );

      CREATE TABLE IF NOT EXISTS work_items (
        id TEXT PRIMARY KEY,
        project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
        title TEXT NOT NULL,
        objective TEXT NOT NULL,
        priority TEXT NOT NULL,
        status TEXT NOT NULL,
        codex_thread_id TEXT,
        codex_turn_id TEXT,
        progress TEXT,
        approval_json TEXT,
        final_response TEXT,
        verification_notes TEXT,
        failure_summary TEXT,
        archived_at TEXT,
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL
      );

      CREATE TABLE IF NOT EXISTS work_events (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        work_item_id TEXT NOT NULL REFERENCES work_items(id) ON DELETE CASCADE,
        kind TEXT NOT NULL,
        payload_json TEXT NOT NULL,
        created_at TEXT NOT NULL
      );

      CREATE TABLE IF NOT EXISTS conversation_writer_leases (
        conversation_id TEXT PRIMARY KEY REFERENCES work_items(id) ON DELETE CASCADE
          CHECK (conversation_id = 'primary'),
        owner_id TEXT NOT NULL CHECK (length(owner_id) BETWEEN 1 AND 128),
        owner_pid INTEGER NOT NULL CHECK (owner_pid > 0),
        lease_expires_at TEXT NOT NULL,
        updated_at TEXT NOT NULL,
        generation INTEGER NOT NULL CHECK (generation > 0)
      );

      CREATE TABLE IF NOT EXISTS installation_receipts (
        id TEXT PRIMARY KEY,
        schema_version TEXT NOT NULL
          CHECK (schema_version = 'lattice.control.installation-receipt.v1'),
        observation_kind TEXT NOT NULL
          CHECK (observation_kind = 'OBSERVED_AFTER_INSTALL'),
        authority TEXT NOT NULL
          CHECK (authority = 'NON_AUTHORITATIVE'),
        project_id TEXT NOT NULL REFERENCES projects(id),
        component TEXT NOT NULL
          CHECK (length(component) BETWEEN 1 AND 64),
        source_commit_sha TEXT NOT NULL
          CHECK (length(source_commit_sha) = 40 AND source_commit_sha NOT GLOB '*[^0-9a-f]*'),
        artifact_path TEXT NOT NULL
          CHECK (length(artifact_path) BETWEEN 1 AND 2048),
        artifact_sha256 TEXT NOT NULL
          CHECK (length(artifact_sha256) = 64 AND artifact_sha256 NOT GLOB '*[^0-9a-f]*'),
        receipt_digest TEXT NOT NULL UNIQUE
          CHECK (length(receipt_digest) = 64 AND receipt_digest NOT GLOB '*[^0-9a-f]*'),
        recorded_at TEXT NOT NULL
      );

      CREATE TRIGGER IF NOT EXISTS installation_receipts_no_update
      BEFORE UPDATE ON installation_receipts
      BEGIN
        SELECT RAISE(ABORT, 'installation receipts are append-only');
      END;

      CREATE TRIGGER IF NOT EXISTS installation_receipts_no_delete
      BEFORE DELETE ON installation_receipts
      BEGIN
        SELECT RAISE(ABORT, 'installation receipts are append-only');
      END;

      CREATE TABLE IF NOT EXISTS development_radar (
        slot TEXT PRIMARY KEY CHECK (slot = 'current'),
        schema_version TEXT NOT NULL
          CHECK (schema_version = 'lattice.control.development-radar.v1'),
        observation_kind TEXT NOT NULL
          CHECK (observation_kind = 'UPSTREAM_DEVELOPMENT_RADAR'),
        authority TEXT NOT NULL
          CHECK (authority = 'NON_AUTHORITATIVE'),
        observed_at TEXT NOT NULL,
        expires_at TEXT NOT NULL,
        summary TEXT NOT NULL CHECK (length(summary) BETWEEN 1 AND 4096),
        decisions_json TEXT NOT NULL CHECK (length(decisions_json) BETWEEN 2 AND 262144),
        updated_at TEXT NOT NULL
      );

      CREATE TABLE IF NOT EXISTS project_registration_details (
        project_id TEXT PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
        canonical_path TEXT NOT NULL COLLATE NOCASE UNIQUE,
        repo_root_path TEXT,
        repo_root_observed_at TEXT,
        registered_at TEXT NOT NULL,
        refreshed_at TEXT NOT NULL,
        refresh_generation INTEGER NOT NULL DEFAULT 0 CHECK (refresh_generation >= 0),
        last_refresh_failure_code TEXT,
        last_refresh_failure_message TEXT,
        last_refresh_failed_at TEXT
      );

      CREATE TABLE IF NOT EXISTS project_registration_claims (
        canonical_path TEXT PRIMARY KEY COLLATE NOCASE,
        generation INTEGER NOT NULL CHECK (generation >= 0),
        claimed_at TEXT NOT NULL
      );

      CREATE TABLE IF NOT EXISTS project_observations (
        id TEXT PRIMARY KEY,
        project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
        git_status TEXT NOT NULL CHECK (git_status IN ('complete', 'partial', 'failed')),
        is_git_repository INTEGER CHECK (is_git_repository IS NULL OR is_git_repository IN (0, 1)),
        branch TEXT,
        detached INTEGER CHECK (detached IS NULL OR detached IN (0, 1)),
        head_sha TEXT,
        dirty INTEGER CHECK (dirty IS NULL OR dirty IN (0, 1)),
        upstream TEXT,
        ahead INTEGER CHECK (ahead IS NULL OR ahead >= 0),
        behind INTEGER CHECK (behind IS NULL OR behind >= 0),
        git_observed_at TEXT NOT NULL,
        git_failures_json TEXT NOT NULL,
        rule_status TEXT NOT NULL CHECK (rule_status IN ('complete', 'partial', 'failed')),
        rules_observed_at TEXT NOT NULL,
        missing_rules_json TEXT NOT NULL,
        rule_failures_json TEXT NOT NULL
      );

      CREATE INDEX IF NOT EXISTS project_observations_project_id
      ON project_observations(project_id);

      CREATE TABLE IF NOT EXISTS project_git_remotes (
        observation_id TEXT NOT NULL REFERENCES project_observations(id) ON DELETE CASCADE,
        name TEXT NOT NULL,
        direction TEXT NOT NULL CHECK (direction IN ('fetch', 'push')),
        url_sanitized TEXT NOT NULL,
        credentials_redacted INTEGER NOT NULL CHECK (credentials_redacted IN (0, 1)),
        PRIMARY KEY (observation_id, name, direction, url_sanitized)
      );

      CREATE TABLE IF NOT EXISTS project_rule_documents (
        observation_id TEXT NOT NULL REFERENCES project_observations(id) ON DELETE CASCADE,
        relative_path TEXT NOT NULL,
        sha256 TEXT NOT NULL,
        purpose TEXT NOT NULL,
        observed_at TEXT NOT NULL,
        PRIMARY KEY (observation_id, relative_path)
      );

    `);
    database.exec(workCoreSchemaSql);
    database.exec(decisionCoreSchemaSql);
    initializeDecisionState(database);
    if (validateProfile) validateControlSchemaProfile(database);
    database.exec(`PRAGMA user_version = ${controlSchemaVersion};`);
    database.exec("COMMIT;");
  } catch (error) {
    database.exec("ROLLBACK;");
    throw error;
  }
}

function migrateControlDatabaseV1ToV7(database) {
  database.exec("BEGIN IMMEDIATE;");
  try {
    validateLegacyWorkCoreMigrationBase(database);
    validateLegacyDecisionCoreAbsent(database);
    database.exec(`
      CREATE TABLE development_radar (
        slot TEXT PRIMARY KEY CHECK (slot = 'current'),
        schema_version TEXT NOT NULL
          CHECK (schema_version = 'lattice.control.development-radar.v1'),
        observation_kind TEXT NOT NULL
          CHECK (observation_kind = 'UPSTREAM_DEVELOPMENT_RADAR'),
        authority TEXT NOT NULL
          CHECK (authority = 'NON_AUTHORITATIVE'),
        observed_at TEXT NOT NULL,
        expires_at TEXT NOT NULL,
        summary TEXT NOT NULL CHECK (length(summary) BETWEEN 1 AND 4096),
        decisions_json TEXT NOT NULL CHECK (length(decisions_json) BETWEEN 2 AND 262144),
        updated_at TEXT NOT NULL
      );

      CREATE TABLE IF NOT EXISTS conversation_writer_leases (
        conversation_id TEXT PRIMARY KEY REFERENCES work_items(id) ON DELETE CASCADE
          CHECK (conversation_id = 'primary'),
        owner_id TEXT NOT NULL CHECK (length(owner_id) BETWEEN 1 AND 128),
        owner_pid INTEGER NOT NULL CHECK (owner_pid > 0),
        lease_expires_at TEXT NOT NULL,
        updated_at TEXT NOT NULL,
        generation INTEGER NOT NULL CHECK (generation > 0)
      );
      PRAGMA user_version = 7;
    `);
    database.exec(workCoreSchemaSql);
    database.exec(decisionCoreSchemaSql);
    initializeDecisionState(database);
    validateControlSchemaProfile(database);
    database.exec("COMMIT;");
  } catch (error) {
    database.exec("ROLLBACK;");
    throw error;
  }
}

function migrateControlDatabaseV2ToV7(database) {
  database.exec("BEGIN IMMEDIATE;");
  try {
    validateLegacyWorkCoreMigrationBase(database);
    validateLegacyDecisionCoreAbsent(database);
    database.exec(`
      CREATE TABLE IF NOT EXISTS conversation_writer_leases (
        conversation_id TEXT PRIMARY KEY REFERENCES work_items(id) ON DELETE CASCADE
          CHECK (conversation_id = 'primary'),
        owner_id TEXT NOT NULL CHECK (length(owner_id) BETWEEN 1 AND 128),
        owner_pid INTEGER NOT NULL CHECK (owner_pid > 0),
        lease_expires_at TEXT NOT NULL,
        updated_at TEXT NOT NULL,
        generation INTEGER NOT NULL CHECK (generation > 0)
      );
      PRAGMA user_version = 7;
    `);
    database.exec(workCoreSchemaSql);
    database.exec(decisionCoreSchemaSql);
    initializeDecisionState(database);
    validateControlSchemaProfile(database);
    database.exec("COMMIT;");
  } catch (error) {
    database.exec("ROLLBACK;");
    throw error;
  }
}

function migrateControlDatabaseV3ToV7(database) {
  database.exec("BEGIN IMMEDIATE;");
  try {
    validateLegacyWorkCoreMigrationBase(database);
    validateLegacyDecisionCoreAbsent(database);
    database.exec(`
      ALTER TABLE conversation_writer_leases RENAME TO conversation_writer_leases_v3;
      CREATE TABLE conversation_writer_leases (
        conversation_id TEXT PRIMARY KEY REFERENCES work_items(id) ON DELETE CASCADE
          CHECK (conversation_id = 'primary'),
        owner_id TEXT NOT NULL CHECK (length(owner_id) BETWEEN 1 AND 128),
        owner_pid INTEGER NOT NULL CHECK (owner_pid > 0),
        lease_expires_at TEXT NOT NULL,
        updated_at TEXT NOT NULL,
        generation INTEGER NOT NULL CHECK (generation > 0)
      );
      INSERT INTO conversation_writer_leases (
        conversation_id, owner_id, owner_pid, lease_expires_at, updated_at, generation
      )
      SELECT conversation_id, owner_id, owner_pid, lease_expires_at, updated_at, 1
      FROM conversation_writer_leases_v3;
      DROP TABLE conversation_writer_leases_v3;
      PRAGMA user_version = 7;
    `);
    database.exec(workCoreSchemaSql);
    database.exec(decisionCoreSchemaSql);
    initializeDecisionState(database);
    validateControlSchemaProfile(database);
    database.exec("COMMIT;");
  } catch (error) {
    database.exec("ROLLBACK;");
    throw error;
  }
}

function migrateControlDatabaseV4ToV7(database) {
  database.exec("BEGIN IMMEDIATE;");
  try {
    validateLegacyWorkCoreMigrationBase(database);
    validateLegacyDecisionCoreAbsent(database);
    database.exec(workCoreSchemaSql);
    database.exec(decisionCoreSchemaSql);
    initializeDecisionState(database);
    database.exec("PRAGMA user_version = 7;");
    validateControlSchemaProfile(database);
    database.exec("COMMIT;");
  } catch (error) {
    database.exec("ROLLBACK;");
    throw error;
  }
}

function migrateControlDatabaseV5ToV7(database) {
  database.exec("BEGIN IMMEDIATE;");
  try {
    validateLegacyWorkCoreMigrationBase(database);
    validateLegacyDecisionCoreAbsent(database);
    database.exec(conversationEventReadIndexesSql);
    database.exec(decisionCoreSchemaSql);
    initializeDecisionState(database);
    database.exec("PRAGMA user_version = 7;");
    validateControlSchemaProfile(database);
    database.exec("COMMIT;");
  } catch (error) {
    database.exec("ROLLBACK;");
    throw error;
  }
}

function migrateControlDatabaseV6ToV7(database) {
  database.exec("BEGIN IMMEDIATE;");
  try {
    validateLegacyWorkCoreMigrationBase(database);
    database.exec(conversationEventReadIndexesSql);
    database.exec("PRAGMA user_version = 7;");
    validateControlSchemaProfile(database);
    database.exec("COMMIT;");
  } catch (error) {
    database.exec("ROLLBACK;");
    throw error;
  }
}

function normalizeComponent(value) {
  const component = requireText(value, "component").toLowerCase();
  if (!/^[a-z0-9][a-z0-9._-]{0,63}$/u.test(component)) {
    throw new TypeError("component must be a lowercase identifier");
  }
  return component;
}

function normalizeHex(value, length, label) {
  const hex = requireText(value, label).toLowerCase();
  if (hex.length !== length || !/^[a-f0-9]+$/u.test(hex)) {
    throw new TypeError(`${label} must be ${length} hexadecimal characters`);
  }
  return hex;
}

function normalizeArtifactPath(value) {
  const artifactPath = requireText(value, "artifact path");
  if (artifactPath.length > 2_048 || !path.isAbsolute(artifactPath)) {
    throw new TypeError("artifact path must be an absolute path of at most 2048 characters");
  }
  return path.normalize(artifactPath);
}

function installationReceiptDigest(receipt) {
  const canonical = JSON.stringify([
    receipt.schema_version,
    receipt.observation_kind,
    receipt.authority,
    receipt.project_id,
    receipt.component,
    receipt.source_commit_sha,
    receipt.artifact_path,
    receipt.artifact_sha256,
  ]);
  return createHash("sha256").update(canonical).digest("hex");
}

function decodeItem(row) {
  if (!row) return null;
  return {
    ...row,
    approval: row.approval_json ? JSON.parse(row.approval_json) : null,
    approval_json: undefined,
  };
}

function decodeWorkEvent(event) {
  if (!event) return null;
  return {
    id: event.id,
    kind: event.kind,
    payload: JSON.parse(event.payload_json),
    created_at: event.created_at,
  };
}

function assertPrimaryConversationIdentity(database, item) {
  if (!item || item.id !== primaryConversationId) {
    throw new Error("primary conversation not found");
  }
  const created = database.prepare(`
    SELECT payload_json FROM work_events
    WHERE work_item_id = ? AND kind = 'created'
    ORDER BY id ASC LIMIT 1
  `).get(primaryConversationId);
  let payload = null;
  try {
    payload = created ? JSON.parse(created.payload_json) : null;
  } catch {
    // The collision error below is intentionally the only public outcome.
  }
  if (payload?.kind !== "primary_conversation") {
    const error = new Error("reserved primary conversation identity is already in use");
    error.code = "CONVERSATION_IDENTITY_COLLISION";
    error.status = 409;
    throw error;
  }
  return item;
}

function workItemChangeEntries(changes) {
  const entries = Object.entries(changes).filter(([key]) => workItemChangeFields.has(key));
  if (changes.status && !statuses.has(changes.status)) throw new TypeError("invalid status");
  return entries;
}

const controlWorkSnapshotSchemaVersion = "lattice.control.work-snapshot.v1";
const controlWorkTreeSchemaVersion = "lattice.control.work-tree.v1";
const controlWorkGraphSchemaVersion = "lattice.control.work-graph.v1";
const controlWorkNodeSchemaVersion = "lattice.control.work-node.v1";
const controlWorkSource = Object.freeze({
  kind: "CONTROL_SQLITE_WORK_ITEMS",
  authority: "CONTROL_LOCAL_PRODUCT_STATE",
});
const controlWorkDefaultMaxNodes = 100;
const controlWorkDefaultMaxEdges = 400;
const controlWorkMaximumNodes = 256;
const controlWorkMaximumEdges = 1_024;
const controlWorkMaximumDependencies = 64;
const controlWorkMaximumDepth = 64;
// Leave deterministic headroom for the MCP JSON-RPC envelope around structuredContent.
const controlWorkMaximumOutputBytes = 1_000_000;
const satisfiedDependencyStatuses = new Set(["verified", "archived"]);

function controlWorkError(code, message, status = 409) {
  const error = new Error(message);
  error.code = code;
  error.status = status;
  return error;
}

function normalizedWorkCoreId(value, label) {
  const id = requireText(value, label);
  if (id.length > 128 || !/^[A-Za-z0-9._:-]+$/u.test(id)) {
    throw controlWorkError(
      "CONTROL_WORK_ID_REJECTED",
      `${label} must contain 1-128 safe ASCII characters`,
      400,
    );
  }
  if (id === primaryConversationId) {
    throw controlWorkError(
      "CONTROL_WORK_PRIMARY_CONVERSATION_REJECTED",
      "the primary conversation is not a work-core node",
      400,
    );
  }
  return id;
}

function normalizedWorkBound(value, fallback, maximum, label) {
  const bound = value ?? fallback;
  if (!Number.isSafeInteger(bound) || bound < 1 || bound > maximum) {
    throw controlWorkError(
      "CONTROL_WORK_BOUND_REJECTED",
      `${label} must be an integer from 1 to ${maximum}`,
      400,
    );
  }
  return bound;
}

function normalizedWorkText(value, label, maximumBytes, { nullable = false } = {}) {
  if (nullable && value == null) return null;
  const text = requireText(value, label);
  if (
    Buffer.byteLength(text, "utf8") > maximumBytes
    || /[\u0000-\u001f\u007f-\u009f]/u.test(text)
  ) {
    throw controlWorkError(
      "CONTROL_WORK_TEXT_REJECTED",
      `${label} is too large or contains unsafe control characters`,
    );
  }
  return text;
}

function normalizedWorkDigest(value, label) {
  if (typeof value !== "string" || !/^[a-f0-9]{64}$/u.test(value)) {
    throw controlWorkError(
      "CONTROL_WORK_REVISION_REJECTED",
      `${label} must be a lowercase SHA-256 digest`,
      400,
    );
  }
  return value;
}

function normalizedBlocker(value) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw controlWorkError("CONTROL_WORK_BLOCKER_REJECTED", "blocker is required", 400);
  }
  const keys = Object.keys(value).sort();
  if (value.status === "clear") {
    if (JSON.stringify(keys) !== JSON.stringify(["status"])) {
      throw controlWorkError(
        "CONTROL_WORK_BLOCKER_REJECTED",
        "a clear blocker cannot include a reason",
        400,
      );
    }
    return { status: "clear", reason: null };
  }
  if (
    value.status !== "blocked"
    || JSON.stringify(keys) !== JSON.stringify(["reason", "status"])
  ) {
    throw controlWorkError(
      "CONTROL_WORK_BLOCKER_REJECTED",
      "blocker must be exactly clear or blocked with one reason",
      400,
    );
  }
  return {
    status: "blocked",
    reason: normalizedWorkText(value.reason, "blocker reason", 2_048),
  };
}

function workSnapshotHash(value) {
  return createHash("sha256").update(JSON.stringify(value), "utf8").digest("hex");
}

function assertWorkGraphAcyclic(nodeIds, adjacency, code, label) {
  const complete = new Set();
  const active = new Set();
  function visit(id, depth) {
    if (depth > controlWorkMaximumDepth) {
      throw controlWorkError(
        "CONTROL_WORK_DEPTH_LIMIT_EXCEEDED",
        `work ${label} exceeds the maximum depth`,
      );
    }
    if (active.has(id)) throw controlWorkError(code, `work ${label} contains a cycle`);
    if (complete.has(id)) return;
    active.add(id);
    for (const next of adjacency.get(id) ?? []) visit(next, depth + 1);
    active.delete(id);
    complete.add(id);
  }
  for (const id of nodeIds) visit(id, 1);
}

function readControlWorkSnapshot(database, {
  projectId,
  maxNodes = controlWorkDefaultMaxNodes,
  maxEdges = controlWorkDefaultMaxEdges,
}) {
  const normalizedProjectId = normalizedWorkCoreId(projectId, "project ID");
  const nodeLimit = normalizedWorkBound(
    maxNodes,
    controlWorkDefaultMaxNodes,
    controlWorkMaximumNodes,
    "work node limit",
  );
  const edgeLimit = normalizedWorkBound(
    maxEdges,
    controlWorkDefaultMaxEdges,
    controlWorkMaximumEdges,
    "work edge limit",
  );
  if (!database.prepare("SELECT 1 FROM projects WHERE id = ?").get(normalizedProjectId)) {
    throw controlWorkError("CONTROL_WORK_PROJECT_NOT_FOUND", "Control project not found", 404);
  }

  const nodeRows = database.prepare(`
    SELECT id, project_id, title, objective, priority, status, progress, updated_at
    FROM work_items
    WHERE project_id = ? AND id <> 'primary'
    ORDER BY id ASC
    LIMIT ?
  `).all(normalizedProjectId, nodeLimit + 1);
  if (nodeRows.length > nodeLimit) {
    throw controlWorkError(
      "CONTROL_WORK_NODE_LIMIT_EXCEEDED",
      `Control work snapshot exceeds ${nodeLimit} nodes`,
    );
  }
  const nodes = nodeRows.map((row) => {
    const id = normalizedWorkCoreId(row.id, "work item ID");
    if (!priorities.has(row.priority) || !statuses.has(row.status)) {
      throw controlWorkError(
        "CONTROL_WORK_NODE_STATE_REJECTED",
        `Control work item ${id} has an invalid state`,
      );
    }
    return {
      id,
      title: normalizedWorkText(row.title, "work item title", 512),
      objective: normalizedWorkText(row.objective, "work item objective", 4_096),
      priority: row.priority,
      status: row.status,
      progress: normalizedWorkText(row.progress, "work item progress", 4_096, { nullable: true }),
      updated_at: normalizedWorkText(row.updated_at, "work item update time", 128),
    };
  });
  const nodeById = new Map(nodes.map((node) => [node.id, node]));
  const primaryConversation = database.prepare(
    "SELECT project_id FROM work_items WHERE id = ?",
  ).get(primaryConversationId);
  if (primaryConversation?.project_id === normalizedProjectId) {
    const reservedRelation = database.prepare(`
      SELECT 1 FROM work_item_relations
      WHERE work_item_id = ? LIMIT 1
    `).get(primaryConversationId) || database.prepare(`
      SELECT 1 FROM work_item_relations
      WHERE parent_work_item_id = ? LIMIT 1
    `).get(primaryConversationId);
    const reservedDependency = database.prepare(`
      SELECT 1 FROM work_item_dependencies
      WHERE work_item_id = ? LIMIT 1
    `).get(primaryConversationId) || database.prepare(`
      SELECT 1 FROM work_item_dependencies
      WHERE depends_on_work_item_id = ? LIMIT 1
    `).get(primaryConversationId);
    if (reservedRelation || reservedDependency) {
      throw controlWorkError(
        "CONTROL_WORK_PRIMARY_CONVERSATION_REJECTED",
        "the primary conversation cannot participate in work-core relations",
      );
    }
  }
  const nodeIds = nodes.map(({ id }) => id);
  let relationRows = [];
  let dependencyRows = [];
  if (nodeIds.length > 0) {
    const placeholders = nodeIds.map(() => "?").join(", ");
    relationRows = database.prepare(`
      SELECT
        relations.work_item_id,
        relations.parent_work_item_id,
        relations.blocker_status,
        relations.blocker_reason,
        child.project_id AS child_project_id,
        parent.project_id AS parent_project_id
      FROM work_item_relations AS relations
      LEFT JOIN work_items AS child ON child.id = relations.work_item_id
      LEFT JOIN work_items AS parent ON parent.id = relations.parent_work_item_id
      WHERE relations.work_item_id IN (${placeholders})
      ORDER BY relations.work_item_id ASC
      LIMIT ?
    `).all(...nodeIds, nodeLimit + 1);
    const invalidIncomingRelation = database.prepare(`
      SELECT child.id AS child_id
      FROM work_item_relations AS relations
      LEFT JOIN work_items AS child ON child.id = relations.work_item_id
      WHERE relations.parent_work_item_id IN (${placeholders})
        AND (child.id IS NULL OR child.project_id <> ?)
      LIMIT 1
    `).get(...nodeIds, normalizedProjectId);
    if (invalidIncomingRelation) {
      throw controlWorkError(
        invalidIncomingRelation.child_id == null
          ? "CONTROL_WORK_ORPHAN_REJECTED"
          : "CONTROL_WORK_CROSS_PROJECT_REJECTED",
        "incoming parent relation must originate in the same Control project",
      );
    }

    dependencyRows = database.prepare(`
      SELECT
        dependencies.work_item_id,
        dependencies.depends_on_work_item_id,
        child.project_id AS child_project_id,
        prerequisite.project_id AS prerequisite_project_id
      FROM work_item_dependencies AS dependencies
      LEFT JOIN work_items AS child ON child.id = dependencies.work_item_id
      LEFT JOIN work_items AS prerequisite
        ON prerequisite.id = dependencies.depends_on_work_item_id
      WHERE dependencies.work_item_id IN (${placeholders})
      ORDER BY dependencies.work_item_id ASC, dependencies.depends_on_work_item_id ASC
      LIMIT ?
    `).all(...nodeIds, edgeLimit + 1);
    if (dependencyRows.length > edgeLimit) {
      throw controlWorkError(
        "CONTROL_WORK_EDGE_LIMIT_EXCEEDED",
        `Control work snapshot exceeds ${edgeLimit} edges`,
      );
    }
    const invalidIncomingDependency = database.prepare(`
      SELECT child.id AS child_id
      FROM work_item_dependencies AS dependencies
      LEFT JOIN work_items AS child ON child.id = dependencies.work_item_id
      WHERE dependencies.depends_on_work_item_id IN (${placeholders})
        AND (child.id IS NULL OR child.project_id <> ?)
      LIMIT 1
    `).get(...nodeIds, normalizedProjectId);
    if (invalidIncomingDependency) {
      throw controlWorkError(
        invalidIncomingDependency.child_id == null
          ? "CONTROL_WORK_ORPHAN_REJECTED"
          : "CONTROL_WORK_CROSS_PROJECT_REJECTED",
        "incoming dependency must originate in the same Control project",
      );
    }
  }
  const parentEdgeCount = relationRows.filter(({ parent_work_item_id: parentId }) => parentId).length;
  if (parentEdgeCount + dependencyRows.length > edgeLimit) {
    throw controlWorkError(
      "CONTROL_WORK_EDGE_LIMIT_EXCEEDED",
      `Control work snapshot exceeds ${edgeLimit} edges`,
    );
  }

  const relations = relationRows.map((row) => {
    const workItemId = normalizedWorkCoreId(row.work_item_id, "work item ID");
    if (!nodeById.has(workItemId)) {
      throw controlWorkError("CONTROL_WORK_ORPHAN_REJECTED", "work relation has no project node");
    }
    const parentId = row.parent_work_item_id == null
      ? null
      : normalizedWorkCoreId(row.parent_work_item_id, "parent work item ID");
    if (
      row.child_project_id !== normalizedProjectId
      || (parentId != null
        && (!nodeById.has(parentId) || row.parent_project_id !== normalizedProjectId))
    ) {
      throw controlWorkError(
        "CONTROL_WORK_CROSS_PROJECT_REJECTED",
        "parent work item must exist in the same Control project",
      );
    }
    if (row.blocker_status === "clear" && row.blocker_reason !== null) {
      throw controlWorkError(
        "CONTROL_WORK_BLOCKER_REJECTED",
        "a clear stored blocker cannot include a reason",
      );
    }
    if (row.blocker_status !== "clear" && row.blocker_status !== "blocked") {
      throw controlWorkError(
        "CONTROL_WORK_BLOCKER_REJECTED",
        "stored blocker status is invalid",
      );
    }
    const blocker = normalizedBlocker(row.blocker_status === "clear"
      ? { status: "clear" }
      : { status: "blocked", reason: row.blocker_reason });
    return {
      work_item_id: workItemId,
      parent_work_item_id: parentId,
      blocker_status: blocker.status,
      blocker_reason: blocker.reason,
    };
  });
  const dependencies = dependencyRows.map((row) => {
    const workItemId = normalizedWorkCoreId(row.work_item_id, "work item ID");
    const dependsOnId = normalizedWorkCoreId(
      row.depends_on_work_item_id,
      "dependency work item ID",
    );
    if (
      !nodeById.has(workItemId)
      || !nodeById.has(dependsOnId)
      || row.child_project_id !== normalizedProjectId
      || row.prerequisite_project_id !== normalizedProjectId
    ) {
      throw controlWorkError(
        "CONTROL_WORK_CROSS_PROJECT_REJECTED",
        "dependency work item must exist in the same Control project",
      );
    }
    return { work_item_id: workItemId, depends_on_work_item_id: dependsOnId };
  });

  const parentByChild = new Map();
  const childrenByParent = new Map(nodes.map(({ id }) => [id, []]));
  for (const relation of relations) {
    if (!relation.parent_work_item_id) continue;
    parentByChild.set(relation.work_item_id, relation.parent_work_item_id);
    childrenByParent.get(relation.parent_work_item_id).push(relation.work_item_id);
  }
  const dependsOnByNode = new Map(nodes.map(({ id }) => [id, []]));
  const reverseByNode = new Map(nodes.map(({ id }) => [id, []]));
  for (const dependency of dependencies) {
    dependsOnByNode.get(dependency.work_item_id).push(dependency.depends_on_work_item_id);
    reverseByNode.get(dependency.depends_on_work_item_id).push(dependency.work_item_id);
  }
  for (const values of [...childrenByParent.values(), ...dependsOnByNode.values(), ...reverseByNode.values()]) {
    values.sort();
  }
  assertWorkGraphAcyclic(
    nodeById.keys(),
    new Map([...parentByChild].map(([childId, parentId]) => [childId, [parentId]])),
    "CONTROL_WORK_HIERARCHY_CYCLE_REJECTED",
    "hierarchy",
  );
  assertWorkGraphAcyclic(
    nodeById.keys(),
    dependsOnByNode,
    "CONTROL_WORK_DEPENDENCY_CYCLE_REJECTED",
    "dependency graph",
  );

  const relationByNode = new Map(relations.map((relation) => [relation.work_item_id, relation]));
  const blockerFor = (id) => {
    const relation = relationByNode.get(id);
    const reasons = [];
    if (relation?.blocker_status === "blocked") {
      reasons.push({ kind: "explicit", reason: relation.blocker_reason });
    }
    for (const dependencyId of dependsOnByNode.get(id)) {
      const dependencyStatus = nodeById.get(dependencyId).status;
      if (!satisfiedDependencyStatuses.has(dependencyStatus)) {
        reasons.push({ kind: "dependency", work_item_id: dependencyId, status: dependencyStatus });
      }
    }
    return { status: reasons.length > 0 ? "blocked" : "clear", reasons };
  };

  const sourceRows = {
    schema_version: "lattice.control.work-source.v1",
    project_id: normalizedProjectId,
    nodes,
    relations,
    dependencies,
  };
  const revision = workSnapshotHash(sourceRows);
  const treeNodes = nodes.map((node) => ({
    id: node.id,
    title: node.title,
    objective: node.objective,
    priority: node.priority,
    status: node.status,
    progress: node.progress,
    parent_id: parentByChild.get(node.id) ?? null,
    children: childrenByParent.get(node.id),
    blocker: blockerFor(node.id),
  }));
  const graphNodes = nodes.map((node) => ({
    id: node.id,
    title: node.title,
    status: node.status,
    depends_on: dependsOnByNode.get(node.id),
    reverse_dependents: reverseByNode.get(node.id),
    blocker: blockerFor(node.id),
  }));
  const treeProjection = {
    schema_version: controlWorkTreeSchemaVersion,
    roots: treeNodes.filter(({ parent_id: parentId }) => parentId == null).map(({ id }) => id),
    nodes: treeNodes,
  };
  const graphProjection = {
    schema_version: controlWorkGraphSchemaVersion,
    nodes: graphNodes,
  };
  const digest = workSnapshotHash({
    schema_version: controlWorkSnapshotSchemaVersion,
    project_id: normalizedProjectId,
    revision,
    tree: treeProjection,
    graph: graphProjection,
  });
  const snapshot = {
    schema_version: controlWorkSnapshotSchemaVersion,
    source: { ...controlWorkSource },
    project_id: normalizedProjectId,
    revision,
    digest,
    tree: { ...treeProjection, revision, digest },
    graph: { ...graphProjection, revision, digest },
  };
  if (
    snapshot.tree.revision !== snapshot.graph.revision
    || snapshot.tree.digest !== snapshot.graph.digest
  ) {
    throw controlWorkError(
      "CONTROL_WORK_PROJECTION_REVISION_MISMATCH",
      "Control work projections do not share one revision and digest",
    );
  }
  if (Buffer.byteLength(JSON.stringify(snapshot), "utf8") > controlWorkMaximumOutputBytes) {
    throw controlWorkError(
      "CONTROL_WORK_OUTPUT_LIMIT_EXCEEDED",
      "Control work snapshot exceeds the response limit",
    );
  }
  return snapshot;
}

export class LatticeStore {
  constructor(databasePath = ":memory:") {
    if (databasePath !== ":memory:") {
      mkdirSync(path.dirname(path.resolve(databasePath)), { recursive: true });
    }
    const database = new DatabaseSync(databasePath);
    try {
      const version = database.prepare("PRAGMA user_version").get().user_version;
      if (![0, 1, 2, 3, 4, 5, 6, controlSchemaVersion].includes(version)) {
        throw new Error(
          `Control database schema ${version} is unsupported; expected 0 through ${controlSchemaVersion}`,
        );
      }
      if (version === 0) initializeControlDatabase(database);
      else if (version === 1) migrateControlDatabaseV1ToV7(database);
      else if (version === 2) migrateControlDatabaseV2ToV7(database);
      else if (version === 3) migrateControlDatabaseV3ToV7(database);
      else if (version === 4) migrateControlDatabaseV4ToV7(database);
      else if (version === 5) migrateControlDatabaseV5ToV7(database);
      else if (version === 6) migrateControlDatabaseV6ToV7(database);
      else validateControlSchemaProfile(database);
      database.exec("PRAGMA foreign_keys = ON;");
      if (databasePath !== ":memory:") database.exec("PRAGMA journal_mode = WAL;");
      this.database = database;
    } catch (error) {
      database.close();
      throw error;
    }
  }

  createInstallationReceipt({
    projectId,
    component,
    sourceCommitSha,
    artifactPath,
    artifactSha256,
  }) {
    const normalizedProjectId = requireText(projectId, "project ID");
    if (!this.getProject(normalizedProjectId)) throw new Error("project not found");
    const receipt = {
      id: randomUUID(),
      schema_version: installationReceiptSchemaVersion,
      observation_kind: installationObservationKind,
      authority: installationReceiptAuthority,
      project_id: normalizedProjectId,
      component: normalizeComponent(component),
      source_commit_sha: normalizeHex(sourceCommitSha, 40, "source commit SHA"),
      artifact_path: normalizeArtifactPath(artifactPath),
      artifact_sha256: normalizeHex(artifactSha256, 64, "artifact SHA-256"),
      recorded_at: now(),
    };
    receipt.receipt_digest = installationReceiptDigest(receipt);
    const result = this.database.prepare(`
      INSERT OR IGNORE INTO installation_receipts (
        id, schema_version, observation_kind, authority, project_id, component,
        source_commit_sha, artifact_path, artifact_sha256, receipt_digest, recorded_at
      ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
    `).run(
      receipt.id,
      receipt.schema_version,
      receipt.observation_kind,
      receipt.authority,
      receipt.project_id,
      receipt.component,
      receipt.source_commit_sha,
      receipt.artifact_path,
      receipt.artifact_sha256,
      receipt.receipt_digest,
      receipt.recorded_at,
    );
    return {
      created: result.changes === 1,
      receipt: this.database.prepare(
        `SELECT installation_receipts.*, projects.name AS project_name
         FROM installation_receipts
         JOIN projects ON projects.id = installation_receipts.project_id
         WHERE receipt_digest = ?`,
      ).get(receipt.receipt_digest),
    };
  }

  listInstallationReceipts({ limit = 50, offset = 0 } = {}) {
    if (!Number.isInteger(limit) || limit < 1 || limit > 100) {
      throw new TypeError("receipt limit must be an integer from 1 to 100");
    }
    if (!Number.isInteger(offset) || offset < 0 || offset > 1_000_000) {
      throw new TypeError("receipt offset must be an integer from 0 to 1000000");
    }
    return this.database.prepare(`
      SELECT installation_receipts.*, projects.name AS project_name
      FROM installation_receipts
      JOIN projects ON projects.id = installation_receipts.project_id
      ORDER BY installation_receipts.rowid DESC
      LIMIT ? OFFSET ?
    `).all(limit, offset);
  }

  getInstallationReceipt(id) {
    return this.database.prepare(`
      SELECT installation_receipts.*, projects.name AS project_name
      FROM installation_receipts
      JOIN projects ON projects.id = installation_receipts.project_id
      WHERE installation_receipts.id = ?
    `).get(requireText(id, "installation receipt ID")) ?? null;
  }

  countInstallationReceipts() {
    return this.database.prepare("SELECT COUNT(*) AS count FROM installation_receipts").get().count;
  }

  replaceDevelopmentRadar(input) {
    const snapshot = normalizedDevelopmentRadar(input);
    const updatedAt = now();
    this.database.prepare(`
      INSERT INTO development_radar (
        slot, schema_version, observation_kind, authority, observed_at,
        expires_at, summary, decisions_json, updated_at
      ) VALUES ('current', ?, ?, ?, ?, ?, ?, ?, ?)
      ON CONFLICT(slot) DO UPDATE SET
        schema_version = excluded.schema_version,
        observation_kind = excluded.observation_kind,
        authority = excluded.authority,
        observed_at = excluded.observed_at,
        expires_at = excluded.expires_at,
        summary = excluded.summary,
        decisions_json = excluded.decisions_json,
        updated_at = excluded.updated_at
    `).run(
      developmentRadarSchemaVersion,
      developmentRadarObservationKind,
      developmentRadarAuthority,
      snapshot.observed_at,
      snapshot.expires_at,
      snapshot.summary,
      JSON.stringify(snapshot.decisions),
      updatedAt,
    );
    return this.getDevelopmentRadar();
  }

  getDevelopmentRadar() {
    const row = this.database.prepare(
      "SELECT * FROM development_radar WHERE slot = 'current'",
    ).get();
    if (!row) return null;
    const { decisions_json: decisionsJson, ...snapshot } = row;
    return {
      ...snapshot,
      decisions: parseJsonArray(decisionsJson),
      freshness: Date.now() < Date.parse(row.expires_at) ? "CURRENT" : "EXPIRED",
    };
  }

  close() {
    this.database.close();
  }

  createProject({ name, rootPath }) {
    const project = {
      id: randomUUID(),
      name: normalizeProjectDisplayName(name),
      root_path: path.resolve(requireText(rootPath, "project root")),
      created_at: now(),
    };
    this.database.prepare(`
      INSERT INTO projects (id, name, root_path, created_at, updated_at)
      VALUES (?, ?, ?, ?, ?)
    `).run(project.id, project.name, project.root_path, project.created_at, project.created_at);
    return project;
  }

  beginProjectRegistration(canonicalPath) {
    const normalizedPath = normalizedAbsolutePath(canonicalPath, "canonical project path");
    const claimedAt = now();
    this.database.exec("BEGIN IMMEDIATE;");
    try {
      const current = this.database.prepare(`
        SELECT generation
        FROM project_registration_claims
        WHERE canonical_path = ? COLLATE NOCASE
      `).get(normalizedPath);
      if (
        current
        && (!Number.isSafeInteger(current.generation)
          || current.generation < 0
          || !Number.isSafeInteger(current.generation + 1))
      ) {
        throw new Error("stored project registration generation is invalid or exhausted");
      }
      const generation = (current?.generation ?? 0) + 1;
      this.database.prepare(`
        INSERT INTO project_registration_claims (canonical_path, generation, claimed_at)
        VALUES (?, ?, ?)
        ON CONFLICT(canonical_path) DO UPDATE SET
          generation = excluded.generation,
          claimed_at = excluded.claimed_at
      `).run(normalizedPath, generation, claimedAt);
      const registered = this.database.prepare(`
        SELECT project_id, refresh_generation
        FROM project_registration_details
        WHERE canonical_path = ? COLLATE NOCASE
      `).get(normalizedPath);
      let projectRefreshGeneration = null;
      if (registered) {
        if (
          !Number.isSafeInteger(registered.refresh_generation)
          || registered.refresh_generation < 0
          || !Number.isSafeInteger(registered.refresh_generation + 1)
        ) {
          throw new Error("stored project refresh generation is invalid or exhausted");
        }
        projectRefreshGeneration = registered.refresh_generation + 1;
        this.database.prepare(`
          UPDATE project_registration_details
          SET refresh_generation = ?
          WHERE project_id = ?
        `).run(projectRefreshGeneration, registered.project_id);
      }
      this.database.exec("COMMIT;");
      return {
        registration_generation: generation,
        project_refresh_generation: projectRefreshGeneration,
      };
    } catch (error) {
      this.database.exec("ROLLBACK;");
      throw error;
    }
  }

  registerProject({
    name,
    inspection,
    registrationGeneration = null,
    projectRefreshGeneration = null,
    claimedCanonicalPath = null,
  }) {
    const normalizedName = normalizeProjectDisplayName(name);
    const normalized = normalizedInspection(inspection);
    const callerClaimed = registrationGeneration != null;
    const claim = callerClaimed
      ? {
          registration_generation: refreshGeneration(registrationGeneration),
          project_refresh_generation: projectRefreshGeneration == null
            ? null
            : refreshGeneration(projectRefreshGeneration),
        }
      : this.beginProjectRegistration(normalized.canonical_path);
    const claimPath = claimedCanonicalPath == null
      ? normalized.canonical_path
      : normalizedAbsolutePath(claimedCanonicalPath, "claimed canonical project path");
    if (claimPath.toLowerCase() !== normalized.canonical_path.toLowerCase()) {
      throw new Error("project canonical path changed during registration");
    }
    let created = false;
    let projectId;
    this.database.exec("BEGIN IMMEDIATE;");
    try {
      const activeClaim = this.database.prepare(`
        SELECT generation
        FROM project_registration_claims
        WHERE canonical_path = ? COLLATE NOCASE
      `).get(normalized.canonical_path);
      if (!activeClaim || activeClaim.generation !== claim.registration_generation) {
        throw new ProjectRegistrationSupersededError();
      }
      const registered = this.database.prepare(`
        SELECT project_id, repo_root_path, refresh_generation, last_refresh_failed_at
        FROM project_registration_details
        WHERE canonical_path = ? COLLATE NOCASE
      `).get(normalized.canonical_path);
      if (registered) {
        projectId = boundedText(registered.project_id, "project ID", 256);
        if (
          !Number.isSafeInteger(registered.refresh_generation)
          || registered.refresh_generation < 0
          || !Number.isSafeInteger(registered.refresh_generation + 1)
        ) {
          throw new Error("stored project refresh generation is invalid or exhausted");
        }
        if (
          claim.project_refresh_generation == null
          || registered.refresh_generation !== claim.project_refresh_generation
        ) {
          throw new ProjectRegistrationSupersededError();
        }
      } else {
        if (claim.project_refresh_generation != null) {
          throw new ProjectRegistrationSupersededError();
        }
        const legacyMatches = this.database.prepare(`
          SELECT id
          FROM projects
          WHERE root_path = ? COLLATE NOCASE
          ORDER BY created_at ASC, id ASC
        `).all(normalized.canonical_path);
        if (legacyMatches.length > 1) {
          throw new Error("multiple legacy projects use the same canonical path");
        }
        if (legacyMatches.length === 1) {
          projectId = boundedText(legacyMatches[0].id, "project ID", 256);
        } else {
          projectId = randomUUID();
          created = true;
          this.database.prepare(`
            INSERT INTO projects (id, name, root_path, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?)
          `).run(
            projectId,
            normalizedName,
            normalized.canonical_path,
            normalized.git.observed_at,
            normalized.git.observed_at,
          );
        }
        this.database.prepare(`
          INSERT INTO project_registration_details (
            project_id, canonical_path, repo_root_path, repo_root_observed_at,
            registered_at, refreshed_at, refresh_generation
          ) VALUES (?, ?, ?, ?, ?, ?, 0)
        `).run(
          projectId,
          normalized.canonical_path,
          normalized.repo_root,
          repositoryRootWasObserved(normalized) ? normalized.git.observed_at : null,
          normalized.git.observed_at,
          normalized.git.observed_at,
        );
      }

      if (!callerClaimed) {
        const latestObservation = this.database.prepare(`
          SELECT git_observed_at
          FROM project_observations
          WHERE project_id = ?
          ORDER BY git_observed_at DESC, rowid DESC
          LIMIT 1
        `).get(projectId);
        if (newerRecordedTime(
          latestObservation,
          registered?.last_refresh_failed_at,
          normalized.git.observed_at,
        )) {
          this.database.exec("COMMIT;");
          return { created: false, project: this.getProjectRegistration(projectId) };
        }
      }

      this.database.prepare(`
        UPDATE projects
        SET name = ?, root_path = ?, updated_at = ?
        WHERE id = ?
      `).run(normalizedName, normalized.canonical_path, normalized.git.observed_at, projectId);
      this.database.prepare(`
        UPDATE project_registration_details
        SET repo_root_path = CASE WHEN ? THEN ? ELSE repo_root_path END,
            repo_root_observed_at = CASE WHEN ? THEN ? ELSE repo_root_observed_at END,
            refreshed_at = ?,
            refresh_generation = CASE WHEN ? THEN refresh_generation ELSE refresh_generation + 1 END,
            last_refresh_failure_code = NULL,
            last_refresh_failure_message = NULL,
            last_refresh_failed_at = NULL
        WHERE project_id = ?
      `).run(
        Number(repositoryRootWasObserved(normalized)),
        normalized.repo_root,
        Number(repositoryRootWasObserved(normalized)),
        normalized.git.observed_at,
        normalized.git.observed_at,
        Number(registered != null),
        projectId,
      );
      this.#insertProjectObservation(projectId, normalized);
      this.database.exec("COMMIT;");
    } catch (error) {
      this.database.exec("ROLLBACK;");
      throw error;
    }
    return { created, project: this.getProjectRegistration(projectId) };
  }

  beginProjectRefresh(projectId) {
    const normalizedProjectId = boundedText(projectId, "project ID", 256);
    this.database.exec("BEGIN IMMEDIATE;");
    try {
      const current = this.database.prepare(`
        SELECT refresh_generation
        FROM project_registration_details
        WHERE project_id = ?
      `).get(normalizedProjectId);
      if (!current) throw new Error("registered project not found");
      if (!Number.isSafeInteger(current.refresh_generation) || current.refresh_generation < 0) {
        throw new Error("stored project refresh generation is invalid");
      }
      const nextGeneration = current.refresh_generation + 1;
      if (!Number.isSafeInteger(nextGeneration)) {
        throw new Error("project refresh generation is exhausted");
      }
      this.database.prepare(`
        UPDATE project_registration_details
        SET refresh_generation = ?
        WHERE project_id = ?
      `).run(nextGeneration, normalizedProjectId);
      this.database.exec("COMMIT;");
      return nextGeneration;
    } catch (error) {
      this.database.exec("ROLLBACK;");
      throw error;
    }
  }

  refreshProject({ projectId, inspection, attemptGeneration = null }) {
    const normalizedProjectId = boundedText(projectId, "project ID", 256);
    const normalized = normalizedInspection(inspection);
    const callerClaimed = attemptGeneration != null;
    const generation = !callerClaimed
      ? this.beginProjectRefresh(normalizedProjectId)
      : refreshGeneration(attemptGeneration);
    this.database.exec("BEGIN IMMEDIATE;");
    try {
      const registered = this.database.prepare(`
        SELECT canonical_path, refresh_generation, last_refresh_failed_at
        FROM project_registration_details
        WHERE project_id = ?
      `).get(normalizedProjectId);
      if (!registered) throw new Error("registered project not found");
      if (registered.canonical_path.toLowerCase() !== normalized.canonical_path.toLowerCase()) {
        throw new Error("project canonical path changed during refresh");
      }
      const latestObservation = callerClaimed ? null : this.database.prepare(`
        SELECT git_observed_at
        FROM project_observations
        WHERE project_id = ?
        ORDER BY git_observed_at DESC, rowid DESC
        LIMIT 1
      `).get(normalizedProjectId);
      if (
        registered.refresh_generation !== generation
        || (!callerClaimed && newerRecordedTime(
          latestObservation,
          registered.last_refresh_failed_at,
          normalized.git.observed_at,
        ))
      ) {
        throw new ProjectRefreshSupersededError();
      }
      this.database.prepare(`
        UPDATE projects SET root_path = ?, updated_at = ? WHERE id = ?
      `).run(normalized.canonical_path, normalized.git.observed_at, normalizedProjectId);
      this.database.prepare(`
        UPDATE project_registration_details
        SET repo_root_path = CASE WHEN ? THEN ? ELSE repo_root_path END,
            repo_root_observed_at = CASE WHEN ? THEN ? ELSE repo_root_observed_at END,
            refreshed_at = ?,
            last_refresh_failure_code = NULL,
            last_refresh_failure_message = NULL,
            last_refresh_failed_at = NULL
        WHERE project_id = ? AND refresh_generation = ?
      `).run(
        Number(repositoryRootWasObserved(normalized)),
        normalized.repo_root,
        Number(repositoryRootWasObserved(normalized)),
        normalized.git.observed_at,
        normalized.git.observed_at,
        normalizedProjectId,
        generation,
      );
      this.#insertProjectObservation(normalizedProjectId, normalized);
      this.database.exec("COMMIT;");
    } catch (error) {
      this.database.exec("ROLLBACK;");
      throw error;
    }
    return this.getProjectRegistration(normalizedProjectId);
  }

  recordProjectRefreshFailure({
    projectId,
    code,
    message,
    observedAt,
    attemptGeneration = null,
  }) {
    const normalizedProjectId = boundedText(projectId, "project ID", 256);
    const normalizedCode = boundedText(code, "project refresh failure code", 128);
    const normalizedMessage = boundedText(message, "project refresh failure message", 512);
    const normalizedObservedAt = observationTime(observedAt, "project refresh failure time");
    const callerClaimed = attemptGeneration != null;
    const generation = !callerClaimed
      ? this.beginProjectRefresh(normalizedProjectId)
      : refreshGeneration(attemptGeneration);
    this.database.exec("BEGIN IMMEDIATE;");
    try {
      const registered = this.database.prepare(`
        SELECT refresh_generation, last_refresh_failed_at
        FROM project_registration_details
        WHERE project_id = ?
      `).get(normalizedProjectId);
      if (!registered) throw new Error("registered project not found");
      const latestObservation = callerClaimed ? null : this.database.prepare(`
        SELECT git_observed_at
        FROM project_observations
        WHERE project_id = ?
        ORDER BY git_observed_at DESC, rowid DESC
        LIMIT 1
      `).get(normalizedProjectId);
      if (
        registered.refresh_generation !== generation
        || (!callerClaimed && newerRecordedTime(
          latestObservation,
          registered.last_refresh_failed_at,
          normalizedObservedAt,
        ))
      ) {
        throw new ProjectRefreshSupersededError();
      }
      this.database.prepare(`
        UPDATE project_registration_details
        SET last_refresh_failure_code = ?,
            last_refresh_failure_message = ?,
            last_refresh_failed_at = ?
        WHERE project_id = ? AND refresh_generation = ?
      `).run(
        normalizedCode,
        normalizedMessage,
        normalizedObservedAt,
        normalizedProjectId,
        generation,
      );
      this.database.exec("COMMIT;");
    } catch (error) {
      this.database.exec("ROLLBACK;");
      throw error;
    }
    return this.getProjectRegistration(normalizedProjectId);
  }

  #insertProjectObservation(projectId, inspection) {
    const observationId = randomUUID();
    const booleanValue = (value) => value == null ? null : Number(value);
    this.database.prepare(`
      INSERT INTO project_observations (
        id, project_id, git_status, is_git_repository, branch, detached, head_sha,
        dirty, upstream, ahead, behind, git_observed_at, git_failures_json,
        rule_status, rules_observed_at, missing_rules_json, rule_failures_json
      ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
    `).run(
      observationId,
      projectId,
      inspection.git.status,
      booleanValue(inspection.git.is_repository),
      inspection.git.branch,
      booleanValue(inspection.git.detached),
      inspection.git.head_sha,
      booleanValue(inspection.git.dirty),
      inspection.git.upstream,
      inspection.git.ahead,
      inspection.git.behind,
      inspection.git.observed_at,
      JSON.stringify(inspection.git.failures),
      inspection.rules.status,
      inspection.rules.observed_at,
      JSON.stringify(inspection.rules.missing_standard_documents),
      JSON.stringify(inspection.rules.failures),
    );
    const remoteInsert = this.database.prepare(`
      INSERT INTO project_git_remotes (
        observation_id, name, direction, url_sanitized, credentials_redacted
      ) VALUES (?, ?, ?, ?, ?)
    `);
    for (const remote of inspection.git.remotes) {
      remoteInsert.run(
        observationId,
        remote.name,
        remote.direction,
        remote.url,
        Number(remote.credentials_redacted),
      );
    }
    const ruleInsert = this.database.prepare(`
      INSERT INTO project_rule_documents (
        observation_id, relative_path, sha256, purpose, observed_at
      ) VALUES (?, ?, ?, ?, ?)
    `);
    for (const document of inspection.rules.documents) {
      ruleInsert.run(
        observationId,
        document.relative_path,
        document.sha256,
        document.purpose,
        document.observed_at,
      );
    }
    this.database.prepare(`
      DELETE FROM project_observations
      WHERE project_id = ? AND id <> ?
    `).run(projectId, observationId);
  }

  listProjects() {
    return this.database.prepare(`
      SELECT
        projects.*,
        project_registration_details.canonical_path,
        project_registration_details.repo_root_path AS repo_root,
        project_registration_details.repo_root_observed_at,
        project_registration_details.registered_at,
        project_registration_details.refreshed_at
      FROM projects
      LEFT JOIN project_registration_details
        ON project_registration_details.project_id = projects.id
      ORDER BY projects.created_at DESC
    `).all().map((project) => ({
      ...project,
      schema_version: project.registered_at ? projectCatalogSchemaVersion : null,
      record_kind: project.registered_at ? projectCatalogRecordKind : legacyProjectRecordKind,
      registry_authority: "NONE",
      registry_project_id: null,
      control_project_id: project.id,
    }));
  }

  projectContextCandidates() {
    return this.database.prepare(`
      SELECT id, name
      FROM projects
      LIMIT 2
    `).all();
  }

  getProject(id) {
    return this.database.prepare("SELECT * FROM projects WHERE id = ?").get(id) ?? null;
  }

  getProjectRegistration(id) {
    this.database.exec("BEGIN;");
    try {
      const project = this.#getProjectRegistrationSnapshot(id);
      this.database.exec("COMMIT;");
      return project;
    } catch (error) {
      this.database.exec("ROLLBACK;");
      throw error;
    }
  }

  #getProjectRegistrationSnapshot(id) {
    const projectId = boundedText(id, "project ID", 256);
    const project = this.database.prepare(`
      SELECT
        projects.*,
        project_registration_details.canonical_path,
        project_registration_details.repo_root_path AS repo_root,
        project_registration_details.repo_root_observed_at,
        project_registration_details.registered_at,
        project_registration_details.refreshed_at,
        project_registration_details.last_refresh_failure_code,
        project_registration_details.last_refresh_failure_message,
        project_registration_details.last_refresh_failed_at
      FROM projects
      JOIN project_registration_details
        ON project_registration_details.project_id = projects.id
      WHERE projects.id = ?
    `).get(projectId);
    if (!project) return null;
    const observation = this.database.prepare(`
      SELECT *
      FROM project_observations
      WHERE project_id = ?
      ORDER BY rowid DESC
      LIMIT 1
    `).get(projectId);
    if (!observation) throw new Error("registered project has no observation");
    const remotes = this.database.prepare(`
      SELECT name, direction, url_sanitized AS url, credentials_redacted
      FROM project_git_remotes
      WHERE observation_id = ?
      ORDER BY name ASC, direction ASC, url_sanitized ASC
    `).all(observation.id).map((remote) => ({
      ...remote,
      credentials_redacted: remote.credentials_redacted === 1,
    }));
    const documents = this.database.prepare(`
      SELECT relative_path, sha256, observed_at, purpose
      FROM project_rule_documents
      WHERE observation_id = ?
      ORDER BY relative_path ASC
    `).all(observation.id);
    const {
      last_refresh_failure_code: failureCode,
      last_refresh_failure_message: failureMessage,
      last_refresh_failed_at: failureObservedAt,
      ...projectIdentity
    } = project;
    return {
      ...projectIdentity,
      schema_version: projectCatalogSchemaVersion,
      record_kind: projectCatalogRecordKind,
      registry_authority: "NONE",
      registry_project_id: null,
      control_project_id: projectIdentity.id,
      last_refresh_failure: failureCode
        ? {
            code: failureCode,
            message: failureMessage,
            observed_at: failureObservedAt,
          }
        : null,
      git_observation: {
        status: observation.git_status,
        is_repository: decodeNullableBoolean(observation.is_git_repository),
        branch: observation.branch,
        detached: decodeNullableBoolean(observation.detached),
        head_sha: observation.head_sha,
        dirty: decodeNullableBoolean(observation.dirty),
        upstream: observation.upstream,
        ahead: observation.ahead,
        behind: observation.behind,
        remotes,
        observed_at: observation.git_observed_at,
        failures: parseJsonArray(observation.git_failures_json),
      },
      rule_index: {
        status: observation.rule_status,
        observed_at: observation.rules_observed_at,
        documents,
        missing_standard_documents: parseJsonArray(observation.missing_rules_json),
        failures: parseJsonArray(observation.rule_failures_json),
      },
    };
  }

  acquirePrimaryConversationLease({ ownerId, ownerPid, ttlMs = 15_000 }) {
    const normalizedOwner = normalizeConversationLeaseOwner(ownerId);
    const normalizedPid = normalizeConversationLeasePid(ownerPid);
    const normalizedTtl = normalizeConversationLeaseTtl(ttlMs);
    const timestamp = now();
    const expiresAt = new Date(Date.parse(timestamp) + normalizedTtl).toISOString();
    this.database.exec("BEGIN IMMEDIATE;");
    try {
      const item = this.database.prepare("SELECT * FROM work_items WHERE id = ?")
        .get(primaryConversationId);
      assertPrimaryConversationIdentity(this.database, item);
      const current = this.database.prepare(`
        SELECT * FROM conversation_writer_leases WHERE conversation_id = ?
      `).get(primaryConversationId);
      if (
        current
        && current.owner_id !== normalizedOwner
        && current.lease_expires_at > timestamp
      ) {
        const error = new Error("另一個 LATTICE Control 正在處理這條對話；本次不會重複送出");
        error.code = "CONVERSATION_WRITER_BUSY";
        error.status = 409;
        throw error;
      }
      const generation = current
        && current.owner_id === normalizedOwner
        && current.lease_expires_at > timestamp
        ? current.generation
        : (current?.generation ?? 0) + 1;
      this.database.prepare(`
        INSERT INTO conversation_writer_leases (
          conversation_id, owner_id, owner_pid, generation, lease_expires_at, updated_at
        ) VALUES (?, ?, ?, ?, ?, ?)
        ON CONFLICT(conversation_id) DO UPDATE SET
          owner_id = excluded.owner_id,
          owner_pid = excluded.owner_pid,
          generation = excluded.generation,
          lease_expires_at = excluded.lease_expires_at,
          updated_at = excluded.updated_at
      `).run(
        primaryConversationId,
        normalizedOwner,
        normalizedPid,
        generation,
        expiresAt,
        timestamp,
      );
      this.database.exec("COMMIT;");
      return {
        conversation_id: primaryConversationId,
        owner_id: normalizedOwner,
        owner_pid: normalizedPid,
        generation,
        lease_expires_at: expiresAt,
        updated_at: timestamp,
      };
    } catch (error) {
      this.database.exec("ROLLBACK;");
      throw error;
    }
  }

  renewPrimaryConversationLease({ ownerId, generation, ttlMs = 15_000 }) {
    const normalizedOwner = normalizeConversationLeaseOwner(ownerId);
    const normalizedGeneration = normalizeConversationLeaseGeneration(generation);
    const normalizedTtl = normalizeConversationLeaseTtl(ttlMs);
    const timestamp = now();
    const expiresAt = new Date(Date.parse(timestamp) + normalizedTtl).toISOString();
    const result = this.database.prepare(`
      UPDATE conversation_writer_leases
      SET lease_expires_at = ?, updated_at = ?
      WHERE conversation_id = ? AND owner_id = ? AND generation = ?
        AND lease_expires_at > ?
    `).run(
      expiresAt,
      timestamp,
      primaryConversationId,
      normalizedOwner,
      normalizedGeneration,
      timestamp,
    );
    if (result.changes !== 1) return null;
    return {
      conversation_id: primaryConversationId,
      owner_id: normalizedOwner,
      generation: normalizedGeneration,
      lease_expires_at: expiresAt,
      updated_at: timestamp,
    };
  }

  assertPrimaryConversationLease(fence) {
    return assertPrimaryConversationFence(this.database, fence);
  }

  ownsPrimaryConversationLease(ownerId, generation) {
    const normalizedOwner = normalizeConversationLeaseOwner(ownerId);
    const normalizedGeneration = normalizeConversationLeaseGeneration(generation);
    const lease = this.database.prepare(`
      SELECT owner_id, generation, lease_expires_at
      FROM conversation_writer_leases WHERE conversation_id = ?
    `).get(primaryConversationId);
    return Boolean(
      lease?.owner_id === normalizedOwner
      && lease?.generation === normalizedGeneration
      && lease.lease_expires_at > now()
    );
  }

  releasePrimaryConversationLease({ ownerId, generation }) {
    const normalizedOwner = normalizeConversationLeaseOwner(ownerId);
    const normalizedGeneration = normalizeConversationLeaseGeneration(generation);
    const timestamp = now();
    return this.database.prepare(`
      UPDATE conversation_writer_leases
      SET lease_expires_at = '1970-01-01T00:00:00.000Z', updated_at = ?
      WHERE conversation_id = ? AND owner_id = ? AND generation = ?
    `).run(
      timestamp,
      primaryConversationId,
      normalizedOwner,
      normalizedGeneration,
    ).changes === 1;
  }

  ensurePrimaryConversation(projectId, { provisional = false } = {}) {
    const normalizedProjectId = requireText(projectId, "project ID");
    if (typeof provisional !== "boolean") throw new TypeError("provisional must be boolean");
    if (!this.getProject(normalizedProjectId)) throw new Error("project not found");
    this.database.exec("BEGIN IMMEDIATE;");
    try {
      let item = this.database.prepare("SELECT * FROM work_items WHERE id = ?")
        .get(primaryConversationId);
      if (!item) {
        const timestamp = now();
        this.database.prepare(`
          INSERT INTO work_items (
            id, project_id, title, objective, priority, status, created_at, updated_at
          ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
        `).run(
          primaryConversationId,
          normalizedProjectId,
          "主對話",
          "Stable LATTICE Control user conversation binding.",
          "normal",
          provisional ? "selection_pending" : "draft",
          timestamp,
          timestamp,
        );
        this.database.prepare(`
          INSERT INTO work_events (work_item_id, kind, payload_json, created_at)
          VALUES (?, ?, ?, ?)
        `).run(
          primaryConversationId,
          "created",
          JSON.stringify({ priority: "normal", kind: "primary_conversation" }),
          timestamp,
        );
        item = this.database.prepare("SELECT * FROM work_items WHERE id = ?")
          .get(primaryConversationId);
      }
      assertPrimaryConversationIdentity(this.database, item);
      this.database.exec("COMMIT;");
      return decodeItem(item);
    } catch (error) {
      this.database.exec("ROLLBACK;");
      throw error;
    }
  }

  selectPrimaryConversationProject({ projectId, fence }) {
    const normalizedProjectId = requireText(projectId, "project ID");
    if (!this.getProject(normalizedProjectId)) throw new Error("project not found");
    this.database.exec("BEGIN IMMEDIATE;");
    try {
      const item = this.database.prepare("SELECT * FROM work_items WHERE id = ?")
        .get(primaryConversationId);
      assertPrimaryConversationIdentity(this.database, item);
      assertPrimaryConversationFence(this.database, fence);
      if (["starting", "running", "waiting_approval"].includes(item.status)) {
        const error = new Error("目前的對話仍在執行，請先等待完成或中斷");
        error.code = "CONVERSATION_BUSY";
        error.status = 409;
        throw error;
      }
      if (this.primaryConversationUnresolvedMessage()) {
        const error = new Error("已有保存但尚未確認的訊息，請先重新連線");
        error.code = "CONVERSATION_RECONCILIATION_REQUIRED";
        error.status = 409;
        throw error;
      }
      if (this.primaryConversationHasUnresolvedTurn()) {
        const error = new Error("目前的對話尚未留下可驗證的結尾，請先重新連線");
        error.code = "CONVERSATION_RECONCILIATION_REQUIRED";
        error.status = 409;
        throw error;
      }
      if (item.project_id === normalizedProjectId && item.status !== "selection_pending") {
        this.database.exec("COMMIT;");
        return decodeItem(item);
      }
      const timestamp = now();
      const targetStatus = item.status === "selection_pending" ? "draft" : item.status;
      this.database.prepare(`
        UPDATE work_items
        SET project_id = ?, status = ?, progress = ?, updated_at = ?
        WHERE id = ?
      `).run(
        normalizedProjectId,
        targetStatus,
        "已選定工作專案；準備連接 Codex",
        timestamp,
        primaryConversationId,
      );
      this.database.prepare(`
        INSERT INTO work_events (work_item_id, kind, payload_json, created_at)
        VALUES (?, 'conversation_project_selected', ?, ?)
      `).run(
        primaryConversationId,
        JSON.stringify({
          projectId: normalizedProjectId,
          previousProjectId: item.project_id,
        }),
        timestamp,
      );
      const updated = this.database.prepare("SELECT * FROM work_items WHERE id = ?")
        .get(primaryConversationId);
      this.database.exec("COMMIT;");
      return decodeItem(updated);
    } catch (error) {
      this.database.exec("ROLLBACK;");
      throw error;
    }
  }

  primaryConversationMessage(clientMessageId) {
    const normalizedId = normalizeClientMessageId(clientMessageId);
    return decodeWorkEvent(this.database.prepare(`
      SELECT id, kind, payload_json, created_at
      FROM work_events
      WHERE work_item_id = ? AND kind = 'conversation_message_claimed'
        AND json_extract(payload_json, '$.clientMessageId') = ?
      ORDER BY id DESC LIMIT 1
    `).get(primaryConversationId, normalizedId));
  }

  primaryConversationUnresolvedMessage() {
    const latestClaim = this.database.prepare(`
      SELECT id, kind, payload_json, created_at
      FROM work_events
      WHERE work_item_id = ? AND kind = 'conversation_message_claimed'
      ORDER BY id DESC LIMIT 1
    `).get(primaryConversationId);
    if (!latestClaim) return null;
    const latestAcceptance = this.database.prepare(`
      SELECT id FROM work_events
      WHERE work_item_id = ? AND kind = 'conversation_message_accepted'
      ORDER BY id DESC LIMIT 1
    `).get(primaryConversationId)?.id ?? null;
    // Claiming a newer message is forbidden until the prior claim is accepted.
    return latestAcceptance === null || latestClaim.id > latestAcceptance
      ? decodeWorkEvent(latestClaim)
      : null;
  }

  hasUnresolvedPrimaryConversationMessage() {
    return this.primaryConversationUnresolvedMessage() !== null;
  }

  primaryConversationHasUnresolvedTurn() {
    const item = this.database.prepare("SELECT * FROM work_items WHERE id = ?")
      .get(primaryConversationId);
    if (!item) return false;
    if (Boolean(item.codex_thread_id) !== Boolean(item.codex_turn_id)) return true;
    if (!item.codex_thread_id || !item.codex_turn_id) return false;
    const accepted = this.primaryConversationAcceptedForTurn(
      item.codex_thread_id,
      item.codex_turn_id,
    );
    if (!accepted) return false;
    return !this.primaryConversationTerminalEvent(item.codex_thread_id, item.codex_turn_id)
      || this.primaryConversationMissingFinal(item.codex_thread_id, item.codex_turn_id);
  }

  latestPrimaryConversationBinding() {
    return decodeWorkEvent(this.database.prepare(`
      SELECT id, kind, payload_json, created_at
      FROM work_events
      WHERE work_item_id = ? AND kind = 'conversation_thread_bound'
      ORDER BY id DESC LIMIT 1
    `).get(primaryConversationId));
  }

  primaryConversationAcceptedEvent(clientMessageId) {
    const normalizedId = normalizeClientMessageId(clientMessageId);
    return decodeWorkEvent(this.database.prepare(`
      SELECT id, kind, payload_json, created_at
      FROM work_events
      WHERE work_item_id = ? AND kind = 'conversation_message_accepted'
        AND json_extract(payload_json, '$.clientMessageId') = ?
      ORDER BY id DESC LIMIT 1
    `).get(primaryConversationId, normalizedId));
  }

  primaryConversationAcceptedForTurn(threadId, turnId) {
    const normalizedThreadId = requireText(threadId, "Codex thread ID");
    const normalizedTurnId = requireText(turnId, "Codex turn ID");
    return decodeWorkEvent(this.database.prepare(`
      SELECT id, kind, payload_json, created_at
      FROM work_events
      WHERE work_item_id = ? AND kind = 'conversation_message_accepted'
        AND json_extract(payload_json, '$.threadId') = ?
        AND json_extract(payload_json, '$.turnId') = ?
      ORDER BY id DESC LIMIT 1
    `).get(primaryConversationId, normalizedThreadId, normalizedTurnId));
  }

  primaryConversationFirstActivity(threadId, turnId) {
    const normalizedThreadId = requireText(threadId, "Codex thread ID");
    const normalizedTurnId = requireText(turnId, "Codex turn ID");
    return decodeWorkEvent(this.database.prepare(`
      SELECT id, kind, payload_json, created_at
      FROM work_events
      WHERE work_item_id = ? AND kind = 'conversation_first_activity'
        AND json_extract(payload_json, '$.threadId') = ?
        AND json_extract(payload_json, '$.turnId') = ?
      ORDER BY id ASC LIMIT 1
    `).get(primaryConversationId, normalizedThreadId, normalizedTurnId));
  }

  primaryConversationHasAcceptedThread(threadId) {
    const normalizedThreadId = requireText(threadId, "Codex thread ID");
    return Boolean(this.database.prepare(`
      SELECT 1 AS found FROM work_events
      WHERE work_item_id = ? AND kind = 'conversation_message_accepted'
        AND json_extract(payload_json, '$.threadId') = ?
      LIMIT 1
    `).get(primaryConversationId, normalizedThreadId)?.found);
  }

  primaryConversationTerminalEvent(threadId, turnId) {
    const normalizedThreadId = requireText(threadId, "Codex thread ID");
    const normalizedTurnId = requireText(turnId, "Codex turn ID");
    return decodeWorkEvent(this.database.prepare(`
      SELECT id, kind, payload_json, created_at
      FROM work_events
      WHERE work_item_id = ? AND kind = 'turn_completed'
        AND json_extract(payload_json, '$.threadId') = ?
        AND json_extract(payload_json, '$.turnId') = ?
        AND json_extract(payload_json, '$.status') IN ('completed', 'interrupted', 'failed')
      ORDER BY id DESC LIMIT 1
    `).get(primaryConversationId, normalizedThreadId, normalizedTurnId));
  }

  primaryConversationMissingFinal(threadId, turnId) {
    if (typeof threadId !== "string" || !threadId || typeof turnId !== "string" || !turnId) {
      return false;
    }
    const latestMissing = this.database.prepare(`
      SELECT id FROM work_events
      WHERE work_item_id = ? AND kind = 'conversation_terminal_missing_reply'
        AND json_extract(payload_json, '$.threadId') = ?
        AND json_extract(payload_json, '$.turnId') = ?
      ORDER BY id DESC LIMIT 1
    `).get(primaryConversationId, threadId, turnId)?.id ?? null;
    if (latestMissing === null) return false;
    const latestReply = this.database.prepare(`
      SELECT id FROM work_events
      WHERE work_item_id = ? AND kind = 'conversation_assistant_message'
        AND json_extract(payload_json, '$.threadId') = ?
        AND json_extract(payload_json, '$.turnId') = ?
      ORDER BY id DESC LIMIT 1
    `).get(primaryConversationId, threadId, turnId)?.id ?? null;
    return latestReply === null || latestMissing > latestReply;
  }

  claimPrimaryConversationMessage({ projectId, clientMessageId, text, fence }) {
    const normalizedProjectId = requireText(projectId, "project ID");
    const normalizedId = normalizeClientMessageId(clientMessageId);
    const normalizedText = boundedConversationText(text);
    const promptDigest = conversationMessageDigest(normalizedText);
    if (!this.getProject(normalizedProjectId)) throw new Error("project not found");

    this.database.exec("BEGIN IMMEDIATE;");
    try {
      let item = this.database.prepare("SELECT * FROM work_items WHERE id = ?")
        .get(primaryConversationId);
      if (!item) {
        const timestamp = now();
        this.database.prepare(`
          INSERT INTO work_items (
            id, project_id, title, objective, priority, status, created_at, updated_at
          ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
        `).run(
          primaryConversationId,
          normalizedProjectId,
          "主對話",
          "Stable LATTICE Control user conversation binding.",
          "normal",
          "draft",
          timestamp,
          timestamp,
        );
        this.database.prepare(`
          INSERT INTO work_events (work_item_id, kind, payload_json, created_at)
          VALUES (?, ?, ?, ?)
        `).run(
          primaryConversationId,
          "created",
          JSON.stringify({ priority: "normal", kind: "primary_conversation" }),
          timestamp,
        );
        item = this.database.prepare("SELECT * FROM work_items WHERE id = ?")
          .get(primaryConversationId);
      }

      assertPrimaryConversationIdentity(this.database, item);
      assertPrimaryConversationFence(this.database, fence);
      const existing = this.primaryConversationMessage(normalizedId);
      const unresolved = this.primaryConversationUnresolvedMessage();
      if (
        unresolved
        && (!existing || unresolved.payload.clientMessageId !== normalizedId)
      ) {
        const error = new Error("an earlier saved conversation message must be reconnected first");
        error.code = "CONVERSATION_BUSY";
        error.status = 409;
        throw error;
      }
      if (existing) {
        if (
          existing.payload.promptDigest !== promptDigest
          || existing.payload.projectId !== normalizedProjectId
        ) {
          throw new TypeError("client message ID was already used for different content");
        }
        const accepted = this.primaryConversationAcceptedEvent(normalizedId) !== null;
        if (!accepted && item.status === "failed") {
          const timestamp = now();
          this.database.prepare(`
            UPDATE work_items
            SET status = 'starting', progress = ?, failure_summary = NULL, updated_at = ?
            WHERE id = ? AND status = 'failed'
          `).run(
            "正在找回已保存的訊息；不會重複送出",
            timestamp,
            primaryConversationId,
          );
          item = this.database.prepare("SELECT * FROM work_items WHERE id = ?")
            .get(primaryConversationId);
        }
        this.database.exec("COMMIT;");
        return { claimed: false, item: decodeItem(item), event: existing };
      }
      if (!["draft", "codex_done", "failed"].includes(item.status)) {
        const error = new Error("the primary conversation is already handling a message");
        error.code = "CONVERSATION_BUSY";
        error.status = 409;
        throw error;
      }

      const timestamp = now();
      const update = this.database.prepare(`
        UPDATE work_items
        SET project_id = ?, status = 'starting', progress = ?, approval_json = NULL,
            failure_summary = NULL, updated_at = ?
        WHERE id = ? AND status IN ('draft', 'codex_done', 'failed')
      `).run(
        normalizedProjectId,
        "訊息已保存；正在連接 Codex",
        timestamp,
        primaryConversationId,
      );
      if (update.changes !== 1) {
        const error = new Error("the primary conversation changed before the message was claimed");
        error.code = "CONVERSATION_BUSY";
        error.status = 409;
        throw error;
      }
      const inserted = this.database.prepare(`
        INSERT INTO work_events (work_item_id, kind, payload_json, created_at)
        VALUES (?, ?, ?, ?)
        RETURNING id
      `).get(
        primaryConversationId,
        "conversation_message_claimed",
        JSON.stringify({
          clientMessageId: normalizedId,
          projectId: normalizedProjectId,
          text: normalizedText,
          promptDigest,
        }),
        timestamp,
      );
      item = this.database.prepare("SELECT * FROM work_items WHERE id = ?")
        .get(primaryConversationId);
      this.database.exec("COMMIT;");
      return {
        claimed: true,
        item: decodeItem(item),
        event: {
          id: inserted.id,
          kind: "conversation_message_claimed",
          payload: {
            clientMessageId: normalizedId,
            projectId: normalizedProjectId,
            text: normalizedText,
            promptDigest,
          },
          created_at: timestamp,
        },
      };
    } catch (error) {
      this.database.exec("ROLLBACK;");
      throw error;
    }
  }

  primaryConversationDispatchIntent(clientMessageId) {
    const normalizedId = normalizeClientMessageId(clientMessageId);
    return decodeWorkEvent(this.database.prepare(`
      SELECT id, kind, payload_json, created_at
      FROM work_events
      WHERE work_item_id = ? AND kind = 'conversation_turn_dispatch_intended'
        AND json_extract(payload_json, '$.clientMessageId') = ?
      ORDER BY id DESC LIMIT 1
    `).get(primaryConversationId, normalizedId));
  }

  primaryConversationDispatchNotSent({
    clientMessageId,
    threadId,
    promptDigest,
    originalIntentEventId,
  }) {
    const normalizedId = normalizeClientMessageId(clientMessageId);
    return decodeWorkEvent(this.database.prepare(`
      SELECT id, kind, payload_json, created_at
      FROM work_events
      WHERE work_item_id = ? AND kind = 'conversation_turn_dispatch_not_sent'
        AND id > ?
        AND json_extract(payload_json, '$.clientMessageId') = ?
        AND json_extract(payload_json, '$.threadId') = ?
        AND json_extract(payload_json, '$.promptDigest') = ?
        AND json_extract(payload_json, '$.originalIntentEventId') = ?
        AND json_extract(payload_json, '$.attempt') = 1
        AND json_extract(payload_json, '$.errorCode')
          = 'CODEX_APP_SERVER_EFFECT_IDENTITY_CHANGED'
      ORDER BY id ASC LIMIT 1
    `).get(
      primaryConversationId,
      originalIntentEventId,
      normalizedId,
      threadId,
      promptDigest,
      originalIntentEventId,
    ));
  }

  primaryConversationRetryIntent({
    clientMessageId,
    threadId,
    promptDigest,
    originalIntentEventId,
    afterEventId,
  }) {
    const normalizedId = normalizeClientMessageId(clientMessageId);
    return decodeWorkEvent(this.database.prepare(`
      SELECT id, kind, payload_json, created_at
      FROM work_events
      WHERE work_item_id = ? AND kind = 'conversation_turn_dispatch_retry_intended'
        AND id > ?
        AND json_extract(payload_json, '$.clientMessageId') = ?
        AND json_extract(payload_json, '$.threadId') = ?
        AND json_extract(payload_json, '$.promptDigest') = ?
        AND json_extract(payload_json, '$.originalIntentEventId') = ?
        AND json_extract(payload_json, '$.attempt') = 2
      ORDER BY id ASC LIMIT 1
    `).get(
      primaryConversationId,
      afterEventId,
      normalizedId,
      threadId,
      promptDigest,
      originalIntentEventId,
    ));
  }

  recordPrimaryConversationDispatchIntent({
    clientMessageId,
    threadId,
    promptDigest,
    fence,
  }) {
    const normalizedId = normalizeClientMessageId(clientMessageId);
    const normalizedThreadId = boundedText(threadId, "Codex thread ID", 256);
    const normalizedDigest = normalizeHex(promptDigest, 64, "conversation prompt digest");
    const normalizedFence = normalizeConversationFence(fence);
    this.database.exec("BEGIN IMMEDIATE;");
    try {
      const item = this.database.prepare("SELECT * FROM work_items WHERE id = ?")
        .get(primaryConversationId);
      assertPrimaryConversationIdentity(this.database, item);
      assertPrimaryConversationFence(this.database, normalizedFence);
      if (item.codex_thread_id !== normalizedThreadId || item.status !== "starting") {
        throw new Error("primary conversation changed before turn dispatch intent was saved");
      }
      const claimed = this.primaryConversationMessage(normalizedId)?.payload ?? null;
      if (!claimed || claimed.promptDigest !== normalizedDigest) {
        throw new Error("turn dispatch intent has no exact saved message claim");
      }
      const existing = this.primaryConversationDispatchIntent(normalizedId);
      if (existing) {
        if (
          existing.payload.threadId !== normalizedThreadId
          || existing.payload.promptDigest !== normalizedDigest
        ) throw new Error("saved turn dispatch intent changed identity");
        this.database.exec("COMMIT;");
        return {
          created: false,
          event: {
            ...existing,
          },
        };
      }
      const timestamp = now();
      const payload = {
        clientMessageId: normalizedId,
        threadId: normalizedThreadId,
        promptDigest: normalizedDigest,
        leaseGeneration: normalizedFence.generation,
      };
      const inserted = this.database.prepare(`
        INSERT INTO work_events (work_item_id, kind, payload_json, created_at)
        VALUES (?, 'conversation_turn_dispatch_intended', ?, ?)
        RETURNING id
      `).get(primaryConversationId, JSON.stringify(payload), timestamp);
      this.database.exec("COMMIT;");
      return {
        created: true,
        event: {
          id: inserted.id,
          kind: "conversation_turn_dispatch_intended",
          payload,
          created_at: timestamp,
        },
      };
    } catch (error) {
      this.database.exec("ROLLBACK;");
      throw error;
    }
  }

  bindPrimaryConversationThread({ projectId, threadId, previousThreadId = null, reason, fence }) {
    const normalizedProjectId = requireText(projectId, "project ID");
    const normalizedThreadId = boundedText(threadId, "Codex thread ID", 256);
    const normalizedReason = boundedText(reason, "conversation handoff reason", 128);
    if (!this.getProject(normalizedProjectId)) throw new Error("project not found");
    this.database.exec("BEGIN IMMEDIATE;");
    try {
      const item = this.database.prepare("SELECT * FROM work_items WHERE id = ?")
        .get(primaryConversationId);
      assertPrimaryConversationIdentity(this.database, item);
      assertPrimaryConversationFence(this.database, fence);
      if ((item.codex_thread_id ?? null) !== (previousThreadId ?? null)) {
        const error = new Error("primary conversation binding changed before it could be saved");
        error.code = "CONVERSATION_BINDING_CHANGED";
        error.status = 409;
        throw error;
      }
      const previousBinding = this.latestPrimaryConversationBinding();
      const previousGeneration = previousBinding?.payload.generation ?? 0;
      if (!Number.isInteger(previousGeneration) || previousGeneration < 0) {
        throw new Error("primary conversation binding generation is invalid");
      }
      const generation = previousGeneration + 1;
      const timestamp = now();
      this.database.prepare(`
        UPDATE work_items
        SET project_id = ?, codex_thread_id = ?, codex_turn_id = NULL,
            progress = ?, updated_at = ?
        WHERE id = ?
      `).run(
        normalizedProjectId,
        normalizedThreadId,
        "Codex 對話已連接；準備送出訊息",
        timestamp,
        primaryConversationId,
      );
      const binding = {
        generation,
        projectId: normalizedProjectId,
        fromThreadId: previousThreadId,
        toThreadId: normalizedThreadId,
        reason: normalizedReason,
      };
      this.database.prepare(`
        INSERT INTO work_events (work_item_id, kind, payload_json, created_at)
        VALUES (?, ?, ?, ?)
      `).run(
        primaryConversationId,
        "conversation_thread_bound",
        JSON.stringify(binding),
        timestamp,
      );
      if (previousThreadId && previousThreadId !== normalizedThreadId) {
        this.database.prepare(`
          INSERT INTO work_events (work_item_id, kind, payload_json, created_at)
          VALUES (?, ?, ?, ?)
        `).run(
          primaryConversationId,
          "conversation_thread_handoff",
          JSON.stringify(binding),
          timestamp,
        );
      }
      const updated = this.database.prepare("SELECT * FROM work_items WHERE id = ?")
        .get(primaryConversationId);
      this.database.exec("COMMIT;");
      return decodeItem(updated);
    } catch (error) {
      this.database.exec("ROLLBACK;");
      throw error;
    }
  }

  acceptPrimaryConversationTurn({ clientMessageId, threadId, turnId, fence }) {
    const normalizedId = normalizeClientMessageId(clientMessageId);
    const normalizedThreadId = boundedText(threadId, "Codex thread ID", 256);
    const normalizedTurnId = boundedText(turnId, "Codex turn ID", 256);
    this.database.exec("BEGIN IMMEDIATE;");
    try {
      const item = this.database.prepare("SELECT * FROM work_items WHERE id = ?")
        .get(primaryConversationId);
      assertPrimaryConversationIdentity(this.database, item);
      assertPrimaryConversationFence(this.database, fence);
      if (item.codex_thread_id !== normalizedThreadId || item.status !== "starting") {
        throw new Error("primary conversation changed before its Codex turn was saved");
      }
      const existing = this.primaryConversationAcceptedEvent(normalizedId)?.payload ?? null;
      if (existing) {
        if (existing.threadId !== normalizedThreadId || existing.turnId !== normalizedTurnId) {
          throw new Error("client message is already bound to a different Codex turn");
        }
        this.database.exec("COMMIT;");
        return decodeItem(item);
      }
      const timestamp = now();
      this.database.prepare(`
        UPDATE work_items
        SET codex_turn_id = ?, progress = ?, updated_at = ?
        WHERE id = ? AND status = 'starting' AND codex_thread_id = ?
      `).run(
        normalizedTurnId,
        "Codex 已接受訊息；等待開始回覆",
        timestamp,
        primaryConversationId,
        normalizedThreadId,
      );
      this.database.prepare(`
        INSERT INTO work_events (work_item_id, kind, payload_json, created_at)
        VALUES (?, ?, ?, ?)
      `).run(
        primaryConversationId,
        "conversation_message_accepted",
        JSON.stringify({
          clientMessageId: normalizedId,
          threadId: normalizedThreadId,
          turnId: normalizedTurnId,
        }),
        timestamp,
      );
      const updated = this.database.prepare("SELECT * FROM work_items WHERE id = ?")
        .get(primaryConversationId);
      this.database.exec("COMMIT;");
      return decodeItem(updated);
    } catch (error) {
      this.database.exec("ROLLBACK;");
      throw error;
    }
  }

  recordPrimaryConversationReply({ threadId, turnId, messageId, text, fence }) {
    const normalizedThreadId = boundedText(threadId, "Codex thread ID", 256);
    const normalizedTurnId = boundedText(turnId, "Codex turn ID", 256);
    const normalizedMessageId = boundedText(messageId, "Codex message ID", 256);
    const normalizedText = boundedConversationReply(text);
    this.database.exec("BEGIN IMMEDIATE;");
    try {
      const item = this.database.prepare("SELECT * FROM work_items WHERE id = ?")
        .get(primaryConversationId);
      assertPrimaryConversationIdentity(this.database, item);
      assertPrimaryConversationFence(this.database, fence);
      if (
        item.codex_thread_id !== normalizedThreadId
        || item.codex_turn_id !== normalizedTurnId
      ) {
        this.database.exec("COMMIT;");
        return false;
      }
      const existingRow = this.database.prepare(`
        SELECT payload_json FROM work_events
        WHERE work_item_id = ? AND kind = 'conversation_assistant_message'
          AND json_extract(payload_json, '$.messageId') = ?
        ORDER BY id DESC LIMIT 1
      `).get(primaryConversationId, normalizedMessageId);
      const existing = existingRow ? JSON.parse(existingRow.payload_json) : null;
      if (existing) {
        if (
          existing.threadId !== normalizedThreadId
          || existing.turnId !== normalizedTurnId
          || existing.text !== normalizedText
        ) throw new Error("Codex message identity changed during replay");
        this.database.exec("COMMIT;");
        return false;
      }
      const timestamp = now();
      this.database.prepare(`
        UPDATE work_items SET final_response = ?, progress = ?, updated_at = ?
        WHERE id = ?
      `).run(normalizedText, "已收到 Codex 回覆", timestamp, primaryConversationId);
      this.database.prepare(`
        INSERT INTO work_events (work_item_id, kind, payload_json, created_at)
        VALUES (?, ?, ?, ?)
      `).run(
        primaryConversationId,
        "conversation_assistant_message",
        JSON.stringify({
          messageId: normalizedMessageId,
          threadId: normalizedThreadId,
          turnId: normalizedTurnId,
          text: normalizedText,
        }),
        timestamp,
      );
      this.database.exec("COMMIT;");
      return true;
    } catch (error) {
      this.database.exec("ROLLBACK;");
      throw error;
    }
  }

  recordDecision(input) {
    const normalized = normalizedDecisionRecordInput(input);
    const requestDigest = decisionRequestDigest(normalized);
    this.database.exec("BEGIN IMMEDIATE;");
    try {
      const before = assertDecisionStateIntegrity(this.database);
      const replay = before.rows.find(
        ({ client_request_id: requestId }) => requestId === normalized.clientRequestId,
      );
      if (replay) {
        if (replay.request_digest !== requestDigest) {
          throw decisionError(
            "DECISION_IDEMPOTENCY_CONFLICT",
            "client request ID was already used with a different decision payload",
          );
        }
        this.database.exec("COMMIT;");
        return {
          schema_version: decisionMutationSchemaVersion,
          source: decisionStoreSource,
          changed: false,
          revision: before.state.revision,
          digest: before.state.digest,
          decision: decodeDecision(replay),
        };
      }
      assertExpectedDecisionState(
        before.state,
        normalized.expectedRevision,
        normalized.expectedDigest,
      );

      const current = before.rows.find((row) => (
        row.scope === normalized.scope
        && row.subject === normalized.subject
        && row.status === "current"
      ));
      let predecessor = null;
      if (normalized.supersedesDecisionId == null) {
        if (current) {
          throw decisionError(
            "DECISION_CURRENT_EXISTS",
            "a current decision already exists; explicitly supersede it",
          );
        }
      } else {
        predecessor = before.byId.get(normalized.supersedesDecisionId);
        if (!predecessor) {
          throw decisionError(
            "DECISION_SUPERSESSION_TARGET_NOT_FOUND",
            "superseded decision was not found",
            404,
          );
        }
        if (
          predecessor.scope !== normalized.scope
          || predecessor.subject !== normalized.subject
        ) {
          throw decisionError(
            "DECISION_CROSS_SCOPE_SUPERSESSION_REJECTED",
            "a decision can supersede only the current decision for the same scope and subject",
          );
        }
        if (predecessor.status !== "current" || current?.id !== predecessor.id) {
          throw decisionError(
            "DECISION_SUPERSESSION_TARGET_NOT_CURRENT",
            "only the current decision can be superseded",
          );
        }
      }
      if (before.rows.length >= decisionMaximumRows) {
        throw decisionError("DECISION_STORE_LIMIT_EXCEEDED", "decision store row limit exceeded");
      }

      const previousTimestamp = Math.max(
        new Date(before.state.updated_at).getTime(),
        predecessor == null ? 0 : new Date(predecessor.created_at).getTime(),
      );
      const createdAt = new Date(Math.max(Date.now(), previousTimestamp + 1)).toISOString();
      const id = randomUUID();
      if (predecessor) {
        const superseded = this.database.prepare(`
          UPDATE decisions SET status = 'superseded'
          WHERE id = ? AND status = 'current'
        `).run(predecessor.id);
        if (superseded.changes !== 1) {
          throw decisionError(
            "DECISION_SUPERSESSION_RACE_REJECTED",
            "the decision stopped being current during supersession",
          );
        }
      }
      this.database.prepare(`
        INSERT INTO decisions (
          id, scope, subject, content, rationale, source_kind, source_reference,
          status, supersedes_decision_id, client_request_id, request_digest, created_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, 'current', ?, ?, ?, ?)
      `).run(
        id,
        normalized.scope,
        normalized.subject,
        normalized.content,
        normalized.rationale,
        normalized.source.kind,
        normalized.source.reference,
        normalized.supersedesDecisionId,
        normalized.clientRequestId,
        requestDigest,
        createdAt,
      );
      const after = advanceDecisionState(this.database, before.state, createdAt);
      const recorded = after.byId.get(id);
      this.database.exec("COMMIT;");
      return {
        schema_version: decisionMutationSchemaVersion,
        source: decisionStoreSource,
        changed: true,
        revision: after.state.revision,
        digest: after.state.digest,
        decision: decodeDecision(recorded),
      };
    } catch (error) {
      this.database.exec("ROLLBACK;");
      throw error;
    }
  }

  decisionStateIdentity() {
    for (let attempt = 0; attempt < 2; attempt += 1) {
      const before = this.conversationReadIdentity();
      const state = decisionStateMetadata(this.database);
      const after = this.conversationReadIdentity();
      if (
        before.connection_changes === after.connection_changes
        && before.data_version === after.data_version
      ) {
        return {
          revision: state.revision,
          digest: state.digest,
          connection_changes: after.connection_changes,
          data_version: after.data_version,
        };
      }
    }
    throw decisionError("DECISION_READ_UNSTABLE", "decision state changed repeatedly");
  }

  getCurrentDecisionsPacket(input) {
    const query = normalizedCurrentDecisionQuery(input);
    this.database.exec("BEGIN;");
    try {
      const snapshot = assertDecisionStateIntegrity(this.database);
      const matching = query.subject == null
        ? this.database.prepare(`
          SELECT
            id, scope, subject, content, rationale, source_kind, source_reference,
            status, supersedes_decision_id, client_request_id, request_digest, created_at
          FROM decisions
          WHERE status = 'current' AND scope = ?
          ORDER BY subject ASC, id ASC
          LIMIT ?
        `).all(query.scope, query.limit + 1)
        : this.database.prepare(`
          SELECT
            id, scope, subject, content, rationale, source_kind, source_reference,
            status, supersedes_decision_id, client_request_id, request_digest, created_at
          FROM decisions
          WHERE status = 'current' AND scope = ? AND subject = ?
          ORDER BY subject ASC, id ASC
          LIMIT ?
        `).all(query.scope, query.subject, query.limit + 1);
      const packet = {
        schema_version: currentDecisionsPacketSchemaVersion,
        source: decisionStoreSource,
        scope: query.scope,
        subject: query.subject,
        revision: snapshot.state.revision,
        digest: snapshot.state.digest,
        decisions: matching.slice(0, query.limit).map(decodeDecision),
        truncated: matching.length > query.limit,
      };
      if (Buffer.byteLength(JSON.stringify(packet), "utf8") > 262_144) {
        throw decisionError(
          "DECISION_OUTPUT_LIMIT_EXCEEDED",
          "current decisions packet exceeds the output bound",
        );
      }
      this.database.exec("COMMIT;");
      return packet;
    } catch (error) {
      this.database.exec("ROLLBACK;");
      throw error;
    }
  }

  readDecision(input) {
    const query = normalizedDecisionReadQuery(input);
    this.database.exec("BEGIN;");
    try {
      const snapshot = assertDecisionStateIntegrity(this.database);
      assertExpectedDecisionState(
        snapshot.state,
        query.expectedRevision,
        query.expectedDigest,
      );
      const target = snapshot.byId.get(query.decisionId);
      if (!target) {
        throw decisionError("DECISION_NOT_FOUND", "decision was not found", 404);
      }
      const group = snapshot.rows.filter((row) => (
        row.scope === target.scope && row.subject === target.subject
      ));
      let cursor = group.find(({ supersedes_decision_id: parentId }) => parentId == null);
      const lineage = [];
      while (cursor) {
        lineage.push(cursor);
        cursor = snapshot.childByParent.get(cursor.id) ?? null;
      }
      const targetIndex = lineage.findIndex(({ id }) => id === target.id);
      if (targetIndex < 0) {
        throw decisionError("DECISION_LINEAGE_DISCONNECTED", "decision lineage is disconnected");
      }
      const start = Math.min(targetIndex, Math.max(0, lineage.length - query.maxDepth));
      const end = Math.min(lineage.length, start + query.maxDepth);
      const result = {
        schema_version: decisionReadSchemaVersion,
        source: decisionStoreSource,
        revision: snapshot.state.revision,
        digest: snapshot.state.digest,
        decision: decodeDecision(target),
        lineage: lineage.slice(start, end).map(decodeDecision),
        truncated_before: start > 0,
        truncated_after: end < lineage.length,
      };
      if (Buffer.byteLength(JSON.stringify(result), "utf8") > 524_288) {
        throw decisionError("DECISION_OUTPUT_LIMIT_EXCEEDED", "decision read exceeds the output bound");
      }
      this.database.exec("COMMIT;");
      return result;
    } catch (error) {
      this.database.exec("ROLLBACK;");
      throw error;
    }
  }

  searchDecisions(input) {
    const query = normalizedDecisionSearchQuery(input);
    this.database.exec("BEGIN;");
    try {
      const snapshot = assertDecisionStateIntegrity(this.database);
      assertExpectedDecisionState(
        snapshot.state,
        query.expectedRevision,
        query.expectedDigest,
      );
      const needle = query.query.toLocaleLowerCase("en-US");
      const matches = snapshot.rows.filter((row) => (
        row.scope === query.scope
        && [
          row.subject,
          row.content,
          row.rationale,
          row.source_reference,
        ].some((value) => value.toLocaleLowerCase("en-US").includes(needle))
      )).sort((left, right) => (
        right.created_at.localeCompare(left.created_at)
        || right.id.localeCompare(left.id)
      ));
      const result = {
        schema_version: decisionSearchSchemaVersion,
        source: decisionStoreSource,
        scope: query.scope,
        query: query.query,
        revision: snapshot.state.revision,
        digest: snapshot.state.digest,
        decisions: matches.slice(0, query.limit).map(decodeDecision),
        truncated: matches.length > query.limit,
      };
      if (Buffer.byteLength(JSON.stringify(result), "utf8") > 196_608) {
        throw decisionError("DECISION_OUTPUT_LIMIT_EXCEEDED", "decision search exceeds the output bound");
      }
      this.database.exec("COMMIT;");
      return result;
    } catch (error) {
      this.database.exec("ROLLBACK;");
      throw error;
    }
  }

  setWorkRelations({
    projectId,
    workItemId,
    parentId = null,
    dependsOn = [],
    blocker = { status: "clear" },
    expectedRevision,
    expectedDigest,
  }) {
    const normalizedProjectId = normalizedWorkCoreId(projectId, "project ID");
    const normalizedWorkItemId = normalizedWorkCoreId(workItemId, "work item ID");
    const normalizedParentId = parentId == null
      ? null
      : normalizedWorkCoreId(parentId, "parent work item ID");
    if (!Array.isArray(dependsOn) || dependsOn.length > controlWorkMaximumDependencies) {
      throw controlWorkError(
        "CONTROL_WORK_DEPENDENCY_LIMIT_REJECTED",
        `dependsOn must contain at most ${controlWorkMaximumDependencies} work item IDs`,
        400,
      );
    }
    const normalizedDependencies = dependsOn
      .map((id) => normalizedWorkCoreId(id, "dependency work item ID"))
      .sort();
    if (new Set(normalizedDependencies).size !== normalizedDependencies.length) {
      throw controlWorkError(
        "CONTROL_WORK_DUPLICATE_DEPENDENCY_REJECTED",
        "duplicate dependency edges are not allowed",
        400,
      );
    }
    if (normalizedParentId === normalizedWorkItemId) {
      throw controlWorkError(
        "CONTROL_WORK_SELF_PARENT_REJECTED",
        "a work item cannot be its own parent",
        400,
      );
    }
    if (normalizedDependencies.includes(normalizedWorkItemId)) {
      throw controlWorkError(
        "CONTROL_WORK_SELF_DEPENDENCY_REJECTED",
        "a work item cannot depend on itself",
        400,
      );
    }
    const normalizedBlockerValue = normalizedBlocker(blocker);
    const normalizedExpectedRevision = normalizedWorkDigest(expectedRevision, "expected revision");
    const normalizedExpectedDigest = normalizedWorkDigest(expectedDigest, "expected digest");

    this.database.exec("BEGIN IMMEDIATE;");
    try {
      const before = readControlWorkSnapshot(this.database, {
        projectId: normalizedProjectId,
        maxNodes: controlWorkMaximumNodes,
        maxEdges: controlWorkMaximumEdges,
      });
      const target = this.database.prepare(
        "SELECT project_id FROM work_items WHERE id = ?",
      ).get(normalizedWorkItemId);
      if (!target) {
        throw controlWorkError(
          "CONTROL_WORK_NODE_NOT_FOUND",
          `Control work item ${normalizedWorkItemId} was not found`,
          404,
        );
      }
      if (target.project_id !== normalizedProjectId) {
        throw controlWorkError(
          "CONTROL_WORK_CROSS_PROJECT_REJECTED",
          "work relations cannot cross Control projects",
        );
      }
      const currentRelation = this.database.prepare(`
        SELECT parent_work_item_id, blocker_status, blocker_reason
        FROM work_item_relations WHERE work_item_id = ?
      `).get(normalizedWorkItemId) ?? {
        parent_work_item_id: null,
        blocker_status: "clear",
        blocker_reason: null,
      };
      const currentDependencies = this.database.prepare(`
        SELECT depends_on_work_item_id
        FROM work_item_dependencies
        WHERE work_item_id = ?
        ORDER BY depends_on_work_item_id ASC
      `).all(normalizedWorkItemId).map(({ depends_on_work_item_id: id }) => id);
      if (
        before.revision !== normalizedExpectedRevision
        || before.digest !== normalizedExpectedDigest
      ) {
        throw controlWorkError(
          "CONTROL_WORK_REVISION_MISMATCH",
          "Control work state changed before the relation mutation",
        );
      }
      const alreadyApplied = currentRelation.parent_work_item_id === normalizedParentId
        && currentRelation.blocker_status === normalizedBlockerValue.status
        && currentRelation.blocker_reason === normalizedBlockerValue.reason
        && JSON.stringify(currentDependencies) === JSON.stringify(normalizedDependencies);
      if (alreadyApplied) {
        this.database.exec("COMMIT;");
        return { changed: false, snapshot: before };
      }

      const referencedIds = [normalizedWorkItemId, normalizedParentId, ...normalizedDependencies]
        .filter((id) => id != null);
      for (const id of referencedIds) {
        const item = this.database.prepare(
          "SELECT project_id FROM work_items WHERE id = ?",
        ).get(id);
        if (!item) {
          throw controlWorkError(
            "CONTROL_WORK_NODE_NOT_FOUND",
            `Control work item ${id} was not found`,
            404,
          );
        }
        if (item.project_id !== normalizedProjectId) {
          throw controlWorkError(
            "CONTROL_WORK_CROSS_PROJECT_REJECTED",
            "work relations cannot cross Control projects",
          );
        }
      }

      if (normalizedParentId == null && normalizedBlockerValue.status === "clear") {
        this.database.prepare(
          "DELETE FROM work_item_relations WHERE work_item_id = ?",
        ).run(normalizedWorkItemId);
      } else {
        this.database.prepare(`
          INSERT INTO work_item_relations (
            work_item_id, parent_work_item_id, blocker_status, blocker_reason
          ) VALUES (?, ?, ?, ?)
          ON CONFLICT(work_item_id) DO UPDATE SET
            parent_work_item_id = excluded.parent_work_item_id,
            blocker_status = excluded.blocker_status,
            blocker_reason = excluded.blocker_reason
        `).run(
          normalizedWorkItemId,
          normalizedParentId,
          normalizedBlockerValue.status,
          normalizedBlockerValue.reason,
        );
      }
      this.database.prepare(
        "DELETE FROM work_item_dependencies WHERE work_item_id = ?",
      ).run(normalizedWorkItemId);
      const insertDependency = this.database.prepare(`
        INSERT INTO work_item_dependencies (
          work_item_id, depends_on_work_item_id, created_at
        ) VALUES (?, ?, ?)
      `);
      const timestamp = now();
      for (const dependencyId of normalizedDependencies) {
        insertDependency.run(normalizedWorkItemId, dependencyId, timestamp);
      }

      const after = readControlWorkSnapshot(this.database, {
        projectId: normalizedProjectId,
        maxNodes: controlWorkMaximumNodes,
        maxEdges: controlWorkMaximumEdges,
      });
      this.database.prepare(`
        INSERT INTO work_events (work_item_id, kind, payload_json, created_at)
        VALUES (?, 'work_relations_set', ?, ?)
      `).run(
        normalizedWorkItemId,
        JSON.stringify({
          parentId: normalizedParentId,
          dependsOn: normalizedDependencies,
          blocker: normalizedBlockerValue,
          revision: after.revision,
          digest: after.digest,
        }),
        timestamp,
      );
      this.database.exec("COMMIT;");
      return { changed: true, snapshot: after };
    } catch (error) {
      this.database.exec("ROLLBACK;");
      throw error;
    }
  }

  getWorkSnapshot(input) {
    this.database.exec("BEGIN;");
    try {
      const snapshot = readControlWorkSnapshot(this.database, input);
      this.database.exec("COMMIT;");
      return snapshot;
    } catch (error) {
      this.database.exec("ROLLBACK;");
      throw error;
    }
  }

  getWorkNode({
    projectId,
    workItemId,
    expectedRevision,
    expectedDigest,
    maxNodes,
    maxEdges,
  }) {
    const normalizedWorkItemId = normalizedWorkCoreId(workItemId, "work item ID");
    const normalizedExpectedRevision = normalizedWorkDigest(expectedRevision, "expected revision");
    const normalizedExpectedDigest = normalizedWorkDigest(expectedDigest, "expected digest");
    const snapshot = this.getWorkSnapshot({ projectId, maxNodes, maxEdges });
    if (
      snapshot.revision !== normalizedExpectedRevision
      || snapshot.digest !== normalizedExpectedDigest
    ) {
      throw controlWorkError(
        "CONTROL_WORK_REVISION_MISMATCH",
        "Control work state changed before the node read",
      );
    }
    const treeNode = snapshot.tree.nodes.find(({ id }) => id === normalizedWorkItemId);
    const graphNode = snapshot.graph.nodes.find(({ id }) => id === normalizedWorkItemId);
    if (!treeNode || !graphNode) {
      throw controlWorkError("CONTROL_WORK_NODE_NOT_FOUND", "Control work item not found", 404);
    }
    return {
      schema_version: controlWorkNodeSchemaVersion,
      source: snapshot.source,
      project_id: snapshot.project_id,
      revision: snapshot.revision,
      digest: snapshot.digest,
      tree_node: treeNode,
      graph_node: graphNode,
    };
  }

  createWorkItem({ projectId, title, objective, priority = "normal" }) {
    if (!this.getProject(projectId)) throw new Error("project not found");
    if (!priorities.has(priority)) throw new TypeError("invalid priority");
    const timestamp = now();
    const item = {
      id: randomUUID(),
      project_id: projectId,
      title: requireText(title, "work item title"),
      objective: requireText(objective, "work item objective"),
      priority,
      status: "draft",
      created_at: timestamp,
      updated_at: timestamp,
    };
    this.database.prepare(`
      INSERT INTO work_items (
        id, project_id, title, objective, priority, status, created_at, updated_at
      ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
    `).run(
      item.id,
      item.project_id,
      item.title,
      item.objective,
      item.priority,
      item.status,
      timestamp,
      timestamp,
    );
    this.appendEvent(item.id, "created", { priority });
    return this.getWorkItem(item.id);
  }

  getWorkItem(id) {
    return decodeItem(this.database.prepare("SELECT * FROM work_items WHERE id = ?").get(id));
  }

  listWorkItems() {
    return this.database.prepare(`
      SELECT work_items.*, projects.name AS project_name, projects.root_path AS project_root
      FROM work_items
      JOIN projects ON projects.id = work_items.project_id
      ORDER BY
        CASE priority WHEN 'urgent' THEN 0 WHEN 'high' THEN 1 WHEN 'normal' THEN 2 ELSE 3 END,
        work_items.created_at DESC
    `).all().map(decodeItem);
  }

  runtimeDataPresence() {
    const row = this.database.prepare(`
      SELECT
        EXISTS(SELECT 1 FROM work_items WHERE id <> 'primary' LIMIT 1) AS work_data,
        EXISTS(SELECT 1 FROM decisions LIMIT 1) AS decision_data
    `).get();
    return {
      work: row.work_data === 1,
      decisions: row.decision_data === 1,
    };
  }

  updateWorkItem(id, changes, fence = null) {
    const entries = workItemChangeEntries(changes);
    if (entries.length === 0) return this.getWorkItem(id);
    entries.push(["updated_at", now()]);
    const primary = id === primaryConversationId;
    if (primary) this.database.exec("BEGIN IMMEDIATE;");
    try {
      if (primary) assertPrimaryConversationFence(this.database, fence);
      const result = this.database.prepare(`
        UPDATE work_items
        SET ${entries.map(([key]) => `${key} = ?`).join(", ")}
        WHERE id = ?
      `).run(...entries.map(([, value]) => value), id);
      if (result.changes !== 1) throw new Error("work item not found");
      const updated = this.getWorkItem(id);
      if (primary) this.database.exec("COMMIT;");
      return updated;
    } catch (error) {
      if (primary) this.database.exec("ROLLBACK;");
      throw error;
    }
  }

  transitionWorkItem(id, fromStatuses, changes, fence = null) {
    if (!Array.isArray(fromStatuses) || fromStatuses.length === 0) {
      throw new TypeError("at least one source status is required");
    }
    if (fromStatuses.some((status) => !statuses.has(status))) {
      throw new TypeError("invalid source status");
    }
    const entries = workItemChangeEntries(changes);
    if (entries.length === 0) throw new TypeError("at least one work item change is required");
    entries.push(["updated_at", now()]);
    const primary = id === primaryConversationId;
    if (primary) this.database.exec("BEGIN IMMEDIATE;");
    try {
      if (primary) assertPrimaryConversationFence(this.database, fence);
      const result = this.database.prepare(`
        UPDATE work_items
        SET ${entries.map(([key]) => `${key} = ?`).join(", ")}
        WHERE id = ? AND status IN (${fromStatuses.map(() => "?").join(", ")})
      `).run(...entries.map(([, value]) => value), id, ...fromStatuses);
      const updated = result.changes === 1 ? this.getWorkItem(id) : null;
      if (primary) this.database.exec("COMMIT;");
      return updated;
    } catch (error) {
      if (primary) this.database.exec("ROLLBACK;");
      throw error;
    }
  }

  appendEvent(workItemId, kind, payload = {}, fence = null) {
    const primary = workItemId === primaryConversationId;
    if (primary) this.database.exec("BEGIN IMMEDIATE;");
    try {
      if (primary) assertPrimaryConversationFence(this.database, fence);
      this.database.prepare(`
        INSERT INTO work_events (work_item_id, kind, payload_json, created_at)
        VALUES (?, ?, ?, ?)
      `).run(workItemId, kind, JSON.stringify(payload), now());
      if (primary) this.database.exec("COMMIT;");
    } catch (error) {
      if (primary) this.database.exec("ROLLBACK;");
      throw error;
    }
  }

  hasEventPayload(workItemId, kind, payload) {
    if (primaryConversationIdempotentKindSet.has(kind)) {
      return Boolean(this.database.prepare(`
        SELECT 1 AS found FROM work_events
        WHERE work_item_id = ? AND kind = ? AND payload_json = ?
          AND kind IN (${primaryConversationIdempotentKindsSql})
          AND length(CAST(payload_json AS BLOB)) <= 16384
        LIMIT 1
      `).get(workItemId, kind, JSON.stringify(payload))?.found);
    }
    return Boolean(this.database.prepare(`
      SELECT 1 AS found FROM work_events
      WHERE work_item_id = ? AND kind = ? AND payload_json = ?
      LIMIT 1
    `).get(workItemId, kind, JSON.stringify(payload))?.found);
  }

  hasEventKind(workItemId, kind) {
    return Boolean(this.database.prepare(`
      SELECT 1 AS found FROM work_events
      WHERE work_item_id = ? AND kind = ? LIMIT 1
    `).get(workItemId, kind)?.found);
  }

  hasConfirmedStartEvent(workItemId, threadId, turnId) {
    return Boolean(this.database.prepare(`
      SELECT 1 AS found FROM work_events
      WHERE work_item_id = ? AND kind = 'codex_started'
        AND json_extract(payload_json, '$.threadId') = ?
        AND json_extract(payload_json, '$.turnId') = ?
        AND json_extract(payload_json, '$.confirmedBy')
          IN ('turn/started', 'thread/resume', 'marker-thread/read')
      LIMIT 1
    `).get(workItemId, threadId, turnId)?.found);
  }

  hasConversationAssistantMessage(threadId, turnId) {
    return Boolean(this.database.prepare(`
      SELECT 1 AS found FROM work_events
      WHERE work_item_id = ? AND kind = 'conversation_assistant_message'
        AND json_extract(payload_json, '$.threadId') = ?
        AND json_extract(payload_json, '$.turnId') = ?
      LIMIT 1
    `).get(primaryConversationId, threadId, turnId)?.found);
  }

  turnCompletedEvent(workItemId, threadId, turnId) {
    return decodeWorkEvent(this.database.prepare(`
      SELECT id, kind, payload_json, created_at
      FROM work_events
      WHERE work_item_id = ? AND kind = 'turn_completed'
        AND json_extract(payload_json, '$.threadId') = ?
        AND json_extract(payload_json, '$.turnId') = ?
      ORDER BY id ASC LIMIT 1
    `).get(workItemId, threadId, turnId));
  }

  listEvents(workItemId) {
    return this.database.prepare(`
      SELECT id, kind, payload_json, created_at
      FROM work_events WHERE work_item_id = ? ORDER BY id ASC
    `).all(workItemId).map(decodeWorkEvent);
  }

  conversationReadIdentity() {
    for (let attempt = 0; attempt < 2; attempt += 1) {
      const before = this.database.prepare("PRAGMA data_version").get().data_version;
      const connectionChanges = this.database.prepare("SELECT total_changes() AS value").get().value;
      const after = this.database.prepare("PRAGMA data_version").get().data_version;
      if (before === after) {
        return { connection_changes: connectionChanges, data_version: after };
      }
    }
    const error = new Error("conversation read identity changed repeatedly");
    error.code = "CONVERSATION_READ_UNSTABLE";
    throw error;
  }

  primaryConversationWindow({
    maximumMessages,
    maximumMessageBytes,
    maximumHandoffs,
    maximumHandoffBytes,
  }) {
    for (const [value, maximum, label] of [
      [maximumMessages, 64, "message count"],
      [maximumMessageBytes, 524_288, "message bytes"],
      [maximumHandoffs, 32, "handoff count"],
      [maximumHandoffBytes, 65_536, "handoff bytes"],
    ]) {
      if (!Number.isInteger(value) || value < 1 || value > maximum) {
        throw new Error(`conversation ${label} limit is invalid`);
      }
    }
    const readSizedWindow = ({ kinds, maximumItems, maximumBytes, minimumId = null }) => {
      const candidateLimit = maximumItems + 1;
      const minimumClause = minimumId === null ? "" : "AND id >= ?";
      const perKindSql = kinds.map(() => `
        SELECT id, kind, payload_json, created_at FROM (
          SELECT id, kind, payload_json, created_at
          FROM work_events
          WHERE work_item_id = ? AND kind = ? ${minimumClause}
          ORDER BY id DESC LIMIT ?
        )
      `).join(" UNION ALL ");
      const candidateArguments = kinds.flatMap((kind) => (
        minimumId === null
          ? [primaryConversationId, kind, candidateLimit]
          : [primaryConversationId, kind, minimumId, candidateLimit]
      ));
      const candidateCount = Number(this.database.prepare(`
        SELECT COUNT(*) AS count FROM (
          SELECT id FROM (${perKindSql})
          ORDER BY id DESC LIMIT ?
        )
      `).get(...candidateArguments, candidateLimit)?.count ?? 0);
      const rows = this.database.prepare(`
        WITH candidates AS (
          SELECT id, kind, payload_json, created_at FROM (${perKindSql})
          ORDER BY id DESC LIMIT ?
        ), sized AS (
          SELECT id, kind, payload_json, created_at,
            ROW_NUMBER() OVER (ORDER BY id DESC) AS row_number,
            SUM(length(CAST(payload_json AS BLOB)) + 64) OVER (
              ORDER BY id DESC ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW
            ) AS cumulative_bytes
          FROM candidates
        )
        SELECT id, kind, payload_json, created_at
        FROM sized
        WHERE row_number <= ? AND cumulative_bytes <= ?
        ORDER BY id ASC
      `).all(
        ...candidateArguments,
        candidateLimit,
        maximumItems,
        maximumBytes,
      );
      return {
        rows,
        truncated: candidateCount > rows.length,
      };
    };
    const messageWindow = readSizedWindow({
      kinds: ["conversation_message_claimed", "conversation_assistant_message"],
      maximumItems: maximumMessages,
      maximumBytes: maximumMessageBytes,
    });
    const visibleClientMessageIds = messageWindow.rows
      .filter(({ kind }) => kind === "conversation_message_claimed")
      .map(({ payload_json: payloadJson }) => JSON.parse(payloadJson).clientMessageId);
    const supportRows = [];
    const supportIncompleteClientMessageIds = new Set();
    const supportStatement = this.database.prepare(`
      SELECT id, kind,
        CASE WHEN length(CAST(payload_json AS BLOB)) <= 16384 THEN payload_json ELSE NULL END
          AS payload_json,
        created_at,
        length(CAST(payload_json AS BLOB)) AS payload_bytes
      FROM work_events
      WHERE work_item_id = ? AND kind = ?
        AND json_extract(payload_json, '$.clientMessageId') = ?
      ORDER BY id DESC LIMIT 1
    `);
    for (const clientMessageId of visibleClientMessageIds) {
      for (const kind of ["conversation_message_accepted", "conversation_message_failed"]) {
        const row = supportStatement.get(primaryConversationId, kind, clientMessageId);
        if (!row) continue;
        if (row.payload_json === null) supportIncompleteClientMessageIds.add(clientMessageId);
        else {
          delete row.payload_bytes;
          supportRows.push(row);
        }
      }
    }
    const handoffWindow = readSizedWindow({
      kinds: ["conversation_thread_handoff"],
      maximumItems: maximumHandoffs,
      maximumBytes: maximumHandoffBytes,
    });
    const rows = [...messageWindow.rows, ...supportRows, ...handoffWindow.rows]
      .sort((left, right) => left.id - right.id);
    return {
      events: rows.map((event) => ({
        id: event.id,
        kind: event.kind,
        payload: JSON.parse(event.payload_json),
        created_at: event.created_at,
      })),
      messages_truncated: messageWindow.truncated || supportIncompleteClientMessageIds.size > 0,
      handoffs_truncated: handoffWindow.truncated,
      support_incomplete_client_message_ids: [...supportIncompleteClientMessageIds],
    };
  }
}
