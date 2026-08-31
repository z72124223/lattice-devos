use lattice_postgres_writer_lease::{
    ExtensionApplyOutcome, ExtensionSetupError, V3BootstrapProfile, V3ExtensionTarget,
    V4ExtensionTarget, V5ExtensionTarget, WRITER_LEASE_EXTENSION_ID, WRITER_LEASE_EXTENSION_PATH,
    WRITER_LEASE_EXTENSION_SCHEMA_VERSION, WRITER_LEASE_V1_EXTENSION_PATH,
    WRITER_LEASE_V2_EXTENSION_PATH, WRITER_LEASE_V3_EXTENSION_PATH, WRITER_LEASE_V3_REBIND_PATH,
    WRITER_LEASE_V4_EXTENSION_PATH, WRITER_LEASE_V4_REBIND_PATH, WRITER_LEASE_V5_EXTENSION_PATH,
    WriterLeaseV3BridgeState, WriterLeaseV4BridgeState, WriterLeaseV5State, apply_v3_extension,
    apply_v4_extension, apply_v5_extension, inspect_v3_bootstrap_profile,
    rebind_existing_v3_extension, rebind_v3_extension, verify_embedded_extension_manifest,
    verify_embedded_v1_extension_manifest, verify_embedded_v2_extension_manifest,
    verify_embedded_v3_extension_manifest, verify_embedded_v3_rebind_manifest,
    verify_embedded_v4_extension_manifest, verify_embedded_v4_rebind_manifest,
    verify_embedded_v5_extension_manifest, verify_writer_lease_v3_transition,
    verify_writer_lease_v4_transition, verify_writer_lease_v5_transition,
};

#[test]
fn v5_process_handoff_profile_is_append_only_and_explicit() {
    let _: fn(
        &mut postgres::Client,
        &V5ExtensionTarget,
    ) -> Result<ExtensionApplyOutcome, ExtensionSetupError> = apply_v5_extension;
    let manifest = verify_embedded_v5_extension_manifest().expect("Writer v5 manifest");
    assert_eq!(manifest.path(), WRITER_LEASE_V5_EXTENSION_PATH);
    assert_eq!(manifest.schema_version(), 5);
    let sql = std::str::from_utf8(manifest.bytes()).expect("UTF-8 v5 SQL");
    for required in [
        "PROCESS_HANDOFF",
        "writer_lease_transitions_identity_v5",
        "writer_lease_bind_runtime_v5",
        "writer_lease_load_for_update_v5",
        "extension_schema_version = 5",
        "global_schema_version = 7",
        "ledger_ordinal = 8",
        "LATTICE_WRITER_LEASE_SCHEMA_V5",
    ] {
        assert!(sql.contains(required), "missing v5 boundary: {required}");
    }
    assert_eq!(sql.matches("CREATE FUNCTION writer_lease.").count(), 2);
    assert!(!sql.contains("CREATE OR REPLACE"));
    assert!(!sql.contains("DROP TABLE"));
    assert_eq!(
        verify_writer_lease_v5_transition(
            WriterLeaseV5State::V4Current,
            7,
            "1:INSTALLED,2:UPGRADED,3:REBOUND",
        )
        .expect("v4 to v5"),
        WriterLeaseV5State::Current
    );
    assert_eq!(
        verify_writer_lease_v5_transition(
            WriterLeaseV5State::Current,
            7,
            "1:INSTALLED,2:UPGRADED,3:REBOUND,4:UPGRADED",
        )
        .expect("exact v5 retry"),
        WriterLeaseV5State::Current
    );
    for (generation, history) in [
        (8, "1:INSTALLED,2:UPGRADED,3:REBOUND,4:UPGRADED"),
        (7, "1:INSTALLED,2:UPGRADED,3:UPGRADED"),
        (7, "1:INSTALLED,2:UPGRADED,3:REBOUND,4:REBOUND"),
    ] {
        assert!(
            verify_writer_lease_v5_transition(WriterLeaseV5State::Current, generation, history,)
                .is_err(),
            "future or substituted v5 history must fail"
        );
    }
}

#[test]
fn store_v8_runtime_successor_is_writer_owned_fixed_and_replay_safe() {
    let setup = include_str!("../src/setup.rs");
    let library = include_str!("../src/lib.rs");
    for required in [
        "V8V5RebindPending",
        "rebind_v5_for_store_v8",
        "verify_v5_v8_current_profile",
        "verify_v5_store_v8_rebind_source",
        "WRITER_LEASE_V5_STORE_V8_REBIND_PATH",
        "verify_embedded_v5_store_v8_rebind_manifest",
        "2b1fcbbc81261c28ab06ac3180f75c2ee458e57a4adc7e49bc399209f421de60",
    ] {
        assert!(
            setup.contains(required) || library.contains(required),
            "missing Writer Store V8 successor contract: {required}"
        );
    }
    assert!(setup.contains("GlobalApplyGate::acquire(client)"));
}

