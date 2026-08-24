use lattice_postgres_store::{
    DatabaseRole, MigrationStatus, MigrationTarget, MigrationTransactionMode,
    POSTGRES_DRIVER_VERSION, POSTGRES_SCHEMA_VERSION, PostgresStoreSetupError,
    PostgresStoreSetupErrorKind, SUPPORTED_POSTGRES_MAJOR, migration_manifest,
    verify_embedded_manifest,
};

const BOOTSTRAP_SHA256: &str = "7bff021fc17f738551309c906578c8015b2dd0307d27d239c21df1697c4d09c8";
const FOUNDATION_SHA256: &str = "e996dc64af3112a647e75ebf07df2a77b1e9b3a018ed443880150365184883f0";
const LIVE_CONTROL_STORE_SHA256: &str =
    "00ae3eedd76704f26b1df58955d9d594c98f0ba525be93b15d8c9ebb1f2115c1";
const PROJECT_REGISTRY_REPOSITORY_SHA256: &str =
    "b7af1f8a8ac370bbfc8a5312497461587cb8a86eb32ff97e5b865c7ae9bf0dcf";

#[test]
#[allow(clippy::too_many_lines)]
fn task094_store_boundary_keeps_writer_semantics_and_live_composition_outside_store() {
    let manifest = include_str!("../Cargo.toml");
    assert!(
        !manifest.contains("lattice-postgres-writer-lease"),
        "Store must not carry a Writer adapter dependency, even for tests"
    );

    let setup = include_str!("../src/postgres_setup.rs");
    for forbidden in [
        "verify_writer_lease_v2_identity_and_ledger",
        "verify_writer_lease_v3_identity",
        "writer_lease_identity_shape",
        "writer_lease_ledger_shape",
        "FROM ONLY writer_lease.writer_lease_extension_",
        "UPDATE ONLY writer_lease.",
        "INSERT INTO writer_lease.",
        "DELETE FROM ONLY writer_lease.",
        "LOCK TABLE writer_lease.",
    ] {
        assert!(
            !setup.contains(forbidden),
            "Store must not retain Writer semantic ownership: {forbidden}"
        );
    }

    let apply = setup
        .split_once("pub fn apply_migrations(")
        .expect("migration entrypoint")
        .1;
    let v5_to_v6 = apply
        .split_once("InstalledManifestState::ExactV5Prefix")
        .expect("exact v5 transition arm")
        .1
        .split_once("InstalledManifestState::ExactV6Full")
        .expect("exact v6 retry arm")
        .0;
    assert_eq!(
        v5_to_v6
            .matches("CALL writer_lease.writer_lease_rebind_v3()")
            .count(),
        1,
        "exact-v5 transition has one fixed Writer executable boundary"
    );
    let v5_order = [
        "verify_v5_upgrade_source",
        "apply_missing_entries(&mut transaction, 6)",
        "advance_compatibility_from_v5",
        "CALL writer_lease.writer_lease_rebind_v3()",
        "verify_runtime_foreman_schema_v6",
    ]
    .map(|needle| {
        v5_to_v6
            .find(needle)
            .unwrap_or_else(|| panic!("exact-v5 transition missing {needle}"))
    });
    assert!(
        v5_order.windows(2).all(|pair| pair[0] < pair[1]),
        "exact-v5 transition must retain v5 verification, 0007, v6 compatibility, fixed CALL, then catalog/ACL verification"
    );
    let v6_retry = apply
        .split_once("InstalledManifestState::ExactV6Full")
        .expect("exact v6 retry arm")
        .1
        .split_once("};\n\n    preflight_connection")
        .expect("migration match boundary")
        .0;
    assert_eq!(
        v6_retry
            .matches("CALL writer_lease.writer_lease_rebind_v3()")
            .count(),
        1,
        "exact-v6 retry must call the same idempotent Writer procedure"
    );
    let rebind_sql = include_str!("../../../db/extensions/writer-lease/v3-rebind.sql");
    for required in [
        "CREATE PROCEDURE writer_lease.writer_lease_rebind_v3()",
        "SECURITY INVOKER",
        "SET search_path = pg_catalog",
        "SET row_security = on",
        "SET lock_timeout = '5s'",
        "SET statement_timeout = '30s'",
    ] {
        assert!(
            rebind_sql.contains(required),
            "missing Writer-owned rebind boundary: {required}"
        );
    }
    assert_eq!(
        rebind_sql.matches("LOCK TABLE ").count(),
        1,
        "Writer-owned rebind SQL must contain exactly one LOCK TABLE statement"
    );
    let lock_block = rebind_sql
        .split_once("LOCK TABLE ")
        .expect("Writer-owned rebind lock statement")
        .1
        .split_once(" IN SHARE ROW EXCLUSIVE MODE;")
        .expect("Writer-owned rebind lock mode terminator")
        .0;
    let lock_order = [
        "writer_lease.writer_lease_extension_identity",
        "writer_lease.writer_lease_extension_ledger",
        "writer_lease.writer_lease_heads",
        "writer_lease.writer_lease_commands",
        "writer_lease.writer_lease_transitions",
    ]
    .map(|table| {
        assert_eq!(
            lock_block.matches(table).count(),
            1,
            "Writer-owned lock block must name {table} exactly once"
        );
        lock_block
            .find(table)
            .unwrap_or_else(|| panic!("Writer-owned lock block missing {table}"))
    });
    assert!(
        lock_order.windows(2).all(|pair| pair[0] < pair[1]),
        "Writer-owned lock block must order identity, ledger, heads, commands, transitions"
    );
    assert!(!rebind_sql.contains(
        "GRANT EXECUTE ON PROCEDURE writer_lease.writer_lease_rebind_v3() TO lattice_runtime"
    ));

    let store_live = include_str!("postgres_live.rs");
    assert!(
        !store_live.contains("TASK094_STAGE_"),
        "TASK-094 live composition belongs to lattice-runtime, not Store"
    );
    assert!(
        !store_live.contains("lattice_postgres_writer_lease"),
        "Store live tests must not import the Writer adapter"
    );
    let runtime_live =
        include_str!("../../../apps/lattice-runtime/tests/task094_writer_v3_transition.rs");
    for required in [
        "TASK094_STAGE_FRESH_V5_PASS",
        "TASK094_STAGE_MEMORY_V3_PASS",
        "TASK094_STAGE_WRITER_V2_PASS",
        "TASK094_STAGE_WRITER_V3_BRIDGE_PASS",
        "TASK094_STAGE_REBIND_FAILURE_ATOMICITY_PASS",
        "TASK094_STAGE_STORE_V6_PASS",
        "SqlState::OBJECT_NOT_IN_PREREQUISITE_STATE",
        "assert_drift_failure_preserves_state",
        "assert_ledger_drift_preserves_state",
        "assert_acl_drift_preserves_state",
        "MigrationApplyOutcome::AlreadyCurrent",
    ] {
        assert!(
            runtime_live.contains(required),
            "runtime live proof missing {required}"
        );
    }
}
#[test]
fn task076_store_migration_locks_global_and_memory_before_catalog_classification() {
    let setup = include_str!("../src/postgres_setup.rs");
    let apply = setup
        .split_once("pub fn apply_migrations")
        .expect("migration runner")
        .1
        .split_once("pub fn verify_postgres_schema")
        .expect("migration runner boundary")
        .0;
    let global = apply
        .find("&MIGRATION_ADVISORY_LOCK")
        .expect("global migration advisory lock");
    let memory = apply
        .find("&CODEBASE_MEMORY_ADVISORY_LOCK")
        .expect("Memory advisory lock");
    let writer = apply
        .find("&WRITER_LEASE_ADVISORY_LOCK")
        .expect("Writer Lease advisory lock");
    let classify = apply
        .find("classify_installed_manifest_state")
        .expect("installed profile classification");
    assert!(global < memory && memory < writer && writer < classify);

    let source = setup
        .split_once("fn verify_v3_upgrade_source")
        .expect("v3 upgrade source verifier")
        .1
        .split_once("fn v3_upgrade_source_has_memory")
        .expect("v3 verifier boundary")
        .0;
    let memory_tables = source
        .find("LOCK TABLE memory.codebase_memory_analyses")
        .expect("Memory source locks");
    let reclassify = source[memory_tables..]
        .find("classify_current_catalog_profile")
        .map(|offset| memory_tables + offset)
        .expect("locked profile reclassification");
    assert!(memory_tables < reclassify);
    assert!(!source.contains("LOCK TABLE writer_lease."));
    assert!(source.contains("CatalogProfile::V3CodebaseMemoryV2WriterLeaseV2Bridge"));
}

#[test]
fn task076_store_freezes_writer_v2_catalog_and_acl_profiles_without_semantic_rows() {
    let setup = include_str!("../src/postgres_setup.rs");
    for required in [
        "WRITER_LEASE_V2_SQL_SHA256",
        "WRITER_LEASE_V2_BRIDGE_CATALOG_PROFILES",
        "WRITER_LEASE_V2_CURRENT_CATALOG_PROFILES",
        "verify_writer_lease_v2_catalog",
        "verify_writer_lease_v2_function_sources",
        "LATTICE_WRITER_LEASE_SCHEMA_V2",
        "LATTICE_WRITER_LEASE_EXTENSION_IDENTITY_V2",
        "LATTICE_WRITER_LEASE_EXTENSION_LEDGER_V2",
        "writer_lease_bind_runtime_v2",
        "writer_lease_load_for_update_v2",
        "WriterLeaseV2RuntimeProfile::Bridge",
        "WriterLeaseV2RuntimeProfile::Current",
    ] {
        assert!(
            setup.contains(required),
            "missing exact Writer v2 sentinel: {required}"
        );
    }
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
            setup.contains(measured_bridge_signature),
            "missing measured Writer v2 bridge profile: {measured_bridge_signature}"
        );
    }
    assert!(setup.contains(
        "WriterLeaseV2RuntimeProfile::Bridge => &WRITER_LEASE_V2_BRIDGE_CATALOG_PROFILES"
    ));
    assert!(setup.contains(
        "WriterLeaseV2RuntimeProfile::Current => &WRITER_LEASE_V2_CURRENT_CATALOG_PROFILES"
    ));
    assert!(setup.contains("bd5b05d60340a1b9f9fbf1de2b4bed8586b7eede4fd8d7c4825841c221e89b7a"));
    assert!(!setup.contains("writer_lease_extension_identity i"));
    assert!(!setup.contains("writer_lease_extension_ledger l"));
    let writer_v2_catalog = setup
        .split_once("fn verify_writer_lease_v2_catalog")
        .expect("Writer v2 companion catalog verifier")
        .1
        .split_once("fn verify_writer_lease_v2_function_catalog")
        .expect("Writer v2 companion catalog verifier boundary")
        .0;
    assert!(!writer_v2_catalog.contains("AND NOT con.connoinherit"));
    assert!(!writer_v2_catalog.contains("AND NOT p.proretset"));
    assert!(
        setup.contains("('pg_catalog.pg_try_advisory_lock(bigint)', 'lattice_migrator'::text)")
    );
    assert!(setup.contains("WRITER_LEASE_V1_SQL"));
    assert!(setup.contains("WRITER_LEASE_V2_SQL"));
    assert!(setup.contains("WriterLeaseV2RuntimeProfile::Bridge => 0_i64"));
    assert!(setup.contains("WriterLeaseV2RuntimeProfile::Current => 7_i64"));
}

#[test]
fn task076_harness_stops_for_final_migration_verify_then_restores_restart_admission() {
    let harness = include_str!("../../../scripts/run-task019-postgres.ps1");
    let phase = harness
        .split_once("function Invoke-Task076WriterLeaseGatePhase")
        .expect("TASK-076 Writer gate phase")
        .1
        .split_once("function Get-PgIsReadyExitCode")
        .expect("TASK-076 Writer gate phase boundary")
        .0;
    let runtime = phase
        .find("-Phase 'runtime'")
        .expect("Writer runtime phase");
    let stopped = runtime
        + phase[runtime..]
            .find("-Mode 'STOPPED'")
            .expect("stopped migration boundary");
    let final_verify = phase
        .find("-Phase 'task076_final_verify'")
        .expect("final Store no-op verify");
    let base_access = phase
        .find("-Phase 'task076_writer_base_access'")
        .expect("initial base access proof");
    let restored = base_access
        + phase[base_access..]
            .find("-Mode 'ACTIVE'")
            .expect("restart admission restore");
    assert!(runtime < stopped && stopped < final_verify);
    assert!(base_access < restored);
    let writer_restart = phase
        .rfind("-Phase 'restart'")
        .expect("Writer restart proof");
    let restart_stopped = writer_restart
        + phase[writer_restart..]
            .find("-Mode 'STOPPED'")
            .expect("stopped restart verification boundary");
    let store_restart = phase
        .find("-Phase 'task076_writer_restart'")
        .expect("Store restart verifier");
    assert!(writer_restart < restart_stopped && restart_stopped < store_restart);
}

