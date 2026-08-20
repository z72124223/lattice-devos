import { randomUUID } from "node:crypto";
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

function now() {
  return new Date().toISOString();
}

function requireText(value, label) {
  if (typeof value !== "string" || !value.trim()) {
    throw new TypeError(`${label} is required`);
  }
  return value.trim();
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
    `);
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
