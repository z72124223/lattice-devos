use lattice_postgres_codebase_memory::{
    ExtensionApplyOutcome, ExtensionBootstrapGlobalProfile, ExtensionBootstrapProfile,
    ExtensionCatalogEvidence, ExtensionDatabaseRole, ExtensionSetupError, ExtensionTarget,
    apply_extension, inspect_bootstrap_profile, verify_extension,
};
use postgres::Client;

#[test]
fn explicit_admin_and_closed_verifier_api_are_the_only_setup_surface() {
    let _: fn(&mut Client, &ExtensionTarget) -> Result<ExtensionApplyOutcome, ExtensionSetupError> =
        apply_extension;
    let _: fn(
        &mut Client,
        &ExtensionTarget,
        ExtensionDatabaseRole,
    ) -> Result<ExtensionCatalogEvidence, ExtensionSetupError> = verify_extension;
    let _: fn(
        &mut Client,
        &ExtensionTarget,
        ExtensionBootstrapGlobalProfile,
    ) -> Result<ExtensionBootstrapProfile, ExtensionSetupError> = inspect_bootstrap_profile;

    let run_id = "1".repeat(32);
    let target =
        ExtensionTarget::new("lattice_task019_12345678_base", run_id).expect("marker-owned target");
    assert_eq!(target.database_name(), "lattice_task019_12345678_base");
    assert_eq!(target.expected_database_uuid().len(), 36);
    assert_eq!(
        target.expected_database_identity_digest().as_str().len(),
        64
    );
    assert!(ExtensionTarget::new("postgres", "1".repeat(32)).is_err());
    assert!(ExtensionTarget::new("lattice_task019_12345678_base", "not-a-run-id").is_err());
}

#[test]
fn bootstrap_inspector_is_read_only_memory_owned_and_writer_agnostic() {
    let source = include_str!("../src/setup.rs");
    let inspector = source
        .split_once("pub fn inspect_bootstrap_profile")
        .expect("bootstrap inspector")
        .1
        .split_once("pub fn verify_extension")
        .expect("bootstrap inspector boundary")
        .0;
    for required in [
        "IsolationLevel::RepeatableRead",
        ".read_only(true)",
        "preflight_bootstrap",
        "verify_global_default_acl_closure",
        "verify_empty_catalog_closure",
        "verify_v2_source",
        "verify_exact_catalog_profile",
        "verify_catalog_closure",
        "read_identity",
    ] {
        assert!(inspector.contains(required), "missing {required}");
    }
    assert!(!inspector.contains("classify_writer_lease_companion"));
    assert!(!inspector.contains("verify_writer_lease_companion"));
}

#[test]
fn bootstrap_inspector_accepts_only_the_five_frozen_store_profiles() {
    let source = include_str!("../src/setup.rs");
    let profiles = [
        ExtensionBootstrapGlobalProfile::V5,
        ExtensionBootstrapGlobalProfile::V6,
        ExtensionBootstrapGlobalProfile::V7,
        ExtensionBootstrapGlobalProfile::V8LegacyPrefix,
        ExtensionBootstrapGlobalProfile::V8,
    ];
    assert_eq!(format!("{:?}", profiles[2]), "V7");
    for required in [
        "const BOOTSTRAP_V7_GLOBAL_SCHEMA_VERSION: u16 = 7",
        "584a446464ab2f7ebd8b85543ba36a6d52b0a708502c39d2653b8814d84313f8",
        "LATTICE_DEVOS_MEMORY_SCHEMA_V7",
        "BOOTSTRAP_V7_GLOBAL_SCHEMA_VERSION",
        "BOOTSTRAP_V7_GLOBAL_MANIFEST_SHA256",
    ] {
        assert!(
            source.contains(required),
            "missing exact Store-v7 profile field {required}"
        );
    }
}