#[test]
fn task076_store_live_phases_are_closed_and_emit_fixed_pass_tokens() {
    let live = include_str!("postgres_live.rs");
    for phase in [
        "task076_writer_source_setup",
        "task076_global_upgrade",
        "task076_final_verify",
        "task076_writer_fresh_setup",
        "task076_writer_fresh_access",
        "task076_writer_base_access",
        "task076_writer_restart",
    ] {
        assert!(
            live.contains(phase),
            "missing TASK076 Store live phase: {phase}"
        );
    }
    for token in [
        "TASK076_WRITER_SOURCE_SETUP_OK",
        "TASK076_GLOBAL_UPGRADE_OK",
        "TASK076_FINAL_VERIFY_OK",
        "TASK076_WRITER_FRESH_G5_SETUP_OK",
        "TASK076_WRITER_FRESH_ACCESS_OK",
        "TASK076_WRITER_BASE_ACCESS_OK",
        "TASK076_WRITER_RESTART_OK",
    ] {
        assert!(
            live.contains(token),
            "missing TASK076 Store PASS token: {token}"
        );
    }
    assert!(live.contains("writer_fresh"));

    let access_start = live
        .find("fn run_task076_writer_access_phase")
        .expect("TASK076 single-target access phase");
    let access_end = live[access_start..]
        .find("\nfn run_initial_phase")
        .map(|offset| access_start + offset)
        .expect("TASK076 single-target access phase end");
    let access = &live[access_start..access_end];
    assert!(access.contains("matches!(database_tag, \"base\" | \"writer_fresh\")"));
    assert_eq!(
        access
            .matches("set_exact_database_access(&mut admin, &database_name)")
            .count(),
        1,
        "TASK076 access phase must switch all direct LOGIN ACLs to one database"
    );
    assert!(
        !live.contains("set_exact_task076_database_access"),
        "TASK076 must not grant direct LOGIN CONNECT to base and fresh together"
    );

    let helper_start = live
        .find("fn set_exact_database_access")
        .expect("single-target database ACL helper");
    let helper_end = live[helper_start..]
        .find("\nfn set_exact_pre_role_function_access")
        .map(|offset| helper_start + offset)
        .expect("single-target database ACL helper end");
    let helper = &live[helper_start..helper_end];
    for required in [
        "SELECT datname::text FROM pg_database ORDER BY datname",
        "REVOKE ALL ON DATABASE {quoted} FROM",
        "lattice_migrator_login, lattice_runtime_login",
        "lattice_guardian_login, lattice_readonly_login",
        "GRANT CONNECT ON DATABASE {quoted_target} TO",
    ] {
        assert!(
            helper.contains(required),
            "single-target database ACL helper drifted: {required}"
        );
    }
    assert_eq!(
        helper.matches("GRANT CONNECT ON DATABASE").count(),
        1,
        "single-target helper must grant exactly one database target"
    );
}

#[test]
fn schema_v6_manifest_preserves_registry_and_autonomy_before_foreman() {
    let manifest = migration_manifest();
    assert_eq!(POSTGRES_SCHEMA_VERSION, 6);
    assert_eq!(manifest.len(), 7);
    assert_eq!(
        verify_embedded_manifest()
            .expect("exact schema-v6 manifest")
            .manifest_sha256()
            .as_str(),
        "e2f1849bf17f78d60e921cbdcff01aced0516214c2216cb0e7c2f541b68ae439"
    );

    let registry = &manifest[4];
    assert_eq!(registry.ordinal(), 5);
    assert_eq!(registry.id(), "0005_project_registry_repository");
    assert_eq!(
        registry.path(),
        "db/migrations/0005_project_registry_repository.sql"
    );
    assert_eq!(registry.byte_length(), 200_547);
    assert_eq!(registry.sha256(), PROJECT_REGISTRY_REPOSITORY_SHA256);
    assert_eq!(registry.schema_version(), 4);
    assert_eq!(registry.reader_compatibility(), 4..=4);
    assert_eq!(registry.writer_compatibility(), 4..=4);

    let autonomy = &manifest[5];
    assert_eq!(autonomy.ordinal(), 6);
    assert_eq!(autonomy.id(), "0006_task_autonomy_receipt");
    assert_eq!(
        autonomy.path(),
        "db/migrations/0006_task_autonomy_receipt.sql"
    );
    assert_eq!(autonomy.schema_version(), 5);
    assert_eq!(autonomy.reader_compatibility(), 5..=5);
    assert_eq!(autonomy.writer_compatibility(), 5..=5);

    assert!(
        manifest
            .iter()
            .all(|entry| entry.path() != "db/migrations/0005_task_autonomy_receipt.sql")
    );
}

#[test]
fn task075_memory_gate_is_separate_from_legacy_memory_only_routing() {
    let live = include_str!("postgres_live.rs");
    let legacy_start = live
        .find("fn run_memory_setup_phase")
        .expect("legacy Memory setup phase");
    let legacy_end = live[legacy_start..]
        .find("\nfn run_initial_phase")
        .map(|offset| legacy_start + offset)
        .expect("legacy Memory setup phase boundary");
    let legacy = &live[legacy_start..legacy_end];
    assert!(legacy.contains("install_exact_v3(config, &base)"));
    assert!(legacy.contains("TASK_LEDGER_V3_MANIFEST_SHA256"));

    let current_start = live
        .find("fn run_task075_memory_setup_phase")
        .expect("TASK075 Memory setup phase");
    let current_end = live[current_start..]
        .find("\nfn run_initial_phase")
        .map(|offset| current_start + offset)
        .expect("TASK075 Memory setup phase boundary");
    let current = &live[current_start..current_end];
    assert!(current.contains("install_exact_v5(config, &base)"));
    assert!(!current.contains("install_codebase_memory_v2"));
    assert!(!current.contains("upgrade_codebase_memory_v3"));
    assert!(current.contains("CURRENT_V5_MANIFEST_SHA256"));

    let harness = include_str!("../../../scripts/run-task019-postgres.ps1");
    assert!(harness.contains("[switch]$RunTask075MemoryGate"));
    assert!(harness.contains("'task075_memory_setup'"));
    assert!(harness.contains("'V5_MEMORY_V3'"));
    assert!(harness.contains("-Task075MemoryGate:$RunTask075MemoryGate"));
}

#[test]
fn unknown_history_fixture_is_constraint_valid_before_verifier_rejection() {
    let live = include_str!("postgres_live.rs");
    let start = live
        .find("fn prove_history_shape_drift")
        .expect("history drift fixture");
    let end = live[start..]
        .find("\nfn prove_runtime_manifest_boundaries_fail_closed")
        .map(|offset| start + offset)
        .expect("history drift fixture boundary");
    let fixture = &live[start..end];

    assert!(
        fixture.contains("7, '0007_unknown', 'db/migrations/0007_unknown.sql', 1, repeat('1', 64)")
    );
    assert!(fixture.contains("'EXECUTABLE', 'RUNNER_OWNED', 5, 5, 5, 5, 5"));
    assert!(!fixture.contains("3, 3, 3, 3, 3 \\\n+                 'EXECUTABLE'"));
}

#[test]
fn current_manifest_substitution_is_isolated_from_unknown_history() {
    let live = include_str!("postgres_live.rs");
    let initial_start = live
        .find("fn run_initial_phase")
        .expect("initial live phase");
    let initial_end = live[initial_start..]
        .find("\nfn run_restart_phase")
        .map(|offset| initial_start + offset)
        .expect("initial live phase boundary");
    let initial = &live[initial_start..initial_end];
    assert!(initial.contains("prove_runtime_manifest_boundaries_fail_closed(config, &base)"));

    let history_start = live
        .find("fn prove_history_shape_drift")
        .expect("history drift fixture");
    let history_end = live[history_start..]
        .find("\nfn prove_runtime_manifest_boundaries_fail_closed")
        .map(|offset| history_start + offset)
        .expect("history drift fixture boundary");
    let history = &live[history_start..history_end];
    assert!(!history.contains("prove_runtime_manifest_boundaries_fail_closed"));
}

#[test]
fn misplaced_autonomy_0005_fixture_proves_pre_ddl_non_mutation() {
    let setup = include_str!("../src/postgres_setup.rs");
    let apply_start = setup
        .find("pub fn apply_migrations")
        .expect("migration runner");
    let apply_end = setup[apply_start..]
        .find("pub fn verify_postgres_schema")
        .map(|offset| apply_start + offset)
        .expect("migration runner boundary");
    let apply = &setup[apply_start..apply_end];
    let classify = apply
        .find("let installed = classify_installed_manifest_state")
        .expect("installed history classification");
    let migration_batch = apply
        .find("apply_missing_entries")
        .expect("migration DDL dispatch");
    assert!(classify < migration_batch);

    let live = include_str!("postgres_live.rs");
    let start = live
        .find("fn prove_misplaced_autonomy_0005_pre_ddl_rejection")
        .expect("misplaced autonomy live fixture");
    let end = live[start..]
        .find("\nfn ")
        .map(|offset| start + offset)
        .expect("misplaced autonomy live fixture boundary");
    let fixture = &live[start..end];
    for required in [
        "provision_database(config, admin, \"misplaced_auto\", true)",
        "0005_task_autonomy_receipt",
        "db/migrations/0005_task_autonomy_receipt.sql",
        "5dbf7439887ba30e8070bcb8883c1994e42a3d3a7ce78dc174771d3b89049436",
        "9378bbadf1e990e7d2617b66343b07193b2b8dd19bc8bb3dd6a3b618b134538a",
        "read_migration_history_fingerprint",
        "read_owned_catalog_fingerprint",
        "PostgresStoreSetupErrorKind::HistoryMismatch",
    ] {
        assert!(
            fixture.contains(required),
            "missing fixture proof: {required}"
        );
    }
    assert!(
        fixture
            .matches("read_migration_history_fingerprint")
            .count()
            >= 2
    );
    assert!(fixture.matches("read_owned_catalog_fingerprint").count() >= 2);

    let initial = live
        .split("fn run_initial_phase")
        .nth(1)
        .and_then(|source| source.split("fn run_restart_phase").next())
        .expect("initial live phase");
    let misplaced = initial
        .find("MISPLACED_AUTONOMY_0005_PRE_DDL")
        .expect("misplaced autonomy stage");
    let registry = initial
        .find("STORE_TASK022_STAGE_01_PROJECT_REGISTRY")
        .expect("registry stage");
    let between = &initial[misplaced..registry];
    assert!(between.contains("set_exact_database_access(&mut admin, base.database_name())"));
}

