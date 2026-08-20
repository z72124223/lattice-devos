use lattice_workflow_learning::{
    Action, CandidateDisposition, ChallengeOutcome, Confidence, EpistemicDecision, EpistemicRecord,
    EpistemicRecordInput, EvidencePointer, EvidenceSource, Gate, GateReceipt, Hypothesis,
    LearningError, LearningLog, Metrics, ModelTier, MonitorOutcome, Observation, ObservationCode,
    ObserveOutcome, PolicyCandidate, PolicyCandidateInput, PolicyController, PolicyRule,
    PolicyScope, ProbePlan, ProbeResult, ProbeRisk, ProposedChange, ProtectedAuthority,
    RecordStatus, RefreshReason, RefreshTrigger, Scope, Signal, StopAudit, StopClaim,
    StopCondition, Suspicion, SuspicionDomain, SuspicionInput, SuspicionQueue,
    SuspicionQueueOutcome, SuspicionState, TopologyAction, TopologyDecision, Window, WindowState,
    WorkCandidate, WorktreeMoveReceipt, WorktreeTopology, WorktreeTopologyInput,
};

fn digest(byte: char) -> String {
    format!("sha256:{}", byte.to_string().repeat(64))
}

fn evidence(source: EvidenceSource, byte: char) -> EvidencePointer {
    EvidencePointer::new(source, digest(byte), true).unwrap()
}

fn complete_evidence() -> Vec<EvidencePointer> {
    vec![
        evidence(EvidenceSource::TaskLedger, 'a'),
        evidence(EvidenceSource::Ticket, 'b'),
        evidence(EvidenceSource::Plans, 'c'),
        evidence(EvidenceSource::Dashboard, 'd'),
        evidence(EvidenceSource::GitWorktree, 'e'),
        evidence(EvidenceSource::DependencyGraph, 'f'),
        evidence(EvidenceSource::BacklogInventory, '1'),
        evidence(EvidenceSource::ToolInventory, '2'),
        evidence(EvidenceSource::ModelCatalog, '3'),
    ]
}

#[test]
fn task080_and_large_project_replay_prohibit_stop_and_dispatch_safe_work() {
    let task080 = include_str!("fixtures/task-080-backlog-contradiction-v1.txt");
    let foreman = include_str!("fixtures/large-project-zero-work-misjudgment-v1.txt");
    assert!(task080.contains("dashboard_unknown=23"));
    assert!(task080.contains("ready_low_cost_candidates=TASK-081,TASK-082,TASK-083"));
    assert!(foreman.contains("rejected_decision=ALL_WAITING_OR_ZERO_WORK"));

    let audit = StopAudit::new(
        StopClaim::ZeroWork,
        vec![
            Window::new("worker-a", WindowState::Unknown),
            Window::new("worker-b", WindowState::Idle),
        ],
        vec![WorkCandidate::ready("TASK-081", ModelTier::Luna, 2).unwrap()],
        complete_evidence(),
        true,
        true,
    );

    let ChallengeOutcome::Continue {
        counterexamples,
        actions,
    } = audit.challenge(1, 9, 8)
    else {
        panic!("counterevidence must prohibit a zero-work certificate");
    };
    assert!(counterexamples.len() >= 4);
    assert!(actions.contains(&Action::Reconcile));
    assert!(actions.contains(&Action::Dispatch("TASK-081".into())));
}

#[test]
fn all_windows_stale_unknown_or_idle_never_prove_no_work() {
    let audit = StopAudit::new(
        StopClaim::AllWaiting,
        vec![
            Window::new("worker-a", WindowState::Unknown),
            Window::new("worker-b", WindowState::Idle),
        ],
        Vec::new(),
        complete_evidence(),
        false,
        false,
    );
    assert!(matches!(
        audit.challenge(1, 4, 5),
        ChallengeOutcome::Continue { .. }
    ));
}

#[test]
fn active_work_also_blocks_a_stop_certificate() {
    let audit = StopAudit::new(
        StopClaim::EndRound,
        vec![Window::new("worker-active", WindowState::Active)],
        Vec::new(),
        complete_evidence(),
        false,
        false,
    );
    let ChallengeOutcome::Continue {
        counterexamples, ..
    } = audit.challenge(1, 4, 5)
    else {
        panic!("active work must prohibit ending the round");
    };
    assert!(counterexamples.iter().any(|counterexample| matches!(
        counterexample,
        lattice_workflow_learning::Counterexample::ActiveWindow { .. }
    )));
}

