use std::env;

use lattice_cjson::{CanonicalValue, HashDomain, canonical_sha256};
use lattice_codebase_memory::{digest_query_text, normalize_analysis, plan_retrieval};
use lattice_contracts::{
    AttemptId, CONTRACT_VERSION, CodeSnapshotEvidence, CodebaseMemoryPersistenceIdentity,
    ContentDigest, GatewayActorId, GatewayCommandId, GatewayCorrelationId, GatewayDenialCode,
    GatewayProjectStatusTarget, GatewayReply, GatewayReplyBody, GatewayRequest, GatewayRequestBody,
    GatewayStatusTarget, GitObjectId, GraphConfidence, GraphMemoryPersistenceEvidence,
    GraphMemoryReceipt, GraphMemoryRunRequest, GraphSourceProvenance, GraphifyIdentity,
    GraphifyRawEvidence, GraphifyRawNode, HERMES_REFLECTION_SCHEMA_VERSION,
    HermesReflectionCandidate, HermesReflectionContent, HermesReflectionFinding,
    HermesReflectionReceipt, HermesReflectionStatus, Invocation, MemoryQuery,
    MemoryRetrievalEvidence, MemoryRetrievalPlan, NormalizedGraphAnalysis, ProjectId,
    ProjectSnapshotId, RankedMemoryRecord, RequestId, TaskId, TrackedSource,
};
use lattice_gateway_ipc::{build_reply, build_request};
use lattice_ports::{
    CodebaseMemoryPort, HermesReflectionMemoryPort, OpenClawCommandScope,
    OpenClawIdempotencyDecision, OpenClawIdempotencyStore, OpenClawTerminalCommandRecord,
};
use lattice_postgres_codebase_memory::{
    ExtensionApplyOutcome, ExtensionDatabaseRole, ExtensionSetupErrorKind, ExtensionTarget,
    PostgresCodebaseMemory, apply_extension, verify_embedded_extension_manifest,
    verify_embedded_v2_extension_manifest, verify_extension,
};
use postgres::config::SslMode;
use postgres::error::SqlState;
use postgres::{Client, Config, NoTls};

const APPLICATION_NAME: &str = "lattice-devos-task019";
const HISTORICAL_GLOBAL_MANIFEST_SHA256: &str =
    "09c431df18ad71a4f44239a5d2ddf6b1774b8ffec06c7f9223f0e41757f3d407";
const CURRENT_GLOBAL_MANIFEST_SHA256: &str =
    "f92a51fa19c4fe0ffebfc40f20924bd1209bb2441b1bc69f787bc3c4a925425d";

#[derive(Clone)]
struct LiveConfig {
    host: String,
    port: u16,
    password: String,
    run_id: String,
    phase: String,
}

impl LiveConfig {
    fn from_environment() -> Option<Self> {
        if env::var("LATTICE_TASK019_LIVE").ok().as_deref() != Some("1") {
            return None;
        }
        let host = required("LATTICE_TASK019_HOST");
        let port = required("LATTICE_TASK019_PORT")
            .parse::<u16>()
            .expect("MEMORY_EXTENSION_LIVE_PORT_INVALID");
        let password = required("LATTICE_TASK019_PASSWORD");
        let run_id = required("LATTICE_TASK019_RUN_ID");
        let phase = required("LATTICE_TASK019_PHASE");
        assert_eq!(host, "127.0.0.1", "MEMORY_EXTENSION_LIVE_HOST_INVALID");
        assert!(
            port != 0 && port != 5432,
            "MEMORY_EXTENSION_LIVE_PORT_INVALID"
        );
        assert_eq!(run_id.len(), 32, "MEMORY_EXTENSION_LIVE_RUN_ID_INVALID");
        assert!(matches!(phase.as_str(), "initial" | "restart"));
        Some(Self {
            host,
            port,
            password,
            run_id,
            phase,
        })
    }

    fn target(&self) -> ExtensionTarget {
        ExtensionTarget::new(
            format!("lattice_task019_{}_base", &self.run_id[..8]),
            self.run_id.clone(),
        )
        .expect("MEMORY_EXTENSION_TARGET_INVALID")
    }

    fn role_client(&self, role: ExtensionDatabaseRole) -> Client {
        let mut config = Config::new();
        config
            .host(&self.host)
            .port(self.port)
            .user(role.login_role())
            .password(&self.password)
            .dbname(self.target().database_name())
            .application_name(APPLICATION_NAME)
            .ssl_mode(SslMode::Disable);
        let mut client = config
            .connect(NoTls)
            .expect("MEMORY_EXTENSION_LIVE_CONNECT_FAILED");
        client
            .batch_execute(&format!("SET ROLE {}", role.as_str()))
            .expect("MEMORY_EXTENSION_SET_ROLE_FAILED");
        client
    }

    fn admin_client(&self) -> Client {
        let mut config = Config::new();
        config
            .host(&self.host)
            .port(self.port)
            .user("task019_harness")
            .password(&self.password)
            .dbname(self.target().database_name())
            .application_name("lattice-devos-memory-live-admin")
            .ssl_mode(SslMode::Disable);
        config
            .connect(NoTls)
            .expect("MEMORY_EXTENSION_ADMIN_CONNECT_FAILED")
    }
}

#[test]
fn exact_memory_extension_install_and_restart_profile() {
    let Some(config) = LiveConfig::from_environment() else {
        return;
    };
    let target = config.target();
    if config.phase == "initial" {
        prove_partial_collision_and_transaction_rollback(&config, &target);
        let mut migrator = config.role_client(ExtensionDatabaseRole::Migrator);
        assert_eq!(
            apply_extension(&mut migrator, &target)
                .unwrap_or_else(|error| panic!("{}", error.code())),
            ExtensionApplyOutcome::Installed
        );
        let fresh_evidence =
            verify_extension(&mut migrator, &target, ExtensionDatabaseRole::Migrator)
                .unwrap_or_else(|error| panic!("{}", error.code()));
        assert_eq!(
            fresh_evidence.database_uuid(),
            target.expected_database_uuid()
        );
        assert_eq!(
            fresh_evidence.identity().extension_id(),
            "lattice-codebase-memory"
        );
        prove_foreign_acl_rejected(&config, &target);
        let before_no_op = extension_install_fingerprint(&mut migrator);
        assert_eq!(
            apply_extension(&mut migrator, &target)
                .unwrap_or_else(|error| panic!("{}", error.code())),
            ExtensionApplyOutcome::AlreadyCurrent
        );
        assert_eq!(extension_install_fingerprint(&mut migrator), before_no_op);
        drop(migrator);

        stage_exact_v2_upgrade_source(&config, &target);
        let mut migrator = config.role_client(ExtensionDatabaseRole::Migrator);
        let historical_bytes_before_upgrade = historical_receipt_fingerprint(&mut migrator);
        assert_eq!(
            apply_extension(&mut migrator, &target)
                .unwrap_or_else(|error| panic!("{}", error.code())),
            ExtensionApplyOutcome::Installed
        );
        let evidence = verify_extension(&mut migrator, &target, ExtensionDatabaseRole::Migrator)
            .unwrap_or_else(|error| panic!("{}", error.code()));
        assert_eq!(
            evidence, fresh_evidence,
            "MEMORY_EXTENSION_FRESH_UPGRADE_PROFILE_DIVERGED"
        );
        assert_eq!(
            historical_receipt_fingerprint(&mut migrator),
            historical_bytes_before_upgrade,
            "MEMORY_HISTORICAL_RECEIPT_BYTES_CHANGED_DURING_UPGRADE"
        );
        drop(migrator);
        prove_foreign_acl_rejected(&config, &target);

        let mut runtime = config.role_client(ExtensionDatabaseRole::Runtime);
        assert!(
            verify_extension(&mut runtime, &target, ExtensionDatabaseRole::Runtime).is_err(),
            "MEMORY_EXTENSION_RUNTIME_ADMIN_VERIFY_ALLOWED"
        );
        let denied = runtime
            .query_one("SELECT count(*) FROM memory.codebase_memory_analyses", &[])
            .expect_err("MEMORY_EXTENSION_RUNTIME_TABLE_SELECT_ALLOWED");
        assert_eq!(
            denied.code(),
            Some(&SqlState::INSUFFICIENT_PRIVILEGE),
            "MEMORY_EXTENSION_RUNTIME_TABLE_DENIAL_WRONG"
        );
        drop(runtime);
        prove_reflection_durability_boundary(&config);
        exercise_runtime_memory(&config, &target, false);
        prove_historical_v2_replay(&config, &target);
        assert_current_v3_profiles(&config, &target);
        prove_profile_corruption_denial(&config, &target);
        exercise_openclaw_idempotency(&config, &target, false);
        println!(
            "MEMORY_EXTENSION_INITIAL_OK database_uuid={} extension_manifest={}",
            evidence.database_uuid(),
            evidence.identity().extension_manifest_digest().as_str()
        );
    } else {
        let mut migrator = config.role_client(ExtensionDatabaseRole::Migrator);
        let evidence = verify_extension(&mut migrator, &target, ExtensionDatabaseRole::Migrator)
            .unwrap_or_else(|error| panic!("{}", error.code()));
        assert_eq!(evidence.database_uuid(), target.expected_database_uuid());
        assert_eq!(
            apply_extension(&mut migrator, &target)
                .unwrap_or_else(|error| panic!("{}", error.code())),
            ExtensionApplyOutcome::AlreadyCurrent
        );
        drop(migrator);
        exercise_runtime_memory(&config, &target, true);
        prove_historical_v2_replay(&config, &target);
        assert_current_v3_profiles(&config, &target);
        exercise_openclaw_idempotency(&config, &target, true);
        println!("MEMORY_EXTENSION_RESTART_PROFILE_OK");
    }
}

struct HistoricalMemoryFixture {
    analysis: NormalizedGraphAnalysis,
    receipt: GraphMemoryReceipt,
    reflection: HermesReflectionReceipt,
}

