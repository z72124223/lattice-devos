import { createHash, randomUUID } from "node:crypto";
import { mkdirSync } from "node:fs";
import path from "node:path";
import { DatabaseSync } from "node:sqlite";

const priorities = new Set(["low", "normal", "high", "urgent"]);
const statuses = new Set([
  "draft",
  "running",
  "waiting_approval",
  "codex_done",
  "verified",
  "failed",
  "archived",
]);
const installationReceiptSchemaVersion = "lattice.control.installation-receipt.v1";
const installationObservationKind = "OBSERVED_AFTER_INSTALL";
const installationReceiptAuthority = "NON_AUTHORITATIVE";

function now() {
  return new Date().toISOString();
}

function requireText(value, label) {
  if (typeof value !== "string" || !value.trim()) {
    throw new TypeError(`${label} is required`);
  }
  return value.trim();
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

export class LatticeStore {
  constructor(databasePath = ":memory:") {
    if (databasePath !== ":memory:") {
      mkdirSync(path.dirname(path.resolve(databasePath)), { recursive: true });
    }
    this.database = new DatabaseSync(databasePath);
    this.database.exec("PRAGMA foreign_keys = ON;");
    if (databasePath !== ":memory:") this.database.exec("PRAGMA journal_mode = WAL;");
    this.database.exec(`
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
    `);
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

  close() {
    this.database.close();
  }

  createProject({ name, rootPath }) {
    const project = {
      id: randomUUID(),
      name: requireText(name, "project name"),
      root_path: path.resolve(requireText(rootPath, "project root")),
      created_at: now(),
    };
    this.database.prepare(`
      INSERT INTO projects (id, name, root_path, created_at, updated_at)
      VALUES (?, ?, ?, ?, ?)
    `).run(project.id, project.name, project.root_path, project.created_at, project.created_at);
    return project;
  }

  listProjects() {
    return this.database.prepare("SELECT * FROM projects ORDER BY created_at DESC").all();
  }

  getProject(id) {
    return this.database.prepare("SELECT * FROM projects WHERE id = ?").get(id) ?? null;
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

  updateWorkItem(id, changes) {
    const allowed = new Set([
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
    const entries = Object.entries(changes).filter(([key]) => allowed.has(key));
    if (entries.length === 0) return this.getWorkItem(id);
    if (changes.status && !statuses.has(changes.status)) throw new TypeError("invalid status");
    entries.push(["updated_at", now()]);
    const result = this.database.prepare(`
      UPDATE work_items
      SET ${entries.map(([key]) => `${key} = ?`).join(", ")}
      WHERE id = ?
    `).run(...entries.map(([, value]) => value), id);
    if (result.changes !== 1) throw new Error("work item not found");
    return this.getWorkItem(id);
  }

  appendEvent(workItemId, kind, payload = {}) {
    this.database.prepare(`
      INSERT INTO work_events (work_item_id, kind, payload_json, created_at)
      VALUES (?, ?, ?, ?)
    `).run(workItemId, kind, JSON.stringify(payload), now());
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
