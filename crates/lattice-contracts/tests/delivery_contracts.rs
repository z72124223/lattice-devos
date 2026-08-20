use lattice_contracts::{
    AttemptId, CONTRACT_VERSION, CodexDeliveryEvidence, CodexDeliveryRequest,
    CompletedDeliveryEvidence, ContentDigest, DaemonEpoch, DeliveryContractError,
    DeliveryOutcomeEvidence, DeliveryOutcomeRequest, DeliveryProfile, DeliveryReceipt,
    DeliveryRunRequest, DeliveryRuntime, DeliveryStage, DeliveryTerminalStatus, FencingToken,
    FixedTestEvidence, GitCommitEvidence, HolderProcessId, Invocation, PreparedWorkspaceEvidence,
    ProjectId, ProjectSnapshotId, RequestId, RuntimeAdmissionMode, RuntimeKind, TaskId,
    WRITER_LEASE_PRODUCER_ID, WRITER_LEASE_PRODUCER_VERSION, WorkspaceChangeEvidence,
    WriterLeaseAuthorityHead, WriterLeaseAuthorityReceipt, WriterLeaseIdentity,
    WriterLeaseRevision, WriterLeaseStatus,
};

fn digest(byte: char) -> ContentDigest {
    ContentDigest::from_sha256(byte.to_string().repeat(64)).expect("valid digest")
}

fn invocation(request_id: &str) -> Invocation {
    Invocation::new(
        CONTRACT_VERSION,
        RequestId::new(request_id).expect("request id"),
        TaskId::new("TASK-032").expect("task id"),
        AttemptId::new("attempt-1").expect("attempt id"),
        ProjectSnapshotId::new("snapshot-1").expect("snapshot id"),
        digest('a'),
    )
    .expect("invocation")
}

#[test]
fn delivery_request_rejects_a_zero_configuration_binding() {
    assert!(
        DeliveryRunRequest::new(
            invocation("request-1"),
            DeliveryProfile::Task032CodexPostgres,
            digest('0')
        )
        .is_err()
    );
}

#[test]
fn adapter_workspace_evidence_cannot_cross_request_or_intent_bindings() {
    let first = DeliveryRunRequest::new(
        invocation("request-1"),
        DeliveryProfile::Task032CodexPostgres,
        digest('b'),
    )
    .expect("first request");
    let second = DeliveryRunRequest::new(
        invocation("request-2"),
        DeliveryProfile::Task032CodexPostgres,
        digest('c'),
    )
    .expect("second request");
    let intent = lattice_contracts::DurableIntentEvidence::new(&first, digest('d'))
        .expect("intent evidence");

    assert!(
        PreparedWorkspaceEvidence::new(
            &second,
            &intent,
            "workspace-1",
            r"C:\bounded\repo",
            "1".repeat(40),
            digest('e'),
        )
        .is_err()
    );
}

#[test]
fn complete_stage_chain_preserves_official_runtime_and_terminal_bindings() {
    let request = DeliveryRunRequest::new(
        invocation("request-1"),
        DeliveryProfile::Task032CodexPostgres,
        digest('b'),
    )
    .expect("request");
    let (completed, runtime) =
        completed_chain(request.clone(), DeliveryRuntime::OfficialCodexAppServer);
    let terminal = DeliveryOutcomeRequest::completed(&request, completed).expect("terminal");
    let outcome = DeliveryOutcomeEvidence::new(terminal, digest('a')).expect("outcome");
    let receipt = DeliveryReceipt::new(outcome, digest('f')).expect("receipt");

    assert_eq!(runtime, DeliveryRuntime::OfficialCodexAppServer);
    assert_eq!(receipt.status(), DeliveryTerminalStatus::Completed);
    assert!(receipt.matches_run(&request));
    assert_eq!(
        receipt
            .outcome()
            .request()
            .completed_evidence()
            .expect("success")
            .codex()
            .runtime(),
        DeliveryRuntime::OfficialCodexAppServer
    );
}

#[test]
fn scripted_codex_evidence_cannot_be_misread_as_official_live() {
    let request = DeliveryRunRequest::new(
        invocation("request-1"),
        DeliveryProfile::Task032CodexPostgres,
        digest('b'),
    )
    .expect("request");
    let (completed, _) = completed_chain(request, DeliveryRuntime::ScriptedAcceptance);

    assert_eq!(
        completed.codex().runtime(),
        DeliveryRuntime::ScriptedAcceptance
    );
    assert_ne!(
        completed.codex().runtime(),
        DeliveryRuntime::OfficialCodexAppServer
    );
}