#[test]
fn managed_writer_profile_rejects_unmodeled_casts_and_transforms() {
    let setup = include_str!("../src/setup.rs");
    for required in [
        "pg_catalog.pg_cast",
        "pg_catalog.pg_transform",
        "tr.trftype",
        "tr.trffromsql",
        "tr.trftosql",
    ] {
        assert!(
            setup.contains(required),
            "missing Writer catalog closure: {required}"
        );
    }
}

#[test]
fn task105_bootstrap_profile_is_read_only_closed_and_fully_verified() {
    let _: fn(
        &mut postgres::Client,
        &V3ExtensionTarget,
    ) -> Result<V3BootstrapProfile, ExtensionSetupError> = inspect_v3_bootstrap_profile;
    assert_eq!(
        format!("{:?}", V3BootstrapProfile::V7V4Current),
        "V7V4Current"
    );
    assert_eq!(
        format!("{:?}", V3BootstrapProfile::V6V4BridgeLegacyF252Rebind),
        "V6V4BridgeLegacyF252Rebind"
    );

    let setup = include_str!("../src/setup.rs");
    let inspector = setup
        .split("pub fn inspect_v3_bootstrap_profile")
        .nth(1)
        .expect("TASK105 Writer-owned inspector")
        .split("fn apply_v3_extension_under_gate")
        .next()
        .expect("bounded inspector body");
    for required in [
        ".read_only(true)",
        "V3InstalledState::Absent",
        "verify_v2_bootstrap_predecessor",
        "verify_v3_bridge_profile",
        "verify_v3_bridge_pending_profile",
        "verify_v3_current_profile",
        "verify_v4_bridge_profile",
        "verify_legacy_f252_v4_bridge_profile",
        "verify_v4_v7_current_profile",
        "verify_v3_rebind_boundary",
        "verify_replay_safe_history",
    ] {
        assert!(
            inspector.contains(required),
            "missing preflight gate: {required}"
        );
    }
    for prohibited in [
        "batch_execute(\"CALL",
        "apply_v2_to_v3_bridge",
        "activate_v3_runtime_acl",
    ] {
        assert!(
            !inspector.contains(prohibited),
            "read-only preflight contains mutation: {prohibited}"
        );
    }
    for required in [
        "V3_BRIDGE_EXPECTED_CATALOG_PROFILES",
        "V3_CURRENT_EXPECTED_CATALOG_PROFILES",
        "V4_BRIDGE_EXPECTED_CATALOG_PROFILES",
        "V4_CURRENT_EXPECTED_CATALOG_PROFILES",
        "verify_catalog_profile",
    ] {
        assert!(
            setup.contains(required),
            "missing exact catalog profile gate: {required}"
        );
    }
}

#[test]
fn v4_profile_names_exact_store_v7_without_future_wildcards() {
    let setup = include_str!("../src/setup.rs");
    let extension = include_str!("../../../db/extensions/writer-lease/v4.sql");
    let rebind = include_str!("../../../db/extensions/writer-lease/v4-rebind.sql");
    for required in [
        "const V7_GLOBAL_MANIFEST_SHA256: &str",
        "584a446464ab2f7ebd8b85543ba36a6d52b0a708502c39d2653b8814d84313f8",
        "G7MemoryV3WriterV4Current",
        "V3BootstrapProfile::V7V4Current",
    ] {
        assert!(
            setup.contains(required),
            "missing exact v7 Writer profile {required}"
        );
    }
    for required in [
        "global_schema_version IN (6, 7)",
        "ledger_ordinal = 7",
        "global_schema_version = 7",
        "1:INSTALLED:3:6,2:UPGRADED:4:6,3:REBOUND:4:7",
        "w.global_manifest_sha256 = p_global_manifest_sha256",
    ] {
        assert!(
            extension.contains(required),
            "missing v7 extension constraint {required}"
        );
    }
    assert!(rebind.contains("584a446464ab2f7ebd8b85543ba36a6d52b0a708502c39d2653b8814d84313f8"));
    assert!(rebind.contains("writer_lease_rebind_v4"));
    assert_eq!(extension.matches(")) IS NOT TRUE THEN").count(), 2);
    assert_eq!(rebind.matches(") IS NOT TRUE THEN").count(), 2);
    assert!(!extension.contains("global_schema_version >="));
    assert!(!rebind.contains("current_schema_version >="));
    assert!(!extension.contains("extension_schema_version = 5"));
    assert!(!rebind.contains("current_schema_version IN"));
}

