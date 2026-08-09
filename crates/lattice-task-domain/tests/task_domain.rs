use std::collections::BTreeMap;

use lattice_cjson::{HashDomain, canonical_sha256};
use lattice_contracts::{ProjectSnapshotId, TaskId};
use lattice_task_domain::{
    AcceptanceCriterion, ApprovalRequirement, ApprovalRequirements, Capability, CapabilityRequest,
    DeploymentPolicy, EvidenceType, NetworkPolicy, RequiredCheck, RiskClass, RuntimeProfile,
    ScopeOperation, TASK_SPEC_SCHEMA_VERSION, TaskBudget, TaskScope, TaskSpec, TaskSpecInput,
    TaskState, is_transition_allowed, transition, v1_compat, validate_task_graph,
};

fn task_id(value: &str) -> TaskId {
    TaskId::new(value).expect("non-empty task id")
}

fn snapshot_id(value: &str) -> ProjectSnapshotId {
    ProjectSnapshotId::new(value).expect("non-empty snapshot id")
}

fn base_input() -> TaskSpecInput {
    TaskSpecInput {
        schema_version: "2.1".to_owned(),
        task_id: task_id("TASK-2026-010"),
        revision: "1".to_owned(),
        created_at: "2026-07-29T00:00:00.1200Z".to_owned(),
        created_by: "owner".to_owned(),
        project_id: "lattice-devos".to_owned(),
        project_snapshot_id: snapshot_id("snapshot-010"),
        base_ref: "main".to_owned(),
        base_commit_id: "A".repeat(40),
        goal: "Build Cafe\u{301} safely.".to_owned(),
        non_goals: vec!["Do not deploy.".to_owned()],
        risk_class: RiskClass::R1,
        depends_on: vec![],
        scope: TaskScope {
            allowed_paths: vec!["test/**".to_owned(), "src/**".to_owned()],
            forbidden_paths: vec![".git/**".to_owned()],
            allowed_operations: vec![ScopeOperation::Modify, ScopeOperation::Create],
        },
        acceptance_criteria: vec![AcceptanceCriterion {
            id: "AC-01".to_owned(),
            description: "Focused tests pass.".to_owned(),
            evidence_type: EvidenceType::Test,
            expected_result: "Command exits with code 0.".to_owned(),
        }],
        verification_commands: vec!["cargo test -p lattice-task-domain".to_owned()],
        required_checks: vec![RequiredCheck::Scope, RequiredCheck::Test],
        requested_capabilities: vec![
            CapabilityRequest {
                capability: Capability::RunTests,
                contract_version: "1".to_owned(),
            },
            CapabilityRequest {
                capability: Capability::WriteProductCode,
                contract_version: "1".to_owned(),
            },
        ],
        budget: TaskBudget {
            accounting_currency: "TWD".to_owned(),
            max_agents: "4".to_owned(),
            max_duration_seconds: "1800".to_owned(),
            max_attempts: "2".to_owned(),
            max_model_calls: "0".to_owned(),
            max_external_cost: "0".to_owned(),
        },
        runtime_profile: RuntimeProfile::Fake,
        network_policy: NetworkPolicy::Deny,
        deployment_policy: DeploymentPolicy::Deny,
        approval_requirements: ApprovalRequirements {
            execution: ApprovalRequirement::Policy,
            merge: ApprovalRequirement::ResponsibleUser,
            protected_release: ApprovalRequirement::ProtectedGuardian,
        },
    }
}

fn hash(input: TaskSpecInput) -> String {
    TaskSpec::new(input)
        .expect("valid task spec")
        .spec_hash()
        .to_hex()
}

