import assert from "node:assert/strict";
import {
  existsSync,
  mkdtempSync,
  mkdirSync,
  readFileSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import test from "node:test";

import {
  finishDelivery,
  formatSuccessOutput,
} from "../scripts/finish-lattice-delivery.mjs";

const finisherScript = path.resolve("scripts", "finish-lattice-delivery.mjs");

function run(command, args, cwd) {
  const result = spawnSync(command, args, {
    cwd,
    encoding: "utf8",
    windowsHide: true,
  });
  if (result.status !== 0) {
    throw new Error(
      `${command} ${args.join(" ")} failed (${result.status}): ${result.stderr || result.stdout}`,
    );
  }
  return result.stdout.trim();
}

function git(cwd, ...args) {
  return run("git", args, cwd);
}

function writeTicket(repository, {
  task = "TASK-901",
  branch = "feature/task-901-demo",
  push = "authorized_non_force_feature_branch",
  archive = "after_success",
  remote = "origin",
  remoteIdentity = "test.invalid/missing",
  status = "complete",
} = {}) {
  const ticketDirectory = path.join(repository, "docs", "tickets");
  mkdirSync(ticketDirectory, { recursive: true });
  const pushLine = push === null ? "" : `delivery_push: ${push}\n`;
  const archiveLine = archive === null ? "" : `delivery_archive: ${archive}\n`;
  writeFileSync(
    path.join(ticketDirectory, `${task}-demo.md`),
    `---\nticket_id: ${task}\nstatus: ${status}\nbranch: ${branch}\ndelivery_remote: ${remote}\ndelivery_repository: ${remoteIdentity}\n${pushLine}${archiveLine}---\n\n# ${task}\n`,
    "utf8",
  );
}

function createRepository(options = {}) {
  const root = mkdtempSync(path.join(os.tmpdir(), "lattice-finisher-test-"));
  const remote = path.join(root, "remote.git");
  const repository = path.join(root, "repository");
  const outputDirectory = path.join(root, "status-output");
  mkdirSync(repository);
  git(root, "init", "--bare", remote);
  git(repository, "init", "-b", "main");
  git(repository, "config", "user.name", "LATTICE Test");
  git(repository, "config", "user.email", "lattice@example.invalid");
  writeFileSync(path.join(repository, "README.md"), "base\n", "utf8");
  git(repository, "add", "README.md");
  git(repository, "commit", "-m", "base");
  git(repository, "remote", "add", "origin", remote);
  git(repository, "push", "-u", "origin", "main");
  git(root, "--git-dir", remote, "symbolic-ref", "HEAD", "refs/heads/main");
  git(repository, "remote", "set-head", "origin", "main");
  git(repository, "switch", "-c", options.branch || "feature/task-901-demo");
  const testRemoteIdentity = `file:${path.resolve(remote).replaceAll("\\", "/").toLowerCase()}`;
  writeTicket(repository, { ...options.ticket, remoteIdentity: testRemoteIdentity });
  writeFileSync(path.join(repository, "PLANS.md"), "CURRENT TASK-901 - fixture\n", "utf8");
  git(repository, "add", "docs/tickets", "PLANS.md");
  git(repository, "commit", "-m", "add task ticket");
  return { root, remote, repository, outputDirectory };
}

function remoteHead(repository, branch = "feature/task-901-demo") {
  const output = git(
    repository,
    "ls-remote",
    "--heads",
    "origin",
    `refs/heads/${branch}`,
  );
  return output ? output.split(/\s+/u)[0] : null;
}

async function successfulRefresh({ repository, outputDirectory }) {
  const branch = git(repository, "branch", "--show-current");
  const head = git(repository, "rev-parse", "HEAD");
  const snapshot = {
    schema: "lattice.engineering-status/2.0",
    items: [{
      branch,
      git: { head, sync: { state: "synced", remoteVerified: true } },
    }],
  };
  writeFileSync(
    path.join(outputDirectory, "status.json"),
    `${JSON.stringify(snapshot)}\n`,
    "utf8",
  );
  writeFileSync(
    path.join(outputDirectory, "index.html"),
    "<title>LATTICE 分支工作地圖</title>\n",
    "utf8",
  );
}

test("local_only refreshes and completes without running a push", async () => {
  const fixture = createRepository({
    ticket: { push: "local_only", archive: "after_success" },
  });
  let refreshCount = 0;
  try {
    const result = await finishDelivery({
      repository: fixture.repository,
      outputDirectory: fixture.outputDirectory,
      refresh: async (context) => {
        refreshCount += 1;
        await successfulRefresh(context);
      },
    });

    assert.equal(result.success, true);
    assert.equal(result.push.performed, false);
    assert.equal(result.push.policy, "local_only");
    assert.equal(result.archiveReady, true);
    assert.equal(refreshCount, 1);
    assert.equal(remoteHead(fixture.repository), null);
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("authorized policy performs a non-force current-branch push and verifies equality", async () => {
  const fixture = createRepository();
  try {
    git(fixture.repository, "tag", "-a", "must-stay-local", "-m", "local tag");
    git(fixture.repository, "config", "push.followTags", "true");
    const result = await finishDelivery({
      repository: fixture.repository,
      outputDirectory: fixture.outputDirectory,
      refresh: successfulRefresh,
    });
    const localHead = git(fixture.repository, "rev-parse", "HEAD");

    assert.equal(result.success, true);
    assert.equal(result.push.performed, true);
    assert.equal(result.remote.verified, true);
    assert.equal(remoteHead(fixture.repository), localHead);
    assert.equal(git(fixture.repository, "rev-parse", "@{u}"), localHead);
    assert.equal(
      git(
        fixture.repository,
        "ls-remote",
        "--tags",
        "origin",
        "refs/tags/must-stay-local",
      ),
      "",
    );
    assert.match(formatSuccessOutput(result), /LATTICE_DELIVERY_FINISHED=1/u);
    assert.match(formatSuccessOutput(result), /LATTICE_DELIVERY_READY_TO_ARCHIVE=1/u);
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("repository Git hooks cannot add side effects to delivery", async () => {
  const fixture = createRepository();
  const hookResult = path.join(fixture.repository, "hook-ran.txt");
  try {
    writeFileSync(
      path.join(fixture.repository, ".git", "hooks", "pre-push"),
      "#!/bin/sh\necho ran > hook-ran.txt\n",
      "utf8",
    );
    const result = await finishDelivery({
      repository: fixture.repository,
      outputDirectory: fixture.outputDirectory,
      refresh: successfulRefresh,
    });
    assert.equal(result.success, true);
    assert.equal(existsSync(hookResult), false);
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("a remote that becomes the default branch immediately before push fails closed", async () => {
  const fixture = createRepository();
  try {
    await assert.rejects(
      finishDelivery({
        repository: fixture.repository,
        outputDirectory: fixture.outputDirectory,
        beforePush: async () => {
          git(
            fixture.root,
            "--git-dir",
            fixture.remote,
            "symbolic-ref",
            "HEAD",
            "refs/heads/feature/task-901-demo",
          );
        },
        refresh: successfulRefresh,
      }),
      /default branch/u,
    );
    assert.equal(remoteHead(fixture.repository), null);
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("remote endpoint identity cannot change after authorization checks", async () => {
  const fixture = createRepository();
  const otherRemote = path.join(fixture.root, "other.git");
  git(fixture.root, "init", "--bare", otherRemote);
  try {
    await assert.rejects(
      finishDelivery({
        repository: fixture.repository,
        outputDirectory: fixture.outputDirectory,
        beforePush: async () => {
          git(fixture.repository, "remote", "set-url", "origin", otherRemote);
        },
        refresh: successfulRefresh,
      }),
      /remote endpoint changed/u,
    );
    assert.equal(remoteHead(fixture.repository), null);
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("different fetch and push endpoints are rejected", async () => {
  const fixture = createRepository();
  const otherRemote = path.join(fixture.root, "push-target.git");
  git(fixture.root, "init", "--bare", otherRemote);
  try {
    git(fixture.repository, "remote", "set-url", "--add", "--push", "origin", otherRemote);
    await assert.rejects(
      finishDelivery({
        repository: fixture.repository,
        outputDirectory: fixture.outputDirectory,
        refresh: successfulRefresh,
      }),
      /fetch and push endpoint/u,
    );
    assert.equal(remoteHead(fixture.repository), null);
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("ticket repository identity must match the configured endpoint", async () => {
  const fixture = createRepository();
  try {
    writeTicket(fixture.repository, {
      remoteIdentity: "github.com/example/not-authorized",
    });
    git(fixture.repository, "add", "docs/tickets");
    git(fixture.repository, "commit", "-m", "change authorized identity");
    await assert.rejects(
      finishDelivery({
        repository: fixture.repository,
        outputDirectory: fixture.outputDirectory,
        refresh: successfulRefresh,
      }),
      /authorized repository identity/u,
    );
    assert.equal(remoteHead(fixture.repository), null);
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("authorization is read from captured HEAD despite skip-worktree changes", async () => {
  const fixture = createRepository({
    ticket: { push: "local_only", archive: "keep_open" },
  });
  const ticket = path.join(
    fixture.repository,
    "docs",
    "tickets",
    "TASK-901-demo.md",
  );
  try {
    git(fixture.repository, "update-index", "--skip-worktree", ticket);
    const hiddenWorkingCopy = readFileSync(ticket, "utf8").replace(
      "delivery_push: local_only",
      "delivery_push: authorized_non_force_feature_branch",
    );
    writeFileSync(ticket, hiddenWorkingCopy, "utf8");
    assert.equal(git(fixture.repository, "status", "--porcelain=v1"), "");

    const result = await finishDelivery({
      repository: fixture.repository,
      outputDirectory: fixture.outputDirectory,
      refresh: successfulRefresh,
    });
    assert.equal(result.push.performed, false);
    assert.equal(remoteHead(fixture.repository), null);
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("credential-bearing remote URLs are rejected before network delivery", async () => {
  const fixture = createRepository();
  try {
    const ticket = path.join(fixture.repository, "docs", "tickets", "TASK-901-demo.md");
    const content = readFileSync(ticket, "utf8").replace(
      /^delivery_repository:.*$/mu,
      "delivery_repository: github.com/z72124223/lattice-devos",
    );
    writeFileSync(ticket, content, "utf8");
    git(fixture.repository, "add", ticket);
    git(fixture.repository, "commit", "-m", "declare GitHub repository identity");
    git(
      fixture.repository,
      "remote",
      "set-url",
      "origin",
      "https://user:secret@github.com/z72124223/lattice-devos.git",
    );
    await assert.rejects(
      finishDelivery({
        repository: fixture.repository,
        outputDirectory: fixture.outputDirectory,
        refresh: successfulRefresh,
      }),
      /unsupported/u,
    );
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("live remote default branch outranks a stale local origin HEAD cache", async () => {
  const fixture = createRepository();
  try {
    git(
      fixture.root,
      "--git-dir",
      fixture.remote,
      "symbolic-ref",
      "HEAD",
      "refs/heads/feature/task-901-demo",
    );
    assert.equal(
      git(fixture.repository, "symbolic-ref", "--short", "refs/remotes/origin/HEAD"),
      "origin/main",
    );

    await assert.rejects(
      finishDelivery({
        repository: fixture.repository,
        outputDirectory: fixture.outputDirectory,
        refresh: successfulRefresh,
      }),
      /default branch/u,
    );
    assert.equal(remoteHead(fixture.repository), null);
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("keep_open completes delivery without emitting archive permission", async () => {
  const fixture = createRepository({ ticket: { archive: "keep_open" } });
  try {
    const result = await finishDelivery({
      repository: fixture.repository,
      outputDirectory: fixture.outputDirectory,
      refresh: successfulRefresh,
    });

    assert.equal(result.success, true);
    assert.equal(result.archiveReady, false);
    assert.doesNotMatch(formatSuccessOutput(result), /READY_TO_ARCHIVE/u);
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("failed terminal work may be preserved but never archived", async () => {
  const fixture = createRepository({ ticket: { status: "failed", archive: "after_success" } });
  try {
    const result = await finishDelivery({
      repository: fixture.repository,
      outputDirectory: fixture.outputDirectory,
      refresh: successfulRefresh,
    });

    assert.equal(result.success, true);
    assert.equal(result.push.performed, true);
    assert.equal(result.archiveReady, false);
    assert.doesNotMatch(formatSuccessOutput(result), /READY_TO_ARCHIVE/u);
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("completed is a successful terminal status eligible for after-success archival", async () => {
  const fixture = createRepository({ ticket: { status: "completed" } });
  try {
    const result = await finishDelivery({
      repository: fixture.repository,
      outputDirectory: fixture.outputDirectory,
      refresh: successfulRefresh,
    });

    assert.equal(result.success, true);
    assert.equal(result.archiveReady, true);
    assert.match(formatSuccessOutput(result), /LATTICE_DELIVERY_READY_TO_ARCHIVE=1/u);
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("dirty worktree fails before push, refreshes best-effort, and forbids archive", async () => {
  const fixture = createRepository();
  let refreshCount = 0;
  writeFileSync(path.join(fixture.repository, "dirty.txt"), "dirty\n", "utf8");
  try {
    await assert.rejects(
      finishDelivery({
        repository: fixture.repository,
        outputDirectory: fixture.outputDirectory,
        refresh: async (context) => {
          refreshCount += 1;
          await successfulRefresh(context);
        },
      }),
      /worktree must be clean/u,
    );
    assert.equal(refreshCount, 1);
    assert.equal(remoteHead(fixture.repository), null);
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("missing or unknown delivery policy fails closed", async () => {
  for (const push of [null, "push_everything"]) {
    const fixture = createRepository({ ticket: { push } });
    try {
      await assert.rejects(
        finishDelivery({
          repository: fixture.repository,
          outputDirectory: fixture.outputDirectory,
          refresh: successfulRefresh,
        }),
        /delivery_push/u,
      );
      assert.equal(remoteHead(fixture.repository), null);
    } finally {
      rmSync(fixture.root, { recursive: true, force: true });
    }
  }
});

test("default and detached branches are rejected without push", async () => {
  const defaultFixture = createRepository();
  try {
    git(defaultFixture.repository, "switch", "main");
    await assert.rejects(
      finishDelivery({
        repository: defaultFixture.repository,
        outputDirectory: defaultFixture.outputDirectory,
        refresh: successfulRefresh,
      }),
      /default branch|TASK feature branch/u,
    );
    assert.equal(remoteHead(defaultFixture.repository), null);
  } finally {
    rmSync(defaultFixture.root, { recursive: true, force: true });
  }

  const detachedFixture = createRepository();
  try {
    git(detachedFixture.repository, "switch", "--detach");
    await assert.rejects(
      finishDelivery({
        repository: detachedFixture.repository,
        outputDirectory: detachedFixture.outputDirectory,
        refresh: successfulRefresh,
      }),
      /named branch/u,
    );
    assert.equal(remoteHead(detachedFixture.repository), null);
  } finally {
    rmSync(detachedFixture.root, { recursive: true, force: true });
  }
});

test("a rejected non-fast-forward push refreshes but remains failed and unarchivable", async () => {
  const fixture = createRepository();
  let refreshCount = 0;
  try {
    git(fixture.repository, "push", "-u", "origin", "feature/task-901-demo");
    const peer = path.join(fixture.root, "peer");
    git(fixture.root, "clone", fixture.remote, peer);
    git(peer, "config", "user.name", "Peer");
    git(peer, "config", "user.email", "peer@example.invalid");
    git(peer, "switch", "feature/task-901-demo");
    writeFileSync(path.join(peer, "peer.txt"), "peer\n", "utf8");
    git(peer, "add", "peer.txt");
    git(peer, "commit", "-m", "peer move");
    git(peer, "push", "origin", "feature/task-901-demo");
    writeFileSync(path.join(fixture.repository, "local.txt"), "local\n", "utf8");
    git(fixture.repository, "add", "local.txt");
    git(fixture.repository, "commit", "-m", "local move");

    await assert.rejects(
      finishDelivery({
        repository: fixture.repository,
        outputDirectory: fixture.outputDirectory,
        refresh: async (context) => {
          refreshCount += 1;
          await successfulRefresh(context);
        },
      }),
      /git push failed/u,
    );
    assert.equal(refreshCount, 1);
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("a branch switch after policy checks cannot push a different commit", async () => {
  const fixture = createRepository();
  let refreshCount = 0;
  try {
    await assert.rejects(
      finishDelivery({
        repository: fixture.repository,
        outputDirectory: fixture.outputDirectory,
        beforePush: async () => {
          git(fixture.repository, "switch", "-c", "feature/task-902-other");
          writeFileSync(path.join(fixture.repository, "other.txt"), "other\n", "utf8");
          git(fixture.repository, "add", "other.txt");
          git(fixture.repository, "commit", "-m", "other task commit");
        },
        refresh: async (context) => {
          refreshCount += 1;
          await successfulRefresh(context);
        },
      }),
      /worktree changed before push/u,
    );
    assert.equal(refreshCount, 1);
    assert.equal(remoteHead(fixture.repository), null);
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("refresh failure after a successful push prevents completion and archive", async () => {
  const fixture = createRepository();
  try {
    await assert.rejects(
      finishDelivery({
        repository: fixture.repository,
        outputDirectory: fixture.outputDirectory,
        refresh: async () => {
          throw new Error("refresh unavailable");
        },
      }),
      /refresh unavailable/u,
    );
    assert.equal(
      remoteHead(fixture.repository),
      git(fixture.repository, "rev-parse", "HEAD"),
    );
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("a zero-output refresh cannot replace an existing map or permit archive", async () => {
  const fixture = createRepository({
    ticket: { push: "local_only", archive: "after_success" },
  });
  const oldStatus = path.join(fixture.outputDirectory, "status.json");
  const oldIndex = path.join(fixture.outputDirectory, "index.html");
  mkdirSync(fixture.outputDirectory);
  writeFileSync(oldStatus, "old status\n", "utf8");
  writeFileSync(oldIndex, "old map\n", "utf8");
  try {
    await assert.rejects(
      finishDelivery({
        repository: fixture.repository,
        outputDirectory: fixture.outputDirectory,
        refresh: async () => {},
      }),
      /ownership is not proven/u,
    );
    assert.equal(readFileSync(oldStatus, "utf8"), "old status\n");
    assert.equal(readFileSync(oldIndex, "utf8"), "old map\n");
    assert.equal(
      existsSync(path.join(fixture.outputDirectory, ".lattice-engineering-status-owned")),
      false,
    );
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("dashboard output cannot be placed inside the source repository", async () => {
  const fixture = createRepository();
  try {
    await assert.rejects(
      finishDelivery({
        repository: fixture.repository,
        outputDirectory: path.join(fixture.repository, "generated-status"),
        refresh: async () => {
          throw new Error("refresh must not run");
        },
      }),
      /disjoint from the source repository/u,
    );
    assert.equal(remoteHead(fixture.repository), null);
    assert.equal(git(fixture.repository, "status", "--porcelain"), "");
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("an existing output directory with unowned data is preserved", async () => {
  const fixture = createRepository({
    ticket: { push: "local_only", archive: "keep_open" },
  });
  const sentinel = path.join(fixture.outputDirectory, "sentinel.txt");
  mkdirSync(fixture.outputDirectory);
  writeFileSync(sentinel, "keep me\n", "utf8");
  let refreshCount = 0;
  try {
    await assert.rejects(
      finishDelivery({
        repository: fixture.repository,
        outputDirectory: fixture.outputDirectory,
        refresh: async () => {
          refreshCount += 1;
        },
      }),
      /unowned data/u,
    );
    assert.equal(refreshCount, 0);
    assert.equal(readFileSync(sentinel, "utf8"), "keep me\n");
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("a repository ancestor cannot be used as dashboard output", async () => {
  const fixture = createRepository({
    ticket: { push: "local_only", archive: "keep_open" },
  });
  let refreshCount = 0;
  try {
    await assert.rejects(
      finishDelivery({
        repository: fixture.repository,
        outputDirectory: fixture.root,
        refresh: async () => {
          refreshCount += 1;
        },
      }),
      /disjoint/u,
    );
    assert.equal(refreshCount, 0);
    assert.equal(existsSync(fixture.repository), true);
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("a source mutation during refresh fails the final state gate", async () => {
  const fixture = createRepository();
  try {
    await assert.rejects(
      finishDelivery({
        repository: fixture.repository,
        outputDirectory: fixture.outputDirectory,
        refresh: async (context) => {
          await successfulRefresh(context);
          writeFileSync(path.join(fixture.repository, "late-change.txt"), "late\n", "utf8");
        },
      }),
      /changed during delivery/u,
    );
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("a remote move during refresh fails the final remote gate", async () => {
  const fixture = createRepository();
  try {
    await assert.rejects(
      finishDelivery({
        repository: fixture.repository,
        outputDirectory: fixture.outputDirectory,
        refresh: async (context) => {
          await successfulRefresh(context);
          const peer = path.join(fixture.root, "late-peer");
          git(fixture.root, "clone", fixture.remote, peer);
          git(peer, "config", "user.name", "Late Peer");
          git(peer, "config", "user.email", "late-peer@example.invalid");
          git(peer, "switch", "feature/task-901-demo");
          writeFileSync(path.join(peer, "late-peer.txt"), "late peer\n", "utf8");
          git(peer, "add", "late-peer.txt");
          git(peer, "commit", "-m", "late remote move");
          git(peer, "push", "origin", "feature/task-901-demo");
        },
      }),
      /remote branch changed during delivery/u,
    );
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("a live default-branch change during refresh fails the final gate", async () => {
  const fixture = createRepository();
  try {
    await assert.rejects(
      finishDelivery({
        repository: fixture.repository,
        outputDirectory: fixture.outputDirectory,
        refresh: async (context) => {
          await successfulRefresh(context);
          git(
            fixture.root,
            "--git-dir",
            fixture.remote,
            "symbolic-ref",
            "HEAD",
            "refs/heads/feature/task-901-demo",
          );
        },
      }),
      /default branch/u,
    );
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("a remote config change during refresh fails the final gate", async () => {
  const fixture = createRepository();
  const otherRemote = path.join(fixture.root, "late-target.git");
  git(fixture.root, "init", "--bare", otherRemote);
  try {
    await assert.rejects(
      finishDelivery({
        repository: fixture.repository,
        outputDirectory: fixture.outputDirectory,
        refresh: async (context) => {
          await successfulRefresh(context);
          git(fixture.repository, "remote", "set-url", "origin", otherRemote);
        },
      }),
      /remote endpoint changed/u,
    );
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("a dashboard output junction swap during refresh prevents success", async () => {
  const fixture = createRepository();
  mkdirSync(fixture.outputDirectory);
  try {
    await assert.rejects(
      finishDelivery({
        repository: fixture.repository,
        outputDirectory: fixture.outputDirectory,
        refresh: async (context) => {
          await successfulRefresh(context);
          rmSync(fixture.outputDirectory, { recursive: true, force: true });
          symlinkSync(
            fixture.repository,
            fixture.outputDirectory,
            process.platform === "win32" ? "junction" : "dir",
          );
        },
      }),
      /dashboard output changed during (?:refresh|delivery)/u,
    );
    assert.equal(git(fixture.repository, "status", "--porcelain"), "");
    assert.equal(existsSync(path.join(fixture.repository, "last-delivery.json")), false);
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("a failed preflight cannot refresh through a swapped output junction", async () => {
  const fixture = createRepository();
  mkdirSync(fixture.outputDirectory);
  try {
    await assert.rejects(
      finishDelivery({
        repository: fixture.repository,
        outputDirectory: fixture.outputDirectory,
        beforePush: async () => {
          rmSync(fixture.outputDirectory, { recursive: true, force: true });
          symlinkSync(
            fixture.repository,
            fixture.outputDirectory,
            process.platform === "win32" ? "junction" : "dir",
          );
          git(fixture.repository, "switch", "-c", "feature/task-902-other");
        },
        refresh: async ({ outputDirectory }) => {
          writeFileSync(path.join(outputDirectory, "status.json"), "{}\n", "utf8");
          writeFileSync(path.join(outputDirectory, "index.html"), "safe\n", "utf8");
        },
      }),
      /worktree changed before push/u,
    );
    assert.equal(existsSync(path.join(fixture.repository, "status.json")), false);
    assert.equal(existsSync(path.join(fixture.repository, "index.html")), false);
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("only the current terminal TASK feature branch may finish", async () => {
  for (const options of [
    { branch: "release", ticket: { branch: "release" } },
    { ticket: { status: "in_progress" } },
  ]) {
    const fixture = createRepository(options);
    try {
      await assert.rejects(
        finishDelivery({
          repository: fixture.repository,
          outputDirectory: fixture.outputDirectory,
          refresh: successfulRefresh,
        }),
        /TASK feature branch|terminal before delivery/u,
      );
    } finally {
      rmSync(fixture.root, { recursive: true, force: true });
    }
  }

  const wrongCurrent = createRepository();
  try {
    writeFileSync(path.join(wrongCurrent.repository, "PLANS.md"), "CURRENT TASK-902 - other\n", "utf8");
    git(wrongCurrent.repository, "add", "PLANS.md");
    git(wrongCurrent.repository, "commit", "-m", "move current task marker");
    await assert.rejects(
      finishDelivery({
        repository: wrongCurrent.repository,
        outputDirectory: wrongCurrent.outputDirectory,
        refresh: successfulRefresh,
      }),
      /CURRENT TASK must match/u,
    );
  } finally {
    rmSync(wrongCurrent.root, { recursive: true, force: true });
  }
});

test("delivery_remote must name a configured Git remote", async () => {
  const fixture = createRepository({ ticket: { remote: "missing" } });
  try {
    await assert.rejects(
      finishDelivery({
        repository: fixture.repository,
        outputDirectory: fixture.outputDirectory,
        refresh: successfulRefresh,
      }),
      /configured Git remote/u,
    );
    assert.equal(remoteHead(fixture.repository), null);
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("malformed, duplicate, or mismatched current TASK tickets fail closed", async () => {
  const malformed = createRepository();
  try {
    const ticket = path.join(malformed.repository, "docs", "tickets", "TASK-901-demo.md");
    writeFileSync(ticket, "branch: feature/task-901-demo\n", "utf8");
    git(malformed.repository, "add", ticket);
    git(malformed.repository, "commit", "-m", "break ticket frontmatter");
    await assert.rejects(
      finishDelivery({
        repository: malformed.repository,
        outputDirectory: malformed.outputDirectory,
        refresh: successfulRefresh,
      }),
      /frontmatter is invalid/u,
    );
  } finally {
    rmSync(malformed.root, { recursive: true, force: true });
  }

  const duplicate = createRepository();
  try {
    writeFileSync(
      path.join(duplicate.repository, "docs", "tickets", "TASK-901-duplicate.md"),
      "---\nticket_id: TASK-901\nstatus: complete\nbranch: feature/task-901-demo\ndelivery_remote: origin\ndelivery_push: local_only\ndelivery_archive: keep_open\n---\n",
      "utf8",
    );
    git(duplicate.repository, "add", "docs/tickets");
    git(duplicate.repository, "commit", "-m", "duplicate ticket");
    await assert.rejects(
      finishDelivery({
        repository: duplicate.repository,
        outputDirectory: duplicate.outputDirectory,
        refresh: successfulRefresh,
      }),
      /multiple TASK tickets/u,
    );
  } finally {
    rmSync(duplicate.root, { recursive: true, force: true });
  }

  const mismatch = createRepository();
  try {
    const ticket = path.join(mismatch.repository, "docs", "tickets", "TASK-901-demo.md");
    writeTicket(mismatch.repository, { branch: "feature/task-901-other" });
    git(mismatch.repository, "add", ticket);
    git(mismatch.repository, "commit", "-m", "mismatch ticket branch");
    await assert.rejects(
      finishDelivery({
        repository: mismatch.repository,
        outputDirectory: mismatch.outputDirectory,
        refresh: successfulRefresh,
      }),
      /exactly one TASK ticket/u,
    );
  } finally {
    rmSync(mismatch.root, { recursive: true, force: true });
  }
});

test("CLI failures cannot inject the reserved archive marker", () => {
  const injected = "--bad\nLATTICE_DELIVERY_READY_TO_ARCHIVE=1";
  const result = spawnSync(process.execPath, [finisherScript, injected], {
    encoding: "utf8",
    windowsHide: true,
  });
  const combined = `${result.stdout || ""}${result.stderr || ""}`;

  assert.equal(result.status, 1);
  assert.doesNotMatch(combined, /LATTICE_DELIVERY_READY_TO_ARCHIVE=1/u);
  assert.match(combined, /LATTICE_DELIVERY_FINISHED=0/u);
});

test("CLI path options require an explicit value and never fall back to cwd", () => {
  for (const option of ["--repository", "--output"]) {
    const fixture = createRepository({
      ticket: { push: "local_only", archive: "keep_open" },
    });
    try {
      const result = spawnSync(process.execPath, [finisherScript, option], {
        cwd: fixture.repository,
        encoding: "utf8",
        env: { ...process.env, LOCALAPPDATA: fixture.root },
        windowsHide: true,
      });
      const combined = `${result.stdout || ""}${result.stderr || ""}`;

      assert.equal(result.status, 1, option);
      assert.doesNotMatch(combined, /LATTICE_DELIVERY_FINISHED=1/u);
      assert.equal(remoteHead(fixture.repository), null);
    } finally {
      rmSync(fixture.root, { recursive: true, force: true });
    }
  }
});

test("CLI failure diagnostics do not expose an absolute output path", () => {
  const fixture = createRepository({
    ticket: { push: "local_only", archive: "keep_open" },
  });
  const notDirectory = path.join(fixture.root, "private-parent");
  const sensitiveOutput = path.join(notDirectory, "secret-output");
  writeFileSync(notDirectory, "file\n", "utf8");
  try {
    const result = spawnSync(
      process.execPath,
      [
        finisherScript,
        "--repository",
        fixture.repository,
        "--output",
        sensitiveOutput,
      ],
      { encoding: "utf8", windowsHide: true },
    );
    const combined = `${result.stdout || ""}${result.stderr || ""}`;

    assert.equal(result.status, 1);
    assert.doesNotMatch(combined, new RegExp(fixture.root.replaceAll("\\", "\\\\"), "u"));
    assert.match(combined, /error=delivery failed; task kept open/u);
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});