#[test]
fn task094_exposes_typed_v3_bridge_and_rebind_owner_apis() {
    let _: fn(
        &mut postgres::Client,
        &V3ExtensionTarget,
    ) -> Result<ExtensionApplyOutcome, ExtensionSetupError> = apply_v3_extension;
    let _: fn(
        &mut postgres::Client,
        &V3ExtensionTarget,
    ) -> Result<ExtensionApplyOutcome, ExtensionSetupError> = rebind_v3_extension;
    let _: fn(
        &mut postgres::Client,
        &V3ExtensionTarget,
    ) -> Result<ExtensionApplyOutcome, ExtensionSetupError> = rebind_existing_v3_extension;
    let _: fn(
        &mut postgres::Client,
        &V4ExtensionTarget,
    ) -> Result<ExtensionApplyOutcome, ExtensionSetupError> = apply_v4_extension;

    let setup = include_str!("../src/setup.rs");
    for required in [
        "G5MemoryV3WriterV3Bridge",
        "G6MemoryV3WriterV3BridgePending",
        "G6MemoryV3WriterV3Current",
        "apply_v2_to_v3_bridge",
        "writer_lease_rebind_v3()",
        "verify_v3_bridge_profile",
        "verify_v3_current_profile",
        "if !allow_fresh_install",
    ] {
        assert!(
            setup.contains(required),
            "missing v3 owner boundary: {required}"
        );
    }

    let rebind = verify_embedded_v3_rebind_manifest().expect("fixed rebind bytes");
    assert_eq!(rebind.path(), WRITER_LEASE_V3_REBIND_PATH);
    let sql = include_str!("../../../db/extensions/writer-lease/v3-rebind.sql");
    for required in [
        "CREATE PROCEDURE writer_lease.writer_lease_rebind_v3()",
        "LANGUAGE plpgsql",
        "SECURITY INVOKER",
        "SET search_path = pg_catalog",
        "SET row_security = on",
        "SET lock_timeout = '5s'",
        "SET statement_timeout = '30s'",
        "LOCK TABLE writer_lease.writer_lease_extension_identity",
        "writer_lease.writer_lease_extension_ledger",
        "writer_lease.writer_lease_heads",
        "$lattice_writer_lease_rebind_v3$",
        "GRANT USAGE ON SCHEMA writer_lease TO lattice_runtime",
        "SELECT CASE pg_catalog.count(*) WHEN 2 THEN 3 WHEN 4 THEN 5 END",
        "event_kind = 'REBOUND'",
    ] {
        assert!(
            sql.contains(required),
            "missing fixed v3 SQL boundary: {required}"
        );
    }
    assert!(!sql.contains("CREATE OR REPLACE"));
    assert!(!sql.contains(
        "GRANT EXECUTE ON PROCEDURE writer_lease.writer_lease_rebind_v3() TO lattice_runtime"
    ));

    let v4_rebind = verify_embedded_v4_rebind_manifest().expect("fixed v4 rebind bytes");
    assert_eq!(v4_rebind.path(), WRITER_LEASE_V4_REBIND_PATH);
    let v4_sql = include_str!("../../../db/extensions/writer-lease/v4-rebind.sql");
    for required in [
        "CREATE OR REPLACE PROCEDURE writer_lease.writer_lease_rebind_v4()",
        "$lattice_writer_lease_rebind_v4$",
        "WHEN 2 THEN 3 WHEN 4 THEN 5 WHEN 6 THEN 7",
        "writer_lease_bind_runtime_v4",
        "writer_lease_load_for_update_v4",
        ") IS NOT TRUE THEN",
    ] {
        assert!(
            v4_sql.contains(required),
            "missing v4 rebind boundary: {required}"
        );
    }
}

