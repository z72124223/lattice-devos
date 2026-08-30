use lattice_postgres_foreman::{
    FOREMAN_EXTENSION_ID, FOREMAN_EXTENSION_PATH, FOREMAN_EXTENSION_SCHEMA_VERSION,
    MAX_ACTIVE_TASK_REPLAY_ROWS, MAX_EXTENSION_ATTEMPTS, MAX_GLOBAL_ACTIVE_ATTEMPTS,
    MAX_TASK_ACTIVE_ATTEMPTS, REQUIRED_GLOBAL_MANIFEST_SHA256, REQUIRED_GLOBAL_SCHEMA_VERSION,
    verify_embedded_extension,
};

#[test]
fn embedded_v1_identity_is_exact_and_bound_to_store_v7() {
    let evidence = verify_embedded_extension().expect("frozen v1 extension");
    assert_eq!(evidence.extension_id(), FOREMAN_EXTENSION_ID);
    assert_eq!(evidence.schema_version(), FOREMAN_EXTENSION_SCHEMA_VERSION);
    assert_eq!(evidence.path(), FOREMAN_EXTENSION_PATH);
    assert_eq!(evidence.byte_length(), evidence.bytes().len());
    assert_eq!(evidence.byte_length(), 349_470);
    assert_eq!(
        evidence.sql_sha256().as_str(),
        "32dd034191b9d87c8792f78c26b5d84533a95405ff4d1cc5be00da54a08d4b13"
    );
    assert_eq!(
        evidence.manifest_sha256().as_str(),
        "0b1855611b37da4ed8b17be3d85e6410598fb13a255ce307d0907e702afeea63"
    );
    assert_eq!(REQUIRED_GLOBAL_SCHEMA_VERSION, 7);
    assert_eq!(
        REQUIRED_GLOBAL_MANIFEST_SHA256,
        "584a446464ab2f7ebd8b85543ba36a6d52b0a708502c39d2653b8814d84313f8"
    );
}

#[test]
fn catalog_profile_pins_are_closed_and_live_measurement_is_coordinator_owned() {
    let setup = include_str!("../src/setup.rs");
    assert!(setup.contains("const EXPECTED_TABLE_COUNT: i64 = 17;"));
    assert!(setup.contains("const EXPECTED_FUNCTION_COUNT: i64 = 43;"));
    assert!(setup.contains("const EXPECTED_RUNTIME_FUNCTION_COUNT: i64 = 39;"));
    assert!(setup.contains("7b249bf8416f734a34b6e1b9e7b407d17b00771139ac71a12294a3b0543e6120"));
    assert!(setup.contains("e772c3041a4c30908c555c5b96f6705f48011634e47db8f562410371705ce807"));
    assert!(setup.contains("function_digest != EXPECTED_FUNCTION_CATALOG_SHA256"));
    assert!(setup.contains("table_digest != EXPECTED_TABLE_CATALOG_SHA256"));
    assert!(!setup.contains("LATTICE_FOREMAN_CATALOG_DEBUG"));
    assert!(setup.contains("LATTICE_FOREMAN_CATALOG_SIGNATURE_URL"));
    assert!(setup.contains("FOREMAN_FUNCTION_CATALOG_SHA256={function_digest}"));
    assert!(setup.contains("FOREMAN_TABLE_CATALOG_SHA256={table_digest}"));
}

#[test]
fn catalog_profile_pins_complete_schema_table_and_function_acl_rows() {
    let setup = include_str!("../src/setup.rs");

    assert!(setup.contains("'SCHEMA_PROFILE'"));
    assert!(setup.contains("'SCHEMA_ACL'"));
    assert!(setup.contains("'TABLE_ACL'"));
    assert!(setup.contains("'TABLE_COLUMN_ACL'"));
    assert!(setup.contains("'FUNCTION_ACL'"));
    assert!(setup.contains("schema_owner.oid = n.nspowner"));
    assert!(setup.contains("COALESCE(n.nspacl, pg_catalog.acldefault('n',n.nspowner))"));
    assert!(setup.contains("COALESCE(c.relacl,pg_catalog.acldefault('r',c.relowner))"));
    assert!(setup.contains("pg_catalog.aclexplode(a.attacl)"));
    assert!(setup.contains("COALESCE(p.proacl,pg_catalog.acldefault('f',p.proowner))"));
    assert!(setup.contains("grantor.rolname"));
    assert!(setup.contains("grantee.rolname"));
}

#[test]
fn exact_extension_replay_verifies_the_catalog_and_never_silently_repairs() {
    let setup = include_str!("../src/setup.rs");
    let exact_start = setup
        .find("if state == ExtensionPreState::Exact {")
        .expect("exact replay branch");
    let exact_end = setup[exact_start..]
        .find("    match state {")
        .map(|offset| exact_start + offset)
        .expect("fresh install branch follows exact replay");
    let exact = &setup[exact_start..exact_end];
    let fresh = &setup[exact_end..];

    assert!(exact.contains("verify_catalog("));
    assert!(exact.contains("ExtensionApplyOutcome::AlreadyCurrent(evidence)"));
    assert!(exact.contains(".commit()"));
    assert!(!exact.contains("batch_execute(sql)"));
    assert!(fresh.contains("ExtensionPreState::Fresh => {}"));
    assert!(fresh.contains("batch_execute(sql)"));
    assert!(fresh.contains("ExtensionApplyOutcome::Installed(evidence)"));
}

#[test]
fn capacity_live_fixture_uses_the_general_submission_api_not_generic_execute() {
    let fixture = include_str!("postgres_live.rs");
    let start = fixture
        .find("fn build_claim_fixture(")
        .expect("capacity fixture");
    let end = fixture[start..]
        .find("fn activate_fixture_authority(")
        .map(|offset| start + offset)
        .expect("capacity fixture end");
    let fixture = &fixture[start..end];
    let submission = fixture
        .find("TaskSubmissionEnvelope::new(")
        .expect("general submission fixture");
    let persisted = fixture
        .find(".expect(\"persist intake event\")")
        .expect("intake persistence assertion");
    let intake = &fixture[submission..persisted];

    assert!(
        intake
            .contains("execute_submission(intake_command, authority.clone(), submission.clone())")
    );
    assert!(!intake.contains("execute(intake_command, authority.clone())"));
    assert!(fixture.contains("provision_capacity_project("));
    assert!(fixture.contains("ProjectRegistryCommand::register("));
    assert!(fixture.contains("ProjectRegistryCommand::observe("));
    assert!(fixture.contains("assert_capacity_submission_rejects_unverified_registry("));
    assert!(fixture.contains("mcp-submit:foreman-capacity-request-{task_number}"));
    assert!(fixture.contains("PostgresTaskLedgerErrorKind::ProjectRegistryCurrentnessConflict"));
    assert!(fixture.contains("PostgresWriterLease::new_v5_v7("));
    assert!(fixture.contains("WriterLeaseRepositoryCommand::Acquire("));
    assert!(fixture.contains("managed-lease-{task_ref}-1"));
    assert!(fixture.contains("task_ref[..59].to_ascii_uppercase()"));
    assert!(fixture.contains("writer_fence,"));
}

#[test]
fn extension_is_subordinate_and_capacity_is_closed() {
    let sql = std::str::from_utf8(verify_embedded_extension().expect("extension").bytes())
        .expect("utf8 SQL");

    assert!(!sql.to_ascii_lowercase().contains("task_state"));
    assert!(sql.contains("GENERAL_TASK_INTAKE_V1"));
    assert!(!sql.contains("INSERT INTO control.task_ledger_streams"));
    assert!(sql.contains("SECURITY DEFINER"));
    assert!(sql.contains("SET search_path = pg_catalog"));
    assert!(sql.contains("pg_advisory_xact_lock"));
    assert!(sql.contains("control.task_ledger_events"));
    assert!(sql.contains("REVOKE ALL ON ALL TABLES IN SCHEMA foreman_execution"));
    assert_eq!(MAX_GLOBAL_ACTIVE_ATTEMPTS, 4);
    assert_eq!(MAX_TASK_ACTIVE_ATTEMPTS, 1);
    assert_eq!(MAX_EXTENSION_ATTEMPTS, 3);
    assert_eq!(MAX_ACTIVE_TASK_REPLAY_ROWS, 256);
    assert!(sql.contains("p_limit NOT BETWEEN 1 AND 256"));
    assert!(sql.contains("ORDER BY a.task_ref"));
    assert!(sql.contains("TERMINAL_PENDING_VERIFICATION"));
    assert!(sql.contains("VERIFICATION_RECONCILE_REQUIRED"));
    assert!(sql.contains("ATTEMPT_CLOSED_PENDING_RELEASE"));
}

#[test]
fn sql_exposes_only_fixed_runtime_functions() {
    let sql = std::str::from_utf8(verify_embedded_extension().expect("extension").bytes())
        .expect("utf8 SQL");
    for function in [
        "record_preparation_observation_v1",
        "read_preparation_observation_v1",
        "record_promotion_intent_v1",
        "read_promotion_intent_v1",
        "record_task_promotion_v1",
        "reserve_worker_attempt_v1",
        "record_execution_environment_v1",
        "claim_worker_attempt_v1",
        "record_worker_observation_v1",
        "record_verification_v1",
        "stage_artifact_reference_v1",
        "finalize_staged_artifact_reference_v1",
        "claim_provider_dispatch_v1",
        "read_provider_dispatch_claim_v1",
        "record_attempt_closure_v1",
        "record_approval_evidence_v1",
        "read_worker_budget_v1",
        "read_staged_artifact_reference_v1",
        "read_task_promotion_source_v1",
        "read_pending_worker_attempt_v1",
        "read_execution_environment_rows_v1",
        "read_managed_evidence_v1",
        "read_attempt_closure_v1",
        "read_task_promotion_row_v1",
        "read_execution_authority_v1",
        "read_reference_event_rows_v1",
        "list_restart_task_refs_v1",
        "list_active_task_refs_v1",
        "read_task_replay_v1",
    ] {
        assert!(sql.contains(function), "missing {function}");
    }
    assert!(!sql.to_ascii_lowercase().contains("execute p_"));
    assert!(!sql.to_ascii_lowercase().contains("format("));
}

