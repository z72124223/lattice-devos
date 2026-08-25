import assert from "node:assert/strict";
import { readFile, stat } from "node:fs/promises";
import path from "node:path";
import test from "node:test";

const root = path.resolve(".");
const read = (file) => readFile(path.join(root, file), "utf8");

test("public GitHub surface reflects the current local product", async () => {
  const [readme, handoff, plans, workflow] = await Promise.all([
    read("README.md"),
    read("HANDOFF.md"),
    read("PLANS.md"),
    read(".github/workflows/ci.yml"),
  ]);

  assert.match(readme, /本機優先的 AI 開發工作控制台與耐久執行環境/u);
  assert.match(readme, /product\/lattice-control-mvp/u);
  assert.match(readme, /目前尚未選定 `LICENSE`/u);
  assert.match(readme, /公開可見性與 `git clone` 功能不等於開源授權/u);
  assert.match(readme, /沒有公開雲端服務/u);
  assert.match(handoff, /Runtime 工作真相：LATTICE／PostgreSQL/u);
  assert.match(handoff, /程式交付真相：GitHub 提交、PR 與 CI/u);
  assert.match(plans, /正式產品分支是 `product\/lattice-control-mvp`/u);
  assert.match(plans, /歷史保留在 Git 與 LATTICE/u);
  assert.match(workflow, /- product\/lattice-control-mvp/u);
  assert.doesNotMatch(workflow, /^\s*- main\s*$/mu);

  const staleClaims =
    /COMPLETE \/ DELIVERY_PENDING|未推送、未開 PR|未合併產品分支|CURRENT TASK-\d+|目前步驟|GitHub 有 \d+ 個非預設公開分支|CI 的 push 觸發仍指向不存在的 `main`|\b[0-9a-f]{8,40}\b/u;
  assert.doesNotMatch(readme, staleClaims);
  assert.doesNotMatch(handoff, staleClaims);
  assert.doesNotMatch(plans, staleClaims);
  assert.ok(Buffer.byteLength(handoff, "utf8") < 4_096);
  assert.ok(Buffer.byteLength(plans, "utf8") < 12_000);
});

test("relative README links resolve inside the repository", async () => {
  const readme = await read("README.md");
  const links = [...readme.matchAll(/\[[^\]]+\]\(([^)]+)\)/gu)]
    .map((match) => match[1])
    .filter((target) => !/^(?:https?:|#)/u.test(target));

  for (const target of links) {
    const resolved = path.resolve(root, decodeURIComponent(target));
    assert.ok(resolved.startsWith(root + path.sep), `link escapes repository: ${target}`);
    await stat(resolved);
  }
});