#[test]
fn valid_v21_spec_normalizes_owned_fields_and_hashes_deterministically() {
    let first = TaskSpec::new(base_input()).expect("valid task spec");
    let mut reordered = base_input();
    reordered.scope.allowed_paths.reverse();
    reordered.scope.allowed_operations.reverse();
    reordered.required_checks.reverse();
    reordered.requested_capabilities.reverse();
    let second = TaskSpec::new(reordered).expect("valid reordered task spec");

    assert_eq!(first.created_at(), "2026-07-29T00:00:00.12Z");
    assert_eq!(
        first.base_commit_id(),
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    );
    assert_eq!(first.goal(), "Build Café safely.");
    assert_eq!(first.fields().budget.accounting_currency, "TWD");
    assert_eq!(TASK_SPEC_SCHEMA_VERSION, "2.1");
    assert_eq!(first.spec_hash(), second.spec_hash());
    assert_eq!(first.spec_hash().to_hex().len(), 64);
    assert_ne!(
        first.spec_hash().to_hex(),
        "88e9f8502132b7216bb0d4a1080c32429a1e982e6a80d572654ba1dd5a21da51",
        "the V1 characterization hash must not share the V2 path"
    );
    assert_eq!(
        first
            .fields()
            .required_checks
            .iter()
            .map(|value| value.as_str())
            .collect::<Vec<_>>(),
        ["scope", "test"]
    );
    assert_eq!(
        first
            .fields()
            .requested_capabilities
            .iter()
            .map(|value| value.capability.as_str())
            .collect::<Vec<_>>(),
        ["RUN_TESTS", "WRITE_PRODUCT_CODE"]
    );
}

#[test]
fn canonical_document_is_byte_stable_and_bound_to_the_spec_hash() {
    let spec = TaskSpec::new(base_input()).expect("valid task spec");
    let document = spec
        .canonical_document()
        .expect("domain-owned canonical document");
    let domain = HashDomain::new("lattice.task-spec", TASK_SPEC_SCHEMA_VERSION)
        .expect("task-spec hash domain");
    let subject = spec.canonical_subject();
    let recomputed = canonical_sha256(&domain, &subject).expect("document digest");

    assert_eq!(document, spec.canonical_document().expect("stable bytes"));
    assert_eq!(
        document,
        lattice_cjson::canonicalize(&subject)
            .expect("canonical bytes")
            .into_vec()
    );
    assert_eq!(recomputed.to_hex(), spec.spec_hash().to_hex());
}

