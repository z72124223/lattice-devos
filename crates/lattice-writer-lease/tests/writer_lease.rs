use lattice_cjson::CanonicalValue;
use lattice_contracts::{
    ContentDigest, DaemonEpoch, HolderProcessId, ProjectId, RuntimeAdmissionMode, RuntimeKind,
    WriterLeaseStatus,
};
use lattice_writer_lease::test_support::{acquire_command, digest, observation};
use lattice_writer_lease::{
    CommandOutcome, FakeWriterLease, HeartbeatCommand, LeaseDenial, MarkSuspectCommand,
    ProcessHandoffCommand, RecoveryEvidence, ReleaseCommand, RevokeCommand,
    WriterLeaseAcquireRequest, WriterLeaseCommand, WriterLeaseCurrentAuthority, WriterLeaseError,
    WriterLeaseProjectEvidence, WriterLeaseRepository, WriterLeaseRepositoryCommand,
    WriterLeaseRepositoryError, WriterLeaseRepositoryErrorKind, apply_plan, plan_command,
    verify_snapshot, verify_snapshot_against_checkpoint,
};

fn project(name: &str) -> ProjectId {
    ProjectId::new(name).expect("project")
}

fn object_field_mut<'a>(value: &'a mut CanonicalValue, field_name: &str) -> &'a mut CanonicalValue {
    let CanonicalValue::Object(fields) = value else {
        panic!("expected object");
    };
    fields
        .iter_mut()
        .find_map(|(name, value)| (name == field_name).then_some(value))
        .expect("field")
}

fn array_field_mut<'a>(
    value: &'a mut CanonicalValue,
    field_name: &str,
) -> &'a mut Vec<CanonicalValue> {
    let CanonicalValue::Array(values) = object_field_mut(value, field_name) else {
        panic!("expected array");
    };
    values
}

fn array_field<'a>(value: &'a CanonicalValue, field_name: &str) -> &'a [CanonicalValue] {
    let CanonicalValue::Object(fields) = value else {
        panic!("expected object");
    };
    let CanonicalValue::Array(values) = fields
        .iter()
        .find_map(|(name, value)| (name == field_name).then_some(value))
        .expect("field")
    else {
        panic!("expected array");
    };
    values
}

fn set_string_field(value: &mut CanonicalValue, field_name: &str, replacement: &str) {
    *object_field_mut(value, field_name) = CanonicalValue::String(replacement.to_owned());
}

fn assert_corrupt(snapshot: &lattice_writer_lease::UntrustedWriterLeaseSnapshot) {
    assert_eq!(
        verify_snapshot(snapshot),
        Err(WriterLeaseError::CorruptSnapshot)
    );
}

fn acquire(project: &ProjectId, command_id: &str) -> WriterLeaseCommand {
    acquire_command(project, command_id, 7)
}

fn heartbeat(
    fake: &FakeWriterLease,
    project: &ProjectId,
    command_id: &str,
    observed_at: &str,
    expires_at: &str,
    admission: RuntimeAdmissionMode,
) -> WriterLeaseCommand {
    WriterLeaseCommand::Heartbeat(HeartbeatCommand {
        command_id: command_id.to_owned(),
        project_id: project.clone(),
        expected_head: fake.current_head(project).expect("current head"),
        observation: observation(admission, observed_at),
        expires_at: expires_at.to_owned(),
    })
}

fn suspect(
    fake: &FakeWriterLease,
    project: &ProjectId,
    command_id: &str,
    observed_at: &str,
    admission: RuntimeAdmissionMode,
) -> WriterLeaseCommand {
    WriterLeaseCommand::MarkSuspect(MarkSuspectCommand {
        command_id: command_id.to_owned(),
        project_id: project.clone(),
        expected_head: fake.current_head(project).expect("current head"),
        observation: observation(admission, observed_at),
    })
}

fn release(
    fake: &FakeWriterLease,
    project: &ProjectId,
    command_id: &str,
    admission: RuntimeAdmissionMode,
) -> WriterLeaseCommand {
    WriterLeaseCommand::Release(ReleaseCommand {
        command_id: command_id.to_owned(),
        project_id: project.clone(),
        expected_head: fake.current_head(project).expect("current head"),
        observation: observation(admission, "2026-07-29T00:20:00Z"),
    })
}

fn revoke(
    fake: &FakeWriterLease,
    project: &ProjectId,
    command_id: &str,
    evidence: RecoveryEvidence,
    admission: RuntimeAdmissionMode,
) -> WriterLeaseCommand {
    WriterLeaseCommand::Revoke(RevokeCommand {
        command_id: command_id.to_owned(),
        project_id: project.clone(),
        expected_head: fake.current_head(project).expect("current head"),
        observation: observation(admission, "2026-07-29T00:20:00Z"),
        evidence,
    })
}

fn process_death(start_digest: ContentDigest) -> RecoveryEvidence {
    RecoveryEvidence::ProcessDeath {
        holder_process_id: HolderProcessId::new(42).expect("pid"),
        holder_process_start_identity: start_digest,
        holder_daemon_instance_id: "daemon-1".to_owned(),
        evidence_digest: digest('8'),
    }
}

fn process_handoff(
    fake: &FakeWriterLease,
    project: &ProjectId,
    command_id: &str,
    successor_process_id: u64,
    successor_process_start_identity: ContentDigest,
    observed_at: &str,
    expires_at: &str,
) -> WriterLeaseCommand {
    WriterLeaseCommand::ProcessHandoff(ProcessHandoffCommand {
        command_id: command_id.to_owned(),
        project_id: project.clone(),
        expected_head: fake.current_head(project).expect("current head"),
        successor_holder_process_id: HolderProcessId::new(successor_process_id).expect("pid"),
        successor_holder_process_start_identity: successor_process_start_identity,
        successor_daemon_instance_id: "daemon-1".to_owned(),
        successor_daemon_epoch: DaemonEpoch::new(7).expect("epoch"),
        observation: observation(RuntimeAdmissionMode::Active, observed_at),
        expires_at: expires_at.to_owned(),
        evidence: process_death(digest('2')),
    })
}

fn assert_process_death_daemon_binding(
    fake: &FakeWriterLease,
    project: &ProjectId,
    exact_start: &ContentDigest,
) {
    let aggregate = verify_snapshot(&fake.export_snapshot(project).expect("snapshot"))
        .expect("verified aggregate");
    let exact_binding = revoke(
        fake,
        project,
        "daemon-binding",
        process_death(exact_start.clone()),
        RuntimeAdmissionMode::ReconciliationRequired,
    );
    let mut substituted_binding = exact_binding.clone();
    {
        let WriterLeaseCommand::Revoke(substituted) = &mut substituted_binding else {
            unreachable!()
        };
        let RecoveryEvidence::ProcessDeath {
            holder_daemon_instance_id,
            ..
        } = &mut substituted.evidence
        else {
            unreachable!()
        };
        "daemon-substitute".clone_into(holder_daemon_instance_id);
    }
    assert_ne!(
        plan_command(&aggregate, &exact_binding)
            .expect("exact binding plan")
            .receipt()
            .request_digest,
        plan_command(&aggregate, &substituted_binding)
            .expect("substituted binding plan")
            .receipt()
            .request_digest
    );
    let WriterLeaseCommand::Revoke(substituted) = &mut substituted_binding else {
        unreachable!()
    };
    let RecoveryEvidence::ProcessDeath {
        holder_daemon_instance_id,
        ..
    } = &mut substituted.evidence
    else {
        unreachable!()
    };
    *holder_daemon_instance_id = String::new();
    assert!(matches!(
        plan_command(&aggregate, &substituted_binding),
        Err(WriterLeaseError::InvalidRecoveryEvidence)
    ));
}

