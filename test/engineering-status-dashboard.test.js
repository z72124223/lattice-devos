import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { EventEmitter } from "node:events";
import {
  copyFileSync,
  existsSync,
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
  isSnapshotFresh,
  openLocalFile,
  recommendCodexSetup,
  writeDashboard,
} from "../scripts/export-lattice-engineering-status.mjs";

const projectRoot = path.resolve(import.meta.dirname, "..");
const exporter = path.join(projectRoot, "scripts", "export-lattice-engineering-status.mjs");
const dashboardLauncher = path.join(projectRoot, "Open-LATTICE-Engineering-Status.cmd");

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
  ticketStatus = "in_progress",
  malicious = false,
  sourceName = "source",
} = {}) {
  const root = mkdtempSync(path.join(os.tmpdir(), "lattice-dashboard-test-"));
  const repository = path.join(root, sourceName);
  const remote = path.join(root, "remote.git");
  const output = path.join(root, "output");
  const guide = path.join(root, "branch-guide.zh-TW.json");
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
    `---\nticket_id: TASK-101\ntitle: Visible demo ${unsafe}\nstatus: ${ticketStatus}\nbranch: feature/task-101-demo\n---\n\n# Demo\n\n## Objective\n\nMake branch state understandable. ${unsafe}${privatePaths}\n\n## Result\n\nCurrent terminal state is \`${terminalState}\`, not \`VERIFIED\`.\n\n## Next action\n\nReview the bounded correction.\n\n## Human gate\n\nNo user action is required.\n`,
    "utf8",
  );
  git(repository, "add", ".");
  git(repository, "commit", "-m", "seed fixture");
  git(repository, "remote", "add", "origin", remote);
  git(repository, "push", "-u", "origin", "main");
  run("git", ["--git-dir", remote, "symbolic-ref", "HEAD", "refs/heads/main"], root);
  git(repository, "switch", "-c", "feature/task-101-demo");
  writeFileSync(path.join(repository, "feature.txt"), "feature child\n", "utf8");
  git(repository, "add", "feature.txt");
  git(repository, "commit", "-m", "add feature child commit");
  git(repository, "push", "-u", "origin", "feature/task-101-demo");
  writeFileSync(
    guide,
    `${JSON.stringify({
      schema: "lattice.branch-guide.zh-TW/1.0",
      branches: {
        main: {
          name: "穩定專案根節點",
          purpose: "這是遠端預設分支，可作為不依賴新功能的工作起點。",
        },
        "feature/task-101-demo": {
          name: "任務 101：示範分支",
          purpose: "用來示範如何看懂分支狀態與派工資格。",
        },
      },
    }, null, 2)}\n`,
    "utf8",
  );
  return { root, repository, remote, output, guide };
}

function createIssueFixture({
  issueNumber = "007",
  status = "complete",
  branch = `feature/issue-${issueNumber}-resource-aware-scheduler`,
} = {}) {
  const fixture = createFixture();
  const issueId = `ISSUE-${issueNumber}`;
  const fileName = `${issueId}-resource-aware-scheduler.md`;
  git(fixture.repository, "switch", "-c", branch);
  mkdirSync(path.join(fixture.repository, "docs", "issues"), { recursive: true });
  writeFileSync(
    path.join(fixture.repository, "docs", "issues", fileName),
    `---\nissue_id: ${issueId}\ntitle: ${issueId} terminal evidence\nstatus: ${status}\nbranch: ${branch}\ndelivery_remote: origin\ndelivery_repository: github.com/example/lattice-devos\ndelivery_push: authorized_non_force_feature_branch\ndelivery_archive: keep_open\n---\n\n# ${issueId}\n\n## Objective\n\nProject committed terminal issue evidence into the engineering map.\n`,
    "utf8",
  );
  git(fixture.repository, "add", "docs/issues");
  git(fixture.repository, "commit", "-m", `add ${issueId} terminal evidence`);
  git(fixture.repository, "push", "-u", "origin", branch);
  const guide = JSON.parse(readFileSync(fixture.guide, "utf8"));
  guide.branches[branch] = {
    name: `${issueId}：終端證據`,
    purpose: "驗證已提交的 Issue 終端證據能投影到工程地圖。",
  };
  writeFileSync(fixture.guide, `${JSON.stringify(guide, null, 2)}\n`, "utf8");
  return { ...fixture, issueId, fileName, branch };
}