#[test]
fn zero_work_certificate_requires_fresh_exhaustive_exact_reasons() {
    let candidates = vec![
        WorkCandidate::blocked_dependency("TASK-051", "TASK-079").unwrap(),
        WorkCandidate::resource_conflict("TASK-090", "worktree:task-079").unwrap(),
        WorkCandidate::duplicate("TASK-091", "TASK-081").unwrap(),
    ];
    let audit = StopAudit::new(
        StopClaim::EndRound,
        vec![Window::new("worker-a", WindowState::Blocked)],
        candidates,
        complete_evidence(),
        false,
        false,
    );
    let ChallengeOutcome::Certified(certificate) = audit.challenge(3, 11, 8) else {
        panic!("fresh exhaustive reasons must produce a certificate");
    };
    assert_eq!(certificate.schema(), "lattice.zero-work-certificate/1.0");
    assert_eq!(certificate.policy_version(), 3);
    assert_eq!(certificate.fence(), 11);

    let mut stale = complete_evidence();
    stale[0] = EvidencePointer::new(EvidenceSource::TaskLedger, digest('4'), false).unwrap();
    let stale_audit = StopAudit::new(
        StopClaim::Healthy,
        Vec::new(),
        vec![WorkCandidate::blocked_dependency("TASK-051", "TASK-079").unwrap()],
        stale,
        false,
        false,
    );
    assert!(matches!(
        stale_audit.challenge(3, 11, 8),
        ChallengeOutcome::Continue { .. }
    ));
}

#[test]
fn malicious_reports_do_not_contaminate_learning_and_duplicates_are_idempotent() {
    let mut log = LearningLog::default();
    let event = Observation::new(
        "event-1",
        "incident-1",
        ObservationCode::ReadyBacklog,
        vec![evidence(EvidenceSource::UntrustedReport, '5')],
    )
    .unwrap();
    assert_eq!(log.observe(event.clone()), ObserveOutcome::Recorded);
    assert_eq!(log.observe(event), ObserveOutcome::DuplicateIgnored);
    assert!(log.diagnose().is_empty());

    let trusted = Observation::new(
        "event-2",
        "incident-1",
        ObservationCode::ReadyBacklog,
        vec![
            evidence(EvidenceSource::Ticket, '6'),
            evidence(EvidenceSource::GitWorktree, '7'),
        ],
    )
    .unwrap();
    assert_eq!(log.observe(trusted), ObserveOutcome::Recorded);
    assert_eq!(log.diagnose().len(), 1);
}

fn epistemic_input() -> EpistemicRecordInput {
    let observation = Observation::new(
        "event-epistemic-1",
        "incident-zero-work",
        ObservationCode::ZeroWorkClaim,
        vec![evidence(EvidenceSource::Dashboard, '8')],
    )
    .unwrap();
    EpistemicRecordInput {
        id: "record-1".into(),
        observation,
        hypotheses: vec![Hypothesis::NoDispatchableWork, Hypothesis::UnopenedBacklog],
        confidence: Confidence::High,
        unknowns: vec!["unopened-ticket-inventory".into()],
        supporting_evidence: vec![evidence(EvidenceSource::Dashboard, '9')],
        counterevidence: Vec::new(),
        assumptions: vec!["dashboard-identity-is-current".into()],
        scope: Scope::ForemanDispatch,
        checked_at: 100,
        expires_at: 200,
        refresh_triggers: vec![
            RefreshTrigger::EvidenceExpired,
            RefreshTrigger::Counterevidence,
            RefreshTrigger::DependencyChanged,
            RefreshTrigger::ToolFailure,
        ],
        decision: EpistemicDecision::Wait,
        expected_signal: Signal::ExhaustiveNoReadyCandidates,
        actual_signal: None,
        falsifier: Signal::ReadyTicketFound,
        next_probe: ProbePlan::new(
            Hypothesis::NoDispatchableWork,
            Hypothesis::UnopenedBacklog,
            2,
            ProbeRisk::ReadOnly,
            StopCondition::FirstContradiction,
        )
        .unwrap(),
    }
}

fn epistemic_record() -> EpistemicRecord {
    EpistemicRecord::new(epistemic_input()).unwrap()
}

#[test]
fn untrusted_support_and_arbitrary_paths_cannot_enter_epistemic_learning() {
    let mut input = epistemic_input();
    input.supporting_evidence = vec![evidence(EvidenceSource::UntrustedReport, '0')];
    assert_eq!(
        EpistemicRecord::new(input),
        Err(LearningError::MissingEvidence)
    );
    assert_eq!(
        Observation::new(
            "C:\\Users\\worker\\secret.txt",
            "incident-path",
            ObservationCode::ToolPathFailure,
            vec![evidence(EvidenceSource::ToolInventory, '0')],
        ),
        Err(LearningError::ForbiddenContent)
    );
}

