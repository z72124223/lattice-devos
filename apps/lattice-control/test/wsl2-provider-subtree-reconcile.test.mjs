import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import test from "node:test";

import { canonicalJson } from "../src/wsl2-execution-domain.mjs";

import {
  buildWsl2ProviderSubtreeMarker,
  buildWsl2ProviderSubtreeReceipt,
  buildWsl2ReviewerSubtreeMarker,
  canonicalWslBootIdDigest,
  canonicalProviderSubtreeDigest,
  reconcileWsl2ProviderSubtree,
  reconcileWsl2ReviewerSubtree,
  validateWsl2ProviderSubtreeMarker,
  validateWsl2ProviderSubtreeReceipt,
} from "../src/wsl2-provider-subtree-reconcile.mjs";

const hex = (value) => value.repeat(64);
const typed = (domain, value) => `${domain}:sha256:${hex(value)}`;

test("WSL boot identity hashes the canonical trimmed boot id", () => {
  const bootId = "4f3f7d25-472e-4ca0-b5ad-809445b9e6ce";
  const digest = createHash("sha256").update(bootId, "utf8").digest("hex");
  assert.equal(canonicalWslBootIdDigest(`${bootId}\n`), `wsl-boot:sha256:${digest}`);
});

function marker() {
  const unit = `lattice-wsl2-${"a".repeat(16)}-provider-${"f".repeat(12)}.service`;
  return {
    schema: "lattice.wsl2-process-fence/1.1",
    fence: hex("f"),
    unit,
    execution_environment_ref: typed("execution-environment", "e"),
    credential_seal_digest: typed("credential-seal", "c"),
    boot_id_digest: typed("wsl-boot", "b"),
    pid: 123,
    process_start_ticks: "456",
    process_group_id: 123,
    cgroup_path: `/user.slice/user-1000.slice/user@1000.service/app.slice/${unit}`,
    cgroup_version: 2,
    delegated: false,
    attempt: 1,
    retry_of: null,
    reconnect_of: null,
  };
}

function seal(path, value, extra = {}) {
  return {
    ...extra,
    path,
    resolved_path: path,
    sha256: hex(value),
    device: "2049",
    inode: String(12_000 + Number.parseInt(value, 16)),
    owner_uid: 0,
    mode: 0o500,
    size: 4096,
  };
}

function subtreeExit(processMarker = marker()) {
  return {
    schema: "lattice.wsl2-subtree-exit/1.2",
    fence: processMarker.fence,
    unit: processMarker.unit,
    execution_environment_ref: processMarker.execution_environment_ref,
    credential_seal_digest: processMarker.credential_seal_digest,
    cgroup_path: processMarker.cgroup_path,
    zero_descendants: true,
    credential_seal_intact: true,
    credential_watch_intact: true,
    keyring_daemon_sha256: hex("7"),
    keyring_library_manifest_digest: typed("keyring-library-manifest", "8"),
    tool_input_identities: {
      executable: seal("/home/zk/task/codex", "1"),
      verifier_tool: null,
      sandbox_helper: seal("/usr/bin/bwrap", "2"),
      node_runtime: null,
      rustc: null,
      rustdoc: null,
      keyring_daemon: seal("/home/zk/task/keyring-daemon", "7"),
      keyring_libraries: [
        seal("/home/zk/task/keyring/libgck-1.so.0.0.0", "8", {
          manifest_path: "libgck-1.so.0.0.0",
        }),
        seal("/home/zk/task/keyring/libgcr-base-3.so.1.0.0", "9", {
          manifest_path: "libgcr-base-3.so.1.0.0",
        }),
      ],
    },
    stdout_bytes: 128,
    stderr_bytes: 256,
    stdout_limit_bytes: 262_144,
    stderr_limit_bytes: 262_144,
    output_bound_exceeded: false,
    timeout_ms: 1_000,
    timed_out: false,
    interrupted: false,
    stdin_bytes: 0,
    stdin_sha256: createHash("sha256").update(Buffer.alloc(0)).digest("hex"),
    stdin_complete: true,
    attempt: processMarker.attempt,
    retry_of: processMarker.retry_of,
    reconnect_of: processMarker.reconnect_of,
    exit_code: 0,
    exit_signal: null,
  };
}

function outerPostExit(processMarker = marker()) {
  return {
    schema: "lattice.wsl2-provider-outer-post-exit/1.0",
    unit: processMarker.unit,
    fence: processMarker.fence,
    cgroup_path: processMarker.cgroup_path,
    boot_id_digest: processMarker.boot_id_digest,
    active_state: "inactive",
    sub_state: "dead",
    result: "success",
    delegate: "no",
    cgroup_exists: false,
    populated: null,
  };
}