#[test]
fn task094_exact_f252_rebind_reconciliation_is_closed() {
    let setup = include_str!("../src/setup.rs");
    for required in [
        "LEGACY_F252_WRITER_LEASE_V4_REBIND_BODY_SHA256",
        "4834f71b90744dddbf828baa5b1e0c5b3e3efbc64bb1d186b8c48bce8c88da52",
        "LEGACY_F252_WRITER_LEASE_V4_REBIND_OBJECT_SIGNATURE",
        "0b5001595269061b484a31710f22e16dbf9d323d50bf67d5a08f949d4c4ddbf8",
        "LEGACY_F252_WRITER_LEASE_V4_REBIND_ACL_SIGNATURE",
        "50bfe792c391202917747211e6a28f4319e46285be5ae10b30e3cee79bedad41",
        "LEGACY_F252_WRITER_LEASE_V4_FUNCTION_SIGNATURE",
        "3a0d9b1593e0adff27ba54cb080538286ee1bbcc204d2ae4cb23ae1558dda4a8",
        "install_new_v4_rebind_boundary(&mut transaction, v4_rebind)",
        "reconcile_existing_v4_rebind_boundary(&mut transaction, v4_rebind)",
        "sha256_hex(observed_body.trim().as_bytes())",
        "V4_REBIND_OBJECT_PROFILE_SQL",
        "V4_REBIND_ACL_PROFILE_SQL",
    ] {
        assert!(
            setup.contains(required),
            "missing exact f252 v4 reconciliation boundary: {required}"
        );
    }
    let install = setup
        .split("fn install_new_v4_rebind_boundary")
        .nth(1)
        .expect("new v4 rebind installation")
        .split("fn reconcile_existing_v4_rebind_boundary")
        .next()
        .expect("bounded new v4 installation");
    assert!(install.contains("if !rows.is_empty()"));
    let reconcile = setup
        .split("fn reconcile_existing_v4_rebind_boundary")
        .nth(1)
        .expect("existing v4 rebind reconciliation")
        .split("fn verify_v4_rebind_boundary")
        .next()
        .expect("bounded existing v4 reconciliation");
    assert!(
        reconcile.find("classify_v4_rebind_boundary").unwrap()
            < reconcile.find("batch_execute(sql)").unwrap()
    );
    assert!(!reconcile.contains("rows.is_empty()"));
}

#[test]
fn v1_history_is_immutable_and_v2_is_the_current_append_only_successor() {
    let historical = verify_embedded_v1_extension_manifest().expect("frozen v1 manifest");
    assert_eq!(historical.path(), WRITER_LEASE_V1_EXTENSION_PATH);
    assert_eq!(historical.schema_version(), 1);
    assert_eq!(historical.byte_length(), 44_366);
    assert_eq!(
        historical.sql_sha256().as_str(),
        "63ffbf8f8b6c22bf35c3d393bd84e9462ca37e4ace94ceaedd6c27b729daa562"
    );
    assert_eq!(
        historical.manifest_sha256().as_str(),
        "0179e2a9b0976008902ab0d1cce6ab493a16047a649571f9ce4f13cc53cc6b33"
    );

    let manifest = verify_embedded_extension_manifest().expect("frozen manifest");
    assert_eq!(manifest.extension_id(), WRITER_LEASE_EXTENSION_ID);
    assert_eq!(manifest.path(), WRITER_LEASE_EXTENSION_PATH);
    assert_eq!(
        manifest.schema_version(),
        WRITER_LEASE_EXTENSION_SCHEMA_VERSION
    );
    assert_eq!(manifest.sql_sha256().as_str().len(), 64);
    assert_eq!(manifest.manifest_sha256().as_str().len(), 64);

    let sql = std::str::from_utf8(manifest.bytes()).expect("UTF-8 SQL");
    assert!(!sql.contains("CREATE OR REPLACE"));
    assert!(!sql.contains("DROP TABLE"));
    assert!(!sql.contains("DROP CONSTRAINT writer_lease_extension_ledger_identity_fk"));
    for required in [
        "DROP CONSTRAINT writer_lease_extension_ledger_singleton_key",
        "DROP CONSTRAINT writer_lease_extension_ledger_single",
        "DROP CONSTRAINT writer_lease_extension_ledger_fixed",
        "DROP CONSTRAINT writer_lease_extension_identity_fixed",
        "CREATE FUNCTION writer_lease.writer_lease_bind_runtime_v2",
        "CREATE FUNCTION writer_lease.writer_lease_load_for_update_v2",
        "ledger_ordinal = 2",
        "event_kind = 'UPGRADED'",
        "ledger_ordinal = 3",
        "event_kind = 'REBOUND'",
        "ledger_ordinal = 1",
        "event_kind = 'INSTALLED'",
        "FOR UPDATE OF h",
        "SECURITY DEFINER",
        "REVOKE ALL ON SCHEMA writer_lease",
        "REVOKE ALL ON ALL FUNCTIONS IN SCHEMA writer_lease",
        "LATTICE_WRITER_LEASE_SCHEMA_V2",
    ] {
        assert!(sql.contains(required), "missing SQL boundary: {required}");
    }
    assert_eq!(
        sql.matches("CREATE FUNCTION writer_lease.").count(),
        2,
        "v2 adds only the two ordinal-aware runtime functions"
    );
    assert!(!sql.contains("writer_lease_assert_memory_upgrade"));
    assert!(!sql.contains("GRANT USAGE ON SCHEMA writer_lease"));
    assert!(!sql.contains("GRANT EXECUTE ON FUNCTION writer_lease"));
}

