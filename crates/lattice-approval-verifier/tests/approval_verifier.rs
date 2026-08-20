use lattice_approval_verifier::{
    ApprovalChallenge, ApprovalCommand, ApprovalCommandOutcome, ApprovalCommandReceipt,
    ApprovalDenial, ApprovalEffectClaimIntent, ApprovalIssueRequest, ApprovalNormalClaimExecution,
    ApprovalNormalClaimReceipt, ApprovalNormalClaimRequest, ApprovalPhase, ApprovalRepository,
    ApprovalRepositoryCommand, ApprovalVerifierCheckpoint, ApprovalVerifierError,
    ConsumeNormalApprovalCommand, FakeApprovalVerifier, FakeNormalSigner, FakeProtectedSigner,
    IssueApprovalCommand, RevokeApprovalCommand, SecretMaterial, UntrustedApprovalSnapshot,
    VerifyApprovalCommand, apply_normal_claim_plan, nonce_commitment, plan_normal_claim,
    verify_snapshot, verify_snapshot_against_checkpoint,
};
use lattice_cjson::CanonicalValue;
use lattice_contracts::{
    ApprovalAuthority, ApprovalIdentity, ApprovalLane, ApprovalOrigin, ApprovalStatus,
    ApprovalSubject, ContentDigest, DaemonEpoch, ExternalCostSubject, GuardianRuntimeSubject,
    MemoryCandidateSubject, MemoryKind, MergeSubject, MergeTarget, ProjectId, ProjectSnapshotId,
    ProtectedChangeClass, ProtectedChangeSubject, ProtectedReleaseSubject, ReleaseSubject,
    RuntimeAdmissionMode, RuntimeKind, SubjectBinding, TaskId, UpgradeDelta,
};

fn digest(character: char) -> ContentDigest {
    ContentDigest::from_sha256(character.to_string().repeat(64)).expect("test digest")
}

#[test]
fn repository_contract_binds_exact_normal_effect_intent_without_component_dependency() {
    let _: Option<&mut dyn ApprovalRepository> = None;
    let intent = ApprovalEffectClaimIntent::new("task-transition", "effect-1", digest('e'))
        .expect("bounded effect intent");
    assert_eq!(intent.effect_kind(), "task-transition");
    assert_eq!(intent.effect_id(), "effect-1");
    assert_eq!(intent.effect_digest(), &digest('e'));

    let verifier = verified_normal_fixture("approval-repository-contract");
    let expected_head = verifier
        .state_head("approval-repository-contract")
        .expect("verified normal head");
    let request = ApprovalNormalClaimRequest::new(
        "claim-effect-1",
        "approval-repository-contract",
        expected_head,
        intent,
    )
    .expect("bounded normal claim request");
    let bytes = request
        .canonical_bytes()
        .expect("canonical repository intent");
    assert!(!bytes.is_empty());
    assert_eq!(
        ApprovalNormalClaimRequest::from_canonical_bytes(&bytes)
            .expect("strict normal claim intent parse"),
        request
    );
    assert_eq!(request.command_id(), "claim-effect-1");
    assert_eq!(request.effect().effect_id(), "effect-1");
}

#[test]
fn repository_snapshot_and_checkpoint_bytes_round_trip_through_strict_replay() {
    let verifier = verified_normal_fixture("approval-repository-round-trip");
    let snapshot = verifier.export_snapshot();
    let snapshot_bytes = snapshot
        .canonical_bytes()
        .expect("bounded canonical snapshot bytes");
    let decoded_snapshot = UntrustedApprovalSnapshot::from_canonical_bytes(&snapshot_bytes)
        .expect("strict snapshot bytes");
    assert_eq!(
        decoded_snapshot
            .canonical_bytes()
            .expect("re-encoded snapshot bytes"),
        snapshot_bytes
    );

    let checkpoint = verifier.current_checkpoint().expect("trusted checkpoint");
    let checkpoint_bytes = checkpoint
        .canonical_bytes()
        .expect("bounded canonical checkpoint bytes");
    let decoded_checkpoint = ApprovalVerifierCheckpoint::from_canonical_bytes(&checkpoint_bytes)
        .expect("strict checkpoint bytes");
    assert_eq!(decoded_checkpoint, checkpoint);
    verify_snapshot_against_checkpoint(&decoded_snapshot, &decoded_checkpoint)
        .expect("repository bytes replay against independent checkpoint");
}

#[test]
fn repository_issue_intent_excludes_time_until_database_observation_is_bound() {
    let signer = normal_signer();
    let request = ApprovalRepositoryCommand::Issue(ApprovalIssueRequest {
        command_id: "repository-issue-1".to_owned(),
        expected_head: None,
        identity: normal_identity_with(
            "approval-repository-issue",
            "challenge-repository-issue",
            digest('a'),
        ),
        nonce_id: "nonce-repository-issue".to_owned(),
        nonce_commitment: digest('b'),
        ttl_seconds: 300,
        authenticator_id: signer.authenticator_id().to_owned(),
        key_id: signer.key_id().to_owned(),
        verification_key_commitment: signer.verification_key_commitment().clone(),
        evidence_digest: signer.evidence_digest().clone(),
        review_set_digest: None,
    });
    let intent_bytes = request.canonical_bytes().expect("repository issue intent");
    let intent_text = std::str::from_utf8(&intent_bytes).expect("canonical UTF-8");
    assert!(!intent_text.contains("2026-08-20T00:00:00Z"));
    assert!(!intent_text.contains("2026-08-20T00:05:00Z"));

    let ApprovalCommand::Issue(bound) = request
        .bind_observation("2026-08-20T00:00:00Z", Some("2026-08-20T00:05:00Z"))
        .expect("database observation binding")
    else {
        panic!("issue intent must bind to issue command")
    };
    assert_eq!(bound.runtime, RuntimeKind::Fake);
    assert_eq!(bound.issued_at, "2026-08-20T00:00:00Z");
    assert_eq!(bound.expires_at, "2026-08-20T00:05:00Z");
}

#[test]
fn normal_effect_claim_plans_one_domain_consume_and_protected_lane_has_no_effect_receipt() {
    let normal = verified_normal_fixture("approval-normal-effect-plan");
    let normal_snapshot = normal.export_snapshot();
    let normal_aggregate = verify_snapshot(&normal_snapshot).expect("normal aggregate");
    let normal_head = normal
        .state_head("approval-normal-effect-plan")
        .expect("normal head");
    let request = ApprovalNormalClaimRequest::new(
        "normal-effect-plan-1",
        "approval-normal-effect-plan",
        normal_head,
        ApprovalEffectClaimIntent::new("task-transition", "effect-plan-1", digest('e'))
            .expect("effect"),
    )
    .expect("normal request");
    let plan = plan_normal_claim(
        &normal_aggregate,
        request,
        "2026-07-29T00:03:00Z",
        "daemon-1",
        DaemonEpoch::new(1).expect("epoch"),
        RuntimeAdmissionMode::Active,
    )
    .expect("normal claim plan");
    let ApprovalNormalClaimExecution::Claimed(claimed) = plan.execution() else {
        panic!("normal approval must produce one effect claim")
    };
    assert_eq!(claimed.request().effect().effect_id(), "effect-plan-1");
    assert_eq!(
        claimed.approval_receipt().outcome,
        ApprovalCommandOutcome::Applied
    );
    assert!(
        !claimed
            .approval_receipt()
            .request
            .canonical_bytes()
            .expect("command bytes")
            .is_empty()
    );
    assert!(
        !claimed
            .approval_receipt()
            .canonical_bytes()
            .expect("terminal receipt bytes")
            .is_empty()
    );
    let claimed_bytes = claimed.canonical_bytes().expect("effect receipt bytes");
    let rebuilt = ApprovalNormalClaimReceipt::from_verified_parts(
        claimed.request().clone(),
        claimed.approval_receipt().clone(),
        claimed.observed_at().to_owned(),
        claimed.daemon_instance_id().to_owned(),
        claimed.daemon_epoch(),
        claimed.admission(),
        claimed.claim_digest().clone(),
    )
    .expect("verified durable receipt reconstruction");
    assert_eq!(rebuilt, *claimed);
    assert_eq!(
        rebuilt.canonical_bytes().expect("rebuilt bytes"),
        claimed_bytes
    );
    let normal_after =
        apply_normal_claim_plan(&normal_aggregate, plan).expect("apply normal claim plan");
    assert!(
        normal_after
            .current_authority_at("approval-normal-effect-plan", "2026-07-29T00:03:00Z")
            .expect("currentness")
            .is_none()
    );

    let protected = verified_protected_fixture("approval-protected-effect-plan");
    let protected_aggregate =
        verify_snapshot(&protected.export_snapshot()).expect("protected aggregate");
    let protected_request = ApprovalNormalClaimRequest::new(
        "protected-effect-plan-1",
        "approval-protected-effect-plan",
        protected
            .state_head("approval-protected-effect-plan")
            .expect("protected head"),
        ApprovalEffectClaimIntent::new("release-activation", "effect-protected-1", digest('f'))
            .expect("protected effect intent"),
    )
    .expect("protected request shape");
    let protected_plan = plan_normal_claim(
        &protected_aggregate,
        protected_request,
        "2026-07-29T00:03:00Z",
        "daemon-1",
        DaemonEpoch::new(1).expect("epoch"),
        RuntimeAdmissionMode::Active,
    )
    .expect("protected terminal denial plan");
    let ApprovalNormalClaimExecution::Denied(denied) = protected_plan.execution() else {
        panic!("protected lane must not produce an effect claim")
    };
    assert_eq!(
        denied.outcome,
        ApprovalCommandOutcome::Denied(ApprovalDenial::NormalClaimRequired)
    );
}