#[test]
fn schema_v5_adds_successors_and_registry_command_profile_provenance() {
    let migration = migration_manifest()
        .iter()
        .find(|entry| entry.id() == "0006_task_autonomy_receipt")
        .expect("schema-v5 autonomy migration");
    let sql = std::str::from_utf8(migration.bytes()).expect("UTF-8 SQL");
    let normalized = sql
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_uppercase();

    assert!(!normalized.contains("CREATE OR REPLACE FUNCTION"));
    assert!(!normalized.contains("DROP FUNCTION"));
    let store_finalize_v5 = normalized
        .split_once("CREATE FUNCTION CONTROL.STORE_FINALIZE_V5(")
        .expect("Store v5 finalizer")
        .1
        .split_once("$LATTICE_STORE_FINALIZE_V5$;")
        .expect("Store v5 finalizer terminator")
        .0;
    assert!(!store_finalize_v5.contains("CONTROL.STORE_PREPARE_V4("));
    assert!(store_finalize_v5.contains("CONTROL.STORE_PREPARE_V5("));
    for function in normalized.split("CREATE FUNCTION CONTROL.").skip(1) {
        let name = function.split_once('(').expect("function name").0;
        if name.ends_with("_V5") || name.ends_with("_V3") || name.ends_with("_V2") {
            assert!(
                !function.contains("V_MANIFEST_ENTRY_COUNT IS DISTINCT FROM 5"),
                "schema-v5 successor {name} retained a five-entry guard"
            );
        }
    }
    for successor in [
        "STORE_PREPARE_V5",
        "STORE_FINALIZE_V5",
        "STORE_CURRENT_HEAD_V5",
        "TASK_LEDGER_PREPARE_V3",
        "TASK_LEDGER_READ_HEAD_V3",
        "TASK_LEDGER_READ_EVENTS_V3",
        "TASK_LEDGER_READ_COMMANDS_V3",
        "TASK_LEDGER_FINALIZE_V3",
        "PROJECT_REGISTRY_PREPARE_V2",
        "PROJECT_REGISTRY_READ_STATE_V2",
        "PROJECT_REGISTRY_READ_OBSERVATIONS_V2",
        "PROJECT_REGISTRY_READ_PROJECTS_V2",
        "PROJECT_REGISTRY_READ_COMMANDS_V2",
        "PROJECT_REGISTRY_READ_RESERVATIONS_V2",
        "PROJECT_REGISTRY_STAGE_COMMAND_V2",
        "PROJECT_REGISTRY_STAGE_PROJECT_V2",
        "PROJECT_REGISTRY_FINALIZE_V2",
    ] {
        assert!(
            normalized.contains(&format!("CREATE FUNCTION CONTROL.{successor}(")),
            "missing schema-v5 successor {successor}"
        );
    }
    for autonomy in [
        "TASK_LEDGER_RECORD_AUTONOMY_RECEIPT_V1",
        "TASK_LEDGER_READ_AUTONOMY_RECEIPTS_V1",
    ] {
        assert!(
            normalized.contains(&format!("CREATE FUNCTION CONTROL.{autonomy}")),
            "missing schema-v5 autonomy function {autonomy}"
        );
    }

    assert!(normalized.contains(
        "ALTER TABLE CONTROL.PROJECT_REGISTRY_COMMANDS ADD COLUMN PERSISTENCE_SCHEMA_VERSION SMALLINT"
    ));
    assert!(normalized.contains(
        "ALTER TABLE CONTROL.PROJECT_REGISTRY_COMMANDS ADD COLUMN PERSISTENCE_MANIFEST_SHA256 TEXT"
    ));
    assert!(normalized.contains("P_PERSISTENCE_SCHEMA_VERSION SMALLINT"));
    assert!(normalized.contains("P_PERSISTENCE_MANIFEST_SHA256 TEXT"));
    assert!(normalized.contains("P_PERSISTENCE_SCHEMA_VERSION, P_PERSISTENCE_MANIFEST_SHA256"));
}

#[test]
fn task075_registry_v4_v5_live_fixture_is_closed() {
    let fixture = include_str!("postgres_live.rs");

    for required in [
        "task075_seed_v5_registry(config, admin, \"reg_cross_four\")",
        "task075_seed_v5_registry(config, admin, \"reg_cross_five\")",
        "provision_database(config, admin, \"three_memory\", true)",
        "REGISTRY_V4_MANIFEST_SHA256",
        "fn install_exact_v4",
        "fn prove_exact_nonempty_v4_registry_upgrade_and_mixed_replay",
        "fn prove_task075_registry_mixed_restart",
        "task075-registry-mixed-restart-access",
        "set_exact_database_access(&mut restart_admin, &mixed_database)",
        "fn prove_task075_registry_provenance_corruption",
        "PROJECT_REGISTRY_STAGE_COMMAND_V1",
        "PROVENANCE_OMISSION",
        "PROVENANCE_MUTATION",
        "PROVENANCE_CROSS_PAIR",
        "CURRENT_PROFILE_SUBSTITUTION",
        "COHERENT_PREFIX_ROLLBACK",
    ] {
        assert!(
            fixture.contains(required),
            "missing Registry v4-v5 live proof: {required}"
        );
    }
}

#[test]
fn schema_v5_global_upgrade_locks_memory_before_classification_and_rechecks_v2_tables() {
    let source = include_str!("../src/postgres_setup.rs");
    let apply_start = source
        .find("pub fn apply_migrations")
        .expect("migration entry point");
    let apply_end = source[apply_start..]
        .find("pub fn verify_postgres_schema")
        .map(|offset| apply_start + offset)
        .expect("migration entry point boundary");
    let apply = &source[apply_start..apply_end];
    let global_lock = apply
        .find("&MIGRATION_ADVISORY_LOCK")
        .expect("global advisory lock");
    let memory_lock = apply
        .find("&CODEBASE_MEMORY_ADVISORY_LOCK")
        .expect("Memory advisory lock");
    let history_classification = apply
        .find("classify_installed_manifest_state")
        .expect("installed history classification");
    assert!(global_lock < memory_lock && memory_lock < history_classification);

    let upgrade_start = source
        .find("fn verify_v3_upgrade_source")
        .expect("V3 upgrade verifier");
    let upgrade_end = source[upgrade_start..]
        .find("fn v3_upgrade_source_has_memory")
        .map(|offset| upgrade_start + offset)
        .expect("V3 upgrade verifier boundary");
    let upgrade = &source[upgrade_start..upgrade_end];
    for table in [
        "memory.codebase_memory_analyses",
        "memory.codebase_memory_extension_identity",
        "memory.codebase_memory_extension_ledger",
        "memory.codebase_memory_receipts",
        "memory.codebase_memory_records",
        "memory.codebase_memory_reflections",
        "memory.codebase_memory_retrieval_audits",
        "memory.openclaw_gateway_commands",
    ] {
        assert!(upgrade.contains(&format!("LOCK TABLE {table} IN ACCESS EXCLUSIVE MODE")));
    }
    let locked = upgrade
        .find("LOCK TABLE memory.codebase_memory_analyses")
        .expect("Memory table lock");
    let reclassified = upgrade[locked..]
        .find("classify_current_catalog_profile(client, 3)")
        .expect("post-lock Memory reclassification");
    assert!(reclassified > 0);
}

#[test]
fn schema_v5_memory_identity_and_ledger_require_migrator_authority() {
    let setup = include_str!("../src/postgres_setup.rs");
    let runtime_start = setup
        .find("pub(crate) fn verify_runtime_store_schema")
        .expect("runtime verifier exists");
    let runtime_end = setup[runtime_start..]
        .find("\nfn preflight_connection")
        .map(|offset| runtime_start + offset)
        .expect("runtime verifier boundary exists");
    let runtime = &setup[runtime_start..runtime_end];
    assert!(!runtime.contains("verify_codebase_memory_v3_identity"));

    let live = include_str!("postgres_live.rs");
    for required in [
        "codebase_memory_extension_identity",
        "ledger_ordinal = 1",
        "ledger_ordinal = 2",
        "PostgresStoreSetupErrorKind::CompatibilityMismatch",
        "TASK075_MEMORY_V3_ADMIN_SUBSTITUTION_FAILED",
        "verify_postgres_schema(&mut migrator, &target, DatabaseRole::Migrator)",
        "TASK075_MEMORY_V3_ADMIN_SUBSTITUTION_RESTORE_FAILED",
        "TASK075_MEMORY_V3_RUNTIME_PROFILE_REJECTED",
    ] {
        assert!(
            live.contains(required),
            "missing runtime identity gate: {required}"
        );
    }
}

#[test]
fn schema_v5_forbidden_object_closure_counts_autonomy_reference_triggers() {
    let setup = include_str!("../src/postgres_setup.rs");
    assert!(setup.contains("CatalogProfile::V5 => 42"));
    let trigger_counts = setup
        .split_once("fn expected_internal_trigger_count")
        .expect("internal-trigger classifier")
        .1
        .split_once("fn verify_owned_function_boundary")
        .expect("internal-trigger classifier boundary")
        .0;
    for profile in [
        "CatalogProfile::V5CodebaseMemoryV2UpgradePending",
        "CatalogProfile::V5CodebaseMemoryV3Current",
        "CatalogProfile::V5CodebaseMemoryV2WriterLeaseV2BridgePending",
        "CatalogProfile::V5CodebaseMemoryV3WriterLeaseV2BridgePending",
        "CatalogProfile::V5CodebaseMemoryV3WriterLeaseV2Current",
    ] {
        assert!(trigger_counts.contains(profile));
    }
    assert!(trigger_counts.contains("=> 66"));
    let migration = include_str!("../../../db/migrations/0006_task_autonomy_receipt.sql");
    assert_eq!(
        migration
            .matches("FOREIGN KEY (stream_id, event_sequence)")
            .count(),
        1
    );
    assert!(migration.contains("REFERENCES control.task_ledger_events (stream_id, sequence)"));
}

#[test]
// Keep the emitted signature, role, ACL, diagnostic, and holder-receipt
// assertions together; they define one closed measurement surface.
#[allow(clippy::too_many_lines)]
fn schema_v5_catalog_measurement_has_closed_role_boundary_diagnostics() {
    let setup = include_str!("../src/postgres_setup.rs");
    let compact = setup.split_whitespace().collect::<String>();
    for diagnostic in [
        "ROLE_SIGNATURE",
        "DB_ACL_SIGNATURE",
        "OWNER",
        "TEMPLATE",
        "ALLOWCONN",
        "CONNLIMIT",
        "MEMBERSHIPS",
        "EXTRA_ROLES",
        "ROLE_SETTINGS",
        "DANGEROUS_FUNCTIONS",
        "DB_PRIVILEGES",
        "CLUSTER_ACL",
        "LOGIN_CLOSURE",
    ] {
        assert!(
            compact.contains(&format!("diagnose_role_boundary(\"{diagnostic}\",")),
            "missing static role-boundary diagnostic: {diagnostic}"
        );
    }
    assert!(setup.contains("ROLE_DATABASE_BOUNDARY_SQL"));

    let live = include_str!("postgres_live.rs");
    assert!(!live.contains("set_exact_signature_fixture_database_access"));
    let harness = include_str!("../../../scripts/run-task019-postgres.ps1");
    let switch_start = harness
        .find("function New-Task075CatalogDatabaseAccessQuery")
        .expect("catalog database ACL switch exists");
    let switch_end = harness[switch_start..]
        .find("\nfunction Invoke-Task068HermesReplayGate")
        .map(|offset| switch_start + offset)
        .expect("catalog database ACL switch boundary exists");
    let database_acl_switch = &harness[switch_start..switch_end];
    assert_eq!(
        database_acl_switch
            .matches("GRANT CONNECT ON DATABASE")
            .count(),
        1,
        "measurement fixture must grant login access to one current database only"
    );
    assert!(database_acl_switch.contains("REVOKE ALL ON DATABASE $($quotedTargets -join ', ')"));
    let revoke_start = database_acl_switch
        .find("REVOKE ALL ON DATABASE")
        .expect("catalog database ACL revocation exists");
    let revoke_end = database_acl_switch[revoke_start..]
        .find(';')
        .map(|offset| revoke_start + offset)
        .expect("catalog database ACL revocation is bounded");
    let revoke = &database_acl_switch[revoke_start..revoke_end];
    for capability in [
        "lattice_migrator,",
        "lattice_runtime,",
        "lattice_guardian,",
        "lattice_readonly,",
    ] {
        assert!(
            !revoke.contains(capability),
            "measurement ACL switch must preserve capability and owner grants: {capability}"
        );
    }
    for login in [
        "lattice_migrator_login",
        "lattice_runtime_login",
        "lattice_guardian_login",
        "lattice_readonly_login",
    ] {
        assert!(revoke.contains(login), "missing login revocation: {login}");
    }
    for required in [
        "[switch]$MeasureTask075CurrentCatalog",
        "-CurrentOnly:$MeasureTask075CurrentCatalog",
        "if ($CurrentOnly) { 17 } else { 43 }",
        "LATTICE_TASK075_CURRENT_CATALOG_ONLY",
        "CATALOG_SIGNATURES_PARTIAL",
        "-RecordPartial:$CurrentOnly",
    ] {
        assert!(
            harness.contains(required),
            "missing current-only measurement gate: {required}"
        );
    }
    assert!(live.contains("let current_only = env::var(\"LATTICE_TASK075_CURRENT_CATALOG_ONLY\")"));
    for required in [
        "function Get-Task019AllowlistedDiagnosticTokens",
        "(?:TASK019|TASK075|TASK076|STORE|POSTGRES_TASK_LEDGER|POSTGRES_PROJECT_REGISTRY|MEMORY|WRITER_LEASE|OPENCLAW)",
        "function Get-Task019SafeDiagnosticSummary",
        "No allowlisted static diagnostic was emitted.",
        "EventType 'LIVE_GATE_FAILED'",
        "diagnostics = $safeTokens",
        "TASK075_FAILURE_DIAGNOSTIC_SELF_TEST_REJECTED_ALLOWLIST",
        "TASK075_FAILURE_DIAGNOSTIC_SELF_TEST_REJECTED_ZERO_TOKEN_FALLBACK",
        "function Get-Task075LastIncompleteStageToken",
        "last_incomplete_task075_stage = $lastIncompleteTask075Stage",
    ] {
        assert!(
            harness.contains(required),
            "missing safe live failure evidence: {required}"
        );
    }
    assert!(!harness.contains("diagnostics = $suiteOutput"));

    for stage in [
        "FRESH_V5_RECONCILIATION",
        "V3_MEMORY_V2_GLOBAL_UPGRADE",
        "V3_MEMORY_V2_SOURCE",
        "GLOBAL_V5_PENDING",
        "PENDING_RUNTIME_REJECTION",
        "MEMORY_V3_UPGRADE",
        "MEMORY_V3_LEDGER_FK",
        "MEMORY_V3_CURRENT_ROLES",
        "MEMORY_V3_IDENTITY_SUBSTITUTION",
        "MEMORY_V3_ADMIN_IDENTITY_SUBSTITUTIONS",
        "FIRST_APPLY",
        "FIRST_VERIFY",
        "MANIFEST_RECOMPUTE",
        "SECOND_APPLY",
        "SECOND_VERIFY",
    ] {
        assert!(
            live.contains(&format!("\"{stage}\"")),
            "missing fixed TASK075 live stage: {stage}"
        );
    }
    assert!(live.contains("concat!(\"TASK075_STAGE_ENTER_\", $name)"));
    assert!(live.contains("concat!(\"TASK075_STAGE_PASS_\", $name)"));
}

