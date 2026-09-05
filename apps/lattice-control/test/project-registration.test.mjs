import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { createHash } from "node:crypto";
import { EventEmitter } from "node:events";
import { request as httpRequest } from "node:http";
import {
  access,
  mkdir,
  mkdtemp,
  readFile,
  realpath,
  rename,
  rm,
  stat,
  symlink,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { constants as sqliteConstants, DatabaseSync } from "node:sqlite";
import { pathToFileURL } from "node:url";
import { promisify } from "node:util";
import test from "node:test";

import {
  createWsl2ProjectGitExecutor,
  defaultGitExecutor,
  inspectProject,
  normalizeRequestedProjectPath,
  parseWsl2ProjectPath,
  ProjectInspectionError,
  sanitizeRemoteUrl,
} from "../src/project-inspector.mjs";
import { runProjectCommand } from "../src/project-client.mjs";
import { createLatticeServer } from "../src/server.mjs";
import { LatticeStore } from "../src/store.mjs";

const execFileAsync = promisify(execFile);

async function git(repository, ...args) {
  return execFileAsync("git", args, {
    cwd: repository,
    encoding: "utf8",
    windowsHide: true,
  });
}

class IdleCodex extends EventEmitter {
  connected = false;

  async close() {}
}

async function listen(application) {
  await new Promise((resolve) => application.server.listen(0, "127.0.0.1", resolve));
  const address = application.server.address();
  return `http://127.0.0.1:${address.port}`;
}

async function close(application) {
  await new Promise((resolve) => application.server.close(resolve));
}

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

async function rawHttpStatus(url, { method = "GET", headers = {}, body = "" } = {}) {
  const parsed = new URL(url);
  return new Promise((resolve, reject) => {
    const request = httpRequest({
      host: parsed.hostname,
      port: parsed.port,
      path: `${parsed.pathname}${parsed.search}`,
      method,
      headers,
    }, (response) => {
      response.resume();
      response.once("end", () => resolve(response.statusCode));
    });
    request.once("error", reject);
    request.end(body);
  });
}

function fixtureInspection(canonicalPath, observedAt, {
  headSha = null,
  isRepository = headSha != null,
  gitStatus = "complete",
  gitFailures = [],
  remotes = [],
} = {}) {
  return {
    canonical_path: canonicalPath,
    repo_root: isRepository === true ? canonicalPath : null,
    git: {
      status: gitStatus,
      is_repository: isRepository,
      branch: isRepository === true ? "main" : null,
      detached: isRepository === true ? false : null,
      head_sha: headSha,
      dirty: isRepository === true ? false : null,
      upstream: null,
      ahead: null,
      behind: null,
      remotes,
      observed_at: observedAt,
      failures: gitFailures,
    },
    rules: {
      status: "complete",
      observed_at: observedAt,
      documents: [],
      missing_standard_documents: ["AGENTS.md", "PROJECT_STATE.md", "PLANS.md"],
      failures: [],
    },
  };
}

function assertControlCatalogLocator(project) {
  assert.equal(project.schema_version, "lattice.control.project-catalog.v1");
  assert.equal(project.record_kind, "CONTROL_LOCAL_CATALOG");
  assert.equal(project.registry_authority, "NONE");
  assert.equal(project.registry_project_id, null);
  assert.equal(project.control_project_id, project.id);
  assert.equal("storage_authority" in project, false);
  for (const forbidden of ["authority_receipt", "authority_snapshot", "registry_revision"]) {
    assert.equal(forbidden in project, false);
  }
}

test("project inspection captures bounded Git state and hashes rule documents without storing content", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-project-inspection-"));
  try {
    await git(directory, "init", "-b", "main");
    await git(directory, "config", "user.name", "LATTICE Test");
    await git(directory, "config", "user.email", "lattice@example.invalid");
    const agents = "# Fixture rules\n\nNever expose credentials.\n";
    const projectState = "# Project State\n\nCurrent fixture state.\n";
    await mkdir(path.join(directory, ".codex"));
    await mkdir(path.join(directory, "docs"));
    await writeFile(path.join(directory, "AGENTS.md"), agents, "utf8");
    await writeFile(path.join(directory, "PROJECT_STATE.md"), projectState, "utf8");
    await writeFile(path.join(directory, "DONE_CRITERIA.md"), "# Done criteria\n", "utf8");
    await writeFile(path.join(directory, "VERIFY.md"), "# Verification\n", "utf8");
    await writeFile(path.join(directory, "ROADMAP.md"), "# Roadmap\n", "utf8");
    await writeFile(
      path.join(directory, ".codex", "project_controller.md"),
      "# Project controller\n",
      "utf8",
    );
    await writeFile(
      path.join(directory, "docs", "VALIDATION_RULES.md"),
      "# Validation rules\n",
      "utf8",
    );
    await git(directory, "add", "AGENTS.md", "PROJECT_STATE.md");
    await git(directory, "commit", "-m", "fixture");
    await git(
      directory,
      "remote",
      "add",
      "origin",
      "https://fixture-user:credential-value@example.invalid/team/repository.git?token=hidden#fragment",
    );
    await git(
      directory,
      "config",
      "--add",
      "remote.origin.url",
      "https://fixture-user:credential-value@example.invalid/team/repository.git?token=hidden#fragment",
    );
    await writeFile(path.join(directory, "untracked.txt"), "dirty\n", "utf8");

    const inspection = await inspectProject(directory);
    const canonicalDirectory = await realpath(directory);
    const expectedHead = (await git(directory, "rev-parse", "HEAD")).stdout.trim();

    assert.equal(inspection.canonical_path, canonicalDirectory);
    assert.equal(inspection.repo_root, canonicalDirectory);
    assert.equal(inspection.git.is_repository, true);
    assert.equal(inspection.git.branch, "main");
    assert.equal(inspection.git.detached, false);
    assert.equal(inspection.git.head_sha, expectedHead);
    assert.equal(inspection.git.dirty, true);
    assert.equal(inspection.git.upstream, null);
    assert.equal(inspection.git.ahead, null);
    assert.equal(inspection.git.behind, null);
    assert.ok(Date.parse(inspection.git.observed_at));
    assert.deepEqual(
      inspection.git.remotes.map(({ name, direction, url, credentials_redacted }) => ({
        name,
        direction,
        url,
        credentials_redacted,
      })),
      [
        {
          name: "origin",
          direction: "fetch",
          url: "https://example.invalid/team/repository.git",
          credentials_redacted: true,
        },
        {
          name: "origin",
          direction: "push",
          url: "https://example.invalid/team/repository.git",
          credentials_redacted: true,
        },
      ],
    );
    assert.doesNotMatch(JSON.stringify(inspection), /fixture-user|credential-value|token=hidden/u);

    const indexedAgents = inspection.rules.documents.find(
      (document) => document.relative_path === "AGENTS.md",
    );
    assert.deepEqual(indexedAgents, {
      relative_path: "AGENTS.md",
      sha256: createHash("sha256").update(agents).digest("hex"),
      observed_at: inspection.rules.observed_at,
      purpose: "Codex and agent working rules",
    });
    assert.ok(inspection.rules.documents.some(
      (document) => document.relative_path === "PROJECT_STATE.md",
    ));
    for (const relativePath of [
      ".codex/project_controller.md",
      "DONE_CRITERIA.md",
      "ROADMAP.md",
      "VERIFY.md",
      "docs/VALIDATION_RULES.md",
    ]) {
      assert.ok(
        inspection.rules.documents.some((document) => document.relative_path === relativePath),
        `${relativePath} should be indexed as an authoritative project document`,
      );
    }
    assert.ok(inspection.rules.missing_standard_documents.includes("PLANS.md"));
    assert.equal("content" in indexedAgents, false);

    const databasePath = path.join(directory, "control.db");
    const store = new LatticeStore(databasePath);
    try {
      const registered = store.registerProject({ name: "Credential fixture", inspection });
      assertControlCatalogLocator(registered.project);
      assert.doesNotMatch(
        JSON.stringify(registered),
        /fixture-user|credential-value|token=hidden/u,
      );
      assert.deepEqual(
        store.database.prepare(`
          SELECT url_sanitized
          FROM project_git_remotes
          ORDER BY direction ASC
        `).all().map((row) => ({ ...row })),
        [
          { url_sanitized: "https://example.invalid/team/repository.git" },
          { url_sanitized: "https://example.invalid/team/repository.git" },
        ],
      );
      const injectedCredential = structuredClone(inspection);
      injectedCredential.git.remotes[0].url = "https://injected-user:INJECTED_SECRET@example.invalid/repo.git";
      assert.throws(
        () => store.registerProject({ name: "Injected", inspection: injectedCredential }),
        /sanitized Git remote URL is invalid/u,
      );
    } finally {
      store.close();
    }
    const databaseBytes = await readFile(databasePath);
    assert.equal(databaseBytes.includes(Buffer.from("credential-value")), false);
    assert.equal(databaseBytes.includes(Buffer.from("token=hidden")), false);
    assert.equal(databaseBytes.includes(Buffer.from("INJECTED_SECRET")), false);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test("remote sanitization rejects helper payloads and never returns credential sentinels", async () => {
  const sentinel = "SENTINEL_REMOTE_CREDENTIAL";
  const rejected = [
    `ext::sh -c token=${sentinel}`,
    `data:text/plain,${sentinel}`,
    `user@host.invalid:repository.git?token=${sentinel}#fragment`,
  ];
  for (const remote of rejected) {
    let failure;
    try {
      sanitizeRemoteUrl(remote, process.cwd());
    } catch (error) {
      failure = error;
    }
    assert.ok(failure instanceof Error);
    assert.doesNotMatch(failure.message, new RegExp(sentinel, "u"));
  }

  const sanitized = sanitizeRemoteUrl(
    `https://user:${sentinel}@example.invalid/team/repository.git?token=${sentinel}#fragment`,
    process.cwd(),
  );
  assert.deepEqual(sanitized, {
    url: "https://example.invalid/team/repository.git",
    credentials_redacted: true,
  });

  const directory = await mkdtemp(path.join(tmpdir(), "lattice-project-remote-failure-"));
  try {
    await git(directory, "init", "-b", "main");
    const inspection = await inspectProject(directory, {
      gitExecutor: async (request) => {
        if (request.args[0] === "remote" && request.args[1] === "get-url") {
          return { exit_code: 1, stdout: "", stderr: sentinel };
        }
        return defaultGitExecutor(request);
      },
    });
    assert.doesNotMatch(JSON.stringify(inspection), new RegExp(sentinel, "u"));
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test("remote observation explains bounded truncation instead of silently dropping URLs", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-project-remote-limit-"));
  try {
    await git(directory, "init", "-b", "main");
    await git(directory, "remote", "add", "origin", "https://example.invalid/fetch.git");
    const pushUrls = Array.from(
      { length: 17 },
      (_, index) => `https://example.invalid/push-${index}.git`,
    );
    const inspection = await inspectProject(directory, {
      gitExecutor: async (request) => {
        if (
          request.args[0] === "remote"
          && request.args[1] === "get-url"
          && request.args.includes("--push")
        ) {
          return { exit_code: 0, stdout: `${pushUrls.join("\n")}\n`, stderr: "" };
        }
        return defaultGitExecutor(request);
      },
    });
    assert.equal(
      inspection.git.remotes.filter((remote) => remote.direction === "push").length,
      16,
    );
    assert.equal(inspection.git.status, "partial");
    assert.ok(inspection.git.failures.some(
      (failure) => failure.code === "GIT_REMOTE_URL_LIMIT_EXCEEDED",
    ));
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test("Git observation ignores inherited repository selectors and repository-local executables", async (context) => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-project-git-boundary-"));
  const target = path.join(directory, "target");
  const other = path.join(directory, "other");
  const originalGitDirectory = process.env.GIT_DIR;
  const originalGitWorkTree = process.env.GIT_WORK_TREE;
  const originalGitTrace = process.env.GIT_TRACE;
  try {
    await mkdir(target);
    await mkdir(other);
    await git(other, "init", "-b", "main");
    await git(other, "config", "user.name", "LATTICE Test");
    await git(other, "config", "user.email", "lattice@example.invalid");
    await writeFile(path.join(other, "tracked.txt"), "other\n", "utf8");
    await git(other, "add", ".");
    await git(other, "commit", "-m", "other");
    process.env.GIT_DIR = path.join(other, ".git");
    process.env.GIT_WORK_TREE = target;
    const traceSentinel = path.join(directory, "git-trace-sentinel.log");
    process.env.GIT_TRACE = traceSentinel;
    const unpoisoned = await inspectProject(target);
    assert.equal(unpoisoned.git.status, "complete");
    assert.equal(unpoisoned.git.is_repository, false);
    await assert.rejects(access(traceSentinel), (error) => error.code === "ENOENT");

    if (process.platform !== "win32") {
      context.diagnostic("repository-local executable resolution is Windows-specific");
      return;
    }
    delete process.env.GIT_DIR;
    delete process.env.GIT_WORK_TREE;
    await writeFile(path.join(target, "git.exe"), "not a trusted executable\n", "utf8");
    const localExecutableIgnored = await inspectProject(target);
    assert.equal(localExecutableIgnored.git.status, "complete");
    assert.equal(localExecutableIgnored.git.is_repository, false);
  } finally {
    if (originalGitDirectory === undefined) delete process.env.GIT_DIR;
    else process.env.GIT_DIR = originalGitDirectory;
    if (originalGitWorkTree === undefined) delete process.env.GIT_WORK_TREE;
    else process.env.GIT_WORK_TREE = originalGitWorkTree;
    if (originalGitTrace === undefined) delete process.env.GIT_TRACE;
    else process.env.GIT_TRACE = originalGitTrace;
    await rm(directory, { recursive: true, force: true });
  }
});

test("Git inspection disables repository-defined filters and bounds the whole observation", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-project-git-filter-"));
  const marker = path.join(directory, "FILTER_RAN");
  const filterScript = path.join(directory, "filter.mjs");
  try {
    await git(directory, "init", "-b", "main");
    await git(directory, "config", "user.name", "LATTICE Test");
    await git(directory, "config", "user.email", "lattice@example.invalid");
    await writeFile(path.join(directory, ".gitattributes"), "tracked.txt filter=evil\n", "utf8");
    await writeFile(path.join(directory, "tracked.txt"), "initial\n", "utf8");
    await git(directory, "add", ".gitattributes", "tracked.txt");
    await git(directory, "commit", "-m", "filter fixture");
    await writeFile(
      filterScript,
      `import { writeFileSync } from "node:fs";\nwriteFileSync(${JSON.stringify(marker)}, "ran");\nprocess.stdin.pipe(process.stdout);\n`,
      "utf8",
    );
    const filterCommand = `"${process.execPath.replaceAll("\\", "/")}" "${filterScript.replaceAll("\\", "/")}"`;
    await git(directory, "config", "filter.evil.clean", filterCommand);
    await git(directory, "config", "filter.evil.required", "true");
    await writeFile(path.join(directory, "tracked.txt"), "changed\n", "utf8");

    const inspection = await inspectProject(directory);
    assert.equal(inspection.git.is_repository, true);
    assert.equal(inspection.git.dirty, true);
    await assert.rejects(access(marker), (error) => error.code === "ENOENT");

    await git(directory, "config", "--unset-all", "filter.evil.clean");
    await git(directory, "config", "--unset-all", "filter.evil.required");
    await git(directory, "config", "extensions.worktreeConfig", "true");
    await git(directory, "config", "--worktree", "filter.evil.clean", filterCommand);
    await git(directory, "config", "--worktree", "filter.evil.required", "true");
    const worktreeConfigured = await inspectProject(directory);
    assert.equal(worktreeConfigured.git.dirty, true);
    await assert.rejects(access(marker), (error) => error.code === "ENOENT");

    const timedOut = await inspectProject(directory, {
      gitExecutor: async () => new Promise(() => {}),
      maximumGitDurationMs: 10,
    });
    assert.equal(timedOut.git.status, "partial");
    assert.ok(timedOut.git.failures.some(
      (failure) => failure.code === "GIT_OBSERVATION_TIMEOUT",
    ));
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test("Git metadata redirects and repository-local config includes are rejected before Git runs", async (context) => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-project-git-metadata-"));
  const includedRepository = path.join(directory, "included");
  const redirectedRepository = path.join(directory, "redirected");
  const linkedRepository = path.join(directory, "linked");
  const nestedRedirectRepository = path.join(directory, "nested-redirect");
  const alternatesRepository = path.join(directory, "alternates");
  const outsideMetadata = path.join(directory, "outside-metadata");
  try {
    await mkdir(includedRepository);
    await git(includedRepository, "init", "-b", "main");
    const configPath = path.join(includedRepository, ".git", "config");
    const config = await readFile(configPath, "utf8");
    await writeFile(
      configPath,
      `${config}\n[includeIf "gitdir:**"]\n\tpath = ../outside-config\n`,
      "utf8",
    );
    let gitCalls = 0;
    const included = await inspectProject(includedRepository, {
      gitExecutor: async () => {
        gitCalls += 1;
        return { exit_code: 0, stdout: "", stderr: "" };
      },
    });
    assert.equal(gitCalls, 0);
    assert.equal(included.git.status, "partial");
    assert.equal(included.git.is_repository, null);
    assert.ok(included.git.failures.some(
      (failure) => failure.code === "GIT_CONFIG_INCLUDE_UNSAFE",
    ));

    await mkdir(redirectedRepository);
    await mkdir(outsideMetadata);
    await writeFile(
      path.join(redirectedRepository, ".git"),
      `gitdir: ${outsideMetadata}\n`,
      "utf8",
    );
    gitCalls = 0;
    const redirected = await inspectProject(redirectedRepository, {
      gitExecutor: async () => {
        gitCalls += 1;
        return { exit_code: 0, stdout: "", stderr: "" };
      },
    });
    assert.equal(gitCalls, 0);
    assert.equal(redirected.git.is_repository, null);
    assert.ok(redirected.git.failures.some(
      (failure) => failure.code === "GIT_METADATA_REDIRECTED",
    ));

    await mkdir(linkedRepository);
    try {
      await symlink(
        outsideMetadata,
        path.join(linkedRepository, ".git"),
        process.platform === "win32" ? "junction" : "dir",
      );
      gitCalls = 0;
      const linked = await inspectProject(linkedRepository, {
        gitExecutor: async () => {
          gitCalls += 1;
          return { exit_code: 0, stdout: "", stderr: "" };
        },
      });
      assert.equal(gitCalls, 0);
      assert.equal(linked.git.is_repository, null);
      assert.ok(linked.git.failures.some(
        (failure) => failure.code === "GIT_METADATA_REDIRECTED",
      ));
    } catch (error) {
      if (!["EPERM", "EACCES", "ENOTSUP"].includes(error?.code)) throw error;
      context.diagnostic("Git metadata junction creation is unavailable on this host");
    }

    await mkdir(nestedRedirectRepository);
    await git(nestedRedirectRepository, "init", "-b", "main");
    const objectsPath = path.join(nestedRedirectRepository, ".git", "objects");
    const outsideObjects = path.join(directory, "outside-objects");
    await rename(objectsPath, outsideObjects);
    try {
      await symlink(
        outsideObjects,
        objectsPath,
        process.platform === "win32" ? "junction" : "dir",
      );
      gitCalls = 0;
      const nestedRedirect = await inspectProject(nestedRedirectRepository, {
        gitExecutor: async () => {
          gitCalls += 1;
          return { exit_code: 0, stdout: "", stderr: "" };
        },
      });
      assert.equal(gitCalls, 0);
      assert.equal(nestedRedirect.git.is_repository, null);
      assert.ok(nestedRedirect.git.failures.some(
        (failure) => failure.code === "GIT_METADATA_REDIRECTED",
      ));
    } catch (error) {
      if (!["EPERM", "EACCES", "ENOTSUP"].includes(error?.code)) throw error;
      context.diagnostic("Nested Git metadata junction creation is unavailable on this host");
    }

    await mkdir(alternatesRepository);
    await git(alternatesRepository, "init", "-b", "main");
    await writeFile(
      path.join(alternatesRepository, ".git", "objects", "info", "alternates"),
      `${outsideObjects}\n`,
      "utf8",
    );
    gitCalls = 0;
    const alternates = await inspectProject(alternatesRepository, {
      gitExecutor: async () => {
        gitCalls += 1;
        return { exit_code: 0, stdout: "", stderr: "" };
      },
    });
    assert.equal(gitCalls, 0);
    assert.equal(alternates.git.is_repository, null);
    assert.ok(alternates.git.failures.some(
      (failure) => failure.code === "GIT_METADATA_REDIRECTED",
    ));
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

async function linkedWorktreeFixture(directory) {
  const repository = path.join(directory, "repository");
  const worktree = path.join(directory, "linked");
  await mkdir(repository);
  await git(repository, "init", "-b", "main");
  await git(repository, "config", "core.autocrlf", "false");
  await git(repository, "config", "user.name", "LATTICE Test");
  await git(repository, "config", "user.email", "lattice@example.invalid");
  await writeFile(path.join(repository, "AGENTS.md"), "# Linked worktree rules\n");
  await writeFile(path.join(repository, ".gitattributes"), "tracked.txt filter=evil\n");
  await writeFile(path.join(repository, "tracked.txt"), "initial\n");
  await git(repository, "add", ".");
  await git(repository, "commit", "-m", "linked worktree fixture");
  await git(repository, "worktree", "add", "-b", "linked", worktree);
  const marker = path.join(worktree, ".git");
  const metadata = (await readFile(marker, "utf8")).trim().slice("gitdir: ".length);
  return { repository, worktree, marker, metadata };
}

test("Git linked worktrees report their own root and branch and disable worktree filters", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-project-linked-worktree-"));
  try {
    const { repository, worktree, marker, metadata } = await linkedWorktreeFixture(directory);
    const originalMarker = await readFile(marker);
    const originalIndex = await readFile(path.join(metadata, "index"));
    const clean = await inspectProject(worktree);
    assert.equal(clean.git.status, "complete", JSON.stringify(clean.git.failures));
    assert.equal(clean.repo_root, await realpath(worktree));
    assert.equal(clean.git.branch, "linked");
    assert.equal(clean.git.dirty, false);
    assert.equal(clean.git.head_sha, (await git(worktree, "rev-parse", "HEAD")).stdout.trim());

    const sentinel = path.join(directory, "FILTER_RAN");
    const filter = path.join(directory, "filter.mjs");
    await writeFile(filter, `import { writeFileSync } from "node:fs";\nwriteFileSync(${JSON.stringify(sentinel)}, "ran");\nprocess.stdin.pipe(process.stdout);\n`);
    await git(repository, "config", "extensions.worktreeConfig", "true");
    await git(worktree, "config", "--worktree", "filter.evil.clean",
      `"${process.execPath.replaceAll("\\", "/")}" "${filter.replaceAll("\\", "/")}"`);
    await git(worktree, "config", "--worktree", "filter.evil.required", "true");
    await writeFile(path.join(worktree, "tracked.txt"), "changed\n");
    const nested = path.join(worktree, "nested");
    await mkdir(nested);
    const dirty = await inspectProject(nested);
    assert.equal(dirty.git.status, "complete", JSON.stringify(dirty.git.failures));
    assert.equal(dirty.repo_root, clean.repo_root);
    assert.equal(dirty.git.branch, "linked");
    assert.equal(dirty.git.dirty, true);
    await assert.rejects(access(sentinel), (error) => error.code === "ENOENT");
    assert.deepEqual(await readFile(marker), originalMarker);
    assert.deepEqual(await readFile(path.join(metadata, "index")), originalIndex);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test("Git linked worktrees reject broken back-links and configuration includes before Git", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-project-worktree-boundary-"));
  try {
    const { repository, worktree, metadata } = await linkedWorktreeFixture(directory);
    const variants = [
      [path.join(metadata, "gitdir"), `${path.join(repository, ".git")}\n`, "GIT_METADATA_REDIRECTED"],
      [path.join(metadata, "commondir"), `${repository}\n`, "GIT_METADATA_REDIRECTED"],
      [path.join(metadata, "commondir"), "nested/../../..\n", "GIT_METADATA_REDIRECTED"],
      [path.join(repository, ".git", "config"), '[include]\npath = outside\n', "GIT_CONFIG_INCLUDE_UNSAFE"],
      [path.join(metadata, "config.worktree"), '[includeIf "gitdir:**"]\npath = outside\n', "GIT_CONFIG_INCLUDE_UNSAFE"],
    ];
    for (const [file, replacement, expectedCode] of variants) {
      const original = await readFile(file).catch((error) => {
        if (error.code === "ENOENT") return null;
        throw error;
      });
      try {
        await writeFile(file, replacement);
        let gitCalls = 0;
        const inspection = await inspectProject(worktree, {
          gitExecutor: async () => {
            gitCalls += 1;
            return { exit_code: 0, stdout: "", stderr: "" };
          },
        });
        assert.equal(gitCalls, 0, file);
        assert.equal(inspection.git.is_repository, null);
        assert.ok(inspection.git.failures.some((failure) => failure.code === expectedCode), file);
      } finally {
        if (original === null) await rm(file);
        else await writeFile(file, original);
      }
    }
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test("Git linked worktree metadata changes invalidate observation before another Git command", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-project-worktree-changed-"));
  try {
    const { worktree, metadata } = await linkedWorktreeFixture(directory);
    for (const file of [path.join(metadata, "gitdir"), path.join(metadata, "HEAD")]) {
      const original = await readFile(file, "utf8");
      let gitCalls = 0;
      try {
        const inspection = await inspectProject(worktree, {
          gitExecutor: async (request) => {
            gitCalls += 1;
            const result = await defaultGitExecutor(request);
            if (gitCalls === 1) await writeFile(file, `${original}\n`);
            return result;
          },
        });
        assert.equal(gitCalls, 1, file);
        assert.equal(inspection.repo_root, null);
        assert.equal(inspection.git.is_repository, null);
        assert.ok(inspection.git.failures.some((failure) => /^GIT_METADATA_/u.test(failure.code)));
      } finally {
        await writeFile(file, original);
      }
    }
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test("Git core.worktree cannot expand the observed repository root beyond its metadata boundary", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-project-git-worktree-root-"));
  const repository = path.join(directory, "repository");
  try {
    await mkdir(repository);
    await git(repository, "init", "-b", "main");
    await git(repository, "config", "core.worktree", directory);
    const inspection = await inspectProject(repository);
    assert.equal(inspection.repo_root, null);
    assert.equal(inspection.git.status, "partial");
    assert.equal(inspection.git.is_repository, null);
    assert.ok(inspection.git.failures.some(
      (failure) => failure.code === "GIT_REPOSITORY_ROOT_INVALID",
    ));
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test("Git metadata scanning shares the whole observation deadline and stops before Git", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-project-git-metadata-timeout-"));
  try {
    await git(directory, "init", "-b", "main");
    let ticks = 0;
    let gitCalls = 0;
    const inspection = await inspectProject(directory, {
      gitExecutor: async () => {
        gitCalls += 1;
        return { exit_code: 0, stdout: "", stderr: "" };
      },
      maximumGitDurationMs: 10,
      gitMonotonicClock: () => ticks++,
    });
    assert.equal(gitCalls, 0);
    assert.equal(inspection.git.status, "partial");
    assert.equal(inspection.git.is_repository, null);
    assert.ok(inspection.git.failures.some(
      (failure) => failure.code === "GIT_OBSERVATION_TIMEOUT",
    ));
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test("submodule working tree state is explicitly unobserved instead of reported complete and clean", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-project-submodule-"));
  const child = path.join(directory, "child-source");
  const parent = path.join(directory, "parent");
  try {
    await mkdir(child);
    await git(child, "init", "-b", "main");
    await git(child, "config", "user.name", "LATTICE Test");
    await git(child, "config", "user.email", "lattice@example.invalid");
    await writeFile(path.join(child, "tracked.txt"), "child\n", "utf8");
    await git(child, "add", ".");
    await git(child, "commit", "-m", "child");
    await mkdir(parent);
    await git(parent, "init", "-b", "main");
    await git(parent, "config", "user.name", "LATTICE Test");
    await git(parent, "config", "user.email", "lattice@example.invalid");
    await git(parent, "-c", "protocol.file.allow=always", "submodule", "add", child, "nested");
    await git(parent, "commit", "-am", "submodule");
    await writeFile(path.join(parent, "nested", "tracked.txt"), "dirty child\n", "utf8");

    const inspection = await inspectProject(parent);
    assert.equal(inspection.git.status, "partial");
    assert.equal(inspection.git.dirty, null);
    assert.ok(inspection.git.failures.some(
      (failure) => failure.code === "GIT_SUBMODULE_STATE_NOT_OBSERVED",
    ));
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test("WSL2 project paths stay in the bound Linux Git domain", async (context) => {
  if (process.platform !== "win32") {
    context.skip("WSL2 UNC namespaces are Windows-only");
    return;
  }
  const windowsPath = "\\\\wsl.localhost\\Ubuntu\\home\\zk\\phase4-source";
  const identity = parseWsl2ProjectPath(windowsPath);
  assert.equal(identity.schema, "lattice.wsl2-project-path/1.0");
  assert.equal(identity.distribution, "Ubuntu");
  assert.equal(identity.linux_path, "/home/zk/phase4-source");
  assert.match(identity.identity_ref, /^wsl2-project-path:sha256:[a-f0-9]{64}$/u);
  assert.equal(normalizeRequestedProjectPath(windowsPath), windowsPath);
  assert.equal(parseWsl2ProjectPath("\\\\wsl.localhost\\Ubuntu\\mnt\\c\\phase4"), null);
  assert.throws(
    () => normalizeRequestedProjectPath("\\\\wsl.localhost\\Ubuntu\\mnt\\c\\phase4"),
    (error) => error instanceof ProjectInspectionError
      && error.code === "PROJECT_PATH_UNSAFE_NAMESPACE",
  );

  let observed = null;
  const executor = createWsl2ProjectGitExecutor(identity, {
    systemRoot: "C:\\Windows",
    executeFile: async (executable, args, options) => {
      observed = { executable, args, options };
      return { stdout: "/home/zk/phase4-source\n", stderr: "" };
    },
  });
  const result = await executor({
    cwd: windowsPath,
    args: ["rev-parse", "--show-toplevel"],
    timeoutMs: 1_000,
  });
  assert.equal(result.exit_code, 0);
  assert.equal(result.stdout, `${windowsPath}\n`);
  assert.equal(observed.executable, "C:\\Windows\\System32\\wsl.exe");
  assert.deepEqual(observed.args.slice(0, 5), [
    "-d", "Ubuntu", "--exec", "/usr/bin/env", "-i",
  ]);
  assert.equal(observed.args.includes("/usr/bin/git"), true);
  assert.equal(observed.args.includes("-C"), true);
  assert.equal(observed.args.includes("/home/zk/phase4-source"), true);
  assert.deepEqual(Object.keys(observed.options.env).sort(), ["SystemRoot", "WINDIR"].filter(
    (key) => process.env[key] !== undefined,
  ).sort());
  await assert.rejects(() => executor({
    cwd: "\\\\wsl.localhost\\Ubuntu-Other\\home\\zk\\phase4-source",
    args: ["status"],
    timeoutMs: 1_000,
  }), /bound Linux project domain/u);
});

test("inspection explains non-Git, missing, and redirected project paths without following links", async (context) => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-project-paths-"));
  const nonGit = path.join(directory, "non-git");
  const outside = path.join(directory, "outside");
  const redirectedRuleDirectory = path.join(nonGit, "redirected-rules");
  const redirectedRoot = path.join(directory, "redirected-root");
  try {
    await mkdir(nonGit);
    await mkdir(outside);
    await writeFile(path.join(outside, "AGENTS.md"), "# Outside rules\n", "utf8");

    const plain = await inspectProject(nonGit);
    assert.equal(plain.git.status, "complete");
    assert.equal(plain.git.is_repository, false);
    assert.equal(plain.repo_root, null);

    await assert.rejects(
      inspectProject(path.join(directory, "missing")),
      (error) => error instanceof ProjectInspectionError && error.code === "PROJECT_PATH_NOT_FOUND",
    );
    if (process.platform === "win32") {
      for (const unsafe of [
        "\\\\example.invalid\\share\\project",
        "\\\\.\\GLOBALROOT\\Device\\HarddiskVolumeShadowCopy1",
        "\\\\?\\UNC\\example.invalid\\share\\project",
      ]) {
        assert.throws(
          () => normalizeRequestedProjectPath(unsafe),
          (error) => error instanceof ProjectInspectionError
            && error.code === "PROJECT_PATH_UNSAFE_NAMESPACE",
        );
      }
    }

    try {
      await symlink(outside, redirectedRuleDirectory, "junction");
      await symlink(nonGit, redirectedRoot, "junction");
    } catch (error) {
      if (["EPERM", "EACCES", "ENOTSUP"].includes(error?.code)) {
        context.skip(`link creation unavailable: ${error.code}`);
        return;
      }
      throw error;
    }

    const linkedRules = await inspectProject(nonGit);
    assert.equal(linkedRules.rules.status, "partial");
    assert.ok(linkedRules.rules.failures.some(
      (failure) => failure.code === "RULE_PATH_REDIRECTED"
        && failure.relative_path === "redirected-rules",
    ));
    assert.equal(linkedRules.rules.documents.some(
      (document) => document.relative_path.endsWith("AGENTS.md"),
    ), false);
    await assert.rejects(
      inspectProject(redirectedRoot),
      (error) => error instanceof ProjectInspectionError
        && error.code === "PROJECT_PATH_REDIRECTED",
    );
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test("rule discovery bounds documents, total bytes, time, failures, and opened-handle reads", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-project-rule-limits-"));
  try {
    for (const name of ["A_RULES.md", "B_RULES.md", "C_RULES.md"]) {
      await writeFile(path.join(directory, name), "rule", "utf8");
    }
    await writeFile(path.join(directory, "AGENTS.md"), "agents", "utf8");

    const documentLimited = await inspectProject(directory, {
      ruleLimits: { maximumDocuments: 2 },
    });
    assert.equal(documentLimited.rules.documents.length, 2);
    assert.equal(documentLimited.rules.status, "partial");
    assert.ok(documentLimited.rules.failures.some(
      (failure) => failure.code === "RULE_DOCUMENT_LIMIT_EXCEEDED",
    ));
    assert.equal(documentLimited.rules.missing_standard_documents.includes("AGENTS.md"), false);

    const byteLimited = await inspectProject(directory, {
      ruleLimits: { maximumTotalBytes: 5 },
    });
    assert.ok(byteLimited.rules.documents.length <= 1);
    assert.ok(byteLimited.rules.failures.some(
      (failure) => failure.code === "RULE_TOTAL_BYTES_EXCEEDED",
    ));

    let monotonicTick = 0;
    const timeLimited = await inspectProject(directory, {
      ruleLimits: { maximumDurationMs: 1 },
      monotonicClock: () => monotonicTick++,
    });
    assert.equal(timeLimited.rules.status, "partial");
    assert.ok(timeLimited.rules.failures.some(
      (failure) => failure.code === "RULE_SCAN_TIMEOUT",
    ));

    const singleRuleDirectory = path.join(directory, "single");
    await mkdir(singleRuleDirectory);
    const rulePath = path.join(singleRuleDirectory, "AGENTS.md");
    await writeFile(rulePath, "safe", "utf8");
    const initialStat = await stat(rulePath, { bigint: true });
    let bytesOffered = 0;
    let handleClosed = false;
    const oversizedHandle = {
      async stat() { return initialStat; },
      async read(buffer, offset, length) {
        buffer.fill(0x78, offset, offset + length);
        bytesOffered += length;
        return { bytesRead: length, buffer };
      },
      async close() { handleClosed = true; },
    };
    const boundedRead = await inspectProject(singleRuleDirectory, {
      ruleLimits: { maximumFileBytes: 8 },
      ruleFileOpener: async () => oversizedHandle,
    });
    assert.equal(boundedRead.rules.documents.length, 0);
    assert.ok(boundedRead.rules.failures.some(
      (failure) => failure.code === "RULE_DOCUMENT_TOO_LARGE",
    ));
    assert.ok(bytesOffered <= 9, `opened handle offered ${bytesOffered} bytes`);
    assert.equal(handleClosed, true);

    let mismatchedReadCalled = false;
    const mismatchedStat = {
      dev: initialStat.dev,
      ino: initialStat.ino + 1n,
      size: initialStat.size,
      mtimeMs: initialStat.mtimeMs,
    };
    const changedHandle = {
      async stat() { return mismatchedStat; },
      async read() {
        mismatchedReadCalled = true;
        return { bytesRead: 0, buffer: Buffer.alloc(0) };
      },
      async close() {},
    };
    const changedDuringOpen = await inspectProject(singleRuleDirectory, {
      ruleFileOpener: async () => changedHandle,
    });
    assert.equal(changedDuringOpen.rules.documents.length, 0);
    assert.ok(changedDuringOpen.rules.failures.some(
      (failure) => failure.code === "RULE_DOCUMENT_CHANGED",
    ));
    assert.equal(mismatchedReadCalled, false);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test("future and drifted Control schemas fail closed before mutation and release the database", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-project-schema-"));
  try {
    for (const [filename, version, malformedV0] of [
      ["future.db", 8, false],
      ["negative.db", -1, false],
      ["drifted.db", 1, false],
      ["malformed-v0.db", 0, true],
    ]) {
      const databasePath = path.join(directory, filename);
      const seed = new DatabaseSync(databasePath);
      seed.exec(`
        CREATE TABLE sentinel (value TEXT);
        ${malformedV0 ? "CREATE TABLE project_rule_documents (x TEXT);" : ""}
        PRAGMA user_version = ${version};
      `);
      seed.close();

      assert.throws(
        () => new LatticeStore(databasePath),
        version !== 0 && version !== 1 ? /unsupported/u : /schema profile/u,
      );
      const verify = new DatabaseSync(databasePath, { readOnly: true });
      const schema = verify.prepare(`
        SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name
      `).all().map((entry) => entry.name);
      verify.close();
      assert.deepEqual(schema, malformedV0 ? ["project_rule_documents", "sentinel"] : ["sentinel"]);
      const versionCheck = new DatabaseSync(databasePath, { readOnly: true });
      assert.equal(versionCheck.prepare("PRAGMA user_version").get().user_version, version);
      versionCheck.close();
      const moved = `${databasePath}.moved`;
      await rename(databasePath, moved);
      await rename(moved, databasePath);
    }
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test("exact Control schema validation rejects semantic literal and trigger-body drift", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-project-exact-schema-"));
  try {
    for (const drift of ["literal", "trigger"]) {
      const databasePath = path.join(directory, `${drift}.db`);
      const store = new LatticeStore(databasePath);
      store.close();
      const database = new DatabaseSync(databasePath);
      try {
        if (drift === "literal") {
          const schemaVersion = database.prepare("PRAGMA schema_version").get().schema_version;
          database.enableDefensive(false);
          database.exec("PRAGMA writable_schema = ON;");
          database.prepare(`
            UPDATE sqlite_master
            SET sql = replace(sql, ?, ?)
            WHERE type = 'table' AND name = 'installation_receipts'
          `).run("'NON_AUTHORITATIVE'", "'non_authoritative'");
          database.exec(`PRAGMA schema_version = ${schemaVersion + 1};`);
          database.exec("PRAGMA writable_schema = OFF;");
        } else {
          database.exec(`
            DROP TRIGGER installation_receipts_no_update;
            CREATE TRIGGER installation_receipts_no_update
            BEFORE UPDATE ON installation_receipts
            BEGIN
              SELECT 1;
            END;
          `);
        }
      } finally {
        database.close();
      }
      assert.throws(() => new LatticeStore(databasePath), /exact SQL manifest/u);
    }
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test("legacy Control data migrates in place and a fresh process reads the same registered identity", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-project-store-"));
  const databasePath = path.join(directory, "control.db");
  const projectPath = path.join(directory, "project");
  const legacyId = "legacy-project-id";
  const createdAt = "2026-01-02T03:04:05.000Z";
  let store;
  try {
    await mkdir(projectPath);
    const canonicalPath = await realpath(projectPath);
    const legacy = new DatabaseSync(databasePath);
    legacy.exec(`
      PRAGMA foreign_keys = ON;
      CREATE TABLE projects (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        root_path TEXT NOT NULL,
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL
      );
    `);
    legacy.prepare(`
      INSERT INTO projects (id, name, root_path, created_at, updated_at)
      VALUES (?, ?, ?, ?, ?)
    `).run(legacyId, "Legacy", canonicalPath, createdAt, createdAt);
    legacy.close();

    const observationTime = "2026-08-26T04:00:00.000Z";
    const inspection = {
      canonical_path: canonicalPath,
      repo_root: null,
      git: {
        status: "complete",
        is_repository: false,
        branch: null,
        detached: null,
        head_sha: null,
        dirty: null,
        upstream: null,
        ahead: null,
        behind: null,
        remotes: [],
        observed_at: observationTime,
        failures: [],
      },
      rules: {
        status: "complete",
        observed_at: observationTime,
        documents: [{
          relative_path: "AGENTS.md",
          sha256: "a".repeat(64),
          observed_at: observationTime,
          purpose: "Codex and agent working rules",
        }],
        missing_standard_documents: ["PROJECT_STATE.md", "PLANS.md"],
        failures: [],
      },
    };

    store = new LatticeStore(databasePath);
    assert.equal(store.database.prepare("PRAGMA user_version").get().user_version, 7);
    const registered = store.registerProject({ name: "Migrated", inspection });
    assert.equal(registered.created, false);
    assert.equal(registered.project.id, legacyId);
    assert.equal(registered.project.created_at, createdAt);
    assert.equal(registered.project.updated_at, observationTime);
    assert.equal(registered.project.canonical_path, canonicalPath);
    assert.equal(registered.project.repo_root, null);
    assertControlCatalogLocator(registered.project);
    assert.equal(registered.project.git_observation.is_repository, false);
    assert.deepEqual(
      registered.project.rule_index.missing_standard_documents,
      ["PROJECT_STATE.md", "PLANS.md"],
    );
    store.close();
    store = null;

    const storeModule = pathToFileURL(
      path.resolve(import.meta.dirname, "../src/store.mjs"),
    ).href;
    const childScript = `
      import { LatticeStore } from ${JSON.stringify(storeModule)};
      const [databasePath, projectId] = process.argv.slice(-2);
      const store = new LatticeStore(databasePath);
      process.stdout.write(JSON.stringify(store.getProjectRegistration(projectId)));
      store.close();
    `;
    const replay = JSON.parse((await execFileAsync(
      process.execPath,
      ["--input-type=module", "--eval", childScript, databasePath, legacyId],
      { encoding: "utf8", windowsHide: true },
    )).stdout);
    assert.equal(replay.id, legacyId);
    assertControlCatalogLocator(replay);
    assert.equal(replay.created_at, createdAt);
    assert.equal(replay.canonical_path, canonicalPath);
    assert.equal(replay.git_observation.observed_at, observationTime);
    assert.deepEqual(replay.rule_index.documents, inspection.rules.documents);

    const mixedVersion = new DatabaseSync(databasePath);
    mixedVersion.prepare(`
      INSERT INTO projects (id, name, root_path, created_at, updated_at)
      VALUES (?, ?, ?, ?, ?)
    `).run("old-binary-row", "Old binary", path.join(directory, "old"), createdAt, createdAt);
    mixedVersion.close();
    const mixedStore = new LatticeStore(databasePath);
    const oldRow = mixedStore.listProjects().find((project) => project.id === "old-binary-row");
    assert.equal(oldRow.record_kind, "LEGACY_CONTROL_PROJECT");
    assert.equal(oldRow.registry_authority, "NONE");
    assert.equal(oldRow.schema_version, null);
    mixedStore.close();
  } finally {
    store?.close();
    await rm(directory, { recursive: true, force: true });
  }
});

test("invalid legacy project IDs cannot be partially adopted", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-project-invalid-legacy-"));
  const databasePath = path.join(directory, "control.db");
  let store;
  try {
    const paths = [path.join(directory, "oversized"), path.join(directory, "control")];
    await Promise.all(paths.map((projectPath) => mkdir(projectPath)));
    const canonicalPaths = await Promise.all(paths.map((projectPath) => realpath(projectPath)));
    const legacy = new DatabaseSync(databasePath);
    legacy.exec(`
      CREATE TABLE projects (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        root_path TEXT NOT NULL,
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL
      );
    `);
    const insert = legacy.prepare(`
      INSERT INTO projects (id, name, root_path, created_at, updated_at)
      VALUES (?, ?, ?, ?, ?)
    `);
    const invalidIds = ["x".repeat(257), "control\u001bid"];
    for (let index = 0; index < invalidIds.length; index += 1) {
      insert.run(
        invalidIds[index],
        `Legacy ${index}`,
        canonicalPaths[index],
        "2026-01-02T03:04:05.000Z",
        "2026-01-02T03:04:05.000Z",
      );
    }
    legacy.close();

    store = new LatticeStore(databasePath);
    for (let index = 0; index < invalidIds.length; index += 1) {
      assert.throws(
        () => store.registerProject({
          name: `Rejected ${index}`,
          inspection: fixtureInspection(
            canonicalPaths[index],
            `2026-08-26T04:00:0${index}.000Z`,
          ),
        }),
        /project ID is too long or contains unsafe control characters/u,
      );
    }
    assert.equal(store.database.prepare(`
      SELECT COUNT(*) AS count FROM project_registration_details
    `).get().count, 0);
    assert.equal(store.database.prepare(`
      SELECT COUNT(*) AS count FROM project_observations
    `).get().count, 0);
    assert.equal(store.database.prepare(`
      SELECT COUNT(*) AS count
      FROM projects
      WHERE name LIKE 'Rejected %'
    `).get().count, 0);
  } finally {
    store?.close();
    await rm(directory, { recursive: true, force: true });
  }
});

test("stale refresh completion cannot replace a newer observation or erase a known repository root", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-project-monotonic-"));
  const databasePath = path.join(directory, "control.db");
  let store;
  try {
    const canonicalPath = await realpath(directory);
    store = new LatticeStore(databasePath);
    const registered = store.registerProject({
      name: "Monotonic",
      inspection: fixtureInspection(
        canonicalPath,
        "2026-08-26T01:00:00.000Z",
        { headSha: "0".repeat(40) },
      ),
    }).project;
    store.refreshProject({
      projectId: registered.id,
      inspection: fixtureInspection(
        canonicalPath,
        "2026-08-26T03:00:00.000Z",
        { headSha: "2".repeat(40) },
      ),
    });
    assert.throws(() => store.refreshProject({
      projectId: registered.id,
      inspection: fixtureInspection(
        canonicalPath,
        "2026-08-26T02:00:00.000Z",
        { headSha: "1".repeat(40) },
      ),
    }), (error) => error.code === "PROJECT_REFRESH_SUPERSEDED");
    const current = store.getProjectRegistration(registered.id);
    assert.equal(current.git_observation.observed_at, "2026-08-26T03:00:00.000Z");
    assert.equal(current.git_observation.head_sha, "2".repeat(40));
    assert.equal(current.updated_at, "2026-08-26T03:00:00.000Z");
    assert.equal(current.repo_root, canonicalPath);

    const partialTime = "2026-08-26T04:00:00.000Z";
    store.refreshProject({
      projectId: registered.id,
      inspection: fixtureInspection(canonicalPath, partialTime, {
        isRepository: null,
        gitStatus: "partial",
        gitFailures: [{
          stage: "repository",
          code: "GIT_UNAVAILABLE",
          message: "Git executable is unavailable",
        }],
      }),
    });
    const partial = store.getProjectRegistration(registered.id);
    assert.equal(partial.repo_root, canonicalPath);
    assert.equal(partial.git_observation.is_repository, null);
    assert.equal(partial.git_observation.observed_at, partialTime);
    assert.equal(
      store.database.prepare(
        "SELECT COUNT(*) AS count FROM project_observations WHERE project_id = ?",
      ).get(registered.id).count,
      1,
    );

    const olderSuccess = store.beginProjectRefresh(registered.id);
    const newerFailure = store.beginProjectRefresh(registered.id);
    store.recordProjectRefreshFailure({
      projectId: registered.id,
      code: "NEWER_FAILURE",
      message: "newer refresh failed",
      observedAt: "2026-08-26T06:00:00.000Z",
      attemptGeneration: newerFailure,
    });
    assert.throws(() => store.refreshProject({
      projectId: registered.id,
      inspection: fixtureInspection(
        canonicalPath,
        "2026-08-26T05:00:00.000Z",
        { headSha: "5".repeat(40) },
      ),
      attemptGeneration: olderSuccess,
    }), (error) => error.code === "PROJECT_REFRESH_SUPERSEDED");
    assert.equal(
      store.getProjectRegistration(registered.id).last_refresh_failure.code,
      "NEWER_FAILURE",
    );

    const olderFailure = store.beginProjectRefresh(registered.id);
    const newerSuccess = store.beginProjectRefresh(registered.id);
    store.refreshProject({
      projectId: registered.id,
      inspection: fixtureInspection(
        canonicalPath,
        "2026-08-26T08:00:00.000Z",
        { headSha: "8".repeat(40) },
      ),
      attemptGeneration: newerSuccess,
    });
    assert.throws(() => store.recordProjectRefreshFailure({
      projectId: registered.id,
      code: "OLDER_FAILURE",
      message: "older refresh failed late",
      observedAt: "2026-08-26T07:00:00.000Z",
      attemptGeneration: olderFailure,
    }), (error) => error.code === "PROJECT_REFRESH_SUPERSEDED");
    let raced = store.getProjectRegistration(registered.id);
    assert.equal(raced.last_refresh_failure, null);
    assert.equal(raced.git_observation.head_sha, "8".repeat(40));

    const sameMillisecond = "2026-08-26T09:00:00.000Z";
    const firstSameTime = store.beginProjectRefresh(registered.id);
    store.refreshProject({
      projectId: registered.id,
      inspection: fixtureInspection(canonicalPath, sameMillisecond, {
        headSha: "9".repeat(40),
      }),
      attemptGeneration: firstSameTime,
    });
    const secondSameTime = store.beginProjectRefresh(registered.id);
    store.refreshProject({
      projectId: registered.id,
      inspection: fixtureInspection(canonicalPath, sameMillisecond, {
        headSha: "a".repeat(40),
      }),
      attemptGeneration: secondSameTime,
    });
    raced = store.getProjectRegistration(registered.id);
    assert.equal(raced.git_observation.head_sha, "a".repeat(40));

    const clockMovedBackward = store.beginProjectRefresh(registered.id);
    store.refreshProject({
      projectId: registered.id,
      inspection: fixtureInspection(canonicalPath, "2026-08-26T00:30:00.000Z", {
        headSha: "b".repeat(40),
      }),
      attemptGeneration: clockMovedBackward,
    });
    assert.equal(
      store.getProjectRegistration(registered.id).git_observation.head_sha,
      "b".repeat(40),
    );
  } finally {
    store?.close();
    await rm(directory, { recursive: true, force: true });
  }
});

test("project registration reads one SQLite snapshot while another Control connection refreshes", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-project-read-snapshot-"));
  const databasePath = path.join(directory, "control.db");
  let reader;
  let writer;
  try {
    const canonicalPath = await realpath(directory);
    const oldInspection = fixtureInspection(
      canonicalPath,
      "2026-08-26T10:00:00.000Z",
      {
        headSha: "c".repeat(40),
        remotes: [{
          name: "origin",
          direction: "fetch",
          url: "https://example.invalid/old.git",
          credentials_redacted: false,
        }],
      },
    );
    reader = new LatticeStore(databasePath);
    const project = reader.registerProject({ name: "Snapshot", inspection: oldInspection }).project;
    writer = new LatticeStore(databasePath);
    let refreshedDuringRead = false;
    reader.database.setAuthorizer((actionCode, tableName) => {
      if (
        !refreshedDuringRead
        && actionCode === sqliteConstants.SQLITE_READ
        && tableName === "project_git_remotes"
      ) {
        refreshedDuringRead = true;
        writer.refreshProject({
          projectId: project.id,
          inspection: fixtureInspection(
            canonicalPath,
            "2026-08-26T11:00:00.000Z",
            {
              headSha: "d".repeat(40),
              remotes: [{
                name: "origin",
                direction: "fetch",
                url: "https://example.invalid/new.git",
                credentials_redacted: false,
              }],
            },
          ),
        });
      }
      return sqliteConstants.SQLITE_OK;
    });
    const snapshot = reader.getProjectRegistration(project.id);
    reader.database.setAuthorizer(null);
    assert.equal(refreshedDuringRead, true);
    assert.equal(snapshot.git_observation.head_sha, "c".repeat(40));
    assert.deepEqual(
      snapshot.git_observation.remotes.map((remote) => remote.url),
      ["https://example.invalid/old.git"],
    );
    const current = reader.getProjectRegistration(project.id);
    assert.equal(current.git_observation.head_sha, "d".repeat(40));
    assert.deepEqual(
      current.git_observation.remotes.map((remote) => remote.url),
      ["https://example.invalid/new.git"],
    );
  } finally {
    reader?.database.setAuthorizer(null);
    writer?.close();
    reader?.close();
    await rm(directory, { recursive: true, force: true });
  }
});

test("project API rejects cross-origin, rebound-host, and simple-content mutations before inspection", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-project-http-boundary-"));
  const databasePath = path.join(directory, "control.db");
  let inspections = 0;
  let application;
  try {
    const canonicalPath = await realpath(directory);
    application = createLatticeServer({
      databasePath,
      codex: new IdleCodex(),
      projectInspector: async () => {
        inspections += 1;
        return fixtureInspection(canonicalPath, "2026-08-26T05:00:00.000Z");
      },
    });
    const origin = await listen(application);
    const payload = JSON.stringify({ name: "Guarded", rootPath: canonicalPath });
    const reboundStatus = await rawHttpStatus(`${origin}/api/projects`, {
      method: "POST",
      headers: { host: "evil.invalid", "content-type": "application/json" },
      body: payload,
    });
    assert.equal(reboundStatus, 403);
    assert.equal(await rawHttpStatus(`${origin}/api/state`, {
      headers: { host: "evil.invalid" },
    }), 403);
    const crossOrigin = await fetch(`${origin}/api/projects`, {
      method: "POST",
      headers: { origin: "https://evil.invalid", "content-type": "application/json" },
      body: payload,
    });
    assert.equal(crossOrigin.status, 403);
    const simpleContent = await fetch(`${origin}/api/projects`, {
      method: "POST",
      headers: { "content-type": "text/plain" },
      body: payload,
    });
    assert.equal(simpleContent.status, 415);
    assert.equal(inspections, 0);

    const invalidName = await fetch(`${origin}/api/projects`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ name: "unsafe\u001bname", rootPath: canonicalPath }),
    });
    assert.equal(invalidName.status, 400);
    assert.equal(inspections, 0);

    const accepted = await fetch(`${origin}/api/projects`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: payload,
    });
    assert.equal(accepted.status, 201);
    const project = await accepted.json();
    assertControlCatalogLocator(project);
    assert.equal(inspections, 1);
    assert.equal(await rawHttpStatus(
      `${origin}/api/projects/${encodeURIComponent(project.id)}`,
      { headers: { host: "evil.invalid" } },
    ), 403);

    const rejectedRefresh = await fetch(
      `${origin}/api/projects/${encodeURIComponent(project.id)}/refresh`,
      {
        method: "POST",
        headers: { origin: "https://evil.invalid", "content-type": "application/json" },
        body: "{}",
      },
    );
    assert.equal(rejectedRefresh.status, 403);
    const simpleRefresh = await fetch(
      `${origin}/api/projects/${encodeURIComponent(project.id)}/refresh`,
      { method: "POST", headers: { "content-type": "text/plain" }, body: "{}" },
    );
    assert.equal(simpleRefresh.status, 415);
    assert.equal(inspections, 1);
  } finally {
    if (application?.server.listening) await close(application);
    await rm(directory, { recursive: true, force: true });
  }
});

test("project API rejects stale concurrent registration and refresh responses", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-project-api-races-"));
  const databasePath = path.join(directory, "control.db");
  let application;
  try {
    const canonicalPath = await realpath(directory);
    const firstRegisterEntered = deferred();
    const firstRegisterRelease = deferred();
    let registrationInspections = 0;
    application = createLatticeServer({
      databasePath,
      codex: new IdleCodex(),
      projectInspector: async () => {
        registrationInspections += 1;
        if (registrationInspections === 1) {
          firstRegisterEntered.resolve();
          return firstRegisterRelease.promise;
        }
        return fixtureInspection(canonicalPath, "2026-08-26T12:00:00.000Z", {
          headSha: "2".repeat(40),
        });
      },
    });
    const origin = await listen(application);
    const firstRequest = fetch(`${origin}/api/projects`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ name: "Older registration", rootPath: canonicalPath }),
    });
    await firstRegisterEntered.promise;
    const secondResponse = await fetch(`${origin}/api/projects`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ name: "Newer registration", rootPath: canonicalPath }),
    });
    assert.equal(secondResponse.status, 201);
    const registered = await secondResponse.json();
    firstRegisterRelease.resolve(fixtureInspection(
      canonicalPath,
      "2026-08-26T11:00:00.000Z",
      { headSha: "1".repeat(40) },
    ));
    const staleRegistration = await firstRequest;
    assert.equal(staleRegistration.status, 409);
    assert.equal(
      (await staleRegistration.json()).code,
      "PROJECT_REGISTRATION_SUPERSEDED",
    );
    let current = await (await fetch(
      `${origin}/api/projects/${encodeURIComponent(registered.id)}`,
    )).json();
    assert.equal(current.name, "Newer registration");
    assert.equal(current.git_observation.head_sha, "2".repeat(40));

    await close(application);
    const firstRefreshEntered = deferred();
    const secondRefreshEntered = deferred();
    const firstRefreshRelease = deferred();
    const secondRefreshRelease = deferred();
    let refreshInspections = 0;
    application = createLatticeServer({
      databasePath,
      codex: new IdleCodex(),
      projectInspector: async () => {
        refreshInspections += 1;
        if (refreshInspections === 1) {
          firstRefreshEntered.resolve();
          return firstRefreshRelease.promise;
        }
        secondRefreshEntered.resolve();
        return secondRefreshRelease.promise;
      },
    });
    const refreshOrigin = await listen(application);
    const firstRefresh = fetch(
      `${refreshOrigin}/api/projects/${encodeURIComponent(registered.id)}/refresh`,
      { method: "POST", headers: { "content-type": "application/json" }, body: "{}" },
    );
    await firstRefreshEntered.promise;
    const secondRefresh = fetch(
      `${refreshOrigin}/api/projects/${encodeURIComponent(registered.id)}/refresh`,
      { method: "POST", headers: { "content-type": "application/json" }, body: "{}" },
    );
    await secondRefreshEntered.promise;
    firstRefreshRelease.resolve(fixtureInspection(
      canonicalPath,
      "2026-08-26T13:00:00.000Z",
      { headSha: "3".repeat(40) },
    ));
    const staleRefresh = await firstRefresh;
    assert.equal(staleRefresh.status, 409);
    assert.equal((await staleRefresh.json()).code, "PROJECT_REFRESH_SUPERSEDED");
    current = await (await fetch(
      `${refreshOrigin}/api/projects/${encodeURIComponent(registered.id)}`,
    )).json();
    assert.equal(current.git_observation.head_sha, "2".repeat(40));
    secondRefreshRelease.resolve(fixtureInspection(
      canonicalPath,
      "2026-08-26T14:00:00.000Z",
      { headSha: "4".repeat(40) },
    ));
    const appliedRefresh = await secondRefresh;
    assert.equal(appliedRefresh.status, 200);
    assert.equal((await appliedRefresh.json()).git_observation.head_sha, "4".repeat(40));

    await close(application);
    const reregisterEntered = deferred();
    const reregisterRelease = deferred();
    let crossOperationInspections = 0;
    application = createLatticeServer({
      databasePath,
      codex: new IdleCodex(),
      projectInspector: async () => {
        crossOperationInspections += 1;
        if (crossOperationInspections === 1) {
          reregisterEntered.resolve();
          return reregisterRelease.promise;
        }
        return fixtureInspection(canonicalPath, "2026-08-26T16:00:00.000Z", {
          headSha: "6".repeat(40),
        });
      },
    });
    const crossOrigin = await listen(application);
    const staleReregister = fetch(`${crossOrigin}/api/projects`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ name: "Stale rename", rootPath: canonicalPath }),
    });
    await reregisterEntered.promise;
    const newerCrossRefresh = await fetch(
      `${crossOrigin}/api/projects/${encodeURIComponent(registered.id)}/refresh`,
      { method: "POST", headers: { "content-type": "application/json" }, body: "{}" },
    );
    assert.equal(newerCrossRefresh.status, 200);
    reregisterRelease.resolve(fixtureInspection(
      canonicalPath,
      "2026-08-26T15:00:00.000Z",
      { headSha: "5".repeat(40) },
    ));
    const staleCrossResponse = await staleReregister;
    assert.equal(staleCrossResponse.status, 409);
    assert.equal(
      (await staleCrossResponse.json()).code,
      "PROJECT_REGISTRATION_SUPERSEDED",
    );
    current = await (await fetch(
      `${crossOrigin}/api/projects/${encodeURIComponent(registered.id)}`,
    )).json();
    assert.equal(current.name, "Newer registration");
    assert.equal(current.git_observation.head_sha, "6".repeat(40));

    await close(application);
    const oldRefreshEntered = deferred();
    const oldRefreshRelease = deferred();
    let reverseCrossInspections = 0;
    application = createLatticeServer({
      databasePath,
      codex: new IdleCodex(),
      projectInspector: async () => {
        reverseCrossInspections += 1;
        if (reverseCrossInspections === 1) {
          oldRefreshEntered.resolve();
          return oldRefreshRelease.promise;
        }
        return fixtureInspection(canonicalPath, "2026-08-26T18:00:00.000Z", {
          headSha: "8".repeat(40),
        });
      },
    });
    const reverseOrigin = await listen(application);
    const oldRefresh = fetch(
      `${reverseOrigin}/api/projects/${encodeURIComponent(registered.id)}/refresh`,
      { method: "POST", headers: { "content-type": "application/json" }, body: "{}" },
    );
    await oldRefreshEntered.promise;
    const latestReregister = await fetch(`${reverseOrigin}/api/projects`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ name: "Latest registration", rootPath: canonicalPath }),
    });
    assert.equal(latestReregister.status, 200);
    oldRefreshRelease.resolve(fixtureInspection(
      canonicalPath,
      "2026-08-26T17:00:00.000Z",
      { headSha: "7".repeat(40) },
    ));
    const oldRefreshResponse = await oldRefresh;
    assert.equal(oldRefreshResponse.status, 409);
    assert.equal((await oldRefreshResponse.json()).code, "PROJECT_REFRESH_SUPERSEDED");
    current = await (await fetch(
      `${reverseOrigin}/api/projects/${encodeURIComponent(registered.id)}`,
    )).json();
    assert.equal(current.name, "Latest registration");
    assert.equal(current.git_observation.head_sha, "8".repeat(40));
  } finally {
    if (application?.server.listening) await close(application);
    await rm(directory, { recursive: true, force: true });
  }
});

test("project API reuses stable identity, refreshes observations, and reads them after Control restart", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-project-api-"));
  const repository = path.join(directory, "repository");
  const databasePath = path.join(directory, "control.db");
  let application;
  try {
    await mkdir(repository);
    await git(repository, "init", "-b", "main");
    await git(repository, "config", "user.name", "LATTICE Test");
    await git(repository, "config", "user.email", "lattice@example.invalid");
    await writeFile(path.join(repository, "AGENTS.md"), "# Rules v1\n", "utf8");
    await writeFile(path.join(repository, "PROJECT_STATE.md"), "# State v1\n", "utf8");
    await git(repository, "add", ".");
    await git(repository, "commit", "-m", "initial");

    application = createLatticeServer({ databasePath, codex: new IdleCodex() });
    let origin = await listen(application);
    const register = await fetch(`${origin}/api/projects`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ name: "Fixture", rootPath: repository }),
    });
    assert.equal(register.status, 201);
    const first = await register.json();
    assert.equal(first.created, true);
    assertControlCatalogLocator(first);
    assert.equal(first.git_observation.branch, "main");
    assert.equal(first.git_observation.dirty, false);
    const firstHead = first.git_observation.head_sha;
    const firstRuleHash = first.rule_index.documents.find(
      (document) => document.relative_path === "PROJECT_STATE.md",
    ).sha256;

    const duplicate = await fetch(`${origin}/api/projects`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ name: "Fixture renamed", rootPath: path.join(repository, ".") }),
    });
    assert.equal(duplicate.status, 200);
    const repeated = await duplicate.json();
    assert.equal(repeated.id, first.id);
    assert.equal(repeated.created, false);
    assert.equal(repeated.created_at, first.created_at);

    await writeFile(path.join(repository, "PROJECT_STATE.md"), "# State v2\n", "utf8");
    await git(repository, "add", "PROJECT_STATE.md");
    await git(repository, "commit", "-m", "refresh fixture");
    const refresh = await fetch(
      `${origin}/api/projects/${encodeURIComponent(first.id)}/refresh`,
      { method: "POST", headers: { "content-type": "application/json" }, body: "{}" },
    );
    assert.equal(refresh.status, 200);
    const refreshed = await refresh.json();
    assert.equal(refreshed.id, first.id);
    assert.notEqual(refreshed.git_observation.head_sha, firstHead);
    assert.equal(refreshed.git_observation.dirty, false);
    assert.notEqual(
      refreshed.rule_index.documents.find(
        (document) => document.relative_path === "PROJECT_STATE.md",
      ).sha256,
      firstRuleHash,
    );

    const state = await (await fetch(`${origin}/api/state`)).json();
    assert.equal(state.projects.length, 1);
    assert.equal(state.projects[0].id, first.id);
    assert.equal("rule_index" in state.projects[0], false);

    await close(application);
    application = createLatticeServer({ databasePath, codex: new IdleCodex() });
    origin = await listen(application);
    const replayResponse = await fetch(
      `${origin}/api/projects/${encodeURIComponent(first.id)}`,
    );
    assert.equal(replayResponse.status, 200);
    const replay = await replayResponse.json();
    assert.equal(replay.id, first.id);
    assert.equal(replay.git_observation.head_sha, refreshed.git_observation.head_sha);
    assert.deepEqual(replay.rule_index, refreshed.rule_index);

    const movedRepository = `${repository}-temporarily-moved`;
    await rename(repository, movedRepository);
    const failedRefresh = await fetch(
      `${origin}/api/projects/${encodeURIComponent(first.id)}/refresh`,
      { method: "POST", headers: { "content-type": "application/json" }, body: "{}" },
    );
    assert.equal(failedRefresh.status, 400);
    const afterFailure = await (await fetch(
      `${origin}/api/projects/${encodeURIComponent(first.id)}`,
    )).json();
    assert.equal(afterFailure.git_observation.head_sha, refreshed.git_observation.head_sha);
    assert.deepEqual(afterFailure.rule_index, refreshed.rule_index);
    assert.equal(afterFailure.last_refresh_failure.code, "PROJECT_PATH_NOT_FOUND");
    assert.ok(Date.parse(afterFailure.last_refresh_failure.observed_at));
    await rename(movedRepository, repository);
    const recovered = await fetch(
      `${origin}/api/projects/${encodeURIComponent(first.id)}/refresh`,
      { method: "POST", headers: { "content-type": "application/json" }, body: "{}" },
    );
    assert.equal(recovered.status, 200);
    assert.equal((await recovered.json()).last_refresh_failure, null);

    const beforeMissing = (await (await fetch(`${origin}/api/state`)).json()).projects.length;
    const missing = await fetch(`${origin}/api/projects`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        name: "Missing",
        rootPath: path.join(directory, "does-not-exist"),
      }),
    });
    assert.equal(missing.status, 400);
    assert.equal((await missing.json()).code, "PROJECT_PATH_NOT_FOUND");
    assert.equal((await (await fetch(`${origin}/api/state`)).json()).projects.length, beforeMissing);
  } finally {
    if (application?.server.listening) await close(application);
    await rm(directory, { recursive: true, force: true });
  }
});