fn normal_identity_with(
    approval_id: &str,
    challenge_id: &str,
    task_spec_digest: ContentDigest,
) -> ApprovalIdentity {
    let binding = SubjectBinding::new(
        ProjectId::new("project-alpha").expect("project"),
        ProjectSnapshotId::new("snapshot-1").expect("snapshot"),
        TaskId::new("task-1").expect("task"),
        "1",
        task_spec_digest.clone(),
    )
    .expect("binding");
    ApprovalIdentity::new(
        approval_id,
        challenge_id,
        binding,
        ApprovalSubject::Execution {
            task_spec_hash: task_spec_digest,
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
    .expect("identity")
}

fn normal_identity() -> ApprovalIdentity {
    normal_identity_with("approval-1", "challenge-1", digest('a'))
}

fn normal_signer() -> FakeNormalSigner {
    FakeNormalSigner::new(
        "approver-1",
        "fake-os-authenticator",
        "fake-key-1",
        SecretMaterial::new(b"fake-key-secret-1".to_vec()).expect("signer secret"),
    )
    .expect("signer")
}

fn issue_command(
    command_id: &str,
    identity: ApprovalIdentity,
    nonce_id: &str,
    nonce_commitment: ContentDigest,
    signer: &FakeNormalSigner,
) -> IssueApprovalCommand {
    IssueApprovalCommand {
        command_id: command_id.into(),
        expected_head: None,
        runtime: RuntimeKind::Fake,
        identity,
        nonce_id: nonce_id.into(),
        nonce_commitment,
        issued_at: "2026-07-29T00:00:00Z".into(),
        expires_at: "2026-07-29T00:05:00Z".into(),
        authenticator_id: signer.authenticator_id().into(),
        key_id: signer.key_id().into(),
        verification_key_commitment: signer.verification_key_commitment().clone(),
        evidence_digest: signer.evidence_digest().clone(),
        review_set_digest: None,
    }
}

#[derive(Clone)]
struct BindingFixture {
    project_id: String,
    project_snapshot_id: String,
    task_id: String,
    task_revision: String,
    task_spec_digest: ContentDigest,
}

impl BindingFixture {
    fn baseline() -> Self {
        Self {
            project_id: "project-subject".into(),
            project_snapshot_id: "snapshot-subject".into(),
            task_id: "task-subject".into(),
            task_revision: "1".into(),
            task_spec_digest: digest('a'),
        }
    }

    fn build(&self) -> SubjectBinding {
        SubjectBinding::new(
            ProjectId::new(self.project_id.clone()).expect("project"),
            ProjectSnapshotId::new(self.project_snapshot_id.clone()).expect("snapshot"),
            TaskId::new(self.task_id.clone()).expect("task"),
            self.task_revision.clone(),
            self.task_spec_digest.clone(),
        )
        .expect("binding")
    }
}

#[derive(Clone)]
struct ReleaseFixture {
    activation_id: String,
    saga_id: String,
    release_id: String,
    release_revision: String,
    manifest_digest: ContentDigest,
    source_commit: String,
    source_tree_digest: ContentDigest,
    dependency_lock_digest: ContentDigest,
    binary_digests: Vec<ContentDigest>,
    migration_digests: Vec<ContentDigest>,
    evidence_digest: ContentDigest,
    source_release_id: String,
    source_manifest_digest: ContentDigest,
    source_slot_id: String,
    target_slot_id: String,
    requested_epoch: u64,
    schema_compatible: bool,
    delta: UpgradeDelta,
}

impl ReleaseFixture {
    fn baseline() -> Self {
        Self {
            activation_id: "activation-subject".into(),
            saga_id: "saga-subject".into(),
            release_id: "release-subject".into(),
            release_revision: "1".into(),
            manifest_digest: digest('1'),
            source_commit: "commit-subject".into(),
            source_tree_digest: digest('2'),
            dependency_lock_digest: digest('3'),
            binary_digests: vec![digest('4')],
            migration_digests: vec![digest('5')],
            evidence_digest: digest('6'),
            source_release_id: "release-source".into(),
            source_manifest_digest: digest('7'),
            source_slot_id: "slot-source".into(),
            target_slot_id: "slot-target".into(),
            requested_epoch: 7,
            schema_compatible: true,
            delta: UpgradeDelta::default(),
        }
    }

    fn build(&self) -> ReleaseSubject {
        ReleaseSubject::new(
            self.activation_id.clone(),
            self.saga_id.clone(),
            self.release_id.clone(),
            self.release_revision.clone(),
            self.manifest_digest.clone(),
            self.source_commit.clone(),
            self.source_tree_digest.clone(),
            self.dependency_lock_digest.clone(),
            self.binary_digests.clone(),
            self.migration_digests.clone(),
            self.evidence_digest.clone(),
            self.source_release_id.clone(),
            self.source_manifest_digest.clone(),
            self.source_slot_id.clone(),
            self.target_slot_id.clone(),
            DaemonEpoch::new(self.requested_epoch).expect("requested epoch"),
            self.schema_compatible,
            self.delta,
        )
        .expect("release subject")
    }
}

fn protected_signer() -> FakeProtectedSigner {
    FakeProtectedSigner::new(
        "guardian-1",
        "fake-guardian-authenticator",
        "fake-guardian-key",
        SecretMaterial::new(b"guardian-root".to_vec()).expect("secret"),
        "daemon-1",
        7,
    )
    .expect("protected signer")
}

fn guardian_subject_with(
    guardian_id: &str,
    trust_root_digest: ContentDigest,
    daemon_instance_id: &str,
    observed_epoch: u64,
) -> GuardianRuntimeSubject {
    GuardianRuntimeSubject::new(
        guardian_id,
        trust_root_digest,
        daemon_instance_id,
        DaemonEpoch::new(observed_epoch).expect("observed epoch"),
    )
    .expect("guardian subject")
}

fn baseline_guardian_subject() -> GuardianRuntimeSubject {
    let signer = protected_signer();
    guardian_subject_with(
        signer.guardian_id(),
        signer.trust_root_digest().clone(),
        signer.daemon_instance_id(),
        signer.observed_epoch(),
    )
}

fn identity_for_subject(binding: SubjectBinding, subject: ApprovalSubject) -> ApprovalIdentity {
    let (approver_id, authority, origin, lane, channel_id) = match &subject {
        ApprovalSubject::ProtectedRelease(protected) => (
            protected.guardian().guardian_id().to_owned(),
            ApprovalAuthority::ProtectedGuardian,
            ApprovalOrigin::GuardianTrustRoot,
            ApprovalLane::Protected,
            "channel-protected-subject".to_owned(),
        ),
        _ => (
            "approver-1".to_owned(),
            ApprovalAuthority::ResponsibleUser,
            ApprovalOrigin::OsAuthenticatedUser,
            ApprovalLane::Normal,
            "channel-normal-subject".to_owned(),
        ),
    };
    ApprovalIdentity::new(
        "approval-subject",
        "challenge-subject",
        binding,
        subject,
        "requester-subject",
        approver_id,
        authority,
        origin,
        lane,
        channel_id,
        "session-subject",
    )
    .expect("approval identity")
}

fn subject_digest_for(binding: SubjectBinding, subject: ApprovalSubject) -> ContentDigest {
    let protected = matches!(&subject, ApprovalSubject::ProtectedRelease(_));
    let identity = identity_for_subject(binding, subject);
    let mut verifier = FakeApprovalVerifier::new();
    let command = if protected {
        let signer = protected_signer();
        IssueApprovalCommand {
            command_id: "issue-subject".into(),
            expected_head: None,
            runtime: RuntimeKind::Fake,
            identity,
            nonce_id: "nonce-subject".into(),
            nonce_commitment: nonce_commitment(
                &SecretMaterial::new(b"subject-nonce".to_vec()).expect("nonce"),
            )
            .expect("commitment"),
            issued_at: "2026-07-29T00:00:00Z".into(),
            expires_at: "2026-07-29T00:05:00Z".into(),
            authenticator_id: signer.authenticator_id().into(),
            key_id: signer.key_id().into(),
            verification_key_commitment: signer.trust_root_digest().clone(),
            evidence_digest: signer.evidence_digest().clone(),
            review_set_digest: Some(digest('8')),
        }
    } else {
        let signer = normal_signer();
        issue_command(
            "issue-subject",
            identity,
            "nonce-subject",
            nonce_commitment(&SecretMaterial::new(b"subject-nonce".to_vec()).expect("nonce"))
                .expect("commitment"),
            &signer,
        )
    };
    verifier
        .issue(command)
        .expect("issue subject")
        .challenge
        .expect("challenge")
        .subject_digest()
        .clone()
}

fn assert_subject_digest_matrix(
    baseline: (SubjectBinding, ApprovalSubject),
    variants: Vec<(&'static str, SubjectBinding, ApprovalSubject)>,
) {
    let baseline_digest = subject_digest_for(baseline.0, baseline.1);
    let mut seen = vec![baseline_digest.clone()];
    for (field, binding, subject) in variants {
        let candidate = subject_digest_for(binding, subject);
        assert_ne!(
            candidate, baseline_digest,
            "{field} substitution retained the baseline subject digest"
        );
        assert!(
            !seen.contains(&candidate),
            "{field} substitution collided with another matrix digest"
        );
        seen.push(candidate);
    }
}

fn external_cost_with(
    amount: &str,
    currency: &str,
    provider_id: &str,
    quote: char,
    pricing: char,
) -> ExternalCostSubject {
    ExternalCostSubject::new(
        amount,
        currency,
        provider_id,
        digest(quote),
        digest(pricing),
    )
    .expect("external cost subject")
}

fn merge_subject_with(
    target: MergeTarget,
    reviewed_commit: &str,
    target_head: &str,
    diff: char,
) -> MergeSubject {
    MergeSubject::new(target, reviewed_commit, target_head, digest(diff)).expect("merge subject")
}

fn release_field_variants() -> Vec<(&'static str, ReleaseSubject)> {
    let baseline = ReleaseFixture::baseline();
    macro_rules! changed {
        ($field:ident, $value:expr) => {{
            let mut candidate = baseline.clone();
            candidate.$field = $value;
            candidate.build()
        }};
    }
    let mut variants = vec![
        (
            "activation_id",
            changed!(activation_id, "activation-other".into()),
        ),
        ("saga_id", changed!(saga_id, "saga-other".into())),
        ("release_id", changed!(release_id, "release-other".into())),
        ("release_revision", changed!(release_revision, "2".into())),
        ("manifest_digest", changed!(manifest_digest, digest('9'))),
        (
            "source_commit",
            changed!(source_commit, "commit-other".into()),
        ),
        (
            "source_tree_digest",
            changed!(source_tree_digest, digest('a')),
        ),
        (
            "dependency_lock_digest",
            changed!(dependency_lock_digest, digest('b')),
        ),
        (
            "binary_digests",
            changed!(binary_digests, vec![digest('c')]),
        ),
        (
            "migration_digests",
            changed!(migration_digests, vec![digest('d')]),
        ),
        ("evidence_digest", changed!(evidence_digest, digest('e'))),
        (
            "source_release_id",
            changed!(source_release_id, "release-source-other".into()),
        ),
        (
            "source_manifest_digest",
            changed!(source_manifest_digest, digest('f')),
        ),
        (
            "source_slot_id",
            changed!(source_slot_id, "slot-source-other".into()),
        ),
        (
            "target_slot_id",
            changed!(target_slot_id, "slot-target-other".into()),
        ),
        ("requested_epoch", changed!(requested_epoch, 8)),
        ("schema_compatible", changed!(schema_compatible, false)),
    ];
    let deltas = [
        UpgradeDelta::new(true, false, false, false, false, false, false, false),
        UpgradeDelta::new(false, true, false, false, false, false, false, false),
        UpgradeDelta::new(false, false, true, false, false, false, false, false),
        UpgradeDelta::new(false, false, false, true, false, false, false, false),
        UpgradeDelta::new(false, false, false, false, true, false, false, false),
        UpgradeDelta::new(false, false, false, false, false, true, false, false),
        UpgradeDelta::new(false, false, false, false, false, false, true, false),
        UpgradeDelta::new(false, false, false, false, false, false, false, true),
    ];
    let labels = [
        "delta_schema",
        "delta_policy",
        "delta_constitution",
        "delta_supervisor",
        "delta_credentials",
        "delta_public_exposure",
        "delta_destructive",
        "delta_capability",
    ];
    for (label, delta) in labels.into_iter().zip(deltas) {
        variants.push((label, changed!(delta, delta)));
    }
    variants
}

#[test]
fn approval_subject_hash_binds_every_outer_task_and_project_field() {
    let baseline = BindingFixture::baseline();
    let subject = ApprovalSubject::Merge(merge_subject_with(
        MergeTarget::FeatureBranch("refs/heads/feature-a".into()),
        "commit-a",
        "head-a",
        'b',
    ));
    macro_rules! changed_binding {
        ($field:ident, $value:expr) => {{
            let mut candidate = baseline.clone();
            candidate.$field = $value;
            candidate.build()
        }};
    }
    let variants = vec![
        (
            "project_id",
            changed_binding!(project_id, "project-other".into()),
            subject.clone(),
        ),
        (
            "project_snapshot_id",
            changed_binding!(project_snapshot_id, "snapshot-other".into()),
            subject.clone(),
        ),
        (
            "task_id",
            changed_binding!(task_id, "task-other".into()),
            subject.clone(),
        ),
        (
            "task_revision",
            changed_binding!(task_revision, "2".into()),
            subject.clone(),
        ),
        (
            "task_spec_digest",
            changed_binding!(task_spec_digest, digest('c')),
            subject.clone(),
        ),
    ];
    assert_subject_digest_matrix((baseline.build(), subject), variants);
}

#[test]
fn execution_subject_hash_binds_task_spec_and_every_cost_field() {
    let binding = BindingFixture::baseline().build();
    let baseline_cost = external_cost_with("1.25", "USD", "provider-a", 'b', 'c');
    let baseline_subject = ApprovalSubject::Execution {
        task_spec_hash: binding.task_spec_digest().clone(),
        external_cost: Some(baseline_cost),
    };
    let variants = vec![
        (
            "external_cost_presence",
            binding.clone(),
            ApprovalSubject::Execution {
                task_spec_hash: binding.task_spec_digest().clone(),
                external_cost: None,
            },
        ),
        (
            "amount",
            binding.clone(),
            ApprovalSubject::Execution {
                task_spec_hash: binding.task_spec_digest().clone(),
                external_cost: Some(external_cost_with("1.26", "USD", "provider-a", 'b', 'c')),
            },
        ),
        (
            "currency",
            binding.clone(),
            ApprovalSubject::Execution {
                task_spec_hash: binding.task_spec_digest().clone(),
                external_cost: Some(external_cost_with("1.25", "EUR", "provider-a", 'b', 'c')),
            },
        ),
        (
            "provider_id",
            binding.clone(),
            ApprovalSubject::Execution {
                task_spec_hash: binding.task_spec_digest().clone(),
                external_cost: Some(external_cost_with("1.25", "USD", "provider-b", 'b', 'c')),
            },
        ),
        (
            "quote_digest",
            binding.clone(),
            ApprovalSubject::Execution {
                task_spec_hash: binding.task_spec_digest().clone(),
                external_cost: Some(external_cost_with("1.25", "USD", "provider-a", 'd', 'c')),
            },
        ),
        (
            "pricing_digest",
            binding.clone(),
            ApprovalSubject::Execution {
                task_spec_hash: binding.task_spec_digest().clone(),
                external_cost: Some(external_cost_with("1.25", "USD", "provider-a", 'b', 'e')),
            },
        ),
    ];
    assert_subject_digest_matrix((binding, baseline_subject), variants);
}

#[test]
fn merge_subject_hash_binds_target_class_ref_commit_head_and_diff() {
    let binding = BindingFixture::baseline().build();
    let baseline = ApprovalSubject::Merge(merge_subject_with(
        MergeTarget::FeatureBranch("refs/heads/feature-a".into()),
        "commit-a",
        "head-a",
        'b',
    ));
    let variants = vec![
        (
            "target_class",
            binding.clone(),
            ApprovalSubject::Merge(merge_subject_with(
                MergeTarget::PrimaryBranch("refs/heads/feature-a".into()),
                "commit-a",
                "head-a",
                'b',
            )),
        ),
        (
            "target_ref",
            binding.clone(),
            ApprovalSubject::Merge(merge_subject_with(
                MergeTarget::FeatureBranch("refs/heads/feature-b".into()),
                "commit-a",
                "head-a",
                'b',
            )),
        ),
        (
            "reviewed_commit",
            binding.clone(),
            ApprovalSubject::Merge(merge_subject_with(
                MergeTarget::FeatureBranch("refs/heads/feature-a".into()),
                "commit-b",
                "head-a",
                'b',
            )),
        ),
        (
            "target_head",
            binding.clone(),
            ApprovalSubject::Merge(merge_subject_with(
                MergeTarget::FeatureBranch("refs/heads/feature-a".into()),
                "commit-a",
                "head-b",
                'b',
            )),
        ),
        (
            "diff_digest",
            binding.clone(),
            ApprovalSubject::Merge(merge_subject_with(
                MergeTarget::FeatureBranch("refs/heads/feature-a".into()),
                "commit-a",
                "head-a",
                'c',
            )),
        ),
    ];
    assert_subject_digest_matrix((binding, baseline), variants);
}

#[test]
fn preference_and_protected_change_hashes_bind_their_complete_typed_fields() {
    let binding = BindingFixture::baseline().build();
    let preference = ApprovalSubject::Preference(
        MemoryCandidateSubject::new(binding.clone(), digest('b'), MemoryKind::Preference)
            .expect("preference"),
    );
    let preference_variants = vec![
        (
            "candidate_digest",
            binding.clone(),
            ApprovalSubject::Preference(
                MemoryCandidateSubject::new(binding.clone(), digest('c'), MemoryKind::Preference)
                    .expect("preference"),
            ),
        ),
        (
            "memory_kind",
            binding.clone(),
            ApprovalSubject::Preference(
                MemoryCandidateSubject::new(binding.clone(), digest('b'), MemoryKind::Fact)
                    .expect("candidate"),
            ),
        ),
    ];
    assert_subject_digest_matrix((binding.clone(), preference), preference_variants);

    let protected = ApprovalSubject::ProtectedChange(
        ProtectedChangeSubject::new(ProtectedChangeClass::Policy, digest('d'))
            .expect("protected change"),
    );
    let protected_variants = vec![
        (
            "protected_class",
            binding.clone(),
            ApprovalSubject::ProtectedChange(
                ProtectedChangeSubject::new(ProtectedChangeClass::Constitution, digest('d'))
                    .expect("protected change"),
            ),
        ),
        (
            "operation_digest",
            binding.clone(),
            ApprovalSubject::ProtectedChange(
                ProtectedChangeSubject::new(ProtectedChangeClass::Policy, digest('e'))
                    .expect("protected change"),
            ),
        ),
    ];
    assert_subject_digest_matrix((binding, protected), protected_variants);
}

#[test]
fn protected_release_hash_binds_every_release_delta_and_guardian_field() {
    let binding = BindingFixture::baseline().build();
    let release = ReleaseFixture::baseline().build();
    let guardian = baseline_guardian_subject();
    let baseline = ApprovalSubject::ProtectedRelease(Box::new(ProtectedReleaseSubject::new(
        release.clone(),
        guardian.clone(),
    )));
    let mut variants = release_field_variants()
        .into_iter()
        .map(|(label, changed_release)| {
            (
                label,
                binding.clone(),
                ApprovalSubject::ProtectedRelease(Box::new(ProtectedReleaseSubject::new(
                    changed_release,
                    guardian.clone(),
                ))),
            )
        })
        .collect::<Vec<_>>();
    let guardian_variants = [
        (
            "guardian_id",
            guardian_subject_with(
                "guardian-other",
                guardian.trust_root_digest().clone(),
                guardian.daemon_instance_id(),
                guardian.observed_epoch().get(),
            ),
        ),
        (
            "guardian_trust_root",
            guardian_subject_with(
                guardian.guardian_id(),
                digest('b'),
                guardian.daemon_instance_id(),
                guardian.observed_epoch().get(),
            ),
        ),
        (
            "guardian_daemon",
            guardian_subject_with(
                guardian.guardian_id(),
                guardian.trust_root_digest().clone(),
                "daemon-other",
                guardian.observed_epoch().get(),
            ),
        ),
        (
            "guardian_epoch",
            guardian_subject_with(
                guardian.guardian_id(),
                guardian.trust_root_digest().clone(),
                guardian.daemon_instance_id(),
                8,
            ),
        ),
    ];
    variants.extend(
        guardian_variants
            .into_iter()
            .map(|(label, changed_guardian)| {
                (
                    label,
                    binding.clone(),
                    ApprovalSubject::ProtectedRelease(Box::new(ProtectedReleaseSubject::new(
                        release.clone(),
                        changed_guardian,
                    ))),
                )
            }),
    );
    assert_subject_digest_matrix((binding, baseline), variants);
}

#[test]
fn issue_verify_and_query_one_exact_normal_approval() {
    let nonce = SecretMaterial::new(b"nonce-secret-1".to_vec()).expect("nonce");
    let signer = normal_signer();
    let mut verifier = FakeApprovalVerifier::new();

    let issued = verifier
        .issue(issue_command(
            "command-issue-1",
            normal_identity(),
            "nonce-1",
            nonce_commitment(&nonce).expect("commitment"),
            &signer,
        ))
        .expect("issue command");
    let challenge = issued.challenge.as_ref().expect("issued challenge");
    let proof = signer.sign(challenge).expect("fake proof");

    let verification_receipt = verifier
        .verify(VerifyApprovalCommand {
            command_id: "command-verify-1".into(),
            approval_id: "approval-1".into(),
            expected_head: issued.after.clone().expect("challenge head"),
            observed_at: "2026-07-29T00:01:00Z".into(),
            proof,
        })
        .expect("verify command");
    let authority = verification_receipt
        .authority_receipt
        .as_ref()
        .expect("authority receipt");

    assert_eq!(authority.status(), ApprovalStatus::Available);
    assert_eq!(
        verifier
            .current_head_at("approval-1", "2026-07-29T00:02:00Z")
            .expect("current lookup"),
        Some(authority.head())
    );
}

#[test]
fn challenge_proof_and_authority_receipt_digests_match_fixed_golden_values() {
    let signer = normal_signer();
    let nonce = SecretMaterial::new(b"golden-nonce".to_vec()).expect("nonce");
    let mut verifier = FakeApprovalVerifier::new();
    let issued = verifier
        .issue(issue_command(
            "command-golden-issue",
            normal_identity_with("approval-golden", "challenge-golden", digest('a')),
            "nonce-golden",
            nonce_commitment(&nonce).expect("commitment"),
            &signer,
        ))
        .expect("issue");
    let challenge = issued.challenge.as_ref().expect("challenge");
    let challenge_digest = challenge.challenge_digest().as_str().to_owned();
    let proof = signer.sign(challenge).expect("proof");
    let proof_digest = proof.proof_digest().as_str().to_owned();
    let command_receipt = verifier
        .verify(VerifyApprovalCommand {
            command_id: "command-golden-verify".into(),
            approval_id: "approval-golden".into(),
            expected_head: issued.after.expect("challenge head"),
            observed_at: "2026-07-29T00:01:00Z".into(),
            proof,
        })
        .expect("verify");
    let receipt_digest = command_receipt
        .authority_receipt
        .as_ref()
        .expect("authority receipt")
        .receipt_digest()
        .as_str()
        .to_owned();

    assert_eq!(
        challenge_digest,
        "7d3251a5bac79061ec0f783c14f934a237b37ce6e5a022a54fc92c9f72fbdcc8"
    );
    assert_eq!(
        proof_digest,
        "ef218f0926363b855dbd3ef882b76877cc0a60328b93fdaa774aea9bad47b346"
    );
    assert_eq!(
        receipt_digest,
        "a43d9e6451589fc65aed8c22df8c8b3d8c16dcc6077cd8dfd23528d5a59a111f"
    );
}

#[test]
fn raw_secret_and_fake_signer_debug_are_redacted() {
    let secret = SecretMaterial::new(b"nonce-that-must-not-leak".to_vec()).expect("secret");
    let debug = format!("{secret:?}");
    assert!(debug.contains("[REDACTED]"));
    assert!(!debug.contains("nonce-that-must-not-leak"));

    let signer = normal_signer();
    let signer_debug = format!("{signer:?}");
    assert!(signer_debug.contains("[REDACTED]"));
    assert!(!signer_debug.contains("fake-key-secret-1"));
}

#[test]
fn nonce_commitment_is_global_and_denial_does_not_rebind_it() {
    let signer = normal_signer();
    let nonce = nonce_commitment(&SecretMaterial::new(b"global-nonce".to_vec()).expect("nonce"))
        .expect("commitment");
    let mut verifier = FakeApprovalVerifier::new();
    let first = verifier
        .issue(issue_command(
            "issue-global-1",
            normal_identity_with("approval-global-1", "challenge-global-1", digest('a')),
            "nonce-global-1",
            nonce.clone(),
            &signer,
        ))
        .expect("first issue");
    assert_eq!(first.outcome, ApprovalCommandOutcome::Applied);

    let second = verifier
        .issue(issue_command(
            "issue-global-2",
            normal_identity_with("approval-global-2", "challenge-global-2", digest('b')),
            "nonce-global-2",
            nonce,
            &signer,
        ))
        .expect("second issue is terminal");
    assert_eq!(
        second.outcome,
        ApprovalCommandOutcome::Denied(ApprovalDenial::NonceAlreadyBound)
    );
    assert!(verifier.state_head("approval-global-2").is_none());
    assert_eq!(second.previous_receipt_digest, Some(first.receipt_digest));
}

#[test]
fn exact_retry_precedes_stale_state_and_changed_command_id_content_rejects() {
    let signer = normal_signer();
    let nonce = nonce_commitment(&SecretMaterial::new(b"retry-nonce".to_vec()).expect("nonce"))
        .expect("commitment");
    let issue = issue_command(
        "issue-retry",
        normal_identity_with("approval-retry", "challenge-retry", digest('a')),
        "nonce-retry",
        nonce,
        &signer,
    );
    let mut verifier = FakeApprovalVerifier::new();
    let issued = verifier.issue(issue.clone()).expect("issue");
    let proof = signer
        .sign(issued.challenge.as_ref().expect("challenge"))
        .expect("proof");
    verifier
        .verify(VerifyApprovalCommand {
            command_id: "verify-retry".into(),
            approval_id: "approval-retry".into(),
            expected_head: issued.after.clone().expect("head"),
            observed_at: "2026-07-29T00:01:00Z".into(),
            proof,
        })
        .expect("verify");

    let retry = verifier.issue(issue.clone()).expect("exact stale retry");
    assert_eq!(retry, issued);
    assert_eq!(verifier.command_receipts().len(), 2);

    let mut changed = issue;
    changed.expires_at = "2026-07-29T00:04:00Z".into();
    assert_eq!(
        verifier.issue(changed),
        Err(ApprovalVerifierError::CommandIdReuse)
    );
    assert_eq!(verifier.command_receipts().len(), 2);
}

#[test]
fn denied_command_retry_is_exact_and_never_partially_mutates_authority() {
    let signer = normal_signer();
    let mut verifier = FakeApprovalVerifier::new();
    let issued = verifier
        .issue(issue_command(
            "issue-denied-retry",
            normal_identity_with(
                "approval-denied-retry",
                "challenge-denied-retry",
                digest('a'),
            ),
            "nonce-denied-retry",
            nonce_commitment(&SecretMaterial::new(b"denied-retry-nonce".to_vec()).expect("nonce"))
                .expect("commitment"),
            &signer,
        ))
        .expect("issue");
    let proof = signer
        .sign(issued.challenge.as_ref().expect("challenge"))
        .expect("proof");
    let denied_command = VerifyApprovalCommand {
        command_id: "verify-denied-retry".into(),
        approval_id: "approval-denied-retry".into(),
        expected_head: issued.after.clone().expect("head"),
        observed_at: "2026-07-28T23:59:59Z".into(),
        proof: proof.clone(),
    };
    let denied = verifier
        .verify(denied_command.clone())
        .expect("terminal denial");
    assert_eq!(
        denied.outcome,
        ApprovalCommandOutcome::Denied(ApprovalDenial::NotYetValid)
    );
    assert!(denied.authority_receipt.is_none());
    assert_eq!(
        verifier
            .state_head("approval-denied-retry")
            .expect("challenge retained")
            .phase(),
        ApprovalPhase::Challenged
    );
    assert!(
        verifier
            .current_head_at("approval-denied-retry", "2026-07-29T00:01:00Z")
            .expect("lookup")
            .is_none()
    );

    verifier
        .verify(VerifyApprovalCommand {
            command_id: "verify-denied-retry-valid".into(),
            approval_id: "approval-denied-retry".into(),
            expected_head: issued.after.expect("head"),
            observed_at: "2026-07-29T00:01:00Z".into(),
            proof,
        })
        .expect("valid verification");
    let verified_head = verifier
        .state_head("approval-denied-retry")
        .expect("verified head");
    let history_len = verifier.command_receipts().len();

    assert_eq!(
        verifier
            .verify(denied_command.clone())
            .expect("exact denied retry"),
        denied
    );
    assert_eq!(verifier.command_receipts().len(), history_len);
    assert_eq!(
        verifier.state_head("approval-denied-retry"),
        Some(verified_head.clone())
    );

    let mut changed = denied_command;
    changed.observed_at = "2026-07-29T00:01:00Z".into();
    assert_eq!(
        verifier.verify(changed),
        Err(ApprovalVerifierError::CommandIdReuse)
    );
    assert_eq!(verifier.command_receipts().len(), history_len);
    assert_eq!(
        verifier.state_head("approval-denied-retry"),
        Some(verified_head)
    );
}

#[test]
fn proof_substitution_denies_without_partial_authority_mutation() {
    let signer = normal_signer();
    let mut verifier = FakeApprovalVerifier::new();
    let issued = verifier
        .issue(issue_command(
            "issue-proof",
            normal_identity_with("approval-proof", "challenge-proof", digest('a')),
            "nonce-proof",
            nonce_commitment(&SecretMaterial::new(b"proof-nonce".to_vec()).expect("nonce"))
                .expect("commitment"),
            &signer,
        ))
        .expect("issue");
    let challenge = issued.challenge.as_ref().expect("challenge");
    let correct_proof = signer.sign(challenge).expect("right proof");
    let substitute = verifier
        .issue(issue_command(
            "issue-proof-substitute",
            normal_identity_with(
                "approval-proof-substitute",
                "challenge-proof-substitute",
                digest('a'),
            ),
            "nonce-proof-substitute",
            nonce_commitment(
                &SecretMaterial::new(b"proof-nonce-substitute".to_vec()).expect("nonce"),
            )
            .expect("commitment"),
            &signer,
        ))
        .expect("substitute issue");
    let substitute_proof = signer
        .sign(substitute.challenge.as_ref().expect("substitute challenge"))
        .expect("substitute proof");
    let denied = verifier
        .verify(VerifyApprovalCommand {
            command_id: "verify-proof-wrong".into(),
            approval_id: "approval-proof".into(),
            expected_head: issued.after.clone().expect("head"),
            observed_at: "2026-07-29T00:01:00Z".into(),
            proof: substitute_proof,
        })
        .expect("terminal denial");
    assert_eq!(
        denied.outcome,
        ApprovalCommandOutcome::Denied(ApprovalDenial::ProofMismatch)
    );
    assert_eq!(
        verifier
            .state_head("approval-proof")
            .expect("retained challenge")
            .phase(),
        ApprovalPhase::Challenged
    );
    assert!(
        verifier
            .current_head_at("approval-proof", "2026-07-29T00:01:00Z")
            .expect("lookup")
            .is_none()
    );

    let verification_receipt = verifier
        .verify(VerifyApprovalCommand {
            command_id: "verify-proof-right".into(),
            approval_id: "approval-proof".into(),
            expected_head: issued.after.expect("head"),
            observed_at: "2026-07-29T00:01:00Z".into(),
            proof: correct_proof,
        })
        .expect("verify");
    assert_eq!(
        verification_receipt.outcome,
        ApprovalCommandOutcome::Applied
    );
}

#[test]
fn time_window_is_inclusive_at_issue_and_exclusive_at_expiry() {
    let signer = normal_signer();
    let mut verifier = FakeApprovalVerifier::new();
    let issued = verifier
        .issue(issue_command(
            "issue-time",
            normal_identity_with("approval-time", "challenge-time", digest('a')),
            "nonce-time",
            nonce_commitment(&SecretMaterial::new(b"time-nonce".to_vec()).expect("nonce"))
                .expect("commitment"),
            &signer,
        ))
        .expect("issue");
    let proof = signer
        .sign(issued.challenge.as_ref().expect("challenge"))
        .expect("proof");

    let before = verifier
        .verify(VerifyApprovalCommand {
            command_id: "verify-before".into(),
            approval_id: "approval-time".into(),
            expected_head: issued.after.clone().expect("head"),
            observed_at: "2026-07-28T23:59:59Z".into(),
            proof: proof.clone(),
        })
        .expect("terminal");
    assert_eq!(
        before.outcome,
        ApprovalCommandOutcome::Denied(ApprovalDenial::NotYetValid)
    );
    let expired = verifier
        .verify(VerifyApprovalCommand {
            command_id: "verify-expiry".into(),
            approval_id: "approval-time".into(),
            expected_head: issued.after.clone().expect("head"),
            observed_at: "2026-07-29T00:05:00Z".into(),
            proof: proof.clone(),
        })
        .expect("terminal");
    assert_eq!(
        expired.outcome,
        ApprovalCommandOutcome::Denied(ApprovalDenial::Expired)
    );
    let valid = verifier
        .verify(VerifyApprovalCommand {
            command_id: "verify-at-issue".into(),
            approval_id: "approval-time".into(),
            expected_head: issued.after.expect("head"),
            observed_at: "2026-07-29T00:00:00Z".into(),
            proof,
        })
        .expect("valid");
    assert_eq!(valid.outcome, ApprovalCommandOutcome::Applied);
    assert!(
        verifier
            .current_head_at("approval-time", "2026-07-29T00:05:00Z")
            .expect("lookup")
            .is_none()
    );
}

#[test]
fn malformed_or_reversed_time_rejects_before_state_mutation() {
    let signer = normal_signer();
    let mut command = issue_command(
        "issue-bad-time",
        normal_identity_with("approval-bad-time", "challenge-bad-time", digest('a')),
        "nonce-bad-time",
        nonce_commitment(&SecretMaterial::new(b"bad-time-nonce".to_vec()).expect("nonce"))
            .expect("commitment"),
        &signer,
    );
    command.expires_at = command.issued_at.clone();
    let mut verifier = FakeApprovalVerifier::new();
    assert_eq!(
        verifier.issue(command),
        Err(ApprovalVerifierError::InvalidExpiry)
    );
    assert!(verifier.command_receipts().is_empty());
    assert!(verifier.state_head("approval-bad-time").is_none());
}

#[test]
fn normal_claim_invalidates_independent_current_head() {
    let signer = normal_signer();
    let mut verifier = FakeApprovalVerifier::new();
    let issued = verifier
        .issue(issue_command(
            "issue-claim",
            normal_identity_with("approval-claim", "challenge-claim", digest('a')),
            "nonce-claim",
            nonce_commitment(&SecretMaterial::new(b"claim-nonce".to_vec()).expect("nonce"))
                .expect("commitment"),
            &signer,
        ))
        .expect("issue");
    let verification_receipt = verifier
        .verify(VerifyApprovalCommand {
            command_id: "verify-claim".into(),
            approval_id: "approval-claim".into(),
            expected_head: issued.after.clone().expect("challenge head"),
            observed_at: "2026-07-29T00:01:00Z".into(),
            proof: signer
                .sign(issued.challenge.as_ref().expect("challenge"))
                .expect("proof"),
        })
        .expect("verify");
    assert!(
        verifier
            .current_head_at("approval-claim", "2026-07-29T00:02:00Z")
            .expect("lookup")
            .is_some()
    );
    let claimed = verifier
        .consume_normal(ConsumeNormalApprovalCommand {
            command_id: "consume-claim".into(),
            approval_id: "approval-claim".into(),
            expected_head: verification_receipt.after.expect("verified head"),
            observed_at: "2026-07-29T00:02:00Z".into(),
            claim_digest: digest('c'),
        })
        .expect("consume");
    assert_eq!(claimed.outcome, ApprovalCommandOutcome::Applied);
    assert_eq!(
        claimed.after.expect("claimed head").phase(),
        ApprovalPhase::ClaimedNormal
    );
    assert!(
        verifier
            .current_head_at("approval-claim", "2026-07-29T00:02:01Z")
            .expect("lookup")
            .is_none()
    );
}

fn protected_release_identity_with(
    signer: &FakeProtectedSigner,
    approval_id: &str,
    challenge_id: &str,
) -> ApprovalIdentity {
    let binding = SubjectBinding::new(
        ProjectId::new("lattice-system").expect("project"),
        ProjectSnapshotId::new("snapshot-protected").expect("snapshot"),
        TaskId::new("task-release").expect("task"),
        "1",
        digest('d'),
    )
    .expect("binding");
    let release = ReleaseSubject::new(
        "activation-1",
        "saga-1",
        "release-1",
        "1",
        digest('e'),
        "commit-1",
        digest('f'),
        digest('1'),
        vec![digest('2')],
        vec![digest('3')],
        digest('4'),
        "source-release-1",
        digest('5'),
        "slot-a",
        "slot-b",
        DaemonEpoch::new(signer.observed_epoch()).expect("epoch"),
        true,
        UpgradeDelta::new(false, true, false, true, false, false, false, true),
    )
    .expect("release");
    let guardian = GuardianRuntimeSubject::new(
        signer.guardian_id(),
        signer.trust_root_digest().clone(),
        signer.daemon_instance_id(),
        DaemonEpoch::new(signer.observed_epoch()).expect("epoch"),
    )
    .expect("guardian");
    ApprovalIdentity::new(
        approval_id,
        challenge_id,
        binding,
        ApprovalSubject::ProtectedRelease(Box::new(ProtectedReleaseSubject::new(
            release, guardian,
        ))),
        "requester-protected",
        signer.guardian_id(),
        ApprovalAuthority::ProtectedGuardian,
        ApprovalOrigin::GuardianTrustRoot,
        ApprovalLane::Protected,
        "channel-protected",
        "session-protected",
    )
    .expect("identity")
}

fn protected_release_identity(signer: &FakeProtectedSigner) -> ApprovalIdentity {
    protected_release_identity_with(signer, "approval-protected", "challenge-protected")
}

#[test]
fn protected_release_requires_exact_guardian_and_has_no_consumable_lane() {
    let signer = FakeProtectedSigner::new(
        "guardian-1",
        "fake-guardian-authenticator",
        "fake-guardian-key",
        SecretMaterial::new(b"guardian-root".to_vec()).expect("secret"),
        "daemon-1",
        7,
    )
    .expect("signer");
    let identity = protected_release_identity(&signer);
    let mut verifier = FakeApprovalVerifier::new();
    let issued = verifier
        .issue(IssueApprovalCommand {
            command_id: "issue-protected".into(),
            expected_head: None,
            runtime: RuntimeKind::Fake,
            identity,
            nonce_id: "nonce-protected".into(),
            nonce_commitment: nonce_commitment(
                &SecretMaterial::new(b"protected-nonce".to_vec()).expect("nonce"),
            )
            .expect("commitment"),
            issued_at: "2026-07-29T00:00:00Z".into(),
            expires_at: "2026-07-29T00:05:00Z".into(),
            authenticator_id: signer.authenticator_id().into(),
            key_id: signer.key_id().into(),
            verification_key_commitment: signer.trust_root_digest().clone(),
            evidence_digest: signer.evidence_digest().clone(),
            review_set_digest: Some(digest('6')),
        })
        .expect("issue");
    let verification_receipt = verifier
        .verify(VerifyApprovalCommand {
            command_id: "verify-protected".into(),
            approval_id: "approval-protected".into(),
            expected_head: issued.after.expect("head"),
            observed_at: "2026-07-29T00:01:00Z".into(),
            proof: signer
                .sign(issued.challenge.as_ref().expect("challenge"))
                .expect("proof"),
        })
        .expect("verify");
    assert_eq!(
        verification_receipt
            .authority_receipt
            .as_ref()
            .expect("authority")
            .status(),
        ApprovalStatus::ProtectedPendingClaim
    );
    assert!(matches!(
        &verification_receipt.request,
        ApprovalCommand::Verify(VerifyApprovalCommand { .. })
    ));

    let denied = verifier
        .consume_normal(ConsumeNormalApprovalCommand {
            command_id: "consume-protected".into(),
            approval_id: "approval-protected".into(),
            expected_head: verification_receipt.after.expect("head"),
            observed_at: "2026-07-29T00:02:00Z".into(),
            claim_digest: digest('7'),
        })
        .expect("terminal denial");
    assert_eq!(
        denied.outcome,
        ApprovalCommandOutcome::Denied(ApprovalDenial::NormalClaimRequired)
    );
    assert!(
        verifier
            .current_head_at("approval-protected", "2026-07-29T00:02:01Z")
            .expect("lookup")
            .is_some()
    );
}

fn issue_protected_challenge(
    signer: &FakeProtectedSigner,
    guardian: GuardianRuntimeSubject,
    command_id: &str,
) -> ApprovalChallenge {
    let subject = ApprovalSubject::ProtectedRelease(Box::new(ProtectedReleaseSubject::new(
        ReleaseFixture::baseline().build(),
        guardian,
    )));
    let identity = identity_for_subject(BindingFixture::baseline().build(), subject);
    FakeApprovalVerifier::new()
        .issue(IssueApprovalCommand {
            command_id: command_id.into(),
            expected_head: None,
            runtime: RuntimeKind::Fake,
            identity,
            nonce_id: format!("nonce-{command_id}"),
            nonce_commitment: nonce_commitment(
                &SecretMaterial::new(format!("secret-{command_id}").into_bytes()).expect("nonce"),
            )
            .expect("commitment"),
            issued_at: "2026-07-29T00:00:00Z".into(),
            expires_at: "2026-07-29T00:05:00Z".into(),
            authenticator_id: signer.authenticator_id().into(),
            key_id: signer.key_id().into(),
            verification_key_commitment: signer.trust_root_digest().clone(),
            evidence_digest: signer.evidence_digest().clone(),
            review_set_digest: Some(digest('6')),
        })
        .expect("issue protected challenge")
        .challenge
        .expect("protected challenge")
}

#[test]
fn normal_and_protected_signers_reject_the_opposite_trust_lane() {
    let normal = normal_signer();
    let protected = protected_signer();
    let normal_challenge = FakeApprovalVerifier::new()
        .issue(issue_command(
            "issue-normal-cross-product",
            normal_identity_with(
                "approval-normal-cross-product",
                "challenge-normal-cross-product",
                digest('a'),
            ),
            "nonce-normal-cross-product",
            nonce_commitment(
                &SecretMaterial::new(b"normal-cross-product".to_vec()).expect("nonce"),
            )
            .expect("commitment"),
            &normal,
        ))
        .expect("issue normal")
        .challenge
        .expect("normal challenge");
    assert!(normal.sign(&normal_challenge).is_ok());
    assert_eq!(
        protected.sign(&normal_challenge),
        Err(ApprovalVerifierError::ChallengeIntegrity)
    );

    let guardian = guardian_subject_with(
        protected.guardian_id(),
        protected.trust_root_digest().clone(),
        protected.daemon_instance_id(),
        protected.observed_epoch(),
    );
    let protected_challenge =
        issue_protected_challenge(&protected, guardian, "issue-protected-cross-product");
    assert!(protected.sign(&protected_challenge).is_ok());
    assert_eq!(
        normal.sign(&protected_challenge),
        Err(ApprovalVerifierError::ChallengeIntegrity)
    );
}

#[test]
fn protected_signer_rejects_guardian_subject_identity_runtime_and_trust_substitution() {
    let signer = protected_signer();
    let baseline = baseline_guardian_subject();
    let variants = [
        (
            "guardian_id",
            guardian_subject_with(
                "guardian-other",
                baseline.trust_root_digest().clone(),
                baseline.daemon_instance_id(),
                baseline.observed_epoch().get(),
            ),
        ),
        (
            "daemon_instance_id",
            guardian_subject_with(
                baseline.guardian_id(),
                baseline.trust_root_digest().clone(),
                "daemon-other",
                baseline.observed_epoch().get(),
            ),
        ),
        (
            "observed_epoch",
            guardian_subject_with(
                baseline.guardian_id(),
                baseline.trust_root_digest().clone(),
                baseline.daemon_instance_id(),
                8,
            ),
        ),
        (
            "trust_root_digest",
            guardian_subject_with(
                baseline.guardian_id(),
                digest('b'),
                baseline.daemon_instance_id(),
                baseline.observed_epoch().get(),
            ),
        ),
    ];
    for (field, guardian) in variants {
        let challenge =
            issue_protected_challenge(&signer, guardian, &format!("issue-substituted-{field}"));
        assert_eq!(
            signer.sign(&challenge),
            Err(ApprovalVerifierError::ChallengeIntegrity),
            "{field} substitution was signable"
        );
    }
}

fn verified_normal_fixture(approval_id: &str) -> FakeApprovalVerifier {
    let signer = normal_signer();
    let mut verifier = FakeApprovalVerifier::new();
    let issued = verifier
        .issue(issue_command(
            &format!("issue-{approval_id}"),
            normal_identity_with(
                approval_id,
                &format!("challenge-{approval_id}"),
                digest('a'),
            ),
            &format!("nonce-{approval_id}"),
            nonce_commitment(
                &SecretMaterial::new(format!("secret-{approval_id}").into_bytes()).expect("nonce"),
            )
            .expect("commitment"),
            &signer,
        ))
        .expect("issue");
    verifier
        .verify(VerifyApprovalCommand {
            command_id: format!("verify-{approval_id}"),
            approval_id: approval_id.into(),
            expected_head: issued.after.clone().expect("head"),
            observed_at: "2026-07-29T00:01:00Z".into(),
            proof: signer
                .sign(issued.challenge.as_ref().expect("challenge"))
                .expect("proof"),
        })
        .expect("verify");
    verifier
}

fn verified_protected_fixture(approval_id: &str) -> FakeApprovalVerifier {
    let signer = protected_signer();
    let identity =
        protected_release_identity_with(&signer, approval_id, &format!("challenge-{approval_id}"));
    let mut verifier = FakeApprovalVerifier::new();
    let issued = verifier
        .issue(IssueApprovalCommand {
            command_id: format!("issue-{approval_id}"),
            expected_head: None,
            runtime: RuntimeKind::Fake,
            identity,
            nonce_id: format!("nonce-{approval_id}"),
            nonce_commitment: nonce_commitment(
                &SecretMaterial::new(format!("secret-{approval_id}").into_bytes()).expect("nonce"),
            )
            .expect("commitment"),
            issued_at: "2026-07-29T00:00:00Z".into(),
            expires_at: "2026-07-29T00:05:00Z".into(),
            authenticator_id: signer.authenticator_id().into(),
            key_id: signer.key_id().into(),
            verification_key_commitment: signer.trust_root_digest().clone(),
            evidence_digest: signer.evidence_digest().clone(),
            review_set_digest: Some(digest('6')),
        })
        .expect("issue protected");
    verifier
        .verify(VerifyApprovalCommand {
            command_id: format!("verify-{approval_id}"),
            approval_id: approval_id.into(),
            expected_head: issued.after.expect("head"),
            observed_at: "2026-07-29T00:01:00Z".into(),
            proof: signer
                .sign(issued.challenge.as_ref().expect("challenge"))
                .expect("proof"),
        })
        .expect("verify protected");
    verifier
}

fn revoke_current(
    verifier: &mut FakeApprovalVerifier,
    command_id: &str,
    approval_id: &str,
    revoker_id: &str,
) -> ApprovalCommandReceipt {
    verifier
        .revoke(RevokeApprovalCommand {
            command_id: command_id.into(),
            approval_id: approval_id.into(),
            expected_head: verifier
                .state_head(approval_id)
                .expect("current state head"),
            observed_at: "2026-07-29T00:02:00Z".into(),
            revoker_id: revoker_id.into(),
            revocation_evidence_digest: digest('d'),
        })
        .expect("terminal revoke")
}

fn assert_available_revocation_replays(
    mut verifier: FakeApprovalVerifier,
    approval_id: &str,
    revoker_id: &str,
) {
    let prior_authority = verifier
        .current_head_at(approval_id, "2026-07-29T00:01:30Z")
        .expect("lookup")
        .expect("available authority");
    let receipt = revoke_current(
        &mut verifier,
        &format!("revoke-{approval_id}"),
        approval_id,
        revoker_id,
    );
    assert_eq!(receipt.outcome, ApprovalCommandOutcome::Applied);
    assert_eq!(
        receipt.after.as_ref().expect("revoked head").phase(),
        ApprovalPhase::Revoked
    );
    let revocation = receipt.revocation.as_ref().expect("revocation");
    assert_eq!(revocation.status(), ApprovalStatus::Revoked);
    assert_eq!(revocation.revoker_id(), revoker_id);
    assert_eq!(
        revocation.prior_authority_receipt_digest(),
        prior_authority.receipt_digest()
    );
    assert_eq!(verifier.revocation(approval_id), Some(revocation));
    assert!(
        verifier
            .current_head_at(approval_id, "2026-07-29T00:02:01Z")
            .expect("lookup")
            .is_none()
    );

    let snapshot = verifier.export_snapshot();
    let replayed = verify_snapshot(&snapshot).expect("revoked replay");
    assert_eq!(
        replayed
            .state_head(approval_id)
            .expect("replayed state")
            .phase(),
        ApprovalPhase::Revoked
    );
    assert_eq!(replayed.revocation(approval_id), Some(revocation));
}

#[test]
fn normal_and_protected_available_authority_can_be_revoked_and_strictly_replayed() {
    assert_available_revocation_replays(
        verified_normal_fixture("approval-revoke-normal"),
        "approval-revoke-normal",
        "approver-1",
    );
    assert_available_revocation_replays(
        verified_protected_fixture("approval-revoke-protected"),
        "approval-revoke-protected",
        "guardian-1",
    );
}

#[test]
fn wrong_revoker_denies_without_partial_revocation_or_authority_loss() {
    let approval_id = "approval-revoke-wrong";
    let mut verifier = verified_normal_fixture(approval_id);
    let before = verifier.state_head(approval_id).expect("verified state");
    let current = verifier
        .current_head_at(approval_id, "2026-07-29T00:01:30Z")
        .expect("lookup");
    let denied = revoke_current(
        &mut verifier,
        "revoke-wrong-actor",
        approval_id,
        "approver-other",
    );
    assert_eq!(
        denied.outcome,
        ApprovalCommandOutcome::Denied(ApprovalDenial::RevokerMismatch)
    );
    assert!(denied.revocation.is_none());
    assert_eq!(verifier.state_head(approval_id), Some(before));
    assert_eq!(
        verifier
            .current_head_at(approval_id, "2026-07-29T00:02:01Z")
            .expect("lookup"),
        current
    );
    assert!(verifier.revocation(approval_id).is_none());
}

#[test]
fn revocation_rejects_challenged_and_claimed_normal_states() {
    let signer = normal_signer();
    let challenged_id = "approval-revoke-challenged";
    let mut challenged = FakeApprovalVerifier::new();
    challenged
        .issue(issue_command(
            "issue-revoke-challenged",
            normal_identity_with(challenged_id, "challenge-revoke-challenged", digest('a')),
            "nonce-revoke-challenged",
            nonce_commitment(&SecretMaterial::new(b"revoke-challenged".to_vec()).expect("nonce"))
                .expect("commitment"),
            &signer,
        ))
        .expect("issue");
    assert_eq!(
        revoke_current(
            &mut challenged,
            "revoke-challenged",
            challenged_id,
            "approver-1"
        )
        .outcome,
        ApprovalCommandOutcome::Denied(ApprovalDenial::InvalidState)
    );

    let claimed_id = "approval-revoke-claimed";
    let mut claimed = verified_normal_fixture(claimed_id);
    claimed
        .consume_normal(ConsumeNormalApprovalCommand {
            command_id: "consume-before-revoke".into(),
            approval_id: claimed_id.into(),
            expected_head: claimed.state_head(claimed_id).expect("verified head"),
            observed_at: "2026-07-29T00:01:30Z".into(),
            claim_digest: digest('c'),
        })
        .expect("consume");
    assert_eq!(
        revoke_current(&mut claimed, "revoke-claimed", claimed_id, "approver-1").outcome,
        ApprovalCommandOutcome::Denied(ApprovalDenial::InvalidState)
    );
}

#[test]
fn revocation_is_terminal_and_cannot_be_applied_twice() {
    let approval_id = "approval-revoke-twice";
    let mut verifier = verified_normal_fixture(approval_id);
    let first = revoke_current(&mut verifier, "revoke-first", approval_id, "approver-1");
    assert_eq!(first.outcome, ApprovalCommandOutcome::Applied);
    let second = revoke_current(&mut verifier, "revoke-second", approval_id, "approver-1");
    assert_eq!(
        second.outcome,
        ApprovalCommandOutcome::Denied(ApprovalDenial::InvalidState)
    );
    assert_eq!(
        verifier.state_head(approval_id).expect("state").phase(),
        ApprovalPhase::Revoked
    );
}

fn object_field_mut<'a>(value: &'a mut CanonicalValue, key: &str) -> &'a mut CanonicalValue {
    let CanonicalValue::Object(entries) = value else {
        panic!("expected object");
    };
    entries
        .iter_mut()
        .find_map(|(candidate, value)| (candidate == key).then_some(value))
        .expect("field")
}

#[test]
fn raw_snapshot_strict_replay_and_checkpoint_restore_round_trip() {
    let mut verifier = verified_normal_fixture("approval-snapshot");
    let current = verifier
        .state_head("approval-snapshot")
        .expect("verified state");
    verifier
        .consume_normal(ConsumeNormalApprovalCommand {
            command_id: "consume-approval-snapshot".into(),
            approval_id: "approval-snapshot".into(),
            expected_head: current,
            observed_at: "2026-07-29T00:02:00Z".into(),
            claim_digest: digest('8'),
        })
        .expect("claim");
    let snapshot = verifier.export_snapshot();
    let checkpoint = verifier.current_checkpoint().expect("checkpoint");
    let replayed = verify_snapshot(&snapshot).expect("strict replay");
    assert_eq!(replayed.command_receipts().len(), 3);
    assert_eq!(checkpoint.command_high_water(), 3);
    assert!(!format!("{snapshot:?}").contains("secret-approval-snapshot"));

    let mut restored = FakeApprovalVerifier::new();
    restored
        .restore_snapshot(&snapshot, &checkpoint)
        .expect("restore");
    assert_eq!(restored.command_receipts(), verifier.command_receipts());
    assert!(
        restored
            .current_head_at("approval-snapshot", "2026-07-29T00:02:01Z")
            .expect("lookup")
            .is_none()
    );
    assert_eq!(
        restored.restore_snapshot(&snapshot, &checkpoint),
        Err(ApprovalVerifierError::RestoreWouldOverwrite)
    );
}

#[test]
fn raw_snapshot_rejects_unknown_fields_reorder_truncation_and_claimed_drift() {
    let mut verifier = verified_normal_fixture("approval-tamper");
    let current = verifier
        .state_head("approval-tamper")
        .expect("verified state");
    verifier
        .consume_normal(ConsumeNormalApprovalCommand {
            command_id: "consume-approval-tamper".into(),
            approval_id: "approval-tamper".into(),
            expected_head: current,
            observed_at: "2026-07-29T00:02:00Z".into(),
            claim_digest: digest('9'),
        })
        .expect("consume");
    let snapshot = verifier.export_snapshot();

    let mut unknown = snapshot.clone();
    let CanonicalValue::Object(entries) = &mut unknown.payload else {
        panic!("snapshot object");
    };
    entries.push(("unknown".into(), CanonicalValue::Null));
    assert_eq!(
        verify_snapshot(&unknown),
        Err(ApprovalVerifierError::CorruptSnapshot)
    );

    let mut reordered = snapshot.clone();
    let CanonicalValue::Array(commands) = object_field_mut(&mut reordered.payload, "commands")
    else {
        panic!("commands");
    };
    commands.swap(0, 1);
    assert_eq!(
        verify_snapshot(&reordered),
        Err(ApprovalVerifierError::CorruptSnapshot)
    );

    let mut truncated = snapshot.clone();
    let CanonicalValue::Array(commands) = object_field_mut(&mut truncated.payload, "commands")
    else {
        panic!("commands");
    };
    commands.pop();
    assert_eq!(
        verify_snapshot(&truncated),
        Err(ApprovalVerifierError::CorruptSnapshot)
    );

    let mut drifted = snapshot;
    let CanonicalValue::Array(approvals) = object_field_mut(&mut drifted.payload, "approvals")
    else {
        panic!("approvals");
    };
    *object_field_mut(approvals.first_mut().expect("record"), "phase") =
        CanonicalValue::String("VERIFIED_AVAILABLE".into());
    assert_eq!(
        verify_snapshot(&drifted),
        Err(ApprovalVerifierError::CorruptSnapshot)
    );
}

fn assert_corrupt(snapshot: &lattice_approval_verifier::UntrustedApprovalSnapshot) {
    assert_eq!(
        verify_snapshot(snapshot),
        Err(ApprovalVerifierError::CorruptSnapshot)
    );
}

#[test]
fn raw_snapshot_corruption_matrix_fails_closed() {
    let snapshot = verified_normal_fixture("approval-matrix").export_snapshot();

    let mut malformed = snapshot.clone();
    *object_field_mut(&mut malformed.payload, "command_high_water") = CanonicalValue::Bool(true);
    assert_corrupt(&malformed);

    let mut unknown_version = snapshot.clone();
    *object_field_mut(&mut unknown_version.payload, "version") =
        CanonicalValue::String("2.0".into());
    assert_corrupt(&unknown_version);

    let mut unknown_kind = snapshot.clone();
    let CanonicalValue::Array(commands) = object_field_mut(&mut unknown_kind.payload, "commands")
    else {
        panic!("commands");
    };
    let request = object_field_mut(commands.first_mut().expect("command"), "request");
    *object_field_mut(request, "kind") = CanonicalValue::String("UNKNOWN".into());
    assert_corrupt(&unknown_kind);

    let mut digest_tamper = snapshot.clone();
    let CanonicalValue::Array(commands) = object_field_mut(&mut digest_tamper.payload, "commands")
    else {
        panic!("commands");
    };
    *object_field_mut(commands.first_mut().expect("command"), "request_digest") =
        CanonicalValue::String("f".repeat(64));
    assert_corrupt(&digest_tamper);

    let mut duplicated = snapshot.clone();
    let CanonicalValue::Array(commands) = object_field_mut(&mut duplicated.payload, "commands")
    else {
        panic!("commands");
    };
    commands.push(commands.first().expect("command").clone());
    *object_field_mut(&mut duplicated.payload, "command_high_water") =
        CanonicalValue::String("3".into());
    assert_corrupt(&duplicated);

    let mut orphan = snapshot.clone();
    let CanonicalValue::Array(approvals) = object_field_mut(&mut orphan.payload, "approvals")
    else {
        panic!("approvals");
    };
    approvals.push(approvals.first().expect("approval").clone());
    assert_corrupt(&orphan);

    let mut rebound = snapshot.clone();
    let CanonicalValue::Array(bindings) = object_field_mut(&mut rebound.payload, "nonce_bindings")
    else {
        panic!("bindings");
    };
    *object_field_mut(bindings.first_mut().expect("binding"), "approval_id") =
        CanonicalValue::String("approval-other".into());
    assert_corrupt(&rebound);

    let mut fake_live_mix = snapshot.clone();
    let CanonicalValue::Array(commands) = object_field_mut(&mut fake_live_mix.payload, "commands")
    else {
        panic!("commands");
    };
    let request = object_field_mut(commands.first_mut().expect("command"), "request");
    *object_field_mut(request, "runtime") = CanonicalValue::String("LIVE".into());
    assert_corrupt(&fake_live_mix);

    let mut chain_tamper = snapshot.clone();
    let CanonicalValue::Array(commands) = object_field_mut(&mut chain_tamper.payload, "commands")
    else {
        panic!("commands");
    };
    *object_field_mut(
        commands.get_mut(1).expect("second command"),
        "previous_receipt_digest",
    ) = CanonicalValue::String("e".repeat(64));
    assert_corrupt(&chain_tamper);

    let mut high_water_tamper = snapshot;
    *object_field_mut(&mut high_water_tamper.payload, "command_high_water") =
        CanonicalValue::String("1".into());
    assert_corrupt(&high_water_tamper);
}

#[test]
fn trusted_checkpoint_rejects_an_internally_coherent_older_prefix() {
    let signer = normal_signer();
    let mut verifier = FakeApprovalVerifier::new();
    let issued = verifier
        .issue(issue_command(
            "issue-rollback",
            normal_identity_with("approval-rollback", "challenge-rollback", digest('a')),
            "nonce-rollback",
            nonce_commitment(&SecretMaterial::new(b"rollback-nonce".to_vec()).expect("nonce"))
                .expect("commitment"),
            &signer,
        ))
        .expect("issue");
    let older_prefix = verifier.export_snapshot();
    verifier
        .verify(VerifyApprovalCommand {
            command_id: "verify-rollback".into(),
            approval_id: "approval-rollback".into(),
            expected_head: issued.after.expect("head"),
            observed_at: "2026-07-29T00:01:00Z".into(),
            proof: signer
                .sign(issued.challenge.as_ref().expect("challenge"))
                .expect("proof"),
        })
        .expect("verify");
    let current_checkpoint = verifier.current_checkpoint().expect("checkpoint");

    assert!(verify_snapshot(&older_prefix).is_ok());
    assert_eq!(
        verify_snapshot_against_checkpoint(&older_prefix, &current_checkpoint),
        Err(ApprovalVerifierError::CheckpointMismatch)
    );
}

#[test]
fn denied_tail_truncation_and_coherent_prefix_rollback_fail_closed() {
    let signer = normal_signer();
    let mut verifier = FakeApprovalVerifier::new();
    let issued = verifier
        .issue(issue_command(
            "issue-denied-tail",
            normal_identity_with("approval-denied-tail", "challenge-denied-tail", digest('a')),
            "nonce-denied-tail",
            nonce_commitment(&SecretMaterial::new(b"denied-tail-nonce".to_vec()).expect("nonce"))
                .expect("commitment"),
            &signer,
        ))
        .expect("issue");
    let coherent_older_prefix = verifier.export_snapshot();
    let denied = verifier
        .verify(VerifyApprovalCommand {
            command_id: "verify-denied-tail".into(),
            approval_id: "approval-denied-tail".into(),
            expected_head: issued.after.expect("head"),
            observed_at: "2026-07-28T23:59:59Z".into(),
            proof: signer
                .sign(issued.challenge.as_ref().expect("challenge"))
                .expect("proof"),
        })
        .expect("terminal denial");
    assert_eq!(
        denied.outcome,
        ApprovalCommandOutcome::Denied(ApprovalDenial::NotYetValid)
    );
    let post_denial_checkpoint = verifier.current_checkpoint().expect("checkpoint");
    assert_eq!(post_denial_checkpoint.command_high_water(), 2);

    let mut truncated = verifier.export_snapshot();
    let CanonicalValue::Array(commands) = object_field_mut(&mut truncated.payload, "commands")
    else {
        panic!("commands");
    };
    commands.pop();
    assert_eq!(
        verify_snapshot(&truncated),
        Err(ApprovalVerifierError::CorruptSnapshot)
    );

    assert!(verify_snapshot(&coherent_older_prefix).is_ok());
    assert_eq!(
        verify_snapshot_against_checkpoint(&coherent_older_prefix, &post_denial_checkpoint),
        Err(ApprovalVerifierError::CheckpointMismatch)
    );
}
