import { createHash, randomUUID } from "node:crypto";
import { mkdirSync } from "node:fs";
import path from "node:path";
import { DatabaseSync } from "node:sqlite";

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
const controlSchemaVersion = 4;
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

const schemaColumns = new Map([
  ["projects", ["id", "name", "root_path", "created_at", "updated_at"]],
  ["work_items", [
    "id", "project_id", "title", "objective", "priority", "status", "codex_thread_id",
    "codex_turn_id", "progress", "approval_json", "final_response", "verification_notes",
    "failure_summary", "archived_at", "created_at", "updated_at",
  ]],
  ["work_events", ["id", "work_item_id", "kind", "payload_json", "created_at"]],
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
]);

const schemaForeignKeys = new Map([
  ["work_items", [["project_id", "projects", "id", "CASCADE"]]],
  ["work_events", [["work_item_id", "work_items", "id", "CASCADE"]]],
  ["conversation_writer_leases", [["conversation_id", "work_items", "id", "CASCADE"]]],
  ["installation_receipts", [["project_id", "projects", "id", "NO ACTION"]]],
  ["project_registration_details", [["project_id", "projects", "id", "CASCADE"]]],
  ["project_observations", [["project_id", "projects", "id", "CASCADE"]]],
  ["project_git_remotes", [["observation_id", "project_observations", "id", "CASCADE"]]],
  ["project_rule_documents", [["observation_id", "project_observations", "id", "CASCADE"]]],
]);

function quotedIdentifier(value) {
  return `"${value.replaceAll('"', '""')}"`;
}

function schemaProfileFailure(detail) {
  throw new Error(`Control database schema profile mismatch: ${detail}`);
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
    if (validateProfile) validateControlSchemaProfile(database);
    database.exec(`PRAGMA user_version = ${controlSchemaVersion};`);
    database.exec("COMMIT;");
  } catch (error) {
    database.exec("ROLLBACK;");
    throw error;
  }
}

function migrateControlDatabaseV1ToV4(database) {
  database.exec("BEGIN IMMEDIATE;");
  try {
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
      PRAGMA user_version = 4;
    `);
    validateControlSchemaProfile(database);
    database.exec("COMMIT;");
  } catch (error) {
    database.exec("ROLLBACK;");
    throw error;
  }
}

function migrateControlDatabaseV2ToV4(database) {
  database.exec("BEGIN IMMEDIATE;");
  try {
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
      PRAGMA user_version = 4;
    `);
    validateControlSchemaProfile(database);
    database.exec("COMMIT;");
  } catch (error) {
    database.exec("ROLLBACK;");
    throw error;
  }
}