test("packaged project CLI registers and reads human-readable durable state across server restart", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "lattice-project-cli-"));
  const repository = path.join(directory, "repository");
  const databasePath = path.join(directory, "control.db");
  const clientPath = path.resolve(import.meta.dirname, "../src/project-client.mjs");
  let application;
  try {
    await mkdir(repository);
    await writeFile(path.join(repository, "AGENTS.md"), "# CLI rules\n", "utf8");
    application = createLatticeServer({ databasePath, codex: new IdleCodex() });
    let origin = await listen(application);

    const registered = await execFileAsync(process.execPath, [
      clientPath,
      "register",
      "--name",
      "CLI Fixture",
      "--path",
      repository,
      "--origin",
      origin,
    ], { encoding: "utf8", windowsHide: true });
    assert.match(registered.stdout, /Control 本機專案目錄項目/u);
    assert.match(registered.stdout, /Registry authority：NONE/u);
    assert.match(registered.stdout, /規則索引/u);
    assert.match(registered.stdout, /AGENTS\.md/u);
    assert.match(registered.stdout, /不是 Git repository/u);

    const state = await (await fetch(`${origin}/api/state`)).json();
    const projectId = state.projects[0].id;
    await close(application);
    application = createLatticeServer({ databasePath, codex: new IdleCodex() });
    origin = await listen(application);

    const replayed = await execFileAsync(process.execPath, [
      clientPath,
      "read",
      "--project-id",
      projectId,
      "--origin",
      origin,
    ], { encoding: "utf8", windowsHide: true });
    assert.match(replayed.stdout, new RegExp(projectId, "u"));
    assert.match(replayed.stdout, /Control 本機專案目錄項目/u);

    const json = await execFileAsync(process.execPath, [
      clientPath,
      "read",
      "--project-name",
      "CLI Fixture",
      "--origin",
      origin,
      "--json",
    ], { encoding: "utf8", windowsHide: true });
    const parsed = JSON.parse(json.stdout);
    assert.equal(parsed.status, "PROJECT_READ");
    assert.equal(parsed.project.id, projectId);
    assertControlCatalogLocator(parsed.project);

    const movedRepository = `${repository}-missing`;
    await rename(repository, movedRepository);
    await assert.rejects(
      execFileAsync(process.execPath, [
        clientPath,
        "refresh",
        "--project-id",
        projectId,
        "--origin",
        origin,
      ], { encoding: "utf8", windowsHide: true }),
      (error) => error.code === 1 && /does not exist/u.test(error.stderr),
    );
    const staleRead = await execFileAsync(process.execPath, [
      clientPath,
      "read",
      "--project-id",
      projectId,
      "--origin",
      origin,
    ], { encoding: "utf8", windowsHide: true });
    assert.match(staleRead.stdout, /最近一次 refresh 失敗/u);
    assert.match(staleRead.stdout, /PROJECT_PATH_NOT_FOUND/u);

    await assert.rejects(
      execFileAsync(process.execPath, [
        clientPath,
        "read",
        "--project-id",
        projectId,
        "--origin",
        "https://example.invalid",
      ], { encoding: "utf8", windowsHide: true }),
      (error) => error.code === 1 && /loopback/u.test(error.stderr),
    );
  } finally {
    if (application?.server.listening) await close(application);
    await rm(directory, { recursive: true, force: true });
  }
});

