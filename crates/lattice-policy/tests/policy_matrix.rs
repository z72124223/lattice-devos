use lattice_approval_verifier::{
    FakeApprovalVerifier, FakeNormalSigner, IssueApprovalCommand, SecretMaterial,
    VerifyApprovalCommand, nonce_commitment,
};
use lattice_contracts::{
    APPROVAL_VERIFIER_PRODUCER_ID, APPROVAL_VERIFIER_PRODUCER_VERSION, ApprovalAuthorityHead,
    ApprovalAuthorityReceipt, ApprovalIdentity, ApprovalLane, ApprovalRevision, ApprovalStatus,
    AttemptId, CONTRACT_VERSION, ContentDigest, DaemonEpoch, FencingToken, GitRefIdentity,
    HolderProcessId, ProjectAuthorityReceipt, ProjectClass, ProjectId, ProjectLifecycle,
    ProjectSnapshotId, ResourceCounters, ResourceRequest, RuntimeAdmissionMode, RuntimeKind,
    TASK_LEDGER_PRODUCER_ID, TASK_LEDGER_PRODUCER_VERSION, TaskId, TaskLedgerResourceReceipt,
    TaskLedgerStreamHead, TaskLedgerStreamIdentity, WRITER_LEASE_PRODUCER_ID,
    WRITER_LEASE_PRODUCER_VERSION, WriterLeaseAuthorityReceipt, WriterLeaseIdentity,
    WriterLeaseRevision, WriterLeaseStatus,
};
use lattice_policy::{
    AgentActionGate, AgentRole, ApprovalAuthority, ApprovalFact, ApprovalKind, ApprovalOrigin,
    ApprovalSubject, Boundary, DecisionStage, DecisionSubject, DeploymentIntent, ExecutionGate,
    ExternalCostFact, ExternalCostSubject, GuardianAuthorityFact, GuardianRecoverySubject,
    GuardianRuntimeSubject, GuardianSagaOutcome, GuardianSagaResolution, MemoryCandidateSubject,
    MemoryKind, MemoryPromotionGate, MemoryReviewFact, MergeAnalysisProducer, MergeGate,
    MergeReadinessFact, MergeSubject, MergeTarget, NetworkIntent, NormalRecoveryResolution,
    NormalRecoverySubject, PolicyAction, PolicyDecision, PolicyReason, ProjectAuthorityFact,
    ProtectedActivationReceipt, ProtectedChangeClass, ProtectedChangeGate, ProtectedChangeSubject,
    ProtectedReleaseSubject, ProviderCapabilityFact, ProviderKind, RecoveryAuthorityFact,
    RecoveryGate, RecoveryOwner, RecoverySubject, ReleaseSubject, ResolvedEffectOutcome,
    ResourceObservationSubject, ResourceUsageFact, RollbackSubject, RuntimeAdmission,
    SubjectBinding, TaskContext, UpgradeDelta, UpgradeEvidenceFact, UpgradeGate, UpgradeStage,
    WorkerAdmissionGate, WriterLeaseFact, WriterLeaseSubject, evaluate,
};
use lattice_task_domain::{
    AcceptanceCriterion, ApprovalRequirement, ApprovalRequirements, Capability, CapabilityRequest,
    DeploymentPolicy, EvidenceType, NetworkPolicy, RequiredCheck, RiskClass, RuntimeProfile,
    ScopeOperation, TaskBudget, TaskScope, TaskSpec, TaskSpecInput, TaskState,
};
use lattice_task_ledger::{
    ActionId, ActorId, AppendCommand, CommandId, CorrelationId, EffectClaimId, FakeTaskLedger,
    LedgerEventKind, LedgerOutcome, ReasonCode, ResourceSnapshot,
};

type ProjectMutation = fn(&mut ProjectAuthorityFact);
type UpgradeMutation = for<'a> fn(&mut UpgradeGate<'a>);

#[derive(Clone, Debug, Eq, PartialEq)]
struct ResourceUsage {
    active_agents: u64,
    requested_agents: u64,
    active_implementers: u64,
    requested_implementers: u64,
    elapsed_seconds: u64,
    requested_duration_seconds: u64,
    attempt_number: u64,
    requested_attempts: u64,
    used_model_calls: u64,
    requested_model_calls: u64,
    used_external_cost: String,
    requested_external_cost: Option<String>,
}

#[derive(Clone)]
struct WriterIdentityFixture {
    project_id: ProjectId,
    project_snapshot_id: ProjectSnapshotId,
    task_id: TaskId,
    task_revision: String,
    task_spec_digest: ContentDigest,
    attempt_id: AttemptId,
    lease_id: String,
    lease_holder_id: String,
    worktree_id: String,
    holder_process_id: HolderProcessId,
    holder_process_start_identity: ContentDigest,
    daemon_instance_id: String,
    daemon_epoch: DaemonEpoch,
    fencing_token: FencingToken,
}

#[derive(Clone)]
struct WriterReceiptFixture {
    identity: WriterIdentityFixture,
    runtime: RuntimeKind,
    status: WriterLeaseStatus,
    revision: WriterLeaseRevision,
    runtime_admission: RuntimeAdmissionMode,
    acquired_at: String,
    heartbeat_at: String,
    expires_at: String,
    time_observation_digest: ContentDigest,
    admission_observation_digest: ContentDigest,
    transition_digest: ContentDigest,
    receipt_digest: ContentDigest,
}

#[derive(Clone)]
struct StreamHeadFixture {
    runtime: RuntimeKind,
    identity: TaskLedgerStreamIdentity,
    stream_id: ContentDigest,
    sequence: u64,
    last_event_digest: ContentDigest,
    resource_revision: u64,
    resource_projection_digest: ContentDigest,
    head_digest: ContentDigest,
}

impl StreamHeadFixture {
    fn from_head(head: &TaskLedgerStreamHead) -> Self {
        Self {
            runtime: head.runtime(),
            identity: head.identity().clone(),
            stream_id: head.stream_id().clone(),
            sequence: head.sequence(),
            last_event_digest: head.last_event_digest().clone(),
            resource_revision: head.resource_revision(),
            resource_projection_digest: head.resource_projection_digest().clone(),
            head_digest: head.head_digest().clone(),
        }
    }

    fn build(self) -> TaskLedgerStreamHead {
        TaskLedgerStreamHead::new(
            CONTRACT_VERSION,
            TASK_LEDGER_PRODUCER_ID,
            TASK_LEDGER_PRODUCER_VERSION,
            self.runtime,
            self.identity,
            self.stream_id,
            self.sequence,
            self.last_event_digest,
            self.resource_revision,
            self.resource_projection_digest,
            self.head_digest,
        )
        .expect("valid substituted stream head")
    }
}

#[derive(Clone)]
struct ResourceReceiptFixture {
    runtime: RuntimeKind,
    stream_head: TaskLedgerStreamHead,
    observation_revision: u64,
    effect_claim_id: String,
    effect_subject_digest: ContentDigest,
    counters: ResourceCounters,
    request: ResourceRequest,
    accounting_currency: String,
    observation_digest: ContentDigest,
    receipt_digest: ContentDigest,
}

impl ResourceReceiptFixture {
    fn from_receipt(receipt: &TaskLedgerResourceReceipt) -> Self {
        Self {
            runtime: receipt.runtime(),
            stream_head: receipt.stream_head().clone(),
            observation_revision: receipt.observation_revision(),
            effect_claim_id: receipt.effect_claim_id().to_owned(),
            effect_subject_digest: receipt.effect_subject_digest().clone(),
            counters: receipt.counters().clone(),
            request: receipt.request().clone(),
            accounting_currency: receipt.accounting_currency().to_owned(),
            observation_digest: receipt.observation_digest().clone(),
            receipt_digest: receipt.receipt_digest().clone(),
        }
    }

    fn build(self) -> TaskLedgerResourceReceipt {
        TaskLedgerResourceReceipt::new(
            CONTRACT_VERSION,
            TASK_LEDGER_PRODUCER_ID,
            TASK_LEDGER_PRODUCER_VERSION,
            self.runtime,
            self.stream_head,
            self.observation_revision,
            self.effect_claim_id,
            self.effect_subject_digest,
            self.counters,
            self.request,
            self.accounting_currency,
            self.observation_digest,
            self.receipt_digest,
        )
        .expect("valid substituted resource receipt")
    }
}

const ALL_CAPABILITIES: [Capability; 15] = [
    Capability::ReadRepository,
    Capability::MapCode,
    Capability::PlanTask,
    Capability::WriteProductCode,
    Capability::RunTests,
    Capability::GitWorktree,
    Capability::GitIntegrate,
    Capability::ReadReview,
    Capability::StopRuntime,
    Capability::UseCodex,
    Capability::UseGraphify,
    Capability::UseHermes,
    Capability::ReadCodebaseMemory,
    Capability::ProposeMemory,
    Capability::ProposeUpgrade,
];

#[test]
fn fixed_precedence_rejects_invalid_boundaries_before_project_and_runtime() {
    let spec = task_spec();

    let mut missing_spec = agent_gate(
        &spec,
        AgentRole::Planner,
        PolicyAction::ReadRepository,
        TaskState::Draft,
        RuntimeAdmission::Stopped,
    );
    missing_spec.context.task_spec = None;
    missing_spec.context.project = None;
    missing_spec.role = Boundary::Unknown;
    missing_spec.action = Boundary::Unknown;
    assert_denial(
        evaluate(DecisionSubject::AgentAction(missing_spec)),
        PolicyReason::InvalidDecisionSubject,
        DecisionStage::Input,
    );

    let mut unknown_role = agent_gate(
        &spec,
        AgentRole::Planner,
        PolicyAction::ReadRepository,
        TaskState::Draft,
        RuntimeAdmission::Stopped,
    );
    unknown_role.role = Boundary::Unknown;
    unknown_role
        .context
        .project
        .as_mut()
        .map(set_project_drifted)
        .expect("project");
    assert_denial(
        evaluate(DecisionSubject::AgentAction(unknown_role)),
        PolicyReason::UnknownRole,
        DecisionStage::Input,
    );

    let mut unknown_action = agent_gate(
        &spec,
        AgentRole::Planner,
        PolicyAction::ReadRepository,
        TaskState::Draft,
        RuntimeAdmission::Stopped,
    );
    unknown_action.action = Boundary::Unknown;
    let project = unknown_action.context.project.as_mut().expect("project");
    project.binding = binding_with_project_id(&project.binding, "other-project");
    assert_denial(
        evaluate(DecisionSubject::AgentAction(unknown_action)),
        PolicyReason::UnknownAction,
        DecisionStage::Input,
    );

    let mut unknown_state = agent_gate(
        &spec,
        AgentRole::Planner,
        PolicyAction::ReadRepository,
        TaskState::Draft,
        RuntimeAdmission::Active,
    );
    unknown_state.context.state = Boundary::Unknown;
    unknown_state
        .context
        .project
        .as_mut()
        .map(set_project_suspended)
        .expect("project");
    assert_denial(
        evaluate(DecisionSubject::AgentAction(unknown_state)),
        PolicyReason::UnknownState,
        DecisionStage::Input,
    );

    let mut unknown_runtime = agent_gate(
        &spec,
        AgentRole::Planner,
        PolicyAction::ReadRepository,
        TaskState::Draft,
        RuntimeAdmission::Active,
    );
    unknown_runtime.context.runtime_admission = Boundary::Unknown;
    unknown_runtime.context.project = None;
    assert_denial(
        evaluate(DecisionSubject::AgentAction(unknown_runtime)),
        PolicyReason::UnknownRuntimeAdmission,
        DecisionStage::Input,
    );
}

#[test]
fn fixed_precedence_advances_project_runtime_role_state_protected_and_capability() {
    let spec = task_spec();

    let mut project_first = agent_gate(
        &spec,
        AgentRole::Planner,
        PolicyAction::WriteProductCode,
        TaskState::Draft,
        RuntimeAdmission::Stopped,
    );
    let project = project_first.context.project.as_mut().expect("project");
    project.binding = binding_with_project_id(&project.binding, "other-project");
    assert_denial(
        evaluate(DecisionSubject::AgentAction(project_first)),
        PolicyReason::ProjectIdMismatch,
        DecisionStage::Project,
    );

    let runtime_first = agent_gate(
        &spec,
        AgentRole::Planner,
        PolicyAction::WriteProductCode,
        TaskState::Draft,
        RuntimeAdmission::Stopped,
    );
    assert_denial(
        evaluate(DecisionSubject::AgentAction(runtime_first)),
        PolicyReason::RuntimeAdmissionDenied,
        DecisionStage::Runtime,
    );

    let role_first = agent_gate(
        &spec,
        AgentRole::Planner,
        PolicyAction::WriteProductCode,
        TaskState::Draft,
        RuntimeAdmission::Active,
    );
    assert_denial(
        evaluate(DecisionSubject::AgentAction(role_first)),
        PolicyReason::RoleActionDenied,
        DecisionStage::RoleAction,
    );

    let state_first = agent_gate(
        &spec,
        AgentRole::Implementer,
        PolicyAction::WriteProductCode,
        TaskState::Draft,
        RuntimeAdmission::Active,
    );
    assert_denial(
        evaluate(DecisionSubject::AgentAction(state_first)),
        PolicyReason::ActionStateDenied,
        DecisionStage::State,
    );

    let protected_first = agent_gate(
        &spec,
        AgentRole::UpgradeGuardian,
        PolicyAction::ActivateUpgrade,
        TaskState::Merging,
        RuntimeAdmission::Active,
    );
    assert_denial(
        evaluate(DecisionSubject::AgentAction(protected_first)),
        PolicyReason::ProtectedSurfaceRequired,
        DecisionStage::Protected,
    );

    let missing_codex_capability = spec_with(|input| {
        input
            .requested_capabilities
            .retain(|request| request.capability != Capability::UseCodex);
        input.runtime_profile = RuntimeProfile::Codex;
    });
    let capability_first = agent_gate(
        &missing_codex_capability,
        AgentRole::Implementer,
        PolicyAction::RunCodex,
        TaskState::Executing,
        RuntimeAdmission::Active,
    );
    assert_denial(
        evaluate(DecisionSubject::AgentAction(capability_first)),
        PolicyReason::CapabilityNotRequested,
        DecisionStage::RequestedCapability,
    );
}

#[test]
fn runtime_admission_conversion_is_exhaustive_and_wire_compatible() {
    for contract_mode in RuntimeAdmissionMode::ALL {
        let Boundary::Known(policy_mode) = RuntimeAdmission::parse(contract_mode.as_str()) else {
            panic!(
                "contract runtime admission {} must remain a known Policy value",
                contract_mode.as_str()
            );
        };
        assert_eq!(RuntimeAdmissionMode::from(policy_mode), contract_mode);
    }

    assert_eq!(RuntimeAdmission::parse("UNKNOWN"), Boundary::Unknown);
}

#[test]
fn project_binding_substitution_matrix_fails_closed() {
    let spec = task_spec();

    let cases: &[(ProjectMutation, PolicyReason)] = &[
        (set_project_suspended, PolicyReason::ProjectInactive),
        (set_project_drifted, PolicyReason::ProjectDrifted),
        (set_project_stale, PolicyReason::ProjectAuthorityStale),
        (
            |project| {
                project.binding = binding_with_project_id(&project.binding, "other-project");
            },
            PolicyReason::ProjectIdMismatch,
        ),
        (
            |project| {
                project.binding = binding_with_snapshot(
                    &project.binding,
                    ProjectSnapshotId::new("other-snapshot").expect("snapshot"),
                );
            },
            PolicyReason::ProjectSnapshotMismatch,
        ),
        (
            |project| {
                project.binding = binding_with_task(
                    &project.binding,
                    TaskId::new("TASK-OTHER-011").expect("task"),
                );
            },
            PolicyReason::TaskIdMismatch,
        ),
        (
            |project| {
                project.binding = binding_with_revision(&project.binding, "2");
            },
            PolicyReason::TaskRevisionMismatch,
        ),
        (
            |project| {
                project.binding = binding_with_digest(&project.binding, digest('f'));
            },
            PolicyReason::TaskSpecHashMismatch,
        ),
    ];

    for (mutate, expected) in cases {
        let mut gate = agent_gate(
            &spec,
            AgentRole::Planner,
            PolicyAction::ReadRepository,
            TaskState::Draft,
            RuntimeAdmission::Active,
        );
        mutate(gate.context.project.as_mut().expect("project"));
        assert_denial(
            evaluate(DecisionSubject::AgentAction(gate)),
            *expected,
            DecisionStage::Project,
        );
    }

    let mut absent = agent_gate(
        &spec,
        AgentRole::Planner,
        PolicyAction::ReadRepository,
        TaskState::Draft,
        RuntimeAdmission::Active,
    );
    absent.context.project = None;
    assert_denial(
        evaluate(DecisionSubject::AgentAction(absent)),
        PolicyReason::ProjectNotRegistered,
        DecisionStage::Project,
    );
}

#[test]
fn exhaustive_role_action_state_matrix_stays_closed() {
    let spec = task_spec();

    for role in AgentRole::ALL {
        for action in PolicyAction::ALL {
            for state in TaskState::ALL {
                let decision = evaluate(DecisionSubject::AgentAction(agent_gate(
                    &spec,
                    role,
                    action,
                    state,
                    RuntimeAdmission::Active,
                )));
                if !role_allows(role, action) {
                    assert_denial(
                        decision,
                        PolicyReason::RoleActionDenied,
                        DecisionStage::RoleAction,
                    );
                } else if !state_allows(state, action) {
                    assert_denial(
                        decision,
                        PolicyReason::ActionStateDenied,
                        DecisionStage::State,
                    );
                } else if protected_action(action) {
                    assert_denial(
                        decision,
                        PolicyReason::ProtectedSurfaceRequired,
                        DecisionStage::Protected,
                    );
                } else {
                    assert!(
                        decision.evidence().checked_through() > DecisionStage::State,
                        "{role:?} {action:?} {state:?} stopped at {:?} with {:?}",
                        decision.evidence().checked_through(),
                        decision.reason()
                    );
                }
            }
        }
    }
}

#[test]
fn v1_unsafe_and_project_specific_actions_are_not_v2_actions() {
    for action in [
        "RESOLVE_MERGE_CONFLICT",
        "CALL_REAL_MODEL",
        "NETWORK_ACCESS",
        "DEPLOY_PRODUCTION",
        "PURCHASE_SERVICE",
        "MANAGE_CREDENTIALS",
        "PUBLIC_PUBLISH",
        "PERMANENT_DELETE",
        "ACCESS_PLAYMATE",
        "DISABLE_SECURITY",
    ] {
        assert_eq!(
            PolicyAction::parse(action),
            Boundary::Unknown,
            "{action} must not enter the active V2 action set"
        );
    }
    assert_eq!(AgentRole::parse("GRAPHIFY"), Boundary::Unknown);
    assert_eq!(AgentRole::parse("HERMES"), Boundary::Unknown);
}

#[test]
fn provider_capability_requires_exact_current_identity_and_runtime() {
    let spec = spec_with(|input| input.runtime_profile = RuntimeProfile::Codex);
    let mut gate = codex_gate(&spec);
    gate.provider_capability = None;
    assert_denial(
        evaluate(DecisionSubject::AgentAction(gate)),
        PolicyReason::CapabilityEvidenceMissing,
        DecisionStage::ProviderCapability,
    );

    let mut stale = codex_gate(&spec);
    stale.provider_capability.as_mut().expect("provider").fresh = false;
    assert_denial(
        evaluate(DecisionSubject::AgentAction(stale)),
        PolicyReason::CapabilityEvidenceStale,
        DecisionStage::ProviderCapability,
    );

    let mut contract = codex_gate(&spec);
    contract
        .provider_capability
        .as_mut()
        .expect("provider")
        .contract_version = 2;
    assert_denial(
        evaluate(DecisionSubject::AgentAction(contract)),
        PolicyReason::CapabilityContractMismatch,
        DecisionStage::ProviderCapability,
    );

    let mut identity = codex_gate(&spec);
    identity
        .provider_capability
        .as_mut()
        .expect("provider")
        .observed_executable_digest = digest('e');
    assert_denial(
        evaluate(DecisionSubject::AgentAction(identity)),
        PolicyReason::CapabilityIdentityMismatch,
        DecisionStage::ProviderCapability,
    );

    let mut boundary = codex_gate(&spec);
    boundary
        .provider_capability
        .as_mut()
        .expect("provider")
        .boundary_verified = false;
    assert_denial(
        evaluate(DecisionSubject::AgentAction(boundary)),
        PolicyReason::ProviderBoundaryDenied,
        DecisionStage::ProviderCapability,
    );

    let mut wrong_runtime = codex_gate(&spec);
    wrong_runtime
        .provider_capability
        .as_mut()
        .expect("provider")
        .runtime = RuntimeKind::Fake;
    assert_denial(
        evaluate(DecisionSubject::AgentAction(wrong_runtime)),
        PolicyReason::RuntimeKindMismatch,
        DecisionStage::ProviderCapability,
    );

    let valid = evaluate(DecisionSubject::AgentAction(codex_gate(&spec)));
    assert!(
        valid.allowed(),
        "exact live Codex fact should pass: {:?}",
        valid.reason()
    );
}

