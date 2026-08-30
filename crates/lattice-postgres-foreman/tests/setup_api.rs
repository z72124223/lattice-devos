use lattice_postgres_foreman::{ExtensionDatabaseRole, ExtensionSetupErrorKind, ExtensionTarget};

#[test]
fn target_derives_one_exact_database_identity() {
    let target =
        ExtensionTarget::new("lattice_foreman_acceptance", "run-20260826").expect("bounded target");
    assert_eq!(target.database_name(), "lattice_foreman_acceptance");
    assert_eq!(target.expected_database_uuid().len(), 36);
    assert_eq!(target.expected_database_uuid().as_bytes()[14], b'8');
    assert_eq!(
        target.expected_database_identity_digest().as_str().len(),
        64
    );
}

#[test]
fn target_rejects_ambient_or_path_like_input() {
    for (database, run) in [
        ("postgres", "run-20260826"),
        ("LATTICE", "run-20260826"),
        ("lattice_ok", "../run"),
        ("lattice-ok", "run-20260826"),
    ] {
        let error = ExtensionTarget::new(database, run).expect_err("closed target");
        assert_eq!(error.kind(), ExtensionSetupErrorKind::InvalidTarget);
    }
}

#[test]
fn database_roles_are_fixed() {
    assert_eq!(
        ExtensionDatabaseRole::Migrator.session_role(),
        "lattice_migrator_login"
    );
    assert_eq!(
        ExtensionDatabaseRole::Migrator.current_role(),
        "lattice_migrator"
    );
    assert_eq!(
        ExtensionDatabaseRole::Runtime.session_role(),
        "lattice_runtime_login"
    );
    assert_eq!(
        ExtensionDatabaseRole::Runtime.current_role(),
        "lattice_runtime"
    );
}
