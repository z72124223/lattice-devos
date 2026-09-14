use super::{
    Client, ExactCatalogProfile, ExtensionApplyOutcome, ExtensionBootstrapGlobalProfile,
    ExtensionDatabaseRole, ExtensionPreState, ExtensionSetupError, ExtensionSetupErrorKind,
    ExtensionTarget, FUNCTION_SIGNATURE_SQL, GenericClient, IsolationLevel,
    V3_EXPECTED_FUNCTION_SIGNATURE, acquire_writer_companion_advisory_locks, catalog_error,
    catalog_signature, classify_pre_state, harden_transaction, map_extension_sql_error,
    preflight_bootstrap, read_identity, transaction_error, verify_catalog_closure,
    verify_embedded_extension_manifest, verify_exact_catalog_profile,
    verify_global_default_acl_closure, verify_namespace_auxiliary_closure,
};

// Memory's v3 persistence profile and all historical receipt digests remain
// immutable. Only its seven fixed entry points bind to the exact current Store.
const STORE_V8_FUNCTION_SIGNATURE: &str =
    "f810d6558fb5a2f9810359156f137d1913fcf2630ac0008dbc69efe62ea54c79";

fn successor_sql() -> Result<String, ExtensionSetupError> {
    let manifest = verify_embedded_extension_manifest().map_err(|_| catalog_error())?;
    let source = std::str::from_utf8(manifest.bytes()).map_err(|_| catalog_error())?;
    let start = source
        .find("CREATE FUNCTION memory.")
        .ok_or_else(catalog_error)?;
    let end = source
        .find("REVOKE ALL ON ALL TABLES")
        .ok_or_else(catalog_error)?;
    let mut sql = source.get(start..end).ok_or_else(catalog_error)?.to_owned();
    for (old, new) in [
        (
            "CREATE FUNCTION memory.",
            "CREATE OR REPLACE FUNCTION memory.",
        ),
        (
            "i.global_schema_version = c.current_schema_version",
            "i.global_schema_version = 5",
        ),
        (
            "c.current_schema_version = 5",
            "c.current_schema_version = 8\n       AND c.min_reader = 8 AND c.max_reader = 8\n       AND c.min_writer = 8 AND c.max_writer = 8",
        ),
        (
            "pg_catalog.btrim(i.global_manifest_sha256) = pg_catalog.btrim(c.manifest_sha256)",
            "pg_catalog.btrim(i.global_manifest_sha256) = 'f92a51fa19c4fe0ffebfc40f20924bd1209bb2441b1bc69f787bc3c4a925425d'\n       AND pg_catalog.btrim(c.manifest_sha256) = '2b1fcbbc81261c28ab06ac3180f75c2ee458e57a4adc7e49bc399209f421de60'",
        ),
    ] {
        if sql.matches(old).count() != 7 {
            return Err(catalog_error());
        }
        sql = sql.replace(old, new);
    }
    Ok(sql)
}

pub(super) fn catalog_profile(
    client: &mut impl GenericClient,
) -> Result<ExactCatalogProfile, ExtensionSetupError> {
    match catalog_signature(client, FUNCTION_SIGNATURE_SQL)?.as_str() {
        V3_EXPECTED_FUNCTION_SIGNATURE => Ok(ExactCatalogProfile::V3),
        STORE_V8_FUNCTION_SIGNATURE => Ok(ExactCatalogProfile::V3StoreV8),
        _ => Err(catalog_error()),
    }
}

pub(super) const fn function_signature() -> &'static str {
    STORE_V8_FUNCTION_SIGNATURE
}

fn verify_profile(
    client: &mut impl GenericClient,
    target: &ExtensionTarget,
    profile: ExactCatalogProfile,
) -> Result<(), ExtensionSetupError> {
    let version = preflight_bootstrap(client, target, ExtensionBootstrapGlobalProfile::V8)?;
    let manifest = verify_embedded_extension_manifest().map_err(|_| catalog_error())?;
    if classify_pre_state(client)? != ExtensionPreState::ExactV3 {
        return Err(catalog_error());
    }
    verify_global_default_acl_closure(client)?;
    verify_exact_catalog_profile(client, profile)?;
    verify_catalog_closure(client)?;
    verify_namespace_auxiliary_closure(client, 16, "LATTICE_DEVOS_MEMORY_SCHEMA_V7")?;
    read_identity(
        client,
        target,
        ExtensionDatabaseRole::Migrator,
        version,
        &manifest,
    )?;
    Ok(())
}

