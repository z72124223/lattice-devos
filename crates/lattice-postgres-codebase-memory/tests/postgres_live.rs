use std::env;

use lattice_codebase_memory::{digest_query_text, normalize_analysis, plan_retrieval};
use lattice_contracts::{
    AttemptId, CONTRACT_VERSION, CodeSnapshotEvidence, ContentDigest, GatewayActorId,
    GatewayCommandId, GatewayCorrelationId, GatewayDenialCode, GatewayProjectStatusTarget,
    GatewayReply, GatewayReplyBody, GatewayRequest, GatewayRequestBody, GatewayStatusTarget,
    GitObjectId, GraphConfidence, GraphMemoryRunRequest, GraphSourceProvenance, GraphifyIdentity,
    GraphifyRawEvidence, GraphifyRawNode, HERMES_REFLECTION_SCHEMA_VERSION,
    HermesReflectionCandidate, HermesReflectionContent, HermesReflectionFinding,
    HermesReflectionReceipt, HermesReflectionStatus, Invocation, MemoryQuery, MemoryRetrievalPlan,
    NormalizedGraphAnalysis, ProjectId, ProjectSnapshotId, RequestId, TaskId, TrackedSource,
};
use lattice_gateway_ipc::{build_reply, build_request};
use lattice_ports::{
    CodebaseMemoryPort, HermesReflectionMemoryPort, OpenClawCommandScope,
    OpenClawIdempotencyDecision, OpenClawIdempotencyStore, OpenClawTerminalCommandRecord,
};
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
        prove_foreign_acl_rejected(&config, &target);
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
        prove_reflection_durability_boundary(&config);
        exercise_runtime_memory(&config, &target, false);
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
        exercise_openclaw_idempotency(&config, &target, true);
        println!("MEMORY_EXTENSION_RESTART_PROFILE_OK");
    }
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
               FROM memory.codebase_memory_persist_reflection_v2(\
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
