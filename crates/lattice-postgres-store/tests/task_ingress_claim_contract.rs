const MIGRATION: &str = include_str!("../../../db/migrations/0008_task_submission_envelope.sql");

#[test]
fn schema_v7_owns_one_cross_profile_ingress_claim_keyspace() {
    for required in [
        "CREATE TABLE control.task_ingress_claims",
        "PRIMARY KEY (ingress_id, client_request_id)",
        "UNIQUE (ingress_id, client_request_id, stream_id)",
        "UNIQUE (stream_id)",
        "request_kind IN ('CONTROLLED_CODEX_CANARY', 'GENERAL_TASK')",
        "FOREIGN KEY (stream_id, event_sequence)\n        REFERENCES control.task_ledger_events",
        "FOREIGN KEY (stream_id, command_id)\n        REFERENCES control.task_ledger_commands",
        "FOREIGN KEY (ingress_id, client_request_id, stream_id)\n        REFERENCES control.task_ingress_claims",
    ] {
        assert!(MIGRATION.contains(required), "missing contract: {required}");
    }
    assert_eq!(
        MIGRATION
            .matches("CREATE TABLE control.task_ingress_claims")
            .count(),
        1
    );
    assert_eq!(
        MIGRATION
            .matches("e.stream_id AS ingress_request_digest,e.stream_id,e.sequence AS event_sequence")
            .count(),
        2,
        "singleton and ambiguous historical classifications must retain the original stream identity"
    );
    assert!(MIGRATION.contains("e.command_id::text ~ '^mcp-submit:"));
}

#[test]
fn historical_duplicate_claims_are_preserved_as_fail_closed_ambiguities() {
    for required in [
        "CREATE TABLE control.task_ingress_historical_ambiguities",
        "PRIMARY KEY (ingress_id, client_request_id, stream_id)",
        "FOREIGN KEY (stream_id, event_sequence)\n        REFERENCES control.task_ledger_events",
        "FOREIGN KEY (stream_id, command_id)\n        REFERENCES control.task_ledger_commands",
        "count(*) OVER (\n            PARTITION BY historical.ingress_id, historical.client_request_id\n        ) AS historical_identity_count",
        "WHERE classified.historical_identity_count = 1",
        "WHERE classified.historical_identity_count > 1",
        "LATTICE_TASK_INGRESS_HISTORICAL_AMBIGUOUS",
        "REVOKE ALL ON TABLE control.task_ingress_historical_ambiguities FROM lattice_runtime",
    ] {
        assert!(
            MIGRATION.contains(required),
            "missing duplicate-history contract: {required}"
        );
    }
    assert_eq!(
        MIGRATION
            .matches("LATTICE_TASK_INGRESS_HISTORICAL_AMBIGUOUS")
            .count(),
        3,
        "prepare, record, and read must all reject an ambiguous historical key"
    );
    for forbidden_winner in [
        "ON CONFLICT DO NOTHING",
        "DISTINCT ON (ingress_id, client_request_id)",
        "MIN(stream_id)",
        "MAX(stream_id)",
    ] {
        assert!(
            !MIGRATION.contains(forbidden_winner),
            "migration must preserve every historical identity, not select a winner: {forbidden_winner}"
        );
    }
}