#[test]
fn provider_fact_is_bound_to_the_exact_task_and_expected_provider() {
    let spec = spec_with(|input| input.runtime_profile = RuntimeProfile::Codex);

    let mut cross_project = codex_gate(&spec);
    let provider = cross_project
        .provider_capability
        .as_mut()
        .expect("provider");
    provider.binding = binding_with_project_id(&provider.binding, "other-project");
    assert_denied_at(
        evaluate(DecisionSubject::AgentAction(cross_project)),
        DecisionStage::ProviderCapability,
    );

    let mut wrong_provider = codex_gate(&spec);
    wrong_provider
        .provider_capability
        .as_mut()
        .expect("provider")
        .provider = ProviderKind::Hermes;
    assert_denied_at(
        evaluate(DecisionSubject::AgentAction(wrong_provider)),
        DecisionStage::ProviderCapability,
    );

    let graph_spec = task_spec();
    let mut graph = agent_gate(
        &graph_spec,
        AgentRole::CodeMapper,
        PolicyAction::RunGraphify,
        TaskState::Draft,
        RuntimeAdmission::Active,
    );
    graph.provider_capability = Some(provider_fact(
        &graph_spec,
        ProviderKind::Graphify,
        Capability::UseGraphify,
        RuntimeKind::Fake,
    ));
    assert!(
        evaluate(DecisionSubject::AgentAction(graph)).allowed(),
        "exact Graphify capability should be usable"
    );

    let mut hermes = agent_gate(
        &graph_spec,
        AgentRole::Researcher,
        PolicyAction::RunHermes,
        TaskState::Draft,
        RuntimeAdmission::Active,
    );
    hermes.provider_capability = Some(provider_fact(
        &graph_spec,
        ProviderKind::Hermes,
        Capability::UseHermes,
        RuntimeKind::Fake,
    ));
    assert!(
        evaluate(DecisionSubject::AgentAction(hermes)).allowed(),
        "exact Hermes capability should be usable"
    );
}

#[test]
fn execution_risk_and_task_requirements_can_only_raise_approval_floor() {
    let cases = [
        (RiskClass::R0, ApprovalRequirement::NotRequired, None, true),
        (
            RiskClass::R1,
            ApprovalRequirement::NotRequired,
            Some((ApprovalAuthority::InternalPolicy, false)),
            true,
        ),
        (
            RiskClass::R2,
            ApprovalRequirement::NotRequired,
            Some((ApprovalAuthority::ResponsibleUser, false)),
            true,
        ),
        (
            RiskClass::R3,
            ApprovalRequirement::NotRequired,
            Some((ApprovalAuthority::ResponsibleUser, true)),
            false,
        ),
        (
            RiskClass::R0,
            ApprovalRequirement::ResponsibleUser,
            Some((ApprovalAuthority::InternalPolicy, false)),
            false,
        ),
        (
            RiskClass::R0,
            ApprovalRequirement::ResponsibleUser,
            Some((ApprovalAuthority::ResponsibleUser, false)),
            true,
        ),
        (
            RiskClass::R2,
            ApprovalRequirement::Policy,
            Some((ApprovalAuthority::InternalPolicy, false)),
            false,
        ),
        (
            RiskClass::R3,
            ApprovalRequirement::ResponsibleUser,
            Some((ApprovalAuthority::ResponsibleUser, false)),
            false,
        ),
    ];

    for (risk, requirement, supplied, should_allow) in cases {
        let spec = spec_with(|input| {
            input.risk_class = risk;
            input.approval_requirements.execution = requirement;
        });
        let approval = supplied.and_then(|(authority, _checks)| {
            (authority != ApprovalAuthority::InternalPolicy).then(|| {
                approval_fact(
                    &spec,
                    ApprovalKind::Execution,
                    authority,
                    execution_subject(&spec, None),
                )
            })
        });
        let decision = evaluate(DecisionSubject::ExecutionGate(ExecutionGate {
            context: context(
                &spec,
                TaskState::AwaitingExecutionApproval,
                RuntimeAdmission::Active,
            ),
            approval,
        }));
        assert_eq!(
            decision.allowed(),
            should_allow,
            "risk={risk:?} requirement={requirement:?}: {:?}",
            decision.reason()
        );
    }
}

#[test]
fn actual_fake_approval_owner_receipt_and_current_head_allow_r2_execution() {
    let spec = spec_with(|input| {
        input.risk_class = RiskClass::R2;
        input.approval_requirements.execution = ApprovalRequirement::ResponsibleUser;
    });
    let identity = ApprovalIdentity::new(
        "approval-policy-integration",
        "challenge-policy-integration",
        binding(&spec),
        execution_subject(&spec, None),
        "requester-policy-integration",
        "responsible-actor-policy-integration",
        ApprovalAuthority::ResponsibleUser,
        ApprovalOrigin::OsAuthenticatedUser,
        ApprovalLane::Normal,
        "local-os-channel",
        "session-policy-integration",
    )
    .expect("approval identity");
    let signer = FakeNormalSigner::new(
        "responsible-actor-policy-integration",
        "fake-os-authenticator-policy-integration",
        "fake-key-policy-integration",
        SecretMaterial::new(b"fake-policy-signer-secret".to_vec()).expect("signer secret"),
    )
    .expect("fake signer");
    let nonce = SecretMaterial::new(b"fake-policy-nonce-secret".to_vec()).expect("nonce secret");
    let mut verifier = FakeApprovalVerifier::new();

    let issue_receipt = verifier
        .issue(IssueApprovalCommand {
            command_id: "command-policy-issue".to_owned(),
            expected_head: None,
            runtime: RuntimeKind::Fake,
            identity,
            nonce_id: "nonce-policy-integration".to_owned(),
            nonce_commitment: nonce_commitment(&nonce).expect("nonce commitment"),
            issued_at: "2026-07-29T08:00:00Z".to_owned(),
            expires_at: "2026-07-29T09:00:00Z".to_owned(),
            authenticator_id: signer.authenticator_id().to_owned(),
            key_id: signer.key_id().to_owned(),
            verification_key_commitment: signer.verification_key_commitment().clone(),
            evidence_digest: signer.evidence_digest().clone(),
            review_set_digest: None,
        })
        .expect("issue approval");
    let proof = signer
        .sign(issue_receipt.challenge.as_ref().expect("challenge"))
        .expect("fake proof");
    let verify_receipt = verifier
        .verify(VerifyApprovalCommand {
            command_id: "command-policy-verify".to_owned(),
            approval_id: "approval-policy-integration".to_owned(),
            expected_head: issue_receipt.after.expect("challenge head"),
            observed_at: "2026-07-29T08:01:00Z".to_owned(),
            proof,
        })
        .expect("verify approval");
    let receipt = verify_receipt
        .authority_receipt
        .expect("approval authority receipt");
    let current_head = verifier
        .current_head_at("approval-policy-integration", "2026-07-29T08:02:00Z")
        .expect("current approval head");

    let decision = evaluate(DecisionSubject::ExecutionGate(ExecutionGate {
        context: context(
            &spec,
            TaskState::AwaitingExecutionApproval,
            RuntimeAdmission::Active,
        ),
        approval: Some(ApprovalFact {
            receipt,
            current_head,
        }),
    }));

    assert!(decision.allowed(), "{:?}", decision.reason());
}

#[test]
fn r3_requires_both_independent_checks_and_exact_responsible_user_authority() {
    let spec = spec_with(|input| input.risk_class = RiskClass::R3);

    let approval = approval_fact(
        &spec,
        ApprovalKind::Execution,
        ApprovalAuthority::ResponsibleUser,
        execution_subject(&spec, None),
    );
    assert_denial(
        evaluate(DecisionSubject::ExecutionGate(ExecutionGate {
            context: context(
                &spec,
                TaskState::AwaitingExecutionApproval,
                RuntimeAdmission::Active,
            ),
            approval: Some(approval),
        })),
        PolicyReason::ReviewAuthorityUnavailable,
        DecisionStage::Approval,
    );
}

#[test]
fn independent_review_required_by_checks_cannot_bypass_missing_review_authority() {
    let spec = spec_with(|input| {
        input.risk_class = RiskClass::R0;
        input.required_checks = vec![RequiredCheck::Security, RequiredCheck::Architecture];
        input.approval_requirements.execution = ApprovalRequirement::NotRequired;
    });

    assert_denial(
        evaluate(DecisionSubject::ExecutionGate(ExecutionGate {
            context: context(
                &spec,
                TaskState::AwaitingExecutionApproval,
                RuntimeAdmission::Active,
            ),
            approval: None,
        })),
        PolicyReason::ReviewAuthorityUnavailable,
        DecisionStage::Approval,
    );
}

#[test]
fn independent_review_required_fact_memory_cannot_bypass_missing_review_authority() {
    let spec = spec_with(|input| {
        input.risk_class = RiskClass::R0;
        input.required_checks = vec![RequiredCheck::Security, RequiredCheck::Architecture];
    });

    assert_denial(
        evaluate(DecisionSubject::MemoryPromotion(memory_gate(
            &spec,
            MemoryKind::Fact,
        ))),
        PolicyReason::ReviewAuthorityUnavailable,
        DecisionStage::Approval,
    );
}

#[test]
fn approval_substitution_replay_staleness_and_trust_lane_deny() {
    let spec = spec_with(|input| {
        input.risk_class = RiskClass::R2;
        input.approval_requirements.execution = ApprovalRequirement::ResponsibleUser;
    });

    for (approval, expected) in approval_denial_cases(&spec) {
        assert_denial(
            evaluate(DecisionSubject::ExecutionGate(ExecutionGate {
                context: context(
                    &spec,
                    TaskState::AwaitingExecutionApproval,
                    RuntimeAdmission::Active,
                ),
                approval: Some(approval),
            })),
            expected,
            DecisionStage::Approval,
        );
    }

    assert_denial(
        evaluate(DecisionSubject::ExecutionGate(ExecutionGate {
            context: context(
                &spec,
                TaskState::AwaitingExecutionApproval,
                RuntimeAdmission::Active,
            ),
            approval: None,
        })),
        PolicyReason::ApprovalMissing,
        DecisionStage::Approval,
    );
}

fn approval_denial_cases(spec: &TaskSpec) -> Vec<(ApprovalFact, PolicyReason)> {
    let wrong_kind = approval_fact(
        spec,
        ApprovalKind::Merge,
        ApprovalAuthority::ResponsibleUser,
        ApprovalSubject::Merge(
            MergeSubject::new(
                MergeTarget::FeatureBranch("refs/heads/integration".to_owned()),
                "d".repeat(40),
                "c".repeat(40),
                digest('e'),
            )
            .expect("merge subject"),
        ),
    );
    let cross_project = approval_fact_with_binding(
        spec,
        binding_with_project_id(&binding(spec), "other-project"),
        ApprovalKind::Execution,
        ApprovalAuthority::ResponsibleUser,
        execution_subject(spec, None),
    );
    let wrong_subject = approval_fact_with_binding(
        spec,
        binding_with_digest(&binding(spec), digest('e')),
        ApprovalKind::Execution,
        ApprovalAuthority::ResponsibleUser,
        ApprovalSubject::Execution {
            task_spec_hash: digest('e'),
            external_cost: None,
        },
    );
    let wrong_authority = approval_fact(
        spec,
        ApprovalKind::Execution,
        ApprovalAuthority::ProtectedGuardian,
        execution_subject(spec, None),
    );

    let mut stale = approval_fact(
        spec,
        ApprovalKind::Execution,
        ApprovalAuthority::ResponsibleUser,
        execution_subject(spec, None),
    );
    stale.current_head = None;

    let mut replayed = approval_fact(
        spec,
        ApprovalKind::Execution,
        ApprovalAuthority::ResponsibleUser,
        execution_subject(spec, None),
    );
    replayed.current_head = Some(approval_head_with_status(
        &replayed.receipt,
        ApprovalStatus::ClaimedNormal,
    ));

    let mut revoked = approval_fact(
        spec,
        ApprovalKind::Execution,
        ApprovalAuthority::ResponsibleUser,
        execution_subject(spec, None),
    );
    revoked.current_head = Some(approval_head_with_status(
        &revoked.receipt,
        ApprovalStatus::Revoked,
    ));

    vec![
        (wrong_kind, PolicyReason::ApprovalKindMismatch),
        (cross_project, PolicyReason::ApprovalSubjectMismatch),
        (wrong_subject, PolicyReason::ApprovalSubjectMismatch),
        (wrong_authority, PolicyReason::ApprovalAuthorityDenied),
        (stale, PolicyReason::ApprovalStale),
        (replayed, PolicyReason::ApprovalReplayed),
        (revoked, PolicyReason::ApprovalStale),
    ]
}

#[test]
fn missing_spec_never_inherits_partial_v1_approval_or_writer_fields() {
    let spec = task_spec();

    let mut execution = ExecutionGate {
        context: context(
            &spec,
            TaskState::AwaitingExecutionApproval,
            RuntimeAdmission::Active,
        ),
        approval: Some(approval_fact(
            &spec,
            ApprovalKind::Execution,
            ApprovalAuthority::ResponsibleUser,
            execution_subject(&spec, None),
        )),
    };
    execution.context.task_spec = None;
    assert_denial(
        evaluate(DecisionSubject::ExecutionGate(execution)),
        PolicyReason::InvalidDecisionSubject,
        DecisionStage::Input,
    );

    let mut write = write_gate(&spec);
    write.context.task_spec = None;
    assert_denial(
        evaluate(DecisionSubject::AgentAction(write)),
        PolicyReason::InvalidDecisionSubject,
        DecisionStage::Input,
    );
}

#[test]
fn product_write_requires_one_exact_current_implementer_writer() {
    let spec = task_spec();

    let mut missing = write_gate(&spec);
    missing.writer = None;
    assert_denial(
        evaluate(DecisionSubject::AgentAction(missing)),
        PolicyReason::WriterLeaseRequired,
        DecisionStage::Writer,
    );

    let mut no_current_head = write_gate(&spec);
    no_current_head
        .writer
        .as_mut()
        .expect("writer")
        .current_head = None;
    assert_denial(
        evaluate(DecisionSubject::AgentAction(no_current_head)),
        PolicyReason::WriterLeaseNotCurrent,
        DecisionStage::Writer,
    );

    let mut suspect = write_gate(&spec);
    let mut suspect_fixture = writer_receipt_fixture(&spec);
    suspect_fixture.status = WriterLeaseStatus::Suspect;
    suspect.writer = Some(writer_fact_from_fixture(&suspect_fixture));
    suspect.writer_subject = Some(writer_subject_from_fact(
        suspect.writer.as_ref().expect("writer"),
    ));
    assert_denial(
        evaluate(DecisionSubject::AgentAction(suspect)),
        PolicyReason::WriterLeaseNotCurrent,
        DecisionStage::Writer,
    );

    let mut actor_mismatch = write_gate(&spec);
    actor_mismatch
        .writer_subject
        .as_mut()
        .expect("writer subject")
        .lease_holder_id = "another-actor".to_owned();
    assert_denial(
        evaluate(DecisionSubject::AgentAction(actor_mismatch)),
        PolicyReason::WriterLeaseSubjectMismatch,
        DecisionStage::Writer,
    );

    assert_eq!(
        PolicyReason::MultipleImplementers.code(),
        "MULTIPLE_IMPLEMENTERS",
        "legacy reason remains stable although Writer Lease now rejects a second owner"
    );

    let valid = evaluate(DecisionSubject::AgentAction(write_gate(&spec)));
    assert!(
        valid.allowed(),
        "exact current writer should pass: {:?}",
        valid.reason()
    );

    let mut non_implementer = write_gate(&spec);
    non_implementer.role = Boundary::Known(AgentRole::Integrator);
    assert_denial(
        evaluate(DecisionSubject::AgentAction(non_implementer)),
        PolicyReason::RoleActionDenied,
        DecisionStage::RoleAction,
    );
}

#[test]
fn runtime_admission_allows_only_bounded_recovery_work() {
    let spec = task_spec();

    let normal_during_drain = agent_gate(
        &spec,
        AgentRole::Implementer,
        PolicyAction::WriteProductCode,
        TaskState::Executing,
        RuntimeAdmission::Draining,
    );
    assert_denial(
        evaluate(DecisionSubject::AgentAction(normal_during_drain)),
        PolicyReason::RuntimeAdmissionDenied,
        DecisionStage::Runtime,
    );

    let mut stop = agent_gate(
        &spec,
        AgentRole::LatticePm,
        PolicyAction::StopRuntime,
        TaskState::Executing,
        RuntimeAdmission::Draining,
    );
    replace_agent_resources(
        &mut stop,
        resource_fact(
            &spec,
            ResourceUsage {
                elapsed_seconds: u64::MAX,
                requested_duration_seconds: 1,
                ..safe_usage()
            },
        ),
    );
    let stop_decision = evaluate(DecisionSubject::AgentAction(stop));
    assert!(
        stop_decision.allowed(),
        "budget exhaustion must not prevent bounded stop: {:?}",
        stop_decision.reason()
    );

    let canary_read = agent_gate(
        &spec,
        AgentRole::Planner,
        PolicyAction::ReadRepository,
        TaskState::Draft,
        RuntimeAdmission::Canary,
    );
    assert_denial(
        evaluate(DecisionSubject::AgentAction(canary_read)),
        PolicyReason::RuntimeAdmissionDenied,
        DecisionStage::Runtime,
    );

    let mut canary_health = activation_gate(&spec);
    canary_health.stage = UpgradeStage::HealthCanary;
    canary_health.context.runtime_admission = Boundary::Known(RuntimeAdmission::Canary);
    assert!(
        evaluate(DecisionSubject::UpgradeStage(canary_health)).allowed(),
        "CANARY must admit guardian health through the specialized upgrade gate"
    );

    for admission in [
        RuntimeAdmission::Stopped,
        RuntimeAdmission::ReconciliationRequired,
    ] {
        let normal = agent_gate(
            &spec,
            AgentRole::Planner,
            PolicyAction::ReadRepository,
            TaskState::Draft,
            admission,
        );
        assert_denial(
            evaluate(DecisionSubject::AgentAction(normal)),
            PolicyReason::RuntimeAdmissionDenied,
            DecisionStage::Runtime,
        );
    }
}

#[test]
#[allow(clippy::too_many_lines)]
fn worker_admission_uses_checked_budget_arithmetic_and_one_implementer() {
    let spec = task_spec();

    let exact_limit = worker_gate(
        &spec,
        vec![
            AgentRole::Planner,
            AgentRole::CodeMapper,
            AgentRole::Implementer,
            AgentRole::SecurityReviewer,
        ],
        ResourceUsage {
            active_agents: 0,
            requested_agents: 4,
            active_implementers: 0,
            requested_implementers: 1,
            elapsed_seconds: 1_700,
            requested_duration_seconds: 100,
            attempt_number: 1,
            requested_attempts: 1,
            used_model_calls: 0,
            requested_model_calls: 0,
            used_external_cost: "0".to_owned(),
            requested_external_cost: Some("0".to_owned()),
        },
    );
    assert!(
        evaluate(DecisionSubject::WorkerAdmission(exact_limit)).allowed(),
        "equal-to-budget usage must pass"
    );

    let over_agents = worker_gate(
        &spec,
        vec![
            AgentRole::Planner,
            AgentRole::CodeMapper,
            AgentRole::Implementer,
            AgentRole::SecurityReviewer,
            AgentRole::ArchitectureReviewer,
        ],
        ResourceUsage {
            active_agents: 0,
            requested_agents: 5,
            active_implementers: 0,
            requested_implementers: 1,
            ..safe_usage()
        },
    );
    assert_denial(
        evaluate(DecisionSubject::WorkerAdmission(over_agents)),
        PolicyReason::AgentLimitExceeded,
        DecisionStage::Resources,
    );

    let two_implementers = worker_gate(
        &spec,
        vec![AgentRole::Implementer, AgentRole::Implementer],
        ResourceUsage {
            active_agents: 0,
            requested_agents: 2,
            active_implementers: 0,
            requested_implementers: 2,
            ..safe_usage()
        },
    );
    assert_denial(
        evaluate(DecisionSubject::WorkerAdmission(two_implementers)),
        PolicyReason::MultipleImplementers,
        DecisionStage::Resources,
    );

    let overflow_spec = spec_with(|input| {
        input.budget.max_duration_seconds = u64::MAX.to_string();
    });
    let overflow = worker_gate(
        &overflow_spec,
        vec![AgentRole::Planner],
        ResourceUsage {
            active_agents: 0,
            requested_agents: 1,
            elapsed_seconds: u64::MAX,
            requested_duration_seconds: 1,
            ..safe_usage()
        },
    );
    assert_denial(
        evaluate(DecisionSubject::WorkerAdmission(overflow)),
        PolicyReason::ResourceEvidenceInvalid,
        DecisionStage::Resources,
    );
}

#[test]
fn every_resource_boundary_denies_one_above_its_task_budget() {
    let spec = spec_with(|input| {
        input.budget.max_model_calls = "3".to_owned();
        input.budget.max_external_cost = "1.1".to_owned();
    });

    let cases: Vec<(ResourceUsage, PolicyReason)> = vec![
        (
            ResourceUsage {
                elapsed_seconds: 1_800,
                requested_duration_seconds: 1,
                ..safe_usage()
            },
            PolicyReason::DurationBudgetExceeded,
        ),
        (
            ResourceUsage {
                attempt_number: 2,
                requested_attempts: 1,
                ..safe_usage()
            },
            PolicyReason::AttemptBudgetExceeded,
        ),
        (
            ResourceUsage {
                used_model_calls: 3,
                requested_model_calls: 1,
                ..safe_usage()
            },
            PolicyReason::ModelCallBudgetExceeded,
        ),
        (
            ResourceUsage {
                used_external_cost: "0.9".to_owned(),
                requested_external_cost: Some("0.21".to_owned()),
                ..safe_usage()
            },
            PolicyReason::ExternalCostBudgetExceeded,
        ),
    ];

    for (resources, reason) in cases {
        let requested_cost = resources
            .requested_external_cost
            .clone()
            .expect("requested cost");
        let mut gate = graphify_gate(&spec);
        replace_agent_resources(&mut gate, resource_fact(&spec, resources));
        if requested_cost != "0" {
            attach_external_cost(&spec, &mut gate, &requested_cost, "graphify-local");
        }
        assert_denial(
            evaluate(DecisionSubject::AgentAction(gate)),
            reason,
            DecisionStage::Resources,
        );
    }
}

