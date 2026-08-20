import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { EventEmitter } from "node:events";
import {
  copyFileSync,
  mkdtempSync,
  mkdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import {
  classifyTicketStatus,
  openLocalFile,
  writeDashboard,
} from "../scripts/export-lattice-engineering-status.mjs";

const projectRoot = path.resolve(import.meta.dirname, "..");
const exporter = path.join(projectRoot, "scripts", "export-lattice-engineering-status.mjs");

function run(command, args, cwd) {
  return execFileSync(command, args, {
    cwd,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  }).trim();
}

function git(cwd, ...args) {
  return run("git", ["-c", `safe.directory=${cwd}`, "-C", cwd, ...args], cwd);
}

function createFixture({
  terminalState = "FAIL",
  malicious = false,
  sourceName = "source",
} = {}) {
  const root = mkdtempSync(path.join(os.tmpdir(), "lattice-dashboard-test-"));
  const repository = path.join(root, sourceName);
  const remote = path.join(root, "remote.git");
  const output = path.join(root, "output");
  mkdirSync(repository, { recursive: true });
  run("git", ["init", "--bare", remote], root);
  run("git", ["init", "-b", "main", repository], root);
  git(repository, "config", "user.email", "dashboard@example.invalid");
  git(repository, "config", "user.name", "Dashboard Test");
  mkdirSync(path.join(repository, "docs", "tickets"), { recursive: true });
  const unsafe = malicious ? "</script><script>globalThis.pwned=true</script>" : "";
  const privatePaths = malicious
    ? " C:\\Users\\alice\\private\\repo; C:/Users/alice/private/repo; \\\\server\\share\\private; /home/alice/private/repo; /root/private/repo; /data/private/repo"
    : "";
  writeFileSync(
    path.join(repository, "docs", "tickets", "TASK-101-demo.md"),
    `---\nticket_id: TASK-101\ntitle: Visible demo ${unsafe}\nstatus: in_progress\nbranch: feature/task-101-demo\n---\n\n# Demo\n\n## Objective\n\nMake branch state understandable. ${unsafe}${privatePaths}\n\n## Result\n\nCurrent terminal state is \`${terminalState}\`, not \`VERIFIED\`.\n\n## Next action\n\nReview the bounded correction.\n\n## Human gate\n\nNo user action is required.\n`,
    "utf8",
  );
  git(repository, "add", ".");
  git(repository, "commit", "-m", "seed fixture");
  git(repository, "switch", "-c", "feature/task-101-demo");
  git(repository, "remote", "add", "origin", remote);
  git(repository, "push", "-u", "origin", "feature/task-101-demo");
  return { root, repository, remote, output };
}

function exportFixture(repository, output) {
  const result = spawnSync(
    process.execPath,
    [exporter, "--repository", repository, "--output", output],
    { cwd: projectRoot, encoding: "utf8" },
  );
  assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`);
  return {
    snapshot: JSON.parse(readFileSync(path.join(output, "status.json"), "utf8")),
    html: readFileSync(path.join(output, "index.html"), "utf8"),
  };
}

test("exports explicit task failure safely without mutating the source tree", () => {
  const fixture = createFixture({ malicious: true });
  try {
    const before = git(fixture.repository, "status", "--porcelain=v1");
    const { snapshot, html } = exportFixture(fixture.repository, fixture.output);
    const after = git(fixture.repository, "status", "--porcelain=v1");
    const item = snapshot.items.find((candidate) => candidate.id === "TASK-101");

    assert.equal(snapshot.schema, "lattice.engineering-status/1.0");
    assert.equal(snapshot.repository.currentBranch, "feature/task-101-demo");
    assert.equal(snapshot.completeness, "complete");
    assert.equal(item.outcome, "FAIL");
    assert.equal(item.git.clean, true);
    assert.equal(item.git.sync.state, "synced");
    assert.equal(item.nextStep, "Review the bounded correction.");
    assert.equal(before, "");
    assert.equal(after, "");
    assert.equal(html.includes("</script><script>globalThis.pwned=true</script>"), false);
    assert.equal(html.includes(".innerHTML"), false);
    assert.match(html, /LATTICE 工程雷達/u);
    assert.equal(html.includes(fixture.repository), false);
    assert.equal(html.includes("C:\\Users\\alice\\private\\repo"), false);
    assert.equal(html.includes("C:/Users/alice/private/repo"), false);
    assert.equal(html.includes("\\\\server\\share\\private"), false);
    assert.equal(html.includes("/home/alice/private/repo"), false);
    assert.equal(html.includes("/root/private/repo"), false);
    assert.equal(html.includes("/data/private/repo"), false);
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("keeps a TASK with a missing ticket visible as partial and unknown", () => {
  const fixture = createFixture();
  try {
    git(fixture.repository, "rm", "docs/tickets/TASK-101-demo.md");
    git(fixture.repository, "commit", "-m", "remove ticket to simulate partial evidence");
    const { snapshot } = exportFixture(fixture.repository, fixture.output);
    const item = snapshot.items.find((candidate) => candidate.id === "TASK-101");

    assert.equal(snapshot.completeness, "partial");
    assert.equal(item.outcome, "UNKNOWN");
    assert.equal(item.evidenceState, "partial");
    assert.deepEqual(item.errors, ["目前分支找不到對應 TASK 票券"]);
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("rejects duplicate TASK tickets instead of selecting one arbitrarily", () => {
  const fixture = createFixture();
  try {
    writeFileSync(
      path.join(fixture.repository, "docs", "tickets", "TASK-101-duplicate.md"),
      "---\nticket_id: TASK-101\ntitle: Duplicate\nstatus: complete\nbranch: feature/task-101-demo\n---\n",
      "utf8",
    );
    git(fixture.repository, "add", ".");
    git(fixture.repository, "commit", "-m", "add conflicting duplicate ticket");
    const { snapshot } = exportFixture(fixture.repository, fixture.output);
    const item = snapshot.items.find((candidate) => candidate.id === "TASK-101");

    assert.equal(item.outcome, "UNKNOWN");
    assert.equal(item.evidenceState, "partial");
    assert.deepEqual(item.errors, ["目前分支有重複的 TASK 票券"]);
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("normalizes the repository ticket status vocabulary", () => {
  const cases = new Map([
    ["in-progress", "IN_PROGRESS"],
    ["in_progress", "IN_PROGRESS"],
    ["completed", "COMPLETE"],
    ["complete", "COMPLETE"],
    ["partial", "PARTIAL"],
    ["paused", "PAUSED"],
    ["superseded", "SUPERSEDED"],
    ["waiting_dependency", "WAITING_DEPENDENCY"],
  ]);
  for (const [status, expected] of cases) {
    assert.equal(classifyTicketStatus(status), expected, status);
  }
  assert.equal(classifyTicketStatus("invented-success"), null);
});

test("marks an unrecognized TASK status as partial unknown evidence", () => {
  const fixture = createFixture();
  try {
    const ticketPath = path.join(fixture.repository, "docs", "tickets", "TASK-101-demo.md");
    const ticket = readFileSync(ticketPath, "utf8").replace(
      "status: in_progress",
      "status: invented-success",
    );
    writeFileSync(ticketPath, ticket, "utf8");
    git(fixture.repository, "add", "docs/tickets/TASK-101-demo.md");
    git(fixture.repository, "commit", "-m", "add malformed ticket status");
    const { snapshot } = exportFixture(fixture.repository, fixture.output);
    const item = snapshot.items.find((candidate) => candidate.id === "TASK-101");

    assert.equal(item.outcome, "UNKNOWN");
    assert.equal(item.evidenceState, "partial");
    assert.deepEqual(item.errors, ["TASK 票券的 status 無法辨識"]);
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("rejects duplicate authoritative frontmatter fields instead of taking the last value", () => {
  const fixture = createFixture();
  try {
    const ticketPath = path.join(fixture.repository, "docs", "tickets", "TASK-101-demo.md");
    const ticket = readFileSync(ticketPath, "utf8").replace(
      "status: in_progress",
      "status: in_progress\nstatus: complete",
    );
    writeFileSync(ticketPath, ticket, "utf8");
    git(fixture.repository, "add", "docs/tickets/TASK-101-demo.md");
    git(fixture.repository, "commit", "-m", "add conflicting ticket status fields");
    const { snapshot } = exportFixture(fixture.repository, fixture.output);
    const item = snapshot.items.find((candidate) => candidate.id === "TASK-101");

    assert.equal(item.outcome, "UNKNOWN");
    assert.equal(item.evidenceState, "partial");
    assert.deepEqual(item.errors, ["TASK 票券有重複的 frontmatter 欄位"]);
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("rejects an invalid snapshot before replacing an existing output", async () => {
  const root = mkdtempSync(path.join(os.tmpdir(), "lattice-dashboard-output-test-"));
  try {
    writeFileSync(path.join(root, "status.json"), "old-json\n", "utf8");
    writeFileSync(path.join(root, "index.html"), "old-html\n", "utf8");
    await assert.rejects(
      writeDashboard({ schema: "wrong", items: [] }, root),
      /invalid engineering-status snapshot/u,
    );
    assert.equal(readFileSync(path.join(root, "status.json"), "utf8"), "old-json\n");
    assert.equal(readFileSync(path.join(root, "index.html"), "utf8"), "old-html\n");
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("uses the Windows file opener with the exact HTML path before reporting success", async () => {
  const expectedPath = "C:\\Users\\Example Name\\LATTICE\\index.html";
  let observed;
  const spawnProcess = (command, args, options) => {
    observed = { command, args, options };
    const child = new EventEmitter();
    child.unref = () => {};
    queueMicrotask(() => child.emit("spawn"));
    return child;
  };

  await openLocalFile(expectedPath, { platform: "win32", spawnProcess });
  assert.equal(observed.command, "explorer.exe");
  assert.deepEqual(observed.args, [expectedPath]);
  assert.equal(observed.options.detached, true);
});

test(
  "Windows launcher preserves the repository argument with spaces and a trailing separator",
  { skip: process.platform !== "win32" },
  () => {
    const fixture = createFixture({ sourceName: "source with spaces" });
    const localApplicationData = path.join(fixture.root, "local application data");
    try {
      const launcher = path.join(fixture.repository, "Open-LATTICE-Engineering-Status.cmd");
      const fixtureExporter = path.join(
        fixture.repository,
        "scripts",
        "export-lattice-engineering-status.mjs",
      );
      const fixtureTemplate = path.join(
        fixture.repository,
        "tools",
        "engineering-status-dashboard",
        "index.template.html",
      );
      mkdirSync(path.dirname(fixtureExporter), { recursive: true });
      mkdirSync(path.dirname(fixtureTemplate), { recursive: true });
      copyFileSync(path.join(projectRoot, "Open-LATTICE-Engineering-Status.cmd"), launcher);
      copyFileSync(exporter, fixtureExporter);
      copyFileSync(
        path.join(projectRoot, "tools", "engineering-status-dashboard", "index.template.html"),
        fixtureTemplate,
      );
      const result = spawnSync(
        "cmd.exe",
        ["/d", "/c", launcher],
        {
          cwd: fixture.repository,
          encoding: "utf8",
          env: {
            ...process.env,
            LOCALAPPDATA: localApplicationData,
            LATTICE_DASHBOARD_NO_OPEN: "1",
            LATTICE_DASHBOARD_OFFLINE: "1",
          },
        },
      );
      assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`);
      const snapshot = JSON.parse(
        readFileSync(
          path.join(localApplicationData, "LATTICE", "engineering-status", "status.json"),
          "utf8",
        ),
      );
      assert.equal(snapshot.repository.currentBranch, "feature/task-101-demo");
      assert.equal(snapshot.currentItemId, "TASK-101");
    } finally {
      rmSync(fixture.root, { recursive: true, force: true });
    }
  },
);

