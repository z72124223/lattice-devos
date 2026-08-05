use std::cell::RefCell;
use std::rc::Rc;

use lattice_contracts::{
    AttemptId, CONTRACT_VERSION, CodexDeliveryEvidence, CodexDeliveryRequest, ContentDigest,
    DeliveryOutcomeEvidence, DeliveryOutcomeRequest, DeliveryProfile, DeliveryReceipt,
    DeliveryRunRequest, DeliveryRuntime, DeliveryStage, DeliveryStatusRequest,
    DeliveryTerminalStatus, DurableIntentEvidence, FixedTestEvidence, GitCommitEvidence,
    Invocation, PreparedWorkspaceEvidence, ProjectSnapshotId, RequestId, TaskId,
    WorkspaceChangeEvidence,
};
use lattice_orchestrator::{DeliveryOrchestratorError, delivery_status, run_delivery};
use lattice_ports::{
    DeliveryCodexPort, DeliveryFailureCertainty, DeliveryLedgerPort, DeliveryPortError,
    DeliveryPortResult, PortErrorKind, TestRunnerPort, WorkspaceGitPort,
};

type Calls = Rc<RefCell<Vec<&'static str>>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FailAt {
    Intent,
    Prepare,
    Codex,
    Scope,
    Test,
    Commit,
    Outcome,
    Receipt,
}

struct FakeLedger {
    calls: Calls,
    fail: Option<FailAt>,
    fail_outcome: bool,
    outcome_failure_certainty: DeliveryFailureCertainty,
    last_outcome: Option<DeliveryOutcomeEvidence>,
    wrong_receipt: bool,
}

impl DeliveryLedgerPort for FakeLedger {
    fn record_intent(
        &mut self,
        request: &DeliveryRunRequest,
    ) -> DeliveryPortResult<DurableIntentEvidence> {
        self.calls.borrow_mut().push("intent");
        if self.fail == Some(FailAt::Intent) {
            return Err(known(DeliveryStage::Intent, "INTENT_REJECTED"));
        }
        Ok(DurableIntentEvidence::new(request, digest('1')).expect("intent"))
    }

    fn record_outcome(
        &mut self,
        request: &DeliveryOutcomeRequest,
    ) -> DeliveryPortResult<DeliveryOutcomeEvidence> {
        self.calls.borrow_mut().push("outcome");
        if self.fail_outcome {
            return Err(stage_error(
                DeliveryStage::Outcome,
                self.outcome_failure_certainty,
                "OUTCOME_DEADLINE_EXPIRED",
            ));
        }
        let evidence = DeliveryOutcomeEvidence::new(request.clone(), digest('a')).expect("outcome");
        self.last_outcome = Some(evidence.clone());
        Ok(evidence)
    }

    fn load_receipt(
        &mut self,
        _request: &DeliveryStatusRequest,
    ) -> DeliveryPortResult<DeliveryReceipt> {
        self.calls.borrow_mut().push("receipt");
        if self.fail == Some(FailAt::Receipt) {
            return Err(known(DeliveryStage::Receipt, "RECEIPT_UNAVAILABLE"));
        }
        if self.wrong_receipt {
            return Ok(other_request_receipt());
        }
        DeliveryReceipt::new(
            self.last_outcome.clone().expect("outcome precedes receipt"),
            digest('b'),
        )
        .map_err(|_| known(DeliveryStage::Receipt, "RECEIPT_INVALID"))
    }
}

struct FakeWorkspaceGit {
    calls: Calls,
    fail: Option<FailAt>,
    certainty: DeliveryFailureCertainty,
    wrong_git_evidence: bool,
}

impl WorkspaceGitPort for FakeWorkspaceGit {
    fn prepare(
        &mut self,
        request: &DeliveryRunRequest,
        intent: &DurableIntentEvidence,
    ) -> DeliveryPortResult<PreparedWorkspaceEvidence> {
        self.calls.borrow_mut().push("prepare");
        if self.fail == Some(FailAt::Prepare) {
            return Err(stage_error(
                DeliveryStage::WorkspacePrepare,
                self.certainty,
                "WORKSPACE_REJECTED",
            ));
        }
        PreparedWorkspaceEvidence::new(
            request,
            intent,
            "workspace-1",
            r"C:\bounded\repo",
            "1".repeat(40),
            digest('2'),
        )
        .map_err(|_| known(DeliveryStage::WorkspacePrepare, "WORKSPACE_INVALID"))
    }