function migrateControlDatabaseV3ToV4(database) {
  database.exec("BEGIN IMMEDIATE;");
  try {
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
      PRAGMA user_version = 4;
    `);
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

export class LatticeStore {
  constructor(databasePath = ":memory:") {
    if (databasePath !== ":memory:") {
      mkdirSync(path.dirname(path.resolve(databasePath)), { recursive: true });
    }
    const database = new DatabaseSync(databasePath);
    try {
      const version = database.prepare("PRAGMA user_version").get().user_version;
      if (![0, 1, 2, 3, controlSchemaVersion].includes(version)) {
        throw new Error(
          `Control database schema ${version} is unsupported; expected 0, 1, 2, 3, or ${controlSchemaVersion}`,
        );
      }
      if (version === 0) initializeControlDatabase(database);
      else if (version === 1) migrateControlDatabaseV1ToV4(database);
      else if (version === 2) migrateControlDatabaseV2ToV4(database);
      else if (version === 3) migrateControlDatabaseV3ToV4(database);
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

  ensurePrimaryConversation(projectId) {
    const normalizedProjectId = requireText(projectId, "project ID");
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
      this.database.exec("COMMIT;");
      return decodeItem(item);
    } catch (error) {
      this.database.exec("ROLLBACK;");
      throw error;
    }
  }

  primaryConversationMessage(clientMessageId) {
    const normalizedId = normalizeClientMessageId(clientMessageId);
    return this.listEvents(primaryConversationId).find(({ kind, payload }) => (
      kind === "conversation_message_claimed"
      && payload.clientMessageId === normalizedId
    )) ?? null;
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
      const claimedMessages = this.database.prepare(`
        SELECT id, kind, payload_json, created_at
        FROM work_events
        WHERE work_item_id = ? AND kind = ?
        ORDER BY id ASC
      `).all(primaryConversationId, "conversation_message_claimed")
        .map((event) => ({
          id: event.id,
          kind: event.kind,
          payload: JSON.parse(event.payload_json),
          created_at: event.created_at,
        }));
      const acceptedMessageIds = new Set(this.database.prepare(`
        SELECT payload_json FROM work_events
        WHERE work_item_id = ? AND kind = 'conversation_message_accepted'
      `).all(primaryConversationId).map(({ payload_json: payloadJson }) => (
        JSON.parse(payloadJson).clientMessageId
      )));
      const existing = claimedMessages
        .find(({ payload }) => payload.clientMessageId === normalizedId);
      const unresolvedMessages = claimedMessages.filter(
        ({ payload }) => !acceptedMessageIds.has(payload.clientMessageId),
      );
      if (
        unresolvedMessages.length > 0
        && (!existing || unresolvedMessages.some(
          ({ payload }) => payload.clientMessageId !== normalizedId,
        ))
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
        const accepted = acceptedMessageIds.has(normalizedId);
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
    return this.listEvents(primaryConversationId).find(({ kind, payload }) => (
      kind === "conversation_turn_dispatch_intended"
      && payload.clientMessageId === normalizedId
    )) ?? null;
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
      const claimed = this.database.prepare(`
        SELECT payload_json FROM work_events
        WHERE work_item_id = ? AND kind = 'conversation_message_claimed'
        ORDER BY id ASC
      `).all(primaryConversationId)
        .map(({ payload_json: payloadJson }) => JSON.parse(payloadJson))
        .find((payload) => payload.clientMessageId === normalizedId);
      if (!claimed || claimed.promptDigest !== normalizedDigest) {
        throw new Error("turn dispatch intent has no exact saved message claim");
      }
      const intents = this.database.prepare(`
        SELECT id, payload_json, created_at FROM work_events
        WHERE work_item_id = ? AND kind = 'conversation_turn_dispatch_intended'
        ORDER BY id ASC
      `).all(primaryConversationId);
      const existing = intents
        .map((event) => ({ ...event, payload: JSON.parse(event.payload_json) }))
        .find(({ payload }) => payload.clientMessageId === normalizedId);
      if (existing) {
        if (
          existing.payload.threadId !== normalizedThreadId
          || existing.payload.promptDigest !== normalizedDigest
        ) throw new Error("saved turn dispatch intent changed identity");
        this.database.exec("COMMIT;");
        return {
          created: false,
          event: {
            id: existing.id,
            kind: "conversation_turn_dispatch_intended",
            payload: existing.payload,
            created_at: existing.created_at,
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
      const generation = this.database.prepare(`
        SELECT COUNT(*) AS count FROM work_events
        WHERE work_item_id = ? AND kind = ?
      `).get(primaryConversationId, "conversation_thread_bound").count + 1;
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
      const events = this.database.prepare(`
        SELECT payload_json FROM work_events
        WHERE work_item_id = ? AND kind = ?
        ORDER BY id ASC
      `).all(primaryConversationId, "conversation_message_accepted")
        .map(({ payload_json: payloadJson }) => JSON.parse(payloadJson));
      const existing = events.find((payload) => payload.clientMessageId === normalizedId);
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
      const existing = this.database.prepare(`
        SELECT payload_json FROM work_events
        WHERE work_item_id = ? AND kind = ?
        ORDER BY id ASC
      `).all(primaryConversationId, "conversation_assistant_message")
        .map(({ payload_json: payloadJson }) => JSON.parse(payloadJson))
        .find((payload) => payload.messageId === normalizedMessageId);
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

  listEvents(workItemId) {
    return this.database.prepare(`
      SELECT id, kind, payload_json, created_at
      FROM work_events WHERE work_item_id = ? ORDER BY id ASC
    `).all(workItemId).map((event) => ({
      id: event.id,
      kind: event.kind,
      payload: JSON.parse(event.payload_json),
      created_at: event.created_at,
    }));
  }
}