#[test]
fn claim_functions_are_fixed_definer_owned_and_runtime_table_blind() {
    for signature in [
        "control.task_ingress_prepare_v1(text,text,text,bytea,bytea)",
        "control.task_ingress_record_v1(\n    text,text,text,text,bytea,bytea,text,bytea,text,bytea\n)",
        "control.task_ingress_read_by_request_v1(text,text)",
    ] {
        assert!(
            MIGRATION.contains(signature),
            "missing signature: {signature}"
        );
    }
    assert!(
        MIGRATION.contains("REVOKE ALL ON TABLE control.task_ingress_claims FROM lattice_runtime")
    );
    assert!(!MIGRATION.contains("GRANT SELECT ON TABLE control.task_ingress_claims"));
    assert!(!MIGRATION.contains("GRANT INSERT ON TABLE control.task_ingress_claims"));

    for body_marker in [
        "$lattice_task_ingress_prepare_v1$",
        "$lattice_task_ingress_record_v1$",
        "$lattice_task_ingress_read_by_request_v1$",
    ] {
        let occurrences = MIGRATION.matches(body_marker).count();
        assert_eq!(occurrences, 2, "unexpected body delimiter count");
    }
    assert!(MIGRATION.matches("SECURITY DEFINER").count() >= 3);
    assert!(MIGRATION.contains("SET search_path = pg_catalog\nSET row_security = on"));
    assert!(MIGRATION.contains("pg_catalog.pg_advisory_xact_lock(\n        pg_catalog.hashtextextended(p_ingress_id || ':' || p_client_request_id, 0)"));
    for strict_record_validation in [
        "p_ingress_id !~ '^[A-Za-z0-9][A-Za-z0-9._:-]{0,63}$'",
        "p_client_request_id !~ '^[A-Za-z0-9][A-Za-z0-9._:-]{0,63}$'",
        "pg_catalog.octet_length(p_ingress_request_digest) <> 32",
        "pg_catalog.octet_length(p_stream_id) <> 32",
        "pg_catalog.octet_length(p_event_digest) <> 32",
        "p_command_id !~ '^[A-Za-z0-9][A-Za-z0-9._:/-]{0,127}$'",
        "p_command_id IS DISTINCT FROM 'mcp-submit:' || p_client_request_id",
        "pg_catalog.octet_length(p_command_request_digest) <> 32",
    ] {
        assert!(
            MIGRATION.matches(strict_record_validation).count() >= 1,
            "record boundary lost validation: {strict_record_validation}"
        );
    }
}

#[test]
fn envelope_sql_bounds_match_unicode_public_contract() {
    assert!(MIGRATION.contains("pg_catalog.char_length(objective) <= 512"));
    assert!(MIGRATION.contains("pg_catalog.octet_length(objective) <= 2048"));
    assert!(MIGRATION.contains("pg_catalog.char_length(project_display_name) <= 64"));
    assert!(MIGRATION.contains("pg_catalog.octet_length(project_display_name) <= 256"));
}

#[test]
fn schema_v7_accepts_the_exact_project_registry_snapshot_maximum_only() {
    for required in [
        "ALTER TABLE control.physical_heads\n    DROP CONSTRAINT physical_heads_snapshot_id,\n    ALTER COLUMN project_snapshot_id TYPE varchar(159)",
        "ALTER TABLE control.terminal_transactions\n    DROP CONSTRAINT terminal_transactions_snapshot_id,\n    ALTER COLUMN project_snapshot_id TYPE varchar(159)",
        "ALTER TABLE control.task_ledger_streams\n    ALTER COLUMN project_snapshot_id TYPE varchar(159)",
        "project_snapshot_id varchar(159) NOT NULL",
    ] {
        assert!(
            MIGRATION.contains(required),
            "missing 159-byte storage width: {required}"
        );
    }
    assert_eq!(
        MIGRATION
            .matches("project_snapshot_id ~ '^[a-z0-9._:-]{1,159}$'")
            .count(),
        4,
        "all four durable snapshot columns must share the exact Registry maximum"
    );
    assert_eq!(
        MIGRATION
            .matches("p_project_snapshot_id !~ '^[a-z0-9._:-]{1,159}$'")
            .count(),
        4,
        "all current schema-v7 write/finalize paths must accept 159 and reject 160"
    );
    assert_eq!(
        MIGRATION
            .matches("p_expected_project_snapshot_id !~ '^[a-z0-9._:-]{1,159}$'")
            .count(),
        1,
        "the verified read path must use the same bound"
    );
    assert!(!MIGRATION.contains("project_snapshot_id varchar(128)"));
    assert!(!MIGRATION.contains("project_snapshot_id ~ '^[a-z0-9._:-]{1,128}$'"));
    assert!(!MIGRATION.contains("p_project_snapshot_id !~ '^[a-z0-9._:-]{1,128}$'"));
}

