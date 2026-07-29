import assert from "node:assert/strict";
import {
  access,
  mkdir,
  mkdtemp,
  readFile,
  rm,
  symlink,
  writeFile,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import {
  ProjectLock,
  WorkspaceError,
} from "../src/workspace/project-lock.js";

async function projectFixture(t) {
  const projectRoot = await mkdtemp(path.join(os.tmpdir(), "lattice-project-"));
  t.after(async () => {
    await rm(projectRoot, { force: true, recursive: true });
  });
  let currentTime = Date.parse("2026-07-29T00:00:00.000Z");
  let leaseId = 0;
  return {
    projectRoot,
    advance(milliseconds) {
      currentTime += milliseconds;
    },
    lock: new ProjectLock({
      projectRoot,
      clock: () => new Date(currentTime),
      idFactory: () => `LEASE-${String(++leaseId).padStart(4, "0")}`,
      leaseDurationMs: 60_000,
    }),
  };
}

function writerRequest(overrides = {}) {
  return {
    project_id: "lattice-devos",
    task_id: "TASK-2026-0001",
    task_revision: 1,
    spec_hash: "a".repeat(64),
    attempt_id: "ATTEMPT-0001",
    worktree_id: "WORKTREE-0001",
    role: "IMPLEMENTER",
    ...overrides,
  };
}

test("allows one exact writer and preserves monotonic fencing across release", async (t) => {
  const { lock } = await projectFixture(t);
  const first = await lock.acquire(writerRequest());

  assert.equal(first.active, true);
  assert.equal(first.lease_id, "LEASE-0001");
  assert.equal(first.fencing_token, 1);
  assert.equal((await lock.inspect()).lease_id, first.lease_id);

  await assert.rejects(
    lock.acquire(
      writerRequest({
        task_id: "TASK-2026-0002",
        attempt_id: "ATTEMPT-0002",
      }),
    ),
    (error) =>
      error instanceof WorkspaceError && error.code === "LOCK_ALREADY_HELD",
  );
  assert.equal(
    (await lock.validateWriter({
      lease_id: first.lease_id,
      fencing_token: first.fencing_token,
    })).task_id,
    first.task_id,
  );
  await assert.rejects(
    lock.release({
      lease_id: first.lease_id,
      fencing_token: 999,
    }),
    (error) =>
      error instanceof WorkspaceError &&
      error.code === "LOCK_OWNERSHIP_MISMATCH",
  );

  const released = await lock.release({
    lease_id: first.lease_id,
    fencing_token: first.fencing_token,
  });
  assert.equal(released.active, false);
  assert.equal(await lock.inspect(), null);

  const second = await lock.acquire(
    writerRequest({
      task_id: "TASK-2026-0002",
      attempt_id: "ATTEMPT-0002",
    }),
  );
  assert.equal(second.fencing_token, 2);
});

test("rejects expired, stale, non-Implementer, and corrupt lock evidence without breaking it", async (t) => {
  const fixture = await projectFixture(t);
  const { lock, advance, projectRoot } = fixture;
  const held = await lock.acquire(writerRequest());

  await assert.rejects(
    lock.validateWriter({
      lease_id: held.lease_id,
      fencing_token: held.fencing_token + 1,
    }),
    (error) =>
      error instanceof WorkspaceError &&
      error.code === "STALE_FENCING_TOKEN",
  );
  advance(60_001);
  await assert.rejects(
    lock.validateWriter({
      lease_id: held.lease_id,
      fencing_token: held.fencing_token,
    }),
    (error) =>
      error instanceof WorkspaceError && error.code === "WRITER_LEASE_EXPIRED",
  );
  await assert.rejects(
    lock.acquire(writerRequest({ task_id: "TASK-2026-0003" })),
    (error) =>
      error instanceof WorkspaceError && error.code === "LOCK_ALREADY_HELD",
  );

  await lock.release({
    lease_id: held.lease_id,
    fencing_token: held.fencing_token,
  });
  await assert.rejects(
    lock.acquire(writerRequest({ role: "INTEGRATOR" })),
    (error) =>
      error instanceof WorkspaceError && error.code === "WRITER_ROLE_DENIED",
  );

  const lockDirectory = path.join(projectRoot, ".lattice", "locks");
  await mkdir(lockDirectory, { recursive: true });
  await writeFile(path.join(lockDirectory, "project.lock"), "{not-json}\n");
  await assert.rejects(
    lock.acquire(writerRequest()),
    (error) =>
      error instanceof WorkspaceError && error.code === "LOCK_UNKNOWN_STATE",
  );
});

test("rejects a junctioned lock ancestor before creating an external directory", async (t) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "lattice-lock-junction-"));
  t.after(async () => {
    await rm(root, { force: true, recursive: true });
  });
  const projectRoot = path.join(root, "project");
  const unrelatedRoot = path.join(root, "unrelated");
  await mkdir(projectRoot);
  await mkdir(unrelatedRoot);
  await symlink(
    unrelatedRoot,
    path.join(projectRoot, ".lattice"),
    process.platform === "win32" ? "junction" : "dir",
  );
  const lock = new ProjectLock({
    projectRoot,
    idFactory: () => "LEASE-JUNCTION",
  });

  await assert.rejects(
    lock.acquire(writerRequest()),
    (error) =>
      error instanceof WorkspaceError && error.code === "LOCK_PATH_ESCAPE",
  );
  await assert.rejects(
    access(path.join(unrelatedRoot, "locks")),
    (error) => error?.code === "ENOENT",
  );
});

