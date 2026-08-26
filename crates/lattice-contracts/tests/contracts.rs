use lattice_contracts::{
    APPROVAL_VERIFIER_PRODUCER_ID, APPROVAL_VERIFIER_PRODUCER_VERSION,
    ARTIFACT_READ_CLOSURE_PRODUCER_ID, ARTIFACT_READ_CLOSURE_PRODUCER_VERSION,
    ARTIFACT_STORE_PRODUCER_ID, ARTIFACT_STORE_PRODUCER_VERSION, ApprovalAuthority,
    ApprovalAuthorityHead, ApprovalAuthorityReceipt, ApprovalIdentity, ApprovalKind, ApprovalLane,
    ApprovalOrigin, ApprovalRevision, ApprovalStatus, ApprovalSubject, ArtifactAuthorityHead,
    ArtifactAuthorityOwnerKind, ArtifactAuthorityReceipt, ArtifactAuthorityStatus,
    ArtifactAvailability, ArtifactBundleBounds, ArtifactByteLength, ArtifactCounter,
    ArtifactDeleteStatus, ArtifactGeneration, ArtifactObjectHead, ArtifactObjectIdentity,
    ArtifactObjectKey, ArtifactProvenance, ArtifactPurpose, ArtifactQuotaValue,
    ArtifactReadAuthorityAction, ArtifactReadAuthorityBinding, ArtifactReadAuthorityHead,
    ArtifactReadAuthorityPair, ArtifactReadAuthorityReceipt, ArtifactReadClosureEvidenceBinding,
    ArtifactReadClosureEvidenceKind, ArtifactReadClosureEvidencePair,
    ArtifactReadClosureEvidenceReceipt, ArtifactReadHead, ArtifactReadStatus,
    ArtifactReferenceAuthorityAction, ArtifactReferenceAuthorityBinding,
    ArtifactReferenceAuthorityHead, ArtifactReferenceAuthorityPair,
    ArtifactReferenceAuthorityReceipt, ArtifactReferenceHead, ArtifactReferenceManifest,
    ArtifactReferenceStatus, ArtifactRevision, ArtifactSweepAuthorityAction,
    ArtifactSweepAuthorityBinding, ArtifactSweepAuthorityHead, ArtifactSweepAuthorityPair,
    ArtifactSweepAuthorityReceipt, AttemptId, Boundary, CONTRACT_VERSION, CodexEvidence,
    CodexRunRequest, Component, ContentDigest, ContractError, DaemonEpoch, ExternalCostSubject,
    FencingToken, GatewayAction, GatewayActorId, GatewayActorKind, GatewayAdapterId,
    GatewayApprovalId, GatewayApprovalRoute, GatewayChallengeId, GatewayChannelId,
    GatewayClientKind, GatewayCommandId, GatewayCorrelationId, GatewayDenialCode, GatewayEvidence,
    GatewayInstanceId, GatewayNormalApprovalKind, GatewayPeerContext, GatewayReply,
    GatewayReplyBody, GatewayRequest, GatewayRequestBody, GatewaySessionId,
    GatewayStatusObservation, GatewayStatusTarget, GatewayStopDisposition, GatewayStopReason,
    GatewayStopTarget, GatewayTaskProjection, GatewayTaskState, GatewayTaskTarget,
    GatewayUnknownCode, GitRefIdentity, GraphifyBuildRequest, GraphifyEvidence,
    GuardianRuntimeSubject, HermesEvidence, HermesResearchRequest, HolderProcessId, Invocation,
    MemoryCandidateSubject, MemoryKind, MergeSubject, MergeTarget, PROJECT_AUTHORITY_PRODUCER_ID,
    PROJECT_AUTHORITY_PRODUCER_VERSION, ProjectAuthorityReceipt, ProjectClass, ProjectId,
    ProjectLifecycle, ProjectSnapshotId, ProtectedChangeClass, ProtectedChangeSubject,
    ProtectedReleaseSubject, ReleaseSubject, RequestId, ResourceCounters, ResourceRequest,
    RuntimeAdmissionMode, RuntimeKind, SubjectBinding, TASK_INGRESS_CLIENT_REQUEST_ID_MAX_BYTES,
    TASK_LEDGER_PRODUCER_ID, TASK_LEDGER_PRODUCER_VERSION, TaskId, TaskIntakeBinding,
    TaskLedgerResourceReceipt, TaskLedgerStreamHead, TaskLedgerStreamIdentity,
    TaskLedgerSubjectKind, TaskSpecSubmission, UpgradeDelta, WRITER_LEASE_PRODUCER_ID,
    WRITER_LEASE_PRODUCER_VERSION, WriterLeaseAuthorityReceipt, WriterLeaseIdentity,
    WriterLeaseRevision, WriterLeaseStatus, task_ingress_text_contains_recognized_secret,
    valid_task_ingress_client_request_id,
};

fn digest(byte: char) -> ContentDigest {
    ContentDigest::from_sha256(byte.to_string().repeat(64)).expect("valid test digest")
}

#[test]
#[allow(clippy::unicode_not_nfc)]
fn task_ingress_client_request_id_contract_is_bounded_secret_free_ascii() {
    assert!(valid_task_ingress_client_request_id("a"));
    assert!(valid_task_ingress_client_request_id(
        &"a".repeat(TASK_INGRESS_CLIENT_REQUEST_ID_MAX_BYTES)
    ));
    assert!(valid_task_ingress_client_request_id("phase3.retry_1:x-y"));
    assert!(valid_task_ingress_client_request_id("mask-based-request"));

    for rejected in [
        "",
        " contains-spaces ",
        "contains/slash",
        "contains-unicode-任務",
        "sk-do-not-use",
        "prefix-sk-do-not-use",
        "xghp_do-not-use",
        "token:do-not-use",
        "authorization:do-not-use",
        "AKIA1234567890ABCDEF",
    ] {
        assert!(
            !valid_task_ingress_client_request_id(rejected),
            "must reject {rejected:?}"
        );
    }
    assert!(!valid_task_ingress_client_request_id(
        &"a".repeat(TASK_INGRESS_CLIENT_REQUEST_ID_MAX_BYTES + 1)
    ));

    for rejected in [
        "bearer do-not-use",
        "-----BEGIN PRIVATE KEY-----do-not-use",
        r#"{"password":"do-not-use"}"#,
        "password\u{2003}=do-not-use",
        "api_key\u{a0}:do-not-use",
        "Kpassword=do-not-use",
        "Ksk-do-not-use",
        "private key----- marker before -----begin marker",
        "embedded-ghp_do-not-use",
        "use AKIAIOSFODNN7EXAMPLE here",
    ] {
        assert!(
            task_ingress_text_contains_recognized_secret(rejected),
            "must recognize {rejected:?}"
        );
    }
    for benign in [
        "finish mask-based validation",
        "tokenize input",
        "monkey:value",
    ] {
        assert!(
            !task_ingress_text_contains_recognized_secret(benign),
            "must preserve benign {benign:?}"
        );
    }
}

fn approval_binding() -> SubjectBinding {
    SubjectBinding::new(
        ProjectId::new("project-1").expect("project"),
        ProjectSnapshotId::new("snapshot-1").expect("snapshot"),
        TaskId::new("TASK-015").expect("task"),
        "1",
        digest('1'),
    )
    .expect("approval binding")
}

fn normal_approval_identity() -> ApprovalIdentity {
    ApprovalIdentity::new(
        "approval-1",
        "challenge-1",
        approval_binding(),
        ApprovalSubject::Execution {
            task_spec_hash: digest('1'),
            external_cost: None,
        },
        "requester-1",
        "approver-1",
        ApprovalAuthority::ResponsibleUser,
        ApprovalOrigin::OsAuthenticatedUser,
        ApprovalLane::Normal,
        "channel-1",
        "session-1",
    )
    .expect("normal approval identity")
}

fn normal_approval_receipt() -> ApprovalAuthorityReceipt {
    ApprovalAuthorityReceipt::new(
        CONTRACT_VERSION,
        APPROVAL_VERIFIER_PRODUCER_ID,
        APPROVAL_VERIFIER_PRODUCER_VERSION,
        RuntimeKind::Fake,
        normal_approval_identity(),
        ApprovalRevision::new(1).expect("revision"),
        ApprovalStatus::Available,
        "nonce-1",
        digest('2'),
        "2026-07-29T08:00:00Z",
        "2026-07-29T09:00:00Z",
        digest('3'),
        digest('4'),
        "fake-authenticator-1",
        "fake-key-1",
        digest('5'),
        digest('6'),
        None,
        digest('7'),
    )
    .expect("approval receipt")
}

fn protected_release_subject() -> ProtectedReleaseSubject {
    let release = ReleaseSubject::new(
        "activation-1",
        "saga-1",
        "release-1",
        "1",
        digest('8'),
        "d".repeat(40),
        digest('9'),
        digest('a'),
        vec![digest('b'), digest('c')],
        vec![],
        digest('d'),
        "release-0",
        digest('e'),
        "slot-a",
        "slot-b",
        DaemonEpoch::new(8).expect("requested epoch"),
        true,
        UpgradeDelta::new(false, true, false, false, false, false, false, true),
    )
    .expect("release subject");
    let guardian = GuardianRuntimeSubject::new(
        "guardian-1",
        digest('f'),
        "guardian-daemon-1",
        DaemonEpoch::new(8).expect("observed epoch"),
    )
    .expect("guardian subject");
    ProtectedReleaseSubject::new(release, guardian)
}

fn protected_approval_identity() -> ApprovalIdentity {
    ApprovalIdentity::new(
        "approval-protected-1",
        "challenge-protected-1",
        approval_binding(),
        ApprovalSubject::ProtectedRelease(Box::new(protected_release_subject())),
        "candidate-1",
        "guardian-actor-1",
        ApprovalAuthority::ProtectedGuardian,
        ApprovalOrigin::GuardianTrustRoot,
        ApprovalLane::Protected,
        "guardian-channel-1",
        "guardian-session-1",
    )
    .expect("protected approval identity")
}

#[test]
fn approval_receipt_projects_complete_fixed_owner_head() {
    let receipt = normal_approval_receipt();
    let head = receipt.head();

    assert_eq!(receipt.producer_id(), APPROVAL_VERIFIER_PRODUCER_ID);
    assert_eq!(
        receipt.producer_version(),
        APPROVAL_VERIFIER_PRODUCER_VERSION
    );
    assert_eq!(receipt.identity().subject().kind(), ApprovalKind::Execution);
    assert_eq!(receipt.status(), ApprovalStatus::Available);
    assert_eq!(head.identity(), receipt.identity());
    assert_eq!(head.receipt_digest(), receipt.receipt_digest());
    assert_eq!(head, receipt.head());
}