#[test]
fn decimal_external_cost_is_exact_canonical_and_never_float_based() {
    let spec = spec_with(|input| input.budget.max_external_cost = "1.1".to_owned());

    let mut equal = graphify_gate(&spec);
    replace_agent_resources(
        &mut equal,
        resource_fact(
            &spec,
            ResourceUsage {
                used_external_cost: "0.9".to_owned(),
                requested_external_cost: Some("0.2".to_owned()),
                ..safe_usage()
            },
        ),
    );
    attach_external_cost(&spec, &mut equal, "0.2", "graphify-local");
    assert!(
        evaluate(DecisionSubject::AgentAction(equal)).allowed(),
        "0.9 + 0.2 must equal canonical 1.1 exactly"
    );

    for malformed in ["", "00", "01", ".1", "1.", "1.0", "1e1", "-1", "+1"] {
        assert!(
            ResourceCounters::new(1, 0, 0, 1, 0, malformed).is_err(),
            "Task Ledger contract must reject non-canonical cost {malformed:?}"
        );
    }
}

#[test]
#[allow(clippy::too_many_lines)]
fn network_deployment_and_new_cost_envelopes_fail_closed() {
    let deny_spec = task_spec();
    let mut network_denied = agent_gate(
        &deny_spec,
        AgentRole::Planner,
        PolicyAction::ReadRepository,
        TaskState::Draft,
        RuntimeAdmission::Active,
    );
    network_denied.network = NetworkIntent::Loopback;
    assert_denial(
        evaluate(DecisionSubject::AgentAction(network_denied)),
        PolicyReason::NetworkDenied,
        DecisionStage::EffectEnvelope,
    );

    let loopback_spec = spec_with(|input| input.network_policy = NetworkPolicy::LoopbackOnly);
    let mut external = agent_gate(
        &loopback_spec,
        AgentRole::Planner,
        PolicyAction::ReadRepository,
        TaskState::Draft,
        RuntimeAdmission::Active,
    );
    external.network = NetworkIntent::External {
        target_digest: digest('a'),
        allowlist_digest: None,
    };
    assert_denial(
        evaluate(DecisionSubject::AgentAction(external)),
        PolicyReason::NetworkTargetMismatch,
        DecisionStage::EffectEnvelope,
    );

    let allowlisted_spec = spec_with(|input| input.network_policy = NetworkPolicy::Allowlisted);
    let mut unbound_allowlist = agent_gate(
        &allowlisted_spec,
        AgentRole::Planner,
        PolicyAction::ReadRepository,
        TaskState::Draft,
        RuntimeAdmission::Active,
    );
    unbound_allowlist.network = NetworkIntent::External {
        target_digest: digest('a'),
        allowlist_digest: None,
    };
    assert_denial(
        evaluate(DecisionSubject::AgentAction(unbound_allowlist)),
        PolicyReason::NetworkAllowlistUnbound,
        DecisionStage::EffectEnvelope,
    );

    let mut deployment_denied = agent_gate(
        &deny_spec,
        AgentRole::Planner,
        PolicyAction::ReadRepository,
        TaskState::Draft,
        RuntimeAdmission::Active,
    );
    deployment_denied.deployment = DeploymentIntent::PrepareArtifact;
    assert_denial(
        evaluate(DecisionSubject::AgentAction(deployment_denied)),
        PolicyReason::DeploymentDenied,
        DecisionStage::EffectEnvelope,
    );

    let prepare_spec = spec_with(|input| input.deployment_policy = DeploymentPolicy::PrepareOnly);
    let mut deploy = agent_gate(
        &prepare_spec,
        AgentRole::Planner,
        PolicyAction::ReadRepository,
        TaskState::Draft,
        RuntimeAdmission::Active,
    );
    deploy.deployment = DeploymentIntent::Deploy;
    assert_denial(
        evaluate(DecisionSubject::AgentAction(deploy)),
        PolicyReason::DeploymentPrepareOnly,
        DecisionStage::EffectEnvelope,
    );

    let cost_spec = spec_with(|input| input.budget.max_external_cost = "1".to_owned());
    let mut unknown_cost = agent_gate(
        &cost_spec,
        AgentRole::Planner,
        PolicyAction::ReadRepository,
        TaskState::Draft,
        RuntimeAdmission::Active,
    );
    replace_agent_resources(
        &mut unknown_cost,
        resource_fact(
            &cost_spec,
            ResourceUsage {
                requested_external_cost: None,
                ..safe_usage()
            },
        ),
    );
    assert_denial(
        evaluate(DecisionSubject::AgentAction(unknown_cost)),
        PolicyReason::ExternalCostUnknown,
        DecisionStage::EffectEnvelope,
    );

    let mut unapproved_cost = agent_gate(
        &cost_spec,
        AgentRole::Planner,
        PolicyAction::ReadRepository,
        TaskState::Draft,
        RuntimeAdmission::Active,
    );
    replace_agent_resources(
        &mut unapproved_cost,
        resource_fact(
            &cost_spec,
            ResourceUsage {
                requested_external_cost: Some("0.1".to_owned()),
                ..safe_usage()
            },
        ),
    );
    assert_denial(
        evaluate(DecisionSubject::AgentAction(unapproved_cost)),
        PolicyReason::ExternalCostUnknown,
        DecisionStage::EffectEnvelope,
    );

    let mut quoted_non_provider_cost = agent_gate(
        &cost_spec,
        AgentRole::Planner,
        PolicyAction::ReadRepository,
        TaskState::Draft,
        RuntimeAdmission::Active,
    );
    replace_agent_resources(
        &mut quoted_non_provider_cost,
        resource_fact(
            &cost_spec,
            ResourceUsage {
                requested_external_cost: Some("0.1".to_owned()),
                ..safe_usage()
            },
        ),
    );
    attach_external_cost(
        &cost_spec,
        &mut quoted_non_provider_cost,
        "0.1",
        "graphify-local",
    );
    assert_denial(
        evaluate(DecisionSubject::AgentAction(quoted_non_provider_cost)),
        PolicyReason::ExternalCostUnknown,
        DecisionStage::EffectEnvelope,
    );
}

#[test]
fn merge_conflict_and_primary_branch_subjects_do_not_reuse_v1_approval() {
    let spec = task_spec();

    let mut conflict = merge_gate(
        &spec,
        MergeTarget::FeatureBranch("refs/heads/integration".to_owned()),
    );
    conflict
        .readiness
        .as_mut()
        .expect("readiness")
        .conflict_free = false;
    assert_denial(
        evaluate(DecisionSubject::MergeGate(conflict)),
        PolicyReason::MergeConflictRequiresImplementer,
        DecisionStage::Protected,
    );

    let mut primary = merge_gate(
        &spec,
        MergeTarget::PrimaryBranch("refs/heads/main".to_owned()),
    );
    primary.approval = None;
    assert_denial(
        evaluate(DecisionSubject::MergeGate(primary)),
        PolicyReason::PrimaryBranchApprovalRequired,
        DecisionStage::Approval,
    );

    let mut exact = merge_gate(
        &spec,
        MergeTarget::PrimaryBranch("refs/heads/main".to_owned()),
    );
    exact.approval = Some(approval_fact(
        &spec,
        ApprovalKind::Merge,
        ApprovalAuthority::ResponsibleUser,
        ApprovalSubject::Merge(exact.subject.clone()),
    ));
    assert!(
        evaluate(DecisionSubject::MergeGate(exact)).allowed(),
        "exact primary-branch approval should pass the pure decision gate"
    );

    let mut cross_project = merge_gate(
        &spec,
        MergeTarget::PrimaryBranch("refs/heads/main".to_owned()),
    );
    let approval = approval_fact_with_binding(
        &spec,
        binding_with_project_id(&binding(&spec), "other-project"),
        ApprovalKind::Merge,
        ApprovalAuthority::ResponsibleUser,
        ApprovalSubject::Merge(cross_project.subject.clone()),
    );
    cross_project.approval = Some(approval);
    assert_denial(
        evaluate(DecisionSubject::MergeGate(cross_project)),
        PolicyReason::ApprovalSubjectMismatch,
        DecisionStage::Approval,
    );
}

#[test]
fn merge_approval_cannot_be_reused_after_target_commit_or_diff_substitution() {
    let spec = task_spec();
    let mut approved = merge_gate(
        &spec,
        MergeTarget::PrimaryBranch("refs/heads/main".to_owned()),
    );
    approved.approval = Some(approval_fact(
        &spec,
        ApprovalKind::Merge,
        ApprovalAuthority::ResponsibleUser,
        ApprovalSubject::Merge(approved.subject.clone()),
    ));

    let mut switched_target = approved.clone();
    switched_target.subject = MergeSubject::new(
        MergeTarget::PrimaryBranch("refs/heads/release".to_owned()),
        switched_target.subject.reviewed_commit(),
        switched_target.subject.target_head(),
        switched_target.subject.diff_digest().clone(),
    )
    .expect("merge subject");
    assert_denial(
        evaluate(DecisionSubject::MergeGate(switched_target)),
        PolicyReason::MergeReadinessMismatch,
        DecisionStage::Project,
    );

    let mut switched_commit = approved.clone();
    switched_commit.subject = MergeSubject::new(
        switched_commit.subject.target().clone(),
        "a".repeat(40),
        switched_commit.subject.target_head(),
        switched_commit.subject.diff_digest().clone(),
    )
    .expect("merge subject");
    assert_denial(
        evaluate(DecisionSubject::MergeGate(switched_commit)),
        PolicyReason::MergeReadinessMismatch,
        DecisionStage::Project,
    );

    let mut switched_diff = approved;
    switched_diff.subject = MergeSubject::new(
        switched_diff.subject.target().clone(),
        switched_diff.subject.reviewed_commit(),
        switched_diff.subject.target_head(),
        digest('b'),
    )
    .expect("merge subject");
    assert_denial(
        evaluate(DecisionSubject::MergeGate(switched_diff)),
        PolicyReason::MergeReadinessMismatch,
        DecisionStage::Project,
    );
}

#[test]
fn memory_candidates_never_create_authority_and_require_exact_reviewed_provenance() {
    let spec = task_spec();

    let mut claims_authority = memory_gate(&spec, MemoryKind::Fact);
    claims_authority.claims_authority = true;
    assert_denial(
        evaluate(DecisionSubject::MemoryPromotion(claims_authority)),
        PolicyReason::MemoryCannotAuthorize,
        DecisionStage::Protected,
    );

    let mut cross_project = memory_gate(&spec, MemoryKind::Fact);
    cross_project.subject = MemoryCandidateSubject::new(
        binding_with_project_id(cross_project.subject.binding(), "other-project"),
        cross_project.subject.candidate_digest().clone(),
        cross_project.subject.kind(),
    )
    .expect("memory candidate subject");
    assert_denial(
        evaluate(DecisionSubject::MemoryPromotion(cross_project)),
        PolicyReason::MemoryCrossProject,
        DecisionStage::Project,
    );

    let mut no_provenance = memory_gate(&spec, MemoryKind::Fact);
    no_provenance
        .review
        .as_mut()
        .expect("review")
        .immutable_provenance = false;
    assert_denial(
        evaluate(DecisionSubject::MemoryPromotion(no_provenance)),
        PolicyReason::MemoryProvenanceRequired,
        DecisionStage::ProviderCapability,
    );

    let mut malformed = memory_gate(&spec, MemoryKind::Fact);
    malformed.review.as_mut().expect("review").schema_valid = false;
    assert_denied_at(
        evaluate(DecisionSubject::MemoryPromotion(malformed)),
        DecisionStage::ProviderCapability,
    );

    let mut not_reviewed = memory_gate(&spec, MemoryKind::Fact);
    not_reviewed
        .review
        .as_mut()
        .expect("review")
        .review_accepted = false;
    assert_denial(
        evaluate(DecisionSubject::MemoryPromotion(not_reviewed)),
        PolicyReason::MemoryReviewRequired,
        DecisionStage::Approval,
    );

    assert!(
        evaluate(DecisionSubject::MemoryPromotion(memory_gate(
            &spec,
            MemoryKind::Fact,
        )))
        .allowed(),
        "reviewed immutable non-authoritative fact should pass"
    );
}

#[test]
fn preference_memory_requires_exact_responsible_user_evidence() {
    let spec = task_spec();
    let mut missing = memory_gate(&spec, MemoryKind::Preference);
    missing.preference_user_approval = None;
    assert_denial(
        evaluate(DecisionSubject::MemoryPromotion(missing)),
        PolicyReason::PreferenceUserEvidenceRequired,
        DecisionStage::Approval,
    );

    let mut normal_gateway = memory_gate(&spec, MemoryKind::Preference);
    let approval = approval_fact(
        &spec,
        ApprovalKind::Preference,
        ApprovalAuthority::ProtectedGuardian,
        ApprovalSubject::Preference(normal_gateway.subject.clone()),
    );
    normal_gateway.preference_user_approval = Some(approval);
    assert_denied_at(
        evaluate(DecisionSubject::MemoryPromotion(normal_gateway)),
        DecisionStage::Approval,
    );

    let exact = memory_gate(&spec, MemoryKind::Preference);
    assert!(
        evaluate(DecisionSubject::MemoryPromotion(exact)).allowed(),
        "exact OS-authenticated preference approval should pass"
    );
}

#[test]
fn guarded_upgrade_denies_schema_migration_and_fails_closed_without_review_authority() {
    let spec = task_spec();

    let mut schema = activation_gate(&spec);
    schema.subject = rebuild_release(
        &schema.subject,
        None,
        None,
        None,
        None,
        None,
        Some(UpgradeDelta::new(
            true, false, false, false, false, false, false, false,
        )),
    );
    assert_denial(
        evaluate(DecisionSubject::UpgradeStage(schema)),
        PolicyReason::UpgradeSchemaMigrationDenied,
        DecisionStage::Protected,
    );

    let mut protected_delta = activation_gate(&spec);
    protected_delta.subject = rebuild_release(
        &protected_delta.subject,
        None,
        None,
        None,
        None,
        None,
        Some(UpgradeDelta::new(
            false, true, false, false, false, false, false, false,
        )),
    );
    assert_denial(
        evaluate(DecisionSubject::UpgradeStage(protected_delta)),
        PolicyReason::UpgradeDeltaProtected,
        DecisionStage::Protected,
    );

    let mut no_guardian = activation_gate(&spec);
    no_guardian.approval = None;
    assert_denial(
        evaluate(DecisionSubject::UpgradeStage(no_guardian)),
        PolicyReason::GuardianApprovalRequired,
        DecisionStage::Approval,
    );

    assert_denial(
        evaluate(DecisionSubject::UpgradeStage(activation_gate(&spec))),
        PolicyReason::ReviewAuthorityUnavailable,
        DecisionStage::Approval,
    );
}

#[test]
fn upgrade_manifest_slot_saga_epoch_and_candidate_are_all_required() {
    let spec = task_spec();

    let cases: &[(UpgradeMutation, PolicyReason)] = &[
        (
            |gate| {
                gate.evidence
                    .as_mut()
                    .expect("evidence")
                    .candidate_immutable = false;
            },
            PolicyReason::UpgradeStageDenied,
        ),
        (
            |gate| {
                gate.evidence
                    .as_mut()
                    .expect("evidence")
                    .inactive_slot_verified = false;
            },
            PolicyReason::UpgradeStageDenied,
        ),
        (
            |gate| gate.evidence.as_mut().expect("evidence").saga_bound = false,
            PolicyReason::UpgradeStageDenied,
        ),
        (
            |gate| gate.evidence.as_mut().expect("evidence").epoch_bound = false,
            PolicyReason::UpgradeStageDenied,
        ),
    ];

    for (mutate, expected) in cases {
        let mut gate = activation_gate(&spec);
        mutate(&mut gate);
        assert_denial(
            evaluate(DecisionSubject::UpgradeStage(gate)),
            *expected,
            DecisionStage::ProviderCapability,
        );
    }
}

#[test]
fn protected_core_release_requires_guardian_and_fails_closed_without_review_authority() {
    let spec = task_spec();

    let mut missing = protected_gate(&spec, ProtectedChangeClass::CoreReleaseActivation);
    missing.approval = None;
    assert_denial(
        evaluate(DecisionSubject::ProtectedChange(missing)),
        PolicyReason::GuardianApprovalRequired,
        DecisionStage::Approval,
    );

    assert_denial(
        evaluate(DecisionSubject::ProtectedChange(protected_gate(
            &spec,
            ProtectedChangeClass::CoreReleaseActivation,
        ))),
        PolicyReason::ReviewAuthorityUnavailable,
        DecisionStage::Approval,
    );
}

#[test]
fn all_protected_change_classes_require_an_exact_protected_subject() {
    let spec = task_spec();
    let classes = [
        ProtectedChangeClass::AccountOrCredential,
        ProtectedChangeClass::PaymentOrPurchase,
        ProtectedChangeClass::PublicExposure,
        ProtectedChangeClass::ProductionDeployment,
        ProtectedChangeClass::PermanentDelete,
        ProtectedChangeClass::DisableSecurity,
        ProtectedChangeClass::DestructiveMigration,
        ProtectedChangeClass::Policy,
        ProtectedChangeClass::Constitution,
        ProtectedChangeClass::Supervisor,
        ProtectedChangeClass::CapabilityExpansion,
        ProtectedChangeClass::PrimaryBranchMerge,
        ProtectedChangeClass::CoreReleaseActivation,
    ];

    for class in classes {
        let mut gate = protected_gate(&spec, class);
        gate.approval = None;
        let decision = evaluate(DecisionSubject::ProtectedChange(gate));
        assert!(!decision.allowed(), "{class:?} unexpectedly allowed");
        assert!(
            decision.evidence().checked_through() >= DecisionStage::Protected,
            "{class:?} stopped before protected routing: {:?}",
            decision.reason()
        );
    }
}

#[test]
fn generic_agent_action_cannot_bypass_specialized_merge_memory_or_upgrade_gates() {
    let spec = task_spec();
    let cases = vec![
        (
            "merge",
            agent_gate(
                &spec,
                AgentRole::Integrator,
                PolicyAction::IntegrateGit,
                TaskState::Merging,
                RuntimeAdmission::Active,
            ),
        ),
        (
            "memory promotion",
            agent_gate(
                &spec,
                AgentRole::MemoryReviewer,
                PolicyAction::PromoteMemory,
                TaskState::Reviewing,
                RuntimeAdmission::Active,
            ),
        ),
        (
            "upgrade proposal",
            agent_gate(
                &spec,
                AgentRole::LatticePm,
                PolicyAction::ProposeUpgrade,
                TaskState::Draft,
                RuntimeAdmission::Active,
            ),
        ),
        (
            "upgrade shadow",
            agent_gate(
                &spec,
                AgentRole::UpgradeGuardian,
                PolicyAction::GuardianShadow,
                TaskState::Reviewing,
                RuntimeAdmission::Active,
            ),
        ),
        (
            "upgrade health",
            agent_gate(
                &spec,
                AgentRole::UpgradeGuardian,
                PolicyAction::GuardianHealth,
                TaskState::Merging,
                RuntimeAdmission::Canary,
            ),
        ),
        (
            "upgrade activation",
            agent_gate(
                &spec,
                AgentRole::UpgradeGuardian,
                PolicyAction::ActivateUpgrade,
                TaskState::Merging,
                RuntimeAdmission::Active,
            ),
        ),
        (
            "upgrade rollback",
            agent_gate(
                &spec,
                AgentRole::UpgradeGuardian,
                PolicyAction::RollbackUpgrade,
                TaskState::Blocked,
                RuntimeAdmission::Canary,
            ),
        ),
    ];

    let mut bypassed = Vec::new();
    for (name, gate) in cases {
        let decision = evaluate(DecisionSubject::AgentAction(gate));
        if decision.allowed() {
            bypassed.push(name);
        } else {
            assert_eq!(decision.reason(), PolicyReason::ProtectedSurfaceRequired);
            assert_eq!(
                decision.evidence().checked_through(),
                DecisionStage::Protected
            );
        }
    }
    assert!(
        bypassed.is_empty(),
        "generic AgentAction bypassed specialized gates: {bypassed:?}"
    );
}

#[test]
fn protected_class_substitution_cannot_reuse_an_exact_approval() {
    let spec = task_spec();
    let original = protected_gate(&spec, ProtectedChangeClass::Policy);
    assert!(
        evaluate(DecisionSubject::ProtectedChange(original.clone())).allowed(),
        "baseline protected subject must be valid"
    );

    let mut substituted = original;
    substituted.subject = ProtectedChangeSubject::new(
        ProtectedChangeClass::PaymentOrPurchase,
        substituted.subject.operation_digest().clone(),
    )
    .expect("protected change subject");
    assert_denial(
        evaluate(DecisionSubject::ProtectedChange(substituted)),
        PolicyReason::ApprovalSubjectMismatch,
        DecisionStage::Approval,
    );
}

#[test]
fn nonzero_external_cost_amount_substitution_cannot_reuse_approval() {
    let spec = spec_with(|input| input.budget.max_external_cost = "10".to_owned());
    let mut original = graphify_gate(&spec);
    replace_agent_resources(
        &mut original,
        resource_fact(
            &spec,
            ResourceUsage {
                used_external_cost: "0".to_owned(),
                requested_external_cost: Some("1".to_owned()),
                ..safe_usage()
            },
        ),
    );
    attach_external_cost(&spec, &mut original, "1", "graphify-local");
    assert!(
        evaluate(DecisionSubject::AgentAction(original.clone())).allowed(),
        "baseline quoted cost must be valid"
    );

    let mut substituted = original;
    replace_agent_resources(
        &mut substituted,
        resource_fact(
            &spec,
            ResourceUsage {
                used_external_cost: "0".to_owned(),
                requested_external_cost: Some("2".to_owned()),
                ..safe_usage()
            },
        ),
    );
    assert_denial(
        evaluate(DecisionSubject::AgentAction(substituted)),
        PolicyReason::ExternalCostUnknown,
        DecisionStage::EffectEnvelope,
    );
}