function context() {
  return {
    task_ref: hex("a"),
    attempt: 1,
    packet_digest: typed("attempt-packet", "d"),
    worktree_ref: typed("worktree", "w".replace("w", "1")),
    repository_head: "2".repeat(40),
    execution_environment_ref: typed("execution-environment", "e"),
    descriptor_digest: hex("3"),
    source_preflight_descriptor_digest: hex("4"),
    source_preflight_content_digest: hex("5"),
    source_preflight_receipt_digest: typed("wsl2-preflight", "6"),
  };
}

test("provider subtree OPEN and CLOSED receipts are exact, digest-bound, bounded and secret-free", () => {
  const processMarker = marker();
  const open = buildWsl2ProviderSubtreeMarker(context(), processMarker);
  assert.equal(open.status, "OPEN");
  assert.equal(open.provider_effect_count, 0);
  assert.equal(validateWsl2ProviderSubtreeMarker(open).marker_digest, open.marker_digest);
  assert.equal(
    open.marker_digest,
    canonicalProviderSubtreeDigest("provider-subtree-marker", open, "marker_digest"),
  );

  const closed = buildWsl2ProviderSubtreeReceipt(
    open,
    subtreeExit(processMarker),
    outerPostExit(processMarker),
    2,
  );
  assert.equal(closed.status, "CLOSED");
  assert.equal(closed.source_marker_digest, open.marker_digest);
  assert.equal(closed.provider_effect_count, 2);
  assert.equal(validateWsl2ProviderSubtreeReceipt(closed, open).receipt_digest, closed.receipt_digest);
  assert.equal(
    closed.receipt_digest,
    canonicalProviderSubtreeDigest("provider-subtree-receipt", closed, "receipt_digest"),
  );

  for (const mutation of [
    (value) => { value.process_marker.unit += ".substituted"; },
    (value) => { value.subtree_exit.zero_descendants = false; },
    (value) => { value.outer_post_exit.populated = 1; value.outer_post_exit.cgroup_exists = true; },
    (value) => { value.unexpected = true; },
    (value) => { value.outer_post_exit.result = "token=secret-value"; },
  ]) {
    const changed = structuredClone(closed);
    mutation(changed);
    changed.receipt_digest = canonicalProviderSubtreeDigest(
      "provider-subtree-receipt", changed, "receipt_digest",
    );
    assert.throws(() => validateWsl2ProviderSubtreeReceipt(changed, open), {
      code: "WSL2_PROVIDER_SUBTREE_RECEIPT_REJECTED",
    });
  }
});

function preflightReceipt() {
  const processMarker = marker();
  return {
    schema: "lattice.wsl2-zero-model-preflight/1.0",
    status: "PASS",
    task_ref: hex("a"),
    attempt: 1,
    worktree_ref: typed("worktree", "1"),
    repository_head: "2".repeat(40),
    execution_environment_ref: processMarker.execution_environment_ref,
    credential_seal_digest: processMarker.credential_seal_digest,
    process_fence: {
      fence: processMarker.fence,
      boot_id_digest: processMarker.boot_id_digest,
    },
    bounds: { stdout_limit_bytes: 262_144, stderr_limit_bytes: 262_144 },
    timeout: { timeout_ms: 1_000 },
    continuation: { retry_of: null, reconnect_of: null },
    provider_effect_count: 0,
    receipt_digest: typed("wsl2-preflight", "6"),
  };
}

function reconciliationInput(openMarker = null) {
  const receipt = preflightReceipt();
  const receiptJson = JSON.stringify(receipt);
  const environment = {
    distribution: "Ubuntu",
    identity_digest: receipt.execution_environment_ref,
    gateway: { windows_path: String.raw`C:\Windows\System32\wsl.exe` },
    linux: { repository_head: receipt.repository_head },
    process_fence: {
      identity_digest: typed("wsl2-process-fence", "9"),
      user_runtime_dir: "/run/user/1000",
      cgroup_mount: "/sys/fs/cgroup",
      systemctl_path: "/usr/bin/systemctl",
      supervisor_bootstrap_node: { path: "/usr/bin/node" },
    },
    verification_toolchain: { owner_uid: 1000 },
  };
  const descriptorJson = JSON.stringify(environment);
  return {
    schema: "lattice.wsl2-provider-subtree-reconcile-request/1.0",
    descriptor_json: descriptorJson,
    descriptor_digest: createHash("sha256").update(descriptorJson).digest("hex"),
    source_preflight: {
      descriptor_digest: hex("4"),
      content_digest: createHash("sha256").update(receiptJson).digest("hex"),
      receipt_json: receiptJson,
    },
    open_marker: openMarker,
    packet_digest: typed("attempt-packet", "d"),
    provider_effect_count_before: 0,
    provider_effect_count_after: 0,
  };
}