#[test]
fn approval_subject_families_preserve_complete_typed_values_and_derived_kind() {
    let external_cost =
        ExternalCostSubject::new("1.25", "USD", "provider-1", digest('2'), digest('3'))
            .expect("external cost");
    assert_eq!(external_cost.amount(), "1.25");
    assert_eq!(external_cost.currency(), "USD");
    assert_eq!(external_cost.provider_id(), "provider-1");

    let execution = ApprovalSubject::Execution {
        task_spec_hash: digest('1'),
        external_cost: Some(external_cost),
    };
    let merge = ApprovalSubject::Merge(
        MergeSubject::new(
            MergeTarget::PrimaryBranch("refs/heads/Main".to_owned()),
            "a".repeat(40),
            "b".repeat(40),
            digest('4'),
        )
        .expect("merge"),
    );
    let preference = ApprovalSubject::Preference(
        MemoryCandidateSubject::new(approval_binding(), digest('5'), MemoryKind::Preference)
            .expect("memory preference"),
    );
    let protected_change = ApprovalSubject::ProtectedChange(
        ProtectedChangeSubject::new(ProtectedChangeClass::Policy, digest('6'))
            .expect("protected change"),
    );
    let protected_release =
        ApprovalSubject::ProtectedRelease(Box::new(protected_release_subject()));

    assert_eq!(
        [
            execution.kind(),
            merge.kind(),
            preference.kind(),
            protected_change.kind(),
            protected_release.kind(),
        ],
        ApprovalKind::ALL
    );

    let ApprovalSubject::Merge(merge) = merge else {
        panic!("merge variant")
    };
    assert_eq!(merge.target().reference(), Some("refs/heads/Main"));
    assert_eq!(merge.reviewed_commit(), "a".repeat(40));
    assert_eq!(merge.diff_digest(), &digest('4'));

    let ApprovalSubject::Preference(preference) = preference else {
        panic!("preference variant")
    };
    assert_eq!(preference.binding(), &approval_binding());
    assert_eq!(preference.kind(), MemoryKind::Preference);

    let ApprovalSubject::ProtectedChange(change) = protected_change else {
        panic!("protected-change variant")
    };
    assert_eq!(change.class(), ProtectedChangeClass::Policy);
    assert_eq!(change.operation_digest(), &digest('6'));

    let ApprovalSubject::ProtectedRelease(release) = protected_release else {
        panic!("protected-release variant")
    };
    assert_eq!(release.release().activation_id(), "activation-1");
    assert_eq!(release.release().requested_epoch().get(), 8);
    assert!(release.release().delta().policy());
    assert!(release.release().delta().capability_expansion());
    assert_eq!(release.guardian().guardian_id(), "guardian-1");
    assert_eq!(release.guardian().observed_epoch().get(), 8);
}

#[test]
fn approval_subject_constructors_reject_malformed_or_zero_fields() {
    assert_eq!(
        SubjectBinding::new(
            ProjectId::new("project-1").expect("project"),
            ProjectSnapshotId::new("snapshot-1").expect("snapshot"),
            TaskId::new("TASK-015").expect("task"),
            "01",
            digest('1'),
        ),
        Err(ContractError::InvalidApprovalSubject {
            field: "task_revision"
        })
    );
    assert_eq!(
        ExternalCostSubject::new("01", "USD", "provider-1", digest('2'), digest('3')),
        Err(ContractError::InvalidApprovalSubject { field: "amount" })
    );
    assert_eq!(
        ExternalCostSubject::new("1", "usd", "provider-1", digest('2'), digest('3')),
        Err(ContractError::InvalidApprovalSubject { field: "currency" })
    );
    assert_eq!(
        MergeSubject::new(
            MergeTarget::PrimaryBranch("main".to_owned()),
            "a".repeat(40),
            "b".repeat(40),
            digest('4'),
        ),
        Err(ContractError::InvalidApprovalSubject {
            field: "merge_target"
        })
    );
    assert_eq!(
        MemoryCandidateSubject::new(approval_binding(), digest('0'), MemoryKind::Preference),
        Err(ContractError::InvalidApprovalSubject {
            field: "candidate_digest"
        })
    );
    assert_eq!(
        ProtectedChangeSubject::new(ProtectedChangeClass::Policy, digest('0')),
        Err(ContractError::InvalidApprovalSubject {
            field: "operation_digest"
        })
    );
    assert_eq!(
        GuardianRuntimeSubject::new(
            "guardian-1",
            digest('0'),
            "guardian-daemon-1",
            DaemonEpoch::new(1).expect("epoch")
        ),
        Err(ContractError::InvalidApprovalSubject {
            field: "trust_root_digest"
        })
    );
}

#[test]
fn approval_identity_enforces_binding_self_approval_and_trust_lane() {
    assert_eq!(
        normal_approval_identity().subject().kind(),
        ApprovalKind::Execution
    );
    assert_eq!(
        protected_approval_identity().lane(),
        ApprovalLane::Protected
    );

    let self_approval = ApprovalIdentity::new(
        "approval-1",
        "challenge-1",
        approval_binding(),
        ApprovalSubject::Execution {
            task_spec_hash: digest('1'),
            external_cost: None,
        },
        "same-actor",
        "same-actor",
        ApprovalAuthority::ResponsibleUser,
        ApprovalOrigin::OsAuthenticatedUser,
        ApprovalLane::Normal,
        "channel-1",
        "session-1",
    );
    assert_eq!(
        self_approval,
        Err(ContractError::InvalidApprovalIdentity {
            field: "self_approval"
        })
    );

    let wrong_pair = ApprovalIdentity::new(
        "approval-1",
        "challenge-1",
        approval_binding(),
        ApprovalSubject::Execution {
            task_spec_hash: digest('1'),
            external_cost: None,
        },
        "requester-1",
        "approver-1",
        ApprovalAuthority::ResponsibleUser,
        ApprovalOrigin::NormalGateway,
        ApprovalLane::Normal,
        "channel-1",
        "session-1",
    );
    assert_eq!(
        wrong_pair,
        Err(ContractError::InvalidApprovalIdentity {
            field: "authority_origin_lane"
        })
    );

    let wrong_subject_binding = ApprovalIdentity::new(
        "approval-1",
        "challenge-1",
        approval_binding(),
        ApprovalSubject::Execution {
            task_spec_hash: digest('9'),
            external_cost: None,
        },
        "requester-1",
        "approver-1",
        ApprovalAuthority::ResponsibleUser,
        ApprovalOrigin::OsAuthenticatedUser,
        ApprovalLane::Normal,
        "channel-1",
        "session-1",
    );
    assert_eq!(
        wrong_subject_binding,
        Err(ContractError::InvalidApprovalIdentity {
            field: "subject_binding"
        })
    );

    let normal_protected_release = ApprovalIdentity::new(
        "approval-1",
        "challenge-1",
        approval_binding(),
        ApprovalSubject::ProtectedRelease(Box::new(protected_release_subject())),
        "requester-1",
        "approver-1",
        ApprovalAuthority::ResponsibleUser,
        ApprovalOrigin::OsAuthenticatedUser,
        ApprovalLane::Normal,
        "channel-1",
        "session-1",
    );
    assert_eq!(
        normal_protected_release,
        Err(ContractError::InvalidApprovalIdentity {
            field: "protected_release_lane"
        })
    );
}

#[test]
fn approval_receipt_rejects_substituted_owner_status_and_zero_digests() {
    let base = normal_approval_identity();
    let make = |producer_id: &str,
                producer_version: &str,
                status: ApprovalStatus,
                nonce_commitment: ContentDigest,
                review_set_digest: Option<ContentDigest>| {
        ApprovalAuthorityReceipt::new(
            CONTRACT_VERSION,
            producer_id,
            producer_version,
            RuntimeKind::Fake,
            base.clone(),
            ApprovalRevision::new(1).expect("revision"),
            status,
            "nonce-1",
            nonce_commitment,
            "2026-07-29T08:00:00Z",
            "2026-07-29T09:00:00Z",
            digest('3'),
            digest('4'),
            "fake-authenticator-1",
            "fake-key-1",
            digest('5'),
            digest('6'),
            review_set_digest,
            digest('7'),
        )
    };

    assert_eq!(
        make(
            "other-producer",
            APPROVAL_VERIFIER_PRODUCER_VERSION,
            ApprovalStatus::Available,
            digest('2'),
            None,
        ),
        Err(ContractError::UnsupportedApprovalVerifierProducer)
    );
    assert_eq!(
        make(
            APPROVAL_VERIFIER_PRODUCER_ID,
            "2.0",
            ApprovalStatus::Available,
            digest('2'),
            None,
        ),
        Err(ContractError::UnsupportedApprovalVerifierProducerVersion)
    );
    assert_eq!(
        make(
            APPROVAL_VERIFIER_PRODUCER_ID,
            APPROVAL_VERIFIER_PRODUCER_VERSION,
            ApprovalStatus::ProtectedPendingClaim,
            digest('2'),
            None,
        ),
        Err(ContractError::InvalidApprovalReceipt { field: "status" })
    );
    assert_eq!(
        make(
            APPROVAL_VERIFIER_PRODUCER_ID,
            APPROVAL_VERIFIER_PRODUCER_VERSION,
            ApprovalStatus::Available,
            digest('0'),
            None,
        ),
        Err(ContractError::InvalidApprovalReceipt {
            field: "nonce_commitment"
        })
    );
    assert_eq!(
        make(
            APPROVAL_VERIFIER_PRODUCER_ID,
            APPROVAL_VERIFIER_PRODUCER_VERSION,
            ApprovalStatus::Available,
            digest('2'),
            Some(digest('0')),
        ),
        Err(ContractError::InvalidApprovalReceipt {
            field: "review_set_digest"
        })
    );
}

#[derive(Clone)]
struct ApprovalHeadFixture {
    runtime: RuntimeKind,
    identity: ApprovalIdentity,
    revision: ApprovalRevision,
    status: ApprovalStatus,
    nonce_id: String,
    nonce_commitment: ContentDigest,
    issued_at: String,
    expires_at: String,
    subject_digest: ContentDigest,
    challenge_digest: ContentDigest,
    authenticator_id: String,
    key_id: String,
    proof_digest: ContentDigest,
    evidence_digest: ContentDigest,
    review_set_digest: Option<ContentDigest>,
    receipt_digest: ContentDigest,
}

impl ApprovalHeadFixture {
    fn from_receipt(receipt: &ApprovalAuthorityReceipt) -> Self {
        Self {
            runtime: receipt.runtime(),
            identity: receipt.identity().clone(),
            revision: receipt.revision(),
            status: receipt.status(),
            nonce_id: receipt.nonce_id().to_owned(),
            nonce_commitment: receipt.nonce_commitment().clone(),
            issued_at: receipt.issued_at().to_owned(),
            expires_at: receipt.expires_at().to_owned(),
            subject_digest: receipt.subject_digest().clone(),
            challenge_digest: receipt.challenge_digest().clone(),
            authenticator_id: receipt.authenticator_id().to_owned(),
            key_id: receipt.key_id().to_owned(),
            proof_digest: receipt.proof_digest().clone(),
            evidence_digest: receipt.evidence_digest().clone(),
            review_set_digest: receipt.review_set_digest().cloned(),
            receipt_digest: receipt.receipt_digest().clone(),
        }
    }

    fn build(&self) -> ApprovalAuthorityHead {
        ApprovalAuthorityHead::new(
            CONTRACT_VERSION,
            APPROVAL_VERIFIER_PRODUCER_ID,
            APPROVAL_VERIFIER_PRODUCER_VERSION,
            self.runtime,
            self.identity.clone(),
            self.revision,
            self.status,
            self.nonce_id.clone(),
            self.nonce_commitment.clone(),
            self.issued_at.clone(),
            self.expires_at.clone(),
            self.subject_digest.clone(),
            self.challenge_digest.clone(),
            self.authenticator_id.clone(),
            self.key_id.clone(),
            self.proof_digest.clone(),
            self.evidence_digest.clone(),
            self.review_set_digest.clone(),
            self.receipt_digest.clone(),
        )
        .expect("valid head fixture")
    }
}