#[test]
fn historical_v2_fixture_receipts_have_frozen_digests() {
    let target = ExtensionTarget::new(
        "lattice_task019_task075_fixture",
        "0123456789abcdef0123456789abcdef",
    )
    .expect("MEMORY_HISTORICAL_TARGET_INVALID");
    let fixture = historical_memory_fixture(&target);
    assert_eq!(
        fixture.receipt.persistence().persistence_digest().as_str(),
        "0d6d25b2a3684647f69c6b7fbf855ece54b934f8ceea41d70a6e97a79a09063e"
    );
    assert_eq!(
        fixture.receipt.retrieval().retrieval_digest().as_str(),
        "7082740d23164a9c642b9ae080526e401de03391aab44d3fdefe5c71931d9034"
    );
    assert_eq!(
        fixture.receipt.receipt_digest().as_str(),
        "a95d3f137cf25e77cba45a43eeec57b7a632656372cef296393c0f84e2181360"
    );
    assert_eq!(
        fixture.reflection.receipt_digest().as_str(),
        "8b75728ec926b9e4a1ae82f69368d1f902e341d0d8803cc32734ffc7cec55110"
    );
}

#[allow(clippy::too_many_lines)]
fn stage_exact_v2_upgrade_source(config: &LiveConfig, target: &ExtensionTarget) {
    // The Store live gate owns the real global-v3 -> global-v5 transition. This
    // Memory-only gate starts from the already verified global-v5 catalog, then
    // constructs the exact retained v2/global-v3 extension source and restores
    // the v5 compatibility row before invoking the production Memory upgrader.
    let v2_manifest =
        verify_embedded_v2_extension_manifest().expect("MEMORY_EXTENSION_V2_MANIFEST_INVALID");
    let mut admin = config.admin_client();
    admin
        .batch_execute(
            "DROP SCHEMA memory CASCADE; \
             CREATE SCHEMA memory AUTHORIZATION lattice_migrator; \
             REVOKE ALL ON SCHEMA memory FROM PUBLIC; \
             COMMENT ON SCHEMA memory IS 'LATTICE_DEVOS_MEMORY_SCHEMA_V5'",
        )
        .expect("MEMORY_HISTORICAL_SCHEMA_RESET_FAILED");
    drop(admin);

    let mut migrator = config.role_client(ExtensionDatabaseRole::Migrator);
    migrator
        .batch_execute(
            std::str::from_utf8(v2_manifest.bytes()).expect("MEMORY_EXTENSION_V2_SQL_NOT_UTF8"),
        )
        .expect("MEMORY_EXTENSION_V2_SQL_APPLY_FAILED");
    let database_identity = target.expected_database_identity_digest().as_str();
    let changed = migrator
        .execute(
            "INSERT INTO memory.codebase_memory_extension_identity (\
                 singleton, extension_id, extension_schema_version, extension_path, \
                 extension_sql_sha256, extension_manifest_sha256, database_uuid, \
                 database_identity_sha256, global_schema_version, global_manifest_sha256\
             ) VALUES (true, 'lattice-codebase-memory', 2, \
                 'db/extensions/codebase-memory/v2.sql', $1, $2, $3::text::uuid, $4, 3, $5)",
            &[
                &v2_manifest.sql_sha256().as_str(),
                &v2_manifest.manifest_sha256().as_str(),
                &target.expected_database_uuid(),
                &database_identity,
                &HISTORICAL_GLOBAL_MANIFEST_SHA256,
            ],
        )
        .expect("MEMORY_EXTENSION_V2_IDENTITY_STAGE_FAILED");
    assert_eq!(changed, 1, "MEMORY_EXTENSION_V2_IDENTITY_STAGE_COUNT");
    let changed = migrator
        .execute(
            "INSERT INTO memory.codebase_memory_extension_ledger (\
                 ledger_ordinal, singleton, extension_id, extension_schema_version, \
                 extension_sql_sha256, extension_manifest_sha256, database_uuid, \
                 database_identity_sha256, global_schema_version, global_manifest_sha256, \
                 event_kind\
             ) VALUES (1, true, 'lattice-codebase-memory', 2, $1, $2, \
                 $3::text::uuid, $4, 3, $5, 'INSTALLED')",
            &[
                &v2_manifest.sql_sha256().as_str(),
                &v2_manifest.manifest_sha256().as_str(),
                &target.expected_database_uuid(),
                &database_identity,
                &HISTORICAL_GLOBAL_MANIFEST_SHA256,
            ],
        )
        .expect("MEMORY_EXTENSION_V2_LEDGER_STAGE_FAILED");
    assert_eq!(changed, 1, "MEMORY_EXTENSION_V2_LEDGER_STAGE_COUNT");

    let fixture = historical_memory_fixture(target);
    insert_historical_v2_rows(&mut migrator, &fixture);
}

#[allow(clippy::too_many_lines)]
fn historical_receipt_fingerprint(client: &mut Client) -> String {
    let has_row_profiles: bool = client
        .query_one(
            "SELECT EXISTS (\
                 SELECT 1 FROM pg_catalog.pg_attribute AS a \
                 JOIN pg_catalog.pg_class AS c ON c.oid = a.attrelid \
                 JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace \
                 WHERE n.nspname = 'memory' \
                   AND c.relname = 'codebase_memory_analyses' \
                   AND a.attname = 'persistence_database_identity_sha256' \
                   AND a.attnum > 0 AND NOT a.attisdropped\
             )",
            &[],
        )
        .expect("MEMORY_HISTORICAL_PROFILE_COLUMN_QUERY_FAILED")
        .get(0);
    let profile_query = if has_row_profiles {
        "WITH profiles AS (\
             SELECT 'analysis'::text AS source, a.persistence_database_identity_sha256, \
                    a.persistence_global_schema_version, a.persistence_global_manifest_sha256, \
                    a.persistence_extension_id, a.persistence_extension_schema_version, \
                    a.persistence_extension_sql_sha256, a.persistence_extension_manifest_sha256, \
                    pg_catalog.encode(a.persistence_digest, 'hex') AS durable_digest \
               FROM ONLY memory.codebase_memory_analyses AS a \
              WHERE a.project_id = 'task075-history-project' \
             UNION ALL \
             SELECT 'retrieval', r.persistence_database_identity_sha256, \
                    r.persistence_global_schema_version, r.persistence_global_manifest_sha256, \
                    r.persistence_extension_id, r.persistence_extension_schema_version, \
                    r.persistence_extension_sql_sha256, r.persistence_extension_manifest_sha256, \
                    pg_catalog.encode(r.retrieval_digest, 'hex') \
               FROM ONLY memory.codebase_memory_retrieval_audits AS r \
               JOIN ONLY memory.codebase_memory_analyses AS a USING (analysis_digest) \
              WHERE a.project_id = 'task075-history-project' \
             UNION ALL \
             SELECT 'receipt', x.persistence_database_identity_sha256, \
                    x.persistence_global_schema_version, x.persistence_global_manifest_sha256, \
                    x.persistence_extension_id, x.persistence_extension_schema_version, \
                    x.persistence_extension_sql_sha256, x.persistence_extension_manifest_sha256, \
                    pg_catalog.encode(x.receipt_digest, 'hex') \
               FROM ONLY memory.codebase_memory_receipts AS x \
               JOIN ONLY memory.codebase_memory_analyses AS a USING (analysis_digest) \
              WHERE a.project_id = 'task075-history-project' \
             UNION ALL \
             SELECT 'reflection', h.persistence_database_identity_sha256, \
                    h.persistence_global_schema_version, h.persistence_global_manifest_sha256, \
                    h.persistence_extension_id, h.persistence_extension_schema_version, \
                    h.persistence_extension_sql_sha256, h.persistence_extension_manifest_sha256, \
                    pg_catalog.encode(h.reflection_receipt_digest, 'hex') \
               FROM ONLY memory.codebase_memory_reflections AS h \
              WHERE h.project_id = 'task075-history-project'\
         ) \
         SELECT pg_catalog.encode(pg_catalog.sha256(\
             pg_catalog.convert_to(pg_catalog.string_agg(\
                 source || ':' || btrim(persistence_database_identity_sha256) || ':' || \
                 persistence_global_schema_version::text || ':' || \
                 btrim(persistence_global_manifest_sha256) || ':' || persistence_extension_id || ':' || \
                 persistence_extension_schema_version::text || ':' || \
                 btrim(persistence_extension_sql_sha256) || ':' || \
                 btrim(persistence_extension_manifest_sha256) || ':' || durable_digest, \
                 E'\\n' ORDER BY source), 'UTF8')), 'hex')::text FROM profiles"
    } else {
        "WITH profile AS (\
             SELECT i.database_identity_sha256 AS persistence_database_identity_sha256, \
                    i.global_schema_version AS persistence_global_schema_version, \
                    i.global_manifest_sha256 AS persistence_global_manifest_sha256, \
                    i.extension_id AS persistence_extension_id, \
                    i.extension_schema_version AS persistence_extension_schema_version, \
                    i.extension_sql_sha256 AS persistence_extension_sql_sha256, \
                    i.extension_manifest_sha256 AS persistence_extension_manifest_sha256 \
               FROM ONLY memory.codebase_memory_extension_identity AS i WHERE i.singleton\
         ), profiles AS (\
             SELECT 'analysis'::text AS source, p.*, \
                    pg_catalog.encode(a.persistence_digest, 'hex') AS durable_digest \
               FROM ONLY memory.codebase_memory_analyses AS a CROSS JOIN profile AS p \
              WHERE a.project_id = 'task075-history-project' \
             UNION ALL \
             SELECT 'retrieval', p.*, pg_catalog.encode(r.retrieval_digest, 'hex') \
               FROM ONLY memory.codebase_memory_retrieval_audits AS r \
               JOIN ONLY memory.codebase_memory_analyses AS a USING (analysis_digest) \
               CROSS JOIN profile AS p WHERE a.project_id = 'task075-history-project' \
             UNION ALL \
             SELECT 'receipt', p.*, pg_catalog.encode(x.receipt_digest, 'hex') \
               FROM ONLY memory.codebase_memory_receipts AS x \
               JOIN ONLY memory.codebase_memory_analyses AS a USING (analysis_digest) \
               CROSS JOIN profile AS p WHERE a.project_id = 'task075-history-project' \
             UNION ALL \
             SELECT 'reflection', p.*, pg_catalog.encode(h.reflection_receipt_digest, 'hex') \
               FROM ONLY memory.codebase_memory_reflections AS h CROSS JOIN profile AS p \
              WHERE h.project_id = 'task075-history-project'\
         ) \
         SELECT pg_catalog.encode(pg_catalog.sha256(\
             pg_catalog.convert_to(pg_catalog.string_agg(\
                 source || ':' || btrim(persistence_database_identity_sha256) || ':' || \
                 persistence_global_schema_version::text || ':' || \
                 btrim(persistence_global_manifest_sha256) || ':' || persistence_extension_id || ':' || \
                 persistence_extension_schema_version::text || ':' || \
                 btrim(persistence_extension_sql_sha256) || ':' || \
                 btrim(persistence_extension_manifest_sha256) || ':' || durable_digest, \
                 E'\\n' ORDER BY source), 'UTF8')), 'hex')::text FROM profiles"
    };
    client
        .query_one(profile_query, &[])
        .expect("MEMORY_HISTORICAL_RECEIPT_FINGERPRINT_FAILED")
        .get(0)
}