#[test]
fn task_spec_rejects_unknown_versions_bad_scalars_and_unsafe_scope() {
    for version in ["2.0", "3.0"] {
        let mut unsupported_version = base_input();
        unsupported_version.schema_version = version.to_owned();
        assert_eq!(
            TaskSpec::new(unsupported_version)
                .expect_err("unsupported schema must fail closed")
                .code(),
            "UNSUPPORTED_TASK_SPEC_VERSION",
            "{version}"
        );
    }

    for currency in ["twd", "Twd", "TW", "TWDD", "T1D", "ＴＷＤ"] {
        let mut invalid_currency = base_input();
        invalid_currency.budget.accounting_currency = currency.to_owned();
        assert_eq!(
            TaskSpec::new(invalid_currency)
                .expect_err("currency must be canonical uppercase ASCII")
                .code(),
            "INVALID_TASK_SPEC",
            "{currency:?}"
        );
    }

    let mut leading_zero = base_input();
    leading_zero.revision = "01".to_owned();
    assert_eq!(
        TaskSpec::new(leading_zero)
            .expect_err("non-canonical integer")
            .code(),
        "INVALID_CANONICAL_INTEGER"
    );

    let mut non_utc = base_input();
    non_utc.created_at = "2026-07-29T08:00:00+08:00".to_owned();
    assert_eq!(
        TaskSpec::new(non_utc)
            .expect_err("non-UTC timestamp")
            .code(),
        "INVALID_UTC_TIMESTAMP"
    );

    for timestamp in [
        "2026-07-29x00:00:00Z",
        concat!("2026-07-29", "\0", "00:00:00Z"),
        "2026-07-29T00:00:00-00:00",
        "2026-07-29T00:00:00+00:00",
        "2026-07-29t00:00:00z",
        "2026-07-29T00:00:00,1Z",
    ] {
        let mut malformed_timestamp = base_input();
        malformed_timestamp.created_at = timestamp.to_owned();
        assert_eq!(
            TaskSpec::new(malformed_timestamp)
                .expect_err("malformed timestamp")
                .code(),
            "INVALID_UTC_TIMESTAMP",
            "{timestamp:?}"
        );
    }

    let mut decimal_zero = base_input();
    decimal_zero.budget.max_external_cost = "1.20".to_owned();
    assert_eq!(
        TaskSpec::new(decimal_zero)
            .expect_err("non-canonical decimal")
            .code(),
        "INVALID_CANONICAL_DECIMAL"
    );

    let mut maximum_decimal = base_input();
    maximum_decimal.budget.max_external_cost = format!("{}.{}", "9".repeat(127), "9".repeat(128));
    TaskSpec::new(maximum_decimal).expect("maximum precision and scale are in contract");

    let mut oversized_integer = base_input();
    oversized_integer.budget.max_external_cost = "9".repeat(128);
    assert_eq!(
        TaskSpec::new(oversized_integer)
            .expect_err("128 integer digits exceed shared precision bound")
            .code(),
        "INVALID_CANONICAL_DECIMAL"
    );

    let mut oversized_scale = base_input();
    oversized_scale.budget.max_external_cost = format!("0.{}", "9".repeat(129));
    assert_eq!(
        TaskSpec::new(oversized_scale)
            .expect_err("129 fractional digits exceed shared scale bound")
            .code(),
        "INVALID_CANONICAL_DECIMAL"
    );

    for path in [
        "../escape",
        ".git/config",
        ".git./hooks/**",
        ".git /config",
        "C:/outside",
        "src/file:stream",
        "src/name./file",
        "src/name /file",
        " src/leading",
        "src/trailing ",
        "src/control\u{1f}",
        "src/delete\u{7f}",
        "NUL",
        "src/NUL.txt",
        "src/con.rs",
        "src/PRN",
        "src/AUX.log",
        "src/COM1.txt",
        "src/Lpt9",
        "src/COM¹.txt",
        "src/CONIN$",
        r"src\outside",
    ] {
        let mut unsafe_path = base_input();
        unsafe_path.scope.allowed_paths = vec![path.to_owned()];
        assert_eq!(
            TaskSpec::new(unsafe_path)
                .expect_err("unsafe scope path")
                .code(),
            "INVALID_SCOPE_PATH",
            "{path}"
        );
    }

    for git_ref in [
        "main.lock",
        "main.LOCK",
        "feature/x.LOCK",
        "feature/.hidden",
        "feature/dangling.",
        "feature//double",
        "main\u{1f}control",
        "main\u{7f}delete",
        "@",
    ] {
        let mut unsafe_ref = base_input();
        unsafe_ref.base_ref = git_ref.to_owned();
        assert_eq!(
            TaskSpec::new(unsafe_ref)
                .expect_err("invalid Git ref")
                .code(),
            "INVALID_TASK_SPEC",
            "{git_ref:?}"
        );
    }

    let mut duplicate_nfc = base_input();
    duplicate_nfc.non_goals = vec!["Café".to_owned(), "Cafe\u{301}".to_owned()];
    assert_eq!(
        TaskSpec::new(duplicate_nfc)
            .expect_err("NFC-equivalent duplicate")
            .code(),
        "DUPLICATE_TASK_FIELD_VALUE"
    );

    assert_eq!(
        Capability::parse("UNKNOWN")
            .expect_err("unknown capability")
            .code(),
        "INVALID_TASK_SPEC"
    );

    let mut unknown_capability_version = base_input();
    unknown_capability_version.requested_capabilities[0].contract_version = "2".to_owned();
    assert_eq!(
        TaskSpec::new(unknown_capability_version)
            .expect_err("unknown capability contract")
            .code(),
        "INVALID_TASK_SPEC"
    );
}

#[test]
fn task_spec_rejects_leap_second_normalization_collision() {
    let mut ordinary = base_input();
    ordinary.created_at = "2016-12-31T23:59:59.999999999Z".to_owned();
    TaskSpec::new(ordinary).expect("representable final nanosecond");

    let mut leap_second = base_input();
    leap_second.created_at = "2016-12-31T23:59:60Z".to_owned();
    assert_eq!(
        TaskSpec::new(leap_second)
            .expect_err("leap seconds must not normalize onto another timestamp")
            .code(),
        "INVALID_UTC_TIMESTAMP"
    );
}