/// Installs the Memory-owned compatibility successor on the exact Store-v8
/// database. Preserves Memory identities, ledger, data, receipt hashes and ACLs.
///
/// # Errors
/// Rejects unknown catalogs, Store manifests, database identities or roles.
pub fn apply_store_v8_compatibility(
    client: &mut Client,
    target: &ExtensionTarget,
) -> Result<ExtensionApplyOutcome, ExtensionSetupError> {
    let mut tx = client
        .build_transaction()
        .isolation_level(IsolationLevel::ReadCommitted)
        .start()
        .map_err(|_| transaction_error())?;
    harden_transaction(&mut tx)?;
    acquire_writer_companion_advisory_locks(&mut tx)?;
    let profile = catalog_profile(&mut tx)?;
    verify_profile(&mut tx, target, profile)?;
    let outcome = if profile == ExactCatalogProfile::V3 {
        tx.batch_execute(&successor_sql()?)
            .map_err(|e| map_extension_sql_error(&e))?;
        ExtensionApplyOutcome::Installed
    } else {
        ExtensionApplyOutcome::AlreadyCurrent
    };
    verify_profile(&mut tx, target, ExactCatalogProfile::V3StoreV8)?;
    tx.commit().map_err(|_| {
        ExtensionSetupError::new(
            ExtensionSetupErrorKind::CommitOutcomeUnknown,
            "MEMORY_STORE_V8_COMMIT_OUTCOME_UNKNOWN",
        )
    })?;
    Ok(outcome)
}

/// Verifies the installed successor without changing durable state.
///
/// # Errors
/// Rejects the old runtime functions or any identity, catalog or ACL drift.
pub fn verify_store_v8_compatibility(
    client: &mut Client,
    target: &ExtensionTarget,
) -> Result<(), ExtensionSetupError> {
    let mut tx = client
        .build_transaction()
        .isolation_level(IsolationLevel::RepeatableRead)
        .read_only(true)
        .start()
        .map_err(|_| transaction_error())?;
    harden_transaction(&mut tx)?;
    verify_profile(&mut tx, target, ExactCatalogProfile::V3StoreV8)?;
    tx.commit().map_err(|_| transaction_error())
}

#[cfg(test)]
mod tests {
    use super::super::BOOTSTRAP_V8_GLOBAL_MANIFEST_SHA256;
    use super::*;
    fn connect_fixture(port: u16, target: &ExtensionTarget, role: &str) -> Client {
        use postgres::{Config, NoTls};
        let mut client = Config::new()
            .host("127.0.0.1")
            .port(port)
            .dbname(target.database_name())
            .user(&format!("{role}_login"))
            .password(std::env::var("LATTICE_TASK019_PASSWORD").expect("fixture password required"))
            .connect(NoTls)
            .expect("fixture connection");
        client.batch_execute(&format!("SET ROLE {role}")).unwrap();
        client
    }

    fn snapshot(client: &mut Client) -> [Option<String>; 7] {
        [
            "extension_identity",
            "extension_ledger",
            "analyses",
            "records",
            "retrieval_audits",
            "receipts",
            "reflections",
        ]
        .map(|table| {
            client
                .query_one(
                    &format!(
                        "SELECT md5(string_agg(h, '' ORDER BY h)) FROM \
                    (SELECT md5(row_to_json(t)::text) h FROM memory.codebase_memory_{table} t) s"
                    ),
                    &[],
                )
                .unwrap()
                .get::<_, Option<String>>(0)
        })
    }

    #[test]
    fn successor_changes_only_fixed_runtime_bindings() {
        let sql = successor_sql().unwrap();
        assert_eq!(sql.matches("CREATE OR REPLACE FUNCTION memory.").count(), 7);
        assert_eq!(sql.matches("c.current_schema_version = 8").count(), 7);
        assert_eq!(sql.matches("i.global_schema_version = 5").count(), 7);
        assert!(!sql.contains("ALTER TABLE"));
        assert!(!sql.contains("GRANT "));
        assert!(!sql.contains("REVOKE "));
        assert!(!sql.contains("UPDATE memory.codebase_memory_extension"));
        assert_eq!(sql.matches("SECURITY DEFINER").count(), 7);
        assert_eq!(sql.matches("l.event_kind = 'UPGRADED'").count(), 7);
    }