#[allow(clippy::too_many_lines)]
fn insert_historical_v2_rows(client: &mut Client, fixture: &HistoricalMemoryFixture) {
    let analysis = &fixture.analysis;
    let persistence = fixture.receipt.persistence();
    let retrieval = fixture.receipt.retrieval();
    let invocation = analysis.request().invocation();
    let record = &analysis.records()[0];
    let mut transaction = client
        .build_transaction()
        .isolation_level(postgres::IsolationLevel::Serializable)
        .start()
        .expect("MEMORY_HISTORICAL_ROWS_TRANSACTION_FAILED");
    let changed = transaction
        .execute(
            "INSERT INTO memory.codebase_memory_analyses (\
                 analysis_digest, contract_version, request_id, task_id, attempt_id, \
                 project_snapshot_id, subject_digest, project_id, commit_id, query_digest, \
                 configuration_digest, retrieval_limit, tree_id, manifest_digest, \
                 exclusion_digest, graphify_identity_digest, graph_artifact_digest, \
                 raw_output_digest, raw_evidence_digest, record_set_digest, record_count, \
                 persistence_digest\
             ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,\
                 $19,$20,$21,$22)",
            &[
                &digest_bytes(analysis.analysis_digest()),
                &i16::try_from(CONTRACT_VERSION).expect("fixed contract version"),
                &invocation.request_id().as_str(),
                &invocation.task_id().as_str(),
                &invocation.attempt_id().as_str(),
                &invocation.project_snapshot_id().as_str(),
                &digest_bytes(invocation.subject_digest()),
                &analysis.project_id().as_str(),
                &analysis.commit_id().as_str(),
                &digest_bytes(analysis.request().query_digest()),
                &digest_bytes(analysis.request().configuration_digest()),
                &i16::try_from(analysis.request().retrieval_limit()).expect("fixed limit"),
                &analysis.tree_id().as_str(),
                &digest_bytes(analysis.manifest_digest()),
                &digest_bytes(analysis.exclusion_digest()),
                &digest_bytes(analysis.identity_digest()),
                &digest_bytes(analysis.graph_artifact_digest()),
                &digest_bytes(analysis.raw_output_digest()),
                &digest_bytes(analysis.raw_evidence_digest()),
                &digest_bytes(analysis.record_set_digest()),
                &i32::try_from(persistence.record_count()).expect("fixed record count"),
                &digest_bytes(persistence.persistence_digest()),
            ],
        )
        .expect("MEMORY_HISTORICAL_ANALYSIS_INSERT_FAILED");
    assert_eq!(changed, 1, "MEMORY_HISTORICAL_ANALYSIS_INSERT_COUNT");
    let changed = transaction
        .execute(
            "INSERT INTO memory.codebase_memory_records (\
                 analysis_digest, ordinal, record_id, graph_kind, record_kind, review_state, \
                 trusted_context, subject, category, relation, object, source_path, \
                 source_digest, line_start, line_end, confidence, content_digest\
             ) VALUES ($1,$2,$3,$4,'OBSERVATION','CANDIDATE',false,$5,$6,$7,$8,$9,$10,$11,\
                 $12,$13,$14)",
            &[
                &digest_bytes(analysis.analysis_digest()),
                &i32::try_from(record.ordinal()).expect("fixed ordinal"),
                &digest_bytes(record.record_id()),
                &record.graph_kind().as_str(),
                &record.subject(),
                &record.category(),
                &record.relation(),
                &record.object(),
                &record.provenance().relative_path(),
                &digest_bytes(record.provenance().content_digest()),
                &record
                    .provenance()
                    .line_start()
                    .map(|value| i32::try_from(value).expect("fixed line")),
                &record
                    .provenance()
                    .line_end()
                    .map(|value| i32::try_from(value).expect("fixed line")),
                &record.confidence().as_str(),
                &digest_bytes(record.content_digest()),
            ],
        )
        .expect("MEMORY_HISTORICAL_RECORD_INSERT_FAILED");
    assert_eq!(changed, 1, "MEMORY_HISTORICAL_RECORD_INSERT_COUNT");
    let ids = retrieval
        .results()
        .iter()
        .map(|result| digest_bytes(result.record_id()))
        .collect::<Vec<_>>();
    let digests = retrieval
        .results()
        .iter()
        .map(|result| digest_bytes(result.record_digest()))
        .collect::<Vec<_>>();
    let scores = retrieval
        .results()
        .iter()
        .map(|result| i64::from(result.score()))
        .collect::<Vec<_>>();
    let changed = transaction
        .execute(
            "INSERT INTO memory.codebase_memory_retrieval_audits (\
                 retrieval_digest, analysis_digest, persistence_digest, query_digest, algorithm, \
                 retrieval_limit, disposition, result_record_ids, result_record_digests, \
                 result_scores, result_set_digest\
             ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)",
            &[
                &digest_bytes(retrieval.retrieval_digest()),
                &digest_bytes(retrieval.analysis_digest()),
                &digest_bytes(retrieval.persistence_digest()),
                &digest_bytes(retrieval.query_digest()),
                &retrieval.algorithm(),
                &i16::try_from(retrieval.limit()).expect("fixed limit"),
                &retrieval.disposition().as_str(),
                &ids,
                &digests,
                &scores,
                &digest_bytes(retrieval.result_set_digest()),
            ],
        )
        .expect("MEMORY_HISTORICAL_RETRIEVAL_INSERT_FAILED");
    assert_eq!(changed, 1, "MEMORY_HISTORICAL_RETRIEVAL_INSERT_COUNT");
    let changed = transaction
        .execute(
            "INSERT INTO memory.codebase_memory_receipts (\
                 receipt_digest, analysis_digest, retrieval_digest, persistence_digest, query_digest\
             ) VALUES ($1,$2,$3,$4,$5)",
            &[
                &digest_bytes(fixture.receipt.receipt_digest()),
                &digest_bytes(persistence.analysis_digest()),
                &digest_bytes(retrieval.retrieval_digest()),
                &digest_bytes(persistence.persistence_digest()),
                &digest_bytes(retrieval.query_digest()),
            ],
        )
        .expect("MEMORY_HISTORICAL_RECEIPT_INSERT_FAILED");
    assert_eq!(changed, 1, "MEMORY_HISTORICAL_RECEIPT_INSERT_COUNT");
    let reflection = &fixture.reflection;
    let statements = reflection
        .content()
        .findings()
        .iter()
        .map(|finding| finding.statement().to_owned())
        .collect::<Vec<_>>();
    let evidence = reflection
        .content()
        .findings()
        .iter()
        .map(|finding| digest_bytes(finding.evidence_digest()))
        .collect::<Vec<_>>();
    let changed = transaction
        .execute(
            "INSERT INTO memory.codebase_memory_reflections (\
                 reflection_receipt_digest, graph_receipt_digest, contract_version, request_id, \
                 task_id, attempt_id, project_snapshot_id, subject_digest, project_id, commit_id, \
                 query_digest, configuration_digest, retrieval_limit, reflection_schema_version, \
                 reflection_status, hermes_identity_digest, input_digest, reflection_digest, \
                 summary, finding_statements, finding_evidence_digests, next_actions\
             ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,\
                 $20,$21,$22)",
            &[
                &digest_bytes(reflection.receipt_digest()),
                &digest_bytes(reflection.graph_receipt_digest()),
                &i16::try_from(CONTRACT_VERSION).expect("fixed contract version"),
                &invocation.request_id().as_str(),
                &invocation.task_id().as_str(),
                &invocation.attempt_id().as_str(),
                &invocation.project_snapshot_id().as_str(),
                &digest_bytes(invocation.subject_digest()),
                &analysis.project_id().as_str(),
                &analysis.commit_id().as_str(),
                &digest_bytes(analysis.request().query_digest()),
                &digest_bytes(analysis.request().configuration_digest()),
                &i16::try_from(analysis.request().retrieval_limit()).expect("fixed limit"),
                &reflection.schema_version(),
                &reflection.status().as_str(),
                &digest_bytes(reflection.hermes_identity_digest()),
                &digest_bytes(reflection.input_digest()),
                &digest_bytes(reflection.reflection_digest()),
                &reflection.content().summary(),
                &statements,
                &evidence,
                &reflection.content().next_actions(),
            ],
        )
        .expect("MEMORY_HISTORICAL_REFLECTION_INSERT_FAILED");
    assert_eq!(changed, 1, "MEMORY_HISTORICAL_REFLECTION_INSERT_COUNT");
    transaction
        .commit()
        .expect("MEMORY_HISTORICAL_ROWS_COMMIT_FAILED");
}

fn prove_historical_v2_replay(config: &LiveConfig, target: &ExtensionTarget) {
    let fixture = historical_memory_fixture(target);
    let mut memory = PostgresCodebaseMemory::new(
        config.role_client(ExtensionDatabaseRole::Runtime),
        target.clone(),
    )
    .expect("MEMORY_HISTORICAL_ADAPTER_CONSTRUCTION_FAILED");
    assert_eq!(
        memory
            .load_receipt(fixture.analysis.request())
            .unwrap_or_else(|error| panic!("{}", error.code())),
        fixture.receipt,
        "MEMORY_HISTORICAL_RECEIPT_REHASHED"
    );
    assert_eq!(
        memory
            .load_reflection(fixture.analysis.request())
            .unwrap_or_else(|error| panic!("{}", error.code())),
        fixture.reflection,
        "MEMORY_HISTORICAL_REFLECTION_REHASHED"
    );
}

