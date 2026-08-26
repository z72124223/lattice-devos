use lattice_contracts::{
    ContentDigest, ProjectId, ProjectSnapshotId, RuntimeKind, TaskId, TaskLedgerStreamIdentity,
};
use lattice_task_ledger::{
    LedgerError, TASK_LEDGER_PROJECT_SNAPSHOT_ID_MAX_BYTES, TaskIngressClaim,
    TaskIngressRequestKind, TaskSubmissionEnvelope, VerifiedStream,
    verify_untrusted_task_ingress_claim, verify_untrusted_task_ingress_claim_structure,
};

fn digest(byte: char) -> ContentDigest {
    ContentDigest::from_sha256(byte.to_string().repeat(64)).expect("digest")
}

fn task_spec_identity(project_id: &str, snapshot: &str, task_id: &str) -> TaskLedgerStreamIdentity {
    TaskLedgerStreamIdentity::new(
        ProjectId::new(project_id).expect("project"),
        ProjectSnapshotId::new(snapshot).expect("snapshot"),
        TaskId::new(task_id).expect("task"),
        "1",
        digest('a'),
        "TWD",
    )
    .expect("identity")
}

fn intake_identity(project_id: &str, snapshot: &str, task_id: &str) -> TaskLedgerStreamIdentity {
    TaskLedgerStreamIdentity::new_general_task_intake(
        ProjectId::new(project_id).expect("project"),
        ProjectSnapshotId::new(snapshot).expect("snapshot"),
        TaskId::new(task_id).expect("task"),
        "1",
        digest('a'),
    )
    .expect("intake identity")
}

fn submission(
    objective: &str,
    project_id: &str,
    snapshot: &str,
    task_id: &str,
) -> TaskSubmissionEnvelope {
    TaskSubmissionEnvelope::new(
        "lattice_task_submit.v1",
        "request-claim-1",
        objective,
        "AI 劇本",
        intake_identity(project_id, snapshot, task_id),
        digest('b'),
    )
    .expect("submission")
}

#[test]
fn canary_and_general_claims_share_a_key_but_never_a_semantic_kind() {
    let general = submission(
        "完成角色系統",
        "ai-novel",
        "ai-novel:s1",
        "TASK-GENERAL-001",
    );
    let general_claim = TaskIngressClaim::general_submission(&general).expect("general claim");
    let canary_claim = TaskIngressClaim::controlled_canary(
        general.ingress_id(),
        general.client_request_id(),
        general.stream_id().clone(),
    )
    .expect("canary claim");

    assert_eq!(general_claim.ingress_id(), canary_claim.ingress_id());
    assert_eq!(
        general_claim.client_request_id(),
        canary_claim.client_request_id()
    );
    assert_eq!(
        general_claim.request_kind(),
        TaskIngressRequestKind::GeneralTask
    );
    assert_eq!(
        canary_claim.request_kind(),
        TaskIngressRequestKind::ControlledCodexCanary
    );
    assert_ne!(general_claim, canary_claim);
    assert_eq!(canary_claim.request_digest(), canary_claim.stream_id());
    assert!(!format!("{general_claim:?}").contains("request-claim-1"));
}

#[test]
fn general_claim_digest_binds_objective_and_formal_project() {
    let baseline = submission(
        "完成角色系統",
        "ai-novel",
        "ai-novel:s1",
        "TASK-GENERAL-001",
    );
    let exact = baseline.clone();
    let changed_objective = submission(
        "完成道具系統",
        "ai-novel",
        "ai-novel:s1",
        "TASK-GENERAL-001",
    );
    let changed_project = submission(
        "完成角色系統",
        "other-project",
        "other-project:s1",
        "TASK-GENERAL-002",
    );

    let baseline_claim = TaskIngressClaim::general_submission(&baseline).expect("baseline");
    assert_eq!(
        baseline_claim,
        TaskIngressClaim::general_submission(&exact).expect("exact")
    );
    assert_ne!(
        baseline_claim.request_digest(),
        TaskIngressClaim::general_submission(&changed_objective)
            .expect("objective")
            .request_digest()
    );
    assert_ne!(
        baseline_claim.request_digest(),
        TaskIngressClaim::general_submission(&changed_project)
            .expect("project")
            .request_digest()
    );
}