#[test]
fn claim_record_requires_same_transaction_task_created_linkage() {
    assert!(MIGRATION.contains("e.xmin=pg_catalog.pg_current_xact_id()::xid"));
    assert!(MIGRATION.contains("v_event.event_kind IS DISTINCT FROM 'TASK_CREATED'"));
    assert!(
        MIGRATION
            .contains("v_event.action_id IS DISTINCT FROM 'CONTROLLED_CODEX_CANARY_AUTONOMY_V1'")
    );
    assert!(MIGRATION.contains("v_event.action_id IS DISTINCT FROM 'GENERAL_TASK_INTAKE_V1'"));
    assert!(MIGRATION.contains("v_event.request_digest IS DISTINCT FROM p_command_request_digest"));
}

#[test]
fn neutral_claim_preflight_returns_raw_linkage_for_fail_closed_store_verification() {
    for read_contract in [
        "command_request_digest bytea,\n    event_kind text,event_action text,event_audit_outcome text",
        "LEFT JOIN ONLY control.task_ledger_events AS e",
        "e.event_kind::text,e.action_id::text,",
        "e.audit_outcome::text",
    ] {
        assert!(
            MIGRATION.contains(read_contract),
            "neutral claim read lost linkage field: {read_contract}"
        );
    }
    let store = include_str!("../src/task_ledger.rs");
    for store_contract in [
        "pub fn load_ingress_claim_by_request(",
        "verify_untrusted_task_ingress_claim_structure",
        "event_sequence != 1",
        "ingress_claim_command_matches(&claim, &command_id)",
        ".strip_prefix(\"mcp-submit:\")",
        "event_kind.as_deref() != Some(\"TASK_CREATED\")",
        "Some(\"CONTROLLED_CODEX_CANARY\" | \"CONTROLLED_CODEX_CANARY_AUTONOMY_V1\")",
        "event_action == Some(TaskCreatedProfile::GeneralTaskIntakeV1.action())",
        "event_outcome.as_deref() != Some(\"RECORDED\")",
    ] {
        assert!(
            store.contains(store_contract),
            "neutral Store preflight lost verification: {store_contract}"
        );
    }
}

#[test]
fn submission_record_requires_live_drift_free_registry_authority_in_same_transaction() {
    for currentness_guard in [
        "v_project.authority_runtime IS DISTINCT FROM 'LIVE'",
        "v_project.drift_canonical_root IS DISTINCT FROM false",
        "v_project.drift_repository IS DISTINCT FROM false",
        "v_project.drift_file IS DISTINCT FROM false",
        "v_project.drift_primary_ref_name IS DISTINCT FROM false",
        "v_project.drift_primary_ref_storage IS DISTINCT FROM false",
        "v_project.authority_snapshot_id IS DISTINCT FROM p_project_snapshot_id",
        "v_project.authority_receipt_digest IS DISTINCT FROM p_project_authority_receipt_digest",
        "v_project.authority_observation_digest IS DISTINCT FROM v_project.accepted_observation_digest",
        "v_project.pending_observation_digest IS NOT NULL",
    ] {
        assert!(
            MIGRATION.contains(currentness_guard),
            "missing Registry currentness guard: {currentness_guard}"
        );
    }
    assert!(MIGRATION.contains("FOR SHARE OF p"));
    assert!(MIGRATION.contains("ERRCODE = 'LPG01'"));
    assert!(MIGRATION.contains("ERRCODE = 'LPG02'"));
}