#[test]
fn two_reasonable_hypotheses_coexist_and_unknown_remains_unknown() {
    let record = epistemic_record();
    assert_eq!(record.hypotheses().len(), 2);
    assert_eq!(record.decision(), EpistemicDecision::Wait);
    assert_eq!(record.actual_signal(), None);
    assert_eq!(record.status_at(150), RecordStatus::Fresh);
}

#[test]
fn counterevidence_degrades_confidence_and_expiry_requests_refresh() {
    let mut record = epistemic_record();
    record
        .add_counterevidence(evidence(EvidenceSource::Ticket, 'a'), 150)
        .unwrap();
    assert_eq!(record.confidence(), Confidence::Medium);
    assert_eq!(record.decision(), EpistemicDecision::Refresh);
    assert_eq!(
        record.status_at(201),
        RecordStatus::RefreshRequired(RefreshReason::Expired)
    );
}

#[test]
fn reversible_probe_can_falsify_and_overturn_the_original_judgment() {
    let mut record = epistemic_record();
    record
        .apply_probe(
            ProbeResult::Falsified {
                hypothesis: Hypothesis::NoDispatchableWork,
                actual_signal: Signal::ReadyTicketFound,
                evidence: evidence(EvidenceSource::BacklogInventory, 'b'),
            },
            160,
        )
        .unwrap();
    assert_eq!(record.actual_signal(), Some(Signal::ReadyTicketFound));
    assert_eq!(record.decision(), EpistemicDecision::Reconcile);
    assert_eq!(record.confidence(), Confidence::Medium);
}

#[test]
fn continuous_suspicion_queue_prioritizes_product_value_without_busywork() {
    let mut queue = SuspicionQueue::default();
    let product = Suspicion::new(SuspicionInput {
        id: "product-integration-gap".into(),
        domain: SuspicionDomain::ProductBehavior,
        record: epistemic_record(),
        user_value: 40,
        consequence_risk: 40,
        freshness: 90,
        probe_cost: 10,
        reversibility: 90,
        conflict: 5,
        governance_only: false,
    })
    .unwrap();
    let governance = Suspicion::new(SuspicionInput {
        id: "governance-format-question".into(),
        domain: SuspicionDomain::TruthSourceContradiction,
        record: epistemic_record(),
        user_value: 100,
        consequence_risk: 100,
        freshness: 100,
        probe_cost: 1,
        reversibility: 100,
        conflict: 0,
        governance_only: true,
    })
    .unwrap();
    assert_eq!(
        queue.enqueue(product.clone(), 150),
        SuspicionQueueOutcome::Queued
    );
    assert_eq!(
        queue.enqueue(governance, 150),
        SuspicionQueueOutcome::Queued
    );
    assert_eq!(
        queue.enqueue(product, 150),
        SuspicionQueueOutcome::DuplicateFreshRejected
    );
    let next = queue.next_probe(150).unwrap();
    assert_eq!(next.id(), "product-integration-gap");
    assert_eq!(next.state(), SuspicionState::Hypothesis);
    assert!(
        queue.next_probe(201).is_none(),
        "expired suspicions require refresh"
    );
}

#[test]
fn task083_nested_worktree_blocks_completion_and_verifies_git_native_move() {
    let fixture = include_str!("fixtures/task-083-nested-worktree-repair-v1.txt");
    assert!(fixture.contains("safe_repair=GIT_WORKTREE_MOVE"));
    assert!(fixture.contains("actual_outcome=SOURCE_ABSENT,TARGET_REGISTERED"));

    let topology = WorktreeTopology::new(WorktreeTopologyInput {
        worktree: "TASK-083".into(),
        branch: "feature/task-083-task-075-terminal-evidence".into(),
        head: "bce480923dd3204eea62df4570753d7805b6e7e6".into(),
        resolved_path: evidence(EvidenceSource::GitWorktree, 'c'),
        approved_root: evidence(EvidenceSource::GitWorktree, 'd'),
        direct_child_of_approved_root: false,
        nested_ancestor: Some("TASK-078".into()),
        reparse_escape: false,
        child_clean: true,
        parent_clean_before: false,
        parent_clean_after: None,
    })
    .unwrap();
    let decision = topology.analyze();
    assert_eq!(
        decision,
        TopologyDecision::RepairRequired(TopologyAction::GitWorktreeMove)
    );

    let audit = StopAudit::new(
        StopClaim::Completion,
        vec![Window::new("worker-083", WindowState::Complete)],
        vec![WorkCandidate::blocked_dependency("TASK-083", "TASK-085").unwrap()],
        complete_evidence(),
        false,
        false,
    )
    .with_topology(decision);
    let ChallengeOutcome::Continue { actions, .. } = audit.challenge(1, 4, 8) else {
        panic!("nested topology must block completion certification");
    };
    assert!(actions.contains(&Action::RepairWorktreeTopology(
        TopologyAction::GitWorktreeMove
    )));

    let receipt = WorktreeMoveReceipt {
        operation: TopologyAction::GitWorktreeMove,
        source_absent: true,
        target_registered: true,
        branch_head_exact: true,
        child_clean: true,
        parent_clean_after: true,
        used_manual_file_move: false,
        used_delete: false,
        used_reset_or_clean: false,
    };
    assert_eq!(
        topology.verify_repair(&receipt).unwrap(),
        TopologyDecision::RepairVerified
    );
}