#[test]
fn approval_head_equality_covers_every_security_relevant_field() {
    let receipt = normal_approval_receipt();
    let expected = receipt.head();
    let base = ApprovalHeadFixture::from_receipt(&receipt);

    macro_rules! assert_substitution {
        ($field:ident, $value:expr) => {{
            let mut changed = base.clone();
            changed.$field = $value;
            assert_ne!(changed.build(), expected, stringify!($field));
        }};
    }

    assert_substitution!(runtime, RuntimeKind::Live);
    let alternate_identity = ApprovalIdentity::new(
        "approval-1",
        "challenge-1",
        approval_binding(),
        ApprovalSubject::Execution {
            task_spec_hash: digest('1'),
            external_cost: None,
        },
        "requester-1",
        "approver-1",
        ApprovalAuthority::ResponsibleUser,
        ApprovalOrigin::OsAuthenticatedUser,
        ApprovalLane::Normal,
        "channel-other",
        "session-1",
    )
    .expect("alternate identity");
    assert_substitution!(identity, alternate_identity);
    assert_substitution!(revision, ApprovalRevision::new(2).expect("revision"));
    assert_substitution!(status, ApprovalStatus::ClaimedNormal);
    assert_substitution!(nonce_id, "nonce-other".to_owned());
    assert_substitution!(nonce_commitment, digest('8'));
    assert_substitution!(issued_at, "2026-07-29T08:01:00Z".to_owned());
    assert_substitution!(expires_at, "2026-07-29T09:01:00Z".to_owned());
    assert_substitution!(subject_digest, digest('9'));
    assert_substitution!(challenge_digest, digest('a'));
    assert_substitution!(authenticator_id, "fake-authenticator-other".to_owned());
    assert_substitution!(key_id, "fake-key-other".to_owned());
    assert_substitution!(proof_digest, digest('b'));
    assert_substitution!(evidence_digest, digest('c'));
    assert_substitution!(review_set_digest, Some(digest('d')));
    assert_substitution!(receipt_digest, digest('e'));
}

#[test]
fn approval_revision_uses_positive_signed_bigint_bounds() {
    assert_eq!(ApprovalRevision::new(1).expect("one").get(), 1);
    assert_eq!(
        ApprovalRevision::new(i64::MAX as u64)
            .expect("signed bigint max")
            .get(),
        i64::MAX as u64
    );
    assert_eq!(
        ApprovalRevision::new(0),
        Err(ContractError::InvalidPositiveSignedBigInt {
            field: "approval_revision"
        })
    );
    assert_eq!(
        ApprovalRevision::new((i64::MAX as u64) + 1),
        Err(ContractError::InvalidPositiveSignedBigInt {
            field: "approval_revision"
        })
    );
}

fn invocation() -> Invocation {
    Invocation::new(
        CONTRACT_VERSION,
        RequestId::new("request-1").expect("valid request id"),
        TaskId::new("task-9").expect("valid task id"),
        AttemptId::new("attempt-1").expect("valid attempt id"),
        ProjectSnapshotId::new("snapshot-1").expect("valid snapshot id"),
        digest('a'),
    )
    .expect("supported contract")
}

#[test]
fn valid_invocation_and_evidence_preserve_identity() {
    let invocation = invocation();
    let evidence = GraphifyEvidence::new(invocation.clone(), RuntimeKind::Fake, digest('b'));

    assert_eq!(invocation.version(), CONTRACT_VERSION);
    assert_eq!(invocation.request_id().as_str(), "request-1");
    assert_eq!(invocation.task_id().as_str(), "task-9");
    assert_eq!(invocation.attempt_id().as_str(), "attempt-1");
    assert_eq!(invocation.project_snapshot_id().as_str(), "snapshot-1");
    assert_eq!(invocation.subject_digest(), &digest('a'));
    assert_eq!(evidence.invocation(), &invocation);
    assert_eq!(evidence.component(), Component::Graphify);
    assert_eq!(evidence.boundary(), Boundary::DerivedReadOnlyEvidence);
    assert_eq!(evidence.runtime(), RuntimeKind::Fake);
    assert_eq!(evidence.output_digest(), &digest('b'));
}

#[test]
fn lane_specific_evidence_fixes_component_and_boundary_at_construction() {
    let evidence = [
        GatewayEvidence::new(invocation(), RuntimeKind::Fake, digest('1')).into_normalized(),
        CodexEvidence::new(invocation(), RuntimeKind::Fake, digest('3')).into_normalized(),
        GraphifyEvidence::new(invocation(), RuntimeKind::Fake, digest('4')).into_normalized(),
        HermesEvidence::new(invocation(), RuntimeKind::Fake, digest('5')).into_normalized(),
    ];

    assert_eq!(
        evidence.map(|item| (item.component(), item.boundary())),
        [
            (Component::OpenClaw, Boundary::Gateway),
            (Component::Codex, Boundary::ProductCodeWriter),
            (Component::Graphify, Boundary::DerivedReadOnlyEvidence),
            (Component::Hermes, Boundary::UntrustedCandidate),
        ]
    );
}

#[test]
fn gateway_exposes_only_the_six_typed_actions() {
    assert_eq!(
        GatewayAction::ALL,
        [
            GatewayAction::Submit,
            GatewayAction::Plan,
            GatewayAction::Status,
            GatewayAction::Approve,
            GatewayAction::Reject,
            GatewayAction::Stop,
        ]
    );
    assert_eq!(
        GatewayAction::ALL.map(GatewayAction::as_str),
        ["submit", "plan", "status", "approve", "reject", "stop"]
    );
}

#[test]
fn lane_requests_are_distinct_typed_wrappers() {
    let request = invocation();
    let codex = CodexRunRequest::new(request.clone(), digest('d'));
    let graphify = GraphifyBuildRequest::new(request.clone());
    let hermes = HermesResearchRequest::new(request.clone());

    assert_eq!(codex.invocation(), &request);
    assert_eq!(codex.writer_claim_digest(), &digest('d'));
    assert_eq!(graphify.invocation(), &request);
    assert_eq!(hermes.invocation(), &request);
}

#[test]
fn identifiers_reject_empty_or_whitespace_only_values() {
    assert!(matches!(
        RequestId::new(" "),
        Err(ContractError::EmptyIdentifier {
            field: "request_id"
        })
    ));
    assert!(matches!(
        TaskId::new(""),
        Err(ContractError::EmptyIdentifier { field: "task_id" })
    ));
}

#[test]
fn invocation_rejects_unknown_contract_versions() {
    let result = Invocation::new(
        CONTRACT_VERSION + 1,
        RequestId::new("request-1").expect("valid request id"),
        TaskId::new("task-9").expect("valid task id"),
        AttemptId::new("attempt-1").expect("valid attempt id"),
        ProjectSnapshotId::new("snapshot-1").expect("valid snapshot id"),
        digest('a'),
    );

    assert!(matches!(
        result,
        Err(ContractError::UnsupportedVersion {
            supported: CONTRACT_VERSION,
            found
        }) if found == CONTRACT_VERSION + 1
    ));
}

#[test]
fn sha256_references_must_be_exact_lowercase_hex() {
    for invalid in ["", "abc", &"A".repeat(64), &"g".repeat(64)] {
        assert!(matches!(
            ContentDigest::from_sha256(invalid),
            Err(ContractError::MalformedSha256)
        ));
    }
}

#[test]
fn project_ids_share_one_canonical_ascii_contract() {
    for valid in ["ab", "lattice-devos", "project_1", "project.name"] {
        assert_eq!(
            ProjectId::new(valid).expect("valid project id").as_str(),
            valid
        );
    }

    for invalid in [
        "",
        "a",
        "UPPER",
        " leading",
        "trailing ",
        "-leading",
        "has/slash",
        &"a".repeat(65),
    ] {
        assert!(matches!(
            ProjectId::new(invalid),
            Err(ContractError::InvalidProjectId)
        ));
    }
}

#[test]
fn physical_git_ref_identity_accepts_only_fully_qualified_local_branches() {
    let primary =
        GitRefIdentity::new("refs/heads/Main", digest('1')).expect("valid local branch identity");
    let alias =
        GitRefIdentity::new("refs/heads/main", digest('1')).expect("valid local branch identity");
    let uppercase = GitRefIdentity::new("refs/heads/RELEASE_2026", digest('2'))
        .expect("valid uppercase branch");

    assert_eq!(primary.reference(), "refs/heads/Main");
    assert_eq!(primary.storage_identity_digest(), &digest('1'));
    assert_ne!(primary.reference(), alias.reference());
    assert_eq!(uppercase.reference(), "refs/heads/RELEASE_2026");
    assert_eq!(
        primary.storage_identity_digest(),
        alias.storage_identity_digest()
    );

    for invalid in [
        "HEAD",
        "main",
        "refs/tags/main",
        "refs/remotes/origin/main",
        "refs/heads/HEAD",
        "refs/heads/refs/heads/main",
        "refs/heads/main.lock",
        "refs/heads/a..b",
        "refs/heads/a@{b",
    ] {
        assert!(matches!(
            GitRefIdentity::new(invalid, digest('2')),
            Err(ContractError::InvalidGitReference)
        ));
    }
}

#[test]
fn project_authority_receipt_and_head_bind_exact_owner_identity() {
    let receipt = ProjectAuthorityReceipt::new(
        CONTRACT_VERSION,
        "lattice-project-registry",
        "1.0",
        RuntimeKind::Fake,
        ProjectId::new("lattice-devos").expect("project"),
        ProjectSnapshotId::new("lattice-devos:registry:1").expect("snapshot"),
        1,
        ProjectLifecycle::Active,
        ProjectClass::LatticeSystem,
        GitRefIdentity::new("refs/heads/main", digest('3')).expect("primary ref"),
        digest('4'),
        digest('5'),
    )
    .expect("valid receipt");

    assert_eq!(receipt.version(), CONTRACT_VERSION);
    assert_eq!(receipt.producer_id(), "lattice-project-registry");
    assert_eq!(receipt.producer_version(), "1.0");
    assert_eq!(receipt.runtime(), RuntimeKind::Fake);
    assert_eq!(receipt.project_id().as_str(), "lattice-devos");
    assert_eq!(
        receipt.project_snapshot_id().as_str(),
        "lattice-devos:registry:1"
    );
    assert_eq!(receipt.registry_revision(), 1);
    assert_eq!(receipt.lifecycle(), ProjectLifecycle::Active);
    assert_eq!(receipt.project_class(), ProjectClass::LatticeSystem);
    assert_eq!(receipt.primary_branch().reference(), "refs/heads/main");
    assert_eq!(receipt.observation_digest(), &digest('4'));
    assert_eq!(receipt.receipt_digest(), &digest('5'));

    let head = receipt.head();
    assert_eq!(head.producer_id(), receipt.producer_id());
    assert_eq!(head.producer_version(), receipt.producer_version());
    assert_eq!(head.runtime(), receipt.runtime());
    assert_eq!(head.project_id(), receipt.project_id());
    assert_eq!(head.project_snapshot_id(), receipt.project_snapshot_id());
    assert_eq!(head.registry_revision(), receipt.registry_revision());
    assert_eq!(head.lifecycle(), receipt.lifecycle());
    assert_eq!(head.project_class(), receipt.project_class());
    assert_eq!(head.primary_branch(), receipt.primary_branch());
    assert_eq!(head.observation_digest(), receipt.observation_digest());
    assert_eq!(head.receipt_digest(), receipt.receipt_digest());
}