#[test]
fn writer_use_subject_substitution_cannot_reuse_lease_authority() {
    let spec = task_spec();
    let baseline = write_gate(&spec);
    assert!(
        evaluate(DecisionSubject::AgentAction(baseline.clone())).allowed(),
        "baseline writer fact must be valid"
    );
    let subject = baseline.writer_subject.expect("writer subject");

    let mut bypassed = Vec::new();
    for (name, substituted, expected_reason) in writer_subject_substitutions(&subject) {
        let mut gate = write_gate(&spec);
        gate.writer_subject = Some(substituted);
        let decision = evaluate(DecisionSubject::AgentAction(gate));
        if decision.allowed() {
            bypassed.push(name);
        } else {
            assert_eq!(decision.reason(), expected_reason, "{name}");
            assert_eq!(decision.evidence().checked_through(), DecisionStage::Writer);
        }
    }
    assert!(
        bypassed.is_empty(),
        "writer authority survived substitutions: {bypassed:?}"
    );
}

#[test]
fn self_consistent_writer_identity_substitution_cannot_reuse_expected_subject() {
    let spec = task_spec();
    let baseline = writer_receipt_fixture(&spec);

    for (field, identity) in writer_identity_substitutions(&baseline.identity) {
        let mut substituted = baseline.clone();
        substituted.identity = identity;

        let mut gate = write_gate(&spec);
        gate.writer = Some(writer_fact_from_fixture(&substituted));
        let decision = evaluate(DecisionSubject::AgentAction(gate));
        let expected_reason = if field == "fencing_token" {
            PolicyReason::FencingTokenMismatch
        } else {
            PolicyReason::WriterLeaseSubjectMismatch
        };

        assert!(!decision.allowed(), "{field} substitution was admitted");
        assert_eq!(decision.reason(), expected_reason, "{field}");
        assert_eq!(
            decision.evidence().checked_through(),
            DecisionStage::Writer,
            "{field}"
        );
    }
}

#[test]
fn independent_writer_head_substitution_rejects_every_security_field() {
    let spec = task_spec();
    let baseline = writer_receipt_fixture(&spec);

    for (field, substituted) in writer_receipt_substitutions(&baseline) {
        let mut gate = write_gate(&spec);
        gate.writer = Some(WriterLeaseFact {
            receipt: build_writer_receipt(&baseline),
            current_head: Some(build_writer_receipt(&substituted).head()),
        });

        assert_denial(
            evaluate(DecisionSubject::AgentAction(gate)),
            PolicyReason::WriterLeaseNotCurrent,
            DecisionStage::Writer,
        );
        assert_ne!(
            build_writer_receipt(&baseline).head(),
            build_writer_receipt(&substituted).head(),
            "{field} must participate in full-head equality"
        );
    }
}

#[test]
fn writer_receipt_rejects_substituted_contract_and_producer_metadata() {
    let spec = task_spec();
    let fixture = writer_receipt_fixture(&spec);

    for (field, version, producer_id, producer_version) in [
        (
            "contract_version",
            CONTRACT_VERSION.saturating_add(1),
            WRITER_LEASE_PRODUCER_ID,
            WRITER_LEASE_PRODUCER_VERSION,
        ),
        (
            "producer_id",
            CONTRACT_VERSION,
            "caller-asserted-writer",
            WRITER_LEASE_PRODUCER_VERSION,
        ),
        (
            "producer_version",
            CONTRACT_VERSION,
            WRITER_LEASE_PRODUCER_ID,
            "9.9",
        ),
    ] {
        let receipt = WriterLeaseAuthorityReceipt::new(
            version,
            producer_id,
            producer_version,
            fixture.runtime,
            build_writer_identity(&fixture.identity),
            fixture.status,
            fixture.revision,
            fixture.runtime_admission,
            fixture.acquired_at.clone(),
            fixture.heartbeat_at.clone(),
            fixture.expires_at.clone(),
            fixture.time_observation_digest.clone(),
            fixture.admission_observation_digest.clone(),
            fixture.transition_digest.clone(),
            fixture.receipt_digest.clone(),
        );
        assert!(receipt.is_err(), "{field} substitution must be rejected");
    }
}

#[test]
fn stale_writer_precedes_resource_budget_denial() {
    let spec = task_spec();
    let mut gate = write_gate(&spec);
    gate.writer.as_mut().expect("writer").current_head = None;
    replace_agent_resources(
        &mut gate,
        resource_fact(
            &spec,
            ResourceUsage {
                active_agents: 99,
                ..safe_usage()
            },
        ),
    );

    assert_denial(
        evaluate(DecisionSubject::AgentAction(gate)),
        PolicyReason::WriterLeaseNotCurrent,
        DecisionStage::Writer,
    );
}

#[test]
fn memory_candidate_substitution_cannot_reuse_provenance_and_review_bools() {
    let spec = task_spec();
    let original = memory_gate(&spec, MemoryKind::Fact);
    assert!(
        evaluate(DecisionSubject::MemoryPromotion(original.clone())).allowed(),
        "baseline memory candidate must be valid"
    );

    let mut substituted = original;
    substituted.subject = MemoryCandidateSubject::new(
        substituted.subject.binding().clone(),
        digest('6'),
        substituted.subject.kind(),
    )
    .expect("memory candidate subject");
    assert_denial(
        evaluate(DecisionSubject::MemoryPromotion(substituted)),
        PolicyReason::MemoryProvenanceRequired,
        DecisionStage::ProviderCapability,
    );
}

#[test]
fn upgrade_manifest_approval_rejects_replaced_manifest_delta_slot_saga_and_epoch() {
    let spec = task_spec();
    let original = activation_gate(&spec);
    assert_denial(
        evaluate(DecisionSubject::UpgradeStage(original.clone())),
        PolicyReason::ReviewAuthorityUnavailable,
        DecisionStage::Approval,
    );

    let mut manifest = original.clone();
    manifest.subject = rebuild_release(
        &manifest.subject,
        None,
        None,
        Some(digest('6')),
        None,
        None,
        None,
    );
    assert_denial(
        evaluate(DecisionSubject::UpgradeStage(manifest)),
        PolicyReason::UpgradeStageDenied,
        DecisionStage::ProviderCapability,
    );

    let mut delta = original.clone();
    delta.subject = rebuild_release(
        &delta.subject,
        None,
        None,
        None,
        None,
        None,
        Some(UpgradeDelta::new(
            false, true, false, false, false, false, false, false,
        )),
    );
    assert_denial(
        evaluate(DecisionSubject::UpgradeStage(delta)),
        PolicyReason::UpgradeDeltaProtected,
        DecisionStage::Protected,
    );

    for mutate in [
        |gate: &mut UpgradeGate<'_>| {
            gate.subject =
                rebuild_release(&gate.subject, None, None, None, Some("slot-c"), None, None);
        },
        |gate: &mut UpgradeGate<'_>| {
            gate.subject = rebuild_release(
                &gate.subject,
                Some("saga-other"),
                None,
                None,
                None,
                None,
                None,
            );
        },
        |gate: &mut UpgradeGate<'_>| {
            gate.subject = rebuild_release(
                &gate.subject,
                None,
                None,
                None,
                None,
                Some(
                    DaemonEpoch::new(gate.subject.requested_epoch().get() + 1)
                        .expect("release epoch"),
                ),
                None,
            );
        },
    ] {
        let mut substituted = original.clone();
        mutate(&mut substituted);
        assert_denial(
            evaluate(DecisionSubject::UpgradeStage(substituted)),
            PolicyReason::UpgradeStageDenied,
            DecisionStage::ProviderCapability,
        );
    }
}

#[test]
fn upgrade_activation_approval_subject_must_not_be_the_bare_manifest_digest() {
    let spec = task_spec();
    let gate = activation_gate(&spec);
    let protected_release = ProtectedReleaseSubject::new(
        gate.subject.clone(),
        gate.guardian.as_ref().expect("guardian").runtime.clone(),
    );
    assert_eq!(
        gate.approval
            .as_ref()
            .expect("approval")
            .receipt
            .identity()
            .subject(),
        &ApprovalSubject::ProtectedRelease(Box::new(protected_release)),
        "protected approval must bind the full typed activation subject"
    );
}

#[test]
fn registry_primary_branch_classification_cannot_be_caller_masqueraded() {
    let spec = spec_with(|input| {
        input.risk_class = RiskClass::R0;
        input.approval_requirements.merge = ApprovalRequirement::NotRequired;
    });

    let valid_feature = merge_gate(
        &spec,
        MergeTarget::FeatureBranch("refs/heads/integration".to_owned()),
    );
    assert!(
        evaluate(DecisionSubject::MergeGate(valid_feature)).allowed(),
        "a Registry-confirmed non-primary branch may follow the Task Spec floor"
    );

    for target in [
        MergeTarget::FeatureBranch("refs/heads/main".to_owned()),
        MergeTarget::PrimaryBranch("refs/heads/integration".to_owned()),
    ] {
        assert_denial(
            evaluate(DecisionSubject::MergeGate(merge_gate(&spec, target))),
            PolicyReason::InvalidDecisionSubject,
            DecisionStage::Project,
        );
    }
}

#[test]
fn malformed_merge_subject_denies_at_input_before_project_or_runtime() {
    for target in [
        MergeTarget::FeatureBranch("bad:ref".to_owned()),
        MergeTarget::FeatureBranch("foo.lock/bar".to_owned()),
        MergeTarget::FeatureBranch("bad//ref".to_owned()),
    ] {
        assert!(
            MergeSubject::new(target, "d".repeat(40), "c".repeat(40), digest('e')).is_err(),
            "malformed merge target must be unrepresentable"
        );
    }

    let spec = task_spec();
    let mut bad_commit = merge_gate(
        &spec,
        MergeTarget::FeatureBranch("refs/heads/integration".to_owned()),
    );
    bad_commit.subject = MergeSubject::new(
        bad_commit.subject.target().clone(),
        "not-a-git-object",
        bad_commit.subject.target_head(),
        bad_commit.subject.diff_digest().clone(),
    )
    .expect("merge subject");
    bad_commit.context.runtime_admission = Boundary::Known(RuntimeAdmission::Stopped);
    assert_denial(
        evaluate(DecisionSubject::MergeGate(bad_commit)),
        PolicyReason::InvalidDecisionSubject,
        DecisionStage::Input,
    );
}

#[test]
fn protected_change_cannot_lower_r3_or_task_requested_authority() {
    let r3_spec = spec_with(|input| input.risk_class = RiskClass::R3);
    let r3 = protected_gate(&r3_spec, ProtectedChangeClass::PaymentOrPurchase);
    assert_denial(
        evaluate(DecisionSubject::ProtectedChange(r3)),
        PolicyReason::ReviewAuthorityUnavailable,
        DecisionStage::Approval,
    );

    let guardian_spec = spec_with(|input| {
        input.approval_requirements.execution = ApprovalRequirement::ProtectedGuardian;
    });
    assert_denial(
        evaluate(DecisionSubject::ProtectedChange(protected_gate(
            &guardian_spec,
            ProtectedChangeClass::DisableSecurity,
        ))),
        PolicyReason::ApprovalAuthorityDenied,
        DecisionStage::Approval,
    );

    let primary_guardian_spec = spec_with(|input| {
        input.approval_requirements.merge = ApprovalRequirement::ProtectedGuardian;
    });
    assert_denial(
        evaluate(DecisionSubject::ProtectedChange(protected_gate(
            &primary_guardian_spec,
            ProtectedChangeClass::PrimaryBranchMerge,
        ))),
        PolicyReason::ApprovalAuthorityDenied,
        DecisionStage::Approval,
    );
}

#[test]
fn worker_admission_and_merge_cannot_introduce_external_cost() {
    let spec = spec_with(|input| {
        input.budget.max_external_cost = "1".to_owned();
        input.approval_requirements.merge = ApprovalRequirement::NotRequired;
    });
    let worker = worker_gate(
        &spec,
        vec![AgentRole::Planner],
        ResourceUsage {
            active_agents: 0,
            requested_agents: 1,
            requested_external_cost: Some("0.1".to_owned()),
            ..safe_usage()
        },
    );
    assert_denial(
        evaluate(DecisionSubject::WorkerAdmission(worker)),
        PolicyReason::ExternalCostProtected,
        DecisionStage::EffectEnvelope,
    );

    let mut merge = merge_gate(
        &spec,
        MergeTarget::FeatureBranch("refs/heads/integration".to_owned()),
    );
    replace_merge_resources(
        &mut merge,
        resource_fact(
            &spec,
            ResourceUsage {
                requested_external_cost: Some("0.1".to_owned()),
                ..safe_usage()
            },
        ),
    );
    assert_denial(
        evaluate(DecisionSubject::MergeGate(merge)),
        PolicyReason::ExternalCostProtected,
        DecisionStage::EffectEnvelope,
    );
}

#[test]
fn recovery_cannot_carry_new_effects_and_release_requires_exact_writer() {
    let spec = spec_with(|input| {
        input.network_policy = NetworkPolicy::LoopbackOnly;
        input.deployment_policy = DeploymentPolicy::PrepareOnly;
        input.budget.max_external_cost = "1".to_owned();
    });

    let mut network = agent_gate(
        &spec,
        AgentRole::LatticePm,
        PolicyAction::StopRuntime,
        TaskState::Executing,
        RuntimeAdmission::Draining,
    );
    network.network = NetworkIntent::Loopback;
    assert_denial(
        evaluate(DecisionSubject::AgentAction(network)),
        PolicyReason::ProtectedSurfaceRequired,
        DecisionStage::EffectEnvelope,
    );

    let mut deployment = agent_gate(
        &spec,
        AgentRole::LatticePm,
        PolicyAction::StopRuntime,
        TaskState::Executing,
        RuntimeAdmission::Draining,
    );
    deployment.deployment = DeploymentIntent::PrepareArtifact;
    assert_denial(
        evaluate(DecisionSubject::AgentAction(deployment)),
        PolicyReason::ProtectedSurfaceRequired,
        DecisionStage::EffectEnvelope,
    );

    for resources in [
        ResourceUsage {
            requested_agents: 1,
            ..safe_usage()
        },
        ResourceUsage {
            requested_model_calls: 1,
            ..safe_usage()
        },
        ResourceUsage {
            requested_external_cost: Some("0.1".to_owned()),
            ..safe_usage()
        },
    ] {
        let mut gate = agent_gate(
            &spec,
            AgentRole::LatticePm,
            PolicyAction::StopRuntime,
            TaskState::Executing,
            RuntimeAdmission::Draining,
        );
        replace_agent_resources(&mut gate, resource_fact(&spec, resources));
        assert_denial(
            evaluate(DecisionSubject::AgentAction(gate)),
            PolicyReason::ProtectedSurfaceRequired,
            DecisionStage::EffectEnvelope,
        );
    }

    let missing_writer = agent_gate(
        &spec,
        AgentRole::Implementer,
        PolicyAction::ReleaseWriter,
        TaskState::Blocked,
        RuntimeAdmission::Draining,
    );
    assert_denial(
        evaluate(DecisionSubject::AgentAction(missing_writer)),
        PolicyReason::WriterLeaseRequired,
        DecisionStage::Writer,
    );

    let mut exact_release = agent_gate(
        &spec,
        AgentRole::Implementer,
        PolicyAction::ReleaseWriter,
        TaskState::Blocked,
        RuntimeAdmission::Draining,
    );
    let writer = writer_fact(&spec);
    exact_release.writer_subject = Some(writer_subject_from_fact(&writer));
    exact_release.writer = Some(writer);
    assert!(
        evaluate(DecisionSubject::AgentAction(exact_release)).allowed(),
        "bounded release of the exact current writer should remain available"
    );
}

#[test]
fn run_tests_requires_the_exact_current_writer_subject() {
    let spec = task_spec();
    let missing = agent_gate(
        &spec,
        AgentRole::Implementer,
        PolicyAction::RunTests,
        TaskState::Verifying,
        RuntimeAdmission::Active,
    );
    assert_denial(
        evaluate(DecisionSubject::AgentAction(missing)),
        PolicyReason::WriterLeaseRequired,
        DecisionStage::Writer,
    );

    let mut exact = agent_gate(
        &spec,
        AgentRole::Implementer,
        PolicyAction::RunTests,
        TaskState::Verifying,
        RuntimeAdmission::Active,
    );
    "implementer-1".clone_into(&mut exact.actor_id);
    let writer = writer_fact(&spec);
    exact.writer_subject = Some(writer_subject_from_fact(&writer));
    exact.writer = Some(writer);
    replace_agent_resources(
        &mut exact,
        resource_fact(
            &spec,
            ResourceUsage {
                active_implementers: 1,
                ..safe_usage()
            },
        ),
    );
    assert!(
        evaluate(DecisionSubject::AgentAction(exact)).allowed(),
        "tests in the writable worktree require and accept only the exact writer"
    );
}

#[test]
fn external_cost_quote_provider_currency_and_pricing_are_exact() {
    let spec = spec_with(|input| input.budget.max_external_cost = "10".to_owned());
    let mut baseline = graphify_gate(&spec);
    replace_agent_resources(
        &mut baseline,
        resource_fact(
            &spec,
            ResourceUsage {
                requested_external_cost: Some("1".to_owned()),
                ..safe_usage()
            },
        ),
    );
    attach_external_cost(&spec, &mut baseline, "1", "graphify-local");
    assert!(evaluate(DecisionSubject::AgentAction(baseline.clone())).allowed());

    for mutate in [
        |subject: &mut ExternalCostSubject| {
            *subject = rebuild_external_cost(subject, None, None, Some(digest('c')), None);
        },
        |subject: &mut ExternalCostSubject| {
            *subject = rebuild_external_cost(subject, None, None, None, Some(digest('d')));
        },
    ] {
        let mut changed = baseline.clone();
        mutate(&mut changed.external_cost.as_mut().expect("cost").subject);
        assert_denial(
            evaluate(DecisionSubject::AgentAction(changed)),
            PolicyReason::ApprovalSubjectMismatch,
            DecisionStage::Approval,
        );
    }

    let mut changed_currency = baseline.clone();
    let subject = &mut changed_currency
        .external_cost
        .as_mut()
        .expect("cost")
        .subject;
    *subject = rebuild_external_cost(subject, Some("TWD"), None, None, None);
    assert_denial(
        evaluate(DecisionSubject::AgentAction(changed_currency)),
        PolicyReason::ExternalCostUnknown,
        DecisionStage::EffectEnvelope,
    );

    let mut changed_provider = baseline;
    let subject = &mut changed_provider
        .external_cost
        .as_mut()
        .expect("cost")
        .subject;
    *subject = rebuild_external_cost(subject, None, Some("other-provider"), None, None);
    assert_denial(
        evaluate(DecisionSubject::AgentAction(changed_provider)),
        PolicyReason::ExternalCostUnknown,
        DecisionStage::EffectEnvelope,
    );
}

#[test]
fn guardian_canary_is_system_only_and_rollback_never_admits_schema_migration() {
    let spec = task_spec();
    let mut user_project = activation_gate(&spec);
    user_project.stage = UpgradeStage::HealthCanary;
    user_project.context.runtime_admission = Boundary::Known(RuntimeAdmission::Canary);
    set_project_class(
        user_project.context.project.as_mut().expect("project"),
        ProjectClass::UserProject,
    );
    assert_denial(
        evaluate(DecisionSubject::UpgradeStage(user_project)),
        PolicyReason::ProtectedSurfaceRequired,
        DecisionStage::Project,
    );

    for mutate in [
        |guardian: &mut GuardianAuthorityFact| guardian.reserved_system_stream = false,
        |guardian: &mut GuardianAuthorityFact| guardian.user_project_access = true,
    ] {
        let mut gate = activation_gate(&spec);
        gate.stage = UpgradeStage::HealthCanary;
        gate.context.runtime_admission = Boundary::Known(RuntimeAdmission::Canary);
        mutate(gate.guardian.as_mut().expect("guardian"));
        assert_denial(
            evaluate(DecisionSubject::UpgradeStage(gate)),
            PolicyReason::GuardianRequired,
            DecisionStage::Approval,
        );
    }

    let mut rollback = rollback_gate(&spec);
    let migration = digest('f');
    rollback
        .rollback
        .as_mut()
        .expect("rollback")
        .migration_digests
        .push(migration.clone());
    rollback
        .evidence
        .as_mut()
        .expect("evidence")
        .rollback
        .as_mut()
        .expect("rollback evidence")
        .migration_digests
        .push(migration.clone());
    rollback
        .guardian
        .as_mut()
        .expect("guardian")
        .rollback
        .as_mut()
        .expect("guardian rollback")
        .migration_digests
        .push(migration);
    assert_denial(
        evaluate(DecisionSubject::UpgradeStage(rollback)),
        PolicyReason::UpgradeSchemaMigrationDenied,
        DecisionStage::Protected,
    );
}

#[test]
fn allowed_task_envelopes_do_not_grant_unrelated_network_or_deployment_effects() {
    let loopback_spec = spec_with(|input| input.network_policy = NetworkPolicy::LoopbackOnly);
    let mut loopback_read = agent_gate(
        &loopback_spec,
        AgentRole::Planner,
        PolicyAction::ReadRepository,
        TaskState::Draft,
        RuntimeAdmission::Active,
    );
    loopback_read.network = NetworkIntent::Loopback;
    assert_denial(
        evaluate(DecisionSubject::AgentAction(loopback_read)),
        PolicyReason::NetworkTargetMismatch,
        DecisionStage::EffectEnvelope,
    );

    let prepare_spec = spec_with(|input| input.deployment_policy = DeploymentPolicy::PrepareOnly);
    let mut prepare_read = agent_gate(
        &prepare_spec,
        AgentRole::Planner,
        PolicyAction::ReadRepository,
        TaskState::Draft,
        RuntimeAdmission::Active,
    );
    prepare_read.deployment = DeploymentIntent::PrepareArtifact;
    assert_denial(
        evaluate(DecisionSubject::AgentAction(prepare_read)),
        PolicyReason::ProtectedSurfaceRequired,
        DecisionStage::EffectEnvelope,
    );
}

