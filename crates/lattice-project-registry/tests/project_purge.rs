use lattice_contracts::{ContentDigest, GitRefIdentity, ProjectClass, ProjectId, RuntimeKind};
use lattice_project_registry::{
    CommandId, RegistryCommand, RepositoryObservation, VerifiedRegistryState, apply_command_plan,
    export_untrusted_registry_snapshot, plan_command, project_purge_prefix,
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