test("rejects incomplete or internally inconsistent stored lock records", async (t) => {
  const { lock } = await projectFixture(t);
  const held = await lock.acquire(writerRequest());
  const invalidRecords = [
    { ...held, task_revision: 0 },
    { ...held, attempt_id: "" },
    { ...held, worktree_id: null },
    { ...held, expires_at: held.issued_at },
  ];

  for (const record of invalidRecords) {
    await writeFile(lock.lockPath, `${JSON.stringify(record)}\n`);
    await assert.rejects(
      lock.inspect(),
      (error) =>
        error instanceof WorkspaceError && error.code === "LOCK_UNKNOWN_STATE",
    );
  }
});

test("rejects inspect, validation, and release through a junctioned project alias", async (t) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "lattice-lock-alias-"));
  t.after(async () => {
    await rm(root, { force: true, recursive: true });
  });
  const projectRoot = path.join(root, "project");
  const aliasRoot = path.join(root, "project-alias");
  await mkdir(projectRoot);
  const original = new ProjectLock({
    projectRoot,
    idFactory: () => "LEASE-ALIAS",
  });
  const held = await original.acquire(writerRequest());
  await symlink(
    projectRoot,
    aliasRoot,
    process.platform === "win32" ? "junction" : "dir",
  );
  const alias = new ProjectLock({ projectRoot: aliasRoot });

  await assert.rejects(
    alias.inspect(),
    (error) =>
      error instanceof WorkspaceError && error.code === "LOCK_PATH_ESCAPE",
  );
  await assert.rejects(
    alias.validateWriter({
      lease_id: held.lease_id,
      fencing_token: held.fencing_token,
    }),
    (error) =>
      error instanceof WorkspaceError && error.code === "LOCK_PATH_ESCAPE",
  );
  await assert.rejects(
    alias.release({
      lease_id: held.lease_id,
      fencing_token: held.fencing_token,
    }),
    (error) =>
      error instanceof WorkspaceError && error.code === "LOCK_PATH_ESCAPE",
  );
  assert.equal((await original.inspect()).lease_id, held.lease_id);
});

