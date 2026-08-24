use lattice_postgres_writer_lease::{
    ExtensionApplyOutcome, ExtensionSetupError, V3ExtensionTarget, WRITER_LEASE_EXTENSION_ID,
    WRITER_LEASE_EXTENSION_PATH, WRITER_LEASE_EXTENSION_SCHEMA_VERSION,
    WRITER_LEASE_V1_EXTENSION_PATH, WRITER_LEASE_V2_EXTENSION_PATH, WRITER_LEASE_V3_EXTENSION_PATH,
    WRITER_LEASE_V3_REBIND_PATH, WriterLeaseV3BridgeState, apply_v3_extension, rebind_v3_extension,
    verify_embedded_extension_manifest, verify_embedded_v1_extension_manifest,
    verify_embedded_v2_extension_manifest, verify_embedded_v3_extension_manifest,
    verify_embedded_v3_rebind_manifest, verify_writer_lease_v3_transition,
};

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

    let setup = include_str!("../src/setup.rs");
    for required in [
        "G5MemoryV3WriterV3Bridge",
        "G6MemoryV3WriterV3BridgePending",
        "G6MemoryV3WriterV3Current",
        "apply_v2_to_v3_bridge",
        "writer_lease_rebind_v3()",
        "verify_v3_bridge_profile",
        "verify_v3_current_profile",
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
        "WHEN 4 THEN 5",
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
fn task087_v2_stays_frozen_and_v3_is_append_only_schema_v6_successor() {
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

    for (state, generation, ledger) in [
        (WriterLeaseV3BridgeState::V2Current, 6, "1:INSTALLED"),
        (
            WriterLeaseV3BridgeState::Bridge,
            7,
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
