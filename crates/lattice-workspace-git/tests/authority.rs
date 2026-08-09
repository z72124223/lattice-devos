use lattice_contracts::{
    AttemptId, ContentDigest, DaemonEpoch, GitRefIdentity, HolderProcessId, ProjectClass,
    ProjectId, RuntimeAdmissionMode, RuntimeKind, TaskId,
};
use lattice_project_registry::{
    CommandId, FakeProjectRegistry, RegistryCommand, RepositoryObservation,
};
use lattice_workspace_git::{WorkspaceAuthority, WorkspaceError};
use lattice_writer_lease::test_support::{digest, observation as lease_observation};
use lattice_writer_lease::{
    AcquireClaim, AcquireCommand, FakeWriterLease, HeartbeatCommand, WriterLeaseCommand,
};

struct AuthorityFixture {
    registry_observation: RepositoryObservation,
    registry_receipt: lattice_contracts::ProjectAuthorityReceipt,
    registry_head: lattice_contracts::ProjectAuthorityHead,
    lease_receipt: lattice_contracts::WriterLeaseAuthorityReceipt,
    lease_head: lattice_contracts::WriterLeaseAuthorityHead,
    project_id: ProjectId,
    lease_owner: FakeWriterLease,
}

fn owner_authority(root: &str) -> AuthorityFixture {
    let project_id = ProjectId::new("project-a").expect("project id");
    let registry_observation = RepositoryObservation::new(
        root,
        digest('a'),
        digest('b'),
        digest('c'),
        GitRefIdentity::new("refs/heads/main", digest('d')).expect("primary ref"),
    )
    .expect("repository observation");
    let mut registry = FakeProjectRegistry::new();
    registry
        .execute(RegistryCommand::register(
            CommandId::new("register-project-a").expect("command id"),
            project_id.clone(),
            ProjectClass::UserProject,
            registry_observation.clone(),
        ))
        .expect("register project");
    let registry_receipt = registry
        .latest(&project_id)
        .expect("registry receipt")
        .clone();
    let registry_head = registry
        .current_head(&project_id)
        .expect("registry current head");

    let mut lease_owner = FakeWriterLease::new();
    lease_owner
        .execute(WriterLeaseCommand::Acquire(AcquireCommand {
            command_id: "acquire-project-a".to_owned(),
            expected_head: None,
            claim: AcquireClaim {
                project_id: project_id.clone(),
                project_snapshot_id: registry_receipt.project_snapshot_id().clone(),
                task_id: TaskId::new("TASK-026").expect("task id"),
                task_revision: "1".to_owned(),
                task_spec_digest: ContentDigest::from_sha256("1".repeat(64))
                    .expect("task spec digest"),
                attempt_id: AttemptId::new("attempt-026").expect("attempt id"),
                lease_id: "lease-026".to_owned(),
                lease_holder_id: "implementer-026".to_owned(),
                worktree_id: "worktree-026".to_owned(),
                holder_process_id: HolderProcessId::new(42).expect("process id"),
                holder_process_start_identity: digest('2'),
                daemon_instance_id: "daemon-026".to_owned(),
                daemon_epoch: DaemonEpoch::new(7).expect("daemon epoch"),
            },
            observation: lease_observation(RuntimeAdmissionMode::Active, "2026-08-09T00:00:00Z"),
            expires_at: "2026-08-09T00:10:00Z".to_owned(),
        }))
        .expect("acquire writer lease");
    let lease_receipt = lease_owner
        .current_receipt(&project_id)
        .expect("lease receipt");
    let lease_head = lease_owner
        .current_head(&project_id)
        .expect("lease current head");

    AuthorityFixture {
        registry_observation,
        registry_receipt,
        registry_head,
        lease_receipt,
        lease_head,
        project_id,
        lease_owner,
    }
}

#[test]
fn binds_only_current_formal_registry_and_writer_lease_authority() {
    let fixture = owner_authority(r"C:\disposable\project-a");
    let authority = WorkspaceAuthority::bind(
        &fixture.registry_observation,
        fixture.registry_receipt.clone(),
        &fixture.registry_head,
        fixture.lease_receipt.clone(),
        &fixture.lease_head,
    )
    .expect("matching formal authority binds");

    assert_eq!(authority.runtime(), RuntimeKind::Fake);
    assert_eq!(authority.project_id(), &fixture.project_id);
    assert_eq!(authority.fencing_token().get(), 1);
    assert_eq!(authority.worktree_id(), "worktree-026");
}

#[test]
fn stale_lease_receipt_is_rejected_after_owner_head_advances() {
    let mut fixture = owner_authority(r"C:\disposable\project-a");
    fixture
        .lease_owner
        .execute(WriterLeaseCommand::Heartbeat(HeartbeatCommand {
            command_id: "heartbeat-project-a".to_owned(),
            project_id: fixture.project_id.clone(),
            expected_head: fixture.lease_head.clone(),
            observation: lease_observation(RuntimeAdmissionMode::Active, "2026-08-09T00:05:00Z"),
            expires_at: "2026-08-09T00:15:00Z".to_owned(),
        }))
        .expect("advance owner head");
    let current_head = fixture
        .lease_owner
        .current_head(&fixture.project_id)
        .expect("advanced head");

    let error = WorkspaceAuthority::bind(
        &fixture.registry_observation,
        fixture.registry_receipt,
        &fixture.registry_head,
        fixture.lease_receipt,
        &current_head,
    )
    .expect_err("historical lease receipt cannot bind");

    assert_eq!(error, WorkspaceError::LeaseHeadMismatch);
}