#[test]
fn project_authority_receipt_rejects_unknown_version_substituted_owner_and_zero_revision() {
    let make = |version, producer, producer_version, revision| {
        ProjectAuthorityReceipt::new(
            version,
            producer,
            producer_version,
            RuntimeKind::Fake,
            ProjectId::new("project-1").expect("project"),
            ProjectSnapshotId::new("snapshot-1").expect("snapshot"),
            revision,
            ProjectLifecycle::Active,
            ProjectClass::UserProject,
            GitRefIdentity::new("refs/heads/main", digest('6')).expect("primary ref"),
            digest('7'),
            digest('8'),
        )
    };

    assert!(matches!(
        make(
            CONTRACT_VERSION + 1,
            PROJECT_AUTHORITY_PRODUCER_ID,
            PROJECT_AUTHORITY_PRODUCER_VERSION,
            1
        ),
        Err(ContractError::UnsupportedVersion { .. })
    ));
    assert!(matches!(
        make(CONTRACT_VERSION, " ", PROJECT_AUTHORITY_PRODUCER_VERSION, 1),
        Err(ContractError::EmptyIdentifier {
            field: "project_authority_producer_id"
        })
    ));
    assert!(matches!(
        make(
            CONTRACT_VERSION,
            "project-registry-substitute",
            PROJECT_AUTHORITY_PRODUCER_VERSION,
            1
        ),
        Err(ContractError::UnsupportedProjectAuthorityProducer)
    ));
    assert!(matches!(
        make(
            CONTRACT_VERSION,
            PROJECT_AUTHORITY_PRODUCER_ID,
            "1.0-substitute",
            1
        ),
        Err(ContractError::UnsupportedProjectAuthorityProducerVersion)
    ));
    assert!(matches!(
        make(
            CONTRACT_VERSION,
            PROJECT_AUTHORITY_PRODUCER_ID,
            PROJECT_AUTHORITY_PRODUCER_VERSION,
            0
        ),
        Err(ContractError::ZeroRevision)
    ));
}

fn ledger_identity() -> TaskLedgerStreamIdentity {
    TaskLedgerStreamIdentity::new(
        ProjectId::new("project-1").expect("project"),
        ProjectSnapshotId::new("snapshot-1").expect("snapshot"),
        TaskId::new("TASK-013").expect("task"),
        "1",
        digest('2'),
        "TWD",
    )
    .expect("valid Ledger identity")
}

#[test]
fn task_ledger_identity_keeps_task_spec_compatibility_and_separates_general_intake() {
    let task_spec = ledger_identity();
    assert_eq!(task_spec.subject_kind(), TaskLedgerSubjectKind::TaskSpec);
    assert_eq!(task_spec.subject_digest(), &digest('2'));
    assert_eq!(task_spec.task_spec_digest(), Some(&digest('2')));
    assert_eq!(task_spec.general_task_intake_digest(), None);
    assert_eq!(task_spec.accounting_currency(), Some("TWD"));

    let intake = TaskLedgerStreamIdentity::new_general_task_intake(
        ProjectId::new("project-1").expect("project"),
        ProjectSnapshotId::new("snapshot-1").expect("snapshot"),
        TaskId::new("TASK-INTAKE-1").expect("task"),
        "1",
        digest('8'),
    )
    .expect("general intake identity");
    assert_eq!(
        intake.subject_kind(),
        TaskLedgerSubjectKind::GeneralTaskIntake
    );
    assert_eq!(intake.subject_digest(), &digest('8'));
    assert_eq!(intake.task_spec_digest(), None);
    assert_eq!(intake.general_task_intake_digest(), Some(&digest('8')));
    assert_eq!(intake.accounting_currency(), None);

    let binding =
        TaskIntakeBinding::try_from_stream_identity(&intake).expect("typed intake binding");
    assert_eq!(binding.stream_identity(), &intake);
    assert_eq!(binding.project_id(), intake.project_id());
    assert_eq!(binding.project_snapshot_id(), intake.project_snapshot_id());
    assert_eq!(binding.task_id(), intake.task_id());
    assert_eq!(binding.task_revision(), intake.task_revision());
    assert_eq!(binding.intake_digest(), &digest('8'));
    assert!(TaskIntakeBinding::try_from_stream_identity(&task_spec).is_err());
}

#[test]
fn general_intake_identity_rejects_zero_digest_without_gaining_task_spec_fields() {
    let result = TaskLedgerStreamIdentity::new_general_task_intake(
        ProjectId::new("project-1").expect("project"),
        ProjectSnapshotId::new("snapshot-1").expect("snapshot"),
        TaskId::new("TASK-INTAKE-2").expect("task"),
        "1",
        digest('0'),
    );
    assert!(matches!(
        result,
        Err(ContractError::InvalidTaskIntakeBinding {
            field: "intake_digest"
        })
    ));
}

#[test]
fn general_intake_cannot_become_a_resource_accounting_receipt() {
    let identity = TaskLedgerStreamIdentity::new_general_task_intake(
        ProjectId::new("project-1").expect("project"),
        ProjectSnapshotId::new("snapshot-1").expect("snapshot"),
        TaskId::new("TASK-INTAKE-3").expect("task"),
        "1",
        digest('8'),
    )
    .expect("intake identity");
    let head = TaskLedgerStreamHead::new(
        CONTRACT_VERSION,
        TASK_LEDGER_PRODUCER_ID,
        TASK_LEDGER_PRODUCER_VERSION,
        RuntimeKind::Fake,
        identity,
        digest('1'),
        1,
        digest('3'),
        1,
        digest('4'),
        digest('5'),
    )
    .expect("intake Ledger head");
    let receipt = TaskLedgerResourceReceipt::new(
        CONTRACT_VERSION,
        TASK_LEDGER_PRODUCER_ID,
        TASK_LEDGER_PRODUCER_VERSION,
        RuntimeKind::Fake,
        head,
        1,
        "effect-intake-forbidden",
        digest('6'),
        resource_counters(),
        resource_request(),
        "TWD",
        digest('7'),
        digest('9'),
    );
    assert_eq!(receipt, Err(ContractError::InvalidAccountingCurrency));
}

fn ledger_head(runtime: RuntimeKind, sequence: u64) -> TaskLedgerStreamHead {
    TaskLedgerStreamHead::new(
        CONTRACT_VERSION,
        TASK_LEDGER_PRODUCER_ID,
        TASK_LEDGER_PRODUCER_VERSION,
        runtime,
        ledger_identity(),
        digest('1'),
        sequence,
        if sequence == 0 {
            digest('0')
        } else {
            digest('3')
        },
        1,
        digest('4'),
        digest('5'),
    )
    .expect("valid Ledger head")
}

fn resource_counters() -> ResourceCounters {
    ResourceCounters::new(1, 1, 120, 2, 5, "10.5").expect("valid counters")
}

fn resource_request() -> ResourceRequest {
    ResourceRequest::new(1, 0, 30, 1, 2, Some("2.5")).expect("valid request")
}

#[test]
fn task_ledger_resource_receipt_and_head_bind_every_security_field() {
    let stream_head = ledger_head(RuntimeKind::Fake, 7);
    let receipt = TaskLedgerResourceReceipt::new(
        CONTRACT_VERSION,
        TASK_LEDGER_PRODUCER_ID,
        TASK_LEDGER_PRODUCER_VERSION,
        RuntimeKind::Fake,
        stream_head.clone(),
        7,
        "effect-claim-1",
        digest('8'),
        resource_counters(),
        resource_request(),
        "TWD",
        digest('6'),
        digest('7'),
    )
    .expect("valid Ledger resource receipt");

    assert_eq!(receipt.version(), CONTRACT_VERSION);
    assert_eq!(receipt.producer_id(), TASK_LEDGER_PRODUCER_ID);
    assert_eq!(receipt.producer_version(), TASK_LEDGER_PRODUCER_VERSION);
    assert_eq!(receipt.runtime(), RuntimeKind::Fake);
    assert_eq!(receipt.stream_head(), &stream_head);
    assert_eq!(receipt.observation_revision(), 7);
    assert_eq!(receipt.effect_claim_id(), "effect-claim-1");
    assert_eq!(receipt.effect_subject_digest(), &digest('8'));
    assert_eq!(receipt.counters(), &resource_counters());
    assert_eq!(receipt.request(), &resource_request());
    assert_eq!(receipt.accounting_currency(), "TWD");
    assert_eq!(receipt.observation_digest(), &digest('6'));
    assert_eq!(receipt.receipt_digest(), &digest('7'));

    let head = receipt.head();
    assert_eq!(head.producer_id(), receipt.producer_id());
    assert_eq!(head.producer_version(), receipt.producer_version());
    assert_eq!(head.runtime(), receipt.runtime());
    assert_eq!(head.stream_head(), receipt.stream_head());
    assert_eq!(head.observation_revision(), receipt.observation_revision());
    assert_eq!(head.effect_claim_id(), receipt.effect_claim_id());
    assert_eq!(
        head.effect_subject_digest(),
        receipt.effect_subject_digest()
    );
    assert_eq!(head.counters(), receipt.counters());
    assert_eq!(head.request(), receipt.request());
    assert_eq!(head.accounting_currency(), receipt.accounting_currency());
    assert_eq!(head.observation_digest(), receipt.observation_digest());
    assert_eq!(head.receipt_digest(), receipt.receipt_digest());
}

#[test]
fn task_ledger_shared_values_reject_substituted_owner_runtime_and_invalid_usage() {
    let make_head = |producer, producer_version, runtime, revision, currency| {
        let identity = TaskLedgerStreamIdentity::new(
            ProjectId::new("project-1").expect("project"),
            ProjectSnapshotId::new("snapshot-1").expect("snapshot"),
            TaskId::new("TASK-013").expect("task"),
            revision,
            digest('2'),
            currency,
        )?;
        TaskLedgerStreamHead::new(
            CONTRACT_VERSION,
            producer,
            producer_version,
            runtime,
            identity,
            digest('1'),
            7,
            digest('3'),
            1,
            digest('4'),
            digest('5'),
        )
    };

    assert!(matches!(
        make_head(
            "substitute-ledger",
            TASK_LEDGER_PRODUCER_VERSION,
            RuntimeKind::Fake,
            "1",
            "TWD"
        ),
        Err(ContractError::UnsupportedTaskLedgerProducer)
    ));
    assert!(matches!(
        make_head(
            TASK_LEDGER_PRODUCER_ID,
            "9.9",
            RuntimeKind::Fake,
            "1",
            "TWD"
        ),
        Err(ContractError::UnsupportedTaskLedgerProducerVersion)
    ));
    assert!(matches!(
        make_head(
            TASK_LEDGER_PRODUCER_ID,
            TASK_LEDGER_PRODUCER_VERSION,
            RuntimeKind::Fake,
            "01",
            "TWD"
        ),
        Err(ContractError::InvalidTaskRevision)
    ));
    assert!(matches!(
        make_head(
            TASK_LEDGER_PRODUCER_ID,
            TASK_LEDGER_PRODUCER_VERSION,
            RuntimeKind::Fake,
            "1",
            "twd"
        ),
        Err(ContractError::InvalidAccountingCurrency)
    ));
    assert!(matches!(
        ResourceCounters::new(1, 2, 0, 1, 0, "0"),
        Err(ContractError::InvalidResourceUsage)
    ));
    assert!(matches!(
        ResourceRequest::new(1, 2, 0, 0, 0, Some("0")),
        Err(ContractError::InvalidResourceUsage)
    ));

    assert!(matches!(
        TaskLedgerResourceReceipt::new(
            CONTRACT_VERSION,
            TASK_LEDGER_PRODUCER_ID,
            TASK_LEDGER_PRODUCER_VERSION,
            RuntimeKind::Live,
            ledger_head(RuntimeKind::Fake, 7),
            7,
            "effect-claim-1",
            digest('8'),
            resource_counters(),
            resource_request(),
            "TWD",
            digest('6'),
            digest('7'),
        ),
        Err(ContractError::TaskLedgerRuntimeMismatch)
    ));
}