test("project client rejects a replay that changes any persisted catalog projection", async () => {
  const canonicalPath = path.resolve("C:\\fixture");
  const project = {
    schema_version: "lattice.control.project-catalog.v1",
    record_kind: "CONTROL_LOCAL_CATALOG",
    registry_authority: "NONE",
    registry_project_id: null,
    control_project_id: "catalog-id",
    id: "catalog-id",
    name: "Replay fixture",
    root_path: canonicalPath,
    canonical_path: canonicalPath,
    repo_root: canonicalPath,
    repo_root_observed_at: "2026-08-26T01:00:00.000Z",
    created_at: "2026-08-26T01:00:00.000Z",
    updated_at: "2026-08-26T01:00:00.000Z",
    registered_at: "2026-08-26T01:00:00.000Z",
    refreshed_at: "2026-08-26T01:00:00.000Z",
    last_refresh_failure: null,
    git_observation: fixtureInspection(
      canonicalPath,
      "2026-08-26T01:00:00.000Z",
      { headSha: "a".repeat(40) },
    ).git,
    rule_index: fixtureInspection(canonicalPath, "2026-08-26T01:00:00.000Z").rules,
  };
  let requestCount = 0;
  const fetchImpl = async () => {
    requestCount += 1;
    const body = requestCount === 1
      ? { ...project, created: true }
      : {
          ...project,
          git_observation: { ...project.git_observation, head_sha: "b".repeat(40) },
        };
    return new Response(JSON.stringify(body), {
      status: requestCount === 1 ? 201 : 200,
      headers: { "content-type": "application/json" },
    });
  };
  await assert.rejects(
    runProjectCommand({
      command: "register",
      name: project.name,
      rootPath: canonicalPath,
      fetchImpl,
    }),
    /persisted project replay did not match/u,
  );
});