#[test]
fn canonical_primary_aliases_cannot_be_declared_feature() {
    let spec = spec_with(|input| {
        input.risk_class = RiskClass::R0;
        input.approval_requirements.merge = ApprovalRequirement::NotRequired;
    });

    let registry_short = merge_gate(
        &spec,
        MergeTarget::FeatureBranch("refs/heads/main".to_owned()),
    );
    assert_denial(
        evaluate(DecisionSubject::MergeGate(registry_short)),
        PolicyReason::InvalidDecisionSubject,
        DecisionStage::Project,
    );

    assert!(
        MergeSubject::new(
            MergeTarget::FeatureBranch("main".to_owned()),
            "d".repeat(40),
            "c".repeat(40),
            digest('e'),
        )
        .is_err(),
        "short branch aliases must be unrepresentable"
    );

    for ambiguous in [
        "HEAD",
        "refs/heads/HEAD",
        "AUTO_MERGE",
        "BISECT_EXPECTED_REV",
        "BISECT_START",
        "integration",
        "origin/main",
        "v1",
        "heads/main",
        "tags/main",
        "remotes/origin/main",
    ] {
        assert!(
            MergeSubject::new(
                MergeTarget::FeatureBranch(ambiguous.to_owned()),
                "d".repeat(40),
                "c".repeat(40),
                digest('e'),
            )
            .is_err(),
            "{ambiguous} must be unrepresentable"
        );
    }

    let case_alias = merge_gate(
        &spec,
        MergeTarget::FeatureBranch("refs/heads/Main".to_owned()),
    );
    assert_denial(
        evaluate(DecisionSubject::MergeGate(case_alias)),
        PolicyReason::InvalidDecisionSubject,
        DecisionStage::Project,
    );
}

#[test]
fn release_writer_rejects_non_holder_actor() {
    let spec = task_spec();
    let mut gate = agent_gate(
        &spec,
        AgentRole::Implementer,
        PolicyAction::ReleaseWriter,
        TaskState::Blocked,
        RuntimeAdmission::Draining,
    );
    let writer = writer_fact(&spec);
    gate.writer_subject = Some(writer_subject_from_fact(&writer));
    gate.writer = Some(writer);
    gate.actor_id = "implementer-2".to_owned();

    assert_denial(
        evaluate(DecisionSubject::AgentAction(gate)),
        PolicyReason::WriterLeaseSubjectMismatch,
        DecisionStage::Writer,
    );
}

#[test]
fn effect_envelope_precedes_worker_resources_and_merge_approval() {
    let worker_spec = spec_with(|input| input.budget.max_external_cost = "1".to_owned());
    let worker = worker_gate(
        &worker_spec,
        vec![AgentRole::Planner],
        ResourceUsage {
            requested_agents: 2,
            requested_external_cost: Some("0.1".to_owned()),
            ..safe_usage()
        },
    );
    assert_denial(
        evaluate(DecisionSubject::WorkerAdmission(worker)),
        PolicyReason::ExternalCostProtected,
        DecisionStage::EffectEnvelope,
    );

    let merge_spec = spec_with(|input| {
        input.risk_class = RiskClass::R2;
        input.budget.max_external_cost = "1".to_owned();
        input.approval_requirements.merge = ApprovalRequirement::ResponsibleUser;
    });
    let mut merge = merge_gate(
        &merge_spec,
        MergeTarget::FeatureBranch("refs/heads/integration".to_owned()),
    );
    replace_merge_resources(
        &mut merge,
        resource_fact(
            &merge_spec,
            ResourceUsage {
                requested_external_cost: Some("0.1".to_owned()),
                ..safe_usage()
            },
        ),
    );
    assert_denial(
        evaluate(DecisionSubject::MergeGate(merge)),
        PolicyReason::ExternalCostProtected,
        DecisionStage::EffectEnvelope,
    );
}

#[test]
fn protected_release_approval_rejects_guardian_runtime_substitution() {
    let spec = task_spec();
    let baseline = activation_gate(&spec);
    assert_denial(
        evaluate(DecisionSubject::UpgradeStage(baseline.clone())),
        PolicyReason::ReviewAuthorityUnavailable,
        DecisionStage::Approval,
    );

    for mutate in [
        |guardian: &mut GuardianAuthorityFact| {
            guardian.runtime =
                rebuild_guardian_runtime(&guardian.runtime, Some("guardian-other"), None, None);
        },
        |guardian: &mut GuardianAuthorityFact| {
            guardian.runtime =
                rebuild_guardian_runtime(&guardian.runtime, None, Some(digest('f')), None);
        },
        |guardian: &mut GuardianAuthorityFact| {
            guardian.runtime = rebuild_guardian_runtime(
                &guardian.runtime,
                None,
                None,
                Some("guardian-daemon-other"),
            );
        },
    ] {
        let mut changed = baseline.clone();
        mutate(changed.guardian.as_mut().expect("guardian"));
        assert_denial(
            evaluate(DecisionSubject::UpgradeStage(changed)),
            PolicyReason::ApprovalSubjectMismatch,
            DecisionStage::Approval,
        );
    }
}

#[test]
fn generic_agent_action_cannot_reconcile_runtime() {
    let spec = task_spec();
    let gate = agent_gate(
        &spec,
        AgentRole::UpgradeGuardian,
        PolicyAction::ReconcileRuntime,
        TaskState::Blocked,
        RuntimeAdmission::ReconciliationRequired,
    );
    assert_denial(
        evaluate(DecisionSubject::AgentAction(gate)),
        PolicyReason::ProtectedSurfaceRequired,
        DecisionStage::Protected,
    );
}

#[test]
fn activation_subject_cannot_be_reused_as_rollback_subject() {
    let spec = task_spec();
    let mut gate = activation_gate(&spec);
    gate.stage = UpgradeStage::Rollback;
    gate.context.state = Boundary::Known(TaskState::Blocked);
    gate.context.runtime_admission = Boundary::Known(RuntimeAdmission::Canary);

    assert_denial(
        evaluate(DecisionSubject::UpgradeStage(gate)),
        PolicyReason::UpgradeStageDenied,
        DecisionStage::Input,
    );
}

#[test]
fn conflict_result_cannot_be_flipped_outside_merge_analysis() {
    let spec = spec_with(|input| {
        input.risk_class = RiskClass::R0;
        input.approval_requirements.merge = ApprovalRequirement::NotRequired;
    });
    let mut gate = merge_gate(
        &spec,
        MergeTarget::FeatureBranch("refs/heads/integration".to_owned()),
    );
    gate.readiness.as_mut().expect("readiness").conflict_free = false;

    assert_denial(
        evaluate(DecisionSubject::MergeGate(gate)),
        PolicyReason::MergeConflictRequiresImplementer,
        DecisionStage::Protected,
    );
}