#[test]
fn schema_v5_catalog_measurement_has_closed_forbidden_object_diagnostics() {
    let setup = include_str!("../src/postgres_setup.rs");
    for diagnostic in [
        "FUNCTION_COUNT",
        "NONINTERNAL_TRIGGER",
        "REWRITE",
        "POLICY",
        "SPECIAL_TYPE",
        "EVENT_TRIGGER",
        "SCOPE_TRIGGER",
        "INTERNAL_TRIGGER",
        "INHERITS",
        "SUBCLASS",
    ] {
        assert!(
            setup.contains(&format!("\"{diagnostic}\"")),
            "missing static forbidden-object diagnostic: {diagnostic}"
        );
    }
    assert!(setup.contains("FORBIDDEN_SCHEMA_OBJECTS_SQL"));
    assert!(setup.contains("diagnose_forbidden_schema_object("));
    assert!(setup.contains("diagnose_forbidden_schema_objects(&mut client, profile)"));

    let memory_v3 = include_str!("../../../db/extensions/codebase-memory/v3.sql");
    assert!(!memory_v3.contains("DROP CONSTRAINT codebase_memory_extension_ledger_identity_fk"));
    let live = include_str!("postgres_live.rs");
    for required in [
        "prove_memory_v3_ledger_identity_fk(config, &target)",
        "c.conname = 'codebase_memory_extension_ledger_identity_fk'",
        "c.contype = 'f' AND c.convalidated",
        "TASK075_MEMORY_V3_LEDGER_HISTORY_NOT_TWO_ROWS",
        "DROP CONSTRAINT codebase_memory_extension_ledger_identity_fk",
        "ADD CONSTRAINT codebase_memory_extension_ledger_identity_fk",
        "PostgresStoreSetupErrorKind::CorruptCatalog",
    ] {
        assert!(
            live.contains(required),
            "missing Memory v3 FK gate: {required}"
        );
    }
}

#[test]
// Each migration entry is asserted in order here so a partial manifest check
// cannot accidentally bless a reordered or substituted historical prefix.
#[allow(clippy::too_many_lines)]
fn manifest_is_closed_ordered_and_preserves_the_superseded_bootstrap() {
    let manifest = migration_manifest();
    assert_eq!(manifest.len(), 7);

    let draft = &manifest[0];
    assert_eq!(draft.ordinal(), 1);
    assert_eq!(draft.id(), "0001_bootstrap_draft");
    assert_eq!(draft.path(), "db/migrations/0001_bootstrap.sql");
    assert_eq!(draft.byte_length(), 312);
    assert_eq!(draft.sha256(), BOOTSTRAP_SHA256);
    assert_eq!(draft.status(), MigrationStatus::Superseded);
    assert_eq!(
        draft.transaction_mode(),
        MigrationTransactionMode::NotExecuted
    );
    assert_eq!(draft.schema_version(), 0);
    assert_eq!(draft.reader_compatibility(), 0..=0);
    assert_eq!(draft.writer_compatibility(), 0..=0);

    let foundation = &manifest[1];
    assert_eq!(foundation.ordinal(), 2);
    assert_eq!(foundation.id(), "0002_control_store_foundation");
    assert_eq!(
        foundation.path(),
        "db/migrations/0002_control_store_foundation.sql"
    );
    assert!(foundation.byte_length() > 0);
    assert_eq!(foundation.byte_length(), 14_259);
    assert_eq!(foundation.sha256(), FOUNDATION_SHA256);
    assert_eq!(foundation.status(), MigrationStatus::Executable);
    assert_eq!(
        foundation.transaction_mode(),
        MigrationTransactionMode::RunnerOwned
    );
    assert_eq!(foundation.schema_version(), 1);
    assert_eq!(foundation.reader_compatibility(), 1..=1);
    assert_eq!(foundation.writer_compatibility(), 1..=1);

    let live_store = &manifest[2];
    assert_eq!(live_store.ordinal(), 3);
    assert_eq!(live_store.id(), "0003_live_control_store");
    assert_eq!(
        live_store.path(),
        "db/migrations/0003_live_control_store.sql"
    );
    assert_eq!(live_store.byte_length(), 29_518);
    assert_eq!(live_store.sha256(), LIVE_CONTROL_STORE_SHA256);
    assert_eq!(live_store.status(), MigrationStatus::Executable);
    assert_eq!(
        live_store.transaction_mode(),
        MigrationTransactionMode::RunnerOwned
    );
    assert_eq!(live_store.schema_version(), 2);
    assert_eq!(live_store.reader_compatibility(), 2..=2);
    assert_eq!(live_store.writer_compatibility(), 2..=2);

    let task_ledger = &manifest[3];
    assert_eq!(task_ledger.ordinal(), 4);
    assert_eq!(task_ledger.id(), "0004_task_ledger_repository");
    assert_eq!(
        task_ledger.path(),
        "db/migrations/0004_task_ledger_repository.sql"
    );
    assert!(task_ledger.byte_length() > 0);
    assert_eq!(task_ledger.sha256().len(), 64);
    assert_eq!(task_ledger.status(), MigrationStatus::Executable);
    assert_eq!(
        task_ledger.transaction_mode(),
        MigrationTransactionMode::RunnerOwned
    );
    assert_eq!(task_ledger.schema_version(), 3);
    assert_eq!(task_ledger.reader_compatibility(), 3..=3);
    assert_eq!(task_ledger.writer_compatibility(), 3..=3);

    let registry = &manifest[4];
    assert_eq!(registry.ordinal(), 5);
    assert_eq!(registry.id(), "0005_project_registry_repository");
    assert_eq!(
        registry.path(),
        "db/migrations/0005_project_registry_repository.sql"
    );
    assert_eq!(registry.byte_length(), 200_547);
    assert_eq!(registry.sha256(), PROJECT_REGISTRY_REPOSITORY_SHA256);
    assert_eq!(registry.status(), MigrationStatus::Executable);
    assert_eq!(registry.schema_version(), 4);
    assert_eq!(registry.reader_compatibility(), 4..=4);
    assert_eq!(registry.writer_compatibility(), 4..=4);

    let autonomy = &manifest[5];
    assert_eq!(autonomy.ordinal(), 6);
    assert_eq!(autonomy.id(), "0006_task_autonomy_receipt");
    assert_eq!(
        autonomy.path(),
        "db/migrations/0006_task_autonomy_receipt.sql"
    );
    assert_eq!(autonomy.status(), MigrationStatus::Executable);
    assert_eq!(
        autonomy.transaction_mode(),
        MigrationTransactionMode::RunnerOwned
    );
    assert_eq!(autonomy.schema_version(), 5);
    assert_eq!(autonomy.reader_compatibility(), 5..=5);
    assert_eq!(autonomy.writer_compatibility(), 5..=5);

    let evidence = verify_embedded_manifest().expect("embedded manifest");
    let foreman = &manifest[6];
    assert_eq!(foreman.ordinal(), 7);
    assert_eq!(foreman.id(), "0007_foreman_coordination");
    assert_eq!(
        foreman.path(),
        "db/migrations/0007_foreman_coordination.sql"
    );
    assert_eq!(foreman.byte_length(), 217_206);
    assert_eq!(
        foreman.sha256(),
        "36ae91884e849c63ffe2a4b7013b4dea7a8f1a4bc63305d4a91a04808d316f9c"
    );
    assert_eq!(foreman.schema_version(), POSTGRES_SCHEMA_VERSION);
    assert_eq!(foreman.reader_compatibility(), 6..=6);
    assert_eq!(foreman.writer_compatibility(), 6..=6);

    assert_eq!(evidence.entry_count(), 7);
    assert_eq!(evidence.executable_count(), 6);
    assert_eq!(evidence.schema_version(), POSTGRES_SCHEMA_VERSION);
    assert_eq!(evidence.manifest_sha256().as_str().len(), 64);

    let live = include_str!("postgres_live.rs");
    assert!(
        live.contains("executable_count: 5"),
        "fresh setup must stop at the exact v5 bridge predecessor before the one-entry v6 transition"
    );
    let task_ledger = include_str!("postgres_task_ledger.rs");
    assert_eq!(
        task_ledger.matches("executable_count: 6").count(),
        1,
        "only the exact-prefix Ledger fixture may derive its executable count from six entries"
    );
    assert!(task_ledger.contains("executable_count: 6 - prefix_len"));
    assert!(
        task_ledger.contains("executable_count: 5"),
        "historical schema-v5 fixture evidence remains explicit"
    );
}

#[test]
fn foreman_migration_is_event_bound_fenced_and_table_acl_closed() {
    let migration = &migration_manifest()[6];
    let sql = std::str::from_utf8(migration.bytes()).expect("UTF-8 SQL");
    let normalized = sql.split_whitespace().collect::<Vec<_>>().join(" ");
    for required in [
        "CREATE TABLE control.task_ledger_foreman_snapshots",
        "FOREMAN_SNAPSHOT_RECORDED",
        "FOREIGN KEY (stream_id, event_sequence) REFERENCES control.task_ledger_events",
        "writer_lease.writer_lease_assert_current_v1(",
        "xmin = pg_catalog.pg_current_xact_id()::xid",
        "CREATE FUNCTION control.task_ledger_record_foreman_snapshot_v1(",
        "CREATE FUNCTION control.task_ledger_read_foreman_snapshots_v1(",
        "'EVIDENCE_RECORDED', 'FOREMAN_SNAPSHOT_RECORDED'",
        "REVOKE ALL ON TABLE control.task_ledger_foreman_snapshots FROM lattice_runtime",
        "worker_id !~* '^(sk-|bearer )|password|full chat|begin private'",
        "heartbeat_digest_ref ~ '^heartbeat:sha256:[0-9a-f]{64}$'",
        "authority_digest_ref ~ '^authority:sha256:[0-9a-f]{64}$'",
        "decision_ref ~ '^decision:sha256:[0-9a-f]{64}$'",
    ] {
        assert!(
            normalized.contains(required),
            "missing 0007 contract: {required}"
        );
    }
    assert!(!normalized.contains("CREATE TABLE control.foreman_current_state"));
    assert!(!normalized.contains("CREATE TABLE control.task_ledger_autonomy_receipts"));
    assert!(!normalized.contains("GRANT SELECT ON TABLE control.task_ledger_foreman_snapshots"));
    assert_eq!(
        normalized
            .matches("CREATE OR REPLACE FUNCTION control.")
            .count(),
        19,
        "schema-v6 must rebind the exact existing runtime surface"
    );
    assert_eq!(
        normalized
            .matches("v_manifest_entry_count IS DISTINCT FROM 7")
            .count(),
        20,
        "every rebound function must verify all seven retained manifest entries"
    );
    assert!(!normalized.contains("v_manifest_entry_count IS DISTINCT FROM 6"));
    assert!(normalized.contains("LATTICE_DEVOS_CONTROL_SCHEMA_V6"));
}