fn assert_current_v3_profiles(config: &LiveConfig, _target: &ExtensionTarget) {
    let mut admin = config.admin_client();
    let row = admin
        .query_one(
            "SELECT \
                 count(*) FILTER (WHERE project_id = 'task075-history-project' \
                     AND persistence_global_schema_version = 3 \
                     AND persistence_extension_schema_version = 2), \
                 count(*) FILTER (WHERE project_id = 'task033-live-project' \
                     AND persistence_global_schema_version = 5 \
                     AND persistence_extension_schema_version = 3) \
               FROM (\
                 SELECT project_id, persistence_global_schema_version, \
                        persistence_extension_schema_version \
                   FROM ONLY memory.codebase_memory_analyses \
                 UNION ALL \
                 SELECT project_id, persistence_global_schema_version, \
                        persistence_extension_schema_version \
                   FROM ONLY memory.codebase_memory_reflections\
               ) AS rows",
            &[],
        )
        .expect("MEMORY_MIXED_PROFILE_QUERY_FAILED");
    assert_eq!(row.get::<_, i64>(0), 2, "MEMORY_HISTORICAL_PROFILE_COUNT");
    assert_eq!(row.get::<_, i64>(1), 2, "MEMORY_CURRENT_PROFILE_COUNT");
}

fn prove_profile_corruption_denial(config: &LiveConfig, target: &ExtensionTarget) {
    for (mutation, repair, marker) in [
        (
            "UPDATE ONLY memory.codebase_memory_analyses SET persistence_global_manifest_sha256 = repeat('a', 64) WHERE project_id = 'task075-history-project'",
            format!("UPDATE ONLY memory.codebase_memory_analyses SET persistence_global_manifest_sha256 = '{HISTORICAL_GLOBAL_MANIFEST_SHA256}' WHERE project_id = 'task075-history-project'"),
            "MEMORY_MUTATED_PROFILE_REPLAY_ALLOWED",
        ),
        (
            "UPDATE ONLY memory.codebase_memory_retrieval_audits SET persistence_extension_manifest_sha256 = repeat('b', 64) WHERE analysis_digest = (SELECT analysis_digest FROM ONLY memory.codebase_memory_analyses WHERE project_id = 'task075-history-project')",
            "UPDATE ONLY memory.codebase_memory_retrieval_audits SET persistence_extension_manifest_sha256 = (SELECT persistence_extension_manifest_sha256 FROM ONLY memory.codebase_memory_analyses WHERE project_id = 'task075-history-project') WHERE analysis_digest = (SELECT analysis_digest FROM ONLY memory.codebase_memory_analyses WHERE project_id = 'task075-history-project')".to_owned(),
            "MEMORY_CROSS_ROW_PROFILE_REPLAY_ALLOWED",
        ),
    ] {
        assert_corruption_denied(config, target, mutation, &repair, marker);
    }
    assert_profile_omission_denied(config, target);
    assert_current_profile_substitution_denied(config, target);
}

fn assert_corruption_denied(
    config: &LiveConfig,
    target: &ExtensionTarget,
    mutation: &str,
    repair: &str,
    marker: &str,
) {
    let mut admin = config.admin_client();
    admin.batch_execute(mutation).expect(marker);
    drop(admin);
    let fixture = historical_memory_fixture(target);
    let mut memory = PostgresCodebaseMemory::new(
        config.role_client(ExtensionDatabaseRole::Runtime),
        target.clone(),
    )
    .expect("MEMORY_CORRUPTION_ADAPTER_CONSTRUCTION_FAILED");
    let receipt_denied = memory.load_receipt(fixture.analysis.request()).is_err();
    let reflection_denied = memory.load_reflection(fixture.analysis.request()).is_err();
    drop(memory);
    let mut admin = config.admin_client();
    admin
        .batch_execute(repair)
        .expect("MEMORY_PROFILE_REPAIR_FAILED");
    assert!(receipt_denied && reflection_denied, "{marker}");
}

fn assert_profile_omission_denied(config: &LiveConfig, target: &ExtensionTarget) {
    let mut admin = config.admin_client();
    admin
        .batch_execute(
            "ALTER TABLE memory.codebase_memory_receipts \
                 ALTER COLUMN persistence_extension_id DROP NOT NULL; \
             UPDATE ONLY memory.codebase_memory_receipts \
                SET persistence_extension_id = NULL \
              WHERE analysis_digest = (SELECT analysis_digest \
                  FROM ONLY memory.codebase_memory_analyses \
                  WHERE project_id = 'task075-history-project')",
        )
        .expect("MEMORY_PROFILE_OMISSION_STAGE_FAILED");
    drop(admin);
    let fixture = historical_memory_fixture(target);
    let mut memory = PostgresCodebaseMemory::new(
        config.role_client(ExtensionDatabaseRole::Runtime),
        target.clone(),
    )
    .expect("MEMORY_OMISSION_ADAPTER_CONSTRUCTION_FAILED");
    let denied = memory.load_receipt(fixture.analysis.request()).is_err();
    drop(memory);
    let mut admin = config.admin_client();
    admin
        .batch_execute(
            "UPDATE ONLY memory.codebase_memory_receipts \
                SET persistence_extension_id = 'lattice-codebase-memory' \
              WHERE persistence_extension_id IS NULL; \
             ALTER TABLE memory.codebase_memory_receipts \
                 ALTER COLUMN persistence_extension_id SET NOT NULL",
        )
        .expect("MEMORY_PROFILE_OMISSION_REPAIR_FAILED");
    assert!(denied, "MEMORY_MISSING_PROFILE_REPLAY_ALLOWED");
}

fn assert_current_profile_substitution_denied(config: &LiveConfig, target: &ExtensionTarget) {
    let manifest = verify_embedded_extension_manifest().expect("MEMORY_EXTENSION_MANIFEST_INVALID");
    let fixture = historical_memory_fixture(target);
    set_historical_row_profile(
        config,
        5,
        CURRENT_GLOBAL_MANIFEST_SHA256,
        3,
        manifest.sql_sha256().as_str(),
        manifest.manifest_sha256().as_str(),
    );
    let mut memory = PostgresCodebaseMemory::new(
        config.role_client(ExtensionDatabaseRole::Runtime),
        target.clone(),
    )
    .expect("MEMORY_SUBSTITUTION_ADAPTER_CONSTRUCTION_FAILED");
    let receipt_denied = memory.load_receipt(fixture.analysis.request()).is_err();
    let reflection_denied = memory.load_reflection(fixture.analysis.request()).is_err();
    drop(memory);
    let v2 = verify_embedded_v2_extension_manifest().expect("MEMORY_EXTENSION_V2_MANIFEST_INVALID");
    set_historical_row_profile(
        config,
        3,
        HISTORICAL_GLOBAL_MANIFEST_SHA256,
        2,
        v2.sql_sha256().as_str(),
        v2.manifest_sha256().as_str(),
    );
    assert!(
        receipt_denied && reflection_denied,
        "MEMORY_CURRENT_PROFILE_SUBSTITUTION_ALLOWED"
    );
}

fn set_historical_row_profile(
    config: &LiveConfig,
    global_schema: i16,
    global_manifest: &str,
    extension_schema: i16,
    extension_sql: &str,
    extension_manifest: &str,
) {
    let mut admin = config.admin_client();
    for table in [
        "codebase_memory_analyses",
        "codebase_memory_retrieval_audits",
        "codebase_memory_receipts",
        "codebase_memory_reflections",
    ] {
        let predicate = if matches!(
            table,
            "codebase_memory_analyses" | "codebase_memory_reflections"
        ) {
            "project_id = 'task075-history-project'"
        } else {
            "analysis_digest = (SELECT analysis_digest FROM ONLY memory.codebase_memory_analyses WHERE project_id = 'task075-history-project')"
        };
        let statement = format!(
            "UPDATE ONLY memory.{table} SET \
                 persistence_global_schema_version = $1, \
                 persistence_global_manifest_sha256 = $2, \
                 persistence_extension_schema_version = $3, \
                 persistence_extension_sql_sha256 = $4, \
                 persistence_extension_manifest_sha256 = $5 \
             WHERE {predicate}"
        );
        admin
            .execute(
                &statement,
                &[
                    &global_schema,
                    &global_manifest,
                    &extension_schema,
                    &extension_sql,
                    &extension_manifest,
                ],
            )
            .expect("MEMORY_PROFILE_SUBSTITUTION_STAGE_FAILED");
    }
}

fn historical_memory_fixture(target: &ExtensionTarget) -> HistoricalMemoryFixture {
    let (analysis, plan) = historical_graph_memory_fixture();
    let manifest =
        verify_embedded_v2_extension_manifest().expect("MEMORY_EXTENSION_V2_MANIFEST_INVALID");
    let identity = CodebaseMemoryPersistenceIdentity::v2(
        target.expected_database_identity_digest().clone(),
        ContentDigest::from_sha256(HISTORICAL_GLOBAL_MANIFEST_SHA256)
            .expect("MEMORY_HISTORICAL_GLOBAL_MANIFEST_INVALID"),
        manifest.sql_sha256().clone(),
        manifest.manifest_sha256().clone(),
    )
    .expect("MEMORY_HISTORICAL_IDENTITY_INVALID");
    let persistence_digest = fixture_persistence_digest(&analysis, &identity);
    let persistence =
        GraphMemoryPersistenceEvidence::new(&analysis, identity.clone(), persistence_digest)
            .expect("MEMORY_HISTORICAL_PERSISTENCE_INVALID");
    let retrieval_digest = fixture_retrieval_digest(&persistence, &plan);
    let retrieval = MemoryRetrievalEvidence::new(&persistence, plan, retrieval_digest)
        .expect("MEMORY_HISTORICAL_RETRIEVAL_INVALID");
    let receipt_digest = fixture_receipt_digest(&persistence, &retrieval);
    let receipt = GraphMemoryReceipt::new(persistence, retrieval, receipt_digest)
        .expect("MEMORY_HISTORICAL_RECEIPT_INVALID");
    let candidate = reflection_candidate(&receipt);
    let reflection_digest = fixture_reflection_receipt_digest(&candidate, &identity);
    let reflection = HermesReflectionReceipt::from_candidate(candidate, reflection_digest)
        .expect("MEMORY_HISTORICAL_REFLECTION_INVALID");
    HistoricalMemoryFixture {
        analysis,
        receipt,
        reflection,
    }
}