#[test]
fn retained_claim_tampering_fails_against_the_expected_pure_claim() {
    let claim = TaskIngressClaim::general_submission(&submission(
        "完成角色系統",
        "ai-novel",
        "ai-novel:s1",
        "TASK-GENERAL-001",
    ))
    .expect("claim");
    let mut raw = claim.to_untrusted();
    raw.request_digest = digest('f');
    assert_eq!(
        verify_untrusted_task_ingress_claim(&raw, &claim),
        Err(LedgerError::TaskIngressClaimMismatch)
    );

    let mut raw = claim.to_untrusted();
    raw.schema_version = "lattice.task-ledger.task-ingress-claim/2.0".to_owned();
    assert_eq!(
        verify_untrusted_task_ingress_claim(&raw, &claim),
        Err(LedgerError::UnknownTaskIngressClaimVersion)
    );
}

#[test]
fn neutral_claim_verification_is_structural_but_keeps_canary_digest_semantics_closed() {
    let general_claim = TaskIngressClaim::general_submission(&submission(
        "完成角色系統",
        "ai-novel",
        "ai-novel:s1",
        "TASK-GENERAL-STRUCTURAL",
    ))
    .expect("general claim");
    let mut opaque_general = general_claim.to_untrusted();
    opaque_general.request_digest = digest('f');
    assert_eq!(
        verify_untrusted_task_ingress_claim_structure(&opaque_general)
            .expect("general digest is opaque before envelope resolution")
            .request_kind(),
        TaskIngressRequestKind::GeneralTask
    );
    assert_eq!(
        verify_untrusted_task_ingress_claim(&opaque_general, &general_claim),
        Err(LedgerError::TaskIngressClaimMismatch)
    );

    let canary = TaskIngressClaim::controlled_canary(
        "lattice_task_submit.v1",
        "request-canary-structural",
        digest('d'),
    )
    .expect("canary");
    let mut substituted_canary = canary.to_untrusted();
    substituted_canary.request_digest = digest('e');
    assert_eq!(
        verify_untrusted_task_ingress_claim_structure(&substituted_canary),
        Err(LedgerError::TaskIngressClaimMismatch)
    );

    let mut unknown_kind = general_claim.to_untrusted();
    unknown_kind.request_kind = "FUTURE_TASK".to_owned();
    assert_eq!(
        verify_untrusted_task_ingress_claim_structure(&unknown_kind),
        Err(LedgerError::InvalidTaskIngressClaim {
            field: "request_kind"
        })
    );
}

#[test]
fn submission_unicode_character_and_utf8_byte_bounds_match_the_public_contract() {
    let exact_objective = "🧑".repeat(512);
    assert_eq!(exact_objective.chars().count(), 512);
    assert_eq!(exact_objective.len(), 2_048);
    TaskSubmissionEnvelope::new(
        "lattice_task_submit.v1",
        "request-unicode-1",
        exact_objective,
        "角".repeat(64),
        intake_identity("ai-novel", "ai-novel:s1", "TASK-GENERAL-003"),
        digest('c'),
    )
    .expect("exact unicode bounds");

    let too_many_objective_chars = TaskSubmissionEnvelope::new(
        "lattice_task_submit.v1",
        "request-unicode-2",
        "角".repeat(513),
        "AI 劇本",
        intake_identity("ai-novel", "ai-novel:s1", "TASK-GENERAL-004"),
        digest('c'),
    );
    assert_eq!(
        too_many_objective_chars,
        Err(LedgerError::SubmissionEnvelopeLimitExceeded { field: "objective" })
    );

    let too_many_project_chars = TaskSubmissionEnvelope::new(
        "lattice_task_submit.v1",
        "request-unicode-3",
        "完成角色系統",
        "角".repeat(65),
        intake_identity("ai-novel", "ai-novel:s1", "TASK-GENERAL-005"),
        digest('c'),
    );
    assert_eq!(
        too_many_project_chars,
        Err(LedgerError::SubmissionEnvelopeLimitExceeded {
            field: "project_display_name"
        })
    );

    let canary_stream = VerifiedStream::vacant(
        task_spec_identity("ai-novel", "ai-novel:s1", "TASK-CANARY-001"),
        RuntimeKind::Live,
    )
    .expect("canary stream");
    TaskIngressClaim::controlled_canary(
        "lattice_task_submit.v1",
        "request-unicode-4",
        canary_stream.head().stream_id().clone(),
    )
    .expect("canary claim");
}