#[test]
fn empty_bootstrap_profile_closes_auxiliary_catalog_and_default_acl() {
    let source = include_str!("../src/setup.rs");
    let empty = source
        .split_once("fn verify_empty_catalog_closure")
        .expect("empty catalog verifier")
        .1
        .split_once("fn verify_exact_catalog_profile")
        .expect("empty catalog verifier boundary")
        .0;
    for required in [
        "pg_catalog.pg_opclass",
        "pg_catalog.pg_opfamily",
        "pg_catalog.pg_statistic_ext",
        "pg_catalog.pg_ts_config",
        "pg_catalog.pg_ts_dict",
        "pg_catalog.pg_ts_parser",
        "pg_catalog.pg_ts_template",
        "pg_catalog.pg_seclabel",
        "pg_catalog.obj_description",
        "pg_catalog.pg_default_acl",
        "pg_catalog.aclexplode",
    ] {
        assert!(empty.contains(required), "missing empty closure {required}");
    }
    let global_default_acl = empty
        .split_once("fn verify_global_default_acl_closure")
        .expect("global default ACL verifier")
        .1
        .split_once("fn verify_namespace_auxiliary_closure")
        .expect("global default ACL verifier boundary")
        .0;
    for required in [
        "SELECT (SELECT pg_catalog.count(*)",
        "FROM pg_catalog.pg_default_acl x",
        "WHERE x.defaclnamespace=0",
        "default_rows != 2",
        "default_acl_count != 2",
        "d.defaclobjtype='f' AND a.privilege_type='EXECUTE'",
        "d.defaclobjtype='T' AND a.privilege_type='USAGE'",
        "owner.rolname='lattice_migrator'",
        "grantee.rolname='lattice_migrator'",
        "grantor.rolname='lattice_migrator'",
        "NOT a.is_grantable",
    ] {
        assert!(
            global_default_acl.contains(required),
            "missing frozen global default ACL evidence {required}"
        );
    }
    assert!(!global_default_acl.contains("d.defaclobjtype='r'"));
    assert!(!global_default_acl.contains("d.defaclobjtype='S'"));
    assert!(!global_default_acl.contains("count(DISTINCT d.oid)"));
}

#[test]
fn memory_runner_uses_global_then_extension_lock_order() {
    let source = include_str!("../src/setup.rs");
    let helper_start = source
        .find("fn acquire_writer_companion_advisory_locks")
        .expect("advisory lock helper");
    let helper = &source[helper_start..];
    let global = helper
        .find("GLOBAL_MIGRATION_ADVISORY_LOCK")
        .expect("global migration lock");
    let extension = helper
        .find("EXTENSION_ADVISORY_LOCK")
        .expect("memory extension lock");
    assert!(global < extension);

    let apply_start = source
        .find("pub fn apply_extension")
        .expect("apply function");
    let apply = &source[apply_start..];
    let locks = apply
        .find("acquire_writer_companion_advisory_locks(&mut transaction)")
        .expect("advisory locks");
    let classify = apply
        .find("let pre_state = classify_pre_state")
        .expect("classification");
    assert!(locks < classify);
}

