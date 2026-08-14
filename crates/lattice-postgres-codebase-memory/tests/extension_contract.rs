use lattice_contracts::{CodebaseMemoryPersistenceIdentity, ContentDigest};
use lattice_postgres_codebase_memory::{
    CODEBASE_MEMORY_EXTENSION_ID, CODEBASE_MEMORY_EXTENSION_PATH,
    CODEBASE_MEMORY_EXTENSION_SCHEMA_VERSION, CODEBASE_MEMORY_V1_EXTENSION_PATH,
    CODEBASE_MEMORY_V2_EXTENSION_PATH, verify_embedded_extension_manifest,
    verify_embedded_v1_extension_manifest, verify_embedded_v2_extension_manifest,
};

fn digest(byte: char) -> ContentDigest {
    ContentDigest::from_sha256(byte.to_string().repeat(64)).expect("digest")
}

#[test]
fn exact_extension_manifest_and_typed_identity_are_required() {
    let historical_v1 = verify_embedded_v1_extension_manifest().expect("frozen v1 extension");
    assert_eq!(historical_v1.path(), CODEBASE_MEMORY_V1_EXTENSION_PATH);
    assert_eq!(historical_v1.byte_length(), 42_411);
    assert_eq!(
        historical_v1.sql_sha256().as_str(),
        "555eabce843417bcbcd111a3cec42d05f3e2aaff802aa168b54be2fbfb300a3f"
    );
    let historical_v2 = verify_embedded_v2_extension_manifest().expect("frozen v2 extension");
    assert_eq!(historical_v2.path(), CODEBASE_MEMORY_V2_EXTENSION_PATH);
    assert_eq!(historical_v2.byte_length(), 76_866);
    assert_eq!(
        historical_v2.sql_sha256().as_str(),
        "9db54342b88f554ca76054c7a33ae72f04b412d2dfe21fae6eb4d8faf3e854e2"
    );

    let manifest = verify_embedded_extension_manifest().expect("exact embedded extension");
    assert_eq!(manifest.extension_id(), CODEBASE_MEMORY_EXTENSION_ID);
    assert_eq!(manifest.path(), CODEBASE_MEMORY_EXTENSION_PATH);
    assert_eq!(
        manifest.schema_version(),
        CODEBASE_MEMORY_EXTENSION_SCHEMA_VERSION
    );
    assert_eq!(manifest.byte_length(), 87_545);
    assert_eq!(
        manifest.sql_sha256().as_str(),
        "7388f6bfe4c2d30a20306e4f9ebdff5862125bcab58f769ba286af542cb051c3"
    );
    assert_eq!(
        manifest.manifest_sha256().as_str(),
        "d4cc712d262ae1f7c96bd65526eab611c90e193363afd865af2126307b2903f0"
    );

    let identity = CodebaseMemoryPersistenceIdentity::v3(
        digest('1'),
        digest('2'),
        manifest.sql_sha256().clone(),
        manifest.manifest_sha256().clone(),
    )
    .expect("typed database and extension identity");
    assert_eq!(identity.global_schema_version(), 5);
    assert_eq!(identity.extension_id(), CODEBASE_MEMORY_EXTENSION_ID);
    assert_eq!(
        identity.extension_schema_version(),
        CODEBASE_MEMORY_EXTENSION_SCHEMA_VERSION
    );
}