#[test]
fn empty_fake_starts_without_writer_authority() {
    let fake = FakeWriterLease::new();
    assert_eq!(fake.project_count(), 0);
    assert!(fake.current_head(&project("project-a")).is_none());
}

#[test]
fn process_handoff_preserves_logical_attempt_and_fence_while_replacing_process() {
    let project = project("project-process-handoff");
    let mut fake = FakeWriterLease::new();
    fake.execute(acquire(&project, "acquire")).expect("acquire");
    let before = fake.current_receipt(&project).expect("before receipt");

    let receipt = fake
        .execute(process_handoff(
            &fake,
            &project,
            "handoff",
            84,
            digest('5'),
            "2026-07-29T00:05:00Z",
            "2026-07-29T00:15:00Z",
        ))
        .expect("handoff");

    assert_eq!(receipt.outcome, CommandOutcome::Applied);
    let after = fake.current_receipt(&project).expect("after receipt");
    assert_eq!(
        after.identity().project_id(),
        before.identity().project_id()
    );
    assert_eq!(after.identity().task_id(), before.identity().task_id());
    assert_eq!(
        after.identity().attempt_id(),
        before.identity().attempt_id()
    );
    assert_eq!(after.identity().lease_id(), before.identity().lease_id());
    assert_eq!(
        after.identity().worktree_id(),
        before.identity().worktree_id()
    );
    assert_eq!(
        after.identity().fencing_token(),
        before.identity().fencing_token()
    );
    assert_eq!(after.identity().holder_process_id().get(), 84);
    assert_eq!(
        after.identity().holder_process_start_identity(),
        &digest('5')
    );
    assert_eq!(after.status(), WriterLeaseStatus::Active);
    assert_ne!(after.head(), before.head());
}

#[test]
fn process_handoff_supports_pid_reuse_and_exact_retry_but_rejects_substitution() {
    let project = project("project-process-handoff-retry");
    let mut fake = FakeWriterLease::new();
    fake.execute(acquire(&project, "acquire")).expect("acquire");
    let handoff = process_handoff(
        &fake,
        &project,
        "handoff",
        42,
        digest('5'),
        "2026-07-29T00:05:00Z",
        "2026-07-29T00:15:00Z",
    );

    let first = fake.execute(handoff.clone()).expect("pid-reuse handoff");
    let retry = fake.execute(handoff.clone()).expect("exact retry");
    assert_eq!(retry, first);
    assert_eq!(
        fake.current_receipt(&project)
            .expect("authority")
            .identity()
            .holder_process_id()
            .get(),
        42
    );

    let mut substituted = handoff;
    let WriterLeaseCommand::ProcessHandoff(command) = &mut substituted else {
        unreachable!()
    };
    command.successor_holder_process_start_identity = digest('6');
    assert_eq!(
        fake.execute(substituted),
        Err(WriterLeaseError::CommandIdReuse)
    );
}

