use lattice_contracts::{
    AttemptId, CONTRACT_VERSION, CodexDeliveryEvidence, CodexDeliveryRequest,
    CompletedDeliveryEvidence, ContentDigest, DeliveryOutcomeEvidence, DeliveryOutcomeRequest,
    DeliveryProfile, DeliveryReceipt, DeliveryRunRequest, DeliveryRuntime, DeliveryStage,
    DeliveryTerminalStatus, FixedTestEvidence, GitCommitEvidence, Invocation,
    PreparedWorkspaceEvidence, ProjectSnapshotId, RequestId, TaskId, WorkspaceChangeEvidence,
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