#[test]
fn writer_lease_shared_modes_and_positive_bigints_are_closed_and_bounded() {
    assert_eq!(
        RuntimeAdmissionMode::ALL,
        [
            RuntimeAdmissionMode::Active,
            RuntimeAdmissionMode::Draining,
            RuntimeAdmissionMode::Canary,
            RuntimeAdmissionMode::Stopped,
            RuntimeAdmissionMode::ReconciliationRequired,
        ]
    );
    assert_eq!(
        RuntimeAdmissionMode::ALL.map(RuntimeAdmissionMode::as_str),
        [
            "ACTIVE",
            "DRAINING",
            "CANARY",
            "STOPPED",
            "RECONCILIATION_REQUIRED",
        ]
    );
    assert_eq!(WriterLeaseStatus::Active.as_str(), "ACTIVE");
    assert_eq!(WriterLeaseStatus::Suspect.as_str(), "SUSPECT");

    let maximum = i64::MAX as u64;
    for value in [1, maximum] {
        assert_eq!(DaemonEpoch::new(value).expect("epoch").get(), value);
        assert_eq!(FencingToken::new(value).expect("fence").get(), value);
        assert_eq!(
            WriterLeaseRevision::new(value).expect("revision").get(),
            value
        );
        assert_eq!(
            HolderProcessId::new(value).expect("process id").get(),
            value
        );
    }

    for value in [0, maximum + 1] {
        assert!(matches!(
            DaemonEpoch::new(value),
            Err(ContractError::InvalidPositiveSignedBigInt {
                field: "daemon_epoch"
            })
        ));
        assert!(matches!(
            FencingToken::new(value),
            Err(ContractError::InvalidPositiveSignedBigInt {
                field: "fencing_token"
            })
        ));
        assert!(matches!(
            WriterLeaseRevision::new(value),
            Err(ContractError::InvalidPositiveSignedBigInt {
                field: "writer_lease_revision"
            })
        ));
        assert!(matches!(
            HolderProcessId::new(value),
            Err(ContractError::InvalidPositiveSignedBigInt {
                field: "holder_process_id"
            })
        ));
    }
}

fn make_writer_identity(
    holder_process_start_identity: ContentDigest,
) -> Result<WriterLeaseIdentity, ContractError> {
    WriterLeaseIdentity::new(
        ProjectId::new("project-1").expect("project"),
        ProjectSnapshotId::new("snapshot-1").expect("snapshot"),
        TaskId::new("TASK-014").expect("task"),
        "1",
        digest('1'),
        AttemptId::new("attempt-1").expect("attempt"),
        "lease-1",
        "implementer-1",
        "worktree-1",
        HolderProcessId::new(42).expect("process id"),
        holder_process_start_identity,
        "daemon-1",
        DaemonEpoch::new(7).expect("epoch"),
        FencingToken::new(11).expect("fence"),
    )
}

fn writer_identity() -> WriterLeaseIdentity {
    make_writer_identity(digest('6')).expect("valid writer lease identity")
}

#[test]
fn writer_lease_identity_is_exact_bounded_and_fully_observable() {
    let identity = writer_identity();
    assert_eq!(identity.project_id().as_str(), "project-1");
    assert_eq!(identity.project_snapshot_id().as_str(), "snapshot-1");
    assert_eq!(identity.task_id().as_str(), "TASK-014");
    assert_eq!(identity.task_revision(), "1");
    assert_eq!(identity.task_spec_digest(), &digest('1'));
    assert_eq!(identity.attempt_id().as_str(), "attempt-1");
    assert_eq!(identity.lease_id(), "lease-1");
    assert_eq!(identity.lease_holder_id(), "implementer-1");
    assert_eq!(identity.worktree_id(), "worktree-1");
    assert_eq!(identity.holder_process_id().get(), 42);
    assert_eq!(identity.holder_process_start_identity(), &digest('6'));
    assert_eq!(identity.daemon_instance_id(), "daemon-1");
    assert_eq!(identity.daemon_epoch().get(), 7);
    assert_eq!(identity.fencing_token().get(), 11);

    let invalid = |task: &str, revision: &str, attempt: &str, lease: &str| {
        WriterLeaseIdentity::new(
            ProjectId::new("project-1").expect("project"),
            ProjectSnapshotId::new("snapshot-1").expect("snapshot"),
            TaskId::new(task).expect("opaque task"),
            revision,
            digest('1'),
            AttemptId::new(attempt).expect("opaque attempt"),
            lease,
            "implementer-1",
            "worktree-1",
            HolderProcessId::new(42).expect("process id"),
            digest('6'),
            "daemon-1",
            DaemonEpoch::new(7).expect("epoch"),
            FencingToken::new(11).expect("fence"),
        )
    };

    assert!(matches!(
        invalid("bad task", "1", "attempt-1", "lease-1"),
        Err(ContractError::InvalidWriterLeaseIdentifier { field: "task_id" })
    ));
    assert!(matches!(
        invalid("TASK-014", "01", "attempt-1", "lease-1"),
        Err(ContractError::InvalidTaskRevision)
    ));
    assert!(matches!(
        invalid("TASK-014", "1", " attempt-1", "lease-1"),
        Err(ContractError::InvalidWriterLeaseIdentifier {
            field: "attempt_id"
        })
    ));
    assert!(matches!(
        invalid("TASK-014", "1", "attempt-1", &"a".repeat(129)),
        Err(ContractError::InvalidWriterLeaseIdentifier { field: "lease_id" })
    ));
}

#[test]
fn writer_lease_process_start_identity_is_an_exact_nonzero_sha256_subject() {
    for invalid in [
        String::new(),
        "A".repeat(64),
        "a".repeat(63),
        format!("{}g", "a".repeat(63)),
    ] {
        assert!(matches!(
            ContentDigest::from_sha256(invalid),
            Err(ContractError::MalformedSha256)
        ));
    }
    assert!(matches!(
        make_writer_identity(digest('0')),
        Err(ContractError::InvalidWriterLeaseIdentity {
            field: "holder_process_start_identity"
        })
    ));

    let first = make_writer_identity(digest('6')).expect("first identity");
    let second = make_writer_identity(digest('7')).expect("second identity");
    assert_ne!(first, second);

    let first_receipt = writer_receipt_for_identity(first, digest('5'));
    let second_receipt = writer_receipt_for_identity(second, digest('5'));
    assert_ne!(first_receipt, second_receipt);
    assert_ne!(first_receipt.head(), second_receipt.head());
}

fn writer_receipt(
    admission: RuntimeAdmissionMode,
    receipt_digest: ContentDigest,
) -> WriterLeaseAuthorityReceipt {
    let mut receipt = writer_receipt_for_identity(writer_identity(), receipt_digest);
    if admission != RuntimeAdmissionMode::Active {
        receipt = WriterLeaseAuthorityReceipt::new(
            CONTRACT_VERSION,
            WRITER_LEASE_PRODUCER_ID,
            WRITER_LEASE_PRODUCER_VERSION,
            RuntimeKind::Fake,
            receipt.identity().clone(),
            receipt.status(),
            receipt.revision(),
            admission,
            receipt.acquired_at(),
            receipt.heartbeat_at(),
            receipt.expires_at(),
            receipt.time_observation_digest().clone(),
            receipt.admission_observation_digest().clone(),
            receipt.transition_digest().clone(),
            receipt.receipt_digest().clone(),
        )
        .expect("valid writer lease receipt");
    }
    receipt
}

fn writer_receipt_for_identity(
    identity: WriterLeaseIdentity,
    receipt_digest: ContentDigest,
) -> WriterLeaseAuthorityReceipt {
    WriterLeaseAuthorityReceipt::new(
        CONTRACT_VERSION,
        WRITER_LEASE_PRODUCER_ID,
        WRITER_LEASE_PRODUCER_VERSION,
        RuntimeKind::Fake,
        identity,
        WriterLeaseStatus::Active,
        WriterLeaseRevision::new(3).expect("revision"),
        RuntimeAdmissionMode::Active,
        "2026-07-30T00:00:00Z",
        "2026-07-30T00:01:00Z",
        "2026-07-30T00:02:00Z",
        digest('2'),
        digest('3'),
        digest('4'),
        receipt_digest,
    )
    .expect("valid writer lease receipt")
}

#[test]
fn writer_lease_receipt_and_structural_head_bind_every_security_field() {
    let receipt = writer_receipt(RuntimeAdmissionMode::Active, digest('5'));
    let head = receipt.head();

    assert_eq!(receipt.version(), CONTRACT_VERSION);
    assert_eq!(receipt.producer_id(), WRITER_LEASE_PRODUCER_ID);
    assert_eq!(receipt.producer_version(), WRITER_LEASE_PRODUCER_VERSION);
    assert_eq!(receipt.runtime(), RuntimeKind::Fake);
    assert_eq!(receipt.identity(), &writer_identity());
    assert_eq!(receipt.status(), WriterLeaseStatus::Active);
    assert_eq!(receipt.revision().get(), 3);
    assert_eq!(receipt.runtime_admission(), RuntimeAdmissionMode::Active);
    assert_eq!(receipt.acquired_at(), "2026-07-30T00:00:00Z");
    assert_eq!(receipt.heartbeat_at(), "2026-07-30T00:01:00Z");
    assert_eq!(receipt.expires_at(), "2026-07-30T00:02:00Z");
    assert_eq!(receipt.time_observation_digest(), &digest('2'));
    assert_eq!(receipt.admission_observation_digest(), &digest('3'));
    assert_eq!(receipt.transition_digest(), &digest('4'));
    assert_eq!(receipt.receipt_digest(), &digest('5'));

    assert_eq!(head.version(), receipt.version());
    assert_eq!(head.producer_id(), receipt.producer_id());
    assert_eq!(head.producer_version(), receipt.producer_version());
    assert_eq!(head.runtime(), receipt.runtime());
    assert_eq!(head.identity(), receipt.identity());
    assert_eq!(head.status(), receipt.status());
    assert_eq!(head.revision(), receipt.revision());
    assert_eq!(head.runtime_admission(), receipt.runtime_admission());
    assert_eq!(head.acquired_at(), receipt.acquired_at());
    assert_eq!(head.heartbeat_at(), receipt.heartbeat_at());
    assert_eq!(head.expires_at(), receipt.expires_at());
    assert_eq!(
        head.time_observation_digest(),
        receipt.time_observation_digest()
    );
    assert_eq!(
        head.admission_observation_digest(),
        receipt.admission_observation_digest()
    );
    assert_eq!(head.transition_digest(), receipt.transition_digest());
    assert_eq!(head.receipt_digest(), receipt.receipt_digest());

    assert_ne!(
        head,
        writer_receipt(RuntimeAdmissionMode::Draining, digest('5')).head()
    );
    assert_ne!(
        head,
        writer_receipt(RuntimeAdmissionMode::Active, digest('6')).head()
    );
}