#[test]
fn resource_fact_rejects_cross_task_stale_substituted_head_and_currency_drift() {
    let spec = task_spec();
    assert!(
        evaluate(DecisionSubject::AgentAction(write_gate(&spec))).allowed(),
        "baseline Task-Ledger resource fact must be valid"
    );

    let mut cross_project = write_gate(&spec);
    let resources = cross_project.resources.as_mut().expect("resources");
    resources.binding = binding_with_project_id(&resources.binding, "other-project");
    assert_denial(
        evaluate(DecisionSubject::AgentAction(cross_project)),
        PolicyReason::ProjectIdMismatch,
        DecisionStage::Project,
    );

    let mut stale = write_gate(&spec);
    stale.resources.as_mut().expect("resources").current_head = None;
    assert_denial(
        evaluate(DecisionSubject::AgentAction(stale)),
        PolicyReason::ResourceEvidenceStale,
        DecisionStage::Resources,
    );

    let mut substituted_head = write_gate(&spec);
    substituted_head
        .resources
        .as_mut()
        .expect("resources")
        .current_head = Some(
        resource_fact_with_currency(&spec, safe_usage(), "TWD")
            .receipt
            .head(),
    );
    assert_denial(
        evaluate(DecisionSubject::AgentAction(substituted_head)),
        PolicyReason::ResourceEvidenceStale,
        DecisionStage::Resources,
    );

    let mut currency_drift = write_gate(&spec);
    replace_agent_resources(
        &mut currency_drift,
        resource_fact_with_currency(&spec, safe_usage(), "TWD"),
    );
    assert_denial(
        evaluate(DecisionSubject::AgentAction(currency_drift)),
        PolicyReason::ResourceCurrencyMismatch,
        DecisionStage::Resources,
    );

    let mut replayed_action = write_gate(&spec);
    replayed_action
        .resource_subject
        .as_mut()
        .expect("resource subject")
        .effect_claim_id = "effect-claim-from-another-request".to_owned();
    assert_denial(
        evaluate(DecisionSubject::AgentAction(replayed_action)),
        PolicyReason::ResourceEvidenceInvalid,
        DecisionStage::Resources,
    );

    let mut replayed_worker = worker_gate(
        &spec,
        vec![AgentRole::Planner],
        ResourceUsage {
            active_agents: 0,
            requested_agents: 1,
            ..safe_usage()
        },
    );
    replayed_worker.resource_subject.effect_claim_id =
        "effect-claim-from-another-worker-admission".to_owned();
    assert_denial(
        evaluate(DecisionSubject::WorkerAdmission(replayed_worker)),
        PolicyReason::ResourceEvidenceInvalid,
        DecisionStage::Resources,
    );

    let merge_spec = spec_with(|input| {
        input.risk_class = RiskClass::R0;
        input.approval_requirements.merge = ApprovalRequirement::NotRequired;
    });
    let mut replayed_merge = merge_gate(
        &merge_spec,
        MergeTarget::FeatureBranch("refs/heads/integration".to_owned()),
    );
    replayed_merge.resource_subject.effect_claim_id = "effect-claim-from-another-merge".to_owned();
    assert_denial(
        evaluate(DecisionSubject::MergeGate(replayed_merge)),
        PolicyReason::ResourceEvidenceInvalid,
        DecisionStage::Resources,
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn resource_receipt_identity_and_runtime_substitution_matrix_fails_closed() {
    let spec = task_spec();
    let baseline = resource_fact(&spec, safe_usage()).receipt;
    let identity = baseline.stream_head().identity();
    let currency = identity.accounting_currency();
    let identity_cases = [
        (
            "project_id",
            TaskLedgerStreamIdentity::new(
                ProjectId::new("other-project").expect("project"),
                identity.project_snapshot_id().clone(),
                identity.task_id().clone(),
                identity.task_revision(),
                identity.task_spec_digest().clone(),
                currency,
            )
            .expect("identity"),
            PolicyReason::ProjectIdMismatch,
        ),
        (
            "project_snapshot_id",
            TaskLedgerStreamIdentity::new(
                identity.project_id().clone(),
                ProjectSnapshotId::new("other-snapshot").expect("snapshot"),
                identity.task_id().clone(),
                identity.task_revision(),
                identity.task_spec_digest().clone(),
                currency,
            )
            .expect("identity"),
            PolicyReason::ProjectSnapshotMismatch,
        ),
        (
            "task_id",
            TaskLedgerStreamIdentity::new(
                identity.project_id().clone(),
                identity.project_snapshot_id().clone(),
                TaskId::new("TASK-OTHER").expect("task"),
                identity.task_revision(),
                identity.task_spec_digest().clone(),
                currency,
            )
            .expect("identity"),
            PolicyReason::TaskIdMismatch,
        ),
        (
            "task_revision",
            TaskLedgerStreamIdentity::new(
                identity.project_id().clone(),
                identity.project_snapshot_id().clone(),
                identity.task_id().clone(),
                "2",
                identity.task_spec_digest().clone(),
                currency,
            )
            .expect("identity"),
            PolicyReason::TaskRevisionMismatch,
        ),
        (
            "task_spec_digest",
            TaskLedgerStreamIdentity::new(
                identity.project_id().clone(),
                identity.project_snapshot_id().clone(),
                identity.task_id().clone(),
                identity.task_revision(),
                digest('f'),
                currency,
            )
            .expect("identity"),
            PolicyReason::TaskSpecHashMismatch,
        ),
    ];

    for (field, substituted_identity, reason) in identity_cases {
        let mut stream = StreamHeadFixture::from_head(baseline.stream_head());
        stream.identity = substituted_identity;
        let mut receipt = ResourceReceiptFixture::from_receipt(&baseline);
        receipt.stream_head = stream.build();
        let mut gate = write_gate(&spec);
        replace_agent_resources(
            &mut gate,
            resource_fact_from_receipt(&spec, receipt.build()),
        );
        let decision = evaluate(DecisionSubject::AgentAction(gate));
        assert_eq!(decision.reason(), reason, "{field} substitution");
        assert_eq!(
            decision.evidence().checked_through(),
            DecisionStage::Project,
            "{field} substitution"
        );
        assert!(!decision.allowed(), "{field} substitution");
    }

    let mut live_stream = StreamHeadFixture::from_head(baseline.stream_head());
    live_stream.runtime = RuntimeKind::Live;
    let mut live_receipt = ResourceReceiptFixture::from_receipt(&baseline);
    live_receipt.runtime = RuntimeKind::Live;
    live_receipt.stream_head = live_stream.build();
    let mut live_gate = write_gate(&spec);
    replace_agent_resources(
        &mut live_gate,
        resource_fact_from_receipt(&spec, live_receipt.build()),
    );
    assert_denial(
        evaluate(DecisionSubject::AgentAction(live_gate)),
        PolicyReason::RuntimeKindMismatch,
        DecisionStage::Resources,
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn independent_resource_head_rejects_every_security_field_substitution() {
    let spec = task_spec();
    let baseline_fact = resource_fact(&spec, safe_usage());
    let baseline_receipt = baseline_fact.receipt.clone();
    let baseline_head = baseline_receipt.head();
    let mut alternatives = Vec::new();

    let mut changed = StreamHeadFixture::from_head(baseline_receipt.stream_head());
    changed.identity = TaskLedgerStreamIdentity::new(
        ProjectId::new("other-project").expect("project"),
        changed.identity.project_snapshot_id().clone(),
        changed.identity.task_id().clone(),
        changed.identity.task_revision(),
        changed.identity.task_spec_digest().clone(),
        changed.identity.accounting_currency(),
    )
    .expect("identity");
    let mut receipt = ResourceReceiptFixture::from_receipt(&baseline_receipt);
    receipt.stream_head = changed.build();
    alternatives.push(("stream_identity", receipt.build()));

    let mut changed = StreamHeadFixture::from_head(baseline_receipt.stream_head());
    changed.stream_id = digest('8');
    let mut receipt = ResourceReceiptFixture::from_receipt(&baseline_receipt);
    receipt.stream_head = changed.build();
    alternatives.push(("stream_id", receipt.build()));

    let mut changed = StreamHeadFixture::from_head(baseline_receipt.stream_head());
    changed.sequence += 1;
    let mut receipt = ResourceReceiptFixture::from_receipt(&baseline_receipt);
    receipt.stream_head = changed.build();
    alternatives.push(("sequence", receipt.build()));

    let mut changed = StreamHeadFixture::from_head(baseline_receipt.stream_head());
    changed.last_event_digest = digest('8');
    let mut receipt = ResourceReceiptFixture::from_receipt(&baseline_receipt);
    receipt.stream_head = changed.build();
    alternatives.push(("last_event_digest", receipt.build()));

    let mut changed = StreamHeadFixture::from_head(baseline_receipt.stream_head());
    changed.resource_revision -= 1;
    let mut receipt = ResourceReceiptFixture::from_receipt(&baseline_receipt);
    receipt.stream_head = changed.build();
    alternatives.push(("resource_revision", receipt.build()));

    let mut changed = StreamHeadFixture::from_head(baseline_receipt.stream_head());
    changed.resource_projection_digest = digest('8');
    let mut receipt = ResourceReceiptFixture::from_receipt(&baseline_receipt);
    receipt.stream_head = changed.build();
    alternatives.push(("resource_projection_digest", receipt.build()));

    let mut changed = StreamHeadFixture::from_head(baseline_receipt.stream_head());
    changed.head_digest = digest('8');
    let mut receipt = ResourceReceiptFixture::from_receipt(&baseline_receipt);
    receipt.stream_head = changed.build();
    alternatives.push(("stream_head_digest", receipt.build()));

    let mut changed = StreamHeadFixture::from_head(baseline_receipt.stream_head());
    changed.runtime = RuntimeKind::Live;
    let mut receipt = ResourceReceiptFixture::from_receipt(&baseline_receipt);
    receipt.runtime = RuntimeKind::Live;
    receipt.stream_head = changed.build();
    alternatives.push(("runtime", receipt.build()));

    let mut receipt = ResourceReceiptFixture::from_receipt(&baseline_receipt);
    receipt.observation_revision += 1;
    alternatives.push(("observation_revision", receipt.build()));

    let mut receipt = ResourceReceiptFixture::from_receipt(&baseline_receipt);
    receipt.effect_claim_id = "other-effect-claim".to_owned();
    alternatives.push(("effect_claim_id", receipt.build()));

    let mut receipt = ResourceReceiptFixture::from_receipt(&baseline_receipt);
    receipt.effect_subject_digest = digest('8');
    alternatives.push(("effect_subject_digest", receipt.build()));

    let mut receipt = ResourceReceiptFixture::from_receipt(&baseline_receipt);
    receipt.counters = ResourceCounters::new(2, 1, 0, 1, 0, "0").expect("counters");
    alternatives.push(("counters", receipt.build()));

    let mut receipt = ResourceReceiptFixture::from_receipt(&baseline_receipt);
    receipt.request = ResourceRequest::new(1, 0, 0, 0, 0, Some("0")).expect("request");
    alternatives.push(("request", receipt.build()));

    let mut changed = StreamHeadFixture::from_head(baseline_receipt.stream_head());
    changed.identity = TaskLedgerStreamIdentity::new(
        changed.identity.project_id().clone(),
        changed.identity.project_snapshot_id().clone(),
        changed.identity.task_id().clone(),
        changed.identity.task_revision(),
        changed.identity.task_spec_digest().clone(),
        "TWD",
    )
    .expect("identity");
    let mut receipt = ResourceReceiptFixture::from_receipt(&baseline_receipt);
    receipt.stream_head = changed.build();
    receipt.accounting_currency = "TWD".to_owned();
    alternatives.push(("accounting_currency", receipt.build()));

    let mut receipt = ResourceReceiptFixture::from_receipt(&baseline_receipt);
    receipt.observation_digest = digest('8');
    alternatives.push(("observation_digest", receipt.build()));

    let mut receipt = ResourceReceiptFixture::from_receipt(&baseline_receipt);
    receipt.receipt_digest = digest('8');
    alternatives.push(("receipt_digest", receipt.build()));

    for (field, alternate_receipt) in alternatives {
        let alternate_head = alternate_receipt.head();
        assert_ne!(
            alternate_head, baseline_head,
            "{field} must change the head"
        );
        let mut gate = write_gate(&spec);
        gate.resources.as_mut().expect("resources").current_head = Some(alternate_head);
        let decision = evaluate(DecisionSubject::AgentAction(gate));
        assert_eq!(
            decision.reason(),
            PolicyReason::ResourceEvidenceStale,
            "{field} substitution"
        );
        assert_eq!(
            decision.evidence().checked_through(),
            DecisionStage::Resources,
            "{field} substitution"
        );
        assert!(!decision.allowed(), "{field} substitution");
    }
}

#[test]
fn resource_decision_subject_rejects_every_field_substitution() {
    let spec = task_spec();
    let baseline = write_gate(&spec)
        .resource_subject
        .expect("resource subject");
    let mut alternatives = Vec::new();

    let mut subject = baseline.clone();
    subject.stream_id = digest('8');
    alternatives.push(("stream_id", subject));
    let mut subject = baseline.clone();
    subject.stream_head_digest = digest('8');
    alternatives.push(("stream_head_digest", subject));
    let mut subject = baseline.clone();
    subject.observation_revision += 1;
    alternatives.push(("observation_revision", subject));
    let mut subject = baseline.clone();
    subject.effect_claim_id = "other-effect-claim".to_owned();
    alternatives.push(("effect_claim_id", subject));
    let mut subject = baseline.clone();
    subject.effect_subject_digest = digest('8');
    alternatives.push(("effect_subject_digest", subject));
    let mut subject = baseline;
    subject.request = ResourceRequest::new(1, 0, 0, 0, 0, Some("0")).expect("request");
    alternatives.push(("request", subject));

    for (field, subject) in alternatives {
        let mut gate = write_gate(&spec);
        gate.resource_subject = Some(subject);
        let decision = evaluate(DecisionSubject::AgentAction(gate));
        assert_eq!(
            decision.reason(),
            PolicyReason::ResourceEvidenceInvalid,
            "{field} substitution"
        );
        assert_eq!(
            decision.evidence().checked_through(),
            DecisionStage::Resources,
            "{field} substitution"
        );
        assert!(!decision.allowed(), "{field} substitution");
    }
}

#[test]
fn fake_task_ledger_owner_head_composes_with_policy_and_invalidates_history() {
    let spec = task_spec();
    let identity = TaskLedgerStreamIdentity::new(
        ProjectId::new(spec.fields().project_id.clone()).expect("project"),
        spec.fields().project_snapshot_id.clone(),
        spec.fields().task_id.clone(),
        spec.fields().revision.clone(),
        spec_digest(&spec),
        spec.fields().budget.accounting_currency.clone(),
    )
    .expect("Ledger identity");
    let zero = FakeTaskLedger::zero_head(identity).expect("zero head");
    let command = |head, command_id: &str, kind, resource| {
        AppendCommand::new(
            head,
            CommandId::new(command_id).expect("command ID"),
            CorrelationId::new("policy-ledger-composition").expect("correlation"),
            "2026-07-29T10:00:00Z",
            kind,
            ActorId::new("runtime-supervisor").expect("actor"),
            ActionId::new("record-ledger-evidence").expect("action"),
            LedgerOutcome::Recorded,
            ReasonCode::new("POLICY_LEDGER_COMPOSITION").expect("reason"),
            digest('9'),
            None,
            resource,
        )
        .expect("append command")
    };

    let mut ledger = FakeTaskLedger::new();
    let created = ledger
        .execute(command(
            zero,
            "policy-ledger-created",
            LedgerEventKind::TaskCreated,
            None,
        ))
        .expect("created event");
    let counters = ResourceCounters::new(1, 1, 0, 1, 0, "0").expect("counters");
    let projected = ledger
        .execute(command(
            created.after().clone(),
            "policy-ledger-resource",
            LedgerEventKind::ResourceSnapshot,
            Some(ResourceSnapshot::new(counters)),
        ))
        .expect("resource event");
    let request = ResourceRequest::new(0, 0, 0, 0, 0, Some("0")).expect("request");
    let receipt = ledger
        .issue_resource_observation(
            projected.after().clone(),
            &EffectClaimId::new("effect-claim-task-011").expect("claim"),
            digest('5'),
            request,
        )
        .expect("owner receipt");
    let owner_head = ledger
        .current_resource_head(&receipt)
        .expect("independent current owner head");
    let fact = ResourceUsageFact {
        binding: binding(&spec),
        current_head: Some(owner_head),
        receipt: receipt.clone(),
    };
    let mut allowed = write_gate(&spec);
    replace_agent_resources(&mut allowed, fact);
    assert!(
        evaluate(DecisionSubject::AgentAction(allowed)).allowed(),
        "a receipt compared with the actual Fake owner head must pass"
    );

    ledger
        .execute(command(
            projected.after().clone(),
            "policy-ledger-later",
            LedgerEventKind::EvidenceRecorded,
            None,
        ))
        .expect("later event");
    let historical = ResourceUsageFact {
        binding: binding(&spec),
        current_head: ledger.current_resource_head(&receipt),
        receipt,
    };
    let mut denied = write_gate(&spec);
    replace_agent_resources(&mut denied, historical);
    assert_denial(
        evaluate(DecisionSubject::AgentAction(denied)),
        PolicyReason::ResourceEvidenceStale,
        DecisionStage::Resources,
    );
}

#[test]
fn merge_requires_fresh_exact_workspace_git_readiness_and_target_head() {
    let spec = spec_with(|input| {
        input.risk_class = RiskClass::R0;
        input.approval_requirements.merge = ApprovalRequirement::NotRequired;
    });
    assert!(
        evaluate(DecisionSubject::MergeGate(merge_gate(
            &spec,
            MergeTarget::FeatureBranch("refs/heads/integration".to_owned()),
        )))
        .allowed()
    );

    let mut wrong_storage_identity = merge_gate(
        &spec,
        MergeTarget::FeatureBranch("refs/heads/integration".to_owned()),
    );
    wrong_storage_identity
        .readiness
        .as_mut()
        .expect("readiness")
        .target_ref_identity =
        GitRefIdentity::new("refs/heads/integration", digest('0')).expect("target ref");
    assert_denial(
        evaluate(DecisionSubject::MergeGate(wrong_storage_identity)),
        PolicyReason::InvalidDecisionSubject,
        DecisionStage::Project,
    );

    let mut wrong_reference_identity = merge_gate(
        &spec,
        MergeTarget::FeatureBranch("refs/heads/integration".to_owned()),
    );
    wrong_reference_identity
        .readiness
        .as_mut()
        .expect("readiness")
        .target_ref_identity =
        GitRefIdentity::new("refs/heads/other", digest('f')).expect("target ref");
    assert_denial(
        evaluate(DecisionSubject::MergeGate(wrong_reference_identity)),
        PolicyReason::MergeReadinessMismatch,
        DecisionStage::Project,
    );

    let mut missing = merge_gate(
        &spec,
        MergeTarget::FeatureBranch("refs/heads/integration".to_owned()),
    );
    missing.readiness = None;
    assert_denial(
        evaluate(DecisionSubject::MergeGate(missing)),
        PolicyReason::MergeReadinessRequired,
        DecisionStage::Project,
    );

    let mut stale = merge_gate(
        &spec,
        MergeTarget::FeatureBranch("refs/heads/integration".to_owned()),
    );
    stale.readiness.as_mut().expect("readiness").fresh = false;
    assert_denial(
        evaluate(DecisionSubject::MergeGate(stale)),
        PolicyReason::MergeReadinessStale,
        DecisionStage::Project,
    );

    let mut changed_head = merge_gate(
        &spec,
        MergeTarget::FeatureBranch("refs/heads/integration".to_owned()),
    );
    changed_head.subject = MergeSubject::new(
        changed_head.subject.target().clone(),
        changed_head.subject.reviewed_commit(),
        "f".repeat(40),
        changed_head.subject.diff_digest().clone(),
    )
    .expect("merge subject");
    assert_denial(
        evaluate(DecisionSubject::MergeGate(changed_head)),
        PolicyReason::MergeReadinessMismatch,
        DecisionStage::Project,
    );

    let mut unknown_producer = merge_gate(
        &spec,
        MergeTarget::FeatureBranch("refs/heads/integration".to_owned()),
    );
    unknown_producer
        .readiness
        .as_mut()
        .expect("readiness")
        .producer = Boundary::Unknown;
    assert_denial(
        evaluate(DecisionSubject::MergeGate(unknown_producer)),
        PolicyReason::MergeReadinessMismatch,
        DecisionStage::Project,
    );
}

#[test]
fn dedicated_recovery_gate_separates_normal_and_guardian_authority() {
    let spec = task_spec();
    assert_normal_recovery_policy(&spec);
    assert_guardian_recovery_policy(&spec);
}

fn assert_normal_recovery_policy(spec: &TaskSpec) {
    assert!(
        evaluate(DecisionSubject::Recovery(normal_recovery_gate(spec))).allowed(),
        "exact resolved normal recovery may move only to stopped"
    );

    let mut normal_active = normal_recovery_gate(spec);
    let RecoverySubject::Normal(subject) = &mut normal_active.subject else {
        panic!("normal recovery");
    };
    subject.target_admission = RuntimeAdmission::Active;
    normal_active.authority.as_mut().expect("authority").subject = normal_active.subject.clone();
    assert_denial(
        evaluate(DecisionSubject::Recovery(normal_active)),
        PolicyReason::RuntimeAdmissionDenied,
        DecisionStage::Runtime,
    );

    let mut changed_subject = normal_recovery_gate(spec);
    let RecoverySubject::Normal(subject) = &mut changed_subject.subject else {
        panic!("normal recovery");
    };
    "other-claim".clone_into(&mut subject.effect_claim_id);
    assert_denial(
        evaluate(DecisionSubject::Recovery(changed_subject)),
        PolicyReason::RecoveryAuthorityMismatch,
        DecisionStage::Project,
    );

    let mut stale = normal_recovery_gate(spec);
    stale.authority.as_mut().expect("authority").fresh = false;
    assert_denial(
        evaluate(DecisionSubject::Recovery(stale)),
        PolicyReason::RecoveryAuthorityMismatch,
        DecisionStage::ProviderCapability,
    );
}

fn assert_guardian_recovery_policy(spec: &TaskSpec) {
    let guardian = guardian_recovery_gate(spec);
    assert!(
        evaluate(DecisionSubject::Recovery(guardian)).allowed(),
        "exact system-only guardian saga recovery should pass"
    );

    let mut user_project = guardian_recovery_gate(spec);
    set_project_class(
        user_project.context.project.as_mut().expect("project"),
        ProjectClass::UserProject,
    );
    assert_denial(
        evaluate(DecisionSubject::Recovery(user_project)),
        PolicyReason::ProtectedSurfaceRequired,
        DecisionStage::Project,
    );

    let mut wrong_owner = guardian_recovery_gate(spec);
    wrong_owner.authority.as_mut().expect("authority").owner =
        Boundary::Known(RecoveryOwner::RuntimeSupervisor);
    assert_denial(
        evaluate(DecisionSubject::Recovery(wrong_owner)),
        PolicyReason::RecoveryAuthorityMismatch,
        DecisionStage::ProviderCapability,
    );

    let mut substituted_guardian = guardian_recovery_gate(spec);
    "upgrade-guardian-substitute".clone_into(
        &mut substituted_guardian
            .authority
            .as_mut()
            .expect("authority")
            .producer_id,
    );
    assert_denial(
        evaluate(DecisionSubject::Recovery(substituted_guardian)),
        PolicyReason::RecoveryAuthorityMismatch,
        DecisionStage::ProviderCapability,
    );

    let mut mismatched_resolution = guardian_recovery_gate(spec);
    let RecoverySubject::GuardianRelease(subject) = &mut mismatched_resolution.subject else {
        panic!("guardian recovery");
    };
    "unreconciled-release".clone_into(&mut subject.resolution.active_release_id);
    mismatched_resolution
        .authority
        .as_mut()
        .expect("authority")
        .subject = mismatched_resolution.subject.clone();
    assert_denial(
        evaluate(DecisionSubject::Recovery(mismatched_resolution)),
        PolicyReason::InvalidDecisionSubject,
        DecisionStage::Input,
    );

    let mut protected_delta_bypass = guardian_recovery_gate(spec);
    let RecoverySubject::GuardianRelease(subject) = &mut protected_delta_bypass.subject else {
        panic!("guardian recovery");
    };
    subject.release = rebuild_release(
        &subject.release,
        None,
        None,
        None,
        None,
        None,
        Some(UpgradeDelta::new(
            false, true, false, false, false, false, false, false,
        )),
    );
    subject.resolution.activation.subject = ProtectedReleaseSubject::new(
        subject.release.clone(),
        subject.resolution.activation.subject.guardian().clone(),
    );
    protected_delta_bypass
        .authority
        .as_mut()
        .expect("authority")
        .subject = protected_delta_bypass.subject.clone();
    assert_denial(
        evaluate(DecisionSubject::Recovery(protected_delta_bypass)),
        PolicyReason::InvalidDecisionSubject,
        DecisionStage::Input,
    );
}

#[test]
fn rollback_reverses_preverified_slot_with_strictly_newer_epoch() {
    let spec = task_spec();
    assert!(
        evaluate(DecisionSubject::UpgradeStage(rollback_gate(&spec))).allowed(),
        "exact prior-slot rollback should pass without a new user approval"
    );

    let mut same_epoch = rollback_gate(&spec);
    let rollback = same_epoch.rollback.as_mut().expect("rollback");
    rollback.requested_epoch = rollback.current_epoch;
    assert_denial(
        evaluate(DecisionSubject::UpgradeStage(same_epoch)),
        PolicyReason::UpgradeStageDenied,
        DecisionStage::Input,
    );

    let mut wrong_slot = rollback_gate(&spec);
    let rollback = wrong_slot.rollback.as_mut().expect("rollback");
    rollback
        .target_slot_id
        .clone_from(&rollback.current_slot_id);
    assert_denial(
        evaluate(DecisionSubject::UpgradeStage(wrong_slot)),
        PolicyReason::UpgradeStageDenied,
        DecisionStage::Input,
    );

    let mut substituted_activation = rollback_gate(&spec);
    let failed_subject = &mut substituted_activation
        .rollback
        .as_mut()
        .expect("rollback")
        .failed_activation
        .subject;
    *failed_subject = ProtectedReleaseSubject::new(
        rebuild_release(
            failed_subject.release(),
            None,
            Some("other-failed-release"),
            None,
            None,
            None,
            None,
        ),
        failed_subject.guardian().clone(),
    );
    synchronize_rollback_facts(&mut substituted_activation);
    assert_denial(
        evaluate(DecisionSubject::UpgradeStage(substituted_activation)),
        PolicyReason::UpgradeStageDenied,
        DecisionStage::Input,
    );

    let mut substituted_guardian = rollback_gate(&spec);
    let failed_subject = &mut substituted_guardian
        .rollback
        .as_mut()
        .expect("rollback")
        .failed_activation
        .subject;
    *failed_subject = ProtectedReleaseSubject::new(
        failed_subject.release().clone(),
        rebuild_guardian_runtime(
            failed_subject.guardian(),
            Some("other-guardian"),
            None,
            None,
        ),
    );
    synchronize_rollback_facts(&mut substituted_guardian);
    assert_denial(
        evaluate(DecisionSubject::UpgradeStage(substituted_guardian)),
        PolicyReason::GuardianRequired,
        DecisionStage::Approval,
    );

    let mut replaced_evidence = rollback_gate(&spec);
    replaced_evidence
        .rollback
        .as_mut()
        .expect("rollback")
        .failure_evidence_digest = digest('1');
    assert_denial(
        evaluate(DecisionSubject::UpgradeStage(replaced_evidence)),
        PolicyReason::UpgradeStageDenied,
        DecisionStage::ProviderCapability,
    );

    let mut unverified_prior = rollback_gate(&spec);
    unverified_prior
        .evidence
        .as_mut()
        .expect("evidence")
        .prior_slot_verified = false;
    assert_denial(
        evaluate(DecisionSubject::UpgradeStage(unverified_prior)),
        PolicyReason::UpgradeStageDenied,
        DecisionStage::ProviderCapability,
    );
}

fn assert_denial(decision: PolicyDecision, reason: PolicyReason, stage: DecisionStage) {
    assert!(
        !decision.allowed(),
        "unexpected allow: {:?}",
        decision.reason()
    );
    assert_eq!(decision.reason(), reason);
    assert_eq!(decision.evidence().checked_through(), stage);
}

fn assert_denied_at(decision: PolicyDecision, stage: DecisionStage) {
    assert!(
        !decision.allowed(),
        "unexpected allow: {:?}",
        decision.reason()
    );
    assert_eq!(
        decision.evidence().checked_through(),
        stage,
        "unexpected reason: {:?}",
        decision.reason()
    );
}

fn digest(character: char) -> ContentDigest {
    ContentDigest::from_sha256(character.to_string().repeat(64)).expect("SHA-256 digest")
}

fn spec_digest(spec: &TaskSpec) -> ContentDigest {
    ContentDigest::from_sha256(spec.spec_hash().to_hex()).expect("Task Spec SHA-256")
}

fn binding(spec: &TaskSpec) -> SubjectBinding {
    SubjectBinding::new(
        ProjectId::new(spec.fields().project_id.clone()).expect("project"),
        spec.fields().project_snapshot_id.clone(),
        spec.fields().task_id.clone(),
        spec.fields().revision.clone(),
        spec_digest(spec),
    )
    .expect("binding")
}

fn binding_with_project_id(binding: &SubjectBinding, project_id: &str) -> SubjectBinding {
    SubjectBinding::new(
        ProjectId::new(project_id).expect("project"),
        binding.project_snapshot_id().clone(),
        binding.task_id().clone(),
        binding.task_revision(),
        binding.task_spec_digest().clone(),
    )
    .expect("binding")
}

fn binding_with_snapshot(
    binding: &SubjectBinding,
    project_snapshot_id: ProjectSnapshotId,
) -> SubjectBinding {
    SubjectBinding::new(
        binding.project_id().clone(),
        project_snapshot_id,
        binding.task_id().clone(),
        binding.task_revision(),
        binding.task_spec_digest().clone(),
    )
    .expect("binding")
}

fn binding_with_task(binding: &SubjectBinding, task_id: TaskId) -> SubjectBinding {
    SubjectBinding::new(
        binding.project_id().clone(),
        binding.project_snapshot_id().clone(),
        task_id,
        binding.task_revision(),
        binding.task_spec_digest().clone(),
    )
    .expect("binding")
}

fn binding_with_revision(binding: &SubjectBinding, task_revision: &str) -> SubjectBinding {
    SubjectBinding::new(
        binding.project_id().clone(),
        binding.project_snapshot_id().clone(),
        binding.task_id().clone(),
        task_revision,
        binding.task_spec_digest().clone(),
    )
    .expect("binding")
}

fn binding_with_digest(
    binding: &SubjectBinding,
    task_spec_digest: ContentDigest,
) -> SubjectBinding {
    SubjectBinding::new(
        binding.project_id().clone(),
        binding.project_snapshot_id().clone(),
        binding.task_id().clone(),
        binding.task_revision(),
        task_spec_digest,
    )
    .expect("binding")
}

fn rebuild_release(
    subject: &ReleaseSubject,
    saga_id: Option<&str>,
    release_id: Option<&str>,
    manifest_digest: Option<ContentDigest>,
    target_slot_id: Option<&str>,
    requested_epoch: Option<DaemonEpoch>,
    delta: Option<UpgradeDelta>,
) -> ReleaseSubject {
    ReleaseSubject::new(
        subject.activation_id(),
        saga_id.unwrap_or_else(|| subject.saga_id()),
        release_id.unwrap_or_else(|| subject.release_id()),
        subject.release_revision(),
        manifest_digest.unwrap_or_else(|| subject.manifest_digest().clone()),
        subject.source_commit(),
        subject.source_tree_digest().clone(),
        subject.dependency_lock_digest().clone(),
        subject.binary_digests().to_vec(),
        subject.migration_digests().to_vec(),
        subject.evidence_digest().clone(),
        subject.source_release_id(),
        subject.source_manifest_digest().clone(),
        subject.source_slot_id(),
        target_slot_id.unwrap_or_else(|| subject.target_slot_id()),
        requested_epoch.unwrap_or_else(|| subject.requested_epoch()),
        subject.schema_compatible(),
        delta.unwrap_or_else(|| subject.delta()),
    )
    .expect("release subject")
}

fn rebuild_guardian_runtime(
    subject: &GuardianRuntimeSubject,
    guardian_id: Option<&str>,
    trust_root_digest: Option<ContentDigest>,
    daemon_instance_id: Option<&str>,
) -> GuardianRuntimeSubject {
    GuardianRuntimeSubject::new(
        guardian_id.unwrap_or_else(|| subject.guardian_id()),
        trust_root_digest.unwrap_or_else(|| subject.trust_root_digest().clone()),
        daemon_instance_id.unwrap_or_else(|| subject.daemon_instance_id()),
        subject.observed_epoch(),
    )
    .expect("guardian runtime subject")
}

fn rebuild_external_cost(
    subject: &ExternalCostSubject,
    currency: Option<&str>,
    provider_id: Option<&str>,
    quote_digest: Option<ContentDigest>,
    pricing_digest: Option<ContentDigest>,
) -> ExternalCostSubject {
    ExternalCostSubject::new(
        subject.amount(),
        currency.unwrap_or_else(|| subject.currency()),
        provider_id.unwrap_or_else(|| subject.provider_id()),
        quote_digest.unwrap_or_else(|| subject.quote_digest().clone()),
        pricing_digest.unwrap_or_else(|| subject.pricing_digest().clone()),
    )
    .expect("external cost subject")
}

fn project_fact(spec: &TaskSpec) -> ProjectAuthorityFact {
    let runtime = match spec.fields().runtime_profile {
        RuntimeProfile::Fake => RuntimeKind::Fake,
        RuntimeProfile::Codex => RuntimeKind::Live,
    };
    let receipt = ProjectAuthorityReceipt::new(
        CONTRACT_VERSION,
        "lattice-project-registry",
        "1.0",
        runtime,
        ProjectId::new(spec.fields().project_id.clone()).expect("project"),
        spec.fields().project_snapshot_id.clone(),
        1,
        ProjectLifecycle::Active,
        ProjectClass::UserProject,
        GitRefIdentity::new("refs/heads/main", digest('0')).expect("primary branch"),
        digest('7'),
        digest('8'),
    )
    .expect("project authority receipt");
    ProjectAuthorityFact {
        binding: binding(spec),
        current_head: receipt.head(),
        receipt,
    }
}

fn replace_project_receipt(
    project: &mut ProjectAuthorityFact,
    lifecycle: ProjectLifecycle,
    project_class: ProjectClass,
    receipt_byte: char,
) {
    let previous = &project.receipt;
    let receipt = ProjectAuthorityReceipt::new(
        previous.version(),
        previous.producer_id(),
        previous.producer_version(),
        previous.runtime(),
        previous.project_id().clone(),
        previous.project_snapshot_id().clone(),
        previous
            .registry_revision()
            .checked_add(1)
            .expect("fixture revision"),
        lifecycle,
        project_class,
        previous.primary_branch().clone(),
        previous.observation_digest().clone(),
        digest(receipt_byte),
    )
    .expect("replacement project authority receipt");
    project.current_head = receipt.head();
    project.receipt = receipt;
}

fn set_project_suspended(project: &mut ProjectAuthorityFact) {
    replace_project_receipt(
        project,
        ProjectLifecycle::Suspended,
        project.receipt.project_class(),
        '9',
    );
}

fn set_project_drifted(project: &mut ProjectAuthorityFact) {
    replace_project_receipt(
        project,
        ProjectLifecycle::ReconciliationRequired,
        project.receipt.project_class(),
        'a',
    );
}

fn set_project_stale(project: &mut ProjectAuthorityFact) {
    let previous = &project.receipt;
    project.current_head = ProjectAuthorityReceipt::new(
        previous.version(),
        previous.producer_id(),
        previous.producer_version(),
        previous.runtime(),
        previous.project_id().clone(),
        previous.project_snapshot_id().clone(),
        previous
            .registry_revision()
            .checked_add(1)
            .expect("fixture revision"),
        previous.lifecycle(),
        previous.project_class(),
        previous.primary_branch().clone(),
        previous.observation_digest().clone(),
        digest('b'),
    )
    .expect("new current project authority receipt")
    .head();
}

fn set_project_class(project: &mut ProjectAuthorityFact, project_class: ProjectClass) {
    replace_project_receipt(project, project.receipt.lifecycle(), project_class, 'c');
}

fn context(
    spec: &TaskSpec,
    state: TaskState,
    runtime_admission: RuntimeAdmission,
) -> TaskContext<'_> {
    TaskContext {
        task_spec: Some(spec),
        project: Some(project_fact(spec)),
        state: Boundary::Known(state),
        runtime_admission: Boundary::Known(runtime_admission),
    }
}

fn safe_usage() -> ResourceUsage {
    ResourceUsage {
        active_agents: 1,
        requested_agents: 0,
        active_implementers: 0,
        requested_implementers: 0,
        elapsed_seconds: 0,
        requested_duration_seconds: 0,
        attempt_number: 1,
        requested_attempts: 0,
        used_model_calls: 0,
        requested_model_calls: 0,
        used_external_cost: "0".to_owned(),
        requested_external_cost: Some("0".to_owned()),
    }
}

fn resource_fact(spec: &TaskSpec, usage: ResourceUsage) -> ResourceUsageFact {
    resource_fact_with_currency(
        spec,
        usage,
        spec.fields().budget.accounting_currency.as_str(),
    )
}

fn resource_fact_with_currency(
    spec: &TaskSpec,
    usage: ResourceUsage,
    accounting_currency: &str,
) -> ResourceUsageFact {
    let runtime = match spec.fields().runtime_profile {
        RuntimeProfile::Fake => RuntimeKind::Fake,
        RuntimeProfile::Codex => RuntimeKind::Live,
    };
    let identity = TaskLedgerStreamIdentity::new(
        ProjectId::new(spec.fields().project_id.clone()).expect("project"),
        spec.fields().project_snapshot_id.clone(),
        spec.fields().task_id.clone(),
        spec.fields().revision.clone(),
        spec_digest(spec),
        accounting_currency,
    )
    .expect("Ledger identity");
    let stream_head = TaskLedgerStreamHead::new(
        CONTRACT_VERSION,
        TASK_LEDGER_PRODUCER_ID,
        TASK_LEDGER_PRODUCER_VERSION,
        runtime,
        identity,
        digest('1'),
        7,
        digest('2'),
        3,
        digest('3'),
        digest('4'),
    )
    .expect("Ledger head");
    let counters = ResourceCounters::new(
        usage.active_agents,
        usage.active_implementers,
        usage.elapsed_seconds,
        usage.attempt_number,
        usage.used_model_calls,
        usage.used_external_cost,
    )
    .expect("resource counters");
    let request = ResourceRequest::new(
        usage.requested_agents,
        usage.requested_implementers,
        usage.requested_duration_seconds,
        usage.requested_attempts,
        usage.requested_model_calls,
        usage.requested_external_cost,
    )
    .expect("resource request");
    let receipt = TaskLedgerResourceReceipt::new(
        CONTRACT_VERSION,
        TASK_LEDGER_PRODUCER_ID,
        TASK_LEDGER_PRODUCER_VERSION,
        runtime,
        stream_head,
        3,
        "effect-claim-task-011",
        digest('5'),
        counters,
        request,
        accounting_currency,
        digest('6'),
        digest('7'),
    )
    .expect("resource receipt");
    ResourceUsageFact {
        binding: binding(spec),
        current_head: Some(receipt.head()),
        receipt,
    }
}

fn resource_fact_from_receipt(
    spec: &TaskSpec,
    receipt: TaskLedgerResourceReceipt,
) -> ResourceUsageFact {
    ResourceUsageFact {
        binding: binding(spec),
        current_head: Some(receipt.head()),
        receipt,
    }
}

fn resource_observation_from_fact(fact: &ResourceUsageFact) -> ResourceObservationSubject {
    ResourceObservationSubject {
        stream_id: fact.receipt.stream_head().stream_id().clone(),
        stream_head_digest: fact.receipt.stream_head().head_digest().clone(),
        observation_revision: fact.receipt.observation_revision(),
        effect_claim_id: fact.receipt.effect_claim_id().to_owned(),
        effect_subject_digest: fact.receipt.effect_subject_digest().clone(),
        request: fact.receipt.request().clone(),
    }
}

fn resource_observation(spec: &TaskSpec) -> ResourceObservationSubject {
    resource_observation_from_fact(&resource_fact(spec, safe_usage()))
}

fn replace_agent_resources(gate: &mut AgentActionGate<'_>, fact: ResourceUsageFact) {
    gate.resource_subject = Some(resource_observation_from_fact(&fact));
    gate.resources = Some(fact);
}

fn replace_merge_resources(gate: &mut MergeGate<'_>, fact: ResourceUsageFact) {
    gate.resource_subject = resource_observation_from_fact(&fact);
    gate.resources = fact;
}

fn worker_gate(
    spec: &TaskSpec,
    workers: Vec<AgentRole>,
    usage: ResourceUsage,
) -> WorkerAdmissionGate<'_> {
    let resources = resource_fact(spec, usage);
    let resource_subject = resource_observation_from_fact(&resources);
    WorkerAdmissionGate {
        context: context(spec, TaskState::Draft, RuntimeAdmission::Active),
        workers,
        resource_subject,
        resources,
    }
}

fn agent_gate(
    spec: &TaskSpec,
    role: AgentRole,
    action: PolicyAction,
    state: TaskState,
    runtime_admission: RuntimeAdmission,
) -> AgentActionGate<'_> {
    let resources = resource_fact(spec, safe_usage());
    let resource_subject = resource_observation_from_fact(&resources);
    AgentActionGate {
        context: context(spec, state, runtime_admission),
        role: Boundary::Known(role),
        action: Boundary::Known(action),
        actor_id: format!("{}-1", role.as_str().to_ascii_lowercase()),
        approval: None,
        provider_capability: None,
        external_cost: None,
        writer_subject: None,
        writer: None,
        resource_subject: Some(resource_subject),
        resources: Some(resources),
        network: NetworkIntent::None,
        deployment: DeploymentIntent::None,
    }
}

fn writer_identity_fixture(spec: &TaskSpec) -> WriterIdentityFixture {
    WriterIdentityFixture {
        project_id: ProjectId::new(spec.fields().project_id.clone()).expect("project"),
        project_snapshot_id: spec.fields().project_snapshot_id.clone(),
        task_id: spec.fields().task_id.clone(),
        task_revision: spec.fields().revision.clone(),
        task_spec_digest: ContentDigest::from_sha256(spec.spec_hash().to_hex())
            .expect("Task Spec digest"),
        attempt_id: AttemptId::new("attempt-task-014").expect("attempt"),
        lease_id: "lease-task-014".to_owned(),
        lease_holder_id: "implementer-1".to_owned(),
        worktree_id: "worktree-task-014".to_owned(),
        holder_process_id: HolderProcessId::new(4_242).expect("process ID"),
        holder_process_start_identity: digest('9'),
        daemon_instance_id: "daemon-1".to_owned(),
        daemon_epoch: DaemonEpoch::new(7).expect("daemon epoch"),
        fencing_token: FencingToken::new(11).expect("fencing token"),
    }
}

fn build_writer_identity(fixture: &WriterIdentityFixture) -> WriterLeaseIdentity {
    WriterLeaseIdentity::new(
        fixture.project_id.clone(),
        fixture.project_snapshot_id.clone(),
        fixture.task_id.clone(),
        fixture.task_revision.clone(),
        fixture.task_spec_digest.clone(),
        fixture.attempt_id.clone(),
        fixture.lease_id.clone(),
        fixture.lease_holder_id.clone(),
        fixture.worktree_id.clone(),
        fixture.holder_process_id,
        fixture.holder_process_start_identity.clone(),
        fixture.daemon_instance_id.clone(),
        fixture.daemon_epoch,
        fixture.fencing_token,
    )
    .expect("valid Writer Lease identity")
}

fn mutate_writer_identity(
    baseline: &WriterIdentityFixture,
    mutation: impl FnOnce(&mut WriterIdentityFixture),
) -> WriterIdentityFixture {
    let mut substituted = baseline.clone();
    mutation(&mut substituted);
    substituted
}

fn writer_identity_substitutions(
    baseline: &WriterIdentityFixture,
) -> Vec<(&'static str, WriterIdentityFixture)> {
    vec![
        (
            "project_id",
            mutate_writer_identity(baseline, |identity| {
                identity.project_id = ProjectId::new("other-project").expect("project");
            }),
        ),
        (
            "project_snapshot_id",
            mutate_writer_identity(baseline, |identity| {
                identity.project_snapshot_id =
                    ProjectSnapshotId::new("other-snapshot").expect("snapshot");
            }),
        ),
        (
            "task_id",
            mutate_writer_identity(baseline, |identity| {
                identity.task_id = TaskId::new("TASK-OTHER-014").expect("task");
            }),
        ),
        (
            "task_revision",
            mutate_writer_identity(baseline, |identity| {
                "2".clone_into(&mut identity.task_revision);
            }),
        ),
        (
            "task_spec_digest",
            mutate_writer_identity(baseline, |identity| {
                identity.task_spec_digest = digest('1');
            }),
        ),
        (
            "attempt_id",
            mutate_writer_identity(baseline, |identity| {
                identity.attempt_id = AttemptId::new("attempt-other").expect("attempt");
            }),
        ),
        (
            "lease_id",
            mutate_writer_identity(baseline, |identity| {
                "lease-other".clone_into(&mut identity.lease_id);
            }),
        ),
        (
            "lease_holder_id",
            mutate_writer_identity(baseline, |identity| {
                "implementer-2".clone_into(&mut identity.lease_holder_id);
            }),
        ),
        (
            "worktree_id",
            mutate_writer_identity(baseline, |identity| {
                "worktree-other".clone_into(&mut identity.worktree_id);
            }),
        ),
        (
            "holder_process_id",
            mutate_writer_identity(baseline, |identity| {
                identity.holder_process_id = HolderProcessId::new(4_243).expect("process");
            }),
        ),
        (
            "holder_process_start_identity",
            mutate_writer_identity(baseline, |identity| {
                identity.holder_process_start_identity = digest('8');
            }),
        ),
        (
            "daemon_instance_id",
            mutate_writer_identity(baseline, |identity| {
                "daemon-2".clone_into(&mut identity.daemon_instance_id);
            }),
        ),
        (
            "daemon_epoch",
            mutate_writer_identity(baseline, |identity| {
                identity.daemon_epoch = DaemonEpoch::new(8).expect("epoch");
            }),
        ),
        (
            "fencing_token",
            mutate_writer_identity(baseline, |identity| {
                identity.fencing_token = FencingToken::new(12).expect("fence");
            }),
        ),
    ]
}

fn writer_receipt_fixture(spec: &TaskSpec) -> WriterReceiptFixture {
    WriterReceiptFixture {
        identity: writer_identity_fixture(spec),
        runtime: match spec.fields().runtime_profile {
            RuntimeProfile::Fake => RuntimeKind::Fake,
            RuntimeProfile::Codex => RuntimeKind::Live,
        },
        status: WriterLeaseStatus::Active,
        revision: WriterLeaseRevision::new(1).expect("revision"),
        runtime_admission: RuntimeAdmissionMode::Active,
        acquired_at: "2026-07-29T00:00:00Z".to_owned(),
        heartbeat_at: "2026-07-29T00:01:00Z".to_owned(),
        expires_at: "2026-07-29T00:02:00Z".to_owned(),
        time_observation_digest: digest('a'),
        admission_observation_digest: digest('b'),
        transition_digest: digest('c'),
        receipt_digest: digest('d'),
    }
}

fn mutate_writer_receipt(
    baseline: &WriterReceiptFixture,
    mutation: impl FnOnce(&mut WriterReceiptFixture),
) -> WriterReceiptFixture {
    let mut substituted = baseline.clone();
    mutation(&mut substituted);
    substituted
}

fn writer_receipt_substitutions(
    baseline: &WriterReceiptFixture,
) -> Vec<(&'static str, WriterReceiptFixture)> {
    let mut substitutions = writer_identity_substitutions(&baseline.identity)
        .into_iter()
        .map(|(field, identity)| {
            (
                field,
                mutate_writer_receipt(baseline, |receipt| receipt.identity = identity),
            )
        })
        .collect::<Vec<_>>();

    substitutions.extend([
        (
            "runtime",
            mutate_writer_receipt(baseline, |receipt| receipt.runtime = RuntimeKind::Live),
        ),
        (
            "status",
            mutate_writer_receipt(baseline, |receipt| {
                receipt.status = WriterLeaseStatus::Suspect;
            }),
        ),
        (
            "revision",
            mutate_writer_receipt(baseline, |receipt| {
                receipt.revision = WriterLeaseRevision::new(2).expect("revision");
            }),
        ),
        (
            "runtime_admission",
            mutate_writer_receipt(baseline, |receipt| {
                receipt.runtime_admission = RuntimeAdmissionMode::Draining;
            }),
        ),
        (
            "acquired_at",
            mutate_writer_receipt(baseline, |receipt| {
                "2026-07-29T00:00:01Z".clone_into(&mut receipt.acquired_at);
            }),
        ),
        (
            "heartbeat_at",
            mutate_writer_receipt(baseline, |receipt| {
                "2026-07-29T00:01:01Z".clone_into(&mut receipt.heartbeat_at);
            }),
        ),
        (
            "expires_at",
            mutate_writer_receipt(baseline, |receipt| {
                "2026-07-29T00:02:01Z".clone_into(&mut receipt.expires_at);
            }),
        ),
        (
            "time_observation_digest",
            mutate_writer_receipt(baseline, |receipt| {
                receipt.time_observation_digest = digest('e');
            }),
        ),
        (
            "admission_observation_digest",
            mutate_writer_receipt(baseline, |receipt| {
                receipt.admission_observation_digest = digest('f');
            }),
        ),
        (
            "transition_digest",
            mutate_writer_receipt(baseline, |receipt| {
                receipt.transition_digest = digest('2');
            }),
        ),
        (
            "receipt_digest",
            mutate_writer_receipt(baseline, |receipt| {
                receipt.receipt_digest = digest('3');
            }),
        ),
    ]);

    substitutions
}

fn build_writer_receipt(fixture: &WriterReceiptFixture) -> WriterLeaseAuthorityReceipt {
    WriterLeaseAuthorityReceipt::new(
        CONTRACT_VERSION,
        WRITER_LEASE_PRODUCER_ID,
        WRITER_LEASE_PRODUCER_VERSION,
        fixture.runtime,
        build_writer_identity(&fixture.identity),
        fixture.status,
        fixture.revision,
        fixture.runtime_admission,
        fixture.acquired_at.clone(),
        fixture.heartbeat_at.clone(),
        fixture.expires_at.clone(),
        fixture.time_observation_digest.clone(),
        fixture.admission_observation_digest.clone(),
        fixture.transition_digest.clone(),
        fixture.receipt_digest.clone(),
    )
    .expect("valid Writer Lease authority receipt")
}

fn writer_subject(identity: &WriterLeaseIdentity, runtime: RuntimeKind) -> WriterLeaseSubject {
    WriterLeaseSubject {
        lease_holder_id: identity.lease_holder_id().to_owned(),
        lease_id: identity.lease_id().to_owned(),
        attempt_id: identity.attempt_id().clone(),
        worktree_id: identity.worktree_id().to_owned(),
        holder_process_id: identity.holder_process_id(),
        holder_process_start_identity: identity.holder_process_start_identity().to_owned(),
        daemon_instance_id: identity.daemon_instance_id().to_owned(),
        daemon_epoch: identity.daemon_epoch(),
        fencing_token: identity.fencing_token(),
        runtime,
    }
}

fn mutate_writer_subject(
    baseline: &WriterLeaseSubject,
    mutation: impl FnOnce(&mut WriterLeaseSubject),
) -> WriterLeaseSubject {
    let mut substituted = baseline.clone();
    mutation(&mut substituted);
    substituted
}

fn writer_subject_substitutions(
    baseline: &WriterLeaseSubject,
) -> Vec<(&'static str, WriterLeaseSubject, PolicyReason)> {
    let mismatch = PolicyReason::WriterLeaseSubjectMismatch;
    vec![
        (
            "holder",
            mutate_writer_subject(baseline, |subject| {
                "implementer-2".clone_into(&mut subject.lease_holder_id);
            }),
            mismatch,
        ),
        (
            "lease",
            mutate_writer_subject(baseline, |subject| {
                "lease-substituted".clone_into(&mut subject.lease_id);
            }),
            mismatch,
        ),
        (
            "attempt",
            mutate_writer_subject(baseline, |subject| {
                subject.attempt_id =
                    AttemptId::new("attempt-substituted").expect("attempt identity");
            }),
            mismatch,
        ),
        (
            "worktree",
            mutate_writer_subject(baseline, |subject| {
                "substituted-worktree".clone_into(&mut subject.worktree_id);
            }),
            mismatch,
        ),
        (
            "process_id",
            mutate_writer_subject(baseline, |subject| {
                subject.holder_process_id = HolderProcessId::new(4_243).expect("process ID");
            }),
            mismatch,
        ),
        (
            "process_start",
            mutate_writer_subject(baseline, |subject| {
                subject.holder_process_start_identity = digest('8');
            }),
            mismatch,
        ),
        (
            "daemon",
            mutate_writer_subject(baseline, |subject| {
                "substituted-daemon".clone_into(&mut subject.daemon_instance_id);
            }),
            mismatch,
        ),
        (
            "epoch",
            mutate_writer_subject(baseline, |subject| {
                subject.daemon_epoch = DaemonEpoch::new(8).expect("epoch");
            }),
            mismatch,
        ),
        (
            "fence",
            mutate_writer_subject(baseline, |subject| {
                subject.fencing_token = FencingToken::new(12).expect("fencing token");
            }),
            PolicyReason::FencingTokenMismatch,
        ),
        (
            "runtime",
            mutate_writer_subject(baseline, |subject| {
                subject.runtime = RuntimeKind::Live;
            }),
            mismatch,
        ),
    ]
}

fn writer_fact_from_fixture(fixture: &WriterReceiptFixture) -> WriterLeaseFact {
    let receipt = build_writer_receipt(fixture);
    WriterLeaseFact {
        current_head: Some(receipt.head()),
        receipt,
    }
}

fn writer_fact(spec: &TaskSpec) -> WriterLeaseFact {
    writer_fact_from_fixture(&writer_receipt_fixture(spec))
}

fn writer_subject_from_fact(fact: &WriterLeaseFact) -> WriterLeaseSubject {
    writer_subject(fact.receipt.identity(), fact.receipt.runtime())
}

fn write_gate(spec: &TaskSpec) -> AgentActionGate<'_> {
    let mut gate = agent_gate(
        spec,
        AgentRole::Implementer,
        PolicyAction::WriteProductCode,
        TaskState::Executing,
        RuntimeAdmission::Active,
    );
    "implementer-1".clone_into(&mut gate.actor_id);
    let writer = writer_fact(spec);
    gate.writer_subject = Some(writer_subject_from_fact(&writer));
    gate.writer = Some(writer);
    replace_agent_resources(
        &mut gate,
        resource_fact(
            spec,
            ResourceUsage {
                active_implementers: 1,
                ..safe_usage()
            },
        ),
    );
    gate
}