#[test]
fn process_handoff_requires_live_active_time_or_an_exact_suspect_recovery() {
    let project = project("project-process-handoff-state");
    let mut fake = FakeWriterLease::new();
    fake.execute(acquire(&project, "acquire")).expect("acquire");

    let expired = fake
        .execute(process_handoff(
            &fake,
            &project,
            "expired-handoff",
            84,
            digest('5'),
            "2026-07-29T00:10:00Z",
            "2026-07-29T00:20:00Z",
        ))
        .expect("terminal expired denial");
    assert_eq!(
        expired.outcome,
        CommandOutcome::Denied(LeaseDenial::InvalidState)
    );

    fake.execute(suspect(
        &fake,
        &project,
        "suspect",
        "2026-07-29T00:10:00Z",
        RuntimeAdmissionMode::Draining,
    ))
    .expect("mark suspect");
    let recovered = fake
        .execute(process_handoff(
            &fake,
            &project,
            "suspect-handoff",
            84,
            digest('5'),
            "2026-07-29T00:11:00Z",
            "2026-07-29T00:21:00Z",
        ))
        .expect("suspect handoff");
    assert_eq!(recovered.outcome, CommandOutcome::Applied);
    assert_eq!(
        fake.current_receipt(&project).expect("active").status(),
        WriterLeaseStatus::Active
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn process_handoff_rejects_wrong_evidence_leadership_and_admission() {
    let project = project("project-process-handoff-evidence");
    let mut fake = FakeWriterLease::new();
    fake.execute(acquire(&project, "acquire")).expect("acquire");

    let base = process_handoff(
        &fake,
        &project,
        "handoff",
        84,
        digest('5'),
        "2026-07-29T00:05:00Z",
        "2026-07-29T00:15:00Z",
    );
    let mut wrong_start = base.clone();
    let WriterLeaseCommand::ProcessHandoff(command) = &mut wrong_start else {
        unreachable!()
    };
    command.command_id = "wrong-start".to_owned();
    command.evidence = process_death(digest('9'));
    assert_eq!(
        fake.execute(wrong_start)
            .expect("terminal evidence denial")
            .outcome,
        CommandOutcome::Denied(LeaseDenial::RecoveryEvidenceMismatch)
    );

    let mut changed_daemon = base.clone();
    let WriterLeaseCommand::ProcessHandoff(command) = &mut changed_daemon else {
        unreachable!()
    };
    command.command_id = "changed-daemon".to_owned();
    command.successor_daemon_instance_id = "daemon-2".to_owned();
    assert_eq!(
        fake.execute(changed_daemon)
            .expect("terminal daemon denial")
            .outcome,
        CommandOutcome::Denied(LeaseDenial::RecoveryEvidenceMismatch)
    );

    let mut changed_epoch = base.clone();
    let WriterLeaseCommand::ProcessHandoff(command) = &mut changed_epoch else {
        unreachable!()
    };
    command.command_id = "changed-epoch".to_owned();
    command.successor_daemon_epoch = DaemonEpoch::new(8).expect("epoch");
    assert_eq!(
        fake.execute(changed_epoch)
            .expect("terminal epoch denial")
            .outcome,
        CommandOutcome::Denied(LeaseDenial::RecoveryEvidenceMismatch)
    );

    let mut wrong_pid = base.clone();
    let WriterLeaseCommand::ProcessHandoff(command) = &mut wrong_pid else {
        unreachable!()
    };
    command.command_id = "wrong-old-pid".to_owned();
    let RecoveryEvidence::ProcessDeath {
        holder_process_id, ..
    } = &mut command.evidence
    else {
        unreachable!()
    };
    *holder_process_id = HolderProcessId::new(41).expect("pid");
    assert_eq!(
        fake.execute(wrong_pid)
            .expect("terminal old-pid denial")
            .outcome,
        CommandOutcome::Denied(LeaseDenial::RecoveryEvidenceMismatch)
    );

    let mut wrong_evidence_daemon = base.clone();
    let WriterLeaseCommand::ProcessHandoff(command) = &mut wrong_evidence_daemon else {
        unreachable!()
    };
    command.command_id = "wrong-evidence-daemon".to_owned();
    let RecoveryEvidence::ProcessDeath {
        holder_daemon_instance_id,
        ..
    } = &mut command.evidence
    else {
        unreachable!()
    };
    *holder_daemon_instance_id = "daemon-2".to_owned();
    assert_eq!(
        fake.execute(wrong_evidence_daemon)
            .expect("terminal evidence-daemon denial")
            .outcome,
        CommandOutcome::Denied(LeaseDenial::RecoveryEvidenceMismatch)
    );

    let mut same_process = base.clone();
    let WriterLeaseCommand::ProcessHandoff(command) = &mut same_process else {
        unreachable!()
    };
    command.command_id = "same-process".to_owned();
    command.successor_holder_process_id = HolderProcessId::new(42).expect("pid");
    command.successor_holder_process_start_identity = digest('2');
    assert_eq!(
        fake.execute(same_process)
            .expect("terminal same-process denial")
            .outcome,
        CommandOutcome::Denied(LeaseDenial::RecoveryEvidenceMismatch)
    );

    let mut leadership = base.clone();
    let WriterLeaseCommand::ProcessHandoff(command) = &mut leadership else {
        unreachable!()
    };
    command.command_id = "leadership".to_owned();
    command.evidence = RecoveryEvidence::LeadershipReplaced {
        replaced_daemon_instance_id: "daemon-1".to_owned(),
        replaced_epoch: DaemonEpoch::new(7).expect("epoch"),
        replacement_daemon_instance_id: "daemon-2".to_owned(),
        replacement_epoch: DaemonEpoch::new(8).expect("epoch"),
        evidence_digest: digest('8'),
    };
    assert_eq!(
        fake.execute(leadership),
        Err(WriterLeaseError::InvalidRecoveryEvidence)
    );

    let mut draining = base;
    let WriterLeaseCommand::ProcessHandoff(command) = &mut draining else {
        unreachable!()
    };
    command.command_id = "draining".to_owned();
    command.observation.admission = RuntimeAdmissionMode::Draining;
    assert_eq!(
        fake.execute(draining)
            .expect("terminal admission denial")
            .outcome,
        CommandOutcome::Denied(LeaseDenial::AdmissionDenied)
    );
}

#[test]
fn process_handoff_snapshot_replays_and_detects_process_substitution() {
    let project = project("project-process-handoff-snapshot");
    let mut fake = FakeWriterLease::new();
    fake.execute(acquire(&project, "acquire")).expect("acquire");
    fake.execute(process_handoff(
        &fake,
        &project,
        "handoff",
        84,
        digest('5'),
        "2026-07-29T00:05:00Z",
        "2026-07-29T00:15:00Z",
    ))
    .expect("handoff");
    let snapshot = fake.export_snapshot(&project).expect("snapshot");
    let checkpoint = fake
        .current_checkpoint(&project)
        .expect("checkpoint")
        .expect("project checkpoint");
    let bytes = snapshot.canonical_bytes().expect("bytes");
    let decoded = lattice_writer_lease::UntrustedWriterLeaseSnapshot::from_canonical_bytes(&bytes)
        .expect("decode");
    assert_eq!(
        verify_snapshot_against_checkpoint(&decoded, &checkpoint)
            .expect("replay")
            .current_head(),
        fake.current_head(&project)
    );

    let mut tampered = snapshot;
    let handoff_receipt = &mut array_field_mut(&mut tampered.payload, "commands")[1];
    let request = object_field_mut(handoff_receipt, "request");
    set_string_field(
        request,
        "successor_holder_process_start_identity",
        digest('6').as_str(),
    );
    assert_corrupt(&tampered);
}

#[test]
fn historical_authority_lookup_survives_release_and_unknown_digest_is_absent() {
    let project = project("project-historical-authority");
    let mut fake = FakeWriterLease::new();
    fake.execute(acquire(&project, "acquire")).expect("acquire");
    let historical = fake.current_receipt(&project).expect("historical receipt");
    fake.execute(release(
        &fake,
        &project,
        "release",
        RuntimeAdmissionMode::Draining,
    ))
    .expect("release");
    let verified = verify_snapshot(&fake.export_snapshot(&project).expect("snapshot"))
        .expect("verified released aggregate");

    assert_eq!(
        verified
            .historical_authority_receipt(historical.receipt_digest())
            .expect("historical lookup"),
        Some(historical)
    );
    assert_eq!(
        verified
            .historical_authority_receipt(&digest('9'))
            .expect("unknown lookup"),
        None
    );
    assert_eq!(
        verified.historical_authority_receipt(&digest('0')),
        Err(WriterLeaseError::ZeroEvidenceDigest)
    );

    let mut duplicated = fake.export_snapshot(&project).expect("released snapshot");
    let duplicate = array_field_mut(&mut duplicated.payload, "transitions")[0].clone();
    array_field_mut(&mut duplicated.payload, "transitions").push(duplicate);
    assert_eq!(
        verify_snapshot(&duplicated),
        Err(WriterLeaseError::CorruptSnapshot),
        "duplicate historical authority evidence must fail before lookup"
    );
}

#[test]
fn canonical_snapshot_bytes_round_trip_through_the_public_verifier() {
    let project = project("project-canonical-persistence");
    let mut fake = FakeWriterLease::new();
    fake.execute(acquire(&project, "acquire")).expect("acquire");
    let snapshot = fake.export_snapshot(&project).expect("snapshot");
    let checkpoint = fake
        .current_checkpoint(&project)
        .expect("checkpoint")
        .expect("current checkpoint");

    let bytes = snapshot.canonical_bytes().expect("canonical bytes");
    let decoded = lattice_writer_lease::UntrustedWriterLeaseSnapshot::from_canonical_bytes(&bytes)
        .expect("strict canonical decode");
    assert_eq!(decoded.canonical_bytes().expect("decoded bytes"), bytes);
    assert_eq!(
        verify_snapshot_against_checkpoint(&decoded, &checkpoint)
            .expect("verified round trip")
            .current_head(),
        fake.current_head(&project)
    );

    for invalid in [
        b" {\"schema_version\":\"1.0\"}".as_slice(),
        b"{\"schema_version\":\"1.0\",\"schema_version\":\"1.0\"}".as_slice(),
        b"{\"schema_version\":1}".as_slice(),
        b"{\"schema_version\":\"1.0\"} trailing".as_slice(),
        &[0xff_u8][..],
    ] {
        assert_eq!(
            lattice_writer_lease::UntrustedWriterLeaseSnapshot::from_canonical_bytes(invalid),
            Err(WriterLeaseError::CorruptSnapshot)
        );
    }

    let deeply_nested = format!("{}null{}", "[".repeat(130), "]".repeat(130));
    assert_eq!(
        lattice_writer_lease::UntrustedWriterLeaseSnapshot::from_canonical_bytes(
            deeply_nested.as_bytes()
        ),
        Err(WriterLeaseError::CorruptSnapshot)
    );

    let oversized = lattice_writer_lease::UntrustedWriterLeaseSnapshot {
        payload: lattice_cjson::CanonicalValue::String(
            "x".repeat(lattice_writer_lease::MAX_CANONICAL_SNAPSHOT_BYTES),
        ),
    };
    assert_eq!(
        oversized.canonical_bytes(),
        Err(WriterLeaseError::CorruptSnapshot)
    );
}

#[test]
fn repository_intent_bytes_exclude_adapter_owned_observation() {
    let project = project("project-repository-intent");
    let mut pure_command = acquire(&project, "repository-acquire");
    let WriterLeaseCommand::Acquire(command) = &mut pure_command else {
        panic!("acquire fixture");
    };
    command.observation.runtime = RuntimeKind::Live;
    let WriterLeaseCommand::Acquire(pure) = &pure_command else {
        panic!("acquire fixture");
    };
    let request = WriterLeaseRepositoryCommand::Acquire(WriterLeaseAcquireRequest {
        command_id: pure.command_id.clone(),
        expected_head: pure.expected_head.clone(),
        project_id: pure.claim.project_id.clone(),
        project_snapshot_id: pure.claim.project_snapshot_id.clone(),
        task_id: pure.claim.task_id.clone(),
        task_revision: pure.claim.task_revision.clone(),
        task_spec_digest: pure.claim.task_spec_digest.clone(),
        attempt_id: pure.claim.attempt_id.clone(),
        lease_id: pure.claim.lease_id.clone(),
        lease_holder_id: pure.claim.lease_holder_id.clone(),
        worktree_id: pure.claim.worktree_id.clone(),
        holder_process_id: pure.claim.holder_process_id,
        holder_process_start_identity: pure.claim.holder_process_start_identity.clone(),
    });

    let bytes = request.canonical_bytes().expect("canonical intent");
    let text = std::str::from_utf8(&bytes).expect("UTF-8 intent");
    assert_eq!(request.command_id(), "repository-acquire");
    assert!(text.contains("\"kind\":\"ACQUIRE\""));
    assert!(!text.contains("observed_at"));
    assert!(!text.contains("admission"));
    assert!(!text.contains("daemon_instance_id"));
    assert!(!text.contains("fencing_token"));
    assert_eq!(request.canonical_bytes().expect("stable intent"), bytes);
    assert_eq!(
        pure_command
            .repository_intent_canonical_bytes()
            .expect("reconstructed intent"),
        bytes
    );
    assert!(
        !pure_command
            .canonical_bytes()
            .expect("pure command")
            .is_empty()
    );
}

#[test]
fn repository_contract_keeps_current_receipt_and_independent_head_together() {
    struct ContractOnlyRepository;

    impl WriterLeaseRepository for ContractOnlyRepository {
        fn execute(
            &mut self,
            _command: WriterLeaseRepositoryCommand,
        ) -> Result<lattice_writer_lease::WriterLeaseCommandReceipt, WriterLeaseRepositoryError>
        {
            Err(WriterLeaseRepositoryError::new(
                WriterLeaseRepositoryErrorKind::Unavailable,
            ))
        }

        fn current_authority(
            &mut self,
            _project_id: &ProjectId,
        ) -> Result<Option<WriterLeaseCurrentAuthority>, WriterLeaseRepositoryError> {
            Ok(None)
        }

        fn assert_current(
            &mut self,
            _expected: &lattice_contracts::WriterLeaseAuthorityHead,
        ) -> Result<(), WriterLeaseRepositoryError> {
            Err(WriterLeaseRepositoryError::new(
                WriterLeaseRepositoryErrorKind::AuthorityMismatch,
            ))
        }
    }

    fn assert_repository<T: WriterLeaseRepository>() {}
    assert_repository::<ContractOnlyRepository>();
    assert_eq!(
        WriterLeaseRepositoryError::new(WriterLeaseRepositoryErrorKind::Unavailable).code(),
        "WRITER_LEASE_REPOSITORY_UNAVAILABLE"
    );
    assert_eq!(
        WriterLeaseRepositoryError::from_domain(WriterLeaseError::CommandIdReuse).kind(),
        WriterLeaseRepositoryErrorKind::Domain
    );
}

#[test]
fn project_evidence_preserves_active_and_released_replay_high_waters() {
    let project = project("project-persistence-evidence");
    let mut fake = FakeWriterLease::new();
    fake.execute(acquire(&project, "acquire")).expect("acquire");

    let active_snapshot = fake.export_snapshot(&project).expect("active snapshot");
    let active_checkpoint = fake
        .current_checkpoint(&project)
        .expect("active checkpoint")
        .expect("active checkpoint exists");
    let active = verify_snapshot_against_checkpoint(&active_snapshot, &active_checkpoint)
        .expect("active aggregate");
    let active_evidence =
        WriterLeaseProjectEvidence::from_verified_aggregate(&active).expect("active evidence");
    assert_eq!(active_evidence.project_id(), &project);
    assert_eq!(active_evidence.fencing_high_water(), 1);
    assert_eq!(active_evidence.transition_high_water(), 1);
    assert_eq!(active_evidence.command_high_water(), 1);
    assert!(active_evidence.current_authority().is_some());

    let release_command = release(&fake, &project, "release", RuntimeAdmissionMode::Active);
    fake.execute(release_command).expect("release");
    let released_snapshot = fake.export_snapshot(&project).expect("released snapshot");
    let released_checkpoint = fake
        .current_checkpoint(&project)
        .expect("released checkpoint")
        .expect("released checkpoint exists");
    let released = verify_snapshot_against_checkpoint(&released_snapshot, &released_checkpoint)
        .expect("released aggregate");
    let released_evidence =
        WriterLeaseProjectEvidence::from_verified_aggregate(&released).expect("released evidence");
    assert_eq!(released_evidence.project_id(), &project);
    assert_eq!(released_evidence.fencing_high_water(), 1);
    assert_eq!(released_evidence.transition_high_water(), 2);
    assert_eq!(released_evidence.command_high_water(), 2);
    assert!(released_evidence.current_authority().is_none());
}

#[test]
fn public_checkpoint_constructor_rebuilds_trusted_rows_and_rejects_invalid_shapes() {
    let project = project("project-checkpoint");
    let mut fake = FakeWriterLease::new();
    fake.execute(acquire(&project, "acquire")).expect("acquire");
    let expected = fake
        .current_checkpoint(&project)
        .expect("checkpoint")
        .expect("project checkpoint");
    let rebuilt = lattice_writer_lease::WriterLeaseCheckpoint::new(
        project.clone(),
        expected.command_high_water(),
        expected.command_tail_digest().cloned(),
        expected.snapshot_digest().clone(),
    )
    .expect("trusted row reconstruction");
    assert_eq!(rebuilt, expected);

    let tail = expected.command_tail_digest().cloned().expect("tail");
    let snapshot_digest = expected.snapshot_digest().clone();
    for invalid in [
        lattice_writer_lease::WriterLeaseCheckpoint::new(
            project.clone(),
            0,
            Some(tail.clone()),
            snapshot_digest.clone(),
        ),
        lattice_writer_lease::WriterLeaseCheckpoint::new(
            project.clone(),
            1,
            None,
            snapshot_digest.clone(),
        ),
        lattice_writer_lease::WriterLeaseCheckpoint::new(
            project.clone(),
            i64::MAX as u64 + 1,
            Some(tail.clone()),
            snapshot_digest.clone(),
        ),
        lattice_writer_lease::WriterLeaseCheckpoint::new(
            project.clone(),
            1,
            Some(digest('0')),
            snapshot_digest.clone(),
        ),
        lattice_writer_lease::WriterLeaseCheckpoint::new(
            project.clone(),
            1,
            Some(tail),
            digest('0'),
        ),
    ] {
        assert_eq!(invalid, Err(WriterLeaseError::CheckpointMismatch));
    }

    fake.execute(acquire(&project, "conflict"))
        .expect("terminal conflict");
    let snapshot = fake.export_snapshot(&project).expect("snapshot");
    let verified = verify_snapshot(&snapshot).expect("verified chain");
    let receipts = verified.command_receipts();
    assert_eq!(receipts.len(), 2);
    assert_eq!(
        receipts[1].previous_receipt_digest.as_ref(),
        Some(&receipts[0].receipt_digest)
    );

    let mut tampered = snapshot;
    let second = &mut array_field_mut(&mut tampered.payload, "commands")[1];
    set_string_field(second, "previous_receipt_digest", digest('9').as_str());
    assert_corrupt(&tampered);
}

#[test]
fn first_acquire_wins_conflict_denies_and_projects_are_isolated() {
    let project_a = project("project-a");
    let project_b = project("project-b");
    let mut fake = FakeWriterLease::new();

    let first = fake
        .execute(acquire(&project_a, "acquire-a"))
        .expect("first acquire");
    assert_eq!(first.outcome, CommandOutcome::Applied);
    assert_eq!(
        fake.current_receipt(&project_a)
            .expect("authority")
            .identity()
            .fencing_token()
            .get(),
        1
    );

    let conflict = fake
        .execute(acquire(&project_a, "acquire-conflict"))
        .expect("terminal conflict");
    assert_eq!(
        conflict.outcome,
        CommandOutcome::Denied(LeaseDenial::WriterAlreadyHeld)
    );
    assert_eq!(
        fake.current_receipt(&project_a)
            .expect("authority")
            .identity()
            .fencing_token()
            .get(),
        1
    );

    assert_eq!(
        fake.execute(acquire(&project_b, "acquire-b"))
            .expect("independent acquire")
            .outcome,
        CommandOutcome::Applied
    );
    assert_eq!(fake.project_count(), 2);
}

#[test]
fn exact_retry_after_advancement_is_identical_and_changed_content_rejects() {
    let project = project("project-retry");
    let mut fake = FakeWriterLease::new();
    let acquire_request = acquire(&project, "same-command");
    let original = fake.execute(acquire_request.clone()).expect("acquire");
    fake.execute(heartbeat(
        &fake,
        &project,
        "heartbeat",
        "2026-07-29T00:05:00Z",
        "2026-07-29T00:15:00Z",
        RuntimeAdmissionMode::Active,
    ))
    .expect("heartbeat");

    let retry = fake.execute(acquire_request.clone()).expect("exact retry");
    assert_eq!(retry, original);

    let mut changed = acquire_request.clone();
    let WriterLeaseCommand::Acquire(command) = &mut changed else {
        unreachable!()
    };
    command.expires_at = "2026-07-29T00:11:00Z".to_owned();
    assert_eq!(fake.execute(changed), Err(WriterLeaseError::CommandIdReuse));

    let mut malformed_changed = acquire_request;
    let WriterLeaseCommand::Acquire(command) = &mut malformed_changed else {
        unreachable!()
    };
    command.claim.task_spec_digest =
        ContentDigest::from_sha256("0".repeat(64)).expect("zero digest shape");
    assert_eq!(
        fake.execute(malformed_changed),
        Err(WriterLeaseError::CommandIdReuse),
        "changed-content reuse must win even when the changed request is malformed"
    );
}

#[test]
fn heartbeat_advances_revision_without_changing_identity_or_fence() {
    let project = project("project-heartbeat");
    let mut fake = FakeWriterLease::new();
    fake.execute(acquire(&project, "acquire")).expect("acquire");
    let before = fake.current_receipt(&project).expect("before");

    assert_eq!(
        fake.execute(heartbeat(
            &fake,
            &project,
            "heartbeat",
            "2026-07-29T00:05:00Z",
            "2026-07-29T00:15:00Z",
            RuntimeAdmissionMode::Active,
        ))
        .expect("heartbeat")
        .outcome,
        CommandOutcome::Applied
    );
    let after = fake.current_receipt(&project).expect("after");
    assert_eq!(after.identity(), before.identity());
    assert_eq!(after.revision().get(), before.revision().get() + 1);
    assert_eq!(after.heartbeat_at(), "2026-07-29T00:05:00Z");
}

#[test]
fn expiry_only_marks_suspect_and_suspect_cannot_heartbeat() {
    let project = project("project-suspect");
    let mut fake = FakeWriterLease::new();
    fake.execute(acquire(&project, "acquire")).expect("acquire");

    let early = fake
        .execute(suspect(
            &fake,
            &project,
            "early",
            "2026-07-29T00:09:00Z",
            RuntimeAdmissionMode::Draining,
        ))
        .expect("terminal early denial");
    assert_eq!(
        early.outcome,
        CommandOutcome::Denied(LeaseDenial::NotExpired)
    );

    assert_eq!(
        fake.execute(suspect(
            &fake,
            &project,
            "suspect",
            "2026-07-29T00:10:00Z",
            RuntimeAdmissionMode::Draining,
        ))
        .expect("mark suspect")
        .outcome,
        CommandOutcome::Applied
    );
    assert_eq!(
        fake.current_receipt(&project).expect("suspect").status(),
        WriterLeaseStatus::Suspect
    );

    let denied = fake
        .execute(heartbeat(
            &fake,
            &project,
            "revive",
            "2026-07-29T00:11:00Z",
            "2026-07-29T00:20:00Z",
            RuntimeAdmissionMode::Active,
        ))
        .expect("terminal state denial");
    assert_eq!(
        denied.outcome,
        CommandOutcome::Denied(LeaseDenial::InvalidState)
    );
}

#[test]
fn exact_release_then_reacquire_allocates_a_new_fence() {
    let project = project("project-release");
    let mut fake = FakeWriterLease::new();
    fake.execute(acquire(&project, "acquire")).expect("acquire");
    assert_eq!(
        fake.execute(release(
            &fake,
            &project,
            "release",
            RuntimeAdmissionMode::Draining,
        ))
        .expect("release")
        .outcome,
        CommandOutcome::Applied
    );
    assert!(fake.current_head(&project).is_none());

    assert_eq!(
        fake.execute(acquire(&project, "reacquire"))
            .expect("reacquire")
            .outcome,
        CommandOutcome::Applied
    );
    assert_eq!(
        fake.current_receipt(&project)
            .expect("authority")
            .identity()
            .fencing_token()
            .get(),
        2
    );
}

#[test]
fn revoke_requires_exact_process_start_digest() {
    let project = project("project-revoke");
    let mut fake = FakeWriterLease::new();
    fake.execute(acquire(&project, "acquire")).expect("acquire");
    fake.execute(suspect(
        &fake,
        &project,
        "suspect",
        "2026-07-29T00:10:00Z",
        RuntimeAdmissionMode::Draining,
    ))
    .expect("suspect");

    let mismatch = fake
        .execute(revoke(
            &fake,
            &project,
            "bad-revoke",
            process_death(digest('9')),
            RuntimeAdmissionMode::ReconciliationRequired,
        ))
        .expect("terminal mismatch");
    assert_eq!(
        mismatch.outcome,
        CommandOutcome::Denied(LeaseDenial::RecoveryEvidenceMismatch)
    );
    assert!(fake.current_head(&project).is_some());

    let exact_start = fake
        .current_receipt(&project)
        .expect("suspect")
        .identity()
        .holder_process_start_identity()
        .clone();
    assert_process_death_daemon_binding(&fake, &project, &exact_start);

    let wrong_daemon = fake
        .execute(revoke(
            &fake,
            &project,
            "wrong-daemon-revoke",
            RecoveryEvidence::ProcessDeath {
                holder_process_id: HolderProcessId::new(42).expect("pid"),
                holder_process_start_identity: exact_start.clone(),
                holder_daemon_instance_id: "daemon-substitute".to_owned(),
                evidence_digest: digest('8'),
            },
            RuntimeAdmissionMode::ReconciliationRequired,
        ))
        .expect("terminal daemon mismatch");
    assert_eq!(
        wrong_daemon.outcome,
        CommandOutcome::Denied(LeaseDenial::RecoveryEvidenceMismatch)
    );
    assert_eq!(
        fake.execute(revoke(
            &fake,
            &project,
            "revoke",
            process_death(exact_start),
            RuntimeAdmissionMode::ReconciliationRequired,
        ))
        .expect("revoke")
        .outcome,
        CommandOutcome::Applied
    );
    assert!(fake.current_head(&project).is_none());
}

#[test]
fn leadership_replacement_requires_exact_old_and_strictly_newer_epoch() {
    let project = project("project-leadership");
    let mut fake = FakeWriterLease::new();
    fake.execute(acquire(&project, "acquire")).expect("acquire");
    fake.execute(suspect(
        &fake,
        &project,
        "suspect",
        "2026-07-29T00:10:00Z",
        RuntimeAdmissionMode::Draining,
    ))
    .expect("suspect");

    let stale = RecoveryEvidence::LeadershipReplaced {
        replaced_daemon_instance_id: "daemon-1".to_owned(),
        replaced_epoch: DaemonEpoch::new(7).expect("old"),
        replacement_daemon_instance_id: "daemon-2".to_owned(),
        replacement_epoch: DaemonEpoch::new(7).expect("not newer"),
        evidence_digest: digest('8'),
    };
    assert_eq!(
        fake.execute(revoke(
            &fake,
            &project,
            "stale-replacement",
            stale,
            RuntimeAdmissionMode::ReconciliationRequired,
        ))
        .expect("terminal stale")
        .outcome,
        CommandOutcome::Denied(LeaseDenial::RecoveryEvidenceMismatch)
    );

    let newer = RecoveryEvidence::LeadershipReplaced {
        replaced_daemon_instance_id: "daemon-1".to_owned(),
        replaced_epoch: DaemonEpoch::new(7).expect("old"),
        replacement_daemon_instance_id: "daemon-2".to_owned(),
        replacement_epoch: DaemonEpoch::new(8).expect("new"),
        evidence_digest: digest('8'),
    };
    assert_eq!(
        fake.execute(revoke(
            &fake,
            &project,
            "new-replacement",
            newer,
            RuntimeAdmissionMode::ReconciliationRequired,
        ))
        .expect("revoke")
        .outcome,
        CommandOutcome::Applied
    );
}

#[test]
fn runtime_admission_matrix_fails_closed() {
    let project = project("project-admission");
    let mut fake = FakeWriterLease::new();
    let mut draining_acquire = acquire(&project, "draining-acquire");
    let WriterLeaseCommand::Acquire(command) = &mut draining_acquire else {
        unreachable!()
    };
    command.observation.admission = RuntimeAdmissionMode::Draining;
    assert_eq!(
        fake.execute(draining_acquire)
            .expect("terminal denial")
            .outcome,
        CommandOutcome::Denied(LeaseDenial::AdmissionDenied)
    );

    fake.execute(acquire(&project, "acquire")).expect("acquire");
    for (index, admission) in [
        RuntimeAdmissionMode::Draining,
        RuntimeAdmissionMode::Canary,
        RuntimeAdmissionMode::Stopped,
        RuntimeAdmissionMode::ReconciliationRequired,
    ]
    .into_iter()
    .enumerate()
    {
        let denied = fake
            .execute(heartbeat(
                &fake,
                &project,
                &format!("heartbeat-{index}"),
                "2026-07-29T00:05:00Z",
                "2026-07-29T00:15:00Z",
                admission,
            ))
            .expect("terminal admission denial");
        assert_eq!(
            denied.outcome,
            CommandOutcome::Denied(LeaseDenial::AdmissionDenied)
        );
    }

    let canary_release = fake
        .execute(release(
            &fake,
            &project,
            "canary-release",
            RuntimeAdmissionMode::Canary,
        ))
        .expect("terminal canary denial");
    assert_eq!(
        canary_release.outcome,
        CommandOutcome::Denied(LeaseDenial::AdmissionDenied)
    );
}

#[test]
fn snapshot_round_trip_rejects_tamper_reorder_truncation_and_counter_drift() {
    let project = project("project-snapshot");
    let mut fake = FakeWriterLease::new();
    fake.execute(acquire(&project, "acquire")).expect("acquire");
    fake.execute(heartbeat(
        &fake,
        &project,
        "heartbeat",
        "2026-07-29T00:05:00Z",
        "2026-07-29T00:15:00Z",
        RuntimeAdmissionMode::Active,
    ))
    .expect("heartbeat");
    let snapshot = fake.export_snapshot(&project).expect("snapshot");
    let checkpoint = fake
        .current_checkpoint(&project)
        .expect("checkpoint")
        .expect("project checkpoint");
    let verified = verify_snapshot(&snapshot).expect("verified");
    assert_eq!(verified.current_head(), fake.current_head(&project));

    let mut restored = FakeWriterLease::new();
    restored
        .restore_snapshot(&snapshot, &checkpoint)
        .expect("restore");
    assert_eq!(restored.current_head(&project), fake.current_head(&project));

    let mut tampered = snapshot.clone();
    set_string_field(
        &mut array_field_mut(&mut tampered.payload, "commands")[0],
        "request_digest",
        digest('9').as_str(),
    );
    assert_eq!(
        verify_snapshot(&tampered),
        Err(WriterLeaseError::CorruptSnapshot)
    );

    let mut reordered = snapshot.clone();
    array_field_mut(&mut reordered.payload, "commands").swap(0, 1);
    assert_eq!(
        verify_snapshot(&reordered),
        Err(WriterLeaseError::CorruptSnapshot)
    );

    let mut truncated = snapshot.clone();
    array_field_mut(&mut truncated.payload, "commands").pop();
    assert_eq!(
        verify_snapshot(&truncated),
        Err(WriterLeaseError::CorruptSnapshot)
    );

    let mut drift = snapshot;
    set_string_field(&mut drift.payload, "fencing_high_water", "2");
    assert_eq!(
        verify_snapshot(&drift),
        Err(WriterLeaseError::CorruptSnapshot)
    );
}

#[test]
fn denial_only_tail_truncation_cannot_erase_terminal_idempotency() {
    let project = project("project-denial-tail");
    let mut fake = FakeWriterLease::new();
    let mut denied_acquire = acquire(&project, "permanent-command");
    let WriterLeaseCommand::Acquire(command) = &mut denied_acquire else {
        unreachable!()
    };
    command.observation.admission = RuntimeAdmissionMode::Draining;
    assert_eq!(
        fake.execute(denied_acquire)
            .expect("terminal admission denial")
            .outcome,
        CommandOutcome::Denied(LeaseDenial::AdmissionDenied)
    );

    let snapshot = fake.export_snapshot(&project).expect("snapshot");
    let checkpoint = fake
        .current_checkpoint(&project)
        .expect("checkpoint")
        .expect("project checkpoint");
    assert_eq!(checkpoint.command_high_water(), 1);

    let mut truncated = snapshot.clone();
    array_field_mut(&mut truncated.payload, "commands").clear();
    assert_corrupt(&truncated);

    let mut restored = FakeWriterLease::new();
    assert_eq!(
        restored.restore_snapshot(&truncated, &checkpoint),
        Err(WriterLeaseError::CorruptSnapshot)
    );

    let mut coherent_prefix = truncated;
    set_string_field(&mut coherent_prefix.payload, "command_high_water", "0");
    *object_field_mut(&mut coherent_prefix.payload, "command_tail_digest") = CanonicalValue::Null;
    verify_snapshot(&coherent_prefix).expect("a context-free empty prefix is self-consistent");
    assert_eq!(
        verify_snapshot_against_checkpoint(&coherent_prefix, &checkpoint),
        Err(WriterLeaseError::CheckpointMismatch)
    );
    assert_eq!(
        restored.restore_snapshot(&coherent_prefix, &checkpoint),
        Err(WriterLeaseError::CheckpointMismatch)
    );

    restored
        .restore_snapshot(&snapshot, &checkpoint)
        .expect("trusted complete restore");
    let changed_content = acquire(&project, "permanent-command");
    assert_eq!(
        restored.execute(changed_content),
        Err(WriterLeaseError::CommandIdReuse),
        "a restored terminal denial must permanently reserve its command ID"
    );
}

#[test]
fn raw_snapshot_rejects_unknown_versions_kinds_outcomes_denials_and_values() {
    let project = project("project-raw");
    let mut fake = FakeWriterLease::new();
    fake.execute(acquire(&project, "acquire")).expect("acquire");
    let snapshot = fake.export_snapshot(&project).expect("snapshot");

    let mut unknown_snapshot_version = snapshot.clone();
    set_string_field(
        &mut unknown_snapshot_version.payload,
        "schema_version",
        "2.0",
    );
    assert_corrupt(&unknown_snapshot_version);

    let mut duplicate_field = snapshot.clone();
    let CanonicalValue::Object(fields) = &mut duplicate_field.payload else {
        unreachable!()
    };
    let duplicate_value = fields
        .iter()
        .find_map(|(name, value)| (name == "revision").then_some(value.clone()))
        .expect("revision");
    fields.push(("revision".to_owned(), duplicate_value));
    assert_corrupt(&duplicate_field);

    let mut missing_field = snapshot.clone();
    let CanonicalValue::Object(fields) = &mut missing_field.payload else {
        unreachable!()
    };
    fields.retain(|(name, _)| name != "revision");
    assert_corrupt(&missing_field);

    let mut unknown_request_version = snapshot.clone();
    let receipt = &mut array_field_mut(&mut unknown_request_version.payload, "commands")[0];
    set_string_field(
        object_field_mut(receipt, "request"),
        "schema_version",
        "2.0",
    );
    assert_corrupt(&unknown_request_version);

    let mut unknown_receipt_version = snapshot.clone();
    let receipt = &mut array_field_mut(&mut unknown_receipt_version.payload, "commands")[0];
    set_string_field(receipt, "schema_version", "2.0");
    assert_corrupt(&unknown_receipt_version);

    let mut unknown_claim_version = snapshot.clone();
    let receipt = &mut array_field_mut(&mut unknown_claim_version.payload, "commands")[0];
    let request = object_field_mut(receipt, "request");
    set_string_field(object_field_mut(request, "claim"), "schema_version", "2.0");
    assert_corrupt(&unknown_claim_version);

    let mut unknown_observation_version = snapshot.clone();
    let receipt = &mut array_field_mut(&mut unknown_observation_version.payload, "commands")[0];
    let request = object_field_mut(receipt, "request");
    set_string_field(
        object_field_mut(request, "observation"),
        "schema_version",
        "2.0",
    );
    assert_corrupt(&unknown_observation_version);

    let mut unknown_transition_version = snapshot.clone();
    let transition =
        &mut array_field_mut(&mut unknown_transition_version.payload, "transitions")[0];
    set_string_field(transition, "schema_version", "2.0");
    assert_corrupt(&unknown_transition_version);

    let mut unknown_transition_kind = snapshot.clone();
    let transition = &mut array_field_mut(&mut unknown_transition_kind.payload, "transitions")[0];
    set_string_field(transition, "kind", "UNRECOGNIZED");
    assert_corrupt(&unknown_transition_kind);

    let mut unknown_authority_version = snapshot.clone();
    set_string_field(
        object_field_mut(&mut unknown_authority_version.payload, "current_receipt"),
        "schema_version",
        "2.0",
    );
    assert_corrupt(&unknown_authority_version);

    let mut unknown_identity_version = snapshot.clone();
    let authority = object_field_mut(&mut unknown_identity_version.payload, "current_receipt");
    set_string_field(
        object_field_mut(authority, "identity"),
        "schema_version",
        "2.0",
    );
    assert_corrupt(&unknown_identity_version);

    let mut unknown_request_kind = snapshot.clone();
    let receipt = &mut array_field_mut(&mut unknown_request_kind.payload, "commands")[0];
    set_string_field(object_field_mut(receipt, "request"), "kind", "UNRECOGNIZED");
    assert_corrupt(&unknown_request_kind);
}

#[test]
fn raw_snapshot_rejects_unknown_outcomes_denials_and_malformed_values() {
    let project = project("project-raw-values");
    let mut fake = FakeWriterLease::new();
    fake.execute(acquire(&project, "acquire")).expect("acquire");
    let snapshot = fake.export_snapshot(&project).expect("snapshot");

    let mut unknown_outcome = snapshot.clone();
    let receipt = &mut array_field_mut(&mut unknown_outcome.payload, "commands")[0];
    set_string_field(receipt, "outcome", "UNRECOGNIZED");
    assert_corrupt(&unknown_outcome);

    let mut applied_with_denial = snapshot.clone();
    let receipt = &mut array_field_mut(&mut applied_with_denial.payload, "commands")[0];
    *object_field_mut(receipt, "denial_reason") =
        CanonicalValue::String("ADMISSION_DENIED".to_owned());
    assert_corrupt(&applied_with_denial);

    let mut denied_without_reason = snapshot.clone();
    let receipt = &mut array_field_mut(&mut denied_without_reason.payload, "commands")[0];
    set_string_field(receipt, "outcome", "DENIED");
    assert_corrupt(&denied_without_reason);

    let mut unknown_denial = snapshot.clone();
    let receipt = &mut array_field_mut(&mut unknown_denial.payload, "commands")[0];
    set_string_field(receipt, "outcome", "DENIED");
    *object_field_mut(receipt, "denial_reason") = CanonicalValue::String("UNRECOGNIZED".to_owned());
    assert_corrupt(&unknown_denial);

    let mut malformed_identifier = snapshot.clone();
    let receipt = &mut array_field_mut(&mut malformed_identifier.payload, "commands")[0];
    set_string_field(object_field_mut(receipt, "request"), "command_id", "");
    assert_corrupt(&malformed_identifier);

    let mut malformed_identity = snapshot.clone();
    let authority = object_field_mut(&mut malformed_identity.payload, "current_receipt");
    set_string_field(object_field_mut(authority, "identity"), "lease_id", "");
    assert_corrupt(&malformed_identity);

    let mut malformed_value = snapshot.clone();
    *object_field_mut(&mut malformed_value.payload, "revision") = CanonicalValue::Bool(false);
    assert_corrupt(&malformed_value);

    let mut malformed_ordinal = snapshot.clone();
    let receipt = &mut array_field_mut(&mut malformed_ordinal.payload, "commands")[0];
    set_string_field(receipt, "ordinal", "2");
    assert_corrupt(&malformed_ordinal);

    let mut tampered_receipt_digest = snapshot.clone();
    let receipt = &mut array_field_mut(&mut tampered_receipt_digest.payload, "commands")[0];
    set_string_field(receipt, "receipt_digest", digest('9').as_str());
    assert_corrupt(&tampered_receipt_digest);

    let mut tampered_transition_digest = snapshot.clone();
    let transition =
        &mut array_field_mut(&mut tampered_transition_digest.payload, "transitions")[0];
    set_string_field(transition, "transition_digest", digest('9').as_str());
    assert_corrupt(&tampered_transition_digest);

    let mut unknown_contract_version = snapshot.clone();
    let receipt = &mut array_field_mut(&mut unknown_contract_version.payload, "commands")[0];
    set_string_field(object_field_mut(receipt, "after"), "contract_version", "2");
    assert_corrupt(&unknown_contract_version);
}

#[test]
fn raw_snapshot_rejects_unknown_recovery_versions_and_kinds() {
    let project = project("project-raw-recovery");
    let mut fake = FakeWriterLease::new();
    fake.execute(acquire(&project, "acquire")).expect("acquire");
    let exact_start = fake
        .current_receipt(&project)
        .expect("active")
        .identity()
        .holder_process_start_identity()
        .clone();
    fake.execute(suspect(
        &fake,
        &project,
        "suspect-for-raw",
        "2026-07-29T00:10:00Z",
        RuntimeAdmissionMode::Draining,
    ))
    .expect("suspect");
    fake.execute(revoke(
        &fake,
        &project,
        "revoke-for-raw",
        process_death(exact_start),
        RuntimeAdmissionMode::ReconciliationRequired,
    ))
    .expect("revoke");
    let recovery_snapshot = fake.export_snapshot(&project).expect("recovery snapshot");
    let last_index = array_field(&recovery_snapshot.payload, "commands").len() - 1;

    let mut unknown_recovery_version = recovery_snapshot.clone();
    let receipt =
        &mut array_field_mut(&mut unknown_recovery_version.payload, "commands")[last_index];
    let request = object_field_mut(receipt, "request");
    set_string_field(
        object_field_mut(request, "evidence"),
        "schema_version",
        "2.0",
    );
    assert_corrupt(&unknown_recovery_version);

    let mut unknown_recovery_kind = recovery_snapshot;
    let receipt = &mut array_field_mut(&mut unknown_recovery_kind.payload, "commands")[last_index];
    let request = object_field_mut(receipt, "request");
    set_string_field(
        object_field_mut(request, "evidence"),
        "kind",
        "UNRECOGNIZED",
    );
    assert_corrupt(&unknown_recovery_kind);
}

#[test]
fn stale_plan_cannot_apply_after_another_transition() {
    let project = project("project-plan");
    let empty = lattice_writer_lease::VerifiedWriterLeaseAggregate::vacant(project.clone());
    let plan = plan_command(&empty, &acquire(&project, "acquire")).expect("plan");
    let advanced = apply_plan(
        &empty,
        plan_command(&empty, &acquire(&project, "other")).expect("other plan"),
    )
    .expect("advance");
    assert_eq!(
        apply_plan(&advanced, plan),
        Err(WriterLeaseError::PlanPreconditionChanged)
    );
}

#[test]
fn fake_never_issues_live_authority_and_timestamp_is_exact_utc() {
    let project = project("project-runtime");
    let mut fake = FakeWriterLease::new();
    let mut live = acquire(&project, "live");
    let WriterLeaseCommand::Acquire(command) = &mut live else {
        unreachable!()
    };
    command.observation.runtime = RuntimeKind::Live;
    assert_eq!(
        fake.execute(live),
        Err(WriterLeaseError::FakeRuntimeRequired)
    );

    let mut offset = acquire(&project, "offset");
    let WriterLeaseCommand::Acquire(command) = &mut offset else {
        unreachable!()
    };
    command.observation.observed_at = "2026-07-29T08:00:00+08:00".to_owned();
    assert_eq!(
        fake.execute(offset),
        Err(WriterLeaseError::InvalidTimestamp)
    );
}

#[test]
fn pure_planner_rejects_fake_live_substitution_within_one_lease() {
    let project = project("project-runtime-switch");
    let mut fake = FakeWriterLease::new();
    fake.execute(acquire(&project, "acquire")).expect("acquire");
    let snapshot = fake.export_snapshot(&project).expect("snapshot");
    let aggregate = verify_snapshot(&snapshot).expect("verified");
    let mut command = heartbeat(
        &fake,
        &project,
        "live-heartbeat",
        "2026-07-29T00:05:00Z",
        "2026-07-29T00:15:00Z",
        RuntimeAdmissionMode::Active,
    );
    let WriterLeaseCommand::Heartbeat(heartbeat_request) = &mut command else {
        unreachable!()
    };
    heartbeat_request.observation.runtime = RuntimeKind::Live;

    let plan = plan_command(&aggregate, &command).expect("terminal runtime denial");
    assert_eq!(
        plan.receipt().outcome,
        CommandOutcome::Denied(LeaseDenial::RuntimeMismatch)
    );
}

#[test]
fn restore_cannot_overwrite_a_newer_existing_aggregate() {
    let project = project("project-restore-rollback");
    let mut fake = FakeWriterLease::new();
    fake.execute(acquire(&project, "first"))
        .expect("first acquire");
    let old = fake.export_snapshot(&project).expect("old snapshot");
    let old_checkpoint = fake
        .current_checkpoint(&project)
        .expect("checkpoint")
        .expect("old checkpoint");
    fake.execute(release(
        &fake,
        &project,
        "release-first",
        RuntimeAdmissionMode::Active,
    ))
    .expect("release");
    fake.execute(acquire(&project, "second"))
        .expect("second acquire");

    assert!(
        fake.restore_snapshot(&old, &old_checkpoint).is_err(),
        "a valid historical prefix must not replace newer in-memory authority"
    );
    assert_eq!(
        fake.current_head(&project)
            .expect("current")
            .identity()
            .fencing_token()
            .get(),
        2
    );
}

#[test]
fn fake_restore_rejects_live_authority_history() {
    let project = project("project-live-restore");
    let empty = lattice_writer_lease::VerifiedWriterLeaseAggregate::vacant(project.clone());
    let mut live = acquire(&project, "live-acquire");
    let WriterLeaseCommand::Acquire(command) = &mut live else {
        unreachable!()
    };
    command.observation.runtime = RuntimeKind::Live;
    let plan = plan_command(&empty, &live).expect("live plan");
    let live_aggregate = apply_plan(&empty, plan).expect("live apply");
    let live_checkpoint = live_aggregate.checkpoint().expect("live checkpoint");

    let mut fake = FakeWriterLease::new();
    assert_eq!(
        fake.restore_snapshot(&live_aggregate.export_untrusted(), &live_checkpoint),
        Err(WriterLeaseError::FakeRuntimeRequired)
    );
    assert!(fake.current_head(&project).is_none());

    let fake_aggregate = apply_plan(
        &empty,
        plan_command(&empty, &acquire(&project, "fake-acquire")).expect("fake plan"),
    )
    .expect("fake aggregate");
    let mixed_request = WriterLeaseCommand::Heartbeat(HeartbeatCommand {
        command_id: "mixed-stale-head".to_owned(),
        project_id: project.clone(),
        expected_head: live_aggregate.current_head().expect("live head"),
        observation: observation(RuntimeAdmissionMode::Active, "2026-07-29T00:01:00Z"),
        expires_at: "2026-07-29T00:11:00Z".to_owned(),
    });
    let mixed_aggregate = apply_plan(
        &fake_aggregate,
        plan_command(&fake_aggregate, &mixed_request).expect("mixed terminal plan"),
    )
    .expect("mixed aggregate");
    let mixed_checkpoint = mixed_aggregate.checkpoint().expect("mixed checkpoint");
    assert_eq!(
        fake.restore_snapshot(&mixed_aggregate.export_untrusted(), &mixed_checkpoint),
        Err(WriterLeaseError::FakeRuntimeRequired)
    );
}

#[test]
fn heartbeat_must_strictly_advance_the_previous_expiry() {
    let project = project("project-expiry-regression");
    let mut fake = FakeWriterLease::new();
    fake.execute(acquire(&project, "acquire")).expect("acquire");

    let receipt = fake
        .execute(heartbeat(
            &fake,
            &project,
            "shorter-expiry",
            "2026-07-29T00:01:00Z",
            "2026-07-29T00:02:00Z",
            RuntimeAdmissionMode::Active,
        ))
        .expect("terminal denial");
    assert_eq!(
        receipt.outcome,
        CommandOutcome::Denied(LeaseDenial::HeartbeatRejected)
    );
    assert_eq!(
        fake.current_head(&project).expect("head").expires_at(),
        "2026-07-29T00:10:00Z"
    );
}

#[test]
fn mark_suspect_rejects_reconciliation_required_admission() {
    let project = project("project-suspect-admission");
    let mut fake = FakeWriterLease::new();
    fake.execute(acquire(&project, "acquire")).expect("acquire");

    let receipt = fake
        .execute(suspect(
            &fake,
            &project,
            "suspect-reconciliation",
            "2026-07-29T00:10:00Z",
            RuntimeAdmissionMode::ReconciliationRequired,
        ))
        .expect("terminal denial");
    assert_eq!(
        receipt.outcome,
        CommandOutcome::Denied(LeaseDenial::AdmissionDenied)
    );
}