#[test]
fn execution_environment_descriptor_is_attempt_bound_replay_closed_and_queryable() {
    let sql = include_str!("../../../db/extensions/foreman-execution/v1.sql");
    let adapter = include_str!("../src/adapter.rs");
    let table_start = position(
        sql,
        "CREATE TABLE foreman_execution.execution_environments (",
    );
    let table_end = position(
        &sql[table_start..],
        "CREATE TABLE foreman_execution.worker_observations (",
    ) + table_start;
    let table = &sql[table_start..table_end];
    let record = function_body(
        sql,
        "record_execution_environment_v1",
        "claim_worker_attempt_v1",
    );
    let reserve = function_body(sql, "reserve_worker_attempt_v1", "canonical_json_v1");
    let reader = function_body(
        sql,
        "read_execution_environment_rows_v1",
        "read_worker_budget_v1",
    );
    let claim = function_body(
        sql,
        "claim_worker_attempt_v1",
        "record_worker_observation_v1",
    );
    let dispatch = function_body(
        sql,
        "claim_provider_dispatch_v1",
        "read_provider_dispatch_claim_v1",
    );

    for field in [
        "descriptor_schema varchar(64) NOT NULL",
        "environment_kind varchar(24) NOT NULL",
        "canonical_descriptor text NOT NULL",
        "distribution varchar(64) NOT NULL",
        "distribution_version varchar(128) NOT NULL",
        "distribution_identity_digest bytea NOT NULL",
        "linux_repository_path varchar(1024) NOT NULL",
        "linux_codex_home_path varchar(1024) NOT NULL",
        "launcher_path varchar(1024) NOT NULL",
        "launcher_version varchar(128) NOT NULL",
        "launcher_digest bytea NOT NULL",
        "node_path varchar(1024) NOT NULL",
        "node_version varchar(128) NOT NULL",
        "node_digest bytea NOT NULL",
        "npm_path varchar(1024) NOT NULL",
        "npm_version varchar(128) NOT NULL",
        "npm_digest bytea NOT NULL",
        "git_path varchar(1024) NOT NULL",
        "git_version varchar(128) NOT NULL",
        "git_digest bytea NOT NULL",
        "supervisor_path varchar(1024) NOT NULL",
        "supervisor_digest bytea NOT NULL",
        "keyring_library_manifest_ref varchar(128) NOT NULL",
        "keyring_library_manifest_digest bytea NOT NULL",
        "systemd_run_path varchar(1024) NOT NULL",
        "systemctl_path varchar(1024) NOT NULL",
        "supervisor_bootstrap_node_path varchar(1024) NOT NULL",
        "supervisor_bootstrap_node_version varchar(128) NOT NULL",
        "supervisor_bootstrap_node_digest bytea NOT NULL",
        "immutable_probe_lsattr_path varchar(1024) NOT NULL",
        "immutable_probe_lsattr_version varchar(128) NOT NULL",
        "immutable_probe_lsattr_digest bytea NOT NULL",
        "noninteractive_root_probe_path varchar(1024) NOT NULL",
        "noninteractive_root_probe_version varchar(128) NOT NULL",
        "noninteractive_root_probe_digest bytea NOT NULL",
        "process_fence_identity_digest bytea NOT NULL",
        "cargo_path varchar(1024) NOT NULL",
        "cargo_version varchar(128) NOT NULL",
        "cargo_digest bytea NOT NULL",
        "rustc_path varchar(1024) NOT NULL",
        "rustdoc_path varchar(1024) NOT NULL",
        "sandbox_helper_path varchar(1024) NOT NULL",
        "sandbox_helper_version varchar(128) NOT NULL",
        "sandbox_helper_digest bytea NOT NULL",
        "verification_toolchain_identity_digest bytea NOT NULL",
        "immutable_snapshot_ref varchar(128) NOT NULL",
        "immutable_snapshot_digest bytea NOT NULL",
        "sandbox_policy_ref varchar(128) NOT NULL",
        "sandbox_policy_digest bytea NOT NULL",
        "privilege_boundary_ref varchar(128) NOT NULL",
        "privilege_boundary_digest bytea NOT NULL",
        "credential_authority_kind varchar(48) NOT NULL",
        "credential_authority_digest bytea NOT NULL",
        "execution_domain_digest bytea NOT NULL",
        "environment_ref varchar(128) NOT NULL",
    ] {
        assert!(
            table.contains(field),
            "missing durable environment field {field}"
        );
    }
    assert!(table.contains("REFERENCES foreman_execution.task_promotions"));
    assert!(!table.to_ascii_lowercase().contains("credential_value"));
    assert!(!table.to_ascii_lowercase().contains("token"));
    assert!(!table.to_ascii_lowercase().contains("auth_json"));
    assert!(
        record.contains("foreman_execution.canonical_json_v1(v_descriptor - 'identity_digest')")
    );
    assert!(record.contains("pg_catalog.sha256("));
    assert!(record.contains("FOREMAN_EXECUTION_ENVIRONMENT_DIGEST_MISMATCH"));
    assert!(record.contains("FOREMAN_EXECUTION_ENVIRONMENT_SUBSTITUTION"));
    assert!(
        record.contains(
            "'keyring_library_manifest_digest', v_linux->'keyring_library_manifest_digest'"
        )
    );
    assert!(record.contains("(v_toolchain->'sandbox_helper')"));
    assert!(record.contains("'/usr/bin/bwrap'"));
    assert!(record.contains("v_existing.environment_ref IS DISTINCT FROM p_environment_ref"));
    assert!(
        record.contains("v_existing.canonical_descriptor IS DISTINCT FROM v_canonical_descriptor")
    );
    assert!(record.contains("v_existing.linux_repository_path IS DISTINCT FROM v_linux->>'cwd'"));
    assert!(record.contains("v_existing.immutable_snapshot_ref IS DISTINCT FROM"));
    assert!(record.contains("v_existing.sandbox_policy_ref IS DISTINCT FROM"));
    assert!(record.contains("'lattice.wsl2-sandbox-template/1.0'"));
    assert!(record.contains("'$GIT_CONTROL_ROOT/candidate-index'"));
    assert!(record.contains("'sandbox_cwd', 'file://' || (v_linux->>'cwd')"));
    assert!(
        record.contains("foreman_execution.canonical_json_v1(v_sandbox_policy_template), 'UTF8'")
    );
    assert!(
        record
            .contains("v_sandbox_policy->>'policy_digest' IS DISTINCT FROM v_expected_nested_ref")
    );
    assert!(record.contains("candidate_path.value !~ '^/[A-Za-z0-9._~/-]+$'"));
    assert!(record.contains(") AS toolchain_path(value)"));
    for canonical_path_guard in [
        "toolchain_path.value ~ '(^|/)\\.\\.?(/|$)'",
        "pg_catalog.strpos(toolchain_path.value, '//') > 0",
        "pg_catalog.right(toolchain_path.value, 1) = '/'",
        "toolchain_path.value ~ '^/mnt/'",
        "toolchain_path.value !~ '^/[A-Za-z0-9._~/-]+$'",
    ] {
        assert!(
            record.contains(canonical_path_guard),
            "missing durable toolchain canonical-path guard: {canonical_path_guard}"
        );
    }
    for durable_path in [
        "(v_toolchain->>'task_root')",
        "(v_toolchain->>'isolation_root')",
        "(v_toolchain->>'cargo_home')",
        "(v_toolchain->'npm'->>'path')",
        "(v_toolchain->'cargo'->>'path')",
        "(v_toolchain->'rustc'->>'path')",
        "(v_toolchain->'rustdoc'->>'path')",
        "(v_toolchain->'sandbox'->>'path')",
    ] {
        assert!(
            record.contains(durable_path),
            "missing durable canonical-path input: {durable_path}"
        );
    }
    assert!(record.contains(
        "v_immutable_snapshot->'trees'->(tree.tree_name)->>'root'\n                        ~ '(^|/)\\.\\.?(/|$)'"
    ));
    assert!(record.contains("v_existing.privilege_boundary_ref IS DISTINCT FROM"));
    assert!(record.contains("v_fence->'immutable_probe_lsattr'->>'path'"));
    assert!(record.contains("v_fence->'noninteractive_root_probe'->>'path'"));
    assert!(record.contains("pg_catalog.count(DISTINCT tree_value->>'root')"));
    assert!(record.contains("left_tree.tree_name < right_tree.tree_name"));
    assert!(record.contains(
        "left_tree.tree_value->>'root',\n                        right_tree.tree_value->>'root' || '/'"
    ));
    assert!(record.contains(
        "right_tree.tree_value->>'root',\n                        left_tree.tree_value->>'root' || '/'"
    ));
    assert_eq!(record.matches("pg_catalog.starts_with(").count(), 17);
    assert!(!record.contains(" NOT LIKE "));
    for executable_path in [
        "v_linux->>'launcher_path'",
        "v_toolchain->'sandbox'->>'path'",
        "v_linux->>'supervisor_path'",
        "v_linux->>'node_path'",
        "v_toolchain->'npm'->>'path'",
        "v_toolchain->'cargo'->>'path'",
        "v_toolchain->'rustc'->>'path'",
        "v_toolchain->'rustdoc'->>'path'",
    ] {
        assert!(
            record.contains(executable_path),
            "missing independent immutable-tree containment check: {executable_path}"
        );
    }
    assert!(record.contains("v_linux->>'keyring_daemon_path' IS DISTINCT FROM"));
    assert!(record.contains("v_linux->>'keyring_library_path' IS DISTINCT FROM"));
    assert!(record.contains("FROM foreman_execution.read_execution_environment_rows_v1"));
    assert!(record.contains("IF v_active_anchor_count <> 0 THEN"));
    assert!(reader.contains("FOREMAN_EXECUTION_ENVIRONMENT_ANCHOR_MISMATCH"));
    assert!(reader.contains("v_environment.keyring_library_manifest_ref IS DISTINCT FROM"));
    assert!(reader.contains("v_environment.sandbox_helper_path IS DISTINCT FROM"));
    assert!(reader.contains("v_environment.immutable_snapshot_digest IS DISTINCT FROM"));
    assert!(reader.contains("v_environment.sandbox_policy_digest IS DISTINCT FROM"));
    assert!(reader.contains("v_expected_sandbox_policy_ref"));
    assert!(reader.contains(
        "v_descriptor->'sandbox_policy'->>'policy_digest'\n                IS DISTINCT FROM v_expected_sandbox_policy_ref"
    ));
    assert!(reader.contains("sandbox_path.value !~ '^/[A-Za-z0-9._~/-]+$'"));
    assert!(reader.contains("sandbox_path.value ~ '(^|/)\\.\\.?(/|$)'"));
    assert!(reader.contains("pg_catalog.strpos(sandbox_path.value, '//') > 0"));
    for replay_path in [
        "(v_descriptor->'verification_toolchain'->>'isolation_root')",
        "(v_descriptor->'verification_toolchain'->'cargo'->>'path')",
        "(v_descriptor->'verification_toolchain'->'sandbox'->>'path')",
        "(v_descriptor->'immutable_snapshot'->'trees'->'codex'->>'root')",
        "(v_descriptor->'immutable_snapshot'->'trees'->'rust'->>'root')",
    ] {
        assert!(
            reader.contains(replay_path),
            "fresh-process reader omits canonical path: {replay_path}"
        );
    }
    assert!(reader.contains("v_environment.privilege_boundary_digest IS DISTINCT FROM"));
    assert!(reader.contains("v_environment.immutable_probe_lsattr_digest IS DISTINCT FROM"));
    assert!(reader.contains("v_environment.noninteractive_root_probe_digest IS DISTINCT FROM"));
    assert!(reader.contains("ORDER BY environment.attempt_number"));
    assert!(reader.contains("HAVING pg_catalog.count(*) <> 1"));
    assert!(reader.contains(
        "execution-environment:sha256:0000000000000000000000000000000000000000000000000000000000000001"
    ));
    assert!(reserve.contains("FROM foreman_execution.read_execution_environment_rows_v1"));
    assert!(
        sql.contains(
            "CREATE TABLE foreman_execution.worker_attempts (\n    task_ref bytea NOT NULL"
        )
    );
    assert!(sql.contains(
        "CREATE TABLE foreman_execution.pending_worker_claims (\n    task_ref bytea NOT NULL"
    ));
    assert!(
        sql.matches("execution_environment_ref varchar(128) NOT NULL")
            .count()
            >= 2
    );
    assert!(claim.contains("p_execution_environment_ref text"));
    assert!(claim.contains(
        "execution-environment:sha256:0000000000000000000000000000000000000000000000000000000000000001"
    ));
    assert!(claim.contains("FOREMAN_EXECUTION_ENVIRONMENT_REQUIRED"));
    assert!(claim.contains(
        "v_existing.execution_environment_ref IS DISTINCT FROM p_execution_environment_ref"
    ));
    assert!(claim.contains("environment.environment_ref = p_execution_environment_ref"));
    assert!(dispatch.contains("v_attempt.execution_environment_ref"));
    assert!(dispatch.contains(
        "execution-environment:sha256:0000000000000000000000000000000000000000000000000000000000000001"
    ));
    assert!(dispatch.contains("FOREMAN_PROVIDER_DISPATCH_EXECUTION_ENVIRONMENT_NOT_CURRENT"));
    assert!(adapter.contains("pub fn reserve_worker_attempt_with_execution_environment_ref("));
    assert!(adapter.contains("pub fn claim_worker_attempt_with_execution_environment_ref("));
    assert!(adapter.contains("pub fn record_execution_environment("));
    assert!(adapter.contains("pub fn load_execution_environments("));
    assert!(adapter.contains("pub fn load_execution_environment("));
}

