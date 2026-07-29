import { randomUUID } from "node:crypto";
import {
  lstat,
  mkdir,
  open,
  readFile,
  realpath,
  rmdir,
  unlink,
} from "node:fs/promises";
import path from "node:path";

import { deepFreeze } from "../domain/canonical-json.js";
import { WorkspaceError, workspaceFailure } from "./errors.js";

const HASH_PATTERN = /^[a-f0-9]{64}$/;
const TASK_ID_PATTERN = /^TASK-[A-Z0-9][A-Z0-9_-]{2,63}$/;
const PROJECT_ID_PATTERN = /^[a-z0-9][a-z0-9._-]{1,63}$/;

async function optionalLstat(file) {
  try {
    return await lstat(file);
  } catch (error) {
    if (error?.code === "ENOENT") {
      return null;
    }
    throw error;
  }
}

async function durableWrite(file, content, flag = "w") {
  const handle = await open(file, flag);
  try {
    await handle.writeFile(content, "utf8");
    await handle.sync();
  } finally {
    await handle.close();
  }
}

function containedBy(parent, candidate) {
  const relative = path.relative(parent, candidate);
  return relative === "" || (!relative.startsWith("..") && !path.isAbsolute(relative));
}

function samePath(left, right) {
  const resolvedLeft = path.resolve(left);
  const resolvedRight = path.resolve(right);
  if (process.platform === "win32") {
    return resolvedLeft.toLowerCase() === resolvedRight.toLowerCase();
  }
  return resolvedLeft === resolvedRight;
}

function requiredString(value, field) {
  if (typeof value !== "string" || value.trim().length === 0 || value.includes("\0")) {
    workspaceFailure("INVALID_LOCK_REQUEST", `${field} must be a non-empty string.`);
  }
  return value.trim();
}

function validateWriterRequest(request) {
  if (
    request === null ||
    typeof request !== "object" ||
    Array.isArray(request)
  ) {
    workspaceFailure("INVALID_LOCK_REQUEST", "Writer request must be an object.");
  }
  if (request.role !== "IMPLEMENTER") {
    workspaceFailure(
      "WRITER_ROLE_DENIED",
      "Only IMPLEMENTER can acquire the product-code writer lock.",
    );
  }
  const projectId = requiredString(request.project_id, "project_id");
  const taskId = requiredString(request.task_id, "task_id");
  const specHash = requiredString(request.spec_hash, "spec_hash").toLowerCase();
  if (!PROJECT_ID_PATTERN.test(projectId) || !TASK_ID_PATTERN.test(taskId)) {
    workspaceFailure("INVALID_LOCK_REQUEST", "Project or task identity is invalid.");
  }
  if (!Number.isInteger(request.task_revision) || request.task_revision < 1) {
    workspaceFailure("INVALID_LOCK_REQUEST", "task_revision must be positive.");
  }
  if (!HASH_PATTERN.test(specHash)) {
    workspaceFailure("INVALID_LOCK_REQUEST", "spec_hash must be SHA-256.");
  }
  return {
    project_id: projectId,
    task_id: taskId,
    task_revision: request.task_revision,
    spec_hash: specHash,
    attempt_id: requiredString(request.attempt_id, "attempt_id"),
    worktree_id: requiredString(request.worktree_id, "worktree_id"),
    role: "IMPLEMENTER",
  };
}

function validateStoredLock(record) {
  const issuedAt = Date.parse(record?.issued_at);
  const expiresAt = Date.parse(record?.expires_at);
  const storedString = (value) =>
    typeof value === "string" &&
    value.trim().length > 0 &&
    value === value.trim() &&
    !value.includes("\0");
  if (
    record === null ||
    typeof record !== "object" ||
    Array.isArray(record) ||
    record.version !== 1 ||
    record.active !== true ||
    record.role !== "IMPLEMENTER" ||
    !PROJECT_ID_PATTERN.test(record.project_id ?? "") ||
    !TASK_ID_PATTERN.test(record.task_id ?? "") ||
    !Number.isInteger(record.task_revision) ||
    record.task_revision < 1 ||
    !HASH_PATTERN.test(record.spec_hash ?? "") ||
    !storedString(record.attempt_id) ||
    !storedString(record.worktree_id) ||
    !storedString(record.lease_id) ||
    !Number.isInteger(record.fencing_token) ||
    record.fencing_token < 1 ||
    record.current_fencing_token !== record.fencing_token ||
    !Number.isFinite(issuedAt) ||
    !Number.isFinite(expiresAt) ||
    expiresAt <= issuedAt
  ) {
    workspaceFailure("LOCK_UNKNOWN_STATE", "Project lock record is invalid.");
  }
  return deepFreeze(record);
}