function reconciliationDependencies() {
  return {
    validateEnvironment: (value) => value,
    buildLaunch: (environment, options) => {
      const serviceUnit = `lattice-wsl2-${options.preflightReceipt.task_ref.slice(0, 16)}-provider-${options.fence.slice(0, 12)}.service`;
      return {
        processFence: options.fence,
        serviceUnit,
        postExitProbe: {
          distribution: environment.distribution,
          unit: serviceUnit,
          process_fence: options.fence,
          authority_ref: environment.process_fence.identity_digest,
          systemctl_path: environment.process_fence.systemctl_path,
          cgroup_mount: environment.process_fence.cgroup_mount,
        },
      };
    },
    runProbe: async ({ expected }) => ({
      cleanup: {
        schema: "lattice.wsl2-provider-subtree-cleanup/1.0",
        actions: [],
      },
      outer_post_exit: outerPostExit({
        ...marker(),
        unit: expected.unit,
        fence: expected.fence,
        cgroup_path: expected.cgroup_path,
        boot_id_digest: expected.boot_id_digest,
      }),
    }),
  };
}

function openForReconciliation(input) {
  const receipt = JSON.parse(input.source_preflight.receipt_json);
  return buildWsl2ProviderSubtreeMarker({
    task_ref: receipt.task_ref,
    attempt: receipt.attempt,
    packet_digest: typed("attempt-packet", "d"),
    worktree_ref: receipt.worktree_ref,
    repository_head: receipt.repository_head,
    execution_environment_ref: receipt.execution_environment_ref,
    descriptor_digest: input.descriptor_digest,
    source_preflight_descriptor_digest: input.source_preflight.descriptor_digest,
    source_preflight_content_digest: input.source_preflight.content_digest,
    source_preflight_receipt_digest: receipt.receipt_digest,
  }, marker());
}

test("hard-loss reconciliation is preflight-anchored and never mints a normal CLOSED receipt", async () => {
  const absent = await reconcileWsl2ProviderSubtree(
    reconciliationInput(), reconciliationDependencies(),
  );
  assert.equal(absent.schema, "lattice.wsl2-provider-subtree-reconciliation/1.0");
  assert.equal(absent.status, "RECONCILED");
  assert.equal(absent.marker_observation, "ABSENT_AFTER_TRANSPORT_LOSS");
  assert.equal(absent.process_marker, null);
  assert.equal(absent.packet_digest, typed("attempt-packet", "d"));
  assert.equal(absent.provider_effect_count_before, absent.provider_effect_count_after);
  assert.notEqual(absent.schema, "lattice.wsl2-provider-subtree-receipt/1.0");

  const presentInput = reconciliationInput();
  const open = openForReconciliation(presentInput);
  const present = await reconcileWsl2ProviderSubtree(
    { ...presentInput, open_marker: open }, reconciliationDependencies(),
  );
  assert.equal(present.marker_observation, "PRESENT");
  assert.equal(present.source_marker_digest, open.marker_digest);
  assert.deepEqual(present.process_marker, open.process_marker);

  const drift = reconciliationInput(open);
  drift.provider_effect_count_after = 1;
  await assert.rejects(
    reconcileWsl2ProviderSubtree(drift, reconciliationDependencies()),
    { code: "WSL2_PROVIDER_SUBTREE_RECONCILIATION_REJECTED" },
  );

  const staleBoot = reconciliationDependencies();
  staleBoot.runProbe = async ({ expected }) => ({
    cleanup: { schema: "lattice.wsl2-provider-subtree-cleanup/1.0", actions: [] },
    outer_post_exit: { ...outerPostExit(marker()), boot_id_digest: typed("wsl-boot", "0"),
      unit: expected.unit, fence: expected.fence, cgroup_path: expected.cgroup_path },
  });
  await assert.rejects(
    reconcileWsl2ProviderSubtree(reconciliationInput(open), staleBoot),
    { code: "WSL2_PROVIDER_SUBTREE_RECONCILIATION_REJECTED" },
  );
});