#[test]
fn every_immutable_field_family_changes_the_spec_hash() {
    let base_hash = hash(base_input());
    let mut revisions = Vec::new();

    let mut value = base_input();
    value.task_id = task_id("TASK-2026-011");
    revisions.push(("task_id", value));

    let mut value = base_input();
    value.revision = "2".to_owned();
    revisions.push(("revision", value));

    let mut value = base_input();
    value.created_at = "2026-07-29T00:00:01Z".to_owned();
    revisions.push(("created_at", value));

    let mut value = base_input();
    value.created_by = "guardian".to_owned();
    revisions.push(("created_by", value));

    let mut value = base_input();
    value.project_id = "another-project".to_owned();
    revisions.push(("project_id", value));

    let mut value = base_input();
    value.project_snapshot_id = snapshot_id("snapshot-011");
    revisions.push(("project_snapshot_id", value));

    let mut value = base_input();
    value.base_ref = "feature/task".to_owned();
    revisions.push(("base_ref", value));

    let mut value = base_input();
    value.base_commit_id = "b".repeat(40);
    revisions.push(("base_commit_id", value));

    let mut value = base_input();
    value.goal = "Build another slice.".to_owned();
    revisions.push(("goal", value));

    let mut value = base_input();
    value.non_goals.push("Do not publish.".to_owned());
    revisions.push(("non_goals", value));

    let mut value = base_input();
    value.risk_class = RiskClass::R2;
    revisions.push(("risk_class", value));

    let mut value = base_input();
    value.depends_on.push(task_id("TASK-2026-009"));
    revisions.push(("depends_on", value));

    let mut value = base_input();
    value.scope.allowed_paths.push("docs/**".to_owned());
    revisions.push(("scope.allowed_paths", value));

    let mut value = base_input();
    value.scope.forbidden_paths.push("secrets/**".to_owned());
    revisions.push(("scope.forbidden_paths", value));

    let mut value = base_input();
    value.scope.allowed_operations.push(ScopeOperation::Delete);
    revisions.push(("scope.allowed_operations", value));

    let mut value = base_input();
    value.acceptance_criteria[0].id = "AC-02".to_owned();
    revisions.push(("acceptance_criteria.id", value));

    let mut value = base_input();
    value.acceptance_criteria[0].description = "Full suite passes.".to_owned();
    revisions.push(("acceptance_criteria.description", value));

    let mut value = base_input();
    value.acceptance_criteria[0].evidence_type = EvidenceType::Artifact;
    revisions.push(("acceptance_criteria.evidence_type", value));

    let mut value = base_input();
    value.acceptance_criteria[0].expected_result = "Artifact is present.".to_owned();
    revisions.push(("acceptance_criteria.expected_result", value));

    let mut value = base_input();
    value
        .verification_commands
        .push("cargo test --workspace".to_owned());
    revisions.push(("verification_commands", value));

    let mut value = base_input();
    value.required_checks.push(RequiredCheck::Architecture);
    revisions.push(("required_checks", value));

    let mut value = base_input();
    value.requested_capabilities.push(CapabilityRequest {
        capability: Capability::ReadRepository,
        contract_version: "1".to_owned(),
    });
    revisions.push(("requested_capabilities.id", value));

    let mut value = base_input();
    value.budget.max_agents = "3".to_owned();
    revisions.push(("budget.max_agents", value));

    let mut value = base_input();
    value.budget.max_duration_seconds = "1801".to_owned();
    revisions.push(("budget.max_duration_seconds", value));

    let mut value = base_input();
    value.budget.max_attempts = "3".to_owned();
    revisions.push(("budget.max_attempts", value));

    let mut value = base_input();
    value.budget.max_model_calls = "1".to_owned();
    revisions.push(("budget.max_model_calls", value));

    let mut value = base_input();
    value.budget.max_external_cost = "0.01".to_owned();
    revisions.push(("budget.max_external_cost", value));

    let mut value = base_input();
    value.budget.accounting_currency = "USD".to_owned();
    revisions.push(("budget.accounting_currency", value));

    let mut value = base_input();
    value.runtime_profile = RuntimeProfile::Codex;
    revisions.push(("runtime_profile", value));

    let mut value = base_input();
    value.network_policy = NetworkPolicy::LoopbackOnly;
    revisions.push(("network_policy", value));

    let mut value = base_input();
    value.deployment_policy = DeploymentPolicy::PrepareOnly;
    revisions.push(("deployment_policy", value));

    let mut value = base_input();
    value.approval_requirements.execution = ApprovalRequirement::NotRequired;
    revisions.push(("approval_requirements.execution", value));

    let mut value = base_input();
    value.approval_requirements.merge = ApprovalRequirement::Policy;
    revisions.push(("approval_requirements.merge", value));

    let mut value = base_input();
    value.approval_requirements.protected_release = ApprovalRequirement::ResponsibleUser;
    revisions.push(("approval_requirements.protected_release", value));

    for (field, revised) in revisions {
        assert_ne!(base_hash, hash(revised), "{field} must bind spec_hash");
    }
}