    fn inspect_changes(
        &mut self,
        request: &DeliveryRunRequest,
        intent: &DurableIntentEvidence,
        workspace: &PreparedWorkspaceEvidence,
        codex: &CodexDeliveryEvidence,
    ) -> DeliveryPortResult<WorkspaceChangeEvidence> {
        self.calls.borrow_mut().push("scope");
        if self.fail == Some(FailAt::Scope) {
            return Err(stage_error(
                DeliveryStage::ScopeVerification,
                self.certainty,
                "SCOPE_REJECTED",
            ));
        }
        WorkspaceChangeEvidence::new(request, intent, workspace, codex, digest('6'), digest('7'))
            .map_err(|_| known(DeliveryStage::ScopeVerification, "SCOPE_INVALID"))
    }

    fn commit(
        &mut self,
        request: &DeliveryRunRequest,
        _workspace: &PreparedWorkspaceEvidence,
        changes: &WorkspaceChangeEvidence,
        test: &FixedTestEvidence,
    ) -> DeliveryPortResult<GitCommitEvidence> {
        self.calls.borrow_mut().push("commit");
        if self.fail == Some(FailAt::Commit) {
            return Err(stage_error(
                DeliveryStage::GitCommit,
                self.certainty,
                "COMMIT_REJECTED",
            ));
        }
        if self.wrong_git_evidence {
            return Ok(other_request_git_evidence());
        }
        GitCommitEvidence::new(
            request,
            changes,
            test,
            "1".repeat(40),
            "2".repeat(40),
            digest('9'),
        )
        .map_err(|_| known(DeliveryStage::GitCommit, "COMMIT_INVALID"))
    }
}

impl TestRunnerPort for FakeWorkspaceGit {
    fn run_fixed(
        &mut self,
        request: &DeliveryRunRequest,
        _workspace: &PreparedWorkspaceEvidence,
        changes: &WorkspaceChangeEvidence,
    ) -> DeliveryPortResult<FixedTestEvidence> {
        self.calls.borrow_mut().push("test");
        if self.fail == Some(FailAt::Test) {
            return Err(stage_error(
                DeliveryStage::FixedTest,
                self.certainty,
                "TEST_REJECTED",
            ));
        }
        FixedTestEvidence::new(request, changes, digest('8'))
            .map_err(|_| known(DeliveryStage::FixedTest, "TEST_EVIDENCE_INVALID"))
    }
}

struct FakeCodex {
    calls: Calls,
    fail: Option<FailAt>,
    certainty: DeliveryFailureCertainty,
}

impl DeliveryCodexPort for FakeCodex {
    fn run_delivery(
        &mut self,
        request: CodexDeliveryRequest,
    ) -> DeliveryPortResult<CodexDeliveryEvidence> {
        self.calls.borrow_mut().push("codex");
        if self.fail == Some(FailAt::Codex) {
            return Err(stage_error(
                DeliveryStage::Codex,
                self.certainty,
                "CODEX_REJECTED",
            ));
        }
        CodexDeliveryEvidence::new(
            &request,
            DeliveryRuntime::OfficialCodexAppServer,
            r"C:\tools\codex.exe",
            "codex-cli 0.144.6",
            digest('3'),
            digest('4'),
            3,
            "thread-1",
            "turn-1",
            digest('5'),
        )
        .map_err(|_| known(DeliveryStage::Codex, "CODEX_EVIDENCE_INVALID"))
    }

    fn interrupt_delivery(&mut self, _request_id: &RequestId) -> DeliveryPortResult<()> {
        Ok(())
    }
}

struct Scenario {
    calls: Calls,
    ledger: FakeLedger,
    workspace: FakeWorkspaceGit,
    codex: FakeCodex,
}

impl Scenario {
    fn new(fail: Option<FailAt>, certainty: DeliveryFailureCertainty) -> Self {
        let calls = Rc::new(RefCell::new(Vec::new()));
        Self {
            calls: calls.clone(),
            ledger: FakeLedger {
                calls: calls.clone(),
                fail,
                fail_outcome: fail == Some(FailAt::Outcome),
                outcome_failure_certainty: certainty,
                last_outcome: None,
                wrong_receipt: false,
            },
            workspace: FakeWorkspaceGit {
                calls: calls.clone(),
                fail,
                certainty,
                wrong_git_evidence: false,
            },
            codex: FakeCodex {
                calls,
                fail,
                certainty,
            },
        }
    }

    fn run(&mut self) -> Result<DeliveryReceipt, DeliveryOrchestratorError> {
        let request = request("request-1");
        run_delivery(
            &request,
            &mut self.ledger,
            &mut self.workspace,
            &mut self.codex,
        )
    }

    fn call_log(&self) -> Vec<&'static str> {
        self.calls.borrow().clone()
    }
}