#[test]
fn immutable_v3_remains_exact_schema_v5_v6_and_v4_is_the_v7_successor() {
    let v2 = verify_embedded_v2_extension_manifest().expect("frozen v2 manifest");
    assert_eq!(v2.path(), WRITER_LEASE_V2_EXTENSION_PATH);
    assert_eq!(v2.schema_version(), 2);
    assert_eq!(v2.byte_length(), 22_985);
    assert_eq!(
        v2.sql_sha256().as_str(),
        "8243fd39a3565c641423fde3f15cf801a4a48a12c8d238ae8e1657acdcdc56e3"
    );

    let v3 = verify_embedded_v3_extension_manifest().expect("Writer v3 manifest");
    assert_eq!(v3.path(), WRITER_LEASE_V3_EXTENSION_PATH);
    assert_eq!(v3.schema_version(), 3);
    assert_eq!(v3.byte_length(), 17_568);
    assert_eq!(
        v3.sql_sha256().as_str(),
        "677c010a61e5945bcc6b96ca9f3d9e57830dc42f4cfbd46ea76d5e9d8b9262a0"
    );
    assert_eq!(
        v3.manifest_sha256().as_str(),
        "eab2812fa3d94cd3466d7c003386f805a973fd7def1f16aeb15b52f47dad78e4"
    );
    let sql = std::str::from_utf8(v3.bytes()).expect("UTF-8 v3 SQL");
    for required in [
        "extension_schema_version = 2",
        "global_schema_version = 5",
        "extension_schema_version = 3",
        "global_schema_version = 6",
        "ledger_ordinal = 4",
        "ledger_ordinal = 5",
        "FOREMAN_COORDINATION",
        "FOREMAN_SNAPSHOT_RECORDED",
        "writer_lease_bind_runtime_v3",
        "writer_lease_load_for_update_v3",
        "LATTICE_WRITER_LEASE_SCHEMA_V3",
    ] {
        assert!(sql.contains(required), "missing v3 boundary: {required}");
    }
    assert!(!sql.contains("CREATE OR REPLACE"));
    assert!(!sql.contains("DROP TABLE"));
    assert!(!sql.contains("CREATE TABLE"));
    assert_eq!(sql.matches("CREATE FUNCTION writer_lease.").count(), 2);
    assert!(!sql.contains("GRANT USAGE ON SCHEMA writer_lease"));
    assert!(!sql.contains("GRANT EXECUTE ON FUNCTION writer_lease"));
    assert!(!sql.contains("global_schema_version = 7"));

    let v4 = verify_embedded_v4_extension_manifest().expect("Writer v4 manifest");
    assert_eq!(v4.path(), WRITER_LEASE_V4_EXTENSION_PATH);
    assert_eq!(v4.schema_version(), 4);
    let v4_sql = std::str::from_utf8(v4.bytes()).expect("UTF-8 v4 SQL");
    for required in [
        "extension_schema_version = 4",
        "global_schema_version IN (6, 7)",
        "writer_lease_bind_runtime_v4",
        "writer_lease_load_for_update_v4",
        "LATTICE_WRITER_LEASE_SCHEMA_V4",
    ] {
        assert!(v4_sql.contains(required), "missing v4 boundary: {required}");
    }
    assert_eq!(v4_sql.matches("CREATE FUNCTION writer_lease.").count(), 2);
    assert!(!v4_sql.contains("CREATE OR REPLACE"));
    assert!(!v4_sql.contains("global_schema_version >="));
}