fn expected_targets(state: TaskState) -> &'static [TaskState] {
    use TaskState::{
        AwaitingExecutionApproval, AwaitingMergeApproval, Blocked, Cancelled, Completed, Draft,
        Executing, Failed, Merging, Preparing, Rejected, Reviewing, Stopping, Verifying,
    };

    match state {
        Draft => &[AwaitingExecutionApproval, Cancelled],
        AwaitingExecutionApproval => &[Preparing, Rejected, Cancelled],
        Preparing => &[Executing, Blocked, Failed, Stopping],
        Executing => &[Verifying, Blocked, Failed, Stopping],
        Verifying => &[Reviewing, Blocked, Failed, Stopping],
        Reviewing => &[AwaitingMergeApproval, Blocked, Failed, Stopping],
        AwaitingMergeApproval => &[Merging, Rejected, Cancelled],
        Merging => &[Completed, Blocked, Failed, Stopping],
        Stopping => &[Cancelled, Failed],
        Completed | Rejected | Blocked | Failed | Cancelled => &[],
    }
}

#[test]
fn freezes_the_complete_v1_transition_matrix_and_stable_errors() {
    for from in TaskState::ALL {
        for to in TaskState::ALL {
            let expected = expected_targets(from).contains(&to);
            assert_eq!(is_transition_allowed(from, to), expected);
            assert_eq!(
                v1_compat::is_transition_allowed(from.as_str(), to.as_str()),
                expected
            );
        }
    }

    assert_eq!(
        transition(TaskState::Completed, TaskState::Executing)
            .expect_err("illegal transition")
            .code(),
        "INVALID_STATE_TRANSITION"
    );
    assert_eq!(
        v1_compat::transition("UNKNOWN", "DRAFT")
            .expect_err("unknown state")
            .code(),
        "UNKNOWN_TASK_STATE"
    );

    let next = transition(TaskState::Draft, TaskState::AwaitingExecutionApproval)
        .expect("legal transition");
    assert_eq!(next, TaskState::AwaitingExecutionApproval);
}

#[test]
fn task_graph_rejects_unknown_dependencies_and_returns_a_stable_cycle() {
    let acyclic = BTreeMap::from([
        ("TASK-2026-001".to_owned(), vec![]),
        ("TASK-2026-002".to_owned(), vec!["TASK-2026-001".to_owned()]),
        ("TASK-2026-003".to_owned(), vec!["TASK-2026-002".to_owned()]),
    ]);
    validate_task_graph(&acyclic).expect("acyclic graph");

    let unknown = BTreeMap::from([("TASK-2026-001".to_owned(), vec!["TASK-2026-999".to_owned()])]);
    assert_eq!(
        validate_task_graph(&unknown)
            .expect_err("unknown dependency")
            .code(),
        "UNKNOWN_TASK_DEPENDENCY"
    );

    let cycle = BTreeMap::from([
        ("TASK-2026-001".to_owned(), vec!["TASK-2026-003".to_owned()]),
        ("TASK-2026-002".to_owned(), vec!["TASK-2026-001".to_owned()]),
        ("TASK-2026-003".to_owned(), vec!["TASK-2026-002".to_owned()]),
    ]);
    let error = validate_task_graph(&cycle).expect_err("cycle");
    assert_eq!(error.code(), "TASK_DEPENDENCY_CYCLE");
    let cycle_path = error.cycle().expect("cycle evidence");
    assert_eq!(cycle_path.first(), cycle_path.last());
    assert_eq!(
        cycle_path,
        [
            "TASK-2026-001",
            "TASK-2026-003",
            "TASK-2026-002",
            "TASK-2026-001"
        ]
    );

    let self_cycle = BTreeMap::from([(
        "TASK-2026-SELF".to_owned(),
        vec!["TASK-2026-SELF".to_owned()],
    )]);
    let error = validate_task_graph(&self_cycle).expect_err("self-cycle");
    assert_eq!(error.code(), "TASK_DEPENDENCY_CYCLE");
    assert_eq!(
        error.cycle().expect("self-cycle evidence"),
        ["TASK-2026-SELF", "TASK-2026-SELF"]
    );
}