#[test]
fn pending_non_native_environment_may_be_missing_until_record_but_claim_stays_closed() {
    let sql = include_str!("../../../db/extensions/foreman-execution/v1.sql");
    let reader = function_body(
        sql,
        "read_execution_environment_rows_v1",
        "read_worker_budget_v1",
    )
    .split_whitespace()
    .collect::<Vec<_>>()
    .join(" ");
    let claim = function_body(
        sql,
        "claim_worker_attempt_v1",
        "record_worker_observation_v1",
    )
    .split_whitespace()
    .collect::<Vec<_>>()
    .join(" ");
    let reserve = function_body(sql, "reserve_worker_attempt_v1", "canonical_json_v1")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let pending_replay = reserve
        .split("SELECT * INTO v_pending FROM ONLY foreman_execution.pending_worker_claims")
        .nth(1)
        .expect("pending reservation exact-replay branch")
        .split("IF p_model NOT IN")
        .next()
        .expect("new reservation branch follows pending exact replay");
    let close = function_body(
        sql,
        "close_pending_worker_attempt_v1",
        "begin_restart_writer_blocker_guard_v1",
    )
    .split_whitespace()
    .collect::<Vec<_>>()
    .join(" ");
    let native_ref = "execution-environment:sha256:0000000000000000000000000000000000000000000000000000000000000001";

    assert!(reader.contains("pending.execution_environment_ref::text, 'PENDING'::text AS anchor_state FROM ONLY foreman_execution.pending_worker_claims AS pending"));
    assert!(reader.contains("attempt.execution_environment_ref::text, 'ACTIVE'::text AS anchor_state FROM ONLY foreman_execution.worker_attempts AS attempt"));
    assert!(reader.contains(&format!(
        "anchor.execution_environment_ref <> '{native_ref}' AND NOT EXISTS ( SELECT 1 FROM ONLY foreman_execution.execution_environments AS environment WHERE environment.task_ref = anchor.task_ref AND environment.attempt_number = anchor.attempt_number AND environment.attempt_id = anchor.attempt_id AND environment.packet_digest = anchor.packet_digest AND environment.environment_ref = anchor.execution_environment_ref ) AND ( anchor.anchor_state = 'ACTIVE' OR EXISTS ( SELECT 1 FROM ONLY foreman_execution.execution_environments AS environment WHERE environment.task_ref = anchor.task_ref AND environment.attempt_number = anchor.attempt_number ) )"
    )));
    assert!(claim.contains(&format!(
        "IF p_execution_environment_ref = '{native_ref}' THEN"
    )));
    assert!(claim.contains("ELSE IF NOT FOUND THEN RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'FOREMAN_EXECUTION_ENVIRONMENT_REQUIRED'; END IF; IF v_environment.environment_ref IS DISTINCT FROM p_execution_environment_ref OR v_environment.attempt_id IS DISTINCT FROM p_attempt_id OR v_environment.packet_digest IS DISTINCT FROM p_packet_digest THEN"));
    assert!(pending_replay.contains("PERFORM pg_catalog.count(*) FROM foreman_execution.read_execution_environment_rows_v1(p_task_ref); RETURN 'EXACT_REPLAY';"));

    let close_environment_validation = position(
        &close,
        "PERFORM pg_catalog.count(*) FROM foreman_execution.read_execution_environment_rows_v1(p_task_ref);",
    );
    let close_attempt_insert = position(&close, "INSERT INTO foreman_execution.worker_attempts (");
    assert!(close.contains(&format!(
        "IF v_pending.execution_environment_ref <> '{native_ref}' AND NOT EXISTS ( SELECT 1 FROM ONLY foreman_execution.execution_environments AS environment WHERE environment.task_ref = v_pending.task_ref AND environment.attempt_number = v_pending.attempt_number AND environment.attempt_id = v_pending.attempt_id AND environment.packet_digest = v_pending.packet_digest AND environment.environment_ref = v_pending.execution_environment_ref ) THEN RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'FOREMAN_PENDING_CLOSURE_EXECUTION_ENVIRONMENT_REQUIRED'; END IF;"
    )));
    assert!(close_environment_validation < close_attempt_insert);
}

#[test]
fn active_environment_anchor_is_validated_before_closure_replay_or_insert() {
    let sql = include_str!("../../../db/extensions/foreman-execution/v1.sql");
    for (function, next_function) in [
        (
            "record_attempt_closure_v1",
            "close_retained_worker_without_provider_effect_v1",
        ),
        (
            "close_retained_worker_without_provider_effect_v1",
            "close_pending_worker_attempt_v1",
        ),
    ] {
        let closure = function_body(sql, function, next_function)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let lock = position(
            &closure,
            "PERFORM pg_catalog.pg_advisory_xact_lock(7212400260826);",
        );
        let environment_gate = position(
            &closure,
            "PERFORM pg_catalog.count(*) FROM foreman_execution.read_execution_environment_rows_v1(p_task_ref);",
        );
        let existing_closure = position(
            &closure,
            "SELECT * INTO v_existing FROM ONLY foreman_execution.attempt_closures",
        );
        let closure_insert = position(&closure, "INSERT INTO foreman_execution.attempt_closures (");
        assert!(lock < environment_gate);
        assert!(environment_gate < existing_closure);
        assert!(environment_gate < closure_insert);
    }
}

#[test]
fn execution_environment_sql_rejects_bounded_recursive_secret_leaves_before_insert() {
    let sql = include_str!("../../../db/extensions/foreman-execution/v1.sql");
    let record = function_body(
        sql,
        "record_execution_environment_v1",
        "claim_worker_attempt_v1",
    );
    let scan = position(record, "WITH RECURSIVE descriptor_string_nodes");
    let insert = position(
        record,
        "INSERT INTO foreman_execution.execution_environments (",
    );
    assert!(scan < insert);
    assert!(record.contains("v_descriptor_scan_nodes > 512"));
    assert!(record.contains("v_descriptor_scan_depth_exceeded"));
    assert!(record.contains("pg_catalog.octet_length(string_value) > 4096"));
    for pattern in [
        "bearer[[:space:]]",
        "password|passphrase|passwd|pwd|token",
        "api[ _-]?key",
        "ghp_|gho_|ghu_|ghs_|ghr_|github_pat_",
        "(^|[^[:alnum:]])sk-",
        "FOREMAN_EXECUTION_ENVIRONMENT_INPUT_REJECTED",
    ] {
        assert!(
            record.contains(pattern),
            "missing descriptor secret guard {pattern}"
        );
    }
}