fn fixture_persistence_digest(
    analysis: &NormalizedGraphAnalysis,
    identity: &CodebaseMemoryPersistenceIdentity,
) -> ContentDigest {
    fixture_hash(
        "lattice.postgres-codebase-memory.persistence",
        &CanonicalValue::Object(vec![
            (
                "analysis".to_owned(),
                fixture_string(analysis.analysis_digest().as_str()),
            ),
            (
                "commit".to_owned(),
                fixture_string(analysis.commit_id().as_str()),
            ),
            (
                "configuration".to_owned(),
                fixture_string(analysis.request().configuration_digest().as_str()),
            ),
            ("identity".to_owned(), fixture_identity_value(identity)),
            (
                "project".to_owned(),
                fixture_string(analysis.project_id().as_str()),
            ),
            (
                "query".to_owned(),
                fixture_string(analysis.request().query_digest().as_str()),
            ),
            (
                "record_count".to_owned(),
                fixture_string(analysis.records().len().to_string()),
            ),
            (
                "record_set".to_owned(),
                fixture_string(analysis.record_set_digest().as_str()),
            ),
            (
                "request".to_owned(),
                fixture_request_value(analysis.request()),
            ),
        ]),
    )
}

fn fixture_retrieval_digest(
    persistence: &GraphMemoryPersistenceEvidence,
    plan: &MemoryRetrievalPlan,
) -> ContentDigest {
    fixture_hash(
        "lattice.postgres-codebase-memory.retrieval",
        &CanonicalValue::Object(vec![
            ("algorithm".to_owned(), fixture_string(plan.algorithm())),
            (
                "analysis".to_owned(),
                fixture_string(plan.analysis_digest().as_str()),
            ),
            (
                "disposition".to_owned(),
                fixture_string(plan.disposition().as_str()),
            ),
            (
                "identity".to_owned(),
                fixture_identity_value(persistence.identity()),
            ),
            ("limit".to_owned(), fixture_string(plan.limit().to_string())),
            (
                "persistence".to_owned(),
                fixture_string(persistence.persistence_digest().as_str()),
            ),
            (
                "query".to_owned(),
                fixture_string(plan.query_digest().as_str()),
            ),
            (
                "result_set".to_owned(),
                fixture_string(plan.result_set_digest().as_str()),
            ),
            ("results".to_owned(), fixture_results_value(plan.results())),
        ]),
    )
}

fn fixture_receipt_digest(
    persistence: &GraphMemoryPersistenceEvidence,
    retrieval: &MemoryRetrievalEvidence,
) -> ContentDigest {
    fixture_hash(
        "lattice.postgres-codebase-memory.receipt",
        &CanonicalValue::Object(vec![
            (
                "analysis".to_owned(),
                fixture_string(persistence.analysis_digest().as_str()),
            ),
            (
                "identity".to_owned(),
                fixture_identity_value(persistence.identity()),
            ),
            (
                "persistence".to_owned(),
                fixture_string(persistence.persistence_digest().as_str()),
            ),
            (
                "query".to_owned(),
                fixture_string(retrieval.query_digest().as_str()),
            ),
            (
                "retrieval".to_owned(),
                fixture_string(retrieval.retrieval_digest().as_str()),
            ),
        ]),
    )
}

fn fixture_reflection_receipt_digest(
    reflection: &HermesReflectionCandidate,
    identity: &CodebaseMemoryPersistenceIdentity,
) -> ContentDigest {
    fixture_hash(
        "lattice.postgres-codebase-memory.hermes-reflection-receipt",
        &CanonicalValue::Object(vec![
            (
                "commit".to_owned(),
                fixture_string(reflection.request().commit_id().as_str()),
            ),
            (
                "configuration".to_owned(),
                fixture_string(reflection.request().configuration_digest().as_str()),
            ),
            (
                "content".to_owned(),
                fixture_reflection_content_value(reflection.content()),
            ),
            (
                "graph_receipt".to_owned(),
                fixture_string(reflection.graph_receipt_digest().as_str()),
            ),
            (
                "hermes_identity".to_owned(),
                fixture_string(reflection.hermes_identity_digest().as_str()),
            ),
            ("identity".to_owned(), fixture_identity_value(identity)),
            (
                "input".to_owned(),
                fixture_string(reflection.input_digest().as_str()),
            ),
            (
                "project".to_owned(),
                fixture_string(reflection.request().project_id().as_str()),
            ),
            (
                "query".to_owned(),
                fixture_string(reflection.request().query_digest().as_str()),
            ),
            (
                "reflection".to_owned(),
                fixture_string(reflection.reflection_digest().as_str()),
            ),
            (
                "request".to_owned(),
                fixture_request_value(reflection.request()),
            ),
            (
                "schema".to_owned(),
                fixture_string(HERMES_REFLECTION_SCHEMA_VERSION),
            ),
            (
                "status".to_owned(),
                fixture_string(HermesReflectionStatus::InferenceCandidate.as_str()),
            ),
        ]),
    )
}

fn fixture_request_value(request: &GraphMemoryRunRequest) -> CanonicalValue {
    let invocation = request.invocation();
    CanonicalValue::Object(vec![
        (
            "attempt".to_owned(),
            fixture_string(invocation.attempt_id().as_str()),
        ),
        (
            "contract".to_owned(),
            fixture_string(invocation.version().to_string()),
        ),
        (
            "project_snapshot".to_owned(),
            fixture_string(invocation.project_snapshot_id().as_str()),
        ),
        (
            "request".to_owned(),
            fixture_string(invocation.request_id().as_str()),
        ),
        (
            "retrieval_limit".to_owned(),
            fixture_string(request.retrieval_limit().to_string()),
        ),
        (
            "subject".to_owned(),
            fixture_string(invocation.subject_digest().as_str()),
        ),
        (
            "task".to_owned(),
            fixture_string(invocation.task_id().as_str()),
        ),
    ])
}

fn fixture_identity_value(identity: &CodebaseMemoryPersistenceIdentity) -> CanonicalValue {
    CanonicalValue::Object(vec![
        (
            "database".to_owned(),
            fixture_string(identity.database_identity_digest().as_str()),
        ),
        (
            "extension_id".to_owned(),
            fixture_string(identity.extension_id()),
        ),
        (
            "extension_manifest".to_owned(),
            fixture_string(identity.extension_manifest_digest().as_str()),
        ),
        (
            "extension_schema".to_owned(),
            fixture_string(identity.extension_schema_version().to_string()),
        ),
        (
            "extension_sql".to_owned(),
            fixture_string(identity.extension_sql_digest().as_str()),
        ),
        (
            "global_manifest".to_owned(),
            fixture_string(identity.global_manifest_digest().as_str()),
        ),
        (
            "global_schema".to_owned(),
            fixture_string(identity.global_schema_version().to_string()),
        ),
    ])
}

fn fixture_results_value(results: &[RankedMemoryRecord]) -> CanonicalValue {
    CanonicalValue::Array(
        results
            .iter()
            .map(|result| {
                CanonicalValue::Object(vec![
                    (
                        "digest".to_owned(),
                        fixture_string(result.record_digest().as_str()),
                    ),
                    ("id".to_owned(), fixture_string(result.record_id().as_str())),
                    ("rank".to_owned(), fixture_string(result.rank().to_string())),
                    (
                        "score".to_owned(),
                        fixture_string(result.score().to_string()),
                    ),
                ])
            })
            .collect(),
    )
}

