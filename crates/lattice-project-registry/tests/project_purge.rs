use lattice_contracts::{ContentDigest, GitRefIdentity, ProjectClass, ProjectId, RuntimeKind};
use lattice_project_registry::{
    CommandId, RegistryCommand, RegistryCommandOutcome, RegistryCommandRecord, RegistryDenial,
    RegistryError, RepositoryObservation, UntrustedRegistrySnapshot, VerifiedRegistryState,
    apply_command_plan, export_untrusted_registry_snapshot, plan_command, project_purge_prefix,
    verify_untrusted_registry_snapshot,
};

fn add(state: &VerifiedRegistryState, id: &str, byte: char) -> VerifiedRegistryState {
    let digest = ContentDigest::from_sha256(byte.to_string().repeat(64)).unwrap();
    let observation = RepositoryObservation::new(
        format!("C:/fixture/{id}"),
        digest.clone(),
        digest.clone(),
        digest.clone(),
        GitRefIdentity::new("refs/heads/main", digest).unwrap(),
    )
    .unwrap();
    let command = RegistryCommand::register(
        CommandId::new(format!("register-{id}")).unwrap(),
        ProjectId::new(id).unwrap(),
        ProjectClass::UserProject,
        observation,
    );
    apply_command_plan(state, &plan_command(state, command).unwrap())
        .unwrap()
        .state()
        .clone()
}

#[test]
fn purge_suffix_restores_exact_historical_state_and_replay() {
    let empty = VerifiedRegistryState::vacant(RuntimeKind::Live).unwrap();
    let survivor = add(&empty, "survivor", '1');
    let both = add(&survivor, "target", '2');
    let purged = project_purge_prefix(&both, &ProjectId::new("target").unwrap()).unwrap();
    assert_eq!(purged, survivor);
    assert_eq!(
        verify_untrusted_registry_snapshot(&export_untrusted_registry_snapshot(&purged)).unwrap(),
        survivor
    );
    assert!(project_purge_prefix(&both, &ProjectId::new("survivor").unwrap()).is_err());
    assert!(project_purge_prefix(&both, &ProjectId::new("unknown").unwrap()).is_err());
    assert_eq!(
        project_purge_prefix(&survivor, &ProjectId::new("survivor").unwrap()).unwrap(),
        empty
    );
}

#[test]
fn interleaved_target_survivor_target_history_cannot_reuse_survivor_records() {
    let empty = VerifiedRegistryState::vacant(RuntimeKind::Live).unwrap();
    let target_id = ProjectId::new("target").unwrap();
    let target = add(&empty, "target", '1');
    let both = add(&target, "survivor", '2');
    let command = RegistryCommand::suspend(
        CommandId::new("suspend-target").unwrap(),
        target_id.clone(),
        both.project(&target_id).unwrap().authority().head(),
        ContentDigest::from_sha256("3".repeat(64)).unwrap(),
    );
    let interleaved = apply_command_plan(&both, &plan_command(&both, command).unwrap())
        .unwrap()
        .state()
        .clone();
    let original = export_untrusted_registry_snapshot(&interleaved);
    assert_eq!(
        project_purge_prefix(&interleaved, &target_id),
        Err(RegistryError::CorruptSnapshot)
    );
    assert_eq!(export_untrusted_registry_snapshot(&interleaved), original);

    let survivor = &original.commands()[1];
    let fresh = add(&empty, "survivor", '2');
    let fresh_snapshot = export_untrusted_registry_snapshot(&fresh);
    // Closing the ordinal gap cannot repair the old global checkpoint chain.
    let renumbered = RegistryCommandRecord::from_retained(
        1,
        survivor.command().clone(),
        survivor.receipt().clone(),
        survivor.base_checkpoint().clone(),
        survivor.result_checkpoint().clone(),
        survivor.record_set_digest().clone(),
    );
    let filtered = UntrustedRegistrySnapshot::from_retained(
        fresh.checkpoint().clone(),
        fresh_snapshot.observations().to_vec(),
        fresh_snapshot.projects().to_vec(),
        vec![renumbered],
        fresh_snapshot.reservations().to_vec(),
    );
    assert_eq!(
        verify_untrusted_registry_snapshot(&filtered),
        Err(RegistryError::CorruptSnapshot)
    );
}

#[test]
fn survivor_denial_retains_target_identity_and_changes_if_target_is_removed() {
    let empty = VerifiedRegistryState::vacant(RuntimeKind::Live).unwrap();
    let target_id = ProjectId::new("target").unwrap();
    let target = add(&empty, "target", '1');
    let duplicate = RegistryCommand::register(
        CommandId::new("register-survivor-duplicate").unwrap(),
        ProjectId::new("survivor").unwrap(),
        ProjectClass::UserProject,
        target.project(&target_id).unwrap().observation().clone(),
    );
    let denied_plan = plan_command(&target, duplicate.clone()).unwrap();
    assert!(matches!(
        denied_plan.receipt().outcome(),
        RegistryCommandOutcome::Denied(RegistryDenial::DuplicateIdentity {
            existing_project_id, ..
        }) if existing_project_id == target_id
    ));
    let retained = apply_command_plan(&target, &denied_plan)
        .unwrap()
        .state()
        .clone();
    let original = export_untrusted_registry_snapshot(&retained);
    assert_eq!(
        project_purge_prefix(&retained, &target_id),
        Err(RegistryError::CorruptSnapshot)
    );
    let without_target = plan_command(&empty, duplicate).unwrap();
    assert_eq!(
        without_target.receipt().outcome(),
        RegistryCommandOutcome::Applied
    );
    assert_ne!(without_target.receipt(), denied_plan.receipt());
    assert_eq!(export_untrusted_registry_snapshot(&retained), original);
    assert_eq!(
        verify_untrusted_registry_snapshot(&original).unwrap(),
        retained
    );
}