export class ProjectLock {
  #projectRoot;
  #controlDirectory;
  #lockDirectory;
  #lockFile;
  #counterFile;
  #counterInitializedFile;
  #counterInitializationDirectory;
  #clock;
  #idFactory;
  #leaseDurationMs;

  constructor({
    projectRoot,
    clock = () => new Date(),
    idFactory = () => randomUUID(),
    leaseDurationMs = 30 * 60 * 1000,
  }) {
    if (
      typeof projectRoot !== "string" ||
      projectRoot.trim().length === 0 ||
      projectRoot !== projectRoot.trim() ||
      projectRoot.includes("\0")
    ) {
      workspaceFailure("INVALID_PROJECT_ROOT", "projectRoot is required.");
    }
    if (
      typeof clock !== "function" ||
      typeof idFactory !== "function" ||
      !Number.isInteger(leaseDurationMs) ||
      leaseDurationMs < 1
    ) {
      workspaceFailure("INVALID_LOCK_DEPENDENCY", "Lock dependencies are invalid.");
    }
    this.#projectRoot = path.resolve(projectRoot);
    this.#controlDirectory = path.join(this.#projectRoot, ".lattice");
    this.#lockDirectory = path.join(this.#controlDirectory, "locks");
    this.#lockFile = path.join(this.#lockDirectory, "project.lock");
    this.#counterFile = path.join(this.#lockDirectory, "fencing-token");
    this.#counterInitializedFile = path.join(
      this.#lockDirectory,
      "fencing-initialized",
    );
    this.#counterInitializationDirectory = path.join(
      this.#lockDirectory,
      ".fencing-initializing",
    );
    this.#clock = clock;
    this.#idFactory = idFactory;
    this.#leaseDurationMs = leaseDurationMs;
  }

  get lockPath() {
    return this.#lockFile;
  }

  #clockNow() {
    let value;
    try {
      value = this.#clock();
    } catch {
      workspaceFailure("INVALID_LOCK_CLOCK", "Project lock clock failed.");
    }
    const date =
      value instanceof Date
        ? new Date(value.getTime())
        : new Date(value);
    if (!Number.isFinite(date.getTime())) {
      workspaceFailure("INVALID_LOCK_CLOCK", "Project lock clock is invalid.");
    }
    return date;
  }

  async #validateProjectRoot() {
    const projectStat = await optionalLstat(this.#projectRoot);
    if (
      !projectStat ||
      !projectStat.isDirectory() ||
      projectStat.isSymbolicLink()
    ) {
      workspaceFailure(
        "LOCK_PATH_ESCAPE",
        "Project root is not a real directory.",
      );
    }
    const projectRealPath = await realpath(this.#projectRoot);
    if (!samePath(projectRealPath, this.#projectRoot)) {
      workspaceFailure(
        "LOCK_PATH_ESCAPE",
        "Project root traverses a link or junction.",
      );
    }
    return projectRealPath;
  }

  async #validateContainedDirectory(directory, projectRealPath) {
    const directoryStat = await optionalLstat(directory);
    if (!directoryStat) {
      return null;
    }
    if (!directoryStat.isDirectory() || directoryStat.isSymbolicLink()) {
      workspaceFailure(
        "LOCK_PATH_ESCAPE",
        "Project lock ancestor is not a real directory.",
      );
    }
    const directoryRealPath = await realpath(directory);
    if (
      !samePath(directoryRealPath, directory) ||
      !containedBy(projectRealPath, directoryRealPath)
    ) {
      workspaceFailure(
        "LOCK_PATH_ESCAPE",
        "Project lock directory is not a contained real directory.",
      );
    }
    return directoryRealPath;
  }

  async #inspectSafeLockDirectory() {
    const projectRealPath = await this.#validateProjectRoot();
    const controlDirectory = await this.#validateContainedDirectory(
      this.#controlDirectory,
      projectRealPath,
    );
    if (controlDirectory === null) {
      return null;
    }
    return this.#validateContainedDirectory(
      this.#lockDirectory,
      projectRealPath,
    );
  }

  async #ensureSafeLockDirectory() {
    const projectRealPath = await this.#validateProjectRoot();
    for (const directory of [this.#controlDirectory, this.#lockDirectory]) {
      let directoryRealPath = await this.#validateContainedDirectory(
        directory,
        projectRealPath,
      );
      if (directoryRealPath === null) {
        try {
          await mkdir(directory);
        } catch (error) {
          if (error?.code !== "EEXIST") {
            throw error;
          }
        }
        directoryRealPath = await this.#validateContainedDirectory(
          directory,
          projectRealPath,
        );
      }
      if (directoryRealPath === null) {
        workspaceFailure("LOCK_PATH_ESCAPE", "Project lock directory was not created.");
      }
    }
  }

  async #readFencingToken() {
    const counterStat = await optionalLstat(this.#counterFile);
    if (counterStat && (!counterStat.isFile() || counterStat.isSymbolicLink())) {
      workspaceFailure("LOCK_UNKNOWN_STATE", "Fencing counter is not a regular file.");
    }
    if (!counterStat) {
      workspaceFailure(
        "LOCK_UNKNOWN_STATE",
        "Initialized fencing counter is missing.",
      );
    }
    const raw = (await readFile(this.#counterFile, "utf8")).trim();
    const current = Number.parseInt(raw, 10);
    if (!/^\d+$/.test(raw) || !Number.isSafeInteger(current) || current < 0) {
      workspaceFailure("LOCK_UNKNOWN_STATE", "Fencing counter is invalid.");
    }
    return current;
  }

  async #validateFencingMarker() {
    const markerStat = await optionalLstat(this.#counterInitializedFile);
    if (!markerStat) {
      return false;
    }
    if (!markerStat.isFile() || markerStat.isSymbolicLink()) {
      workspaceFailure(
        "LOCK_UNKNOWN_STATE",
        "Fencing initialization marker is unsafe.",
      );
    }
    let marker;
    try {
      marker = JSON.parse(
        await readFile(this.#counterInitializedFile, "utf8"),
      );
    } catch {
      workspaceFailure(
        "LOCK_UNKNOWN_STATE",
        "Fencing initialization marker is invalid.",
      );
    }
    if (
      marker?.version !== 1 ||
      marker?.kind !== "fencing-counter-initialized"
    ) {
      workspaceFailure(
        "LOCK_UNKNOWN_STATE",
        "Fencing initialization marker is invalid.",
      );
    }
    return true;
  }

  async #ensureFencingState() {
    const initializationStat = await optionalLstat(
      this.#counterInitializationDirectory,
    );
    if (initializationStat) {
      if (
        !initializationStat.isDirectory() ||
        initializationStat.isSymbolicLink() ||
        !samePath(
          await realpath(this.#counterInitializationDirectory),
          this.#counterInitializationDirectory,
        )
      ) {
        workspaceFailure(
          "LOCK_UNKNOWN_STATE",
          "Fencing initialization lock is unsafe.",
        );
      }
      workspaceFailure(
        "LOCK_ALREADY_HELD",
        "Fencing-state initialization is already in progress.",
        { phase: "fencing-initialization" },
      );
    }

    const counterExists = Boolean(await optionalLstat(this.#counterFile));
    const markerExists = await this.#validateFencingMarker();
    if (counterExists || markerExists) {
      if (!counterExists || !markerExists) {
        workspaceFailure(
          "LOCK_UNKNOWN_STATE",
          "Fencing initialization evidence is incomplete.",
        );
      }
      await this.#readFencingToken();
      return;
    }

    try {
      await mkdir(this.#counterInitializationDirectory);
    } catch (error) {
      if (error?.code === "EEXIST") {
        workspaceFailure(
          "LOCK_ALREADY_HELD",
          "Fencing-state initialization is already in progress.",
          { phase: "fencing-initialization" },
        );
      }
      throw error;
    }

    let completed = false;
    try {
      const stateAppeared =
        Boolean(await optionalLstat(this.#counterFile)) ||
        (await this.#validateFencingMarker());
      if (!stateAppeared) {
        await durableWrite(this.#counterFile, "0\n", "wx");
        await durableWrite(
          this.#counterInitializedFile,
          `${JSON.stringify({
            version: 1,
            kind: "fencing-counter-initialized",
          })}\n`,
          "wx",
        );
      }
      const initialized = await this.#validateFencingMarker();
      if (!initialized) {
        workspaceFailure(
          "LOCK_UNKNOWN_STATE",
          "Fencing initialization did not complete.",
        );
      }
      await this.#readFencingToken();
      completed = true;
    } finally {
      if (completed) {
        await rmdir(this.#counterInitializationDirectory);
      }
    }
  }

  async #nextFencingToken() {
    const current = await this.#readFencingToken();
    const next = current + 1;
    await durableWrite(this.#counterFile, `${next}\n`);
    return next;
  }

  async inspect() {
    const safeLockDirectory = await this.#inspectSafeLockDirectory();
    if (safeLockDirectory === null) {
      return null;
    }
    const lockStat = await optionalLstat(this.#lockFile);
    if (!lockStat) {
      return null;
    }
    if (!lockStat.isFile() || lockStat.isSymbolicLink()) {
      workspaceFailure("LOCK_UNKNOWN_STATE", "Project lock is not a regular file.");
    }
    let record;
    try {
      record = JSON.parse(await readFile(this.#lockFile, "utf8"));
    } catch {
      workspaceFailure("LOCK_UNKNOWN_STATE", "Project lock is not valid JSON.");
    }
    return validateStoredLock(record);
  }

  async acquire(request) {
    const normalized = validateWriterRequest(request);
    const issuedAt = this.#clockNow();
    const leaseId = requiredString(this.#idFactory(), "lease_id");
    await this.#ensureSafeLockDirectory();
    await this.#ensureFencingState();
    let handle;
    try {
      handle = await open(this.#lockFile, "wx");
    } catch (error) {
      if (error?.code !== "EEXIST") {
        throw error;
      }
      let existing;
      try {
        existing = await this.inspect();
      } catch {
        let raw = null;
        try {
          raw = await readFile(this.#lockFile, "utf8");
        } catch {
          // The stable fail-closed reason below covers unreadable lock state.
        }
        if (raw !== null && raw.trim().length === 0) {
          workspaceFailure(
            "LOCK_ALREADY_HELD",
            "Project lock acquisition is already in progress.",
            { phase: "initializing" },
          );
        }
        workspaceFailure(
          "LOCK_UNKNOWN_STATE",
          "Existing project lock cannot be safely interpreted.",
        );
      }
      workspaceFailure(
        "LOCK_ALREADY_HELD",
        "Project lock is already held and will not be auto-broken.",
        {
          lease_id: existing?.lease_id,
          task_id: existing?.task_id,
          expires_at: existing?.expires_at,
        },
      );
    }

    try {
      const fencingToken = await this.#nextFencingToken();
      const record = deepFreeze({
        version: 1,
        ...normalized,
        lease_id: leaseId,
        fencing_token: fencingToken,
        current_fencing_token: fencingToken,
        issued_at: issuedAt.toISOString(),
        expires_at: new Date(
          issuedAt.getTime() + this.#leaseDurationMs,
        ).toISOString(),
        active: true,
      });
      await handle.writeFile(`${JSON.stringify(record)}\n`, "utf8");
      await handle.sync();
      return record;
    } finally {
      await handle.close();
    }
  }

  async validateWriter({ lease_id, fencing_token }) {
    const current = await this.inspect();
    if (!current) {
      workspaceFailure("LOCK_NOT_HELD", "No project writer lock is held.");
    }
    if (current.lease_id !== lease_id) {
      workspaceFailure("LOCK_OWNERSHIP_MISMATCH", "Writer lease ID is not current.");
    }
    if (
      current.fencing_token !== fencing_token ||
      current.current_fencing_token !== fencing_token
    ) {
      workspaceFailure("STALE_FENCING_TOKEN", "Writer fencing token is stale.");
    }
    const nowTime = this.#clockNow().getTime();
    if (Date.parse(current.expires_at) <= nowTime) {
      workspaceFailure("WRITER_LEASE_EXPIRED", "Writer lease has expired.");
    }
    return current;
  }

  async release({ lease_id, fencing_token }) {
    const current = await this.inspect();
    if (!current) {
      workspaceFailure("LOCK_NOT_HELD", "No project writer lock is held.");
    }
    if (
      current.lease_id !== lease_id ||
      current.fencing_token !== fencing_token
    ) {
      workspaceFailure(
        "LOCK_OWNERSHIP_MISMATCH",
        "Only the exact current writer can release the lock.",
      );
    }
    const releasedAt = this.#clockNow();
    const lockStat = await lstat(this.#lockFile);
    if (!lockStat.isFile() || lockStat.isSymbolicLink()) {
      workspaceFailure("LOCK_UNKNOWN_STATE", "Project lock changed before release.");
    }
    await unlink(this.#lockFile);
    return deepFreeze({
      ...current,
      active: false,
      released_at: releasedAt.toISOString(),
    });
  }
}

export { WorkspaceError };