function exportFixture(repository, output, guide) {
  const result = spawnSync(
    process.execPath,
    [exporter, "--repository", repository, "--output", output, "--guide", guide],
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
    const { snapshot, html } = exportFixture(fixture.repository, fixture.output, fixture.guide);
    const after = git(fixture.repository, "status", "--porcelain=v1");
    const item = snapshot.items.find((candidate) => candidate.id === "TASK-101");

    assert.equal(snapshot.schema, "lattice.engineering-status/2.0");
    assert.equal(snapshot.repository.currentBranch, "feature/task-101-demo");
    assert.equal(snapshot.completeness, "complete");
    assert.equal(item.outcome, "FAIL");
    assert.equal(item.git.clean, true);
    assert.equal(item.git.sync.state, "synced");
    assert.equal(item.displayNameZh, "任務 101：示範分支");
    assert.equal(item.purposeZh, "用來示範如何看懂分支狀態與派工資格。");
    assert.equal(item.dispatch.eligible, false);
    assert.match(item.dispatch.reasonZh, /失敗/u);
    assert.equal(item.nextStep, "Review the bounded correction.");
    assert.equal(before, "");
    assert.equal(after, "");
    assert.equal(html.includes("</script><script>globalThis.pwned=true</script>"), false);
    assert.equal(html.includes(".innerHTML"), false);
    assert.match(html, /LATTICE 分支工作地圖/u);
    assert.match(html, /你現在可以從哪裡開始新工作/u);
    assert.match(html, /全部展開/u);
    assert.match(html, /選這裡安排新工作/u);
    assert.match(html, /選好後會依工作內容推薦低成本且足夠可靠的模型/u);
    assert.match(html, /建議模型與推理強度/u);
    assert.match(html, /建立新 Codex 工作時，請在模型選擇器套用這個建議/u);
    assert.match(html, /gpt-5\.6-luna/u);
    assert.match(html, /gpt-5\.6-terra/u);
    assert.match(html, /gpt-5\.6-sol/u);
    assert.equal(html.includes("__LATTICE_MODEL_"), false);
    assert.match(html, /snapshotFresh/u);
    assert.match(html, /所有派工選擇均已停用/u);
    assert.match(html, /copied=document\.execCommand\("copy"\)/u);
    assert.match(html, /瀏覽器沒有允許自動複製/u);
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
    const { snapshot } = exportFixture(fixture.repository, fixture.output, fixture.guide);
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
    const { snapshot } = exportFixture(fixture.repository, fixture.output, fixture.guide);
    const item = snapshot.items.find((candidate) => candidate.id === "TASK-101");

    assert.equal(item.outcome, "UNKNOWN");
    assert.equal(item.evidenceState, "partial");
    assert.deepEqual(item.errors, ["目前分支有重複的 TASK 票券"]);
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("projects committed terminal ISSUE-007 and ISSUE-008 evidence", () => {
  for (const issueNumber of ["007", "008"]) {
    const fixture = createIssueFixture({ issueNumber });
    try {
      const { snapshot } = exportFixture(fixture.repository, fixture.output, fixture.guide);
      const issue = snapshot.items.find((candidate) => candidate.id === fixture.issueId);

      assert.equal(issue.outcome, "COMPLETE");
      assert.equal(issue.evidenceState, "complete");
      assert.deepEqual(issue.errors, []);
      assert.equal(issue.ticket, null);
      assert.deepEqual(issue.issue, { status: "complete", file: fixture.fileName });
      assert.deepEqual(issue.delivery, {
        state: "READY",
        ready: true,
        reasonZh: "已提交 ISSUE 終端證據可交付。",
      });
      assert.equal(issue.dispatch.eligible, true);
    } finally {
      rmSync(fixture.root, { recursive: true, force: true });
    }
  }
});

test("keeps a completed TASK dependency-blocked until its committed dependency succeeds", () => {
  const fixture = createFixture({ terminalState: "COMPLETE", ticketStatus: "complete" });
  try {
    const ticketPath = path.join(fixture.repository, "docs", "tickets", "TASK-101-demo.md");
    writeFileSync(
      ticketPath,
      readFileSync(ticketPath, "utf8").replace(
        "branch: feature/task-101-demo",
        "branch: feature/task-101-demo\ndepends_on: [TASK-033]",
      ),
      "utf8",
    );
    writeFileSync(
      path.join(fixture.repository, "docs", "tickets", "TASK-033-dependency.md"),
      "---\nticket_id: TASK-033\nstatus: in_progress\nbranch: feature/task-033-dependency\n---\n",
      "utf8",
    );
    git(fixture.repository, "add", "docs/tickets");
    git(fixture.repository, "commit", "-m", "record incomplete task dependency");

    const { snapshot } = exportFixture(fixture.repository, fixture.output, fixture.guide);
    const task = snapshot.items.find((candidate) => candidate.id === "TASK-101");

    assert.equal(task.outcome, "COMPLETE");
    assert.deepEqual(task.delivery, {
      state: "BLOCKED",
      ready: false,
      reasonZh: "相依 TASK-033 尚未唯一且成功終態，不能交付。",
    });
    assert.equal(task.dispatch.eligible, false);
    assert.match(task.dispatch.reasonZh, /TASK-033/u);
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("exports nonterminal evidence subjects as provenance without blocking a completed TASK delivery", () => {
  const fixture = createFixture({ terminalState: "COMPLETE", ticketStatus: "complete" });
  try {
    const ticketPath = path.join(fixture.repository, "docs", "tickets", "TASK-101-demo.md");
    writeFileSync(
      ticketPath,
      readFileSync(ticketPath, "utf8").replace(
        "branch: feature/task-101-demo\n",
        "branch: feature/task-101-demo\nevidence_subjects: [TASK-033]\n",
      ),
      "utf8",
    );
    writeFileSync(
      path.join(fixture.repository, "docs", "tickets", "TASK-033-subject.md"),
      "---\nticket_id: TASK-033\nstatus: in_progress\ndepends_on:\n  - TASK-999\nbranch: feature/task-033-subject\n---\n",
      "utf8",
    );
    git(fixture.repository, "add", "docs/tickets");
    git(fixture.repository, "commit", "-m", "record nonterminal evidence subject");

    const { snapshot } = exportFixture(fixture.repository, fixture.output, fixture.guide);
    const task = snapshot.items.find((candidate) => candidate.id === "TASK-101");

    assert.equal(task.outcome, "COMPLETE");
    assert.equal(task.delivery.state, "READY");
    assert.equal(task.delivery.ready, true);
    assert.deepEqual(task.ticket.evidenceSubjects, ["TASK-033"]);
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("dashboard fails closed for invalid evidence-subject provenance", () => {
  const cases = [
    {
      name: "missing subject",
      lines: "evidence_subjects: [TASK-033]\n",
    },
    {
      name: "overlap",
      lines: "depends_on: [TASK-033]\nevidence_subjects: [TASK-033]\n",
      subject: "---\nticket_id: TASK-033\nstatus: complete\nbranch: feature/task-033-subject\n---\n",
    },
    {
      name: "self reference",
      lines: "evidence_subjects: [TASK-101]\n",
    },
    {
      name: "cycle",
      lines: "evidence_subjects: [TASK-033]\n",
      subject: "---\nticket_id: TASK-033\nstatus: in_progress\nbranch: feature/task-033-subject\nevidence_subjects: [TASK-101]\n---\n",
    },
  ];
  for (const scenario of cases) {
    const fixture = createFixture({ terminalState: "COMPLETE", ticketStatus: "complete" });
    try {
      const ticketPath = path.join(fixture.repository, "docs", "tickets", "TASK-101-demo.md");
      writeFileSync(
        ticketPath,
        readFileSync(ticketPath, "utf8").replace(
          "branch: feature/task-101-demo\n",
          `branch: feature/task-101-demo\n${scenario.lines}`,
        ),
        "utf8",
      );
      if (scenario.subject) {
        writeFileSync(
          path.join(fixture.repository, "docs", "tickets", "TASK-033-subject.md"),
          scenario.subject,
          "utf8",
        );
      }
      git(fixture.repository, "add", "docs/tickets");
      git(fixture.repository, "commit", "-m", `record ${scenario.name} evidence subject`);

      const { snapshot } = exportFixture(fixture.repository, fixture.output, fixture.guide);
      const task = snapshot.items.find((candidate) => candidate.id === "TASK-101");
      assert.equal(task.delivery.state, "BLOCKED", scenario.name);
      assert.equal(task.delivery.ready, false, scenario.name);
      assert.equal(task.dispatch.eligible, false, scenario.name);
    } finally {
      rmSync(fixture.root, { recursive: true, force: true });
    }
  }
});

test("rejects ISSUE evidence that is uncommitted, duplicate, mismatched, or non-terminal", () => {
  const cases = [
    {
      name: "uncommitted",
      mutate: (fixture) => {
        writeFileSync(
          path.join(fixture.repository, "docs", "issues", fixture.fileName),
          readFileSync(path.join(fixture.repository, "docs", "issues", fixture.fileName), "utf8"),
          "utf8",
        );
        git(fixture.repository, "rm", "docs/issues/ISSUE-007-resource-aware-scheduler.md");
        git(fixture.repository, "commit", "-m", "remove committed issue evidence");
        mkdirSync(path.join(fixture.repository, "docs", "issues"), { recursive: true });
        writeFileSync(
          path.join(fixture.repository, "docs", "issues", fixture.fileName),
          "---\nissue_id: ISSUE-007\nstatus: complete\nbranch: feature/issue-007-resource-aware-scheduler\ndelivery_remote: origin\ndelivery_repository: github.com/example/lattice-devos\ndelivery_push: authorized_non_force_feature_branch\ndelivery_archive: keep_open\n---\n",
          "utf8",
        );
      },
      error: "目前分支找不到已提交的 ISSUE 終端證據",
    },
    {
      name: "duplicate",
      mutate: (fixture) => {
        writeFileSync(
          path.join(fixture.repository, "docs", "issues", "ISSUE-007-duplicate.md"),
          readFileSync(path.join(fixture.repository, "docs", "issues", fixture.fileName), "utf8"),
          "utf8",
        );
        git(fixture.repository, "add", "docs/issues");
        git(fixture.repository, "commit", "-m", "add duplicate issue evidence");
      },
      error: "目前分支有重複的 ISSUE 終端證據",
    },
    {
      name: "mismatch",
      mutate: (fixture) => {
        const evidencePath = path.join(fixture.repository, "docs", "issues", fixture.fileName);
        writeFileSync(
          evidencePath,
          readFileSync(evidencePath, "utf8").replace("issue_id: ISSUE-007", "issue_id: ISSUE-008"),
          "utf8",
        );
        git(fixture.repository, "add", "docs/issues");
        git(fixture.repository, "commit", "-m", "mismatch issue identity");
      },
      error: "ISSUE 終端證據的 issue_id 與檔名或分支不符",
    },
    {
      name: "non-terminal",
      mutate: (fixture) => {
        const evidencePath = path.join(fixture.repository, "docs", "issues", fixture.fileName);
        writeFileSync(
          evidencePath,
          readFileSync(evidencePath, "utf8").replace("status: complete", "status: in_progress"),
          "utf8",
        );
        git(fixture.repository, "add", "docs/issues");
        git(fixture.repository, "commit", "-m", "mark issue evidence non-terminal");
      },
      error: "ISSUE 終端證據尚未進入終態",
    },
  ];
  for (const scenario of cases) {
    const fixture = createIssueFixture();
    try {
      scenario.mutate(fixture);
      const { snapshot } = exportFixture(fixture.repository, fixture.output, fixture.guide);
      const issue = snapshot.items.find((candidate) => candidate.id === "ISSUE-007");

      assert.equal(issue.outcome, "UNKNOWN", scenario.name);
      assert.equal(issue.evidenceState, "partial", scenario.name);
      assert.deepEqual(issue.errors, [scenario.error], scenario.name);
      assert.equal(issue.issue, null, scenario.name);
    } finally {
      rmSync(fixture.root, { recursive: true, force: true });
    }
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

test("treats old or implausibly future snapshots as stale", () => {
  const now = Date.parse("2026-08-20T12:00:00.000Z");
  assert.equal(isSnapshotFresh("2026-08-19T13:00:00.000Z", now), true);
  assert.equal(isSnapshotFresh("2026-08-19T11:59:59.000Z", now), false);
  assert.equal(isSnapshotFresh("2026-08-20T12:04:59.000Z", now), true);
  assert.equal(isSnapshotFresh("2026-08-20T12:05:01.000Z", now), false);
  assert.equal(isSnapshotFresh("not-a-date", now), false);
});

test("recommends the smallest capable Codex setup and escalates high-consequence work", () => {
  const evidenceNow = Date.parse("2026-08-20T13:30:00.000Z");
  assert.deepEqual(
    recommendCodexSetup("", evidenceNow).selection,
    { model: "gpt-5.6-terra", reasoning: "medium", reasoningZh: "中等" },
  );
  assert.deepEqual(
    recommendCodexSetup("修正按鈕顏色和兩段文字", evidenceNow).selection,
    { model: "gpt-5.6-luna", reasoning: "low", reasoningZh: "低" },
  );
  assert.deepEqual(
    recommendCodexSetup("跨模組資料庫遷移與權限安全審查", evidenceNow).selection,
    { model: "gpt-5.6-sol", reasoning: "high", reasoningZh: "高" },
  );
  assert.equal(
    recommendCodexSetup("改文字，但也要變更權限", evidenceNow).selection.model,
    "gpt-5.6-sol",
  );
  assert.match(recommendCodexSetup("一般功能開發", evidenceNow).reasonZh, /品質、速度與成本/u);
  assert.equal(recommendCodexSetup("一般功能開發", evidenceNow).checkedAt, "2026-08-20");
  const stale = recommendCodexSetup("改文字", Date.parse("2026-09-21T00:00:00.000Z"));
  assert.equal(stale.fresh, false);
  assert.equal(stale.selection.model, null);
  assert.match(stale.reasonZh, /超過 30 天/u);
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
    const { snapshot } = exportFixture(fixture.repository, fixture.output, fixture.guide);
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
    const { snapshot } = exportFixture(fixture.repository, fixture.output, fixture.guide);
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
    await assert.rejects(
      writeDashboard({ schema: "lattice.engineering-status/2.0", items: [] }, root),
      /invalid engineering-status snapshot/u,
    );
    assert.equal(readFileSync(path.join(root, "status.json"), "utf8"), "old-json\n");
    assert.equal(readFileSync(path.join(root, "index.html"), "utf8"), "old-html\n");
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("rejects orphaned, duplicate, and cyclic V2 tree relationships", async () => {
  const fixture = createFixture({ terminalState: "COMPLETE", ticketStatus: "complete" });
  try {
    const { snapshot } = exportFixture(fixture.repository, fixture.output, fixture.guide);
    const originalJson = readFileSync(path.join(fixture.output, "status.json"), "utf8");
    const child = snapshot.items.find((item) => item.tree.parentKey !== null);
    const parent = snapshot.items.find((item) => item.treeKey === child.tree.parentKey);

    const orphaned = structuredClone(snapshot);
    const orphanParent = orphaned.items.find((item) => item.treeKey === parent.treeKey);
    orphanParent.tree.childrenKeys = orphanParent.tree.childrenKeys.filter(
      (key) => key !== child.treeKey,
    );
    await assert.rejects(
      writeDashboard(orphaned, fixture.output),
      /invalid engineering-status snapshot/u,
    );

    const duplicated = structuredClone(snapshot);
    duplicated.items
      .find((item) => item.treeKey === parent.treeKey)
      .tree.childrenKeys.push(child.treeKey);
    await assert.rejects(
      writeDashboard(duplicated, fixture.output),
      /invalid engineering-status snapshot/u,
    );

    const cyclic = structuredClone(snapshot);
    const cyclicRoot = cyclic.items.find((item) => item.tree.parentKey === null);
    const cyclicChild = cyclic.items.find((item) => item.tree.parentKey !== null);
    cyclicRoot.tree.parentKey = cyclicChild.treeKey;
    cyclicRoot.tree.parentBranch = cyclicChild.branch;
    cyclicChild.tree.childrenKeys.push(cyclicRoot.treeKey);
    cyclic.tree.roots = [];
    await assert.rejects(
      writeDashboard(cyclic, fixture.output),
      /invalid engineering-status snapshot/u,
    );

    assert.equal(readFileSync(path.join(fixture.output, "status.json"), "utf8"), originalJson);
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
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
  {
    skip: process.platform !== "win32"
      ? "Windows only"
      : !existsSync(dashboardLauncher)
        ? "launcher is not part of this product branch"
        : false,
  },
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
      const fixtureGuide = path.join(
        fixture.repository,
        "tools",
        "engineering-status-dashboard",
        "branch-guide.zh-TW.json",
      );
      mkdirSync(path.dirname(fixtureExporter), { recursive: true });
      mkdirSync(path.dirname(fixtureTemplate), { recursive: true });
      copyFileSync(dashboardLauncher, launcher);
      copyFileSync(exporter, fixtureExporter);
      copyFileSync(
        path.join(projectRoot, "tools", "engineering-status-dashboard", "index.template.html"),
        fixtureTemplate,
      );
      copyFileSync(fixture.guide, fixtureGuide);
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

test("keeps remote divergence while a non-canonical integration branch cannot impersonate a TASK", () => {
  const fixture = createFixture({ terminalState: "WAITING_DEPENDENCY" });
  const integrationWorktree = path.join(fixture.root, "integration-worktree");
  try {
    writeFileSync(path.join(fixture.repository, "ahead.txt"), "ahead\n", "utf8");
    git(fixture.repository, "add", "ahead.txt");
    git(fixture.repository, "commit", "-m", "local ahead commit");
    git(fixture.repository, "worktree", "add", "-b", "integration/task-101-task-102", integrationWorktree, "main");

    const { snapshot } = exportFixture(fixture.repository, fixture.output, fixture.guide);
    const task = snapshot.items.find((candidate) => candidate.id === "TASK-101");
    const integration = snapshot.items.find(
      (candidate) => candidate.branch === "integration/task-101-task-102",
    );

    assert.equal(task.outcome, "WAITING_DEPENDENCY");
    assert.equal(task.git.sync.state, "ahead");
    assert.equal(task.git.sync.ahead, 1);
    assert.equal(task.git.sync.behind, 0);
    assert.equal(integration.id, "integration/task-101-task-102");
    assert.equal(integration.kind, "BRANCH");
    assert.equal(integration.outcome, "UNKNOWN");
    assert.equal(integration.delivery.state, "NOT_APPLICABLE");
    assert.equal(integration.dispatch.eligible, false);
    assert.equal(integration.git.sync.state, "no-upstream");
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

    const { snapshot } = exportFixture(fixture.repository, fixture.output, fixture.guide);
    const item = snapshot.items.find((candidate) => candidate.id === "TASK-101");
    assert.equal(snapshot.sources.gitRemote.state, "available");
    assert.equal(item.git.sync.state, "remote-changed");
    assert.equal(item.git.sync.remoteVerified, true);
    assert.equal(item.dispatch.eligible, false);
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("fails closed when a completed branch has no Chinese purpose", () => {
  const fixture = createFixture({ terminalState: "COMPLETE", ticketStatus: "complete" });
  try {
    const guide = JSON.parse(readFileSync(fixture.guide, "utf8"));
    delete guide.branches["feature/task-101-demo"];
    writeFileSync(fixture.guide, `${JSON.stringify(guide, null, 2)}\n`, "utf8");
    const { snapshot } = exportFixture(fixture.repository, fixture.output, fixture.guide);
    const item = snapshot.items.find((candidate) => candidate.id === "TASK-101");

    assert.equal(item.outcome, "COMPLETE");
    assert.equal(item.guideMatched, false);
    assert.equal(item.dispatch.eligible, false);
    assert.match(item.dispatch.reasonZh, /白話中文用途/u);
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("fails closed when a completed branch has uncommitted changes", () => {
  const fixture = createFixture({ terminalState: "COMPLETE", ticketStatus: "complete" });
  try {
    writeFileSync(path.join(fixture.repository, "unfinished.txt"), "not committed\n", "utf8");
    const { snapshot } = exportFixture(fixture.repository, fixture.output, fixture.guide);
    const item = snapshot.items.find((candidate) => candidate.id === "TASK-101");

    assert.equal(item.git.clean, false);
    assert.equal(item.dispatch.eligible, false);
    assert.match(item.dispatch.reasonZh, /未提交變更/u);
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("keeps multiple detached worktrees as distinct reachable tree nodes", () => {
  const fixture = createFixture();
  try {
    git(fixture.repository, "worktree", "add", "--detach", path.join(fixture.root, "detached-one"), "main");
    git(fixture.repository, "worktree", "add", "--detach", path.join(fixture.root, "detached-two"), "main");
    const { snapshot } = exportFixture(fixture.repository, fixture.output, fixture.guide);
    const detached = snapshot.items.filter((item) => item.worktree.detached);
    const byKey = new Map(snapshot.items.map((item) => [item.treeKey, item]));
    const visited = new Set();
    const visit = (key) => {
      if (visited.has(key)) return;
      visited.add(key);
      for (const child of byKey.get(key).tree.childrenKeys) visit(child);
    };
    for (const rootKey of snapshot.tree.roots) visit(rootKey);

    assert.equal(detached.length, 2);
    assert.equal(new Set(detached.map((item) => item.treeKey)).size, 2);
    assert.ok(detached.every((item) => item.dispatch.eligible === false));
    assert.equal(visited.size, snapshot.items.length);
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("prefers a named stable branch over a detached worktree at the same commit", () => {
  const fixture = createFixture();
  const detachedPath = path.join(fixture.root, "detached-at-feature");
  const childPath = path.join(fixture.root, "named-child");
  try {
    git(fixture.repository, "worktree", "add", "--detach", detachedPath, "feature/task-101-demo");
    git(
      fixture.repository,
      "worktree",
      "add",
      "-b",
      "feature/task-102-child",
      childPath,
      "feature/task-101-demo",
    );
    writeFileSync(path.join(childPath, "child.txt"), "child branch\n", "utf8");
    git(childPath, "add", "child.txt");
    git(childPath, "commit", "-m", "add named child");

    const { snapshot } = exportFixture(fixture.repository, fixture.output, fixture.guide);
    const child = snapshot.items.find((item) => item.branch === "feature/task-102-child");
    assert.equal(child.tree.parentBranch, "feature/task-101-demo");
    assert.equal(child.tree.relation, "descendant");
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("marks the snapshot partial and disables dispatch when Git ancestry fails", () => {
  const fixture = createFixture({ terminalState: "COMPLETE", ticketStatus: "complete" });
  try {
    writeFileSync(
      path.join(fixture.repository, ".git", "refs", "heads", "broken-ancestry-ref"),
      `${"1".repeat(40)}\n`,
      "utf8",
    );
    const { snapshot } = exportFixture(fixture.repository, fixture.output, fixture.guide);
    assert.equal(snapshot.sources.gitAncestry.state, "partial");
    assert.equal(snapshot.completeness, "partial");
    assert.equal(snapshot.recommendedBranch, null);
    assert.ok(snapshot.items.every((item) => item.dispatch.eligible === false));
    assert.ok(snapshot.items.every((item) => /版本關係無法讀取/u.test(item.dispatch.reasonZh)));
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("builds a real top-down ancestry tree and enables only a proven branch", () => {
  const fixture = createFixture({ terminalState: "COMPLETE", ticketStatus: "complete" });
  try {
    const { snapshot } = exportFixture(fixture.repository, fixture.output, fixture.guide);
    const root = snapshot.items.find((item) => item.branch === "main");
    const task = snapshot.items.find((item) => item.branch === "feature/task-101-demo");

    assert.equal(snapshot.repository.defaultBranch, "main");
    assert.equal(snapshot.sources.gitAncestry.state, "available");
    assert.equal(root.kind, "BASE");
    assert.equal(root.isDefaultBranch, true);
    assert.equal(root.dispatch.eligible, true);
    assert.equal(task.tree.parentBranch, "main");
    assert.equal(task.tree.relation, "descendant");
    assert.equal(task.tree.depth, 1);
    assert.equal(task.dispatch.eligible, true);
    assert.equal(snapshot.recommendedBranch, "feature/task-101-demo");
    assert.deepEqual(snapshot.tree.roots, [root.treeKey]);
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});
