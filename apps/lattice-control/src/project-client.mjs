import path from "node:path";
import process from "node:process";
import { pathToFileURL } from "node:url";
import { isDeepStrictEqual } from "node:util";

import {
  defaultControlOrigin,
  normalizeControlOrigin,
  requestJson,
  requireText,
  resolveProject,
} from "./control-client.mjs";

function requireProjectRecord(value) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error("Control returned an invalid project record");
  }
  for (const field of ["id", "name", "canonical_path", "created_at", "updated_at"]) {
    requireText(value[field], `project ${field}`);
  }
  if (
    value.schema_version !== "lattice.control.project-catalog.v1"
    || value.record_kind !== "CONTROL_LOCAL_CATALOG"
    || value.registry_authority !== "NONE"
    || value.registry_project_id !== null
    || value.control_project_id !== value.id
  ) {
    throw new Error("Control project catalog contract is invalid or claims unsupported authority");
  }
  if (!value.git_observation || !value.rule_index) {
    throw new Error("Control project observations are missing");
  }
  return value;
}

function assertReplayMatches(expected, replay) {
  const { created: _created, ...expectedProjection } = expected;
  if (!isDeepStrictEqual(expectedProjection, replay)) {
    throw new Error("persisted project replay did not match the complete catalog projection");
  }
}

async function resolveProjectId(fetchImpl, origin, options, timeoutMs) {
  if (options.projectId && !options.projectName) return requireText(options.projectId, "project ID");
  const state = await requestJson(fetchImpl, `${origin}/api/state`, {}, timeoutMs);
  return resolveProject(state.body.projects, options).id;
}

async function readProject(fetchImpl, origin, projectId, timeoutMs) {
  const result = await requestJson(
    fetchImpl,
    `${origin}/api/projects/${encodeURIComponent(projectId)}`,
    {},
    timeoutMs,
  );
  return requireProjectRecord(result.body);
}

export async function runProjectCommand({
  command,
  name,
  rootPath,
  projectId,
  projectName,
  controlOrigin = defaultControlOrigin,
  fetchImpl = fetch,
  requestTimeoutMs = 60_000,
}) {
  const origin = normalizeControlOrigin(controlOrigin);
  if (!Number.isInteger(requestTimeoutMs) || requestTimeoutMs < 1 || requestTimeoutMs > 60_000) {
    throw new TypeError("request timeout must be an integer from 1 to 60000 milliseconds");
  }
  if (!["register", "refresh", "read"].includes(command)) {
    throw new TypeError("project command must be register, refresh, or read");
  }

  if (command === "register") {
    if (projectId || projectName) throw new TypeError("register does not accept a project selector");
    const response = await requestJson(fetchImpl, `${origin}/api/projects`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        name: requireText(name, "project name"),
        rootPath: path.resolve(requireText(rootPath, "project path")),
      }),
    }, requestTimeoutMs);
    if (![200, 201].includes(response.response.status)) {
      throw new Error(`Control returned unexpected project status ${response.response.status}`);
    }
    const recorded = requireProjectRecord(response.body);
    const replay = await readProject(fetchImpl, origin, recorded.id, requestTimeoutMs);
    assertReplayMatches(recorded, replay);
    return {
      status: response.response.status === 201 ? "PROJECT_REGISTERED" : "PROJECT_UPDATED",
      created: response.response.status === 201,
      project: replay,
    };
  }

  if (name || rootPath) throw new TypeError(`${command} does not accept project name or path fields`);
  const selectedId = await resolveProjectId(
    fetchImpl,
    origin,
    { projectId, projectName },
    requestTimeoutMs,
  );
  if (command === "refresh") {
    const refreshed = await requestJson(
      fetchImpl,
      `${origin}/api/projects/${encodeURIComponent(selectedId)}/refresh`,
      {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: "{}",
      },
      requestTimeoutMs,
    );
    const recorded = requireProjectRecord(refreshed.body);
    const replay = await readProject(fetchImpl, origin, selectedId, requestTimeoutMs);
    assertReplayMatches(recorded, replay);
    return { status: "PROJECT_REFRESHED", created: false, project: replay };
  }
  return {
    status: "PROJECT_READ",
    created: false,
    project: await readProject(fetchImpl, origin, selectedId, requestTimeoutMs),
  };
}

function available(value) {
  return value == null ? "無法取得" : String(value);
}

function terminalSafe(value) {
  return String(value).replace(/[\u0000-\u001f\u007f-\u009f]/gu, (character) => (
    `\\u{${character.codePointAt(0).toString(16).padStart(2, "0")}}`
  ));
}