#[test]
fn topology_fault_matrix_and_destructive_repair_fail_closed() {
    for (direct, nested, reparse, child_clean, parent_clean) in [
        (false, None, false, true, true),
        (true, Some("TASK-078".into()), false, true, true),
        (true, None, true, true, true),
        (true, None, false, false, true),
        (true, None, false, true, false),
    ] {
        let topology = WorktreeTopology::new(WorktreeTopologyInput {
            worktree: "TASK-083".into(),
            branch: "feature/task-083".into(),
            head: "bce480923dd3204eea62df4570753d7805b6e7e6".into(),
            resolved_path: evidence(EvidenceSource::GitWorktree, 'c'),
            approved_root: evidence(EvidenceSource::GitWorktree, 'd'),
            direct_child_of_approved_root: direct,
            nested_ancestor: nested,
            reparse_escape: reparse,
            child_clean,
            parent_clean_before: parent_clean,
            parent_clean_after: None,
        })
        .unwrap();
        assert_eq!(
            topology.analyze(),
            TopologyDecision::RepairRequired(TopologyAction::GitWorktreeMove)
        );
        let destructive = WorktreeMoveReceipt {
            operation: TopologyAction::GitWorktreeMove,
            source_absent: true,
            target_registered: true,
            branch_head_exact: true,
            child_clean: true,
            parent_clean_after: true,
            used_manual_file_move: true,
            used_delete: true,
            used_reset_or_clean: true,
        };
        assert_eq!(
            topology.verify_repair(&destructive),
            Err(LearningError::InvalidProbe)
        );
    }
}

fn candidate_with_cases(version: u64, base: u64, fence: u64, cases: &[&str]) -> PolicyCandidate {
    PolicyCandidate::new(PolicyCandidateInput {
        version,
        base_version: base,
        fence,
        changes: vec![
            ProposedChange::Rule(PolicyRule::ChallengeBeforeStop),
            ProposedChange::Rule(PolicyRule::UnknownRequiresReconciliation),
            ProposedChange::Rule(PolicyRule::IdleRequiresBacklogAudit),
        ],
        max_cost_units: 6,
        replay_cases: cases.iter().map(|case| (*case).to_owned()).collect(),
        reason: "counterevidence-found-ready-work".into(),
        expected_impact: "reduce-missed-dispatch-without-authority-expansion".into(),
        scope: PolicyScope::PreStopChallenge,
    })
    .unwrap()
}

fn safe_candidate(version: u64, base: u64, fence: u64) -> PolicyCandidate {
    candidate_with_cases(version, base, fence, &["TASK-080", "fault-all-stale"])
}

fn passing_gates() -> Vec<GateReceipt> {
    Gate::ALL
        .into_iter()
        .enumerate()
        .map(|(index, gate)| {
            GateReceipt::pass(
                gate,
                evidence(
                    EvidenceSource::TaskLedger,
                    char::from(b'a' + u8::try_from(index).unwrap()),
                ),
            )
            .unwrap()
        })
        .collect()
}

#[test]
fn promotion_requires_evidence_exact_version_and_current_fence() {
    let mut controller = PolicyController::new(1, 7, 8);
    assert_eq!(
        controller.promote(&safe_candidate(2, 1, 7), &[]),
        Err(LearningError::MissingMachineGate)
    );
    assert_eq!(controller.active_version(), 1);

    controller
        .promote(&safe_candidate(2, 1, 7), &passing_gates())
        .unwrap();
    assert_eq!(controller.active_version(), 2);
    assert_eq!(controller.previous_version(), Some(1));
    assert_eq!(
        controller.promote(&safe_candidate(3, 1, 7), &passing_gates()),
        Err(LearningError::StalePolicyFence)
    );
}