fn fixture_reflection_content_value(content: &HermesReflectionContent) -> CanonicalValue {
    CanonicalValue::Object(vec![
        (
            "findings".to_owned(),
            CanonicalValue::Array(
                content
                    .findings()
                    .iter()
                    .map(|finding| {
                        CanonicalValue::Object(vec![
                            (
                                "evidence".to_owned(),
                                fixture_string(finding.evidence_digest().as_str()),
                            ),
                            ("statement".to_owned(), fixture_string(finding.statement())),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "next_actions".to_owned(),
            CanonicalValue::Array(content.next_actions().iter().map(fixture_string).collect()),
        ),
        ("summary".to_owned(), fixture_string(content.summary())),
    ])
}

fn fixture_hash(schema_id: &str, value: &CanonicalValue) -> ContentDigest {
    let domain = HashDomain::new(schema_id, "1").expect("MEMORY_FIXTURE_HASH_DOMAIN_INVALID");
    let digest = canonical_sha256(&domain, value).expect("MEMORY_FIXTURE_HASH_FAILED");
    ContentDigest::from_sha256(digest.to_hex()).expect("MEMORY_FIXTURE_DIGEST_INVALID")
}

fn fixture_string(value: impl Into<String>) -> CanonicalValue {
    CanonicalValue::String(value.into())
}

fn digest_bytes(value: &ContentDigest) -> Vec<u8> {
    value
        .as_str()
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            u8::from_str_radix(
                std::str::from_utf8(pair).expect("MEMORY_DIGEST_NOT_UTF8"),
                16,
            )
            .expect("MEMORY_DIGEST_NOT_HEX")
        })
        .collect()
}

fn prove_foreign_acl_rejected(config: &LiveConfig, target: &ExtensionTarget) {
    let mut admin = config.admin_client();
    admin
        .batch_execute(
            "CREATE ROLE task033_memory_foreign NOLOGIN; \
             GRANT USAGE ON SCHEMA memory TO task033_memory_foreign; \
             GRANT SELECT ON memory.codebase_memory_reflections TO task033_memory_foreign",
        )
        .expect("MEMORY_EXTENSION_FOREIGN_ACL_FIXTURE_FAILED");
    drop(admin);

    let mut migrator = config.role_client(ExtensionDatabaseRole::Migrator);
    assert_setup_kind(
        verify_extension(&mut migrator, target, ExtensionDatabaseRole::Migrator),
        ExtensionSetupErrorKind::CatalogMismatch,
    );
    drop(migrator);

    let mut admin = config.admin_client();
    admin
        .batch_execute(
            "REVOKE SELECT ON memory.codebase_memory_reflections FROM task033_memory_foreign; \
             REVOKE USAGE ON SCHEMA memory FROM task033_memory_foreign; \
             DROP ROLE task033_memory_foreign",
        )
        .expect("MEMORY_EXTENSION_FOREIGN_ACL_CLEANUP_FAILED");
    drop(admin);

    let mut migrator = config.role_client(ExtensionDatabaseRole::Migrator);
    verify_extension(&mut migrator, target, ExtensionDatabaseRole::Migrator)
        .unwrap_or_else(|error| panic!("{}", error.code()));
}

fn prove_reflection_durability_boundary(config: &LiveConfig) {
    let mut runtime = config.role_client(ExtensionDatabaseRole::Runtime);
    let legacy_error = runtime
        .query_one(
            "SELECT reflection_status \
               FROM memory.codebase_memory_persist_reflection_v2(\
                   NULL::bytea,NULL::bytea,NULL::bytea,NULL::bytea,NULL::smallint,\
                   NULL::text,NULL::text,NULL::text,NULL::text,NULL::bytea,\
                   NULL::text,NULL::text,NULL::bytea,NULL::bytea,NULL::smallint,\
                   NULL::bytea,NULL::text,NULL::text,NULL::bytea,NULL::bytea,\
                   NULL::bytea,NULL::bytea,NULL::text,NULL::text[],NULL::bytea[],NULL::text[]\
               )",
            &[],
        )
        .expect_err("MEMORY_REFLECTION_V2_RUNTIME_EXECUTE_ALLOWED");
    assert_eq!(
        legacy_error.code(),
        Some(&SqlState::INSUFFICIENT_PRIVILEGE),
        "MEMORY_REFLECTION_V2_RUNTIME_DENIAL_WRONG"
    );
    runtime
        .batch_execute(
            "BEGIN ISOLATION LEVEL SERIALIZABLE; \
             SET LOCAL search_path = pg_catalog; \
             SET LOCAL row_security = on; \
             SET LOCAL synchronous_commit = off",
        )
        .expect("MEMORY_REFLECTION_DURABILITY_FIXTURE_FAILED");
    let error = runtime
        .query_one(
            "SELECT reflection_status \
               FROM memory.codebase_memory_persist_reflection_v3(\
                   NULL::bytea,NULL::bytea,NULL::bytea,NULL::bytea,NULL::smallint,\
                   NULL::text,NULL::text,NULL::text,NULL::text,NULL::bytea,\
                   NULL::text,NULL::text,NULL::bytea,NULL::bytea,NULL::smallint,\
                   NULL::bytea,NULL::text,NULL::text,NULL::bytea,NULL::bytea,\
                   NULL::bytea,NULL::bytea,NULL::text,NULL::text[],NULL::bytea[],NULL::text[]\
               )",
            &[],
        )
        .expect_err("MEMORY_REFLECTION_SYNCHRONOUS_COMMIT_OFF_ALLOWED");
    assert_eq!(
        error.code().map(SqlState::code),
        Some("LCM01"),
        "MEMORY_REFLECTION_DURABILITY_DENIAL_WRONG"
    );
    runtime
        .batch_execute("ROLLBACK")
        .expect("MEMORY_REFLECTION_DURABILITY_ROLLBACK_FAILED");
}

#[allow(clippy::too_many_lines)]
fn exercise_runtime_memory(config: &LiveConfig, target: &ExtensionTarget, restarted: bool) {
    let (analysis, plan) = graph_memory_fixture();
    let before_restart_read = restarted.then(|| reflection_storage_fingerprint(config));
    let runtime = config.role_client(ExtensionDatabaseRole::Runtime);
    let mut memory = PostgresCodebaseMemory::new(runtime, target.clone())
        .expect("MEMORY_ADAPTER_CONSTRUCTION_FAILED");

    if restarted {
        let replayed = memory
            .load_receipt(analysis.request())
            .unwrap_or_else(|error| panic!("{}", error.code()));
        assert!(replayed.matches_request(analysis.request()));
        assert_eq!(replayed.persistence().record_count(), 1);
        assert_eq!(replayed.retrieval().results().len(), 1);
        let reflection = memory
            .load_reflection(analysis.request())
            .unwrap_or_else(|error| panic!("{}", error.code()));
        assert_reflection(&reflection, &replayed);
        let changed_request = changed_request(&analysis);
        assert!(
            memory.load_receipt(&changed_request).is_err(),
            "MEMORY_CHANGED_REQUEST_REPLAY_ALLOWED"
        );
        assert!(
            memory.load_reflection(&changed_request).is_err(),
            "MEMORY_CHANGED_REFLECTION_REPLAY_ALLOWED"
        );
        drop(memory);
        assert_eq!(
            before_restart_read.expect("restart fingerprint"),
            reflection_storage_fingerprint(config),
            "MEMORY_RESTART_READER_MUTATED_REFLECTION"
        );
        println!(
            "MEMORY_RUNTIME_REPLAY_OK phase=restart receipt={} persistence={} database_identity={}",
            replayed.receipt_digest().as_str(),
            replayed.persistence().persistence_digest().as_str(),
            replayed
                .persistence()
                .identity()
                .database_identity_digest()
                .as_str()
        );
        return;
    }

    let persisted = memory
        .persist_analysis(&analysis)
        .unwrap_or_else(|error| panic!("{}", error.code()));
    assert_eq!(persisted.record_count(), 1);
    assert_eq!(persisted.identity(), memory.identity());
    assert_eq!(
        memory
            .persist_analysis(&analysis)
            .unwrap_or_else(|error| panic!("{}", error.code())),
        persisted,
        "MEMORY_ANALYSIS_EXACT_RETRY_CHANGED"
    );
    let receipt = memory
        .retrieve(&persisted, plan.clone())
        .unwrap_or_else(|error| panic!("{}", error.code()));
    assert_eq!(receipt.retrieval().results().len(), 1);
    assert_eq!(receipt.retrieval().results()[0].rank(), 1);
    assert_eq!(
        memory
            .retrieve(&persisted, plan)
            .unwrap_or_else(|error| panic!("{}", error.code())),
        receipt,
        "MEMORY_RETRIEVAL_EXACT_RETRY_CHANGED"
    );
    assert_eq!(
        memory
            .load_receipt(analysis.request())
            .unwrap_or_else(|error| panic!("{}", error.code())),
        receipt,
        "MEMORY_RECEIPT_EXACT_REPLAY_CHANGED"
    );

    let candidate = reflection_candidate(&receipt);
    let reflection = memory
        .persist_reflection(&candidate)
        .unwrap_or_else(|error| panic!("{}", error.code()));
    assert_reflection(&reflection, &receipt);
    assert_eq!(
        memory
            .persist_reflection(&candidate)
            .unwrap_or_else(|error| panic!("{}", error.code())),
        reflection,
        "MEMORY_REFLECTION_EXACT_RETRY_CHANGED"
    );
    assert_eq!(
        memory
            .load_reflection(analysis.request())
            .unwrap_or_else(|error| panic!("{}", error.code())),
        reflection,
        "MEMORY_REFLECTION_CONTENT_REPLAY_CHANGED"
    );

    let changed_request = changed_request(&analysis);
    assert!(
        memory.load_receipt(&changed_request).is_err(),
        "MEMORY_CHANGED_REQUEST_REPLAY_ALLOWED"
    );
    assert!(
        memory.load_reflection(&changed_request).is_err(),
        "MEMORY_CHANGED_REFLECTION_REPLAY_ALLOWED"
    );
    println!(
        "MEMORY_RUNTIME_REPLAY_OK phase=initial receipt={} persistence={} database_identity={}",
        receipt.receipt_digest().as_str(),
        receipt.persistence().persistence_digest().as_str(),
        receipt
            .persistence()
            .identity()
            .database_identity_digest()
            .as_str()
    );
}

#[allow(clippy::too_many_lines)]
fn exercise_openclaw_idempotency(config: &LiveConfig, target: &ExtensionTarget, restarted: bool) {
    let request = openclaw_request("project-a", "openclaw-correlation-a");
    let reply = openclaw_reply(&request);
    let scope = openclaw_scope(&request, 7);
    if !restarted {
        let mut store = new_openclaw_store(config, target);
        assert_eq!(
            store
                .reconcile_and_claim(&scope, &request)
                .expect("OPENCLAW_POSTGRES_INITIAL_CLAIM_FAILED"),
            OpenClawIdempotencyDecision::Claimed,
            "OPENCLAW_POSTGRES_INITIAL_CLAIM_CHANGED"
        );
        drop(store);
        let fingerprint = openclaw_storage_fingerprint(config);
        assert_eq!(fingerprint.0, 1, "OPENCLAW_POSTGRES_INITIAL_ROW_COUNT");
        println!(
            "OPENCLAW_POSTGRES_CLAIM_OK phase=initial command={} digest={}",
            scope.command_id().as_str(),
            request.request_digest().as_str()
        );
        return;
    }

    let claimed_fingerprint = openclaw_storage_fingerprint(config);
    let mut restarted_store = new_openclaw_store(config, target);
    assert_eq!(
        restarted_store
            .reconcile_and_claim(&scope, &request)
            .expect("OPENCLAW_POSTGRES_RESTART_CLAIM_FAILED"),
        OpenClawIdempotencyDecision::InFlight,
        "OPENCLAW_POSTGRES_RESTART_REDISPATCH_ALLOWED"
    );

    let foreign_project_request = openclaw_request("project-b", "openclaw-correlation-project");
    let foreign_project_scope = openclaw_scope(&foreign_project_request, 7);
    assert_eq!(
        restarted_store
            .reconcile_and_claim(&foreign_project_scope, &foreign_project_request)
            .expect("OPENCLAW_POSTGRES_PROJECT_RECONCILIATION_FAILED"),
        OpenClawIdempotencyDecision::CommandSubstitution,
        "OPENCLAW_POSTGRES_CROSS_PROJECT_ALLOWED"
    );
    let foreign_epoch_scope = openclaw_scope(&request, 8);
    assert_eq!(
        restarted_store
            .reconcile_and_claim(&foreign_epoch_scope, &request)
            .expect("OPENCLAW_POSTGRES_EPOCH_RECONCILIATION_FAILED"),
        OpenClawIdempotencyDecision::CommandSubstitution,
        "OPENCLAW_POSTGRES_CROSS_EPOCH_ALLOWED"
    );
    let changed_request = openclaw_request("project-a", "openclaw-correlation-changed");
    assert_eq!(
        restarted_store
            .reconcile_and_claim(&scope, &changed_request)
            .expect("OPENCLAW_POSTGRES_DIGEST_RECONCILIATION_FAILED"),
        OpenClawIdempotencyDecision::CommandSubstitution,
        "OPENCLAW_POSTGRES_CROSS_DIGEST_ALLOWED"
    );
    drop(restarted_store);
    assert_eq!(
        openclaw_storage_fingerprint(config),
        claimed_fingerprint,
        "OPENCLAW_POSTGRES_PURE_RECONCILIATION_MUTATED_ROW"
    );

    let foreign_project_record = OpenClawTerminalCommandRecord::new(
        foreign_project_scope,
        foreign_project_request.clone(),
        openclaw_reply(&foreign_project_request),
    )
    .expect("OPENCLAW_POSTGRES_FOREIGN_PROJECT_RECORD_INVALID");
    assert!(
        new_openclaw_store(config, target)
            .finalize_terminal(foreign_project_record)
            .is_err(),
        "OPENCLAW_POSTGRES_CROSS_PROJECT_FINALIZE_ALLOWED"
    );
    let foreign_epoch_record =
        OpenClawTerminalCommandRecord::new(foreign_epoch_scope, request.clone(), reply.clone())
            .expect("OPENCLAW_POSTGRES_FOREIGN_EPOCH_RECORD_INVALID");
    assert!(
        new_openclaw_store(config, target)
            .finalize_terminal(foreign_epoch_record)
            .is_err(),
        "OPENCLAW_POSTGRES_CROSS_EPOCH_FINALIZE_ALLOWED"
    );
    let changed_record = OpenClawTerminalCommandRecord::new(
        scope.clone(),
        changed_request.clone(),
        openclaw_reply(&changed_request),
    )
    .expect("OPENCLAW_POSTGRES_CHANGED_RECORD_INVALID");
    assert!(
        new_openclaw_store(config, target)
            .finalize_terminal(changed_record)
            .is_err(),
        "OPENCLAW_POSTGRES_CROSS_DIGEST_FINALIZE_ALLOWED"
    );
    assert_eq!(
        openclaw_storage_fingerprint(config),
        claimed_fingerprint,
        "OPENCLAW_POSTGRES_REJECTED_FINALIZE_MUTATED_ROW"
    );

    let record = OpenClawTerminalCommandRecord::new(scope, request.clone(), reply.clone())
        .expect("OPENCLAW_POSTGRES_TERMINAL_RECORD_INVALID");
    new_openclaw_store(config, target)
        .finalize_terminal(record.clone())
        .expect("OPENCLAW_POSTGRES_FINALIZE_FAILED");
    let terminal_fingerprint = openclaw_storage_fingerprint(config);
    assert_ne!(
        terminal_fingerprint, claimed_fingerprint,
        "OPENCLAW_POSTGRES_FINALIZE_DID_NOT_MUTATE_CLAIM"
    );

    let mut fresh_store = new_openclaw_store(config, target);
    let exact = fresh_store
        .reconcile_and_claim(record.scope(), &request)
        .expect("OPENCLAW_POSTGRES_FRESH_EXACT_RETRY_FAILED");
    match exact {
        OpenClawIdempotencyDecision::Exact(replayed) => assert_eq!(
            replayed.as_ref(),
            &reply,
            "OPENCLAW_POSTGRES_TERMINAL_REPLY_CHANGED"
        ),
        other => panic!("OPENCLAW_POSTGRES_TERMINAL_REPLAY_NOT_EXACT: {other:?}"),
    }
    assert_eq!(
        openclaw_storage_fingerprint(config),
        terminal_fingerprint,
        "OPENCLAW_POSTGRES_EXACT_RETRY_MUTATED_ROW"
    );
    fresh_store
        .finalize_terminal(record)
        .expect("OPENCLAW_POSTGRES_FINALIZE_REPLAY_FAILED");
    drop(fresh_store);
    assert_eq!(
        openclaw_storage_fingerprint(config),
        terminal_fingerprint,
        "OPENCLAW_POSTGRES_FINALIZE_REPLAY_MUTATED_ROW"
    );
    println!(
        "OPENCLAW_POSTGRES_RECONCILIATION_OK phase=restart reply={}",
        reply.reply_digest().as_str()
    );
}

fn new_openclaw_store(config: &LiveConfig, target: &ExtensionTarget) -> PostgresCodebaseMemory {
    PostgresCodebaseMemory::new(
        config.role_client(ExtensionDatabaseRole::Runtime),
        target.clone(),
    )
    .expect("OPENCLAW_POSTGRES_STORE_CONSTRUCTION_FAILED")
}

fn openclaw_request(project: &str, correlation: &str) -> GatewayRequest {
    build_request(
        GatewayCommandId::new("openclaw-command-restart").expect("openclaw command"),
        GatewayCorrelationId::new(correlation).expect("openclaw correlation"),
        GatewayRequestBody::Status(GatewayStatusTarget::Project(
            GatewayProjectStatusTarget::new(
                ProjectId::new(project).expect("openclaw project"),
                10,
                None,
            )
            .expect("openclaw status target"),
        )),
    )
    .expect("OPENCLAW_POSTGRES_REQUEST_INVALID")
}

fn openclaw_reply(request: &GatewayRequest) -> GatewayReply {
    build_reply(
        request,
        GatewayReplyBody::Denied(GatewayDenialCode::DownstreamDenied),
    )
    .expect("OPENCLAW_POSTGRES_REPLY_INVALID")
}

fn openclaw_scope(request: &GatewayRequest, epoch: u64) -> OpenClawCommandScope {
    OpenClawCommandScope::new(
        request.project_id().clone(),
        GatewayActorId::new("openclaw-live-actor").expect("openclaw actor"),
        epoch,
        request.command_id().clone(),
    )
    .expect("OPENCLAW_POSTGRES_SCOPE_INVALID")
}

fn openclaw_storage_fingerprint(config: &LiveConfig) -> (i64, String) {
    let mut admin = config.admin_client();
    let row = admin
        .query_one(
            "SELECT pg_catalog.count(*)::bigint, \
                    coalesce(pg_catalog.string_agg( \
                        actor_id || ':' || command_id || ':' || project_id || ':' || \
                        logical_session_epoch::text || ':' || \
                        pg_catalog.encode(request_digest, 'hex') || ':' || command_state || ':' || \
                        coalesce(pg_catalog.encode(terminal_reply_digest, 'hex'), '-') || ':' || \
                        coalesce(pg_catalog.encode(terminal_frame_digest, 'hex'), '-') || ':' || \
                        xmin::text || ':' || claimed_at::text || ':' || \
                        coalesce(finalized_at::text, '-'), \
                        ',' ORDER BY actor_id, command_id \
                    ), '')::text \
               FROM ONLY memory.openclaw_gateway_commands",
            &[],
        )
        .expect("OPENCLAW_POSTGRES_FINGERPRINT_FAILED");
    (row.get(0), row.get(1))
}

fn changed_request(analysis: &NormalizedGraphAnalysis) -> GraphMemoryRunRequest {
    GraphMemoryRunRequest::new(
        analysis.request().invocation().clone(),
        analysis.request().project_id().clone(),
        analysis.request().commit_id().clone(),
        analysis.request().query_digest().clone(),
        digest('9'),
        analysis.request().retrieval_limit(),
    )
    .expect("MEMORY_CHANGED_REQUEST_INVALID")
}

fn reflection_storage_fingerprint(config: &LiveConfig) -> (i64, String) {
    let mut admin = config.admin_client();
    let row = admin
        .query_one(
            "SELECT pg_catalog.count(*)::bigint, \
                    coalesce(pg_catalog.string_agg( \
                        pg_catalog.encode(reflection_receipt_digest, 'hex') || ':' || \
                        xmin::text || ':' || recorded_at::text, \
                        ',' ORDER BY reflection_receipt_digest \
                    ), '')::text \
               FROM ONLY memory.codebase_memory_reflections",
            &[],
        )
        .expect("MEMORY_REFLECTION_FINGERPRINT_QUERY_FAILED");
    (row.get(0), row.get(1))
}

fn reflection_candidate(
    graph_receipt: &lattice_contracts::GraphMemoryReceipt,
) -> HermesReflectionCandidate {
    let finding = HermesReflectionFinding::new(
        "The exact graph snapshot contains the CodebaseMemoryPort boundary.",
        digest('7'),
    )
    .expect("MEMORY_LIVE_REFLECTION_FINDING_INVALID");
    let content = HermesReflectionContent::new(
        "The bounded reflection is tied to the persisted TASK-033 graph receipt.",
        vec![finding],
        vec!["Review the inference before any later implementation decision.".to_owned()],
    )
    .expect("MEMORY_LIVE_REFLECTION_CONTENT_INVALID");
    HermesReflectionCandidate::new(
        graph_receipt.persistence().request(),
        graph_receipt,
        content,
        digest('8'),
        digest('9'),
        digest('b'),
    )
    .expect("MEMORY_LIVE_REFLECTION_CANDIDATE_INVALID")
}

fn assert_reflection(
    reflection: &HermesReflectionReceipt,
    graph_receipt: &lattice_contracts::GraphMemoryReceipt,
) {
    assert_eq!(
        reflection.schema_version(),
        HERMES_REFLECTION_SCHEMA_VERSION
    );
    assert_eq!(
        reflection.status(),
        HermesReflectionStatus::InferenceCandidate
    );
    assert_eq!(reflection.request(), graph_receipt.persistence().request());
    assert_eq!(
        reflection.project_id(),
        graph_receipt.persistence().request().project_id()
    );
    assert_eq!(
        reflection.commit_id(),
        graph_receipt.persistence().request().commit_id()
    );
    assert_eq!(
        reflection.graph_receipt_digest(),
        graph_receipt.receipt_digest()
    );
    assert_eq!(
        reflection.content().summary(),
        "The bounded reflection is tied to the persisted TASK-033 graph receipt."
    );
    assert_eq!(reflection.content().findings().len(), 1);
    assert_eq!(
        reflection.content().findings()[0].statement(),
        "The exact graph snapshot contains the CodebaseMemoryPort boundary."
    );
    assert_eq!(
        reflection.content().findings()[0].evidence_digest(),
        &digest('7')
    );
    assert_eq!(
        reflection.content().next_actions(),
        &["Review the inference before any later implementation decision.".to_owned()]
    );
    assert_eq!(reflection.hermes_identity_digest(), &digest('8'));
    assert_eq!(reflection.input_digest(), &digest('9'));
    assert_eq!(reflection.reflection_digest(), &digest('b'));
    assert_ne!(reflection.receipt_digest(), graph_receipt.receipt_digest());
}

fn historical_graph_memory_fixture() -> (NormalizedGraphAnalysis, MemoryRetrievalPlan) {
    let query_text = "CodebaseMemoryPort";
    let invocation = Invocation::new(
        CONTRACT_VERSION,
        RequestId::new("task075-history-request").expect("request id"),
        TaskId::new("TASK-075").expect("task id"),
        AttemptId::new("task075-history-attempt").expect("attempt id"),
        ProjectSnapshotId::new("task075-history-snapshot").expect("snapshot id"),
        digest('3'),
    )
    .expect("MEMORY_HISTORICAL_INVOCATION_INVALID");
    let request = GraphMemoryRunRequest::new(
        invocation,
        ProjectId::new("task075-history-project").expect("project"),
        GitObjectId::new("3".repeat(40)).expect("commit"),
        digest_query_text(query_text).expect("query digest"),
        digest('7'),
        5,
    )
    .expect("MEMORY_HISTORICAL_REQUEST_INVALID");
    let source = TrackedSource::new("src/history.rs", digest('4')).expect("source");
    let snapshot = CodeSnapshotEvidence::new(
        &request,
        GitObjectId::new("4".repeat(40)).expect("tree"),
        vec![source.clone()],
        digest('5'),
        digest('6'),
    )
    .expect("MEMORY_HISTORICAL_SNAPSHOT_INVALID");
    let provenance = GraphSourceProvenance::new(&source, Some(1), Some(3)).expect("provenance");
    let raw = GraphifyRawEvidence::new(
        &request,
        &snapshot,
        GraphifyIdentity::task033(digest('1'), digest('2'), digest('3')).expect("identity"),
        vec![
            GraphifyRawNode::new(
                "node-task075-history-memory-port",
                query_text,
                "trait",
                provenance,
                GraphConfidence::Extracted,
            )
            .expect("node"),
        ],
        vec![],
        digest('7'),
        digest('8'),
        digest('9'),
    )
    .expect("MEMORY_HISTORICAL_RAW_GRAPH_INVALID");
    let analysis = normalize_analysis(&request, &snapshot, &raw)
        .expect("MEMORY_HISTORICAL_NORMALIZATION_FAILED");
    let query = MemoryQuery::new(&request, query_text, 5).expect("query");
    let plan = plan_retrieval(&analysis, &query).expect("MEMORY_HISTORICAL_PLAN_FAILED");
    assert_eq!(analysis.records().len(), 1);
    assert_eq!(plan.results().len(), 1);
    (analysis, plan)
}

fn graph_memory_fixture() -> (NormalizedGraphAnalysis, MemoryRetrievalPlan) {
    let query_text = "CodebaseMemoryPort";
    let invocation = Invocation::new(
        CONTRACT_VERSION,
        RequestId::new("task033-live-request").expect("request id"),
        TaskId::new("TASK-033").expect("task id"),
        AttemptId::new("task033-live-attempt").expect("attempt id"),
        ProjectSnapshotId::new("task033-live-snapshot").expect("snapshot id"),
        digest('a'),
    )
    .expect("MEMORY_LIVE_INVOCATION_INVALID");
    let request = GraphMemoryRunRequest::new(
        invocation,
        ProjectId::new("task033-live-project").expect("project"),
        GitObjectId::new("1".repeat(40)).expect("commit"),
        digest_query_text(query_text).expect("query digest"),
        digest('c'),
        5,
    )
    .expect("MEMORY_LIVE_REQUEST_INVALID");
    let source = TrackedSource::new("src/lib.rs", digest('d')).expect("source");
    let snapshot = CodeSnapshotEvidence::new(
        &request,
        GitObjectId::new("2".repeat(40)).expect("tree"),
        vec![source.clone()],
        digest('e'),
        digest('f'),
    )
    .expect("MEMORY_LIVE_SNAPSHOT_INVALID");
    let provenance = GraphSourceProvenance::new(&source, Some(1), Some(3)).expect("provenance");
    let raw = GraphifyRawEvidence::new(
        &request,
        &snapshot,
        GraphifyIdentity::task033(digest('1'), digest('2'), digest('3')).expect("identity"),
        vec![
            GraphifyRawNode::new(
                "node-codebase-memory-port",
                query_text,
                "trait",
                provenance,
                GraphConfidence::Extracted,
            )
            .expect("node"),
        ],
        vec![],
        digest('4'),
        digest('5'),
        digest('6'),
    )
    .expect("MEMORY_LIVE_RAW_GRAPH_INVALID");
    let analysis =
        normalize_analysis(&request, &snapshot, &raw).expect("MEMORY_LIVE_NORMALIZATION_FAILED");
    let query = MemoryQuery::new(&request, query_text, 5).expect("query");
    let plan = plan_retrieval(&analysis, &query).expect("MEMORY_LIVE_RETRIEVAL_PLAN_FAILED");
    assert_eq!(analysis.records().len(), 1);
    assert_eq!(plan.results().len(), 1);
    (analysis, plan)
}

fn digest(byte: char) -> ContentDigest {
    ContentDigest::from_sha256(byte.to_string().repeat(64)).expect("valid digest")
}

fn prove_partial_collision_and_transaction_rollback(config: &LiveConfig, target: &ExtensionTarget) {
    let mut migrator = config.role_client(ExtensionDatabaseRole::Migrator);
    migrator
        .batch_execute("CREATE TABLE memory.codebase_memory_extension_identity (probe integer)")
        .expect("MEMORY_EXTENSION_PARTIAL_FIXTURE_FAILED");
    assert_setup_kind(
        apply_extension(&mut migrator, target),
        ExtensionSetupErrorKind::PartialProfile,
    );
    assert_eq!(memory_object_counts(&mut migrator), (1, 0));
    migrator
        .batch_execute("DROP TABLE memory.codebase_memory_extension_identity")
        .expect("MEMORY_EXTENSION_PARTIAL_FIXTURE_CLEANUP_FAILED");

    migrator
        .batch_execute("CREATE TABLE memory.task033_foreign_collision (probe integer)")
        .expect("MEMORY_EXTENSION_COLLISION_FIXTURE_FAILED");
    assert_setup_kind(
        apply_extension(&mut migrator, target),
        ExtensionSetupErrorKind::SchemaCollision,
    );
    assert_eq!(memory_object_counts(&mut migrator), (1, 0));
    migrator
        .batch_execute("DROP TABLE memory.task033_foreign_collision")
        .expect("MEMORY_EXTENSION_COLLISION_FIXTURE_CLEANUP_FAILED");
    drop(migrator);

    let mut admin = config.admin_client();
    admin
        .batch_execute(
            "CREATE FUNCTION public.task033_memory_rollback_probe() \
                 RETURNS event_trigger LANGUAGE plpgsql AS $probe$ \
             BEGIN \
                 IF EXISTS ( \
                     SELECT 1 FROM pg_catalog.pg_event_trigger_ddl_commands() \
                      WHERE object_identity = 'memory.codebase_memory_receipts' \
                 ) THEN \
                     RAISE EXCEPTION USING ERRCODE = 'P0001', \
                         MESSAGE = 'fixed memory rollback probe'; \
                 END IF; \
             END; \
             $probe$; \
             REVOKE ALL ON FUNCTION public.task033_memory_rollback_probe() FROM PUBLIC; \
             CREATE EVENT TRIGGER task033_memory_rollback_probe \
                 ON ddl_command_end \
                 EXECUTE FUNCTION public.task033_memory_rollback_probe()",
        )
        .expect("MEMORY_EXTENSION_ROLLBACK_PROBE_CREATE_FAILED");
    drop(admin);

    let mut migrator = config.role_client(ExtensionDatabaseRole::Migrator);
    assert_setup_kind(
        apply_extension(&mut migrator, target),
        ExtensionSetupErrorKind::TransactionFailed,
    );
    assert_eq!(memory_object_counts(&mut migrator), (0, 0));
    drop(migrator);

    let mut admin = config.admin_client();
    admin
        .batch_execute(
            "DROP EVENT TRIGGER task033_memory_rollback_probe; \
             DROP FUNCTION public.task033_memory_rollback_probe()",
        )
        .expect("MEMORY_EXTENSION_ROLLBACK_PROBE_CLEANUP_FAILED");
}

fn memory_object_counts(client: &mut Client) -> (i64, i64) {
    let row = client
        .query_one(
            "SELECT \
                 (SELECT count(*) \
                    FROM pg_catalog.pg_class AS c \
                    JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace \
                   WHERE n.nspname = 'memory' \
                     AND c.relkind IN ('r', 'p', 'v', 'm', 'S', 'f')), \
                 (SELECT count(*) \
                    FROM pg_catalog.pg_proc AS p \
                    JOIN pg_catalog.pg_namespace AS n ON n.oid = p.pronamespace \
                   WHERE n.nspname = 'memory')",
            &[],
        )
        .expect("MEMORY_EXTENSION_OBJECT_COUNT_FAILED");
    (row.get(0), row.get(1))
}

fn extension_install_fingerprint(client: &mut Client) -> (String, String, String) {
    let row = client
        .query_one(
            "SELECT xmin::text, installed_at::text \
               FROM ONLY memory.codebase_memory_extension_identity \
              WHERE singleton",
            &[],
        )
        .unwrap_or_else(|error| {
            let marker = match error.code().map(SqlState::code) {
                Some("42703") => "MEMORY_EXTENSION_IDENTITY_FINGERPRINT_UNDEFINED_COLUMN",
                Some("42501") => "MEMORY_EXTENSION_IDENTITY_FINGERPRINT_PERMISSION_DENIED",
                Some("25P02") => "MEMORY_EXTENSION_IDENTITY_FINGERPRINT_TRANSACTION_ABORTED",
                Some("42P01") => "MEMORY_EXTENSION_IDENTITY_FINGERPRINT_TABLE_MISSING",
                _ => "MEMORY_EXTENSION_IDENTITY_FINGERPRINT_QUERY_FAILED",
            };
            panic!("{marker}")
        });
    let oids: String = client
        .query_one(
            "SELECT pg_catalog.string_agg(c.oid::text, ',' ORDER BY c.relname) \
               FROM pg_catalog.pg_class AS c \
               JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace \
              WHERE n.nspname = 'memory' \
                AND c.relkind IN ('r', 'p', 'v', 'm', 'S', 'f')",
            &[],
        )
        .expect("MEMORY_EXTENSION_OID_FINGERPRINT_FAILED")
        .get(0);
    (row.get(0), row.get(1), oids)
}

fn assert_setup_kind<T: std::fmt::Debug>(
    result: Result<T, lattice_postgres_codebase_memory::ExtensionSetupError>,
    expected: ExtensionSetupErrorKind,
) {
    let error = result.expect_err("MEMORY_EXTENSION_EXPECTED_FAILURE_MISSING");
    assert_eq!(error.kind(), expected, "{}", error.code());
}

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("MEMORY_EXTENSION_ENVIRONMENT_MISSING"))
}
