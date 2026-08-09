use lattice_workspace_git::{
    ChangeManifest, ChangeOperation, ChangeRecord, ChangeState, PathEvidence, PathKind,
    WorkspaceError,
};

fn commit(character: char) -> String {
    character.to_string().repeat(40)
}

#[test]
fn normalized_contract_sorts_changes_and_hashes_stably() {
    let rename = ChangeRecord::new(
        ChangeOperation::Rename,
        Some(PathEvidence::new("src/old.rs", PathKind::Regular).expect("source")),
        Some(PathEvidence::new("src/new.rs", PathKind::Regular).expect("destination")),
        vec![
            ChangeState::Staged,
            ChangeState::Committed,
            ChangeState::Staged,
        ],
        "R100",
    )
    .expect("rename record");
    let create = ChangeRecord::new(
        ChangeOperation::Create,
        None,
        Some(PathEvidence::new("README.md", PathKind::Regular).expect("destination")),
        vec![ChangeState::Untracked],
        "??",
    )
    .expect("create record");

    let first = ChangeManifest::from_normalized(
        &commit('a'),
        &commit('b'),
        vec![rename.clone(), create.clone()],
    )
    .expect("normalized manifest");
    let second = ChangeManifest::from_normalized(&commit('a'), &commit('b'), vec![create, rename])
        .expect("order-independent manifest");

    assert_eq!(first, second);
    assert_eq!(
        first.changes()[1].states(),
        &[ChangeState::Committed, ChangeState::Staged]
    );
}

#[test]
fn normalized_contract_rejects_escape_and_invalid_rename_shape() {
    assert_eq!(
        PathEvidence::new("../escape.rs", PathKind::Regular),
        Err(WorkspaceError::InvalidChangeEvidence)
    );
    let same = PathEvidence::new("src/same.rs", PathKind::Regular).expect("same path");
    assert_eq!(
        ChangeRecord::new(
            ChangeOperation::Rename,
            Some(same.clone()),
            Some(same),
            vec![ChangeState::Staged],
            "R100",
        ),
        Err(WorkspaceError::InvalidChangeEvidence)
    );
}