#[test]
fn submission_table_rejects_the_full_durable_secret_assignment_and_aws_key_set() {
    let table_start = MIGRATION
        .find("CREATE TABLE control.task_submission_envelopes")
        .expect("submission table start");
    let table_end = MIGRATION[table_start..]
        .find("REVOKE ALL ON TABLE control.task_submission_envelopes")
        .map(|offset| table_start + offset)
        .expect("submission table end");
    let table = &MIGRATION[table_start..table_end];
    for secret_shape in [
        "passphrase|passwd|pwd|token|access_token|access-token|refresh_token|refresh-token",
        "id_token|id-token|session_token|session-token|api_key|api-key|apikey",
        "client_secret|client-secret|secret|credential|credentials|cookie|set-cookie|authorization",
        "(AKIA|ASIA)[A-Z0-9]{16}",
    ] {
        assert!(
            table.contains(secret_shape),
            "missing durable secret rejection: {secret_shape}"
        );
    }
    assert_eq!(
        table.matches("(AKIA|ASIA)[A-Z0-9]{16}").count(),
        5,
        "all envelope text identifiers and human fields must share the AWS-key defense"
    );
    assert_eq!(
        table.matches("[[:space:]]*[\"'']?[[:space:]]*[:=]").count(),
        5,
        "sensitive-key assignments must be rejected in every persisted envelope text field"
    );
    assert_eq!(
        table
            .matches("U&'\\0085\\00A0\\1680\\2000\\2001\\2002\\2003\\2004\\2005\\2006\\2007\\2008\\2009\\200A\\2028\\2029\\202F\\205F\\3000'")
            .count(),
        9,
        "all persisted text secret checks and both human-field trim checks must normalize every Rust whitespace scalar"
    );
    for field in ["project_id", "project_snapshot_id"] {
        assert!(
            table.contains(&format!("({field} COLLATE pg_catalog.\"C\") !~*")),
            "missing secret-free identifier check for {field}"
        );
    }
}

#[test]
fn unicode_outer_whitespace_is_rejected_by_table_and_record_boundaries() {
    for required in [
        "pg_catalog.translate(objective, U&'\\0085\\00A0\\1680",
        "= pg_catalog.btrim(pg_catalog.translate(objective, U&'\\0085\\00A0\\1680",
        "pg_catalog.translate(project_display_name, U&'\\0085\\00A0\\1680",
        "= pg_catalog.btrim(pg_catalog.translate(project_display_name, U&'\\0085\\00A0\\1680",
        "pg_catalog.translate(p_objective, U&'\\0085\\00A0\\1680",
        "IS DISTINCT FROM pg_catalog.btrim(pg_catalog.translate(p_objective, U&'\\0085\\00A0\\1680",
        "pg_catalog.translate(p_project_display_name, U&'\\0085\\00A0\\1680",
        "IS DISTINCT FROM pg_catalog.btrim(pg_catalog.translate(p_project_display_name, U&'\\0085\\00A0\\1680",
    ] {
        assert!(
            MIGRATION.contains(required),
            "missing Unicode trim guard: {required}"
        );
    }
}

#[test]
fn non_nfc_human_fields_are_rejected_by_table_and_record_boundaries() {
    for required in [
        "objective IS NFC NORMALIZED",
        "project_display_name IS NFC NORMALIZED",
        "p_objective IS NOT NFC NORMALIZED",
        "p_project_display_name IS NOT NFC NORMALIZED",
    ] {
        assert!(
            MIGRATION.contains(required),
            "missing NFC guard: {required}"
        );
    }
}

#[test]
fn control_character_rejection_is_locale_independent_and_covers_c1() {
    let explicit_control_range = "COLLATE pg_catalog.\"C\") !~ U&'[\\0001-\\001F\\007F-\\009F]'";
    assert_eq!(
        MIGRATION.matches(explicit_control_range).count(),
        2,
        "both durable human fields must reject C0, DEL, and C1 without locale classes"
    );
    let record_control_range = "COLLATE pg_catalog.\"C\") ~ U&'[\\0001-\\001F\\007F-\\009F]'";
    assert_eq!(
        MIGRATION.matches(record_control_range).count(),
        2,
        "the record definer must reject C0, DEL, and C1 before persistence"
    );
}