#[test]
fn foreman_adapter_uses_the_ledger_transaction_and_verified_fresh_replay() {
    let adapter = include_str!("../src/task_ledger.rs");
    let attempt_start = adapter
        .find("fn run_execute_attempt(")
        .expect("Task Ledger transaction");
    let attempt = &adapter[attempt_start..];
    let writer = attempt
        .find("assert_writer_authority(&mut transaction")
        .expect("same-transaction writer assertion");
    let ledger = attempt
        .find("sql_profile.ledger_finalize_sql()")
        .expect("Ledger finalize");
    let child = attempt
        .find("LEDGER_RECORD_FOREMAN_SNAPSHOT_SQL")
        .expect("foreman child write");
    let commit = child
        + attempt[child..]
            .find("transaction\n        .commit()")
            .expect("transaction commit after child verification");
    assert!(writer < ledger && ledger < child && child < commit);
    assert!(adapter.contains("verify_untrusted_foreman_snapshot_rows(stream, &untrusted)"));
    assert!(adapter.contains("IsolationLevel::RepeatableRead"));
    assert!(adapter.contains("IsolationLevel::Serializable"));
    assert!(adapter.contains("if plan.is_exact_retry()"));
}

#[test]
fn schema_v6_runtime_admission_requires_writer_v3_current_and_closed_acl() {
    let setup = include_str!("../src/postgres_setup.rs");
    let verifier = setup
        .split_once("fn verify_runtime_foreman_schema_v6")
        .expect("schema-v6 runtime verifier")
        .1
        .split_once("fn preflight_connection")
        .expect("verifier boundary")
        .0;
    for required in [
        "verify_history_rows(&rows, migration_manifest())",
        "current_schema_version",
        "task_ledger_foreman_snapshots",
        "has_table_privilege",
        "task_ledger_record_foreman_snapshot_v1",
        "verify_writer_lease_v3_functions(client, true)",
        "n.nspname='writer_lease'",
        "WriterLeaseV3Profile::Current",
        "verify_runtime_admission_present(client)",
    ] {
        assert!(
            verifier.contains(required),
            "missing v6 admission proof: {required}"
        );
    }
    assert!(
        verifier.contains("!= 7"),
        "runtime Writer surface must be exact"
    );
    assert!(!verifier.contains("WriterLeaseV3Profile::Bridge"));
}

#[test]
fn autonomy_receipt_migration_is_fixed_scalar_event_owned_and_function_gated() {
    let migration = migration_manifest()
        .iter()
        .find(|entry| entry.id() == "0006_task_autonomy_receipt")
        .expect("autonomy receipt migration");
    let sql = std::str::from_utf8(migration.bytes()).expect("UTF-8 SQL");
    let normalized = sql
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_uppercase();
    assert!(normalized.contains("CREATE TABLE CONTROL.TASK_LEDGER_AUTONOMY_RECEIPTS ("));
    assert!(normalized.contains("AUTONOMY_RECEIPT_RECORDED"));
    assert!(normalized.contains("LATTICE.AUTONOMY-RECEIPT/1.0"));
    assert!(normalized.contains("REFERENCES CONTROL.TASK_LEDGER_EVENTS (STREAM_ID, SEQUENCE)"));
    assert!(normalized.contains("CREATE FUNCTION CONTROL.TASK_LEDGER_RECORD_AUTONOMY_RECEIPT_V1("));
    assert!(normalized.contains("CREATE FUNCTION CONTROL.TASK_LEDGER_READ_AUTONOMY_RECEIPTS_V1("));
    assert!(!normalized.contains("RECEIPT JSON"));
    assert!(!normalized.contains("SUBJECT JSON"));
    assert!(!normalized.contains("GRANT SELECT ON CONTROL.TASK_LEDGER_AUTONOMY_RECEIPTS"));
}