#[test]
fn restart_writer_blocker_guard_serializes_with_every_durable_attempt_lane() {
    let sql = include_str!("../../../db/extensions/foreman-execution/v1.sql");
    let begin = function_body(
        sql,
        "begin_restart_writer_blocker_guard_v1",
        "end_restart_writer_blocker_guard_v1",
    );
    let end = function_body(
        sql,
        "end_restart_writer_blocker_guard_v1",
        "record_approval_evidence_v1",
    );
    assert!(begin.contains("pg_catalog.pg_advisory_lock(7212400260826)"));
    assert!(end.contains("pg_catalog.pg_advisory_unlock(7212400260826)"));
    for function in [
        "record_worker_observation_v1",
        "record_verification_v1",
        "record_attempt_closure_v1",
        "close_retained_worker_without_provider_effect_v1",
    ] {
        let start = sql
            .find(&format!("CREATE FUNCTION foreman_execution.{function}"))
            .expect("durable lane function");
        let body = &sql[start..];
        let end = body.find("$$;").expect("durable lane function end");
        assert!(body[..end].contains("pg_catalog.pg_advisory_xact_lock(7212400260826)"));
    }
    assert!(sql.contains(
        "GRANT EXECUTE ON FUNCTION foreman_execution.begin_restart_writer_blocker_guard_v1("
    ));
    assert!(sql.contains(
        "GRANT EXECUTE ON FUNCTION foreman_execution.end_restart_writer_blocker_guard_v1("
    ));
}

#[test]
fn worker_observation_identity_is_durable_and_rotates_only_on_reconciliation() {
    let sql = include_str!("../../../db/extensions/foreman-execution/v1.sql");
    let adapter = include_str!("../src/adapter.rs");
    let table_start = position(sql, "CREATE TABLE foreman_execution.worker_observations (");
    let table_end = position(
        &sql[table_start..],
        "CREATE UNIQUE INDEX worker_observations_one_terminal",
    ) + table_start;
    let table = &sql[table_start..table_end];
    let record = function_body(
        sql,
        "record_worker_observation_v1",
        "record_verification_v1",
    );
    let reader = function_body(
        sql,
        "read_worker_observation_rows_v1",
        "read_verification_rows_v1",
    );

    assert!(table.contains("app_server_identity_digest bytea NOT NULL"));
    assert!(table.contains("octet_length(app_server_identity_digest) = 32"));
    assert!(table.contains("app_server_identity_digest <> decode(repeat('00', 32), 'hex')"));
    assert!(record.contains("p_app_server_identity_digest bytea"));
    assert!(record.contains(
        "v_existing.app_server_identity_digest IS DISTINCT FROM p_app_server_identity_digest"
    ));
    assert!(record.contains("SELECT o.app_server_generation, o.app_server_identity_digest"));
    assert!(record.contains("ORDER BY o.observation_ordinal DESC"));
    assert!(record.contains("p_observation_kind <> 'RECONCILED'"));
    assert!(record.contains("FOREMAN_APP_SERVER_IDENTITY_DRIFT"));
    assert!(record.contains("app_server_generation, app_server_identity_digest"));
    assert!(reader.contains("app_server_identity_digest bytea"));
    assert!(reader.contains("o.app_server_identity_digest"));
    assert!(adapter.contains("digest_bytes(record.app_server_identity_digest())?"));
    assert!(adapter.contains("pg_catalog.encode(app_server_identity_digest,'hex')"));
}

#[test]
fn preparation_observation_is_bounded_rebuttable_and_not_a_task_state() {
    let sql = std::str::from_utf8(verify_embedded_extension().expect("extension").bytes())
        .expect("utf8 SQL");
    let record = function_body(
        sql,
        "record_preparation_observation_v1",
        "read_preparation_observation_v1",
    );
    let read = function_body(
        sql,
        "read_preparation_observation_v1",
        "record_promotion_intent_v1",
    );
    assert!(sql.contains("CREATE TABLE foreman_execution.preparation_observations"));
    assert!(sql.contains("task_ref bytea PRIMARY KEY"));
    assert!(sql.contains(
        "CONSTRAINT preparation_observations_intake_stream_fk FOREIGN KEY (intake_stream_id)\n        REFERENCES control.task_submission_envelopes (stream_id)"
    ));
    assert!(sql.contains(
        "CONSTRAINT preparation_observations_intake_event_fk FOREIGN KEY (intake_event_digest)\n        REFERENCES control.task_ledger_events (event_digest)"
    ));
    assert!(!sql.contains(
        "FOREIGN KEY (\n        intake_stream_id, intake_event_digest\n    ) REFERENCES control.task_ledger_events (stream_id, event_digest)"
    ));
    assert!(record.contains("'WORKTREE_NOT_CLEAN'"));
    assert!(record.contains("'PROJECT_REGISTRY_CURRENTNESS_CONFLICT'"));
    assert!(record.contains("'CLEARED'"));
    assert!(record.contains("observation_generation = observation_generation + 1"));
    assert!(record.contains("RETURN 'EXACT_REPLAY'"));
    assert!(record.contains("control.task_submission_envelopes AS submission"));
    assert!(!record.to_ascii_lowercase().contains("task_state"));
    assert!(!record.contains("INSERT INTO control.task_ledger"));
    assert!(read.contains("FROM ONLY foreman_execution.preparation_observations"));
    assert!(read.contains("FROM ONLY control.task_submission_envelopes AS submission"));
    assert!(read.contains("JOIN ONLY control.task_ledger_events AS intake_event"));
    assert!(read.contains("submission.stream_id = v_existing.intake_stream_id"));
    assert!(read.contains("submission.event_digest = v_existing.intake_event_digest"));
    assert!(read.contains("FOREMAN_PREPARATION_OBSERVATION_LINEAGE_MISMATCH"));
    assert!(!read.to_ascii_lowercase().contains("update "));
    assert!(!read.to_ascii_lowercase().contains("insert "));
}

#[test]
fn promotion_intent_pins_clean_source_before_successor_and_fails_closed_on_ambiguity() {
    let sql = std::str::from_utf8(verify_embedded_extension().expect("extension").bytes())
        .expect("utf8 SQL");
    let intent = function_body(
        sql,
        "record_promotion_intent_v1",
        "read_promotion_intent_v1",
    );
    let reader = function_body(sql, "read_promotion_intent_v1", "record_task_promotion_v1");
    let promotion = function_body(sql, "record_task_promotion_v1", "reserve_worker_attempt_v1");
    assert!(sql.contains("CREATE TABLE foreman_execution.promotion_intents"));
    assert!(sql.contains(
        "CONSTRAINT promotion_intents_intake_stream_fk FOREIGN KEY (intake_stream_id)\n        REFERENCES control.task_submission_envelopes (stream_id)"
    ));
    assert!(sql.contains(
        "CONSTRAINT promotion_intents_intake_event_fk FOREIGN KEY (intake_event_digest)\n        REFERENCES control.task_ledger_events (event_digest)"
    ));
    assert!(sql.contains("AND source_clean"));
    assert!(intent.contains("OR NOT p_source_clean"));
    assert!(intent.contains("RETURN 'EXACT_REPLAY'"));
    assert!(intent.contains("submission.stream_id = v_existing.intake_stream_id"));
    assert!(intent.contains("submission.event_digest = v_existing.intake_event_digest"));
    assert!(intent.contains("FOREMAN_PROMOTION_INTENT_LINEAGE_MISMATCH"));
    assert!(
        intent.find("FOREMAN_PROMOTION_INTENT_LINEAGE_MISMATCH")
            < intent.find("RETURN 'EXACT_REPLAY'")
    );
    assert!(intent.find("RETURN 'EXACT_REPLAY'") < intent.find("project.project_class"));
    assert!(reader.contains("created.action_id = 'MANAGED_GENERAL_TASK_V1'"));
    assert!(reader.contains("submission.stream_id = v_intent.intake_stream_id"));
    assert!(reader.contains("submission.event_digest = v_intent.intake_event_digest"));
    assert!(reader.contains("FOREMAN_PROMOTION_INTENT_LINEAGE_MISMATCH"));
    assert!(reader.contains("v_candidate_count > 1"));
    assert!(reader.contains("FOREMAN_PROMOTION_SUCCESSOR_AMBIGUOUS"));
    assert!(promotion.contains("foreman_execution.promotion_intents AS intent"));
    assert!(promotion.contains("AND intent.source_clean"));
    assert!(promotion.contains("JOIN ONLY control.task_submission_envelopes AS submission"));
    assert!(promotion.contains("JOIN ONLY control.task_ledger_events AS intake_event"));
    assert!(promotion.contains("submission.stream_id = intent.intake_stream_id"));
    assert!(promotion.contains("submission.event_digest = intent.intake_event_digest"));
    let lineage = promotion
        .find("FOREMAN_PROMOTION_INTENT_LINEAGE_MISMATCH")
        .expect("successor write lineage assertion");
    let exact_replay = promotion
        .find("SELECT * INTO v_existing")
        .expect("promotion exact replay lookup");
    let child_effect = promotion
        .find("insert_child_event_v1(")
        .expect("promotion child effect");
    assert!(lineage < exact_replay && lineage < child_effect);
}