#[test]
fn writer_lease_receipt_rejects_wrong_owner_and_invalid_receipt_fields() {
    let make = |producer, producer_version, acquired_at, receipt_digest| {
        WriterLeaseAuthorityReceipt::new(
            CONTRACT_VERSION,
            producer,
            producer_version,
            RuntimeKind::Fake,
            writer_identity(),
            WriterLeaseStatus::Active,
            WriterLeaseRevision::new(3).expect("revision"),
            RuntimeAdmissionMode::Active,
            acquired_at,
            "2026-07-30T00:01:00Z",
            "2026-07-30T00:02:00Z",
            digest('2'),
            digest('3'),
            digest('4'),
            receipt_digest,
        )
    };

    assert!(matches!(
        make(
            "substitute-writer-lease",
            WRITER_LEASE_PRODUCER_VERSION,
            "2026-07-30T00:00:00Z",
            digest('5')
        ),
        Err(ContractError::UnsupportedWriterLeaseProducer)
    ));
    assert!(matches!(
        make(
            WRITER_LEASE_PRODUCER_ID,
            "9.9",
            "2026-07-30T00:00:00Z",
            digest('5')
        ),
        Err(ContractError::UnsupportedWriterLeaseProducerVersion)
    ));
    assert!(matches!(
        make(
            WRITER_LEASE_PRODUCER_ID,
            WRITER_LEASE_PRODUCER_VERSION,
            " 2026-07-30T00:00:00Z",
            digest('5')
        ),
        Err(ContractError::InvalidWriterLeaseReceipt {
            field: "acquired_at"
        })
    ));
    assert!(matches!(
        make(
            WRITER_LEASE_PRODUCER_ID,
            WRITER_LEASE_PRODUCER_VERSION,
            "2026-07-30T00:00:00Z",
            digest('0')
        ),
        Err(ContractError::InvalidWriterLeaseReceipt {
            field: "receipt_digest"
        })
    ));
}

fn artifact_object(project_id: &str, digest_byte: char, generation: u64) -> ArtifactObjectIdentity {
    ArtifactObjectIdentity::new(
        ArtifactObjectKey::new(
            ProjectId::new(project_id).expect("project"),
            digest(digest_byte),
        ),
        ArtifactGeneration::new(generation).expect("generation"),
    )
}

fn artifact_reference_authority(
    action: ArtifactReferenceAuthorityAction,
    runtime: RuntimeKind,
    object: ArtifactObjectIdentity,
    reference_id: &str,
    receipt_digest: ContentDigest,
) -> ArtifactReferenceAuthorityPair {
    let binding = ArtifactReferenceAuthorityBinding::new(
        ArtifactAuthorityOwnerKind::TaskLedger,
        runtime,
        "owner-record-1",
        ArtifactRevision::new(4).expect("owner revision"),
        ArtifactAuthorityStatus::Available,
        action,
        object.key().project_id().clone(),
        TaskId::new("TASK-016").expect("task"),
        object,
        reference_id,
        digest('1'),
    )
    .expect("reference authority binding");
    let receipt = ArtifactReferenceAuthorityReceipt::new(
        CONTRACT_VERSION,
        binding.clone(),
        receipt_digest.clone(),
    )
    .expect("reference authority receipt");
    let head = ArtifactReferenceAuthorityHead::new(CONTRACT_VERSION, binding, receipt_digest)
        .expect("reference authority head");
    ArtifactReferenceAuthorityPair::new(receipt, head).expect("matching reference authority pair")
}

fn artifact_read_authority(
    action: ArtifactReadAuthorityAction,
    runtime: RuntimeKind,
    object: ArtifactObjectIdentity,
    read_claim_id: &str,
    receipt_digest: ContentDigest,
) -> ArtifactReadAuthorityPair {
    let binding = ArtifactReadAuthorityBinding::new(
        ArtifactAuthorityOwnerKind::TaskLedger,
        runtime,
        "read-owner-record-1",
        ArtifactRevision::new(5).expect("owner revision"),
        ArtifactAuthorityStatus::Available,
        action,
        object.key().project_id().clone(),
        TaskId::new("TASK-016").expect("task"),
        object,
        read_claim_id,
        digest('2'),
    )
    .expect("read authority binding");
    let receipt = ArtifactReadAuthorityReceipt::new(
        CONTRACT_VERSION,
        binding.clone(),
        receipt_digest.clone(),
    )
    .expect("read authority receipt");
    let head = ArtifactReadAuthorityHead::new(CONTRACT_VERSION, binding, receipt_digest)
        .expect("read authority head");
    ArtifactReadAuthorityPair::new(receipt, head).expect("matching read authority pair")
}

fn artifact_sweep_authority(
    runtime: RuntimeKind,
    object: ArtifactObjectIdentity,
    receipt_digest: ContentDigest,
) -> ArtifactSweepAuthorityPair {
    let binding = ArtifactSweepAuthorityBinding::new(
        runtime,
        "sweep-record-1",
        ArtifactRevision::new(6).expect("owner revision"),
        ArtifactAuthorityStatus::Available,
        ArtifactSweepAuthorityAction::ClaimDelete,
        object,
        digest('3'),
        digest('4'),
        digest('5'),
        "2026-07-30T04:00:00Z",
        "2026-07-30T04:05:00Z",
        digest('6'),
        "daemon-1",
        DaemonEpoch::new(7).expect("daemon epoch"),
        RuntimeAdmissionMode::Active,
        digest('7'),
    )
    .expect("sweep authority binding");
    let receipt = ArtifactSweepAuthorityReceipt::new(
        CONTRACT_VERSION,
        binding.clone(),
        receipt_digest.clone(),
    )
    .expect("sweep authority receipt");
    let head = ArtifactSweepAuthorityHead::new(CONTRACT_VERSION, binding, receipt_digest)
        .expect("sweep authority head");
    ArtifactSweepAuthorityPair::new(receipt, head).expect("matching sweep authority pair")
}

fn artifact_provenance(runtime: RuntimeKind) -> ArtifactProvenance {
    artifact_provenance_at(runtime, "2026-07-30T03:00:00Z").expect("complete artifact provenance")
}

fn artifact_provenance_at(
    runtime: RuntimeKind,
    produced_at: &str,
) -> Result<ArtifactProvenance, ContractError> {
    ArtifactProvenance::new(
        "graphify",
        "0.9.0",
        runtime,
        digest('8'),
        "graphify-adapter",
        "1.0",
        digest('9'),
        "invocation-1",
        "correlation-1",
        "run-1",
        ArtifactCounter::new(0).expect("sequence"),
        produced_at,
        digest('a'),
        "capability-graph-build",
        digest('b'),
        digest('c'),
        digest('d'),
        digest('e'),
        digest('f'),
        "effect-claim-1",
        digest('1'),
        "daemon-1",
        DaemonEpoch::new(7).expect("daemon epoch"),
        RuntimeAdmissionMode::Active,
        digest('2'),
        digest('3'),
        digest('4'),
    )
}

fn artifact_manifest(
    object: ArtifactObjectIdentity,
    authority: ArtifactReferenceAuthorityPair,
) -> ArtifactReferenceManifest {
    ArtifactReferenceManifest::new(
        SubjectBinding::new(
            object.key().project_id().clone(),
            ProjectSnapshotId::new("snapshot-16").expect("snapshot"),
            TaskId::new("TASK-016").expect("task"),
            "2",
            digest('5'),
        )
        .expect("subject binding"),
        AttemptId::new("attempt-1").expect("attempt"),
        RequestId::new("request-1").expect("request"),
        "reference-1",
        object,
        ArtifactByteLength::new(42).expect("byte length"),
        "application/vnd.lattice.graph+json",
        "graphify.graph",
        "1.0",
        Some(
            ArtifactBundleBounds::new(
                ArtifactCounter::new(2).expect("entries"),
                ArtifactCounter::new(1).expect("depth"),
                ArtifactByteLength::new(42).expect("bundle bytes"),
            )
            .expect("bundle bounds"),
        ),
        artifact_provenance(RuntimeKind::Fake),
        authority,
        ArtifactPurpose::GraphifyGraph,
        "2026-08-30T03:00:00Z",
        digest('6'),
    )
    .expect("complete artifact reference manifest")
}

fn available_artifact_object_head(object: ArtifactObjectIdentity) -> ArtifactObjectHead {
    ArtifactObjectHead::new(
        object,
        ArtifactRevision::new(7).expect("object revision"),
        ArtifactAvailability::Available,
        ArtifactByteLength::new(42).expect("length"),
        ArtifactCounter::new(1).expect("reference count"),
        digest('7'),
        "2026-08-30T03:05:00Z",
        ArtifactCounter::new(0).expect("read count"),
        digest('8'),
        ArtifactDeleteStatus::NotClaimed,
        None,
        digest('9'),
        digest('a'),
        digest('b'),
        digest('c'),
        ArtifactCounter::new(1).expect("command high water"),
        digest('d'),
        digest('e'),
    )
    .expect("available object head")
}

#[test]
fn artifact_numeric_values_and_project_scoped_identity_fail_closed() {
    assert_eq!(ArtifactGeneration::new(1).expect("generation").get(), 1);
    assert_eq!(
        ArtifactRevision::new(i64::MAX as u64)
            .expect("revision")
            .get(),
        i64::MAX as u64
    );
    assert_eq!(
        ArtifactGeneration::new(0),
        Err(ContractError::InvalidPositiveSignedBigInt {
            field: "artifact_generation"
        })
    );
    assert_eq!(ArtifactByteLength::new(0).expect("empty artifact").get(), 0);
    assert_eq!(
        ArtifactCounter::new(i64::MAX as u64)
            .expect("counter")
            .get(),
        i64::MAX as u64
    );
    assert_eq!(
        ArtifactQuotaValue::new((i64::MAX as u64) + 1),
        Err(ContractError::InvalidNonNegativeSignedBigInt {
            field: "artifact_quota_value"
        })
    );

    let first = artifact_object("project-1", 'a', 1);
    let second = artifact_object("project-2", 'a', 1);
    assert_ne!(first, second);
    assert_eq!(first.key().algorithm(), "sha256");
    assert_eq!(first.generation().get(), 1);

    assert_eq!(
        ArtifactBundleBounds::new(
            ArtifactCounter::new(100_001).expect("representable counter"),
            ArtifactCounter::new(1).expect("depth"),
            ArtifactByteLength::new(1).expect("bytes"),
        ),
        Err(ContractError::InvalidArtifactValue {
            field: "bundle_entry_count"
        })
    );
}