#[test]
fn submission_accepts_the_largest_registry_snapshot_and_rejects_one_byte_more() {
    let project_id = "p".repeat(64);
    let snapshot = format!("{project_id}:registry:{}:{}", u64::MAX, "a".repeat(64));
    assert_eq!(snapshot.len(), TASK_LEDGER_PROJECT_SNAPSHOT_ID_MAX_BYTES);
    TaskSubmissionEnvelope::new(
        "lattice_task_submit.v1",
        "request-snapshot-boundary-1",
        "完成角色系統",
        "AI 劇本",
        intake_identity(&project_id, &snapshot, "TASK-GENERAL-SNAPSHOT-MAX"),
        digest('c'),
    )
    .expect("maximum Registry authority snapshot");

    let oversized = TaskSubmissionEnvelope::new(
        "lattice_task_submit.v1",
        "request-snapshot-boundary-2",
        "完成角色系統",
        "AI 劇本",
        intake_identity(
            "ai-novel",
            &"s".repeat(TASK_LEDGER_PROJECT_SNAPSHOT_ID_MAX_BYTES + 1),
            "TASK-GENERAL-SNAPSHOT-OVER",
        ),
        digest('c'),
    );
    assert_eq!(
        oversized,
        Err(LedgerError::SubmissionEnvelopeLimitExceeded {
            field: "project_snapshot_id"
        })
    );
}

#[test]
fn submission_rejects_sensitive_assignments_before_claim_or_envelope_creation() {
    for (index, objective) in [
        "完成設定 secret=hunter2",
        "credential: do-not-store",
        "Cookie = session-value",
        "refresh_token=do-not-store",
        "password\u{2003}=hunter2",
        "api_key\u{a0}:do-not-store",
        "private key----- marker before -----begin marker",
        "使用 AKIAIOSFODNN7EXAMPLE 完成設定",
    ]
    .into_iter()
    .enumerate()
    {
        let rejected = TaskSubmissionEnvelope::new(
            "lattice_task_submit.v1",
            format!("request-secret-{index}"),
            objective,
            "AI 劇本",
            intake_identity("ai-novel", "ai-novel:s1", &format!("TASK-GENERAL-S{index}")),
            digest('c'),
        );
        assert_eq!(rejected, Err(LedgerError::SubmissionSecretRejected));
    }
}

#[test]
fn submission_rejects_secret_shaped_formal_project_identity_before_persistence() {
    let secret_project = TaskSubmissionEnvelope::new(
        "lattice_task_submit.v1",
        "request-secret-project",
        "完成角色系統",
        "AI 劇本",
        intake_identity(
            "github_pat_do-not-store",
            "safe-snapshot",
            "TASK-GENERAL-PROJECT-SECRET",
        ),
        digest('c'),
    );
    assert_eq!(secret_project, Err(LedgerError::SubmissionSecretRejected));

    let secret_snapshot = TaskSubmissionEnvelope::new(
        "lattice_task_submit.v1",
        "request-secret-snapshot",
        "完成角色系統",
        "AI 劇本",
        intake_identity(
            "ai-novel",
            "AKIAIOSFODNN7EXAMPLE",
            "TASK-GENERAL-SNAPSHOT-SECRET",
        ),
        digest('c'),
    );
    assert_eq!(secret_snapshot, Err(LedgerError::SubmissionSecretRejected));
}