#[test]
fn provider_dispatch_claims_are_operation_bound_replayable_and_not_runtime_tables() {
    let sql = std::str::from_utf8(verify_embedded_extension().expect("extension").bytes())
        .expect("utf8 SQL");
    let adapter = include_str!("../src/adapter.rs");
    let claim = function_body(
        sql,
        "claim_provider_dispatch_v1",
        "read_provider_dispatch_claim_v1",
    );
    let read = function_body(
        sql,
        "read_provider_dispatch_claim_v1",
        "record_attempt_closure_v1",
    );
    let closure = function_body(
        sql,
        "record_attempt_closure_v1",
        "record_approval_evidence_v1",
    );

    assert!(sql.contains("CREATE TABLE foreman_execution.provider_dispatch_claims"));
    for kind in [
        "WORKER_THREAD",
        "WORKER_TURN",
        "REVIEW_THREAD",
        "REVIEW_TURN",
    ] {
        assert!(claim.contains(kind), "missing operation anchor: {kind}");
    }
    assert!(claim.contains("v_attempt.payload_digest IS DISTINCT FROM p_anchor_digest"));
    assert!(claim.contains("v_attempt.packet_digest IS DISTINCT FROM p_supporting_digest"));
    assert!(claim.contains("THREAD_ACCEPTED"));
    assert!(claim.contains("TERMINAL_COMPLETED"));
    assert!(claim.contains("lattice.managed-review-lifecycle/1.0"));
    assert!(claim.contains("writer_lease.writer_lease_heads AS lease"));
    assert!(claim.contains("lease.current_status = 'ACTIVE'"));
    assert!(!claim.contains("lease.current_status IN ('ACTIVE', 'SUSPECT')"));
    assert!(claim.contains("lease.current_attempt_id = v_attempt.attempt_id"));
    assert!(claim.contains("lease.current_fencing_token = v_attempt.writer_fence"));
    assert!(claim.contains("'WORK-' || pg_catalog.upper(pg_catalog.substr("));
    assert!(claim.contains(
        "lease.current_expires_at::timestamp with time zone > pg_catalog.clock_timestamp()"
    ));
    assert!(claim.contains("control.runtime_admission AS admission"));
    assert!(claim.contains("v_admission.admission_mode IS DISTINCT FROM 'ACTIVE'"));
    assert!(claim.contains("lease.current_daemon_instance_id = v_admission.daemon_instance_id"));
    assert!(claim.contains("lease.current_daemon_epoch = v_admission.daemon_epoch"));
    assert!(claim.contains("FOREMAN_PROVIDER_DISPATCH_WRITER_FENCE_STALE"));
    assert!(claim.contains("foreman_execution.approval_evidence AS authority"));
    assert!(claim.contains("authority.authority_digest = v_attempt.approval_receipt_digest"));
    assert!(claim.contains("authority.successor_stream_id = v_attempt.successor_stream_id"));
    assert!(claim.contains("authority.task_spec_digest = v_attempt.task_spec_digest"));
    assert!(
        claim.contains("authority.approval_subject_digest = promotion.approval_subject_digest")
    );
    assert!(claim.contains("authority.budget_digest = v_attempt.budget_digest"));
    assert!(claim.contains("authority.capability = 'LOCAL_REVERSIBLE_TASK_EXECUTION'"));
    assert!(
        claim.contains(
            "pg_catalog.clock_timestamp() >= authority.issued_at::timestamp with time zone"
        )
    );
    assert!(
        claim.contains(
            "pg_catalog.clock_timestamp() < authority.expires_at::timestamp with time zone"
        )
    );
    assert!(claim.contains("control.project_registry_projects AS project"));
    assert!(claim.contains("project.project_class = 'USER_PROJECT'"));
    assert!(claim.contains("project.authority_lifecycle = 'ACTIVE'"));
    assert!(claim.contains("project.pending_observation_digest IS NULL"));
    assert!(claim.contains("project.authority_snapshot_id = promotion.project_snapshot_id"));
    assert!(
        claim.contains(
            "project.authority_receipt_digest = promotion.project_authority_receipt_digest"
        )
    );
    assert!(claim.contains("FOR SHARE OF authority, promotion, project"));
    assert!(claim.contains("FOREMAN_PROVIDER_DISPATCH_AUTHORITY_NOT_CURRENT"));
    assert!(claim.contains("p_foreman_stream_id bytea"));
    assert!(claim.contains("control.task_ledger_streams AS foreman_stream"));
    assert!(claim.contains("control.task_ledger_foreman_snapshots AS foreman_snapshot"));
    assert!(claim.contains("foreman_stream.checkpoint_digest = p_foreman_checkpoint_digest"));
    assert!(claim.contains("foreman_snapshot.generation = p_foreman_generation"));
    assert!(claim.contains("foreman_snapshot.foreman_state = 'ACTIVE'"));
    assert!(claim.contains("foreman_snapshot.worker_id = 'sole-foreman-v1'"));
    assert!(claim.contains("foreman_snapshot.thread_id = 'lattice-devos-sole-foreman-v1'"));
    assert!(claim.contains("FOREMAN_PROVIDER_DISPATCH_FOREMAN_FENCE_STALE"));
    assert!(adapter.contains("foreman_coordination_identity()"));
    assert!(adapter.contains("VerifiedStream::vacant("));
    assert!(adapter.contains("RuntimeKind::Live"));
    assert!(adapter.contains("foreman_coordination_stream_id"));
    assert!(claim.contains("FOREMAN_PROVIDER_DISPATCH_SUBSTITUTION"));
    assert!(claim.contains("RETURN 'EXACT_REPLAY'"));
    assert!(
        claim.find("RETURN 'EXACT_REPLAY'")
            < claim.find("FOREMAN_PROVIDER_DISPATCH_AUTHORITY_NOT_CURRENT"),
        "historical exact replay must precede the current-authority gate"
    );
    assert!(read.contains("FROM ONLY foreman_execution.provider_dispatch_claims"));
    assert!(sql.contains("'PROVIDER_DISPATCH_' || dispatch.operation_kind"));
    let replay = &sql[position(
        sql,
        "CREATE FUNCTION foreman_execution.read_task_replay_v1(",
    )..];
    for (kind, phase) in [
        ("WORKER_ATTEMPT", 2),
        ("PROVIDER_DISPATCH_WORKER_THREAD", 3),
        ("PROVIDER_DISPATCH_WORKER_TURN", 4),
        ("PROVIDER_DISPATCH_REVIEW_THREAD", 5),
        ("PROVIDER_DISPATCH_REVIEW_TURN", 6),
    ] {
        assert!(
            replay.contains(&format!("WHEN '{kind}' THEN {phase}")),
            "missing replay phase for {kind}"
        );
    }
    assert!(replay.contains("replay.record_ordinal, replay.record_kind"));
    assert!(closure.contains("dispatch.operation_kind = 'WORKER_THREAD'"));
    assert!(closure.contains("dispatch.operation_kind IN ('REVIEW_THREAD', 'REVIEW_TURN')"));
    assert!(closure.contains("'event_type') = 'TURN_TERMINAL'"));
    assert!(
        closure.contains("'TURN_RECONCILED',\n                            'THREAD_RECONCILED'")
    );
    assert!(closure.contains("anchor.payload->>'event_type' = 'THREAD_STARTED'"));
    assert!(closure.contains("anchor.payload->>'event_type' = 'THREAD_RECONCILED'"));
    assert!(closure.contains("(admitted_turn.payload->>'sequence')::bigint + 1"));
    assert!(closure.contains("exact_terminal.payload->>'app_server_generation' ="));
    assert!(closure.contains("FOREMAN_ATTEMPT_CLOSURE_REVIEWER_STILL_POSSIBLY_ACTIVE"));
    assert!(
        sql.contains("REVOKE ALL ON ALL TABLES IN SCHEMA foreman_execution FROM lattice_runtime")
    );
    assert!(sql.contains("GRANT EXECUTE ON FUNCTION foreman_execution.claim_provider_dispatch_v1"));
}

#[test]
fn typed_attempt_closure_releases_capacity_without_fabricating_verification() {
    let sql = std::str::from_utf8(verify_embedded_extension().expect("extension").bytes())
        .expect("utf8 SQL");
    let claim = function_body(
        sql,
        "claim_worker_attempt_v1",
        "record_worker_observation_v1",
    );
    let closure = function_body(
        sql,
        "record_attempt_closure_v1",
        "record_approval_evidence_v1",
    );

    assert!(sql.contains("CREATE TABLE foreman_execution.attempt_closures"));
    assert!(claim.contains("foreman_execution.attempt_closures AS closure"));
    assert!(closure.contains("payload_schema = 'lattice.managed-blocker.v1'"));
    assert!(closure.contains("producer_id = 'lattice-foreman'"));
    assert!(closure.contains("a.writer_fence = p_writer_fence"));
    assert!(closure.contains("FOREMAN_ATTEMPT_CLOSURE_PROVIDER_STILL_POSSIBLY_ACTIVE"));
    assert!(closure.contains("dispatch.operation_kind = 'WORKER_TURN'"));
    assert!(closure.contains("accepted.observation_kind = 'THREAD_ACCEPTED'"));
    assert!(closure.contains("observed.observation_kind <> 'THREAD_ACCEPTED'"));
    assert!(closure.contains("INSERT INTO foreman_execution.attempt_closures"));
    assert!(!closure.contains("INSERT INTO foreman_execution.verification_records"));
    for (code, reason) in [
        (
            "LATTICE_MANAGED_MODEL_PROBE_TIMEOUT_RECONCILIATION_REQUIRED",
            "WORKER_MODEL_PROBE_TIMED_OUT_EXACT_PRESTART_SUBTREE_REAPED",
        ),
        (
            "LATTICE_MANAGED_REVIEW_MODEL_PROBE_TIMEOUT_NO_PROVIDER_EFFECT",
            "REVIEW_MODEL_PROBE_TIMED_OUT_NO_REVIEW_PROVIDER_EFFECT",
        ),
    ] {
        assert!(sql.contains(code), "missing SQL blocker code {code}");
        assert!(
            closure.contains(reason),
            "missing SQL blocker reason {reason}"
        );
    }
    assert!(closure.contains("dispatch.operation_kind IN ('REVIEW_THREAD', 'REVIEW_TURN')"));
}

#[test]
fn artifact_stage_recomputes_descriptor_and_scans_every_closed_media_input_before_insert() {
    let sql = std::str::from_utf8(verify_embedded_extension().expect("extension").bytes())
        .expect("utf8 SQL");
    let stage = function_body(
        sql,
        "stage_artifact_reference_v1",
        "finalize_staged_artifact_reference_v1",
    );
    let content_guard = position(
        stage,
        "p_content_digest IS DISTINCT FROM pg_catalog.sha256(p_evidence_bytes)",
    );
    let secret_guard = position(stage, "FOREMAN_ARTIFACT_SECRET_REJECTED");
    let media_guard = position(stage, "FOREMAN_ARTIFACT_MEDIA_TYPE_REJECTED");
    let descriptor_guard = position(stage, "FOREMAN_ARTIFACT_DESCRIPTOR_DIGEST_MISMATCH");
    let insert = position(
        stage,
        "INSERT INTO foreman_execution.staged_artifact_references",
    );

    assert!(content_guard < insert);
    assert!(media_guard < insert);
    assert!(secret_guard < insert);
    assert!(descriptor_guard < insert);
    assert!(stage.contains("FOREMAN_ARTIFACT_CONTENT_DIGEST_MISMATCH"));
    assert!(stage.contains("p_media_type <> 'application/json'"));
    assert!(stage.contains("pg_catalog.convert_from(p_evidence_bytes, 'UTF8')::jsonb"));
    assert!(stage.contains("p_descriptor_bytes IS DISTINCT FROM v_expected_descriptor_bytes"));
    assert!(stage.contains("pg_catalog.sha256(v_descriptor_frame)"));
    assert!(stage.contains("lattice-hash-1"));
    assert!(stage.contains("lattice-cjson-1"));
    assert!(stage.contains("authorization"));
    assert!(stage.contains("://[^/?#[:space:]]*@"));
}

