import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const testDirectory = path.dirname(fileURLToPath(import.meta.url));
const fixturePath = path.join(
  testDirectory,
  "fixtures",
  "issue-008-model-routing.characterization.json",
);

test("Issue #8 routing characterization is provider-neutral and fail-closed", async () => {
  const fixture = JSON.parse(await readFile(fixturePath, "utf8"));

  assert.deepEqual(fixture.identity, {
    github_issue: 8,
    canonical_id: "model-routing",
    internal_task_041_reused: false,
  });

  assert.deepEqual(
    fixture.governance_profiles.map(({ name, rank }) => [name, rank]),
    [
      ["守望", 0],
      ["快速", 1],
      ["標準", 2],
      ["深度", 3],
      ["關鍵", 4],
    ],
  );

  const serializedProfiles = JSON.stringify(fixture.governance_profiles);
  for (const providerOrModelName of [
    "codex",
    "gpt-",
    "hermes",
    "ollama",
    "openai",
  ]) {
    assert.equal(
      serializedProfiles.toLowerCase().includes(providerOrModelName),
      false,
      `governance profile leaked provider/model identity: ${providerOrModelName}`,
    );
  }

  const evidenceStages = Object.keys(fixture.evidence_shape);
  assert.deepEqual(evidenceStages, ["requested", "resolved", "actual"]);
  for (const stage of evidenceStages) {
    assert.ok(fixture.evidence_shape[stage].required_fields.length > 0);
  }
  assert.ok(fixture.evidence_shape.actual.required_fields.includes("evidence_state"));
  assert.ok(fixture.evidence_shape.actual.required_fields.includes("source_ref"));

  const codexSurface = fixture.evidence_inventory.find(
    ({ surface }) => surface === "Codex App and app-server",
  );
  assert.deepEqual(codexSurface.catalog_snapshot, {
    "gpt-5.6-sol": ["low", "medium", "high", "xhigh", "max", "ultra"],
    "gpt-5.6-terra": ["low", "medium", "high", "xhigh", "max", "ultra"],
    "gpt-5.6-luna": ["low", "medium", "high", "xhigh", "max"],
    "gpt-5.5": ["low", "medium", "high", "xhigh"],
    "gpt-5.4": ["low", "medium", "high", "xhigh"],
    "gpt-5.4-mini": ["low", "medium", "high", "xhigh"],
    "gpt-5.3-codex-spark": ["low", "medium", "high", "xhigh"],
  });

  const reasonCodes = fixture.reason_codes.flatMap((group) => group.codes);
  assert.equal(new Set(reasonCodes).size, reasonCodes.length);
  assert.ok(reasonCodes.includes("ROUTE_CAPABILITY_EVIDENCE_MISSING"));
  assert.ok(reasonCodes.includes("ROUTE_ACTUAL_SETTING_MISMATCH"));
  assert.ok(reasonCodes.includes("ROUTE_REPRODUCIBILITY_PIN"));

  assert.equal(fixture.transition_policy.deterministic_rules_first, true);
  assert.equal(fixture.transition_policy.model_may_self_authorize, false);
  assert.equal(fixture.transition_policy.missing_capability_evidence, "BLOCK");
  assert.equal(fixture.transition_policy.critical_profile_downgrade, "DENY");
  assert.equal(fixture.transition_policy.in_flight_deescalation, "DENY");
  assert.ok(fixture.transition_policy.max_automatic_escalations >= 0);

  const allowedClassifications = new Set([
    "acceptance_reproducibility_pin",
    "temporary_implementation_constraint",
    "general_route_candidate",
  ]);
  assert.ok(fixture.fixed_setting_inventory.length > 0);
  for (const setting of fixture.fixed_setting_inventory) {
    assert.ok(allowedClassifications.has(setting.classification));
    assert.match(setting.source_ref, /^(repo|official|runtime):/);
  }
  const fixedSettingRefs = new Set(
    fixture.fixed_setting_inventory.map(({ source_ref }) => source_ref),
  );
  for (const requiredRef of [
    "repo:crates/lattice-codex-adapter/tests/process.rs:212",
    "repo:crates/lattice-hermes-adapter/tests/reflection_api.rs:183",
    "repo:crates/lattice-hermes-adapter/tests/hermes_sandbox_runner_fixture.py:399",
    "repo:worktree:feature/task-038-chatgpt-mcp:scripts/run-task038-task-submit.ps1:1137",
    "repo:apps/lattice-runtime/src/composition.rs:98",
    "repo:apps/lattice-runtime/src/composition.rs:100",
  ]) {
    assert.ok(fixedSettingRefs.has(requiredRef), `missing fixed-setting evidence: ${requiredRef}`);
  }
  const hermesCompatibilityIdentity = fixture.fixed_setting_inventory.find(
    ({ source_ref }) => source_ref === "repo:apps/lattice-runtime/src/composition.rs:98",
  );
  assert.equal(
    hermesCompatibilityIdentity.classification,
    "acceptance_reproducibility_pin",
  );
  assert.match(hermesCompatibilityIdentity.setting, /not an underlying LLM route/);
  assert.equal(
    fixture.fixed_setting_inventory.find(
      ({ source_ref }) => source_ref === "repo:apps/lattice-runtime/src/composition.rs:100",
    ).classification,
    "temporary_implementation_constraint",
  );

  const forbiddenKeys = new Set([
    "api_key",
    "credential",
    "hidden_reasoning",
    "private_prompt",
    "secret",
  ]);
  const visit = (value) => {
    if (Array.isArray(value)) {
      value.forEach(visit);
      return;
    }
    if (value && typeof value === "object") {
      for (const [key, child] of Object.entries(value)) {
        assert.equal(forbiddenKeys.has(key), false, `forbidden evidence key: ${key}`);
        visit(child);
      }
    }
  };
  visit(fixture);
});