test("invalid clock evidence cannot authorize or release a writer", async (t) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "lattice-lock-clock-"));
  t.after(async () => {
    await rm(root, { force: true, recursive: true });
  });
  const projectRoot = path.join(root, "project");
  const secondProjectRoot = path.join(root, "second-project");
  await mkdir(projectRoot);
  await mkdir(secondProjectRoot);
  let now = new Date("2026-07-29T00:00:00.000Z");
  const lock = new ProjectLock({
    projectRoot,
    clock: () => now,
    idFactory: () => "LEASE-CLOCK",
  });
  const held = await lock.acquire(writerRequest());
  now = new Date(Number.NaN);

  await assert.rejects(
    lock.validateWriter({
      lease_id: held.lease_id,
      fencing_token: held.fencing_token,
    }),
    (error) =>
      error instanceof WorkspaceError && error.code === "INVALID_LOCK_CLOCK",
  );
  await assert.rejects(
    lock.release({
      lease_id: held.lease_id,
      fencing_token: held.fencing_token,
    }),
    (error) =>
      error instanceof WorkspaceError && error.code === "INVALID_LOCK_CLOCK",
  );
  assert.equal((await lock.inspect()).lease_id, held.lease_id);

  const invalidAtAcquire = new ProjectLock({
    projectRoot: secondProjectRoot,
    clock: () => new Date(Number.NaN),
    idFactory: () => "LEASE-INVALID-AT-ACQUIRE",
  });
  await assert.rejects(
    invalidAtAcquire.acquire(writerRequest()),
    (error) =>
      error instanceof WorkspaceError && error.code === "INVALID_LOCK_CLOCK",
  );
  assert.equal(await invalidAtAcquire.inspect(), null);
});

test("missing fencing counter after initialization fails closed", async (t) => {
  const { lock, projectRoot } = await projectFixture(t);
  const held = await lock.acquire(writerRequest());
  await lock.release({
    lease_id: held.lease_id,
    fencing_token: held.fencing_token,
  });
  await rm(path.join(projectRoot, ".lattice", "locks", "fencing-token"));

  await assert.rejects(
    lock.acquire(
      writerRequest({
        task_id: "TASK-2026-0010",
        attempt_id: "ATTEMPT-0010",
        worktree_id: "WORKTREE-0010",
      }),
    ),
    (error) =>
      error instanceof WorkspaceError && error.code === "LOCK_UNKNOWN_STATE",
  );
});

test("concurrent first acquisitions produce one writer and one stable contention denial", async (t) => {
  const projectRoot = await mkdtemp(
    path.join(os.tmpdir(), "lattice-lock-race-"),
  );
  t.after(async () => {
    await rm(projectRoot, { force: true, recursive: true });
  });
  const first = new ProjectLock({
    projectRoot,
    idFactory: () => "LEASE-RACE-1",
  });
  const second = new ProjectLock({
    projectRoot,
    idFactory: () => "LEASE-RACE-2",
  });

  const results = await Promise.allSettled([
    first.acquire(writerRequest({ attempt_id: "ATTEMPT-RACE-1" })),
    second.acquire(writerRequest({ attempt_id: "ATTEMPT-RACE-2" })),
  ]);
  const fulfilled = results.filter((result) => result.status === "fulfilled");
  const rejected = results.filter((result) => result.status === "rejected");

  assert.equal(fulfilled.length, 1);
  assert.equal(rejected.length, 1);
  assert.equal(rejected[0].reason instanceof WorkspaceError, true);
  assert.equal(rejected[0].reason.code, "LOCK_ALREADY_HELD");
  assert.equal((await first.inspect()).lease_id, fulfilled[0].value.lease_id);
  assert.equal(fulfilled[0].value.fencing_token, 1);
  assert.equal(
    await readFile(
      path.join(projectRoot, ".lattice", "locks", "fencing-token"),
      "utf8",
    ),
    "1\n",
  );
});

test("rejects ambiguous project roots before resolving a filesystem path", () => {
  for (const projectRoot of ["", "   ", "C:\\unsafe\u0000root"]) {
    assert.throws(
      () => new ProjectLock({ projectRoot }),
      (error) =>
        error instanceof WorkspaceError &&
        error.code === "INVALID_PROJECT_ROOT",
    );
  }
});