#[test]
fn artifact_manifest_binds_complete_provenance_scope_bundle_and_owner_authority() {
    let object = artifact_object("project-1", 'a', 1);
    let authority = artifact_reference_authority(
        ArtifactReferenceAuthorityAction::PublishInitialReference,
        RuntimeKind::Fake,
        object.clone(),
        "reference-1",
        digest('7'),
    );
    let manifest = artifact_manifest(object.clone(), authority.clone());

    assert_eq!(manifest.binding().project_id(), object.key().project_id());
    assert_eq!(manifest.object(), &object);
    assert_eq!(manifest.byte_length().get(), 42);
    assert_eq!(manifest.reference_id(), "reference-1");
    assert_eq!(manifest.creation_authority(), &authority);
    assert_eq!(
        manifest.provenance().registry_authority_receipt_digest(),
        &digest('e')
    );
    assert_eq!(
        manifest.provenance().capability_owner_current_head_digest(),
        &digest('3')
    );
    assert_eq!(manifest.provenance().limit_snapshot_digest(), &digest('4'));
    assert_eq!(manifest.bundle().expect("bundle").entry_count().get(), 2);

    let wrong_project_binding = SubjectBinding::new(
        ProjectId::new("project-2").expect("project"),
        ProjectSnapshotId::new("snapshot-16").expect("snapshot"),
        TaskId::new("TASK-016").expect("task"),
        "2",
        digest('5'),
    )
    .expect("subject binding");
    assert!(matches!(
        ArtifactReferenceManifest::new(
            wrong_project_binding,
            AttemptId::new("attempt-1").expect("attempt"),
            RequestId::new("request-1").expect("request"),
            "reference-1",
            object,
            ArtifactByteLength::new(42).expect("length"),
            "application/json",
            "graphify.graph",
            "1.0",
            None,
            artifact_provenance(RuntimeKind::Fake),
            authority,
            ArtifactPurpose::GraphifyGraph,
            "2026-08-30T03:00:00Z",
            digest('6'),
        ),
        Err(ContractError::InvalidArtifactValue {
            field: "project_scope"
        })
    ));
}

#[test]
fn artifact_time_fields_require_canonical_ascii_utc_seconds() {
    assert_eq!(
        artifact_provenance_at(RuntimeKind::Fake, "xxxx-xx-xxTxx:xx:xxZ"),
        Err(ContractError::InvalidArtifactValue {
            field: "produced_at"
        })
    );
    assert_eq!(
        artifact_provenance_at(RuntimeKind::Fake, "2026-13-30T03:00:00Z"),
        Err(ContractError::InvalidArtifactValue {
            field: "produced_at"
        })
    );
    assert_eq!(
        artifact_provenance_at(RuntimeKind::Fake, "2026-02-29T03:00:00Z"),
        Err(ContractError::InvalidArtifactValue {
            field: "produced_at"
        })
    );
    assert!(artifact_provenance_at(RuntimeKind::Fake, "2028-02-29T23:59:59Z").is_ok());
}

#[test]
fn artifact_reference_read_and_sweep_authority_pairs_bind_exact_current_heads() {
    let object = artifact_object("project-1", 'a', 1);
    let reference = artifact_reference_authority(
        ArtifactReferenceAuthorityAction::AddReference,
        RuntimeKind::Fake,
        object.clone(),
        "reference-1",
        digest('8'),
    );
    assert_eq!(
        reference.receipt().binding().action(),
        ArtifactReferenceAuthorityAction::AddReference
    );
    assert_eq!(reference.receipt().binding().object(), &object);
    assert_eq!(reference.current_head(), &reference.receipt().head());

    let changed_reference_binding = ArtifactReferenceAuthorityBinding::new(
        ArtifactAuthorityOwnerKind::TaskLedger,
        RuntimeKind::Live,
        "owner-record-1",
        ArtifactRevision::new(4).expect("owner revision"),
        ArtifactAuthorityStatus::Available,
        ArtifactReferenceAuthorityAction::AddReference,
        object.key().project_id().clone(),
        TaskId::new("TASK-016").expect("task"),
        object.clone(),
        "reference-1",
        digest('1'),
    )
    .expect("changed reference binding");
    let changed_reference_head = ArtifactReferenceAuthorityHead::new(
        CONTRACT_VERSION,
        changed_reference_binding,
        digest('8'),
    )
    .expect("changed current head");
    assert_eq!(
        ArtifactReferenceAuthorityPair::new(reference.receipt().clone(), changed_reference_head),
        Err(ContractError::ArtifactAuthorityHeadMismatch {
            field: "reference_authority"
        })
    );

    let read = artifact_read_authority(
        ArtifactReadAuthorityAction::AcquireRead,
        RuntimeKind::Fake,
        object.clone(),
        "read-1",
        digest('9'),
    );
    assert_eq!(read.receipt().binding().object(), &object);
    assert_eq!(read.receipt().binding().read_claim_id(), "read-1");
    assert_eq!(read.current_head(), &read.receipt().head());

    let sweep = artifact_sweep_authority(RuntimeKind::Fake, object.clone(), digest('a'));
    assert_eq!(
        sweep.receipt().binding().action(),
        ArtifactSweepAuthorityAction::ClaimDelete
    );
    assert_eq!(sweep.receipt().binding().object(), &object);
    assert_eq!(sweep.current_head(), &sweep.receipt().head());
    assert_eq!(
        sweep.receipt().binding().producer_id(),
        ARTIFACT_STORE_PRODUCER_ID
    );
}

#[test]
fn artifact_statuses_are_closed_and_receipt_head_mirrors_every_nested_field() {
    assert_eq!(ArtifactAvailability::Available.as_str(), "AVAILABLE");
    assert_eq!(
        ArtifactAvailability::ReconciliationRequired.as_str(),
        "RECONCILIATION_REQUIRED"
    );
    assert_eq!(ArtifactDeleteStatus::Claimed.as_str(), "CLAIMED");
    assert_eq!(
        ArtifactReadStatus::ExpiredSuspect.as_str(),
        "EXPIRED_SUSPECT"
    );
    assert_eq!(ArtifactReferenceStatus::Released.as_str(), "RELEASED");

    let object = artifact_object("project-1", 'a', 1);
    let creation_authority = artifact_reference_authority(
        ArtifactReferenceAuthorityAction::PublishInitialReference,
        RuntimeKind::Fake,
        object.clone(),
        "reference-1",
        digest('7'),
    );
    let reference_head = ArtifactReferenceHead::new(
        artifact_manifest(object.clone(), creation_authority.clone()),
        creation_authority,
        ArtifactRevision::new(7).expect("reference revision"),
        ArtifactReferenceStatus::Active,
        digest('f'),
    )
    .expect("reference head");
    let read_authority = artifact_read_authority(
        ArtifactReadAuthorityAction::AcquireRead,
        RuntimeKind::Fake,
        object.clone(),
        "read-1",
        digest('9'),
    );
    let read_head = ArtifactReadHead::new(
        read_authority,
        ArtifactRevision::new(1).expect("read revision"),
        ArtifactReadStatus::Active,
        "holder-1",
        "2026-07-30T03:01:00Z",
        "2026-07-30T03:16:00Z",
        digest('a'),
    )
    .expect("read head");
    let receipt = ArtifactAuthorityReceipt::new(
        CONTRACT_VERSION,
        ARTIFACT_STORE_PRODUCER_ID,
        ARTIFACT_STORE_PRODUCER_VERSION,
        RuntimeKind::Fake,
        available_artifact_object_head(object),
        Some(reference_head),
        Some(read_head),
        digest('b'),
        digest('c'),
    )
    .expect("artifact authority receipt");
    let head = receipt.head();

    assert_eq!(receipt.producer_id(), ARTIFACT_STORE_PRODUCER_ID);
    assert_eq!(receipt.producer_version(), ARTIFACT_STORE_PRODUCER_VERSION);
    assert_eq!(head, receipt.head());
    assert_eq!(head.object(), receipt.object());
    assert_eq!(head.reference(), receipt.reference());
    assert_eq!(head.read(), receipt.read());
    assert_eq!(head.observation_digest(), receipt.observation_digest());
    assert_eq!(head.receipt_digest(), receipt.receipt_digest());

    let rebuilt = ArtifactAuthorityHead::new(
        CONTRACT_VERSION,
        ARTIFACT_STORE_PRODUCER_ID,
        ARTIFACT_STORE_PRODUCER_VERSION,
        RuntimeKind::Fake,
        receipt.object().clone(),
        receipt.reference().cloned(),
        receipt.read().cloned(),
        digest('b'),
        digest('d'),
    )
    .expect("substituted structural head");
    assert_ne!(rebuilt, head);
}

#[test]
fn artifact_constructors_reject_owner_status_scope_and_delete_state_substitution() {
    let object = artifact_object("project-1", 'a', 1);
    assert_eq!(
        ArtifactReferenceAuthorityBinding::new(
            ArtifactAuthorityOwnerKind::ArtifactStore,
            RuntimeKind::Fake,
            "owner-record-1",
            ArtifactRevision::new(1).expect("revision"),
            ArtifactAuthorityStatus::Available,
            ArtifactReferenceAuthorityAction::PublishInitialReference,
            object.key().project_id().clone(),
            TaskId::new("TASK-016").expect("task"),
            object.clone(),
            "reference-1",
            digest('1'),
        ),
        Err(ContractError::InvalidArtifactAuthority {
            field: "reference_owner_kind"
        })
    );
    assert_eq!(
        ArtifactObjectHead::new(
            object.clone(),
            ArtifactRevision::new(1).expect("revision"),
            ArtifactAvailability::DeleteClaimed,
            ArtifactByteLength::new(42).expect("length"),
            ArtifactCounter::new(0).expect("references"),
            digest('2'),
            "2026-07-30T04:00:00Z",
            ArtifactCounter::new(0).expect("reads"),
            digest('3'),
            ArtifactDeleteStatus::NotClaimed,
            None,
            digest('4'),
            digest('5'),
            digest('6'),
            digest('7'),
            ArtifactCounter::new(1).expect("commands"),
            digest('8'),
            digest('9'),
        ),
        Err(ContractError::InvalidArtifactValue {
            field: "delete_state"
        })
    );
    assert_eq!(
        ArtifactAuthorityReceipt::new(
            CONTRACT_VERSION,
            "graphify",
            ARTIFACT_STORE_PRODUCER_VERSION,
            RuntimeKind::Fake,
            available_artifact_object_head(object),
            None,
            None,
            digest('b'),
            digest('c'),
        ),
        Err(ContractError::UnsupportedArtifactStoreProducer)
    );
}

#[test]
fn artifact_read_closure_evidence_is_fixed_owner_and_full_head_bound() {
    let object = artifact_object("project-1", 'a', 1);
    let build = |daemon_instance: &str| {
        ArtifactReadClosureEvidenceBinding::new(
            RuntimeKind::Fake,
            "closure-record-1",
            ArtifactRevision::new(1).expect("revision"),
            ArtifactAuthorityStatus::Available,
            ArtifactReadClosureEvidenceKind::HandleClosed,
            object.clone(),
            TaskId::new("TASK-016").expect("task"),
            "read-1",
            "holder-1",
            daemon_instance,
            DaemonEpoch::new(7).expect("epoch"),
            "2026-07-30T00:16:00Z",
            digest('1'),
        )
        .expect("closure evidence binding")
    };
    let receipt =
        ArtifactReadClosureEvidenceReceipt::new(CONTRACT_VERSION, build("daemon-1"), digest('2'))
            .expect("closure receipt");
    assert_eq!(
        receipt.binding().producer_id(),
        ARTIFACT_READ_CLOSURE_PRODUCER_ID
    );
    assert_eq!(
        receipt.binding().producer_version(),
        ARTIFACT_READ_CLOSURE_PRODUCER_VERSION
    );
    assert_eq!(
        ArtifactReadClosureEvidencePair::new(receipt.clone(), receipt.head())
            .expect("full pair")
            .receipt(),
        &receipt
    );

    let substituted =
        ArtifactReadClosureEvidenceReceipt::new(CONTRACT_VERSION, build("daemon-2"), digest('2'))
            .expect("substituted receipt")
            .head();
    assert_eq!(
        ArtifactReadClosureEvidencePair::new(receipt, substituted),
        Err(ContractError::ArtifactAuthorityHeadMismatch {
            field: "read_closure_evidence"
        })
    );

    assert_eq!(
        ArtifactReadClosureEvidenceBinding::new(
            RuntimeKind::Fake,
            "closure-record-2",
            ArtifactRevision::new(1).expect("revision"),
            ArtifactAuthorityStatus::Consumed,
            ArtifactReadClosureEvidenceKind::HolderDeath,
            object,
            TaskId::new("TASK-016").expect("task"),
            "read-2",
            "holder-2",
            "daemon-1",
            DaemonEpoch::new(7).expect("epoch"),
            "2026-07-30T00:16:00Z",
            digest('3'),
        ),
        Err(ContractError::InvalidArtifactAuthority {
            field: "closure_evidence_status"
        })
    );
}