fn provider_fact(
    spec: &TaskSpec,
    provider: ProviderKind,
    capability: Capability,
    runtime: RuntimeKind,
) -> ProviderCapabilityFact {
    ProviderCapabilityFact {
        binding: binding(spec),
        provider,
        capability,
        contract_version: 1,
        runtime,
        provider_id: match provider {
            ProviderKind::Codex => "codex-local",
            ProviderKind::Graphify => "graphify-local",
            ProviderKind::Hermes => "hermes-local",
        }
        .to_owned(),
        provider_version: "1.0.0".to_owned(),
        expected_executable_digest: digest('b'),
        observed_executable_digest: digest('b'),
        expected_schema_digest: digest('c'),
        observed_schema_digest: digest('c'),
        available: true,
        identity_verified: true,
        boundary_verified: true,
        fresh: true,
    }
}

fn codex_gate(spec: &TaskSpec) -> AgentActionGate<'_> {
    let mut gate = agent_gate(
        spec,
        AgentRole::Implementer,
        PolicyAction::RunCodex,
        TaskState::Executing,
        RuntimeAdmission::Active,
    );
    "implementer-1".clone_into(&mut gate.actor_id);
    gate.provider_capability = Some(provider_fact(
        spec,
        ProviderKind::Codex,
        Capability::UseCodex,
        RuntimeKind::Live,
    ));
    let writer = writer_fact(spec);
    gate.writer_subject = Some(writer_subject_from_fact(&writer));
    gate.writer = Some(writer);
    replace_agent_resources(
        &mut gate,
        resource_fact(
            spec,
            ResourceUsage {
                active_implementers: 1,
                ..safe_usage()
            },
        ),
    );
    gate
}

fn graphify_gate(spec: &TaskSpec) -> AgentActionGate<'_> {
    let mut gate = agent_gate(
        spec,
        AgentRole::CodeMapper,
        PolicyAction::RunGraphify,
        TaskState::Draft,
        RuntimeAdmission::Active,
    );
    gate.provider_capability = Some(provider_fact(
        spec,
        ProviderKind::Graphify,
        Capability::UseGraphify,
        RuntimeKind::Fake,
    ));
    gate
}

fn execution_subject(
    spec: &TaskSpec,
    external_cost: Option<ExternalCostSubject>,
) -> ApprovalSubject {
    ApprovalSubject::Execution {
        task_spec_hash: spec_digest(spec),
        external_cost,
    }
}

fn external_cost_subject(amount: &str, currency: &str, provider_id: &str) -> ExternalCostSubject {
    ExternalCostSubject::new(amount, currency, provider_id, digest('a'), digest('b'))
        .expect("external cost subject")
}

fn attach_external_cost(
    spec: &TaskSpec,
    gate: &mut AgentActionGate<'_>,
    amount: &str,
    provider_id: &str,
) {
    let subject = external_cost_subject(
        amount,
        &spec.fields().budget.accounting_currency,
        provider_id,
    );
    gate.external_cost = Some(ExternalCostFact {
        binding: binding(spec),
        subject: subject.clone(),
        quote_verified: true,
        fresh: true,
    });
    gate.approval = Some(approval_fact(
        spec,
        ApprovalKind::Execution,
        ApprovalAuthority::ResponsibleUser,
        execution_subject(spec, Some(subject)),
    ));
}

fn approval_fact(
    spec: &TaskSpec,
    kind: ApprovalKind,
    authority: ApprovalAuthority,
    subject: ApprovalSubject,
) -> ApprovalFact {
    approval_fact_with_binding(spec, binding(spec), kind, authority, subject)
}