#[test]
fn writer_lease_v2_companion_is_locked_and_verified_without_adapter_dependency() {
    let source = include_str!("../src/setup.rs");
    let helper_start = source
        .find("fn acquire_writer_companion_advisory_locks")
        .expect("advisory lock helper");
    let helper = &source[helper_start..];
    let global = helper
        .find("GLOBAL_MIGRATION_ADVISORY_LOCK")
        .expect("global migration lock");
    let memory = helper
        .find("EXTENSION_ADVISORY_LOCK")
        .expect("memory extension lock");
    let writer = helper
        .find("WRITER_LEASE_ADVISORY_LOCK")
        .expect("writer lease extension lock");
    assert!(global < memory && memory < writer);

    let apply_start = source
        .find("pub fn apply_extension")
        .expect("apply function");
    let apply = &source[apply_start..];
    let locks = apply
        .find("acquire_writer_companion_advisory_locks(&mut transaction)")
        .expect("advisory locks");
    let classify = apply
        .find("let pre_state = classify_pre_state")
        .expect("memory classification");
    assert!(locks < classify);

    for required in [
        "const WRITER_LEASE_ADVISORY_LOCK: i64 = 0x4c41_5457_4c45_4131",
        "enum WriterLeaseCompanionProfile",
        "ExactV2Bridge",
        "ExactV2Current",
        "lock_writer_lease_tables",
        "verify_writer_lease_v2_bridge",
        "verify_writer_lease_v2_current",
        "LOCK TABLE writer_lease.writer_lease_commands IN SHARE MODE",
        "LOCK TABLE writer_lease.writer_lease_extension_identity IN SHARE MODE",
        "LOCK TABLE writer_lease.writer_lease_extension_ledger IN SHARE MODE",
        "LOCK TABLE writer_lease.writer_lease_heads IN SHARE MODE",
        "LOCK TABLE writer_lease.writer_lease_transitions IN SHARE MODE",
        "WRITER_LEASE_EXPECTED_FUNCTIONS: [&str; 9]",
        "WRITER_LEASE_CURRENT_RUNTIME_FUNCTIONS: [&str; 7]",
        "UPGRADED",
        "REBOUND",
    ] {
        assert!(
            source.contains(required),
            "missing Writer companion field {required}"
        );
    }
    assert!(
        !source.contains("MEMORY_EXTENSION_WRITER_LEASE_PROFILE_UNSUPPORTED"),
        "the blanket Writer Lease rejection must be removed"
    );
    assert!(
        !source.contains("lattice_postgres_writer_lease"),
        "Memory must not depend on the Writer adapter"
    );
}

#[test]
fn writer_bridge_quarantine_and_current_runtime_acl_are_distinct_exact_profiles() {
    let source = include_str!("../src/setup.rs");

    for required in [
        "enum WriterLeaseAclProfile",
        "BridgeQuarantined",
        "CurrentRuntime",
        "WRITER_LEASE_BRIDGE_RUNTIME_FUNCTIONS: [&str; 0]",
        "WRITER_LEASE_CURRENT_RUNTIME_FUNCTIONS: [&str; 7]",
        "WriterLeaseAclProfile::BridgeQuarantined",
        "WriterLeaseAclProfile::CurrentRuntime",
        "has_schema_privilege('lattice_runtime','writer_lease','USAGE')",
    ] {
        assert!(
            source.contains(required),
            "missing split Writer ACL profile field {required}"
        );
    }
    assert!(
        !source.contains("const WRITER_LEASE_RUNTIME_FUNCTIONS: [&str; 7]"),
        "bridge and current must not share one runtime ACL profile"
    );
}

#[test]
fn writer_bridge_companion_freezes_the_measured_ten_profile_catalog() {
    let source = include_str!("../src/setup.rs");
    let bridge_profiles = source
        .split_once("const WRITER_LEASE_V2_BASE_CATALOG_PROFILES")
        .expect("Writer v2 base catalog profiles")
        .1
        .split_once("const WRITER_LEASE_V2_CURRENT_ACL_PROFILES")
        .expect("Writer v2 bridge catalog boundary")
        .0;
    for measured_bridge_signature in [
        "382b81889838d60c02ce5c31f77454e93f23372d90b3137a47663c5de74f9670",
        "560e93c2a765db0024c0e74d25a51b90cfc72b204601139de8fdb688d48c0610",
        "3463b3ac82c1a7c53e5a80c41995f882ffe5f3f07fc5a82a97d50582d4d26915",
        "66b315513cbf50c3c7dbc143eb7061c6dbb823d7eac853c50f83434caf1a1022",
        "caa34168b5f9da4c8d2d02fce6e98882d73456c7c1f5c1af2b71f404efc647d1",
        "f8a84b870fcb8b091dbc7f9cf6835fb4311064eec5c83b31159a9a936a11e738",
        "b99ef0c0ea5b550ae5e805d29b0020e31c1800a016b0de82cda566d7b25e9569",
        "73951f1b33a4d6b3c4742fb49f91cf0601f04fd472b21c4db8bb36815fed0e89",
        "a7ccfc938fbf121a9b807070f69bd5b851be6aa89a8261043ef07336ea7b8dbd",
        "1d6642e77600a93da5b00dda0ee64c15474b4ca2741c51ca760597e7f90ac003",
    ] {
        assert!(
            bridge_profiles.contains(measured_bridge_signature),
            "missing measured Writer bridge signature {measured_bridge_signature}"
        );
    }
    assert!(
        !bridge_profiles
            .contains("0000000000000000000000000000000000000000000000000000000000000000")
    );
}