#[test]
fn git_evidence_rejects_test_evidence_from_another_request() {
    let first = DeliveryRunRequest::new(
        invocation("request-1"),
        DeliveryProfile::Task032CodexPostgres,
        digest('b'),
    )
    .expect("first");
    let second = DeliveryRunRequest::new(
        invocation("request-2"),
        DeliveryProfile::Task032CodexPostgres,
        digest('c'),
    )
    .expect("second");
    let (first_completed, _) =
        completed_chain(first.clone(), DeliveryRuntime::OfficialCodexAppServer);
    let (second_completed, _) = completed_chain(second, DeliveryRuntime::OfficialCodexAppServer);

    assert!(
        GitCommitEvidence::new(
            &first,
            first_completed.changes(),
            second_completed.test(),
            "1".repeat(40),
            "2".repeat(40),
            digest('9'),
        )
        .is_err()
    );
}

#[test]
fn failure_and_reconciliation_are_distinct_terminal_states() {
    let request = DeliveryRunRequest::new(
        invocation("request-1"),
        DeliveryProfile::Task032CodexPostgres,
        digest('b'),
    )
    .expect("request");
    let intent =
        lattice_contracts::DurableIntentEvidence::new(&request, digest('1')).expect("intent");

    let failed =
        DeliveryOutcomeRequest::failed(&request, &intent, DeliveryStage::FixedTest, "TEST_FAILED")
            .expect("failed");
    let ambiguous = DeliveryOutcomeRequest::reconciliation_required(
        &request,
        &intent,
        DeliveryStage::GitCommit,
        "COMMIT_OUTCOME_UNKNOWN",
    )
    .expect("ambiguous");

    assert_eq!(failed.status(), DeliveryTerminalStatus::Failed);
    assert_eq!(
        ambiguous.status(),
        DeliveryTerminalStatus::ReconciliationRequired
    );
    assert!(
        DeliveryOutcomeRequest::failed(&request, &intent, DeliveryStage::FixedTest, "bad\ncode",)
            .is_err()
    );
}

#[test]
fn governed_codex_request_retains_only_an_exact_live_active_writer_head() {
    let (run, intent, workspace) = governed_request_parts("request-governed");
    let authority = writer_authority(
        "snapshot-1",
        "TASK-032",
        "attempt-1",
        digest('a'),
        RuntimeKind::Live,
        WriterLeaseStatus::Active,
        RuntimeAdmissionMode::Active,
        1,
    );

    let governed = CodexDeliveryRequest::new_governed(run, intent, workspace, authority.clone())
        .expect("exact live authority is admitted");

    assert_eq!(governed.writer_authority(), Some(&authority));
}

#[test]
fn governed_codex_request_rejects_cross_bound_or_inactive_writer_authority() {
    let (run, intent, workspace) = governed_request_parts("request-governed-mismatch");
    let mismatches = [
        (
            "different-snapshot",
            "TASK-032",
            "attempt-1",
            'a',
            RuntimeKind::Live,
            WriterLeaseStatus::Active,
            RuntimeAdmissionMode::Active,
        ),
        (
            "snapshot-1",
            "TASK-999",
            "attempt-1",
            'a',
            RuntimeKind::Live,
            WriterLeaseStatus::Active,
            RuntimeAdmissionMode::Active,
        ),
        (
            "snapshot-1",
            "TASK-032",
            "different-attempt",
            'a',
            RuntimeKind::Live,
            WriterLeaseStatus::Active,
            RuntimeAdmissionMode::Active,
        ),
        (
            "snapshot-1",
            "TASK-032",
            "attempt-1",
            '9',
            RuntimeKind::Live,
            WriterLeaseStatus::Active,
            RuntimeAdmissionMode::Active,
        ),
        (
            "snapshot-1",
            "TASK-032",
            "attempt-1",
            'a',
            RuntimeKind::Fake,
            WriterLeaseStatus::Active,
            RuntimeAdmissionMode::Active,
        ),
        (
            "snapshot-1",
            "TASK-032",
            "attempt-1",
            'a',
            RuntimeKind::Live,
            WriterLeaseStatus::Suspect,
            RuntimeAdmissionMode::Active,
        ),
        (
            "snapshot-1",
            "TASK-032",
            "attempt-1",
            'a',
            RuntimeKind::Live,
            WriterLeaseStatus::Active,
            RuntimeAdmissionMode::ReconciliationRequired,
        ),
    ];

    for (snapshot, task, attempt, subject, runtime, status, admission) in mismatches {
        let mismatch = writer_authority(
            snapshot,
            task,
            attempt,
            digest(subject),
            runtime,
            status,
            admission,
            1,
        );
        assert_eq!(
            CodexDeliveryRequest::new_governed(
                run.clone(),
                intent.clone(),
                workspace.clone(),
                mismatch,
            ),
            Err(DeliveryContractError::CrossBinding {
                field: "codex_writer_authority"
            })
        );
    }
}