    #[test]
    #[ignore = "requires an isolated Store-v8 database copy with historical Memory receipts"]
    fn live_successor_preserves_history_and_rejects_substitution() {
        use postgres::types::ToSql;
        let env = |key| std::env::var(key).expect("isolated fixture configuration required");
        assert_eq!(env("LATTICE_MEMORY_STORE_V8_FIXTURE"), "1");
        let port = env("LATTICE_TASK019_PORT").parse::<u16>().unwrap();
        let run_id = env("LATTICE_TASK019_RUN_ID");
        let target =
            ExtensionTarget::new(format!("lattice_task019_{}_base", &run_id[..8]), run_id).unwrap();
        let connect = |role: &str| connect_fixture(port, &target, role);
        let mut owner = connect("lattice_migrator");
        let before = snapshot(&mut owner);
        let a = owner.query_one("SELECT decode(btrim(i.database_identity_sha256),'hex'), \
            decode(btrim(i.global_manifest_sha256),'hex'), decode(btrim(i.extension_sql_sha256),'hex'), \
            decode(btrim(i.extension_manifest_sha256),'hex'), a.contract_version, a.request_id, \
            a.task_id, a.attempt_id, a.project_snapshot_id, a.subject_digest, a.project_id::text, \
            a.commit_id::text, a.query_digest, a.configuration_digest, a.retrieval_limit \
            FROM memory.codebase_memory_analyses a CROSS JOIN memory.codebase_memory_extension_identity i \
            ORDER BY a.recorded_at LIMIT 1", &[]).unwrap();
        let parameters: Vec<Box<dyn ToSql + Sync>> = (0..15)
            .map(|i| -> Box<dyn ToSql + Sync> {
                match i {
                    4 | 14 => Box::new(a.get::<_, i16>(i)),
                    5..=8 | 10..=11 => Box::new(a.get::<_, String>(i)),
                    _ => Box::new(a.get::<_, Vec<u8>>(i)),
                }
            })
            .collect();
        let arguments: Vec<&(dyn ToSql + Sync)> = parameters.iter().map(Box::as_ref).collect();
        let read = "SELECT receipt_digest FROM memory.codebase_memory_load_receipt_v3(\
            $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15)";
        let mut runtime = connect("lattice_runtime");
        let mut tx = runtime
            .build_transaction()
            .isolation_level(IsolationLevel::RepeatableRead)
            .read_only(true)
            .start()
            .unwrap();
        assert_eq!(
            tx.query(read, &arguments)
                .unwrap_err()
                .code()
                .unwrap()
                .code(),
            "LCM02"
        );
        tx.rollback().unwrap();
        assert_eq!(
            apply_store_v8_compatibility(&mut owner, &target).unwrap(),
            ExtensionApplyOutcome::Installed
        );
        assert_eq!(
            apply_store_v8_compatibility(&mut owner, &target).unwrap(),
            ExtensionApplyOutcome::AlreadyCurrent
        );
        assert_eq!(before, snapshot(&mut owner));
        drop(runtime);
        let mut runtime = connect("lattice_runtime");
        let mut tx = runtime
            .build_transaction()
            .isolation_level(IsolationLevel::RepeatableRead)
            .read_only(true)
            .start()
            .unwrap();
        assert_eq!(tx.query(read, &arguments).unwrap().len(), 1);
        let reflection_read = read.replace(
            "receipt_digest FROM memory.codebase_memory_load_receipt_v3",
            "reflection_receipt_digest FROM memory.codebase_memory_load_reflection_v3",
        );
        assert_eq!(tx.query(&reflection_read, &arguments).unwrap().len(), 1);
        tx.commit().unwrap();
        verify_store_v8_compatibility(&mut connect("lattice_migrator"), &target).unwrap();
        // A coherent Store-version substitution still fails the fixed runtime gate.
        owner
            .batch_execute(
                "UPDATE control.schema_compatibility SET manifest_sha256 = repeat('1',64)",
            )
            .unwrap();
        assert!(verify_store_v8_compatibility(&mut owner, &target).is_err());
        let mut tx = runtime
            .build_transaction()
            .isolation_level(IsolationLevel::RepeatableRead)
            .read_only(true)
            .start()
            .unwrap();
        assert_eq!(
            tx.query(read, &arguments)
                .unwrap_err()
                .code()
                .unwrap()
                .code(),
            "LCM02"
        );
        tx.rollback().unwrap();
        owner
            .execute(
                "UPDATE control.schema_compatibility SET manifest_sha256 = $1",
                &[&BOOTSTRAP_V8_GLOBAL_MANIFEST_SHA256],
            )
            .unwrap();
        assert_eq!(before, snapshot(&mut owner));
        verify_store_v8_compatibility(&mut owner, &target).unwrap();
    }
}
