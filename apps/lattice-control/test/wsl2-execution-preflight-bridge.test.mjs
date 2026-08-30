import assert from "node:assert/strict";
import test from "node:test";

import {
  parseWsl2PreflightBridgeInput,
  runWsl2ExecutionPreflightBridge,
  validateWsl2PreflightBridgeRequest,
  WSL2_PREFLIGHT_BRIDGE_MAX_INPUT_BYTES,
} from "../src/wsl2-execution-preflight-bridge.mjs";

const typed = (kind, value) => `${kind}:sha256:${value.repeat(64)}`;

function requestFixture() {
  return {
    schema: "lattice.wsl2-execution-preflight-request/1.0",
    template_descriptor: { template: true },
    windows_worktree_path: String.raw`\\wsl.localhost\Ubuntu\home\zk\task\managed-worktrees\work-a`,
    task_ref: "a".repeat(64),
    attempt: 1,
    worktree_ref: typed("worktree", "b"),
    expected_repository_head: "c".repeat(40),
    process_fence: "d".repeat(64),
    retry_of: null,
    reconnect_of: null,
  };
}

test("one-shot bridge binds exact task worktree and emits a task/attempt/worktree preflight result", async () => {
  const request = requestFixture();
  const calls = [];
  const template = { verification_toolchain: { task_ref: request.task_ref } };
  const environment = {
    schema: "lattice.execution-environment.wsl2-linux/1.1",
    identity_digest: typed("execution-environment", "e"),
    linux: { cwd: "/home/zk/task/managed-worktrees/work-a", repository_head: request.expected_repository_head },
    path_mapping: { windows_path: request.windows_worktree_path, linux_path: "/home/zk/task/managed-worktrees/work-a" },
  };
  const receipt = {
    task_ref: request.task_ref,
    attempt: request.attempt,
    worktree_ref: request.worktree_ref,
    execution_environment_ref: environment.identity_digest,
    repository_head: request.expected_repository_head,
    process_fence: { fence: request.process_fence },
    provider_effect_count: 0,
  };
  const result = await runWsl2ExecutionPreflightBridge(request, {
    validateDescriptor(value) { calls.push(["validate", value]); return template; },
    bindWorktree(value, windowsPath, observed) {
      calls.push(["bind", value, windowsPath, observed]);
      assert.equal(observed.head, request.expected_repository_head);
      assert.match(observed.repository_identity, /^repository:sha256:[a-f0-9]{64}$/u);
      return environment;
    },
    async preflight(value, context) {
      calls.push(["preflight", value, context]);
      assert.deepEqual(context, {
        processFence: request.process_fence, taskRef: request.task_ref, attempt: 1,
        worktreeRef: request.worktree_ref, retryOf: null, reconnectOf: null,
      });
      return { environment, receipt };
    },
  });
  assert.equal(result.schema, "lattice.wsl2-execution-preflight-result/1.0");
  assert.equal(result.status, "PASS");
  assert.equal(result.environment, environment);
  assert.equal(result.receipt, receipt);
  assert.match(result.result_digest, /^wsl2-preflight-result:sha256:[a-f0-9]{64}$/u);
  assert.deepEqual(calls.map(([kind]) => kind), ["validate", "bind", "preflight"]);
});

test("bridge request is exact-key and fail-closed for task, lineage, UNC, HEAD, and fence substitutions", () => {
  const request = requestFixture();
  assert.doesNotThrow(() => validateWsl2PreflightBridgeRequest(request));
  for (const changed of [
    { ...request, extra: true },
    { ...request, task_ref: "short" },
    { ...request, worktree_ref: typed("path-mapping", "b") },
    { ...request, windows_worktree_path: String.raw`C:\repo` },
    { ...request, expected_repository_head: "c".repeat(64) },
    { ...request, process_fence: "d".repeat(40) },
    {
      ...request,
      attempt: 2,
      retry_of: typed("wsl2-preflight", "e"),
      reconnect_of: typed("wsl2-preflight", "f"),
    },
  ]) {
    assert.throws(() => validateWsl2PreflightBridgeRequest(changed), {
      code: "WSL2_PREFLIGHT_BRIDGE_REQUEST_REJECTED",
    });
  }
  assert.doesNotThrow(() => validateWsl2PreflightBridgeRequest({
    ...request, reconnect_of: typed("wsl2-preflight", "e"),
  }));
  assert.doesNotThrow(() => validateWsl2PreflightBridgeRequest({
    ...request, retry_of: typed("wsl2-preflight", "e"),
  }));
  assert.doesNotThrow(() => validateWsl2PreflightBridgeRequest({
    ...request, attempt: 2,
  }));
  assert.doesNotThrow(() => validateWsl2PreflightBridgeRequest({
    ...request, attempt: 2, retry_of: typed("wsl2-preflight", "e"),
  }));
  assert.doesNotThrow(() => validateWsl2PreflightBridgeRequest({
    ...request, attempt: 2, reconnect_of: typed("wsl2-preflight", "e"),
  }));
});

test("one-shot JSONL input rejects multiple records and bytes beyond the fixed bound", () => {
  const request = requestFixture();
  assert.deepEqual(parseWsl2PreflightBridgeInput(Buffer.from(`${JSON.stringify(request)}\n`, "utf8")), request);
  assert.throws(() => parseWsl2PreflightBridgeInput(Buffer.from("{}\n{}\n", "utf8")), {
    code: "WSL2_PREFLIGHT_BRIDGE_REQUEST_REJECTED",
  });
  assert.throws(() => parseWsl2PreflightBridgeInput(Buffer.alloc(WSL2_PREFLIGHT_BRIDGE_MAX_INPUT_BYTES + 1)), {
    code: "WSL2_PREFLIGHT_BRIDGE_INPUT_BOUND_EXCEEDED",
  });
});