#[test]
fn writer_current_companion_freezes_the_measured_acl_catalog() {
    let source = include_str!("../src/setup.rs");
    let current_profiles = source
        .split_once("const WRITER_LEASE_V2_CURRENT_ACL_PROFILES")
        .expect("Writer v2 current ACL profiles")
        .1
        .split_once("const EXPECTED_TABLES")
        .expect("Writer v2 current ACL boundary")
        .0;
    for measured_current_signature in [
        "a2e1be8a403a96b679c18ddfa75e476fa1d6ceeccc1ccf62ff6424b2c259ef7b",
        "bd5b05d60340a1b9f9fbf1de2b4bed8586b7eede4fd8d7c4825841c221e89b7a",
    ] {
        assert!(
            current_profiles.contains(measured_current_signature),
            "missing measured Writer current ACL signature {measured_current_signature}"
        );
    }
    assert!(
        !current_profiles
            .contains("0000000000000000000000000000000000000000000000000000000000000000")
    );
}

#[test]
fn task076_writer_fingerprint_uses_the_sql_coalesce_expression() {
    let live = include_str!("postgres_live.rs");
    assert_eq!(live.matches("SELECT pg_catalog.md5(COALESCE(").count(), 6);
    assert!(!live.contains("pg_catalog.coalesce"));
}

#[test]
fn exact_catalog_signatures_cover_same_count_definition_and_acl_drift() {
    let source = include_str!("../src/setup.rs");
    for required in [
        "RELATION_SIGNATURE_SQL",
        "COLUMN_SIGNATURE_SQL",
        "pg_catalog.pg_get_constraintdef",
        "pg_catalog.pg_get_indexdef",
        "pg_catalog.pg_get_functiondef",
        "p.prosrc",
        "TABLE_ACL_SIGNATURE_SQL",
        "FUNCTION_ACL_SIGNATURE_SQL",
        "if columns != 143 || constraints != 59",
        "SCHEMA_ACL_SIGNATURE_SQL",
        "ExactCatalogProfile::V2",
        "ExactCatalogProfile::V3",
    ] {
        assert!(
            source.contains(required),
            "missing exact catalog field {required}"
        );
    }
    assert!(
        source.matches("verify_exact_catalog_profile(").count() >= 3,
        "v2 source and both v3 apply/read-only paths must verify exact signatures"
    );
}

#[test]
fn bootstrap_inspector_distinguishes_both_exact_store_v8_manifests() {
    let source = include_str!("../src/setup.rs");
    for required in [
        "ExtensionBootstrapGlobalProfile::V8LegacyPrefix",
        "ExtensionBootstrapGlobalProfile::V8",
        "01373ed5092e90bf6a9e383955cd70d0fd4e0ed821667f1905b69e313005ea82",
        "2b1fcbbc81261c28ab06ac3180f75c2ee458e57a4adc7e49bc399209f421de60",
    ] {
        assert!(
            source.contains(required),
            "missing Memory V8 bootstrap profile: {required}"
        );
    }
}