function reviewerReconciliationInput(openMarker = null) {
  const input = reconciliationInput(openMarker);
  let receipt = JSON.parse(input.source_preflight.receipt_json);
  const descriptor = JSON.parse(input.descriptor_json);
  const reviewerContext = {
    task_ref: receipt.task_ref,
    attempt: receipt.attempt,
    subject_digest: hex("7"),
    model_call_identity: `managed-review-${receipt.task_ref}-${receipt.attempt}`,
    worktree_ref: receipt.worktree_ref,
    repository_head: receipt.repository_head,
    execution_environment_ref: receipt.execution_environment_ref,
    packet_digest: input.packet_digest,
  };
  const fence = createHash("sha256").update(canonicalJson({
    schema: "lattice.managed-review-process-fence/1.0",
    task_ref: reviewerContext.task_ref,
    attempt: reviewerContext.attempt,
    subject_digest: reviewerContext.subject_digest,
    model_call_identity: reviewerContext.model_call_identity,
    worktree_ref: reviewerContext.worktree_ref,
    repository_head: reviewerContext.repository_head,
    execution_environment_ref: reviewerContext.execution_environment_ref,
    process_fence_authority_ref: descriptor.process_fence.identity_digest,
    continuation: receipt.continuation,
  })).digest("hex");
  receipt = { ...receipt, process_fence: { ...receipt.process_fence, fence } };
  const receiptJson = JSON.stringify(receipt);
  return {
    ...input,
    schema: "lattice.wsl2-reviewer-subtree-reconcile-request/1.0",
    source_preflight: {
      ...input.source_preflight,
      content_digest: createHash("sha256").update(receiptJson).digest("hex"),
      receipt_json: receiptJson,
    },
    reviewer_context: reviewerContext,
  };
}

function reviewerOpenForReconciliation(input) {
  const receipt = JSON.parse(input.source_preflight.receipt_json);
  const processMarker = marker();
  processMarker.fence = receipt.process_fence.fence;
  processMarker.unit = `lattice-wsl2-${receipt.task_ref.slice(0, 16)}-provider-${processMarker.fence.slice(0, 12)}.service`;
  processMarker.cgroup_path = `/user.slice/user-1000.slice/user@1000.service/app.slice/${processMarker.unit}`;
  return buildWsl2ReviewerSubtreeMarker({
    task_ref: receipt.task_ref,
    attempt: receipt.attempt,
    packet_digest: input.packet_digest,
    worktree_ref: receipt.worktree_ref,
    repository_head: receipt.repository_head,
    execution_environment_ref: receipt.execution_environment_ref,
    descriptor_digest: input.descriptor_digest,
    source_preflight_descriptor_digest: input.source_preflight.descriptor_digest,
    source_preflight_content_digest: input.source_preflight.content_digest,
    source_preflight_receipt_digest: receipt.receipt_digest,
  }, processMarker, input.reviewer_context.subject_digest,
  input.reviewer_context.model_call_identity);
}

test("reviewer hard-loss reconciliation is role, subject, and model-call bound", async () => {
  const absentInput = reviewerReconciliationInput();
  const absent = await reconcileWsl2ReviewerSubtree(
    absentInput, reconciliationDependencies(),
  );
  assert.equal(absent.status, "RECONCILED");
  assert.equal(absent.role, "REVIEWER");
  assert.equal(absent.subject_digest, absentInput.reviewer_context.subject_digest);
  assert.equal(absent.model_call_identity, absentInput.reviewer_context.model_call_identity);
  assert.equal(absent.marker_observation, "ABSENT_AFTER_TRANSPORT_LOSS");
  assert.equal(absent.process_marker, null);

  const presentInput = reviewerReconciliationInput();
  const open = reviewerOpenForReconciliation(presentInput);
  const present = await reconcileWsl2ReviewerSubtree(
    { ...presentInput, open_marker: open }, reconciliationDependencies(),
  );
  assert.equal(present.marker_observation, "PRESENT");
  assert.equal(present.source_marker_digest, open.marker_digest);
  assert.equal(present.provider_subtree_segment_ref, open.provider_subtree_segment_ref);

  for (const mutation of [
    (value) => { value.reviewer_context.subject_digest = hex("8"); },
    (value) => { value.reviewer_context.model_call_identity += "-substituted"; },
    (value) => { value.packet_digest = typed("attempt-packet", "e"); },
    (value) => { value.provider_effect_count_after = 1; },
  ]) {
    const substituted = reviewerReconciliationInput(open);
    mutation(substituted);
    await assert.rejects(
      reconcileWsl2ReviewerSubtree(substituted, reconciliationDependencies()),
      { code: "WSL2_PROVIDER_SUBTREE_RECONCILIATION_REJECTED" },
    );
  }
});