#[test]
fn task087_v3_transition_is_closed_ordered_idempotent_and_runtime_quarantined() {
    let bridge =
        verify_writer_lease_v3_transition(WriterLeaseV3BridgeState::V2Current, 5, "1:INSTALLED")
            .expect("v2 current to v3 bridge");
    assert_eq!(bridge, WriterLeaseV3BridgeState::Bridge);
    assert_eq!(bridge.runtime_function_count(), 0);
    assert_eq!(
        verify_writer_lease_v3_transition(bridge, 5, "1:INSTALLED,2:UPGRADED")
            .expect("exact retry"),
        bridge
    );
    let pending = verify_writer_lease_v3_transition(bridge, 6, "1:INSTALLED,2:UPGRADED")
        .expect("exact schema-v6 migration");
    assert_eq!(pending, WriterLeaseV3BridgeState::BridgePending);
    assert_eq!(pending.runtime_function_count(), 0);
    let current = verify_writer_lease_v3_transition(pending, 6, "1:INSTALLED,2:UPGRADED,3:REBOUND")
        .expect("exact v3 rebind");
    assert_eq!(current, WriterLeaseV3BridgeState::Current);
    assert_eq!(current.runtime_function_count(), 7);
    assert!(
        verify_writer_lease_v3_transition(
            current,
            7,
            "1:INSTALLED,2:UPGRADED,3:REBOUND,4:REBOUND",
        )
        .is_err(),
        "immutable v3 must not accept schema v7"
    );

    for (state, generation, ledger) in [
        (WriterLeaseV3BridgeState::V2Current, 6, "1:INSTALLED"),
        (
            WriterLeaseV3BridgeState::Bridge,
            8,
            "1:INSTALLED,2:UPGRADED",
        ),
        (WriterLeaseV3BridgeState::Bridge, 6, "1:INSTALLED,3:REBOUND"),
        (
            WriterLeaseV3BridgeState::Current,
            5,
            "1:INSTALLED,2:UPGRADED,3:REBOUND",
        ),
    ] {
        assert!(
            verify_writer_lease_v3_transition(state, generation, ledger).is_err(),
            "cross-generation or reordered replay must fail"
        );
    }
}

#[test]
fn v4_transition_accepts_only_old_v3_v6_to_exact_v4_v7() {
    let bridge = verify_writer_lease_v4_transition(
        WriterLeaseV4BridgeState::V3Current,
        6,
        "1:INSTALLED,2:UPGRADED,3:REBOUND",
    )
    .expect("old v3 current to v4 bridge");
    assert_eq!(bridge, WriterLeaseV4BridgeState::Bridge);
    assert_eq!(bridge.runtime_function_count(), 0);
    assert_eq!(
        verify_writer_lease_v4_transition(
            bridge,
            6,
            "1:INSTALLED,2:UPGRADED,3:REBOUND,4:UPGRADED",
        )
        .expect("exact v4 bridge retry"),
        bridge
    );
    let current = verify_writer_lease_v4_transition(
        bridge,
        7,
        "1:INSTALLED,2:UPGRADED,3:REBOUND,4:UPGRADED,5:REBOUND",
    )
    .expect("exact v4 schema-v7 rebind");
    assert_eq!(current, WriterLeaseV4BridgeState::Current);
    assert_eq!(current.runtime_function_count(), 7);
    for (generation, ledger) in [
        (7, "1:INSTALLED,2:UPGRADED,3:REBOUND,4:REBOUND"),
        (8, "1:INSTALLED,2:UPGRADED,3:REBOUND,4:UPGRADED,5:REBOUND"),
    ] {
        assert!(
            verify_writer_lease_v4_transition(bridge, generation, ledger).is_err(),
            "skipped upgrade or future generation must fail"
        );
    }
}

#[test]
fn setup_encodes_the_closed_state_machine_and_common_lock_order() {
    let setup = include_str!("../src/setup.rs");
    let global = setup
        .find("0x4c41_5454_4943_4501")
        .expect("global migration lock");
    let memory = setup
        .find("0x4c41_5443_4d45_4d31")
        .expect("Memory migration lock");
    let writer = setup
        .find("0x4c41_5457_4c45_4131")
        .expect("Writer Lease migration lock");
    assert!(global < memory && memory < writer);
    for required in [
        "G3MemoryV2WriterV1Current",
        "G3MemoryV2WriterV2Bridge",
        "G5MemoryV2WriterV2BridgePending",
        "G5MemoryV3WriterV2BridgePending",
        "G5MemoryV3WriterV2Current",
        "UPGRADED",
        "REBOUND",
        "ACTIVE",
        "SUSPECT",
        "GRANT USAGE ON SCHEMA writer_lease",
        "writer_lease_bind_runtime_v2",
        "writer_lease_load_for_update_v2",
        "{}|t|{}",
    ] {
        assert!(
            setup.contains(required),
            "missing setup boundary: {required}"
        );
    }
}