#[test]
fn autonomy_receipt_functions_fail_closed_on_role_input_and_changed_exact_retry() {
    let migration = migration_manifest()
        .iter()
        .find(|entry| entry.id() == "0006_task_autonomy_receipt")
        .expect("autonomy receipt migration");
    let sql = std::str::from_utf8(migration.bytes()).expect("UTF-8 SQL");
    let autonomy_sql = sql
        .split_once("CREATE FUNCTION control.task_ledger_record_autonomy_receipt_v1(")
        .expect("autonomy function section")
        .1;

    assert_eq!(
        autonomy_sql
            .matches("pg_catalog.current_setting('role') <> 'lattice_runtime'")
            .count(),
        2,
        "both SECURITY DEFINER functions must reject an ambient login role"
    );
    assert_eq!(
        autonomy_sql
            .matches("pg_catalog.octet_length(p_stream_id) <> 32")
            .count(),
        2,
        "record and read must reject malformed stream identifiers"
    );
    assert!(
        autonomy_sql.contains("v_existing.event_sequence::text IS DISTINCT FROM p_event_sequence")
    );

    for field in [
        "event_digest",
        "receipt_schema_version",
        "intent_version",
        "task_kind",
        "risk_class",
        "execution_preapproved",
        "requires_new_authority",
        "irreversible_or_high_risk",
        "observed_task_state",
        "disposition",
        "decision_reason",
        "model",
        "verification",
        "authority_mode",
        "process_start_authority_digest",
        "ingress_profile_adapter_commitment",
        "store_authority_head_digest",
        "writer_lease_receipt_digest",
        "writer_lease_head_digest",
        "authority_digest",
        "receipt_digest",
    ] {
        assert!(
            autonomy_sql.contains(&format!("v_existing.{field} IS DISTINCT FROM p_{field}")),
            "RETAINED must compare canonical field {field}"
        );
    }
    assert!(
        autonomy_sql.contains(
            "v_existing.writer_fencing_token::text IS DISTINCT FROM p_writer_fencing_token"
        )
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn task_ledger_repository_migration_is_fixed_bounded_and_function_gated() {
    let repository = migration_manifest()
        .iter()
        .find(|entry| entry.id() == "0004_task_ledger_repository")
        .expect("Task Ledger repository migration");
    let sql = std::str::from_utf8(repository.bytes()).expect("UTF-8 SQL");
    let uppercase = sql.to_ascii_uppercase();
    let normalized = sql
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_uppercase();

    for forbidden in [
        "BEGIN;",
        "COMMIT;",
        "ROLLBACK;",
        "IF NOT EXISTS",
        "DO $$",
        "EXECUTE FORMAT",
        "EXECUTE IMMEDIATE",
        "CREATE EXTENSION",
        "CREATE ROLE",
        "ALTER ROLE",
        "PASSWORD",
        "CREATE DATABASE",
        "DROP TABLE",
        "DROP SCHEMA",
        "DROP FUNCTION",
    ] {
        assert!(
            !uppercase.contains(forbidden),
            "forbidden Task Ledger migration surface: {forbidden}"
        );
    }

    for table in [
        "TASK_LEDGER_STREAMS",
        "TASK_LEDGER_EVENTS",
        "TASK_LEDGER_COMMANDS",
        "TASK_LEDGER_OUTBOX",
    ] {
        assert!(normalized.contains(&format!("CREATE TABLE CONTROL.{table} (")));
        assert!(!normalized.contains(&format!("GRANT SELECT ON CONTROL.{table}")));
        assert!(!normalized.contains(&format!("GRANT INSERT ON CONTROL.{table}")));
        assert!(!normalized.contains(&format!("GRANT UPDATE ON CONTROL.{table}")));
        assert!(!normalized.contains(&format!("GRANT DELETE ON CONTROL.{table}")));
    }
    assert_eq!(
        uppercase
            .matches("CREATE TABLE CONTROL.TASK_LEDGER_")
            .count(),
        4
    );

    for function in [
        "STORE_PREPARE_V3",
        "STORE_FINALIZE_V3",
        "STORE_CURRENT_HEAD_V3",
        "TASK_LEDGER_PREPARE_V1",
        "TASK_LEDGER_READ_HEAD_V1",
        "TASK_LEDGER_READ_EVENTS_V1",
        "TASK_LEDGER_READ_COMMANDS_V1",
        "TASK_LEDGER_FINALIZE_V1",
    ] {
        assert!(normalized.contains(&format!("CREATE FUNCTION CONTROL.{function}(")));
        assert!(normalized.contains(&format!("REVOKE ALL ON FUNCTION CONTROL.{function}(")));
        assert!(normalized.contains(&format!("GRANT EXECUTE ON FUNCTION CONTROL.{function}(")));
    }
    assert_eq!(uppercase.matches("CREATE FUNCTION CONTROL.").count(), 8);
    assert_eq!(uppercase.matches("SECURITY DEFINER").count(), 8);
    assert_eq!(uppercase.matches("SET SEARCH_PATH = PG_CATALOG").count(), 8);
    assert_eq!(uppercase.matches("SET ROW_SECURITY = ON").count(), 8);
    assert_eq!(uppercase.matches("SET LOCK_TIMEOUT = '5S'").count(), 8);
    assert_eq!(
        uppercase.matches("SET STATEMENT_TIMEOUT = '30S'").count(),
        8
    );
    assert_eq!(
        sql.lines()
            .filter(|line| line.trim() == "global_schema_version smallint,")
            .count(),
        3
    );
    assert_eq!(
        sql.lines()
            .filter(|line| line.trim() == "global_manifest_sha256 text")
            .count(),
        3
    );

    for historical in [
        "STORE_PREPARE_V2",
        "STORE_FINALIZE_V2",
        "STORE_CURRENT_HEAD_V2",
    ] {
        assert!(normalized.contains(&format!("REVOKE EXECUTE ON FUNCTION CONTROL.{historical}(")));
    }

    for required in [
        "NUMERIC(20,0)",
        "18446744073709551615",
        "JSONB_PATH_EXISTS",
        "TYPE() == \"NUMBER\"",
        "LATTICE_DEVOS_CONTROL_SCHEMA_V3",
        "4582EDCE68A947998A8F4C6895BB37CEEC9E842F516471F4D9E2617A6757F129",
        "FROM ONLY CONTROL.TASK_LEDGER_STREAMS",
        "FROM ONLY CONTROL.TASK_LEDGER_EVENTS",
        "FROM ONLY CONTROL.TASK_LEDGER_COMMANDS",
        "FROM ONLY CONTROL.TASK_LEDGER_OUTBOX",
        "UNIQUE (INTENT_DIGEST)",
        "P_PROJECT_SNAPSHOT_ID !~ '^[A-Z0-9._:-]{1,128}$'",
        "V_PHYSICAL_COUNT > 1",
        "P_APPEND_EVENT AND P_EVENT_KIND = 'EFFECT_INTENT' AND P_AUDIT_OUTCOME = 'RECORDED'",
        "V_TERMINAL.EXPECTED_REVISION::NUMERIC IS DISTINCT FROM P_NEXT_COMMAND_COUNT::NUMERIC - 1",
        "V_TERMINAL.BEFORE_REVISION::NUMERIC IS DISTINCT FROM P_NEXT_COMMAND_COUNT::NUMERIC - 1",
        "V_TERMINAL.AFTER_REVISION::NUMERIC IS DISTINCT FROM P_NEXT_COMMAND_COUNT::NUMERIC",
        "TERMINAL_RECEIPT_DIGEST BYTEA, GLOBAL_SCHEMA_VERSION SMALLINT, GLOBAL_MANIFEST_SHA256 TEXT",
        "HEAD_DIGEST BYTEA, GLOBAL_SCHEMA_VERSION SMALLINT, GLOBAL_MANIFEST_SHA256 TEXT",
        "PHYSICAL_HEAD_DIGEST BYTEA, GLOBAL_SCHEMA_VERSION SMALLINT, GLOBAL_MANIFEST_SHA256 TEXT",
        "V_TERMINAL.RECEIPT_DIGEST, V_SCHEMA_VERSION, V_MANIFEST_SHA256",
        "H.HEAD_DIGEST, C.CURRENT_SCHEMA_VERSION, PG_CATALOG.BTRIM(C.MANIFEST_SHA256::TEXT)",
        "H.HEAD_DIGEST, V_GLOBAL_SCHEMA_VERSION, V_GLOBAL_MANIFEST_SHA256",
        "AND O.EVENT_DIGEST = E.EVENT_DIGEST AND O.COMMAND_ID = E.COMMAND_ID AND O.REQUEST_DIGEST = E.REQUEST_DIGEST",
        "T.XMIN = PG_CATALOG.PG_CURRENT_XACT_ID()::XID",
        "V_TERMINAL_CURRENT_XACT IS DISTINCT FROM TRUE",
        "PG_CATALOG.SHA256(",
        "LATTICE_POSTGRES_MIGRATION_MANIFEST_V1",
        "PG_CATALOG.INT8SEND(2::BIGINT)",
    ] {
        assert!(
            normalized.contains(required),
            "missing v3 invariant: {required}"
        );
    }
    assert_eq!(
        normalized
            .matches("LATTICE_POSTGRES_MIGRATION_MANIFEST_V1")
            .count(),
        4,
        "all four schema-sensitive runtime entry points must recompute the exact full manifest",
    );

    let read_head = normalized
        .split_once("CREATE FUNCTION CONTROL.TASK_LEDGER_READ_HEAD_V1(")
        .expect("Task Ledger head reader")
        .1
        .split_once("$LATTICE_TASK_LEDGER_READ_HEAD_V1$;")
        .expect("Task Ledger head reader terminator")
        .0;
    for required in [
        "P_STREAM_ID BYTEA, P_EXPECTED_PROJECT_ID TEXT, P_EXPECTED_PROJECT_SNAPSHOT_ID TEXT",
        "P_EXPECTED_PROJECT_ID !~ '^[A-Z0-9][A-Z0-9._-]{1,63}$'",
        "P_EXPECTED_PROJECT_SNAPSHOT_ID !~ '^[A-Z0-9._:-]{1,128}$'",
        "IF V_STREAM_FOUND AND ( V_PROJECT_ID IS DISTINCT FROM P_EXPECTED_PROJECT_ID OR V_PROJECT_SNAPSHOT_ID IS DISTINCT FROM P_EXPECTED_PROJECT_SNAPSHOT_ID ) THEN RAISE EXCEPTION USING ERRCODE = 'LCR01', MESSAGE = 'LEDGER STREAM SCOPE CORRUPT';",
        "SELECT PG_CATALOG.COUNT(*) INTO V_PHYSICAL_COUNT FROM ONLY CONTROL.PHYSICAL_HEADS AS H WHERE H.REPOSITORY_OWNER = 'TASK_LEDGER' AND H.AGGREGATE_KEY_DIGEST = P_STREAM_ID;",
        "V_PHYSICAL_COUNT > 1 OR (V_PHYSICAL_COUNT = 1 AND NOT EXISTS",
        "H.PROJECT_ID = P_EXPECTED_PROJECT_ID AND H.PROJECT_SNAPSHOT_ID = P_EXPECTED_PROJECT_SNAPSHOT_ID",
        "ON H.PROJECT_ID = P_EXPECTED_PROJECT_ID AND H.PROJECT_SNAPSHOT_ID = P_EXPECTED_PROJECT_SNAPSHOT_ID",
        "V_HISTORY_MANIFEST_SHA256 IS DISTINCT FROM V_GLOBAL_MANIFEST_SHA256",
    ] {
        assert!(
            read_head.contains(required),
            "missing Task Ledger head-reader invariant: {required}"
        );
    }
    assert!(
        normalized.contains(
            "REVOKE ALL ON FUNCTION CONTROL.TASK_LEDGER_READ_HEAD_V1( BYTEA, TEXT, TEXT )"
        )
    );
    assert!(normalized.contains(
        "GRANT EXECUTE ON FUNCTION CONTROL.TASK_LEDGER_READ_HEAD_V1( BYTEA, TEXT, TEXT ) TO LATTICE_RUNTIME"
    ));
    assert!(
        !normalized.contains("REVOKE ALL ON FUNCTION CONTROL.TASK_LEDGER_READ_HEAD_V1( BYTEA )")
    );

    let finalizer = normalized
        .split_once("CREATE FUNCTION CONTROL.TASK_LEDGER_FINALIZE_V1(")
        .expect("Task Ledger finalizer")
        .1
        .split_once("$LATTICE_TASK_LEDGER_FINALIZE_V1$;")
        .expect("Task Ledger finalizer terminator")
        .0;
    for required in [
        "OR (V_STREAM_FOUND AND ( V_TERMINAL.EXPECTED_STATE_DIGEST IS DISTINCT FROM P_BASE_CHECKPOINT_DIGEST OR V_TERMINAL.BEFORE_STATE_DIGEST IS DISTINCT FROM P_BASE_CHECKPOINT_DIGEST )) OR V_TERMINAL.AFTER_STATE_DIGEST IS DISTINCT FROM P_NEXT_CHECKPOINT_DIGEST",
        "OR P_NEXT_COMMAND_COUNT::NUMERIC <> 1 OR P_NEXT_EVENT_COUNT::NUMERIC <> (CASE WHEN P_APPEND_EVENT THEN 1 ELSE 0 END)",
        "V_TERMINAL.EXPECTED_REVISION::NUMERIC IS DISTINCT FROM P_NEXT_COMMAND_COUNT::NUMERIC - 1",
        "V_TERMINAL.BEFORE_REVISION::NUMERIC IS DISTINCT FROM P_NEXT_COMMAND_COUNT::NUMERIC - 1",
        "V_TERMINAL.AFTER_REVISION::NUMERIC IS DISTINCT FROM P_NEXT_COMMAND_COUNT::NUMERIC",
    ] {
        assert!(
            finalizer.contains(required),
            "missing Task Ledger finalizer invariant: {required}"
        );
    }
    assert!(
        !finalizer.contains(
            "OR V_TERMINAL.EXPECTED_STATE_DIGEST IS DISTINCT FROM P_BASE_CHECKPOINT_DIGEST"
        ),
        "fresh Ledger state must not be equated with the Store genesis domain"
    );
}

#[test]
fn executable_migration_has_runner_owned_transaction_and_no_discovery_escape() {
    let executable = migration_manifest()
        .iter()
        .find(|entry| entry.id() == "0002_control_store_foundation")
        .expect("foundation migration");
    let sql = std::str::from_utf8(executable.bytes()).expect("UTF-8 SQL");
    let uppercase = sql.to_ascii_uppercase();

    for forbidden in [
        "BEGIN;",
        "COMMIT;",
        "IF NOT EXISTS",
        "DO $$",
        "EXECUTE ",
        "CREATE EXTENSION",
        "CREATE ROLE",
        "ALTER ROLE",
        "PASSWORD",
        "CREATE DATABASE",
        "DROP TABLE",
        "DROP SCHEMA",
        "DROP FUNCTION",
    ] {
        assert!(
            !uppercase.contains(forbidden),
            "forbidden migration surface: {forbidden}"
        );
    }

    for required in [
        "CREATE SCHEMA CONTROL",
        "CREATE SCHEMA MEMORY",
        "CREATE SCHEMA READMODEL",
        "CREATE TABLE CONTROL.DATABASE_IDENTITY",
        "CREATE TABLE CONTROL.MIGRATION_HISTORY",
        "CREATE TABLE CONTROL.SCHEMA_COMPATIBILITY",
        "CREATE TABLE CONTROL.RUNTIME_ADMISSION",
        "CREATE TABLE CONTROL.PHYSICAL_HEADS",
        "CREATE TABLE CONTROL.TERMINAL_TRANSACTIONS",
        "CONSTRAINT DATABASE_IDENTITY_UUID_V8 CHECK",
        "REVOKE ALL ON SCHEMA CONTROL FROM PUBLIC",
        "ALTER DEFAULT PRIVILEGES",
    ] {
        assert!(
            uppercase.contains(required),
            "missing schema invariant: {required}"
        );
    }
    assert!(!uppercase.contains("DATABASE_IDENTITY_UUID_V5"));
}

#[test]
fn review_regression_sql_nulls_grants_receipt_relations_and_defaults_fail_closed() {
    let executable = migration_manifest()
        .iter()
        .find(|entry| entry.id() == "0002_control_store_foundation")
        .expect("foundation migration");
    let sql = std::str::from_utf8(executable.bytes()).expect("UTF-8 SQL");
    let normalized = sql
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_uppercase();

    assert!(normalized.contains("DAEMON_INSTANCE_ID IS NOT NULL"));
    assert!(normalized.contains("DAEMON_EPOCH IS NOT NULL"));
    assert!(normalized.contains("OBSERVATION_DIGEST IS NOT NULL"));
    assert!(normalized.contains("AUTHORITY_HEAD_DIGEST IS NOT NULL"));

    assert!(!normalized.contains("GRANT SELECT ON ALL TABLES IN SCHEMA CONTROL"));
    assert!(normalized.contains(
        "GRANT SELECT ON CONTROL.DATABASE_IDENTITY, CONTROL.MIGRATION_HISTORY, CONTROL.SCHEMA_COMPATIBILITY, CONTROL.RUNTIME_ADMISSION"
    ));

    assert!(normalized.contains("BEFORE_STATE_DIGEST = EXPECTED_STATE_DIGEST"));
    assert!(normalized.contains("BEFORE_HEAD_DIGEST = EXPECTED_HEAD_DIGEST"));
    assert!(normalized.contains("AFTER_STATE_DIGEST = NEXT_STATE_DIGEST"));
    assert!(normalized.contains("AFTER_STATE_DIGEST = BEFORE_STATE_DIGEST"));
    assert!(normalized.contains("AFTER_HEAD_DIGEST = BEFORE_HEAD_DIGEST"));
    assert!(normalized.contains("AFTER_REVISION - BEFORE_REVISION = 1"));

    for class in ["TABLES", "SEQUENCES", "FUNCTIONS", "TYPES"] {
        assert!(normalized.contains(&format!(
            "ALTER DEFAULT PRIVILEGES FOR ROLE LATTICE_MIGRATOR REVOKE ALL ON {class} FROM PUBLIC"
        )));
    }
    assert!(!normalized.contains("DEFAULT PRIVILEGES FOR ROLE LATTICE_MIGRATOR IN SCHEMA"));

    assert!(normalized.contains("TERMINAL_TRANSACTIONS_DAEMON_INSTANCE_ID CHECK"));
    assert!(normalized.contains("DAEMON_INSTANCE_ID ~ '^[A-Z0-9][A-Z0-9._:-]{0,127}$'"));
}

#[test]
#[allow(clippy::too_many_lines)]
fn live_store_migration_is_fixed_function_gated_and_transaction_control_free() {
    let live = migration_manifest()
        .iter()
        .find(|entry| entry.id() == "0003_live_control_store")
        .expect("live Store migration");
    let sql = std::str::from_utf8(live.bytes()).expect("UTF-8 SQL");
    let uppercase = sql.to_ascii_uppercase();
    let normalized = sql
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_uppercase();

    for forbidden in [
        "BEGIN;",
        "COMMIT;",
        "ROLLBACK;",
        "IF NOT EXISTS",
        "DO $$",
        "EXECUTE FORMAT",
        "EXECUTE IMMEDIATE",
        "CREATE EXTENSION",
        "CREATE ROLE",
        "ALTER ROLE",
        "PASSWORD",
        "CREATE DATABASE",
        "DROP TABLE",
        "DROP SCHEMA",
        "DROP FUNCTION",
    ] {
        assert!(
            !uppercase.contains(forbidden),
            "forbidden live migration surface: {forbidden}"
        );
    }

    assert_eq!(
        uppercase.matches("CREATE FUNCTION CONTROL.STORE_").count(),
        3
    );
    assert_eq!(uppercase.matches("SECURITY DEFINER").count(), 3);
    assert_eq!(uppercase.matches("SET SEARCH_PATH = PG_CATALOG").count(), 3);
    assert_eq!(uppercase.matches("SET ROW_SECURITY = ON").count(), 3);
    for function in [
        "STORE_PREPARE_V2",
        "STORE_FINALIZE_V2",
        "STORE_CURRENT_HEAD_V2",
    ] {
        assert!(normalized.contains(&format!("CREATE FUNCTION CONTROL.{function}(")));
        assert!(normalized.contains(&format!("GRANT EXECUTE ON FUNCTION CONTROL.{function}(")));
        assert!(normalized.contains(&format!("REVOKE ALL ON FUNCTION CONTROL.{function}(")));
    }
    for required in [
        "ADD COLUMN STORE_CONTRACT_VERSION SMALLINT NOT NULL",
        "ADD COLUMN DATABASE_IDENTITY_DIGEST BYTEA NOT NULL",
        "TERMINAL_TRANSACTIONS_STORE_CONTRACT_V2",
        "TERMINAL_TRANSACTIONS_DATABASE_IDENTITY_DIGEST",
        "SESSION_USER <> 'LATTICE_RUNTIME_LOGIN'",
        "CURRENT_SETTING('TRANSACTION_ISOLATION') <> 'SERIALIZABLE'",
        "FROM ONLY CONTROL.TERMINAL_TRANSACTIONS",
        "FROM ONLY CONTROL.PHYSICAL_HEADS",
        "FOR SHARE OF A",
        "FOR UPDATE OF H",
        "LTX01",
        "LAD01",
        "LAU01",
        "LRV01",
        "IS DISTINCT FROM",
        "V_ADMISSION_MODE IS DISTINCT FROM 'ACTIVE'",
        "V_DAEMON_INSTANCE_ID IS DISTINCT FROM P_DAEMON_INSTANCE_ID",
        "V_AUTHORITY_OBSERVATION_DIGEST IS DISTINCT FROM P_AUTHORITY_OBSERVATION_DIGEST",
        "V_TERMINAL.PRODUCER_ID IS DISTINCT FROM 'LATTICE-POSTGRES-STORE'",
        "V_TERMINAL.DATABASE_UUID IS DISTINCT FROM V_DATABASE_UUID",
        "V_TERMINAL.SCHEMA_VERSION IS DISTINCT FROM V_SCHEMA_VERSION",
        "V_PREPARE.PREPARE_STATUS IS DISTINCT FROM 'PREPARED'",
    ] {
        assert!(
            normalized.contains(required),
            "missing live invariant: {required}"
        );
    }
    assert_eq!(
        normalized
            .matches("DROP CONSTRAINT TERMINAL_TRANSACTIONS_SCOPE_HEAD_FK")
            .count(),
        1,
        "v2 must remove the v1 FK so a stale first use can retain a terminal receipt without materializing genesis",
    );
    assert_eq!(
        normalized
            .matches("INSERT INTO CONTROL.PHYSICAL_HEADS")
            .count(),
        1,
        "only an applied transition may materialize or advance a physical head",
    );
    let terminal_lookup = normalized
        .find("FROM ONLY CONTROL.TERMINAL_TRANSACTIONS")
        .expect("terminal replay lookup");
    let new_work_admission = normalized
        .find("IF P_ADMISSION_MODE IS DISTINCT FROM 'ACTIVE'")
        .expect("new-work admission check");
    assert!(
        terminal_lookup < new_work_admission,
        "replay and changed-ID classification must precede mutable admission"
    );
    assert!(!normalized.contains("OR P_ADMISSION_MODE <> 'ACTIVE'"));
    assert!(!normalized.contains("GRANT SELECT ON CONTROL.PHYSICAL_HEADS"));
    assert!(!normalized.contains("GRANT SELECT ON CONTROL.TERMINAL_TRANSACTIONS"));
    assert!(!normalized.contains("GRANT INSERT ON CONTROL.PHYSICAL_HEADS"));
    assert!(!normalized.contains("GRANT UPDATE ON CONTROL.PHYSICAL_HEADS"));
}

#[test]
fn runner_has_closed_fresh_and_exact_prefix_states_through_v6() {
    let source = include_str!("../src/postgres_setup.rs");
    for required in [
        "enum InstalledManifestState",
        "Fresh",
        "ExactV1Prefix",
        "ExactV2Prefix",
        "ExactV3Prefix",
        "ExactV4Prefix",
        "ExactV5Prefix",
        "ExactV6Full",
        "classify_installed_manifest_state",
        "verify_v1_upgrade_source",
        "verify_v2_upgrade_source",
        "verify_v3_upgrade_source",
        "verify_v4_upgrade_source",
        "verify_v5_upgrade_source",
        "apply_missing_entries",
        "advance_compatibility_from_v1",
        "advance_compatibility_from_v2",
        "advance_compatibility_from_v3",
        "advance_compatibility_from_v4",
        "advance_compatibility_from_v5",
        "LOCK TABLE control.physical_heads IN ACCESS EXCLUSIVE MODE",
        "LOCK TABLE control.terminal_transactions IN ACCESS EXCLUSIVE MODE",
        "LOCK TABLE control.runtime_admission IN ACCESS EXCLUSIVE MODE",
        "UPDATE ONLY control.schema_compatibility",
        "t.tgisinternal AND t.tgenabled = 'O'",
    ] {
        assert!(
            source.contains(required),
            "missing runner invariant: {required}"
        );
    }
    assert!(!source.contains("apply_manifest_in_transaction"));
}

#[test]
fn task094_store_calls_only_the_fixed_writer_owned_rebind_boundary() {
    let source = include_str!("../src/postgres_setup.rs");
    let apply = source
        .split_once("pub fn apply_migrations(")
        .expect("migration entrypoint")
        .1;
    let transition = apply
        .split_once("InstalledManifestState::ExactV5Prefix")
        .expect("exact v5 transition arm")
        .1
        .split_once("InstalledManifestState::ExactV6Full")
        .expect("exact v6 no-op arm")
        .0;
    for required in [
        "verify_v5_upgrade_source",
        "apply_missing_entries(&mut transaction, 6)",
        "advance_compatibility_from_v5",
        "CALL writer_lease.writer_lease_rebind_v3()",
        "verify_runtime_foreman_schema_v6",
    ] {
        assert!(
            transition.contains(required),
            "missing atomic v5-to-v6 transition boundary: {required}"
        );
    }
    assert!(!transition.contains("UPDATE ONLY writer_lease."));
    assert!(!transition.contains("INSERT INTO writer_lease."));
    assert!(!transition.contains("GRANT USAGE ON SCHEMA writer_lease"));
    assert!(!transition.contains("GRANT EXECUTE ON FUNCTION writer_lease."));
}

#[test]
fn migration_target_rejects_default_or_ambiguous_database_identity() {
    let run_id = "0123456789abcdef0123456789abcdef";
    for database in [
        "postgres",
        "template0",
        "template1",
        "",
        "UPPERCASE",
        "has-dash",
        "has.dot",
        " leading",
        "trailing ",
    ] {
        assert!(
            MigrationTarget::new(database, run_id).is_err(),
            "unsafe database accepted: {database:?}"
        );
    }

    for bad_run_id in [
        "",
        "abc",
        "0123456789ABCDEF0123456789ABCDEF",
        "0123456789abcdef0123456789abcdeg",
        "0123456789abcdef0123456789abcdef0",
    ] {
        assert!(
            MigrationTarget::new("lattice_task019_a", bad_run_id).is_err(),
            "unsafe run id accepted: {bad_run_id:?}"
        );
    }

    let target = MigrationTarget::new("lattice_task019_a", run_id).expect("safe target");
    assert_eq!(target.database_name(), "lattice_task019_a");
    assert_eq!(target.run_id(), run_id);
    assert_eq!(
        target.database_comment(),
        "LATTICE_DEVOS_DISPOSABLE_V1:0123456789abcdef0123456789abcdef"
    );
    let expected_uuid = target.expected_database_uuid();
    assert_eq!(expected_uuid.len(), 36);
    assert_eq!(expected_uuid.as_bytes()[14], b'8');
    assert!(matches!(
        expected_uuid.as_bytes()[19],
        b'8' | b'9' | b'a' | b'b'
    ));
    assert_ne!(expected_uuid, "00000000-0000-0000-0000-000000000000");
    let expected_identity = target.expected_database_identity_sha256();
    assert_eq!(expected_identity.as_str().len(), 64);
    assert!(
        expected_identity
            .as_str()
            .bytes()
            .all(|byte| { byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte) })
    );
    assert_ne!(expected_identity.as_str(), &"0".repeat(64));
    assert_eq!(
        expected_uuid,
        MigrationTarget::new("lattice_task019_a", run_id)
            .expect("same safe target")
            .expected_database_uuid()
    );
    assert_eq!(
        expected_identity,
        MigrationTarget::new("lattice_task019_a", run_id)
            .expect("same safe target")
            .expected_database_identity_sha256()
    );
    assert_ne!(
        expected_uuid,
        MigrationTarget::new("lattice_task019_b", run_id)
            .expect("different safe target")
            .expected_database_uuid()
    );
    assert_ne!(
        expected_identity,
        MigrationTarget::new("lattice_task019_b", run_id)
            .expect("different safe target")
            .expected_database_identity_sha256()
    );
    assert!(!format!("{target:?}").contains("password"));
}

#[test]
fn database_roles_are_closed_and_never_a_login_or_caller_value() {
    assert_eq!(
        DatabaseRole::ALL,
        [
            DatabaseRole::Migrator,
            DatabaseRole::Runtime,
            DatabaseRole::Guardian,
            DatabaseRole::ReadOnly,
        ]
    );
    assert_eq!(DatabaseRole::Migrator.as_str(), "lattice_migrator");
    assert_eq!(DatabaseRole::Runtime.as_str(), "lattice_runtime");
    assert_eq!(DatabaseRole::Guardian.as_str(), "lattice_guardian");
    assert_eq!(DatabaseRole::ReadOnly.as_str(), "lattice_readonly");
    assert_eq!(
        DatabaseRole::Migrator.login_role(),
        "lattice_migrator_login"
    );
    assert_eq!(DatabaseRole::Runtime.login_role(), "lattice_runtime_login");
    assert_eq!(
        DatabaseRole::Guardian.login_role(),
        "lattice_guardian_login"
    );
    assert_eq!(
        DatabaseRole::ReadOnly.login_role(),
        "lattice_readonly_login"
    );
}

#[test]
fn setup_errors_are_closed_static_bounded_and_redacted() {
    assert_eq!(PostgresStoreSetupErrorKind::ALL.len(), 15);
    assert!(
        PostgresStoreSetupErrorKind::ALL
            .contains(&PostgresStoreSetupErrorKind::PostApplyVerificationFailed)
    );
    assert_eq!(
        PostgresStoreSetupErrorKind::PostApplyVerificationFailed.code(),
        "STORE_MIGRATION_COMMITTED_UNVERIFIED"
    );
    for kind in PostgresStoreSetupErrorKind::ALL {
        let error = PostgresStoreSetupError::new(kind);
        assert!(!error.code().is_empty());
        assert!(error.code().len() <= 64);
        assert!(
            error
                .code()
                .bytes()
                .all(|byte| { byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_' })
        );
        let display = error.to_string();
        let debug = format!("{error:?}");
        for forbidden in [
            "password",
            "postgres://",
            "127.0.0.1",
            "SELECT ",
            "C:\\",
            "DATABASE_URL",
        ] {
            assert!(!display.contains(forbidden));
            assert!(!debug.contains(forbidden));
        }
    }
}

#[test]
fn driver_and_schema_support_are_exact_for_this_foundation() {
    assert_eq!(POSTGRES_DRIVER_VERSION, "0.19.14");
    assert_eq!(SUPPORTED_POSTGRES_MAJOR, 17);
    assert_eq!(POSTGRES_SCHEMA_VERSION, 6);
}

#[test]
fn review_regression_verifier_uses_one_exact_catalog_snapshot_and_fixed_tables() {
    let source = include_str!("../src/postgres_setup.rs");

    assert!(source.contains(".isolation_level(IsolationLevel::ReadCommitted)"));
    assert!(source.contains(".isolation_level(IsolationLevel::RepeatableRead)"));
    assert!(source.contains("current_setting('transaction_isolation')"));
    assert!(source.contains("current_setting('transaction_read_only')"));
    assert!(source.contains("pg_inherits"));
    assert!(source.contains("c.relhassubclass"));
    assert!(source.contains("c.relispartition"));
    assert!(!source.contains("AND NOT a.attisdropped"));
    assert!(source.contains("COALESCE(array_to_string(p.proconfig, ','), '<NULL>')"));
    assert!(
        source.contains(
            "search_path=pg_catalog,row_security=on,lock_timeout=5s,statement_timeout=30s"
        )
    );
    assert!(source.contains("('pg_catalog.pg_current_xact_id()', 'lattice_migrator'::text)"));

    for table in [
        "control.database_identity",
        "control.migration_history",
        "control.schema_compatibility",
        "control.runtime_admission",
    ] {
        assert!(
            source.contains(&format!("FROM ONLY {table}")),
            "authoritative read does not use ONLY: {table}"
        );
    }
}

#[test]
fn task076_pre_snapshot_try_lock_is_narrowly_granted_to_migrator() {
    let source = include_str!("../src/postgres_setup.rs");
    let live = include_str!("postgres_live.rs");
    let task_ledger_live = include_str!("postgres_task_ledger.rs");
    assert!(
        source.contains("('pg_catalog.pg_try_advisory_lock(bigint)', 'lattice_migrator'::text)")
    );
    for denied in [
        "('pg_catalog.pg_advisory_lock(bigint)', NULL::text)",
        "('pg_catalog.pg_try_advisory_lock(integer,integer)', NULL::text)",
        "('pg_catalog.pg_try_advisory_lock_shared(bigint)', NULL::text)",
        "('pg_catalog.pg_try_advisory_lock_shared(integer,integer)', NULL::text)",
    ] {
        assert!(source.contains(denied), "missing fixed denial: {denied}");
    }

    let boundary = live
        .split_once("fn set_exact_pre_role_function_access")
        .expect("system function ACL fixture")
        .1
        .split_once("fn prove_first_apply_and_reconciliation")
        .expect("system function ACL fixture boundary")
        .0;
    let grant = boundary
        .split_once("GRANT EXECUTE ON FUNCTION")
        .expect("fixed migrator grants")
        .1
        .split_once("TO lattice_migrator")
        .expect("fixed migrator grant role")
        .0;
    assert!(grant.contains("pg_catalog.pg_try_advisory_lock(bigint)"));
    assert!(grant.contains("pg_catalog.pg_advisory_xact_lock(bigint)"));
    assert!(!grant.contains("pg_catalog.pg_advisory_lock(bigint)"));
    assert!(!grant.contains("pg_catalog.pg_try_advisory_lock(integer, integer)"));
    assert!(!grant.contains("pg_catalog.pg_try_advisory_lock_shared"));
    let task_ledger_grants: Vec<&str> = task_ledger_live
        .split("GRANT EXECUTE ON FUNCTION")
        .skip(1)
        .map(|suffix| {
            suffix
                .split_once("TO lattice_migrator")
                .expect("fixed TASK-050 migrator grant")
                .0
        })
        .collect();
    assert_eq!(task_ledger_grants.len(), 2);
    for grant in task_ledger_grants {
        assert!(grant.contains("pg_catalog.pg_try_advisory_lock(bigint)"));
        assert!(grant.contains("pg_catalog.pg_advisory_xact_lock(bigint)"));
        assert!(grant.contains("pg_catalog.pg_current_xact_id()"));
        assert!(!grant.contains("pg_catalog.pg_advisory_lock(bigint)"));
        assert!(!grant.contains("pg_catalog.pg_try_advisory_lock(integer, integer)"));
        assert!(!grant.contains("pg_catalog.pg_try_advisory_lock_shared"));
    }
}

#[test]
fn review_regression_requires_real_login_to_capability_role_mapping() {
    let source = include_str!("../src/postgres_setup.rs");
    let live = include_str!("postgres_live.rs");

    for login in [
        "lattice_migrator_login",
        "lattice_runtime_login",
        "lattice_guardian_login",
        "lattice_readonly_login",
    ] {
        assert!(
            source.contains(login),
            "missing fixed login principal: {login}"
        );
    }
    assert!(source.contains("m.inherit_option"));
    assert!(source.contains("m.set_option"));
    assert!(source.contains("m.admin_option"));
    assert!(source.contains("has_schema_privilege($1, n.oid, 'CREATE')"));
    assert!(live.contains("WITH ADMIN FALSE, INHERIT FALSE, SET TRUE"));
    assert!(!live.contains("WITH ADMIN FALSE, INHERIT TRUE, SET TRUE"));
    assert!(live.contains("prove_login_requires_set_role"));
    assert!(live.contains("lattice_readonly_login;"));
    assert!(source.contains("pg_parameter_acl"));
    for acl_catalog in [
        "pg_attribute",
        "pg_language",
        "pg_foreign_data_wrapper",
        "pg_foreign_server",
        "pg_tablespace",
        "pg_largeobject_metadata",
    ] {
        assert!(
            source.contains(acl_catalog),
            "missing ACL closure: {acl_catalog}"
        );
    }
    assert!(source.contains("FROM pg_database d"));
    assert!(source.contains("WHERE acl.grantee = 0"));
    assert!(live.contains("prove_cross_database_acl_drift"));
    assert!(live.contains("prove_parameter_acl_drift"));
    assert!(live.contains("prove_external_column_acl_drift"));
    assert!(source.contains("verify_external_relation_principal_closure"));
    assert!(source.contains("verify_external_function_principal_closure"));
    assert!(source.contains("verify_pre_role_system_function_boundary"));
    assert!(source.contains("verify_large_object_boundary"));
    assert!(source.contains("max_prepared_transactions"));
    assert!(source.contains("FROM pg_shdepend d"));
    assert!(source.contains("a.attacl"));
    assert!(source.contains("c.relacl"));
    assert!(live.contains("prove_external_capability_acl_drift"));
    assert!(live.contains("prove_external_public_acl_drift"));
    assert!(live.contains("prove_external_function_acl_drift"));
    assert!(live.contains("prove_external_function_fixed_acl_drift"));
    assert!(live.contains("prove_non_migrator_default_acl_drift"));
    assert!(live.contains("prove_large_object_acl_drift"));
    assert!(live.contains("prove_login_owner_dependency_drift"));
    assert!(live.contains("PREPARE TRANSACTION 'task019_pre_set_role_forbidden'"));
    assert!(live.contains("pg_cancel_backend"));
    assert!(live.contains("pg_terminate_backend"));
    assert!(live.contains("pg_export_snapshot"));
    assert!(live.contains("pg_current_xact_id"));
    assert!(live.contains("txid_current"));
    assert!(live.contains("lo_import(text, oid)"));
    assert!(live.contains("prove_notifications_are_non_authoritative"));
    assert!(live.contains("NOTIFY lattice_task019, 'ignored'"));
    assert!(live.contains("SqlState::INSUFFICIENT_PRIVILEGE"));
    assert!(live.contains("SqlState::OBJECT_NOT_IN_PREREQUISITE_STATE"));
    assert!(live.contains("pg_logical_emit_message"));
    assert!(live.contains("pg_try_advisory_lock"));
    assert!(!source.contains("pg_catalog.pg_notify(text,text)"));
}

#[test]
fn review_regression_owned_type_and_post_commit_phase_are_closed() {
    let source = include_str!("../src/postgres_setup.rs");
    let live = include_str!("postgres_live.rs");

    assert!(source.contains("TYPE_SIGNATURE_SQL"));
    assert!(source.contains("PostApplyVerificationFailed"));
    assert!(live.contains("CREATE TYPE control.task019_shell"));
    assert!(live.contains("prove_post_apply_verification_failure"));
}

#[test]
fn review_regression_commit_unknown_is_a_real_transport_boundary() {
    let source = include_str!("postgres_live.rs");

    assert!(source.contains("CommitResponseDropProxy"));
    assert!(source.contains("relay_backend_until_commit_ack"));
    assert!(source.contains("frame[0] == b'C'"));
    assert!(source.contains("frame[5..].starts_with(b\"COMMIT\\0\")"));
    assert!(!source.contains("fn inject_commit_response_loss"));
}

#[test]
fn review_regression_harness_cleanup_is_fail_closed_and_preflighted() {
    let source = include_str!("../../../scripts/run-task019-postgres.ps1");
    let pass_position = source
        .rfind("TASK019_POSTGRES_HARNESS=PASS")
        .expect("PASS marker");
    let finalizer_position = source.rfind("finally {").expect("outer finalizer");

    assert!(source.contains("return ($statusExitCode -eq 3)"));
    assert!(source.contains("function Wait-Task019ClusterStopped"));
    assert!(source.contains("$attempt -lt 20"));
    assert!(source.contains("Start-Sleep -Milliseconds 250"));
    assert!(source.contains(
        "return (Wait-Task019ClusterStopped -PgCtl $PgCtl -DataDirectory $DataDirectory)"
    ));
    assert!(source.contains("Assert-NoReparseAncestor"));
    assert!(source.contains("TASK019_HARNESS_SELF_TEST=PASS"));
    assert!(source.contains("TASK019_SERVER_LOG_SANITIZE_FAILED"));
    assert!(source.contains("$safeTokens"));
    assert!(source.contains(".native-stdout.log"));
    assert!(
        pass_position > finalizer_position,
        "PASS marker must follow cleanup and installed-service verification"
    );
}

#[test]
fn schema_v3_runtime_uses_the_frozen_prefix_catalog_contract() {
    let source = include_str!("../src/postgres_setup.rs");
    let runtime_verifier = source
        .split_once("pub(crate) fn verify_runtime_store_schema")
        .expect("runtime Store verifier")
        .1
        .split_once("fn preflight_connection")
        .expect("runtime verifier boundary")
        .0;

    assert!(runtime_verifier.contains("let v3_prefix = installed_schema_version == 3;"));
    assert!(runtime_verifier.contains(
        "verify_schema_objects_with_contract(&mut transaction, current_profile, v3_prefix)?;"
    ));
    assert!(runtime_verifier.contains(
        "verify_roles_and_grants_with_contract(&mut transaction, current_profile, v3_prefix)?;"
    ));
}

#[test]
fn schema_v3_upgrade_source_uses_the_frozen_prefix_catalog_contract() {
    let source = include_str!("../src/postgres_setup.rs");
    let upgrade_verifier = source
        .split_once("fn verify_v3_upgrade_source")
        .expect("schema-v3 upgrade verifier")
        .1
        .split_once("fn v3_upgrade_source_has_memory")
        .expect("schema-v3 verifier boundary")
        .0;

    assert!(
        upgrade_verifier.contains("verify_schema_objects_with_contract(client, profile, true)?;")
    );
    assert!(
        upgrade_verifier.contains("verify_roles_and_grants_with_contract(client, profile, true)")
    );
}

#[test]
fn memory_v3_identity_rows_are_verified_only_with_migrator_authority() {
    let source = include_str!("../src/postgres_setup.rs");
    let runtime_verifier = source
        .split_once("pub(crate) fn verify_runtime_store_schema")
        .expect("runtime Store verifier")
        .1
        .split_once("fn preflight_connection")
        .expect("runtime verifier boundary")
        .0;
    let catalog_verifier = source
        .split_once("fn verify_catalog")
        .expect("catalog verifier")
        .1
        .split_once("fn verify_autonomy_receipt_profile")
        .expect("catalog verifier boundary")
        .0;

    assert!(!runtime_verifier.contains("verify_codebase_memory_v3_identity"));
    assert!(
        catalog_verifier.contains(
            "verify_codebase_memory_v3_identity_for_role(client, target, manifest, role)?;"
        )
    );
    let role_verifier = source
        .split_once("fn verify_codebase_memory_v3_identity_for_role")
        .expect("role-scoped Memory identity verifier")
        .1
        .split_once("fn verify_codebase_memory_v3_identity")
        .expect("direct Memory identity verifier boundary")
        .0;
    assert!(role_verifier.contains("if role == DatabaseRole::Migrator"));
    assert!(
        role_verifier
            .contains("verify_codebase_memory_v3_identity(client, target, global_manifest)?;")
    );
}