#[test]
fn submission_record_rejects_secrets_in_every_formal_text_field_before_lookup() {
    let start = MIGRATION
        .find("AS $lattice_task_submission_record_v1$")
        .expect("submission record body start");
    let end = MIGRATION[start + 1..]
        .find("$lattice_task_submission_record_v1$;")
        .map(|offset| start + 1 + offset)
        .expect("submission record body end");
    let body = &MIGRATION[start..end];
    for field in [
        "p_client_request_id",
        "p_objective",
        "p_project_display_name",
        "p_project_id",
        "p_project_snapshot_id",
    ] {
        for marker in [
            format!("({field} COLLATE pg_catalog.\"C\") ~* '(bearer |"),
            format!("({field} COLLATE pg_catalog.\"C\") ~* '-----begin '"),
            format!("({field} COLLATE pg_catalog.\"C\") ~* 'private key-----'"),
            format!("({field} COLLATE pg_catalog.\"C\") ~ '(^|[^A-Za-z0-9])(AKIA|ASIA)"),
        ] {
            assert!(
                body.contains(&marker),
                "missing {field} record guard: {marker}"
            );
        }
    }
    assert!(
        body.find("(p_project_id COLLATE pg_catalog.\"C\") ~*")
            < body.find("SELECT p.* INTO v_project"),
        "identifier secret guards must run before formal Registry resolution"
    );
}

#[test]
fn client_request_id_secret_contract_is_closed_at_every_durable_boundary() {
    let ingress_start = MIGRATION
        .find("CREATE TABLE control.task_ingress_claims")
        .expect("ingress claim migration tail");
    let ingress = &MIGRATION[ingress_start..];
    for required in [
        "LATTICE_TASK_INGRESS_HISTORICAL_CLIENT_REQUEST_ID_REJECTED",
        "$lattice_task_ingress_historical_client_request_guard_v1$",
        "(historical.client_request_id COLLATE pg_catalog.\"C\") ~*",
        "(pg_catalog.substring(e.command_id::text, 12) COLLATE pg_catalog.\"C\") !~*",
        "(client_request_id COLLATE pg_catalog.\"C\") !~* '(bearer |(^|[^A-Za-z0-9])sk-|",
        "(p_client_request_id COLLATE pg_catalog.\"C\") ~* '(bearer |(^|[^A-Za-z0-9])sk-|",
        "(p_client_request_id COLLATE pg_catalog.\"C\") !~* '(bearer |(^|[^A-Za-z0-9])sk-|",
        "~* '-----begin '",
        "~* 'private key-----'",
        "(AKIA|ASIA)[A-Z0-9]{16}",
    ] {
        assert!(
            MIGRATION.contains(required),
            "missing client-id defense: {required}"
        );
    }
    assert_eq!(
        MIGRATION
            .matches("OR (p_client_request_id COLLATE pg_catalog.\"C\") ~* '(bearer |(^|[^A-Za-z0-9])sk-|")
            .count(),
        4,
        "prepare/record functions for claim and submission must all reject secret-shaped ids"
    );
    assert!(
        !ingress.contains("[^[:alnum:]]") && !ingress.contains("[^[:alnum:]_-]"),
        "schema-v7 secret boundaries must not depend on locale-sensitive POSIX alnum"
    );
    assert!(
        !ingress.contains("-----begin .*private key-----"),
        "private-key detection must require both markers without imposing their order"
    );
    assert!(
        ingress.matches("COLLATE pg_catalog.\"C\"").count() >= 40,
        "every locale-sensitive schema-v7 secret expression must pin the C collation"
    );

    let store = include_str!("../src/task_ledger.rs");
    assert!(store.contains("valid_task_ingress_client_request_id(client_request_id)"));
    assert_eq!(
        store
            .matches("valid_task_ingress_client_request_id(client_request_id)")
            .count(),
        2,
        "neutral-claim and submission lookups must use the shared client-id validator"
    );
}

#[test]
fn submission_reads_return_the_envelope_before_replay_verifies_linkage() {
    for delimiter in [
        "$lattice_task_submission_read_by_task_ref_v1$",
        "$lattice_task_submission_read_by_request_v1$",
    ] {
        let start = MIGRATION.find(delimiter).expect("read function body start") + delimiter.len();
        let end = MIGRATION[start..]
            .find(delimiter)
            .map(|offset| start + offset)
            .expect("read function body end");
        let body = &MIGRATION[start..end];
        assert!(body.contains("FROM ONLY control.task_submission_envelopes AS s"));
        assert!(!body.contains("JOIN control.task_ledger_events"));
        assert!(!body.contains("JOIN ONLY control.task_ledger_events"));
    }
}