#[test]
fn v3_sql_is_append_only_and_retains_complete_row_profile_provenance() {
    let v2 = verify_embedded_v2_extension_manifest().expect("exact v2 extension");
    let v2_sql = std::str::from_utf8(v2.bytes()).expect("utf8 v2 SQL");
    let manifest = verify_embedded_extension_manifest().expect("exact v3 extension");
    let sql = std::str::from_utf8(manifest.bytes()).expect("utf8 SQL");
    assert!(!sql.contains("CREATE OR REPLACE"));
    assert!(!sql.contains("DROP TABLE"));
    assert!(
        !sql.contains("DROP CONSTRAINT codebase_memory_extension_ledger_identity_fk"),
        "v3 must preserve the ledger-to-identity foreign key and its internal triggers"
    );
    assert!(v2_sql.contains(
        "CONSTRAINT codebase_memory_extension_ledger_identity_fk FOREIGN KEY (singleton)"
    ));
    assert!(sql.contains("DROP CONSTRAINT codebase_memory_extension_ledger_singleton_key"));
    for table in [
        "codebase_memory_analyses",
        "codebase_memory_retrieval_audits",
        "codebase_memory_receipts",
        "codebase_memory_reflections",
    ] {
        assert!(sql.contains(&format!("ALTER TABLE memory.{table}")));
    }
    for field in [
        "persistence_database_identity_sha256",
        "persistence_global_schema_version",
        "persistence_global_manifest_sha256",
        "persistence_extension_id",
        "persistence_extension_schema_version",
        "persistence_extension_sql_sha256",
        "persistence_extension_manifest_sha256",
    ] {
        assert!(sql.contains(field), "missing profile field {field}");
    }
    for function in [
        "codebase_memory_persist_analysis_v3",
        "codebase_memory_persist_retrieval_v3",
        "codebase_memory_load_receipt_v3",
        "codebase_memory_persist_reflection_v3",
        "codebase_memory_load_reflection_v3",
        "openclaw_gateway_reconcile_and_claim_v3",
        "openclaw_gateway_finalize_terminal_v3",
    ] {
        assert!(sql.contains(&format!("CREATE FUNCTION memory.{function}")));
    }
    assert!(sql.contains("ledger_ordinal = 2"));
    assert!(sql.contains("event_kind = 'UPGRADED'"));
    assert!(!sql.contains("i.global_schema_version = 3"));
    assert!(sql.contains("i.global_schema_version = c.current_schema_version"));
    for comment in [
        "LATTICE_CODEBASE_MEMORY_ANALYSES_V3",
        "LATTICE_CODEBASE_MEMORY_EXTENSION_IDENTITY_V3",
        "LATTICE_CODEBASE_MEMORY_EXTENSION_LEDGER_V3",
        "LATTICE_CODEBASE_MEMORY_RECEIPTS_V3",
        "LATTICE_CODEBASE_MEMORY_RECORDS_V3",
        "LATTICE_CODEBASE_MEMORY_REFLECTIONS_V3",
        "LATTICE_CODEBASE_MEMORY_RETRIEVAL_AUDITS_V3",
        "LATTICE_OPENCLAW_GATEWAY_COMMANDS_V3",
    ] {
        assert!(sql.contains(comment));
    }
}

#[test]
fn live_durability_gate_uses_v3_and_denies_historical_v2_execute() {
    let live = include_str!("postgres_live.rs");
    let start = live
        .find("fn prove_reflection_durability_boundary")
        .expect("reflection durability boundary");
    let end = live[start..]
        .find("\n#[allow(clippy::too_many_lines)]")
        .map(|offset| start + offset)
        .expect("reflection durability boundary end");
    let durability = &live[start..end];

    assert!(durability.contains("codebase_memory_persist_reflection_v3"));
    assert!(durability.contains("codebase_memory_persist_reflection_v2"));
    assert!(durability.contains("SqlState::INSUFFICIENT_PRIVILEGE"));
    assert!(durability.contains("Some(\"LCM01\")"));
}

#[test]
fn historical_profile_staging_never_downgrades_global_compatibility() {
    let live = include_str!("postgres_live.rs");
    let start = live
        .find("fn stage_exact_v2_upgrade_source")
        .expect("historical v2 upgrade fixture");
    let end = live[start..]
        .find("\n#[allow(clippy::too_many_lines)]\nfn historical_receipt_fingerprint")
        .map(|offset| start + offset)
        .expect("historical v2 upgrade fixture end");
    let staging = &live[start..end];

    assert!(!staging.contains("UPDATE ONLY control.schema_compatibility"));
    assert!(!staging.contains("SET current_schema_version = 3"));
    assert!(staging.contains("COMMENT ON SCHEMA memory IS 'LATTICE_DEVOS_MEMORY_SCHEMA_V5'"));
    assert!(staging.contains("global_schema_version, global_manifest_sha256"));
    assert!(staging.contains("&HISTORICAL_GLOBAL_MANIFEST_SHA256"));
}