#[test]
fn success_uses_the_one_exact_effect_order() {
    let mut scenario = Scenario::new(None, DeliveryFailureCertainty::Known);

    let receipt = scenario.run().expect("delivery succeeds");

    assert_eq!(receipt.status(), DeliveryTerminalStatus::Completed);
    assert_eq!(
        scenario.call_log(),
        [
            "intent", "prepare", "codex", "scope", "test", "commit", "outcome", "receipt"
        ]
    );
}

#[test]
fn intent_failure_has_zero_later_effects() {
    let mut scenario = Scenario::new(Some(FailAt::Intent), DeliveryFailureCertainty::Known);

    assert!(matches!(
        scenario.run(),
        Err(DeliveryOrchestratorError::Intent(_))
    ));
    assert_eq!(scenario.call_log(), ["intent"]);
}

#[test]
fn each_known_stage_failure_stops_later_effects_and_is_reloaded_as_failed() {
    let cases = [
        (
            FailAt::Prepare,
            vec!["intent", "prepare", "outcome", "receipt"],
        ),
        (
            FailAt::Codex,
            vec!["intent", "prepare", "codex", "outcome", "receipt"],
        ),
        (
            FailAt::Scope,
            vec!["intent", "prepare", "codex", "scope", "outcome", "receipt"],
        ),
        (
            FailAt::Test,
            vec![
                "intent", "prepare", "codex", "scope", "test", "outcome", "receipt",
            ],
        ),
        (
            FailAt::Commit,
            vec![
                "intent", "prepare", "codex", "scope", "test", "commit", "outcome", "receipt",
            ],
        ),
    ];

    for (stage, expected) in cases {
        let mut scenario = Scenario::new(Some(stage), DeliveryFailureCertainty::Known);
        let result = scenario.run();
        let Err(DeliveryOrchestratorError::Terminal { receipt, .. }) = result else {
            panic!("expected terminal failure for {stage:?}");
        };
        assert_eq!(receipt.status(), DeliveryTerminalStatus::Failed);
        assert_eq!(scenario.call_log(), expected, "{stage:?}");
    }
}

#[test]
fn ambiguous_commit_is_never_reported_as_failed_or_completed() {
    let mut scenario = Scenario::new(Some(FailAt::Commit), DeliveryFailureCertainty::Ambiguous);

    let Err(DeliveryOrchestratorError::Terminal { receipt, .. }) = scenario.run() else {
        panic!("expected terminal reconciliation result");
    };

    assert_eq!(
        receipt.status(),
        DeliveryTerminalStatus::ReconciliationRequired
    );
    assert_eq!(
        scenario.call_log(),
        [
            "intent", "prepare", "codex", "scope", "test", "commit", "outcome", "receipt"
        ]
    );
}

#[test]
fn cross_bound_post_commit_evidence_is_persisted_as_reconciliation_required() {
    let mut scenario = Scenario::new(None, DeliveryFailureCertainty::Known);
    scenario.workspace.wrong_git_evidence = true;

    let Err(DeliveryOrchestratorError::Terminal { receipt, .. }) = scenario.run() else {
        panic!("expected terminal reconciliation result");
    };

    assert_eq!(
        receipt.status(),
        DeliveryTerminalStatus::ReconciliationRequired
    );
    assert_eq!(
        scenario.call_log(),
        [
            "intent", "prepare", "codex", "scope", "test", "commit", "outcome", "receipt"
        ]
    );
}

#[test]
fn known_outcome_write_failure_after_commit_requires_reconciliation() {
    let mut scenario = Scenario::new(Some(FailAt::Outcome), DeliveryFailureCertainty::Known);

    let Err(DeliveryOrchestratorError::OutcomePersistence(error)) = scenario.run() else {
        panic!("expected outcome persistence failure");
    };
    assert_eq!(error.kind(), PortErrorKind::Ambiguous);
    assert_eq!(error.certainty(), DeliveryFailureCertainty::Ambiguous);
    assert_eq!(
        scenario.call_log(),
        [
            "intent", "prepare", "codex", "scope", "test", "commit", "outcome"
        ]
    );
}

#[test]
fn ambiguous_commit_plus_known_outcome_deadline_remains_reconciliation_required() {
    let mut scenario = Scenario::new(Some(FailAt::Commit), DeliveryFailureCertainty::Ambiguous);
    scenario.ledger.fail_outcome = true;
    scenario.ledger.outcome_failure_certainty = DeliveryFailureCertainty::Known;

    let Err(DeliveryOrchestratorError::OutcomePersistence(error)) = scenario.run() else {
        panic!("expected outcome persistence failure");
    };

    assert_eq!(error.kind(), PortErrorKind::Ambiguous);
    assert_eq!(error.certainty(), DeliveryFailureCertainty::Ambiguous);
    assert_eq!(
        scenario.call_log(),
        [
            "intent", "prepare", "codex", "scope", "test", "commit", "outcome"
        ]
    );
}