#[test]
fn verified_approval_ingress_is_not_self_attestable_by_general_runtime() {
    let sql = std::str::from_utf8(verify_embedded_extension().expect("extension").bytes())
        .expect("utf8 SQL");
    let record = function_body(
        sql,
        "record_approval_evidence_v1",
        "read_extension_identity_v1",
    );
    let role_guard = position(record, "FOREMAN_APPROVAL_OWNER_ROLE_REQUIRED");
    let retained_lookup = position(
        record,
        "SELECT * INTO v_existing FROM ONLY foreman_execution.approval_evidence",
    );
    let owner_insert = position(
        record,
        "INSERT INTO foreman_execution.approval_owner_snapshots",
    );
    assert!(record.contains("p_authority_source = 'VERIFIED_APPROVAL'"));
    assert!(record.contains("pg_catalog.pg_has_role("));
    assert!(record.contains("session_user, 'lattice_migrator', 'MEMBER'"));
    assert!(role_guard < retained_lookup && role_guard < owner_insert);
    assert!(
        sql.contains("REVOKE ALL ON ALL TABLES IN SCHEMA foreman_execution FROM lattice_runtime")
    );
}

#[test]
fn retained_blocker_closure_requires_a_separate_exact_no_effect_proof() {
    let sql = include_str!("../../../db/extensions/foreman-execution/v1.sql");
    let reserve = function_body(sql, "reserve_worker_attempt_v1", "claim_worker_attempt_v1");
    let claim = function_body(
        sql,
        "claim_worker_attempt_v1",
        "record_worker_observation_v1",
    );
    let closure = function_body(
        sql,
        "close_retained_worker_without_provider_effect_v1",
        "close_pending_worker_attempt_v1",
    );
    let observation = function_body(
        sql,
        "record_worker_observation_v1",
        "record_verification_v1",
    );

    assert!(sql.contains("reconciliation_proof_descriptor_digest bytea"));
    assert!(sql.contains("attempt_closures_reconciliation_proof_fk"));
    assert!(closure.contains("pg_advisory_xact_lock(7212400260826)"));
    assert!(closure.contains("lattice.managed-blocker.v1"));
    assert!(closure.contains("lattice.managed-no-provider-effect-proof.v1"));
    assert!(closure.contains("p_blocker_descriptor_digest"));
    assert!(closure.contains("p_reconciliation_proof_descriptor_digest"));
    assert!(closure.contains("PROVEN_NO_PROVIDER_CANDIDATE"));
    assert!(closure.contains("EXACT_EMPTY_THREAD_NO_TURN"));
    assert!(closure.contains("FOREMAN_RETAINED_CLOSURE_SUBSTITUTION"));
    assert!(closure.contains("FOREMAN_RETAINED_CLOSURE_PROOF_REJECTED"));
    assert!(closure.contains("FOREMAN_RETAINED_CLOSURE_PROVIDER_STILL_POSSIBLY_ACTIVE"));
    let no_candidate = closure
        .split("IF v_proof_payload->>'proof_kind' = 'PROVEN_NO_PROVIDER_CANDIDATE' THEN")
        .nth(1)
        .expect("no-provider-candidate proof branch")
        .split("ELSE")
        .next()
        .expect("exact-empty proof branch follows");
    assert!(
        no_candidate.contains("IF v_thread_claimed"),
        "bounded empty discovery cannot close an already claimed provider thread"
    );
    assert!(closure.contains("INSERT INTO foreman_execution.attempt_closures"));
    for retry_gate in [reserve, claim] {
        assert!(retry_gate.contains("foreman_execution.attempt_closures AS closure"));
        assert!(retry_gate.contains("closure.attempt_number = p_attempt_number - 1"));
    }
    let closure_gate = position(observation, "foreman_execution.attempt_closures AS closure");
    let observation_insert = position(
        observation,
        "INSERT INTO foreman_execution.worker_observations",
    );
    assert!(closure_gate < observation_insert);
    assert!(observation.contains("FOREMAN_OBSERVATION_AFTER_CLOSURE"));
    assert!(claim.contains("FOREMAN_RETRY_PREDECESSOR_NOT_TERMINAL"));
    assert!(sql.contains(
        "GRANT EXECUTE ON FUNCTION foreman_execution.close_retained_worker_without_provider_effect_v1("
    ));
}

#[test]
fn reviewer_restart_terminal_closure_is_segment_exact_not_global_generation_counted() {
    let sql = include_str!("../../../db/extensions/foreman-execution/v1.sql");
    let closure = function_body(
        sql,
        "record_attempt_closure_v1",
        "record_approval_evidence_v1",
    );
    assert!(closure.contains("'TURN_RECONCILED',"));
    assert!(closure.contains("'THREAD_RECONCILED'"));
    assert!(closure.contains("anchor.payload->>'event_type' = 'THREAD_STARTED'"));
    assert!(closure.contains("anchor.payload->>'event_type' = 'THREAD_RECONCILED'"));
    assert!(closure.contains("(admitted_turn.payload->>'sequence')::bigint + 1"));
    assert!(closure.contains("exact_terminal.payload->>'app_server_generation' ="));
    assert!(closure.contains("admitted_turn.payload->>'app_server_generation'"));
    assert!(!closure.contains("(terminal.payload->>'sequence')::bigint >"));
}

#[test]
fn restart_discovery_prioritizes_reconciliation_before_capacity_waiters() {
    let sql = std::str::from_utf8(verify_embedded_extension().expect("extension").bytes())
        .expect("utf8 SQL");
    let restart = function_body(sql, "list_restart_task_refs_v1", "list_active_task_refs_v1");
    let priority = position(restart, "CASE candidate.restart_kind");
    let closed = position(restart, "WHEN 'ATTEMPT_CLOSED_PENDING_RELEASE' THEN 0");
    let verification = position(restart, "WHEN 'VERIFICATION_RECONCILE_REQUIRED' THEN 1");
    let terminal = position(restart, "WHEN 'TERMINAL_PENDING_VERIFICATION' THEN 2");
    let active = position(restart, "WHEN 'ATTEMPT_RECONCILE_REQUIRED' THEN 3");
    let capacity = position(restart, "WHEN 'CAPACITY_WAIT' THEN 4");
    let promoted = position(restart, "WHEN 'PROMOTED_NO_ATTEMPT' THEN 5");
    let draft = position(restart, "WHEN 'DRAFT_PENDING_PROMOTION' THEN 6");
    let cursor = position(
        restart,
        "(candidate.restart_priority, candidate.task_ref) >",
    );
    let order = position(
        restart,
        "ORDER BY candidate.restart_priority, candidate.task_ref",
    );
    let limit = position(restart, "LIMIT p_limit");

    assert!(
        priority < closed
            && closed < verification
            && verification < terminal
            && terminal < active
            && active < capacity
            && capacity < promoted
            && promoted < draft
            && draft < cursor
            && cursor < order
            && order < limit,
        "active, terminal, reviewer, and closure reconciliation must run before capacity wait"
    );
    assert!(restart.contains("p_after_restart_priority smallint"));
    assert!(restart.contains("p_after_task_ref bytea"));
    assert!(restart.contains("candidate.restart_priority"));
    assert!(!restart.to_ascii_uppercase().contains("OFFSET"));
    assert!(restart.contains("control.task_submission_envelopes AS submission"));
    assert!(restart.contains("control.task_ingress_claims AS ingress"));
    assert!(restart.contains("control.task_ledger_streams AS intake_stream"));
    assert!(restart.contains("control.task_ledger_events AS intake_event"));
    assert!(restart.contains("control.task_ledger_commands AS intake_command"));
    assert!(restart.contains("control.project_registry_projects AS project"));
    assert!(restart.contains("submission.task_subject_kind = 'GENERAL_TASK_INTAKE'"));
    assert!(restart.contains("intake_event.action_id = 'GENERAL_TASK_INTAKE_V1'"));
    assert!(restart.contains("project.project_class = 'USER_PROJECT'"));
    assert!(restart.contains("project.authority_lifecycle = 'ACTIVE'"));
    assert!(restart.contains("project.pending_observation_digest IS NULL"));
    assert!(restart.contains("project.authority_snapshot_id = submission.project_snapshot_id"));
    assert!(restart.contains(
        "project.authority_receipt_digest = submission.project_authority_receipt_digest"
    ));
    assert!(restart.contains("promotion.task_ref = pg_catalog.decode(submission.task_ref, 'hex')"));
    assert!(!restart.contains("submission.objective"));
}

#[test]
fn restart_discovery_classifies_durable_evidence_before_writer_health() {
    let sql = include_str!("../../../db/extensions/foreman-execution/v1.sql");
    let restart = function_body(sql, "list_restart_task_refs_v1", "list_active_task_refs_v1");
    let attempt_case = restart
        .split("SELECT attempt.task_ref, attempt.attempt_number")
        .nth(1)
        .expect("latest-attempt restart classifier")
        .split("COALESCE((SELECT pg_catalog.max(observed.observed_at)::text")
        .next()
        .expect("latest-attempt classifier boundary");

    let closure = position(attempt_case, "THEN 'ATTEMPT_CLOSED_PENDING_RELEASE'::text");
    let verification = position(attempt_case, "THEN 'VERIFICATION_RECONCILE_REQUIRED'::text");
    let terminal = position(attempt_case, "THEN 'TERMINAL_PENDING_VERIFICATION'::text");
    let writer = position(attempt_case, "THEN 'WRITER_RECONCILIATION_REQUIRED'::text");

    assert!(
        closure < verification && verification < terminal && terminal < writer,
        "durable closure, verification, and terminal evidence must survive stale/expired/foreign Writer classification"
    );
    assert_eq!(
        attempt_case
            .matches("THEN 'ATTEMPT_CLOSED_PENDING_RELEASE'::text")
            .count(),
        1,
        "closure classification must not have a lease-absent special case"
    );
    assert_eq!(
        attempt_case
            .matches("THEN 'VERIFICATION_RECONCILE_REQUIRED'::text")
            .count(),
        1,
        "verification classification must not have a lease-absent special case"
    );
}