fn approval_fact_with_binding(
    spec: &TaskSpec,
    approval_binding: SubjectBinding,
    kind: ApprovalKind,
    authority: ApprovalAuthority,
    subject: ApprovalSubject,
) -> ApprovalFact {
    assert_eq!(
        kind,
        subject.kind(),
        "fixture kind must derive from subject"
    );
    let (origin, lane, status) = match authority {
        ApprovalAuthority::ResponsibleUser => (
            ApprovalOrigin::OsAuthenticatedUser,
            ApprovalLane::Normal,
            ApprovalStatus::Available,
        ),
        ApprovalAuthority::ProtectedGuardian => (
            ApprovalOrigin::GuardianTrustRoot,
            ApprovalLane::Protected,
            ApprovalStatus::ProtectedPendingClaim,
        ),
        ApprovalAuthority::InternalPolicy => {
            panic!("internal Policy is not an external approval authority")
        }
    };
    let runtime = match spec.fields().runtime_profile {
        RuntimeProfile::Fake => RuntimeKind::Fake,
        RuntimeProfile::Codex => RuntimeKind::Live,
    };
    let identity = ApprovalIdentity::new(
        "approval-task-011",
        "challenge-task-011",
        approval_binding,
        subject,
        "requester-task-011",
        "responsible-actor-1",
        authority,
        origin,
        lane,
        "local-os-channel",
        "session-task-011",
    )
    .expect("approval identity");
    let receipt = ApprovalAuthorityReceipt::new(
        CONTRACT_VERSION,
        APPROVAL_VERIFIER_PRODUCER_ID,
        APPROVAL_VERIFIER_PRODUCER_VERSION,
        runtime,
        identity,
        ApprovalRevision::new(1).expect("approval revision"),
        status,
        "nonce-task-011",
        digest('1'),
        "2026-07-29T08:00:00Z",
        "2026-07-29T09:00:00Z",
        digest('2'),
        digest('3'),
        "authenticator-task-011",
        "key-task-011",
        digest('4'),
        digest('5'),
        None,
        digest('6'),
    )
    .expect("approval receipt");
    ApprovalFact {
        current_head: Some(receipt.head()),
        receipt,
    }
}

fn approval_head_with_status(
    receipt: &ApprovalAuthorityReceipt,
    status: ApprovalStatus,
) -> ApprovalAuthorityHead {
    ApprovalAuthorityHead::new(
        receipt.version(),
        receipt.producer_id(),
        receipt.producer_version(),
        receipt.runtime(),
        receipt.identity().clone(),
        receipt.revision(),
        status,
        receipt.nonce_id(),
        receipt.nonce_commitment().clone(),
        receipt.issued_at(),
        receipt.expires_at(),
        receipt.subject_digest().clone(),
        receipt.challenge_digest().clone(),
        receipt.authenticator_id(),
        receipt.key_id(),
        receipt.proof_digest().clone(),
        receipt.evidence_digest().clone(),
        receipt.review_set_digest().cloned(),
        receipt.receipt_digest().clone(),
    )
    .expect("approval authority head")
}

fn merge_gate(spec: &TaskSpec, target: MergeTarget) -> MergeGate<'_> {
    let subject = MergeSubject::new(target, "d".repeat(40), "c".repeat(40), digest('e'))
        .expect("merge subject");
    let target_reference = subject.target().reference().unwrap_or("unbound").to_owned();
    let storage_identity_digest = if target_reference.eq_ignore_ascii_case("refs/heads/main") {
        digest('0')
    } else {
        digest('f')
    };
    let target_ref_identity =
        GitRefIdentity::new(target_reference, storage_identity_digest.clone()).unwrap_or_else(
            |_| {
                GitRefIdentity::new("refs/heads/invalid-target-fixture", storage_identity_digest)
                    .expect("valid fallback target ref")
            },
        );
    let readiness = MergeReadinessFact {
        binding: binding(spec),
        subject: subject.clone(),
        producer: Boundary::Known(MergeAnalysisProducer::WorkspaceGit),
        producer_id: "workspace-git-1".to_owned(),
        producer_version: "1.0.0".to_owned(),
        target_ref_identity,
        analysis_digest: digest('a'),
        scope_evidence_digest: digest('b'),
        scope_verified: true,
        conflict_free: true,
        fresh: true,
    };
    MergeGate {
        context: context(
            spec,
            TaskState::AwaitingMergeApproval,
            RuntimeAdmission::Active,
        ),
        role: Boundary::Known(AgentRole::Integrator),
        subject,
        readiness: Some(readiness),
        approval: None,
        resource_subject: resource_observation(spec),
        resources: resource_fact(spec, safe_usage()),
    }
}

fn memory_gate(spec: &TaskSpec, kind: MemoryKind) -> MemoryPromotionGate<'_> {
    let subject = MemoryCandidateSubject::new(binding(spec), digest('9'), kind)
        .expect("memory candidate subject");
    let review = MemoryReviewFact {
        subject: subject.clone(),
        provenance_digest: digest('4'),
        schema_digest: digest('5'),
        reviewer_id: "memory-reviewer-1".to_owned(),
        immutable_provenance: true,
        schema_valid: true,
        review_accepted: true,
        fresh: true,
    };
    let preference_user_approval = (kind == MemoryKind::Preference).then(|| {
        approval_fact(
            spec,
            ApprovalKind::Preference,
            ApprovalAuthority::ResponsibleUser,
            ApprovalSubject::Preference(subject.clone()),
        )
    });
    MemoryPromotionGate {
        context: context(spec, TaskState::Reviewing, RuntimeAdmission::Active),
        role: Boundary::Known(AgentRole::MemoryReviewer),
        subject,
        review: Some(review),
        claims_authority: false,
        preference_user_approval,
    }
}

fn activation_gate(spec: &TaskSpec) -> UpgradeGate<'_> {
    let subject = ReleaseSubject::new(
        "activation-task-011",
        "saga-task-011",
        "release-task-011",
        "1",
        digest('8'),
        "d".repeat(40),
        digest('1'),
        digest('2'),
        vec![digest('3')],
        Vec::new(),
        digest('4'),
        "release-task-010",
        digest('7'),
        "slot-a",
        "slot-b",
        DaemonEpoch::new(8).expect("release epoch"),
        true,
        UpgradeDelta::default(),
    )
    .expect("release subject");
    let evidence = UpgradeEvidenceFact {
        binding: binding(spec),
        subject: subject.clone(),
        rollback: None,
        candidate_immutable: true,
        inactive_slot_verified: true,
        prior_slot_verified: true,
        saga_bound: true,
        epoch_bound: true,
        fresh: true,
    };
    let guardian_runtime = GuardianRuntimeSubject::new(
        "upgrade-guardian-1",
        digest('6'),
        "guardian-daemon-1",
        subject.requested_epoch(),
    )
    .expect("guardian runtime subject");
    let guardian = GuardianAuthorityFact {
        binding: binding(spec),
        subject: subject.clone(),
        rollback: None,
        origin: Boundary::Known(ApprovalOrigin::GuardianTrustRoot),
        runtime: guardian_runtime.clone(),
        identity_verified: true,
        fresh: true,
        reserved_system_stream: true,
        user_project_access: false,
    };
    let approval = approval_fact(
        spec,
        ApprovalKind::ProtectedRelease,
        ApprovalAuthority::ProtectedGuardian,
        ApprovalSubject::ProtectedRelease(Box::new(ProtectedReleaseSubject::new(
            subject.clone(),
            guardian_runtime,
        ))),
    );
    let mut upgrade_context = context(spec, TaskState::Merging, RuntimeAdmission::Draining);
    set_project_class(
        upgrade_context.project.as_mut().expect("project"),
        ProjectClass::LatticeSystem,
    );
    UpgradeGate {
        context: upgrade_context,
        role: Boundary::Known(AgentRole::UpgradeGuardian),
        stage: UpgradeStage::Activate,
        subject,
        rollback: None,
        evidence: Some(evidence),
        guardian: Some(guardian),
        approval: Some(approval),
    }
}

fn rollback_gate(spec: &TaskSpec) -> UpgradeGate<'_> {
    let mut gate = activation_gate(spec);
    let failed_activation = ProtectedActivationReceipt {
        subject: ProtectedReleaseSubject::new(
            gate.subject.clone(),
            gate.guardian.as_ref().expect("guardian").runtime.clone(),
        ),
        approval_id: gate
            .approval
            .as_ref()
            .expect("approval")
            .receipt
            .identity()
            .approval_id()
            .to_owned(),
        activation_claim_id: "activation-claim-task-011".to_owned(),
    };
    let rollback = RollbackSubject {
        rollback_id: "rollback-task-011".to_owned(),
        failed_activation_id: gate.subject.activation_id().to_owned(),
        saga_id: gate.subject.saga_id().to_owned(),
        failed_activation: Box::new(failed_activation),
        current_release_id: gate.subject.release_id().to_owned(),
        current_manifest_digest: gate.subject.manifest_digest().clone(),
        current_slot_id: gate.subject.target_slot_id().to_owned(),
        current_epoch: gate.subject.requested_epoch().get(),
        target_release_id: gate.subject.source_release_id().to_owned(),
        target_manifest_digest: gate.subject.source_manifest_digest().clone(),
        target_slot_id: gate.subject.source_slot_id().to_owned(),
        requested_epoch: gate.subject.requested_epoch().get() + 1,
        compatibility_evidence_digest: digest('e'),
        failure_evidence_digest: digest('f'),
        schema_compatible: true,
        migration_digests: Vec::new(),
    };
    gate.stage = UpgradeStage::Rollback;
    gate.context.state = Boundary::Known(TaskState::Blocked);
    gate.context.runtime_admission = Boundary::Known(RuntimeAdmission::Canary);
    gate.rollback = Some(rollback.clone());
    let evidence = gate.evidence.as_mut().expect("evidence");
    evidence.rollback = Some(rollback.clone());
    evidence.prior_slot_verified = true;
    gate.guardian.as_mut().expect("guardian").rollback = Some(rollback);
    gate.approval = None;
    gate
}

fn synchronize_rollback_facts(gate: &mut UpgradeGate<'_>) {
    let rollback = gate.rollback.as_ref().expect("rollback").clone();
    gate.evidence.as_mut().expect("evidence").rollback = Some(rollback.clone());
    gate.guardian.as_mut().expect("guardian").rollback = Some(rollback);
}

fn normal_recovery_gate(spec: &TaskSpec) -> RecoveryGate<'_> {
    let subject = RecoverySubject::Normal(NormalRecoverySubject {
        runtime_supervisor_id: "runtime-supervisor-1".to_owned(),
        daemon_instance_id: "daemon-1".to_owned(),
        effect_claim_id: "effect-reconcile-task-011".to_owned(),
        worktree_id: Some("worktree-task-011".to_owned()),
        expected_daemon_epoch: 7,
        resolution: NormalRecoveryResolution::EffectOutcome {
            outcome: Boundary::Known(ResolvedEffectOutcome::Succeeded),
            provider_status_digest: digest('8'),
        },
        resolution_evidence_digest: digest('9'),
        observed_admission: RuntimeAdmission::ReconciliationRequired,
        target_admission: RuntimeAdmission::Stopped,
    });
    let authority = RecoveryAuthorityFact {
        binding: binding(spec),
        subject: subject.clone(),
        owner: Boundary::Known(RecoveryOwner::RuntimeSupervisor),
        producer_id: "runtime-supervisor-1".to_owned(),
        identity_verified: true,
        fresh: true,
        reserved_system_stream: false,
        user_project_access: false,
    };
    RecoveryGate {
        context: context(
            spec,
            TaskState::Blocked,
            RuntimeAdmission::ReconciliationRequired,
        ),
        role: Boundary::Known(AgentRole::LatticePm),
        subject,
        authority: Some(authority),
    }
}

fn guardian_recovery_gate(spec: &TaskSpec) -> RecoveryGate<'_> {
    let activation = activation_gate(spec);
    let guardian = activation.guardian.expect("guardian").runtime;
    let protected_release =
        ProtectedReleaseSubject::new(activation.subject.clone(), guardian.clone());
    let activation_receipt = ProtectedActivationReceipt {
        subject: protected_release.clone(),
        approval_id: "approval-protected-release-task-011".to_owned(),
        activation_claim_id: "activation-claim-task-011".to_owned(),
    };
    let subject = RecoverySubject::GuardianRelease(Box::new(GuardianRecoverySubject {
        release: activation.subject,
        guardian,
        effect_claim_id: "effect-guardian-reconcile-task-011".to_owned(),
        resolution: GuardianSagaResolution {
            outcome: Boundary::Known(GuardianSagaOutcome::ActivationFinalized),
            activation: Box::new(activation_receipt),
            durable_saga_state_digest: digest('3'),
            database_state_digest: digest('4'),
            boot_state_digest: digest('5'),
            active_release_id: protected_release.release().release_id().to_owned(),
            active_manifest_digest: protected_release.release().manifest_digest().clone(),
            active_slot_id: protected_release.release().target_slot_id().to_owned(),
            active_epoch: protected_release.release().requested_epoch().get(),
        },
        resolution_evidence_digest: digest('6'),
        observed_admission: RuntimeAdmission::ReconciliationRequired,
        target_admission: RuntimeAdmission::Active,
    }));
    let authority = RecoveryAuthorityFact {
        binding: binding(spec),
        subject: subject.clone(),
        owner: Boundary::Known(RecoveryOwner::UpgradeGuardian),
        producer_id: "upgrade-guardian-1".to_owned(),
        identity_verified: true,
        fresh: true,
        reserved_system_stream: true,
        user_project_access: false,
    };
    let mut recovery_context = context(
        spec,
        TaskState::Blocked,
        RuntimeAdmission::ReconciliationRequired,
    );
    set_project_class(
        recovery_context.project.as_mut().expect("project"),
        ProjectClass::LatticeSystem,
    );
    RecoveryGate {
        context: recovery_context,
        role: Boundary::Known(AgentRole::UpgradeGuardian),
        subject,
        authority: Some(authority),
    }
}

fn protected_gate(spec: &TaskSpec, class: ProtectedChangeClass) -> ProtectedChangeGate<'_> {
    let subject =
        ProtectedChangeSubject::new(class, digest('7')).expect("protected change subject");
    let (role, authority, state, runtime) = match class {
        ProtectedChangeClass::CoreReleaseActivation => (
            AgentRole::UpgradeGuardian,
            ApprovalAuthority::ProtectedGuardian,
            TaskState::Reviewing,
            RuntimeAdmission::Active,
        ),
        ProtectedChangeClass::PrimaryBranchMerge => (
            AgentRole::Integrator,
            ApprovalAuthority::ResponsibleUser,
            TaskState::AwaitingMergeApproval,
            RuntimeAdmission::Active,
        ),
        _ => (
            AgentRole::LatticePm,
            ApprovalAuthority::ResponsibleUser,
            TaskState::Reviewing,
            RuntimeAdmission::Active,
        ),
    };
    let approval = approval_fact(
        spec,
        ApprovalKind::ProtectedChange,
        authority,
        ApprovalSubject::ProtectedChange(subject.clone()),
    );
    let mut protected_context = context(spec, state, runtime);
    if class == ProtectedChangeClass::CoreReleaseActivation {
        set_project_class(
            protected_context.project.as_mut().expect("project"),
            ProjectClass::LatticeSystem,
        );
    }
    ProtectedChangeGate {
        context: protected_context,
        role: Boundary::Known(role),
        subject,
        approval: Some(approval),
    }
}

fn task_spec() -> TaskSpec {
    spec_with(|_| {})
}

fn spec_with(change: impl FnOnce(&mut TaskSpecInput)) -> TaskSpec {
    let mut input = base_input();
    change(&mut input);
    TaskSpec::new(input).expect("valid Task Spec")
}

fn base_input() -> TaskSpecInput {
    TaskSpecInput {
        schema_version: "2.1".to_owned(),
        task_id: TaskId::new("TASK-011-POLICY").expect("task ID"),
        revision: "1".to_owned(),
        created_at: "2026-07-29T08:00:00Z".to_owned(),
        created_by: "owner".to_owned(),
        project_id: "general-ai-platform".to_owned(),
        project_snapshot_id: ProjectSnapshotId::new("snapshot-policy-1").expect("snapshot"),
        base_ref: "feature/policy".to_owned(),
        base_commit_id: "a".repeat(40),
        goal: "Evaluate policy deterministically.".to_owned(),
        non_goals: vec!["Do not perform I/O.".to_owned()],
        risk_class: RiskClass::R0,
        depends_on: vec![],
        scope: TaskScope {
            allowed_paths: vec!["crates/lattice-policy/**".to_owned()],
            forbidden_paths: vec![".git/**".to_owned()],
            allowed_operations: vec![ScopeOperation::Modify],
        },
        acceptance_criteria: vec![AcceptanceCriterion {
            id: "AC-01".to_owned(),
            description: "Policy matrices pass.".to_owned(),
            evidence_type: EvidenceType::Test,
            expected_result: "Exit zero.".to_owned(),
        }],
        verification_commands: vec!["cargo test -p lattice-policy".to_owned()],
        required_checks: vec![RequiredCheck::Test],
        requested_capabilities: ALL_CAPABILITIES
            .into_iter()
            .map(|capability| CapabilityRequest {
                capability,
                contract_version: "1".to_owned(),
            })
            .collect(),
        budget: TaskBudget {
            accounting_currency: "USD".to_owned(),
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
            execution: ApprovalRequirement::NotRequired,
            merge: ApprovalRequirement::ResponsibleUser,
            protected_release: ApprovalRequirement::ProtectedGuardian,
        },
    }
}

fn role_allows(role: AgentRole, action: PolicyAction) -> bool {
    match role {
        AgentRole::LatticePm => matches!(
            action,
            PolicyAction::SubmitPlan
                | PolicyAction::StopRuntime
                | PolicyAction::ProposeUpgrade
                | PolicyAction::RequestProtectedChange
        ),
        AgentRole::Planner => matches!(
            action,
            PolicyAction::ReadRepository | PolicyAction::PlanTask | PolicyAction::ReadMemory
        ),
        AgentRole::CodeMapper => matches!(
            action,
            PolicyAction::ReadRepository
                | PolicyAction::MapCode
                | PolicyAction::RunGraphify
                | PolicyAction::ReadMemory
        ),
        AgentRole::Researcher => matches!(
            action,
            PolicyAction::ReadRepository
                | PolicyAction::Research
                | PolicyAction::RunHermes
                | PolicyAction::ProposeMemory
        ),
        AgentRole::Implementer => matches!(
            action,
            PolicyAction::ReadRepository
                | PolicyAction::PrepareWorktree
                | PolicyAction::WriteProductCode
                | PolicyAction::RunTests
                | PolicyAction::RunCodex
                | PolicyAction::ReadMemory
                | PolicyAction::ReleaseWriter
        ),
        AgentRole::CorrectnessReviewer => matches!(
            action,
            PolicyAction::ReadRepository | PolicyAction::ReviewCorrectness
        ),
        AgentRole::SecurityReviewer => matches!(
            action,
            PolicyAction::ReadRepository | PolicyAction::ReviewSecurity
        ),
        AgentRole::ArchitectureReviewer => matches!(
            action,
            PolicyAction::ReadRepository | PolicyAction::ReviewArchitecture
        ),
        AgentRole::MemoryReviewer => matches!(
            action,
            PolicyAction::ReadMemory | PolicyAction::PromoteMemory
        ),
        AgentRole::Integrator => matches!(
            action,
            PolicyAction::ReadRepository
                | PolicyAction::PrepareWorktree
                | PolicyAction::IntegrateGit
        ),
        AgentRole::UpgradeGuardian => matches!(
            action,
            PolicyAction::ReconcileRuntime
                | PolicyAction::GuardianShadow
                | PolicyAction::GuardianHealth
                | PolicyAction::ActivateUpgrade
                | PolicyAction::RollbackUpgrade
        ),
    }
}

fn state_allows(state: TaskState, action: PolicyAction) -> bool {
    match action {
        PolicyAction::ReadRepository | PolicyAction::ReadMemory => true,
        PolicyAction::SubmitPlan | PolicyAction::PlanTask | PolicyAction::ProposeUpgrade => {
            matches!(state, TaskState::Draft)
        }
        PolicyAction::MapCode
        | PolicyAction::Research
        | PolicyAction::RunGraphify
        | PolicyAction::RunHermes
        | PolicyAction::ProposeMemory => matches!(
            state,
            TaskState::Draft | TaskState::AwaitingExecutionApproval
        ),
        PolicyAction::PrepareWorktree => matches!(state, TaskState::Preparing),
        PolicyAction::WriteProductCode | PolicyAction::RunCodex => {
            matches!(state, TaskState::Executing)
        }
        PolicyAction::RunTests => matches!(state, TaskState::Executing | TaskState::Verifying),
        PolicyAction::ReviewCorrectness
        | PolicyAction::ReviewSecurity
        | PolicyAction::ReviewArchitecture
        | PolicyAction::PromoteMemory
        | PolicyAction::GuardianShadow => matches!(state, TaskState::Reviewing),
        PolicyAction::IntegrateGit => matches!(state, TaskState::Merging),
        PolicyAction::StopRuntime => matches!(
            state,
            TaskState::Preparing
                | TaskState::Executing
                | TaskState::Verifying
                | TaskState::Reviewing
                | TaskState::Merging
                | TaskState::Stopping
                | TaskState::Blocked
                | TaskState::Failed
        ),
        PolicyAction::ReconcileRuntime
        | PolicyAction::ReleaseWriter
        | PolicyAction::RollbackUpgrade => {
            matches!(
                state,
                TaskState::Stopping | TaskState::Blocked | TaskState::Failed
            )
        }
        PolicyAction::GuardianHealth | PolicyAction::ActivateUpgrade => {
            matches!(state, TaskState::Merging)
        }
        PolicyAction::RequestProtectedChange => false,
    }
}

fn protected_action(action: PolicyAction) -> bool {
    matches!(
        action,
        PolicyAction::PromoteMemory
            | PolicyAction::ReconcileRuntime
            | PolicyAction::ProposeUpgrade
            | PolicyAction::IntegrateGit
            | PolicyAction::GuardianShadow
            | PolicyAction::GuardianHealth
            | PolicyAction::RequestProtectedChange
            | PolicyAction::ActivateUpgrade
            | PolicyAction::RollbackUpgrade
    )
}