#[test]
fn receipt_cross_binding_is_rejected_after_successful_outcome_write() {
    let mut scenario = Scenario::new(None, DeliveryFailureCertainty::Known);
    scenario.ledger.wrong_receipt = true;

    assert_eq!(
        scenario.run(),
        Err(DeliveryOrchestratorError::ReceiptMismatch)
    );
}

#[test]
fn status_path_calls_only_the_durable_receipt_port() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let run = request("request-1");
    let intent = DurableIntentEvidence::new(&run, digest('1')).expect("intent");
    let terminal =
        DeliveryOutcomeRequest::failed(&run, &intent, DeliveryStage::Codex, "CODEX_REJECTED")
            .expect("terminal");
    let outcome = DeliveryOutcomeEvidence::new(terminal, digest('a')).expect("outcome");
    let mut ledger = FakeLedger {
        calls: calls.clone(),
        fail: None,
        fail_outcome: false,
        outcome_failure_certainty: DeliveryFailureCertainty::Known,
        last_outcome: Some(outcome),
        wrong_receipt: false,
    };

    let receipt = delivery_status(&run.status_request(), &mut ledger).expect("status");

    assert_eq!(receipt.status(), DeliveryTerminalStatus::Failed);
    assert_eq!(*calls.borrow(), ["receipt"]);
}

fn request(request_id: &str) -> DeliveryRunRequest {
    let invocation = Invocation::new(
        CONTRACT_VERSION,
        RequestId::new(request_id).expect("request id"),
        TaskId::new("TASK-032").expect("task id"),
        AttemptId::new("attempt-1").expect("attempt id"),
        ProjectSnapshotId::new("snapshot-1").expect("snapshot id"),
        digest('c'),
    )
    .expect("invocation");
    DeliveryRunRequest::new(
        invocation,
        DeliveryProfile::Task032CodexPostgres,
        digest('d'),
    )
    .expect("delivery request")
}

fn other_request_receipt() -> DeliveryReceipt {
    let run = request("other-request");
    let intent = DurableIntentEvidence::new(&run, digest('1')).expect("intent");
    let terminal =
        DeliveryOutcomeRequest::failed(&run, &intent, DeliveryStage::Codex, "OTHER_REQUEST")
            .expect("terminal");
    let outcome = DeliveryOutcomeEvidence::new(terminal, digest('a')).expect("outcome");
    DeliveryReceipt::new(outcome, digest('b')).expect("receipt")
}

fn other_request_git_evidence() -> GitCommitEvidence {
    let run = request("other-request");
    let intent = DurableIntentEvidence::new(&run, digest('1')).expect("intent");
    let workspace = PreparedWorkspaceEvidence::new(
        &run,
        &intent,
        "other-workspace",
        r"C:\bounded\other-repo",
        "3".repeat(40),
        digest('2'),
    )
    .expect("workspace");
    let codex_request = CodexDeliveryRequest::new(run.clone(), intent.clone(), workspace.clone())
        .expect("codex request");
    let codex = CodexDeliveryEvidence::new(
        &codex_request,
        DeliveryRuntime::OfficialCodexAppServer,
        r"C:\tools\codex.exe",
        "codex-cli 0.144.6",
        digest('3'),
        digest('4'),
        3,
        "other-thread",
        "other-turn",
        digest('5'),
    )
    .expect("codex evidence");
    let changes =
        WorkspaceChangeEvidence::new(&run, &intent, &workspace, &codex, digest('6'), digest('7'))
            .expect("changes");
    let test = FixedTestEvidence::new(&run, &changes, digest('8')).expect("test");
    GitCommitEvidence::new(
        &run,
        &changes,
        &test,
        "3".repeat(40),
        "4".repeat(40),
        digest('9'),
    )
    .expect("git evidence")
}

fn digest(byte: char) -> ContentDigest {
    ContentDigest::from_sha256(byte.to_string().repeat(64)).expect("digest")
}

fn known(stage: DeliveryStage, code: &'static str) -> DeliveryPortError {
    stage_error(stage, DeliveryFailureCertainty::Known, code)
}

fn stage_error(
    stage: DeliveryStage,
    certainty: DeliveryFailureCertainty,
    code: &'static str,
) -> DeliveryPortError {
    let kind = if certainty == DeliveryFailureCertainty::Ambiguous {
        PortErrorKind::Ambiguous
    } else {
        PortErrorKind::Denied
    };
    DeliveryPortError::new(stage, kind, certainty, code)
}