export function formatProjectResult(result) {
  const project = requireProjectRecord(result.project);
  const rules = project.rule_index;
  const git = project.git_observation;
  const lines = [
    result.status === "PROJECT_REGISTERED"
      ? "專案登記：已建立"
      : result.status === "PROJECT_UPDATED"
        ? "專案登記：已存在並更新觀察"
        : result.status === "PROJECT_REFRESHED"
          ? "專案觀察：已刷新"
          : "專案登記：已讀回",
    "",
    "Control 本機專案目錄項目（locator，不是 Project Registry 身分或權威收據）",
    `Schema：${project.schema_version}`,
    `Record kind：${project.record_kind}`,
    `Registry authority：${project.registry_authority}`,
    `Control Project ID：${terminalSafe(project.id)}`,
    `顯示名稱：${terminalSafe(project.name)}`,
    `Canonical 路徑 locator：${terminalSafe(project.canonical_path)}`,
    `Repository 根目錄（最近確認）：${terminalSafe(project.repo_root ?? "無")}`,
    `Repository 根目錄觀察時間：${project.repo_root_observed_at ?? "尚未確認"}`,
    `建立時間：${project.created_at}`,
    `更新時間：${project.updated_at}`,
    "此目錄項目不得用於 Policy、approval、lease 或 Runtime authority。",
    "",
    `規則索引（觀察時間：${rules.observed_at}；狀態：${rules.status}）`,
  ];
  if (rules.documents.length === 0) lines.push("- 沒有發現可索引的權威文件");
  for (const document of rules.documents) {
    lines.push(
      `- ${terminalSafe(document.relative_path)} | SHA-256 ${document.sha256} | ${terminalSafe(document.purpose)}`,
    );
  }
  if (rules.missing_standard_documents.length > 0) {
    lines.push(`- 已確認未存在：${rules.missing_standard_documents.map(terminalSafe).join("、")}`);
  }
  for (const failure of rules.failures) {
    lines.push(
      `- 部分失敗：${terminalSafe(failure.code)} ${terminalSafe(failure.relative_path ?? "")}`.trimEnd(),
    );
  }

  if (project.last_refresh_failure) {
    lines.push(
      "",
      `最近一次 refresh 失敗：${terminalSafe(project.last_refresh_failure.code)}`,
      `失敗時間：${project.last_refresh_failure.observed_at}`,
      `說明：${terminalSafe(project.last_refresh_failure.message)}`,
      "下方 Git／規則內容仍是最後一次成功保存的 observation，並非本次刷新結果。",
    );
  }

  lines.push("", `Git 觀察（時間點：${git.observed_at}；可用 refresh 更新）`);
  if (git.is_repository === false) {
    lines.push("- 不是 Git repository");
  } else if (git.is_repository == null) {
    lines.push("- 是否為 Git repository：無法確定");
  } else {
    lines.push(`- Branch：${terminalSafe(git.detached ? "detached" : available(git.branch))}`);
    lines.push(`- HEAD：${available(git.head_sha)}`);
    lines.push(`- Dirty：${git.dirty == null ? "無法取得" : git.dirty ? "是" : "否"}`);
    lines.push(`- Upstream：${terminalSafe(available(git.upstream))}`);
    lines.push(`- Ahead / behind：${available(git.ahead)} / ${available(git.behind)}`);
    if (git.remotes.length === 0) lines.push("- Remotes：無");
    for (const remote of git.remotes) {
      lines.push(
        `- Remote ${terminalSafe(remote.name)} (${remote.direction})：${terminalSafe(remote.url)}`
        + (remote.credentials_redacted ? " [credential 已移除]" : ""),
      );
    }
  }
  for (const failure of git.failures) {
    lines.push(`- 部分失敗：${terminalSafe(failure.code)}`);
  }
  return lines.join("\n");
}

export function parseProjectArguments(argv) {
  const [command, ...arguments_] = argv;
  if (command === "--help" || command == null) return { help: true };
  const options = { command };
  for (let index = 0; index < arguments_.length; index += 1) {
    const argument = arguments_[index];
    if (argument === "--help") return { help: true };
    if (argument === "--json") {
      if (options.json) throw new TypeError("duplicate option: --json");
      options.json = true;
      continue;
    }
    const key = new Map([
      ["--origin", "controlOrigin"],
      ["--name", "name"],
      ["--path", "rootPath"],
      ["--project-id", "projectId"],
      ["--project-name", "projectName"],
    ]).get(argument);
    if (!key) throw new TypeError(`unknown option: ${argument}`);
    if (options[key] !== undefined) throw new TypeError(`duplicate option: ${argument}`);
    const value = arguments_[index + 1];
    if (value === undefined || value.startsWith("--")) {
      throw new TypeError(`missing value for ${argument}`);
    }
    options[key] = value;
    index += 1;
  }
  return options;
}

const usage = [
  "LATTICE Control 專案登記 CLI",
  "",
  "npm.cmd run control:project -- register --name <顯示名稱> --path <絕對路徑>",
  "npm.cmd run control:project -- refresh --project-id <id>",
  "npm.cmd run control:project -- read --project-id <id>",
  "",
  "read/refresh 可用 --project-name <名稱> 取代 --project-id；--json 輸出機器可讀 JSON。",
  "--origin 預設為 http://127.0.0.1:4317，且只接受 HTTP loopback。",
].join("\n");

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    const options = parseProjectArguments(process.argv.slice(2));
    if (options.help) process.stdout.write(`${usage}\n`);
    else {
      const result = await runProjectCommand(options);
      process.stdout.write(options.json
        ? `${JSON.stringify(result)}\n`
        : `${formatProjectResult(result)}\n`);
    }
  } catch (error) {
    process.stderr.write(`錯誤：${error.message}\n`);
    process.exitCode = 1;
  }
}