fn governed_request_parts(
    request_id: &str,
) -> (
    DeliveryRunRequest,
    lattice_contracts::DurableIntentEvidence,
    PreparedWorkspaceEvidence,
) {
    let run = DeliveryRunRequest::new(
        invocation(request_id),
        DeliveryProfile::Task032CodexPostgres,
        digest('b'),
    )
    .expect("request");
    let intent =
        lattice_contracts::DurableIntentEvidence::new(&run, digest('1')).expect("intent evidence");
    let workspace = PreparedWorkspaceEvidence::new(
        &run,
        &intent,
        "workspace-1",
        r"C:\bounded\repo",
        "1".repeat(40),
        digest('2'),
    )
    .expect("workspace");
    (run, intent, workspace)
}

#[allow(clippy::too_many_arguments)]
fn writer_authority(
    snapshot: &str,
    task: &str,
    attempt: &str,
    task_spec_digest: ContentDigest,
    runtime: RuntimeKind,
    status: WriterLeaseStatus,
    admission: RuntimeAdmissionMode,
    fence: u64,
) -> WriterLeaseAuthorityHead {
    let identity = WriterLeaseIdentity::new(
        ProjectId::new("project-1").expect("project"),
        ProjectSnapshotId::new(snapshot).expect("snapshot"),
        TaskId::new(task).expect("task"),
        "1",
        task_spec_digest,
        AttemptId::new(attempt).expect("attempt"),
        "lease-1",
        "codex-writer-1",
        "worktree-1",
        HolderProcessId::new(42).expect("process"),
        digest('6'),
        "daemon-1",
        DaemonEpoch::new(1).expect("daemon epoch"),
        FencingToken::new(fence).expect("fencing token"),
    )
    .expect("writer identity");
    WriterLeaseAuthorityReceipt::new(
        CONTRACT_VERSION,
        WRITER_LEASE_PRODUCER_ID,
        WRITER_LEASE_PRODUCER_VERSION,
        runtime,
        identity,
        status,
        WriterLeaseRevision::new(1).expect("writer revision"),
        admission,
        "2026-08-09T00:00:00Z",
        "2026-08-09T00:00:30Z",
        "2026-08-09T00:05:00Z",
        digest('3'),
        digest('4'),
        digest('5'),
        digest('7'),
    )
    .expect("writer receipt")
    .head()
}

fn completed_chain(
    request: DeliveryRunRequest,
    runtime: DeliveryRuntime,
) -> (CompletedDeliveryEvidence, DeliveryRuntime) {
    let intent =
        lattice_contracts::DurableIntentEvidence::new(&request, digest('1')).expect("intent");
    let workspace = PreparedWorkspaceEvidence::new(
        &request,
        &intent,
        "workspace-1",
        r"C:\bounded\repo",
        "1".repeat(40),
        digest('2'),
    )
    .expect("workspace");
    let codex_request =
        CodexDeliveryRequest::new(request.clone(), intent.clone(), workspace.clone())
            .expect("codex request");
    let codex = CodexDeliveryEvidence::new(
        &codex_request,
        runtime,
        r"C:\tools\codex.exe",
        "codex-cli 0.144.6",
        digest('3'),
        digest('4'),
        3,
        "thread-1",
        "turn-1",
        digest('5'),
    )
    .expect("codex");
    let changes = WorkspaceChangeEvidence::new(
        &request,
        &intent,
        &workspace,
        &codex,
        digest('6'),
        digest('7'),
    )
    .expect("changes");
    let test = FixedTestEvidence::new(&request, &changes, digest('8')).expect("test");
    let git = GitCommitEvidence::new(
        &request,
        &changes,
        &test,
        "1".repeat(40),
        "2".repeat(40),
        digest('9'),
    )
    .expect("git");
    (
        CompletedDeliveryEvidence::new(request, intent, workspace, codex, changes, test, git)
            .expect("completed"),
        runtime,
    )
}