test("keeps remote divergence and issue worktrees visible as separate evidence", () => {
  const fixture = createFixture({ terminalState: "WAITING_DEPENDENCY" });
  const issueWorktree = path.join(fixture.root, "issue-worktree");
  try {
    writeFileSync(path.join(fixture.repository, "ahead.txt"), "ahead\n", "utf8");
    git(fixture.repository, "add", "ahead.txt");
    git(fixture.repository, "commit", "-m", "local ahead commit");
    git(fixture.repository, "worktree", "add", "-b", "issue/42-readable-cards", issueWorktree, "main");

    const { snapshot } = exportFixture(fixture.repository, fixture.output);
    const task = snapshot.items.find((candidate) => candidate.id === "TASK-101");
    const issue = snapshot.items.find((candidate) => candidate.id === "ISSUE-42");

    assert.equal(task.outcome, "WAITING_DEPENDENCY");
    assert.equal(task.git.sync.state, "ahead");
    assert.equal(task.git.sync.ahead, 1);
    assert.equal(task.git.sync.behind, 0);
    assert.equal(issue.kind, "ISSUE");
    assert.equal(issue.outcome, "UNKNOWN");
    assert.equal(issue.git.sync.state, "no-upstream");
    assert.ok(snapshot.items.every((item) => typeof item.evidenceState === "string"));
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("does not call a stale tracking ref synced after the live remote advances", () => {
  const fixture = createFixture();
  const peer = path.join(fixture.root, "peer");
  try {
    run(
      "git",
      ["clone", "--branch", "feature/task-101-demo", fixture.remote, peer],
      fixture.root,
    );
    git(peer, "config", "user.email", "peer@example.invalid");
    git(peer, "config", "user.name", "Remote Peer");
    writeFileSync(path.join(peer, "remote-change.txt"), "remote changed\n", "utf8");
    git(peer, "add", "remote-change.txt");
    git(peer, "commit", "-m", "advance remote outside source tracking ref");
    git(peer, "push", "origin", "feature/task-101-demo");

    const { snapshot } = exportFixture(fixture.repository, fixture.output);
    const item = snapshot.items.find((candidate) => candidate.id === "TASK-101");
    assert.equal(snapshot.sources.gitRemote.state, "available");
    assert.equal(item.git.sync.state, "remote-changed");
    assert.equal(item.git.sync.remoteVerified, true);
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});