#[test]
fn gateway_values_are_bounded_fake_and_action_specific() {
    let actor = GatewayCommandId::new("actor-1").expect("actor-shaped id");
    assert_eq!(actor.as_str(), "actor-1");
    assert!(GatewayCommandId::new("x".repeat(257)).is_err());
    assert!(GatewayCommandId::new("not ascii: 中").is_err());

    let peer = GatewayPeerContext::new_fake(
        GatewayClientKind::OpenClaw,
        GatewayInstanceId::new("gateway-1").expect("gateway"),
        GatewayAdapterId::new("openclaw-adapter").expect("adapter"),
        "2.0",
        digest('a'),
        digest('b'),
        GatewayActorId::new("actor-1").expect("actor"),
        GatewayActorKind::ResponsibleUser,
        GatewayChannelId::new("channel-1").expect("channel"),
        GatewaySessionId::new("session-1").expect("session"),
        1,
        digest('c'),
        digest('c'),
    )
    .expect("fake peer");
    assert_eq!(peer.client_kind(), GatewayClientKind::OpenClaw);
    assert_eq!(peer.actor_id().as_str(), "actor-1");
    assert_eq!(peer.runtime(), RuntimeKind::Fake);

    let binding = approval_binding();
    let document = br#"{"goal":"must-not-leak"}"#.to_vec();
    let submission = TaskSpecSubmission::new(binding.clone(), document.clone(), digest('1'))
        .expect("submission");
    assert_eq!(submission.binding(), &binding);
    assert_eq!(submission.canonical_document(), document.as_slice());
    let debug = format!("{submission:?}");
    assert!(!debug.contains("must-not-leak"));
    assert!(debug.contains("canonical_document_bytes"));

    let target = GatewayTaskTarget::new(binding.clone(), digest('2')).expect("target");
    let approval = GatewayApprovalRoute::new(
        binding.clone(),
        GatewayNormalApprovalKind::Execution,
        GatewayApprovalId::new("approval-1").expect("approval"),
        GatewayChallengeId::new("challenge-1").expect("challenge"),
        digest('3'),
        digest('4'),
        digest('5'),
    )
    .expect("approval route");
    let stop = GatewayStopTarget::new(
        target.clone(),
        AttemptId::new("attempt-1").expect("attempt"),
        GatewayStopReason::UserRequested,
    )
    .expect("stop target");
    let bodies = [
        GatewayRequestBody::Submit(submission),
        GatewayRequestBody::Plan(target.clone()),
        GatewayRequestBody::Status(GatewayStatusTarget::Task(target)),
        GatewayRequestBody::Approve(approval.clone()),
        GatewayRequestBody::Reject(approval),
        GatewayRequestBody::Stop(stop),
    ];
    assert_eq!(bodies.clone().map(|body| body.action()), GatewayAction::ALL);

    let request = GatewayRequest::new(
        1,
        GatewayCommandId::new("command-1").expect("command"),
        GatewayCorrelationId::new("correlation-1").expect("correlation"),
        bodies[1].clone(),
        digest('5'),
    )
    .expect("request");
    assert_eq!(request.action(), GatewayAction::Plan);
    assert_eq!(request.request_digest(), &digest('5'));
}

#[test]
fn gateway_replies_bind_request_identity_action_and_outcome() {
    let binding = approval_binding();
    let request = GatewayRequest::new(
        1,
        GatewayCommandId::new("command-1").expect("command"),
        GatewayCorrelationId::new("correlation-1").expect("correlation"),
        GatewayRequestBody::Status(GatewayStatusTarget::Task(
            GatewayTaskTarget::new(binding.clone(), digest('2')).expect("target"),
        )),
        digest('1'),
    )
    .expect("request");
    let projection = GatewayTaskProjection::new(
        binding.clone(),
        GatewayTaskState::Draft,
        digest('2'),
        digest('3'),
    )
    .expect("projection");
    let reply = GatewayReply::new(
        &request,
        GatewayReplyBody::StatusObserved(GatewayStatusObservation::Task(projection)),
        digest('4'),
    )
    .expect("reply");
    assert_eq!(reply.action(), GatewayAction::Status);
    assert_eq!(reply.request_digest(), &digest('1'));
    assert_eq!(reply.reply_digest(), &digest('4'));

    let other_binding = SubjectBinding::new(
        ProjectId::new("project-2").expect("project"),
        ProjectSnapshotId::new("snapshot-2").expect("snapshot"),
        TaskId::new("TASK-017").expect("task"),
        "1",
        digest('7'),
    )
    .expect("other binding");
    assert_eq!(
        GatewayReply::new(
            &request,
            GatewayReplyBody::StatusObserved(GatewayStatusObservation::Task(
                GatewayTaskProjection::new(
                    other_binding,
                    GatewayTaskState::Draft,
                    digest('8'),
                    digest('9'),
                )
                .expect("other projection"),
            )),
            digest('a'),
        ),
        Err(ContractError::GatewayReplyActionMismatch)
    );

    assert!(
        GatewayReply::new(
            &request,
            GatewayReplyBody::StopRouted {
                target: GatewayStopTarget::new(
                    GatewayTaskTarget::new(binding, digest('2')).expect("target"),
                    AttemptId::new("attempt-1").expect("attempt"),
                    GatewayStopReason::UserRequested,
                )
                .expect("stop target"),
                disposition: GatewayStopDisposition::Requested,
                routing_receipt_digest: digest('5'),
            },
            digest('6'),
        )
        .is_err()
    );
    assert_eq!(
        GatewayDenialCode::ProtectedSurfaceRequired.as_str(),
        "PROTECTED_SURFACE_REQUIRED"
    );
    assert_eq!(
        GatewayUnknownCode::DownstreamAmbiguous.as_str(),
        "DOWNSTREAM_AMBIGUOUS"
    );
}

#[test]
fn gateway_authority_and_evidence_zero_sentinels_fail_closed() {
    let binding = approval_binding();
    assert!(GatewayTaskTarget::new(binding.clone(), digest('0')).is_err());
    for digests in [
        [digest('0'), digest('2'), digest('3')],
        [digest('1'), digest('0'), digest('3')],
        [digest('1'), digest('2'), digest('0')],
    ] {
        assert!(
            GatewayApprovalRoute::new(
                binding.clone(),
                GatewayNormalApprovalKind::Execution,
                GatewayApprovalId::new("approval-zero").expect("approval"),
                GatewayChallengeId::new("challenge-zero").expect("challenge"),
                digests[0].clone(),
                digests[1].clone(),
                digests[2].clone(),
            )
            .is_err()
        );
    }
    assert!(
        GatewayTaskProjection::new(
            binding.clone(),
            GatewayTaskState::Draft,
            digest('0'),
            digest('4'),
        )
        .is_err()
    );

    let target = GatewayTaskTarget::new(binding.clone(), digest('2')).expect("target");
    let plan = GatewayRequest::new(
        1,
        GatewayCommandId::new("zero-plan").expect("command"),
        GatewayCorrelationId::new("zero-plan-correlation").expect("correlation"),
        GatewayRequestBody::Plan(target.clone()),
        digest('5'),
    )
    .expect("plan request");
    assert!(
        GatewayReply::new(
            &plan,
            GatewayReplyBody::PlanRouted {
                binding: binding.clone(),
                command_receipt_digest: digest('0'),
            },
            digest('6'),
        )
        .is_err()
    );

    let stop_target = GatewayStopTarget::new(
        target,
        AttemptId::new("attempt-zero").expect("attempt"),
        GatewayStopReason::UserRequested,
    )
    .expect("stop target");
    let stop = GatewayRequest::new(
        1,
        GatewayCommandId::new("zero-stop").expect("command"),
        GatewayCorrelationId::new("zero-stop-correlation").expect("correlation"),
        GatewayRequestBody::Stop(stop_target.clone()),
        digest('7'),
    )
    .expect("stop request");
    assert!(
        GatewayReply::new(
            &stop,
            GatewayReplyBody::StopRouted {
                target: stop_target,
                disposition: GatewayStopDisposition::Requested,
                routing_receipt_digest: digest('0'),
            },
            digest('8'),
        )
        .is_err()
    );

    let command_status = GatewayRequest::new(
        1,
        GatewayCommandId::new("zero-status").expect("command"),
        GatewayCorrelationId::new("zero-status-correlation").expect("correlation"),
        GatewayRequestBody::Status(GatewayStatusTarget::Command {
            project_id: binding.project_id().clone(),
            original_command_id: GatewayCommandId::new("original").expect("original"),
        }),
        digest('9'),
    )
    .expect("status request");
    assert!(
        GatewayReply::new(
            &command_status,
            GatewayReplyBody::StatusObserved(GatewayStatusObservation::Command {
                project_id: binding.project_id().clone(),
                original_command_id: GatewayCommandId::new("original").expect("original"),
                terminal_reply_digest: digest('0'),
            }),
            digest('a'),
        )
        .is_err()
    );
}

#[test]
fn gateway_reused_identifier_boundaries_are_exact() {
    fn binding(snapshot: usize, task: usize) -> SubjectBinding {
        SubjectBinding::new(
            ProjectId::new("project-1").expect("project"),
            ProjectSnapshotId::new("s".repeat(snapshot)).expect("snapshot"),
            TaskId::new("t".repeat(task)).expect("task"),
            "1",
            digest('1'),
        )
        .expect("binding")
    }

    let exact = GatewayTaskTarget::new(binding(256, 256), digest('2')).expect("exact target");
    assert!(GatewayTaskTarget::new(binding(257, 256), digest('2')).is_err());
    assert!(GatewayTaskTarget::new(binding(256, 257), digest('2')).is_err());
    assert!(
        GatewayStopTarget::new(
            exact.clone(),
            AttemptId::new("a".repeat(256)).expect("attempt"),
            GatewayStopReason::UserRequested,
        )
        .is_ok()
    );
    assert!(
        GatewayStopTarget::new(
            exact,
            AttemptId::new("a".repeat(257)).expect("attempt"),
            GatewayStopReason::UserRequested,
        )
        .is_err()
    );
    assert!(TaskSpecSubmission::new(binding(257, 1), b"{}".to_vec(), digest('1')).is_err());
}