#[test]
fn writer_v2_freezes_measured_bridge_and_current_catalog_profiles() {
    let setup = include_str!("../src/setup.rs");
    for required in [
        "V2_BRIDGE_EXPECTED_CATALOG_PROFILES",
        "V2_CURRENT_EXPECTED_CATALOG_PROFILES",
        "382b81889838d60c02ce5c31f77454e93f23372d90b3137a47663c5de74f9670",
        "560e93c2a765db0024c0e74d25a51b90cfc72b204601139de8fdb688d48c0610",
        "3463b3ac82c1a7c53e5a80c41995f882ffe5f3f07fc5a82a97d50582d4d26915",
        "66b315513cbf50c3c7dbc143eb7061c6dbb823d7eac853c50f83434caf1a1022",
        "caa34168b5f9da4c8d2d02fce6e98882d73456c7c1f5c1af2b71f404efc647d1",
        "f8a84b870fcb8b091dbc7f9cf6835fb4311064eec5c83b31159a9a936a11e738",
        "a2e1be8a403a96b679c18ddfa75e476fa1d6ceeccc1ccf62ff6424b2c259ef7b",
        "b99ef0c0ea5b550ae5e805d29b0020e31c1800a016b0de82cda566d7b25e9569",
        "73951f1b33a4d6b3c4742fb49f91cf0601f04fd472b21c4db8bb36815fed0e89",
        "bd5b05d60340a1b9f9fbf1de2b4bed8586b7eede4fd8d7c4825841c221e89b7a",
        "a7ccfc938fbf121a9b807070f69bd5b851be6aa89a8261043ef07336ea7b8dbd",
        "1d6642e77600a93da5b00dda0ee64c15474b4ca2741c51ca760597e7f90ac003",
    ] {
        assert!(
            setup.contains(required),
            "missing catalog commitment {required}"
        );
    }
    assert!(setup.contains("RuntimeProfile::Quarantined => &V2_BRIDGE_EXPECTED_CATALOG_PROFILES"));
    assert!(setup.contains("RuntimeProfile::Current => &V2_CURRENT_EXPECTED_CATALOG_PROFILES"));
}

#[test]
fn live_acceptance_has_owner_staged_w1_source_and_restart_phases() {
    let live = include_str!("postgres_live.rs");
    for required in [
        "Some(\"source_install\")",
        "Some(\"source_seed\")",
        "Some(\"bridge\")",
        "Some(\"activate\")",
        "Some(\"runtime\")",
        "Some(\"restart\")",
        "Some(\"fresh_install\")",
        "Some(\"fresh_restart\")",
        "writer_lease_bind_runtime_v1",
        "writer_lease_load_for_update_v1",
        "writer_lease_commit_plan_v1",
        "TASK076_WRITER_SOURCE_INSTALL_PASS",
        "TASK076_WRITER_SOURCE_PASS",
        "TASK076_WRITER_BRIDGE_PASS",
        "TASK076_WRITER_ACTIVATE_PASS",
        "TASK076_WRITER_RUNTIME_PASS",
        "TASK076_WRITER_RESTART_PASS",
        "TASK076_WRITER_FRESH_INSTALL_PASS",
        "TASK076_WRITER_FRESH_RESTART_PASS",
        "TASK076_WRITER_FRESH_PROFILE_SHA256",
        "TASK076_WRITER_FRESH_DATABASE_UUID",
    ] {
        assert!(
            live.contains(required),
            "missing live phase boundary: {required}"
        );
    }
}

#[test]
fn fresh_profile_evidence_sql_preserves_keyword_boundaries() {
    let live = include_str!("postgres_live.rs");
    for required in [
        "FROM pg_catalog.pg_proc AS p \\",
        "ON n.oid=p.pronamespace \\",
        "WHERE n.nspname='writer_lease' \\",
        "writer_lease_extension_identity AS i \\",
    ] {
        assert!(live.contains(required), "missing SQL boundary {required}");
    }
}

#[test]
fn phased_live_owner_queries_hold_the_migrator_session_role() {
    let live = include_str!("postgres_live.rs");
    assert!(live.contains("if task076_phase.is_some()"));
    assert!(
        live.contains(
            "SET ROLE lattice_migrator; SET search_path=pg_catalog; SET row_security=on;"
        )
    );
}

#[test]
fn live_acceptance_proves_bridge_activation_and_fresh_noop_idempotency() {
    let live = include_str!("postgres_live.rs");
    for required in [
        "TASK076_WRITER_BRIDGE_REPLAY_PASS",
        "TASK076_WRITER_ACTIVATE_REPLAY_PASS",
        "TASK076_WRITER_FRESH_NOOP_PASS",
        "ExtensionApplyOutcome::AlreadyCurrent",
        "bridge replay must preserve exact",
        "activation replay must preserve exact",
    ] {
        assert!(
            live.contains(required),
            "missing idempotency proof: {required}"
        );
    }
}