#[test]
fn a_single_case_cannot_promote_policy() {
    let mut controller = PolicyController::new(1, 7, 8);
    assert_eq!(
        controller.promote(
            &candidate_with_cases(2, 1, 7, &["TASK-080"]),
            &passing_gates(),
        ),
        Err(LearningError::InsufficientReplayCases)
    );
    assert_eq!(controller.active_version(), 1);
}

#[test]
fn worktree_path_policy_requires_task083_and_an_independent_fault_case() {
    let mut controller = PolicyController::new(1, 7, 8);
    let candidate = PolicyCandidate::new(PolicyCandidateInput {
        version: 2,
        base_version: 1,
        fence: 7,
        changes: vec![ProposedChange::Rule(PolicyRule::ValidateWorktreeTopology)],
        max_cost_units: 2,
        replay_cases: vec!["TASK-083".into(), "fault-reparse-escape".into()],
        reason: "nested-worktree-counterexample".into(),
        expected_impact: "block-invalid-topology-certification".into(),
        scope: PolicyScope::WorktreeTopology,
    })
    .unwrap();
    assert_eq!(candidate.reason(), "nested-worktree-counterexample");
    assert_eq!(candidate.scope(), PolicyScope::WorktreeTopology);
    controller.promote(&candidate, &passing_gates()).unwrap();
    assert_eq!(controller.active_version(), 2);

    let missing_actual_case = PolicyCandidate::new(PolicyCandidateInput {
        version: 2,
        base_version: 1,
        fence: 7,
        changes: vec![ProposedChange::Rule(PolicyRule::ValidateWorktreeTopology)],
        max_cost_units: 2,
        replay_cases: vec!["fault-nested-a".into(), "fault-reparse-b".into()],
        reason: "generic-path-theory".into(),
        expected_impact: "block-invalid-topology-certification".into(),
        scope: PolicyScope::WorktreeTopology,
    })
    .unwrap();
    assert_eq!(
        PolicyController::new(1, 7, 8).promote(&missing_actual_case, &passing_gates()),
        Err(LearningError::InsufficientReplayCases)
    );
}

#[test]
fn fresh_process_recovery_and_monitor_regression_roll_back() {
    let mut controller = PolicyController::new(1, 3, 8);
    controller
        .promote(&safe_candidate(2, 1, 3), &passing_gates())
        .unwrap();
    let checkpoint = controller.checkpoint();
    let mut recovered = PolicyController::recover(checkpoint).unwrap();
    assert_eq!(recovered.active_version(), 2);
    assert_eq!(recovered.previous_version(), Some(1));

    let outcome = recovered.monitor(Metrics {
        misdispatches: 0,
        missed_dispatches: 2,
        duplicate_work: 0,
        false_positives: 0,
        cost_units: 4,
    });
    assert_eq!(outcome, MonitorOutcome::RolledBack { from: 2, to: 1 });
    assert_eq!(recovered.active_version(), 1);
}

#[test]
fn protected_authority_and_cost_ceiling_are_not_learnable() {
    for authority in ProtectedAuthority::ALL {
        assert_eq!(
            PolicyCandidate::new(PolicyCandidateInput {
                version: 2,
                base_version: 1,
                fence: 1,
                changes: vec![ProposedChange::AuthorityExpansion(authority)],
                max_cost_units: 1,
                replay_cases: vec!["case-a".into(), "case-b".into()],
                reason: "attempted-expansion".into(),
                expected_impact: "forbidden".into(),
                scope: PolicyScope::PreStopChallenge,
            }),
            Err(LearningError::ProtectedAuthority)
        );
    }
    assert_eq!(
        PolicyController::new(1, 1, 4).promote(&safe_candidate(2, 1, 1), &passing_gates()),
        Err(LearningError::CostLimitExceeded)
    );
}

#[test]
fn real_dependency_resource_and_duplicate_conflicts_remain_denied() {
    let candidates = [
        WorkCandidate::blocked_dependency("TASK-051", "TASK-079").unwrap(),
        WorkCandidate::resource_conflict("TASK-090", "postgres-global-writer").unwrap(),
        WorkCandidate::duplicate("TASK-091", "TASK-081").unwrap(),
    ];
    assert!(candidates.iter().all(|candidate| !candidate.dispatchable()));
    assert!(matches!(
        candidates[0].disposition(),
        CandidateDisposition::BlockedDependency { .. }
    ));
}