#[test]
fn restart_discovery_keeps_project_and_writer_drift_as_typed_candidates() {
    let sql = include_str!("../../../db/extensions/foreman-execution/v1.sql");
    let restart = function_body(sql, "list_restart_task_refs_v1", "list_active_task_refs_v1");
    let pending = restart
        .split("SELECT pending.task_ref, pending.attempt_number")
        .nth(1)
        .expect("pending-attempt restart classifier")
        .split("SELECT attempt.task_ref, attempt.attempt_number")
        .next()
        .expect("pending-attempt classifier boundary");
    let active = restart
        .split("SELECT attempt.task_ref, attempt.attempt_number")
        .nth(1)
        .expect("active-attempt restart classifier")
        .split("COALESCE((SELECT pg_catalog.max(observed.observed_at)::text")
        .next()
        .expect("active-attempt classifier boundary");

    assert!(restart.contains("LEFT JOIN ONLY control.project_registry_projects AS project"));
    assert!(restart.contains("DRAFT_PROJECT_RECONCILIATION_REQUIRED"));
    assert!(pending.contains("JOIN ONLY foreman_execution.task_promotions AS promotion"));
    assert!(
        pending.contains("LEFT JOIN ONLY control.project_registry_projects AS pending_project")
    );
    assert!(
        pending.contains("pending_project.authority_snapshot_id = promotion.project_snapshot_id")
    );
    assert!(pending.contains("pending_project.authority_receipt_digest ="));
    assert!(pending.contains("promotion.project_authority_receipt_digest"));
    assert!(pending.contains("'PROJECT_RECONCILIATION_REQUIRED'::text"));
    assert!(pending.contains("'CAPACITY_WAIT'::text"));
    assert!(restart.contains("LEFT JOIN ONLY control.project_registry_projects AS active_project"));
    assert!(
        restart.contains("active_project.authority_snapshot_id = promotion.project_snapshot_id")
    );
    assert!(restart.contains("active_project.authority_receipt_digest ="));
    let terminal = position(active, "THEN 'TERMINAL_PENDING_VERIFICATION'::text");
    let project = position(active, "THEN 'PROJECT_RECONCILIATION_REQUIRED'::text");
    let writer = position(active, "THEN 'WRITER_RECONCILIATION_REQUIRED'::text");
    assert!(
        terminal < project && project < writer,
        "durable terminal evidence must win, then Project currentness must fail closed before Writer/provider reconciliation"
    );
    assert!(restart.contains("WHEN 'PROJECT_RECONCILIATION_REQUIRED' THEN 3"));
    assert!(restart.contains("LEFT JOIN ONLY writer_lease.writer_lease_heads AS lease"));
    assert!(restart.contains("WRITER_RECONCILIATION_REQUIRED"));
    assert!(restart.contains("lease.current_status IS DISTINCT FROM 'ACTIVE'"));
    assert!(restart.contains("lease.current_expires_at::timestamp with time zone <="));
    assert!(restart.contains("FROM ONLY control.runtime_admission AS admission"));
    assert!(restart.contains("admission.admission_mode = 'ACTIVE'"));
    assert!(restart.contains("admission.daemon_instance_id ="));
    assert!(restart.contains("lease.current_daemon_instance_id"));
    assert!(restart.contains("admission.daemon_epoch = lease.current_daemon_epoch"));
}

#[test]
fn provider_dispatch_locks_every_mutable_authority_row_before_claim_insert() {
    let sql = std::str::from_utf8(verify_embedded_extension().expect("extension").bytes())
        .expect("utf8 SQL");
    let claim = function_body(
        sql,
        "claim_provider_dispatch_v1",
        "read_provider_dispatch_claim_v1",
    );

    let foreman_lock = position(claim, "FOR SHARE OF foreman_stream");
    let admission_lock = position(claim, "FOR SHARE OF admission");
    let writer_lock = position(claim, "FOR SHARE OF lease");
    let insert = position(
        claim,
        "INSERT INTO foreman_execution.provider_dispatch_claims",
    );
    assert!(foreman_lock < admission_lock && admission_lock < writer_lock && writer_lock < insert);
}

#[test]
fn promotion_persists_one_bounded_restart_source_and_fixed_reader() {
    let sql = std::str::from_utf8(verify_embedded_extension().expect("extension").bytes())
        .expect("utf8 SQL");
    let table = function_body(sql, "record_task_promotion_v1", "claim_worker_attempt_v1");
    let reader = function_body(
        sql,
        "read_task_promotion_source_v1",
        "read_worker_budget_v1",
    );

    assert!(sql.contains("base_ref varchar(255) NOT NULL"));
    assert!(sql.contains("base_commit char(40) NOT NULL"));
    assert!(sql.contains("octet_length(base_ref) BETWEEN 1 AND 255"));
    assert!(sql.contains("base_ref NOT LIKE 'refs/remotes/%'"));
    assert!(sql.contains("base_commit ~ '^[0-9a-f]{40}$'"));
    assert!(table.contains("p_base_ref text, p_base_commit text"));
    assert!(table.contains("v_existing.base_ref IS DISTINCT FROM p_base_ref"));
    assert!(table.contains("v_existing.base_commit IS DISTINCT FROM p_base_commit"));
    assert!(reader.contains("WHERE p.task_ref = p_task_ref"));
    assert!(!reader.to_ascii_lowercase().contains("execute "));
}

#[test]
fn claim_serializes_capacity_and_binds_exact_replay_to_retry_budget() {
    let sql = std::str::from_utf8(verify_embedded_extension().expect("extension").bytes())
        .expect("utf8 SQL");
    let claim = function_body(
        sql,
        "claim_worker_attempt_v1",
        "record_worker_observation_v1",
    );

    let lock = position(claim, "pg_catalog.pg_advisory_xact_lock(7212400260826)");
    let existing_lookup = position(
        claim,
        "SELECT * INTO v_existing FROM ONLY foreman_execution.worker_attempts",
    );
    let retry_budget_check = position(
        claim,
        "IF p_max_attempts NOT BETWEEN 1 AND 3 OR p_attempt_number > p_max_attempts THEN",
    );
    let immutable_budget_binding = position(
        claim,
        "p.repair_retry_limit + 1 = p_max_attempts) <> 1 THEN",
    );
    let exact_replay = position(claim, "RETURN QUERY SELECT 'EXACT_REPLAY'::text");
    let global_count = reverse_position(
        claim,
        "SELECT pg_catalog.count(*) INTO v_global FROM ONLY foreman_execution.worker_attempts AS a",
    );
    let task_count = reverse_position(
        claim,
        "SELECT pg_catalog.count(*) INTO v_task FROM ONLY foreman_execution.worker_attempts AS a",
    );
    let global_reject = position(claim, "IF v_global >= 4 THEN");
    let task_reject = position(claim, "IF v_task >= 1 THEN");
    let insert = position(claim, "INSERT INTO foreman_execution.worker_attempts (");

    assert!(
        lock < existing_lookup,
        "claim lookup must be inside the global transaction lock"
    );
    assert!(
        retry_budget_check < existing_lookup,
        "changed max-attempt input must fail before an existing attempt can exact-replay"
    );
    assert!(
        immutable_budget_binding < existing_lookup,
        "exact replay must bind max attempts to the immutable promotion budget"
    );
    assert!(
        existing_lookup < exact_replay,
        "exact replay must use the locked row"
    );
    assert!(
        claim.contains("p.repair_retry_limit + 1 = p_max_attempts"),
        "claim must bind the caller limit to the immutable promotion budget"
    );
    assert!(exact_replay < global_count && global_count < global_reject && global_reject < insert);
    assert!(exact_replay < task_count && task_count < task_reject && task_reject < insert);
    assert!(claim.contains(
        "RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'FOREMAN_GLOBAL_CAPACITY_EXHAUSTED'"
    ));
    assert!(claim.contains(
        "RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'FOREMAN_TASK_CAPACITY_EXHAUSTED'"
    ));
    assert!(
        claim.contains("o.observation_kind = 'TERMINAL_COMPLETED'")
            && claim.contains("foreman_execution.verification_records AS v"),
        "completed workers must keep their global slot through independent review"
    );
}

#[test]
fn worker_attempt_model_reason_is_closed_model_bound_and_replayable() {
    let sql = std::str::from_utf8(verify_embedded_extension().expect("extension").bytes())
        .expect("utf8 SQL");
    let reserve = function_body(sql, "reserve_worker_attempt_v1", "claim_worker_attempt_v1");
    let claim = function_body(
        sql,
        "claim_worker_attempt_v1",
        "record_worker_observation_v1",
    );
    let pending_reader = function_body(
        sql,
        "read_pending_worker_attempt_v1",
        "read_worker_budget_v1",
    );
    let attempt_reader = function_body(
        sql,
        "read_worker_attempt_rows_v1",
        "read_worker_observation_rows_v1",
    );

    assert_eq!(sql.matches("model_reason varchar(48) NOT NULL").count(), 2);
    for reason in [
        "BOUNDED_STATE_EVIDENCE_DOCUMENTATION",
        "ROUTINE_ENGINEERING",
        "P0",
        "ARCHITECTURE",
        "SECURITY",
        "HIGH_RISK",
        "TERRA_INSUFFICIENT",
    ] {
        assert!(sql.contains(reason), "missing model reason {reason}");
    }
    for function in [reserve, claim] {
        assert!(function.contains("p_model_reason text"));
        assert!(function.contains("model_reason IS DISTINCT FROM p_model_reason"));
        assert!(function.contains("FOREMAN_MODEL_REASON_NOT_ALLOWED"));
    }
    assert!(pending_reader.contains("pending.model_reason::text"));
    assert!(attempt_reader.contains("a.model_reason::text"));
}

