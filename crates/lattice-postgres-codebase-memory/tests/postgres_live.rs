use std::env;

use lattice_codebase_memory::{digest_query_text, normalize_analysis, plan_retrieval};
use lattice_contracts::{
    AttemptId, CONTRACT_VERSION, CodeSnapshotEvidence, ContentDigest, GitObjectId, GraphConfidence,
    GraphMemoryRunRequest, GraphSourceProvenance, GraphifyIdentity, GraphifyRawEvidence,
    GraphifyRawNode, Invocation, MemoryQuery, MemoryRetrievalPlan, NormalizedGraphAnalysis,
    ProjectId, ProjectSnapshotId, RequestId, TaskId, TrackedSource,
};
use lattice_ports::CodebaseMemoryPort;
use lattice_postgres_codebase_memory::{
    ExtensionApplyOutcome, ExtensionDatabaseRole, ExtensionSetupErrorKind, ExtensionTarget,
    PostgresCodebaseMemory, apply_extension, verify_extension,
};
use postgres::config::SslMode;
use postgres::error::SqlState;
use postgres::{Client, Config, NoTls};

const APPLICATION_NAME: &str = "lattice-devos-task019";

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
        let evidence = verify_extension(&mut migrator, &target, ExtensionDatabaseRole::Migrator)
            .unwrap_or_else(|error| panic!("{}", error.code()));
        assert_eq!(evidence.database_uuid(), target.expected_database_uuid());
        assert_eq!(
            evidence.identity().extension_id(),
            "lattice-codebase-memory"
        );
        let before_no_op = extension_install_fingerprint(&mut migrator);
        assert_eq!(
            apply_extension(&mut migrator, &target)
                .unwrap_or_else(|error| panic!("{}", error.code())),
            ExtensionApplyOutcome::AlreadyCurrent
        );
        assert_eq!(extension_install_fingerprint(&mut migrator), before_no_op);
        drop(migrator);

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
        exercise_runtime_memory(&config, &target, false);
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
        println!("MEMORY_EXTENSION_RESTART_PROFILE_OK");
    }
}

fn exercise_runtime_memory(config: &LiveConfig, target: &ExtensionTarget, restarted: bool) {
    let (analysis, plan) = graph_memory_fixture();
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

    let changed_request = GraphMemoryRunRequest::new(
        analysis.request().invocation().clone(),
        analysis.request().project_id().clone(),
        analysis.request().commit_id().clone(),
        analysis.request().query_digest().clone(),
        digest('9'),
        analysis.request().retrieval_limit(),
    )
    .expect("MEMORY_CHANGED_REQUEST_INVALID");
    assert!(
        memory.load_receipt(&changed_request).is_err(),
        "MEMORY_CHANGED_REQUEST_REPLAY_ALLOWED"
    );
    println!(
        "MEMORY_RUNTIME_REPLAY_OK phase={} receipt={} persistence={} database_identity={}",
        if restarted { "restart" } else { "initial" },
        receipt.receipt_digest().as_str(),
        receipt.persistence().persistence_digest().as_str(),
        receipt
            .persistence()
            .identity()
            .database_identity_digest()
            .as_str()
    );
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