#[test]
fn setup_and_live_acceptance_converge_two_concurrent_runners() {
    let setup = include_str!("../src/setup.rs");
    let apply = setup
        .split_once("pub fn apply_extension")
        .expect("apply_extension source")
        .1
        .split_once("pub fn verify_extension")
        .expect("verify_extension source")
        .0;
    assert!(
        apply.contains(".isolation_level(IsolationLevel::Serializable)"),
        "each Writer setup attempt must remain one bounded serializable transaction"
    );
    assert!(!apply.contains("IsolationLevel::ReadCommitted"));
    for required in [
        "MAX_SERIALIZATION_ATTEMPTS: usize = 3",
        "SqlState::T_R_SERIALIZATION_FAILURE",
        "attempt < MAX_SERIALIZATION_ATTEMPTS",
        "struct GlobalApplyGate",
        "GlobalApplyGate::acquire(client)",
        "GLOBAL_APPLY_GATE_TIMEOUT: Duration = Duration::from_secs(30)",
        "GLOBAL_APPLY_GATE_POLL_INTERVAL: Duration = Duration::from_millis(20)",
        "fn try_global_apply_gate_lock",
        "fn release_global_apply_gate_lock",
        "enter_migrator(&mut transaction)",
        "pg_catalog.pg_try_advisory_lock($1)",
        "Instant::now()",
        "std::thread::sleep(",
        "pg_catalog.pg_advisory_unlock($1)",
        "impl Drop for GlobalApplyGate",
        "GLOBAL_APPLY_GATE_TIMEOUT.saturating_sub(elapsed)",
    ] {
        assert!(
            setup.contains(required),
            "missing bounded serialization retry boundary: {required}"
        );
    }
    assert!(!setup.contains("pg_catalog.current_setting('lock_timeout')"));
    assert!(!setup.contains("pg_catalog.set_config('lock_timeout','30s',false)"));
    assert!(!setup.contains("pg_catalog.pg_advisory_lock($1)"));
    assert!(!setup.contains("SqlState::T_R_DEADLOCK_DETECTED"));
    assert!(!setup.contains("42704"));
    assert!(!setup.contains("42723"));
    let session_gate = apply
        .find("GlobalApplyGate::acquire(client)")
        .expect("pre-snapshot session gate");
    let serializable_attempt = apply
        .find("apply_extension_attempt(")
        .expect("serializable attempt");
    assert!(session_gate < serializable_attempt);

    let live = include_str!("postgres_live.rs");
    let concurrent_apply = live
        .split_once("fn task076_concurrent_apply")
        .expect("TASK-076 concurrent apply helper")
        .1
        .split_once("fn run_task076_fresh_install")
        .expect("TASK-076 fresh install helper")
        .0;
    for required in [
        "Duration::from_millis(100)",
        "both Writer setup runners must remain pending while the global lock is held",
        "!runner_a.is_finished()",
        "!runner_b.is_finished()",
        "Client::connect(migrator_url, NoTls).expect(\"Writer setup runner A\")",
        "Client::connect(migrator_url, NoTls).expect(\"Writer setup runner B\")",
    ] {
        assert!(
            concurrent_apply.contains(required),
            "missing concurrent runner proof: {required}"
        );
    }
    assert!(!concurrent_apply.contains("SET ROLE lattice_migrator"));
    assert!(!concurrent_apply.contains("pg_blocking_pids"));
    for required in [
        "fn assert_task076_login_apply_boundary",
        "assert_task076_login_apply_boundary(migrator_url, target)",
        "TASK076_WRITER_LOGIN_APPLY_BOUNDARY_PASS",
        "Writer setup gate must not remain held after error",
    ] {
        assert!(
            live.contains(required),
            "missing login-role setup boundary proof: {required}"
        );
    }
    for required in [
        "TASK076_WRITER_BRIDGE_CONCURRENT_PASS",
        "TASK076_WRITER_ACTIVATE_CONCURRENT_PASS",
        "TASK076_WRITER_FRESH_CONCURRENT_PASS",
    ] {
        assert!(
            live.contains(required),
            "missing concurrent runner proof: {required}"
        );
    }
}

#[test]
fn bridge_live_acceptance_has_fixed_safe_concurrency_stages() {
    let live = include_str!("postgres_live.rs");
    for stage in [
        "PRE_PROFILE",
        "HISTORY_LOAD",
        "BLOCKER_TX",
        "BLOCKER_LOCK",
        "RUNNERS_CONNECTED",
        "RUNNERS_STARTED",
        "RUNNERS_BLOCKED",
        "BLOCKER_RELEASED",
        "RUNNERS_JOINED",
        "OUTCOMES",
        "POST_PROFILE",
        "HISTORY_COMPARE",
        "SEQUENTIAL_NOOP",
    ] {
        for boundary in ["ENTER", "PASS"] {
            let token = format!("TASK076_WRITER_BRIDGE_{stage}_{boundary}");
            assert!(live.contains(&token), "missing safe bridge stage: {token}");
        }
    }
}