#[test]
fn pending_claim_survives_capacity_wait_and_moves_to_active_atomically() {
    let sql = std::str::from_utf8(verify_embedded_extension().expect("extension").bytes())
        .expect("utf8 SQL");
    let reserve = function_body(sql, "reserve_worker_attempt_v1", "claim_worker_attempt_v1");
    let claim = function_body(
        sql,
        "claim_worker_attempt_v1",
        "record_worker_observation_v1",
    );

    assert!(sql.contains("CREATE TABLE foreman_execution.pending_worker_claims"));
    assert!(sql.contains("PRIMARY KEY (task_ref)"));
    assert!(sql.contains("max_attempts smallint NOT NULL"));
    assert!(reserve.contains("pg_catalog.pg_advisory_xact_lock(7212400260826)"));
    assert!(reserve.contains("'WORKER_ATTEMPT'"));
    assert!(reserve.contains("INSERT INTO foreman_execution.pending_worker_claims"));
    assert!(reserve.contains("assert_exact_child_event_v1"));

    let pending_lookup = position(
        claim,
        "SELECT * INTO v_pending FROM ONLY foreman_execution.pending_worker_claims",
    );
    let global_reject = position(claim, "IF v_global >= 4 THEN");
    let attempt_insert = position(claim, "INSERT INTO foreman_execution.worker_attempts");
    let pending_delete = position(
        claim,
        "DELETE FROM ONLY foreman_execution.pending_worker_claims",
    );
    assert!(pending_lookup < global_reject);
    assert!(global_reject < attempt_insert && attempt_insert < pending_delete);
    assert!(claim.contains("v_pending.max_attempts IS DISTINCT FROM p_max_attempts"));
    assert!(claim.contains("MESSAGE = 'FOREMAN_PENDING_CLAIM_REQUIRED'"));
    assert!(claim.contains("MESSAGE = 'FOREMAN_PENDING_CLAIM_SUBSTITUTION'"));
    assert!(claim.contains("MESSAGE = 'FOREMAN_PENDING_CLOSURE_REQUIRED'"));

    assert!(sql.contains("CREATE FUNCTION foreman_execution.list_restart_task_refs_v1"));
    for kind in [
        "PROMOTED_NO_ATTEMPT",
        "CAPACITY_WAIT",
        "ATTEMPT_RECONCILE_REQUIRED",
        "TERMINAL_PENDING_VERIFICATION",
        "VERIFICATION_RECONCILE_REQUIRED",
    ] {
        assert!(sql.contains(kind), "missing restart kind {kind}");
    }
    assert!(sql.contains("'PENDING_CLAIM'::text"));
    assert!(sql.contains("read_pending_worker_attempt_v1"));
}

#[test]
fn pending_no_effect_closure_is_one_atomic_materialize_finalize_close_transaction() {
    let sql = include_str!("../../../db/extensions/foreman-execution/v1.sql");
    let stage = function_body(
        sql,
        "stage_artifact_reference_v1",
        "finalize_staged_artifact_reference_v1",
    );
    let close = function_body(
        sql,
        "close_pending_worker_attempt_v1",
        "record_approval_evidence_v1",
    );
    assert!(stage.contains("FROM ONLY foreman_execution.pending_worker_claims AS pending"));
    assert!(stage.contains("p_payload_schema = 'lattice.managed-blocker.v1'"));
    assert!(stage.contains("p_producer_digest = pending.foreman_checkpoint_digest"));
    assert!(!sql.contains("staged_artifact_references_attempt_fk"));
    assert!(close.contains("pg_catalog.pg_advisory_xact_lock(7212400260826)"));
    let materialize = position(close, "INSERT INTO foreman_execution.worker_attempts");
    let inserted = &close[materialize..];
    let finalize = position(
        inserted,
        "foreman_execution.finalize_staged_artifact_reference_v1",
    ) + materialize;
    let closure = position(inserted, "foreman_execution.record_attempt_closure_v1") + materialize;
    let pending_delete = position(
        inserted,
        "DELETE FROM ONLY foreman_execution.pending_worker_claims",
    ) + materialize;
    assert!(materialize < finalize && finalize < pending_delete && pending_delete < closure);
    assert!(close.contains("FOREMAN_PENDING_CLOSURE_SUBSTITUTION"));
    assert!(close.contains("FOREMAN_PENDING_CLOSURE_REJECTED"));
    assert!(
        sql.contains(
            "GRANT EXECUTE ON FUNCTION foreman_execution.close_pending_worker_attempt_v1("
        )
    );
}

#[test]
fn artifact_quota_is_count_and_byte_bounded_per_attempt_and_task() {
    let sql = std::str::from_utf8(verify_embedded_extension().expect("extension").bytes())
        .expect("utf8 SQL");
    let stage = function_body(
        sql,
        "stage_artifact_reference_v1",
        "finalize_staged_artifact_reference_v1",
    );
    let exact_replay = position(stage, "RETURN 'EXACT_REPLAY'");
    let attempt_quota = position(stage, "FOREMAN_ARTIFACT_ATTEMPT_QUOTA_EXHAUSTED");
    let task_quota = position(stage, "FOREMAN_ARTIFACT_TASK_QUOTA_EXHAUSTED");
    let stage_insert = position(
        stage,
        "INSERT INTO foreman_execution.staged_artifact_references",
    );

    assert!(stage.contains("FROM ONLY foreman_execution.staged_artifact_references"));
    assert!(stage.contains("v_attempt_count >= 64"));
    assert!(
        stage.contains("v_attempt_bytes + pg_catalog.octet_length(p_evidence_bytes) > 8388608")
    );
    assert!(stage.contains("v_task_count >= 192"));
    assert!(stage.contains("v_task_bytes + pg_catalog.octet_length(p_evidence_bytes) > 25165824"));
    assert!(
        exact_replay < attempt_quota && attempt_quota < task_quota && task_quota < stage_insert
    );
}

#[test]
fn staged_artifact_outbox_is_single_task_exact_and_finalize_is_atomic() {
    let sql = std::str::from_utf8(verify_embedded_extension().expect("extension").bytes())
        .expect("utf8 SQL");
    assert!(sql.contains("CREATE TABLE foreman_execution.staged_artifact_references"));
    assert!(sql.contains("PRIMARY KEY (task_ref)"));

    let stage = function_body(
        sql,
        "stage_artifact_reference_v1",
        "finalize_staged_artifact_reference_v1",
    );
    assert!(stage.contains("FOREMAN_ARTIFACT_STAGE_SUBSTITUTION"));
    assert!(stage.contains("FOREMAN_ARTIFACT_STAGE_LEDGER_HEAD_MISMATCH"));
    assert!(stage.contains("p_before_sequence"));
    assert!(stage.contains("p_before_head_digest"));
    assert!(stage.contains("p_command_occurred_at"));
    let retained_replay = &stage[..position(stage, "RETURN 'EXACT_REPLAY'")];
    for exact_command_field in [
        "command.expected_sequence = p_before_sequence",
        "command.expected_last_event_digest = p_before_last_event_digest",
        "command.expected_resource_revision = p_before_resource_revision",
        "command.expected_resource_projection_digest = p_before_resource_projection_digest",
        "command.expected_head_digest = p_before_head_digest",
        "command.correlation_id = p_correlation_id",
        "command.occurred_at = p_command_occurred_at",
    ] {
        assert!(retained_replay.contains(exact_command_field));
    }

    let finalize = function_body(
        sql,
        "finalize_staged_artifact_reference_v1",
        "claim_provider_dispatch_v1",
    );
    let child_insert = position(finalize, "insert_child_event_v1");
    let artifact_insert = position(
        finalize,
        "INSERT INTO foreman_execution.artifact_references",
    );
    let stage_delete = position(
        finalize,
        "DELETE FROM ONLY foreman_execution.staged_artifact_references",
    );
    assert!(finalize.contains("FOREMAN_ARTIFACT_STAGE_REQUIRED"));
    assert!(finalize.contains("attempt_number = p_attempt_number"));
    assert!(finalize.contains("v_staged.attempt_number IS DISTINCT FROM p_attempt_number"));
    assert!(child_insert < artifact_insert && artifact_insert < stage_delete);

    let reader = function_body(
        sql,
        "read_staged_artifact_reference_v1",
        "read_managed_evidence_v1",
    );
    assert!(reader.contains("ORDER BY staged.task_ref"));

    let replay_start = position(
        sql,
        "CREATE FUNCTION foreman_execution.read_task_replay_v1(",
    );
    let replay_end = position(sql, "REVOKE ALL ON ALL TABLES IN SCHEMA foreman_execution");
    let replay = &sql[replay_start..replay_end];
    assert!(replay.contains("WHEN 'ARTIFACT_REFERENCE' THEN e.ledger_event_sequence::bigint"));
    assert!(!replay.contains("WHEN 'ARTIFACT_REFERENCE' THEN r.ledger_event_sequence::bigint"));
    assert!(replay.contains("ORDER BY replay.ledger_event_sequence"));
}

#[test]
fn retry_requires_monotonic_attempt_fence_and_terminal_predecessor() {
    let sql = std::str::from_utf8(verify_embedded_extension().expect("extension").bytes())
        .expect("utf8 SQL");
    let claim = function_body(
        sql,
        "claim_worker_attempt_v1",
        "record_worker_observation_v1",
    );

    assert!(claim.contains("p_attempt_number <> v_max + 1"));
    assert!(claim.contains("p_writer_fence <= v_previous.writer_fence"));
    assert!(claim.contains("p_foreman_generation < v_previous.foreman_generation"));
    assert!(claim.contains(
        "o.observation_kind IN ('PRESTART_TERMINAL_FAILED','TERMINAL_COMPLETED','TERMINAL_FAILED','TERMINAL_INTERRUPTED')"
    ));
    assert!(claim.contains("foreman_execution.attempt_closures AS closure"));
    assert!(claim.contains("closure.attempt_number = p_attempt_number - 1"));
    assert!(claim.contains("MESSAGE = 'FOREMAN_RETRY_PREDECESSOR_NOT_TERMINAL'"));
    assert!(claim.contains("MESSAGE = 'FOREMAN_ATTEMPT_SEQUENCE_MISMATCH'"));
    assert!(claim.contains("MESSAGE = 'FOREMAN_RETRY_BUDGET_EXHAUSTED'"));
}

fn function_body<'a>(sql: &'a str, name: &str, next_name: &str) -> &'a str {
    let start = position(sql, &format!("CREATE FUNCTION foreman_execution.{name}("));
    let end = position(
        &sql[start + 1..],
        &format!("CREATE FUNCTION foreman_execution.{next_name}("),
    ) + start
        + 1;
    &sql[start..end]
}

fn position(haystack: &str, needle: &str) -> usize {
    haystack
        .find(needle)
        .unwrap_or_else(|| panic!("missing SQL contract fragment: {needle}"))
}

fn reverse_position(haystack: &str, needle: &str) -> usize {
    haystack
        .rfind(needle)
        .unwrap_or_else(|| panic!("missing SQL contract fragment: {needle}"))
}
