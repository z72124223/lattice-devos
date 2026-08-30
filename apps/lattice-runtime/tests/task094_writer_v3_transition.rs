//! TASK-094 composition-root PostgreSQL transition proof.
//!
//! Store and Writer remain independently owned adapters. This runtime test is the
//! only place that composes their public APIs for the disposable live transition.

use std::env;
use std::time::Duration;

use lattice_contracts::{
    ContentDigest, DaemonEpoch, ProjectId, ProjectSnapshotId, RuntimeAdmissionMode, RuntimeKind,
    StoreAuthorityHead, StoreAuthorityRevision, StoreDaemonInstanceId, TaskId,
    TaskLedgerStreamIdentity,
};
use lattice_postgres_store::{
    DatabaseRole, MigrationApplyOutcome, MigrationTarget, PostgresStoreSetupErrorKind,
    PostgresTaskLedger, PostgresTaskLedgerErrorKind, apply_migrations, verify_postgres_schema,
};
use lattice_postgres_writer_lease::{
    ExtensionApplyOutcome, ExtensionTarget, V3BootstrapProfile, V3ExtensionTarget,
    V4ExtensionTarget, apply_extension, apply_v3_extension, apply_v4_extension,
    inspect_v3_bootstrap_profile,
};
use lattice_task_ledger::{
    ActorId, AppendCommand, CommandId, CorrelationId, ReasonCode, TaskIngressClaim, VerifiedStream,
};
use postgres::config::SslMode;
use postgres::error::SqlState;
use postgres::{Client, Config, IsolationLevel, NoTls};

const REQUIRED_APPLICATION_NAME: &str = "lattice-devos-task019";
const HARNESS_ROLE: &str = "task019_harness";
const TASK_SUBMIT_INGRESS_ID: &str = "lattice_task_submit.v1";
const DUPLICATE_HISTORY_CLIENT_REQUEST_ID: &str = "delivery-run-controlled-compatibility";
const F252_V7_MANIFEST_SHA256: &str =
    "7e16a8eb119cf4db9910645cabffef8b99703b7dca8ed5e4a9e193fedcd8d44c";
const CURRENT_V5_MANIFEST_SHA256: &str =
    "f92a51fa19c4fe0ffebfc40f20924bd1209bb2441b1bc69f787bc3c4a925425d";
const CODEBASE_MEMORY_V2_PATH: &str = "db/extensions/codebase-memory/v2.sql";
const CODEBASE_MEMORY_V2_SQL_SHA256: &str =
    "9db54342b88f554ca76054c7a33ae72f04b412d2dfe21fae6eb4d8faf3e854e2";
const CODEBASE_MEMORY_V2_MANIFEST_SHA256: &str =
    "0aedbd7d9ef7ca07fc2910d0da34c163cc83e3dd56f9b28292ae1f4f0c3c4d7e";
const CODEBASE_MEMORY_V3_PATH: &str = "db/extensions/codebase-memory/v3.sql";
const CODEBASE_MEMORY_V3_SQL_SHA256: &str =
    "7388f6bfe4c2d30a20306e4f9ebdff5862125bcab58f769ba286af542cb051c3";
const CODEBASE_MEMORY_V3_MANIFEST_SHA256: &str =
    "d4cc712d262ae1f7c96bd65526eab611c90e193363afd865af2126307b2903f0";
const CODEBASE_MEMORY_V2_SQL: &str = include_str!("../../../db/extensions/codebase-memory/v2.sql");
const CODEBASE_MEMORY_V3_SQL: &str = include_str!("../../../db/extensions/codebase-memory/v3.sql");

#[derive(Clone)]
struct HistoricalDuplicateFixture {
    claims: [TaskIngressClaim; 2],
    identities: [TaskLedgerStreamIdentity; 2],
    canonical_streams: [VerifiedStream; 2],
    ledger_fingerprint: Vec<String>,
}

#[derive(Clone)]
struct LiveConfig {
    host: String,
    port: u16,
    password: String,
    run_id: String,
}

impl LiveConfig {
    fn from_environment() -> Option<Self> {
        if env::var("LATTICE_TASK019_LIVE").ok().as_deref() != Some("1") {
            return None;
        }
        let host = required_environment("LATTICE_TASK019_HOST");
        let port = required_environment("LATTICE_TASK019_PORT")
            .parse::<u16>()
            .unwrap_or_else(|_| panic!("TASK094_LIVE_PORT_INVALID"));
        let password = required_environment("LATTICE_TASK019_PASSWORD");
        let run_id = required_environment("LATTICE_TASK019_RUN_ID");
        assert_eq!(
            required_environment("LATTICE_TASK019_PHASE"),
            "task094_transition",
            "TASK094_LIVE_PHASE_INVALID"
        );
        assert_eq!(host, "127.0.0.1", "TASK094_LIVE_HOST_INVALID");
        assert!(
            port != 0 && port != 5432 && port != 58_743,
            "TASK094_LIVE_PORT_INVALID"
        );
        assert!(!password.is_empty(), "TASK094_LIVE_PASSWORD_MISSING");
        assert!(is_lower_hex(&run_id, 32), "TASK094_LIVE_RUN_ID_INVALID");
        Some(Self {
            host,
            port,
            password,
            run_id,
        })
    }

    fn connect(&self, database: &str, application_name: &str) -> Client {
        let mut config = Config::new();
        config
            .host(&self.host)
            .port(self.port)
            .user(HARNESS_ROLE)
            .password(&self.password)
            .dbname(database)
            .application_name(application_name)
            .ssl_mode(SslMode::Disable)
            .connect_timeout(Duration::from_secs(10));
        config
            .connect(NoTls)
            .unwrap_or_else(|_| panic!("TASK094_LIVE_CONNECT_FAILED"))
    }

    fn role_client(&self, database: &str, role: DatabaseRole, application_name: &str) -> Client {
        let mut client = self.connect_as(database, role.login_role(), application_name);
        client
            .batch_execute(set_role_sql(role))
            .unwrap_or_else(|_| panic!("TASK094_SET_ROLE_FAILED"));
        client
    }

    fn connect_as(&self, database: &str, user: &str, application_name: &str) -> Client {
        let mut config = Config::new();
        config
            .host(&self.host)
            .port(self.port)
            .user(user)
            .password(&self.password)
            .dbname(database)
            .application_name(application_name)
            .ssl_mode(SslMode::Disable)
            .connect_timeout(Duration::from_secs(10));
        config
            .connect(NoTls)
            .unwrap_or_else(|_| panic!("TASK094_LIVE_CONNECT_FAILED"))
    }

    fn target(&self) -> MigrationTarget {
        MigrationTarget::new(
            format!("lattice_task019_{}_transition", &self.run_id[..8]),
            self.run_id.clone(),
        )
        .unwrap_or_else(|_| panic!("TASK094_TARGET_CONSTRUCTION_FAILED"))
    }
}

fn task094_digest(value: char) -> ContentDigest {
    ContentDigest::from_sha256(value.to_string().repeat(64)).expect("TASK094_DIGEST")
}

fn task094_digest_bytes(value: &ContentDigest) -> Vec<u8> {
    value
        .as_str()
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let text = std::str::from_utf8(pair).expect("TASK094_HEX_UTF8");
            u8::from_str_radix(text, 16).expect("TASK094_HEX_BYTE")
        })
        .collect()
}

fn task094_store_authority() -> StoreAuthorityHead {
    StoreAuthorityHead::new(
        RuntimeKind::Live,
        StoreDaemonInstanceId::new("task094-historical-canary").expect("TASK094_DAEMON"),
        DaemonEpoch::new(94).expect("TASK094_EPOCH"),
        RuntimeAdmissionMode::Active,
        StoreAuthorityRevision::new(94).expect("TASK094_REVISION"),
        task094_digest('a'),
        task094_digest('b'),
    )
    .expect("TASK094_AUTHORITY")
}

fn set_task094_runtime_admission(client: &mut Client, active: bool) {
    if active {
        let authority = task094_store_authority();
        let epoch = i64::try_from(authority.daemon_epoch().get()).expect("TASK094_EPOCH_I64");
        let revision = i64::try_from(authority.revision().get()).expect("TASK094_REVISION_I64");
        assert_eq!(
            client
                .execute(
                    "UPDATE ONLY control.runtime_admission SET admission_mode='ACTIVE', \
                     daemon_instance_id=$1, daemon_epoch=$2, authority_revision=$3, \
                     observation_digest=$4::bytea, authority_head_digest=$5::bytea, \
                     updated_at=pg_catalog.clock_timestamp() WHERE singleton",
                    &[
                        &authority.daemon_instance_id().as_str(),
                        &epoch,
                        &revision,
                        &task094_digest_bytes(authority.observation_digest()),
                        &task094_digest_bytes(authority.head_digest()),
                    ],
                )
                .expect("TASK094_ACTIVATE_RUNTIME"),
            1
        );
    } else {
        assert_eq!(
            client
                .execute(
                    "UPDATE ONLY control.runtime_admission SET admission_mode='STOPPED', \
                     daemon_instance_id=NULL, daemon_epoch=NULL, authority_revision=0, \
                     observation_digest=NULL, authority_head_digest=NULL, \
                     updated_at=pg_catalog.clock_timestamp() WHERE singleton",
                    &[],
                )
                .expect("TASK094_STOP_RUNTIME"),
            1
        );
    }
}

fn persist_historical_canary_before_v4(
    config: &LiveConfig,
    target: &MigrationTarget,
    migrator: &mut Client,
) -> TaskIngressClaim {
    let retained_writer = migrator
        .query_one(
            "SELECT extension_schema_version, extension_sql_sha256::text, \
                    extension_manifest_sha256::text, global_schema_version \
               FROM ONLY writer_lease.writer_lease_extension_identity WHERE singleton",
            &[],
        )
        .expect("TASK094_RETAINED_WRITER_V3_IDENTITY");
    assert_eq!(retained_writer.get::<_, i16>(0), 3);
    assert_eq!(
        retained_writer.get::<_, String>(1),
        "677c010a61e5945bcc6b96ca9f3d9e57830dc42f4cfbd46ea76d5e9d8b9262a0"
    );
    assert_eq!(
        retained_writer.get::<_, String>(2),
        "eab2812fa3d94cd3466d7c003386f805a973fd7def1f16aeb15b52f47dad78e4"
    );
    assert_eq!(retained_writer.get::<_, i16>(3), 6);

    set_task094_runtime_admission(migrator, true);
    let client_request_id = "task094-historical-canary";
    let identity = TaskLedgerStreamIdentity::new(
        ProjectId::new("task094-historical-project").expect("TASK094_PROJECT"),
        ProjectSnapshotId::new("task094-historical-snapshot").expect("TASK094_SNAPSHOT"),
        TaskId::new("TASK-094-HISTORICAL-CANARY").expect("TASK094_TASK"),
        "1",
        task094_digest('7'),
        "TWD",
    )
    .expect("TASK094_IDENTITY");
    let vacant = lattice_task_ledger::VerifiedStream::vacant(identity, RuntimeKind::Live)
        .expect("TASK094_VACANT_STREAM");
    let command = AppendCommand::new_autonomy_required_task_created(
        vacant.head().clone(),
        CommandId::new(format!("mcp-submit:{client_request_id}")).expect("TASK094_COMMAND_ID"),
        CorrelationId::new("task094-historical-canary").expect("TASK094_CORRELATION"),
        "2026-08-26T00:00:00Z",
        ActorId::new("lattice-runtime").expect("TASK094_ACTOR"),
        ReasonCode::new("TASK038_TASK_ACCEPTED").expect("TASK094_REASON"),
        task094_digest('8'),
        None,
    )
    .expect("TASK094_HISTORICAL_COMMAND");
    let claim = TaskIngressClaim::controlled_canary(
        TASK_SUBMIT_INGRESS_ID,
        client_request_id,
        vacant.head().stream_id().clone(),
    )
    .expect("TASK094_HISTORICAL_CLAIM");
    let mut ledger = PostgresTaskLedger::new(
        config.role_client(
            target.database_name(),
            DatabaseRole::Runtime,
            REQUIRED_APPLICATION_NAME,
        ),
        target,
    )
    .expect("TASK094_V6_LEDGER");
    ledger
        .execute(command, task094_store_authority())
        .expect("TASK094_PERSIST_HISTORICAL_CANARY");
    drop(ledger);

    set_task094_runtime_admission(migrator, false);
    claim
}

fn persist_historical_duplicate_canaries_before_v4(
    config: &LiveConfig,
    target: &MigrationTarget,
    migrator: &mut Client,
) -> HistoricalDuplicateFixture {
    set_task094_runtime_admission(migrator, true);
    let mut ledger = PostgresTaskLedger::new(
        config.role_client(
            target.database_name(),
            DatabaseRole::Runtime,
            REQUIRED_APPLICATION_NAME,
        ),
        target,
    )
    .expect("TASK094_V6_DUPLICATE_LEDGER");
    let mut claims = Vec::with_capacity(2);
    let mut identities = Vec::with_capacity(2);
    for (task_id, task_spec, subject, correlation, occurred_at) in [
        (
            "TASK-094-DUPLICATE-A",
            '9',
            'c',
            "task094-duplicate-history-a",
            "2026-08-26T00:00:01Z",
        ),
        (
            "TASK-094-DUPLICATE-B",
            'd',
            'e',
            "task094-duplicate-history-b",
            "2026-08-26T00:00:02Z",
        ),
    ] {
        let identity = TaskLedgerStreamIdentity::new(
            ProjectId::new("task094-historical-project").expect("TASK094_DUPLICATE_PROJECT"),
            ProjectSnapshotId::new("task094-historical-snapshot")
                .expect("TASK094_DUPLICATE_SNAPSHOT"),
            TaskId::new(task_id).expect("TASK094_DUPLICATE_TASK"),
            "1",
            task094_digest(task_spec),
            "TWD",
        )
        .expect("TASK094_DUPLICATE_IDENTITY");
        let vacant =
            lattice_task_ledger::VerifiedStream::vacant(identity.clone(), RuntimeKind::Live)
                .expect("TASK094_DUPLICATE_VACANT_STREAM");
        let claim = TaskIngressClaim::controlled_canary(
            TASK_SUBMIT_INGRESS_ID,
            DUPLICATE_HISTORY_CLIENT_REQUEST_ID,
            vacant.head().stream_id().clone(),
        )
        .expect("TASK094_DUPLICATE_CLAIM");
        let command = AppendCommand::new_autonomy_required_task_created(
            vacant.head().clone(),
            CommandId::new(format!("mcp-submit:{DUPLICATE_HISTORY_CLIENT_REQUEST_ID}"))
                .expect("TASK094_DUPLICATE_COMMAND_ID"),
            CorrelationId::new(correlation).expect("TASK094_DUPLICATE_CORRELATION"),
            occurred_at,
            ActorId::new("lattice-runtime").expect("TASK094_DUPLICATE_ACTOR"),
            ReasonCode::new("TASK038_TASK_ACCEPTED").expect("TASK094_DUPLICATE_REASON"),
            task094_digest(subject),
            None,
        )
        .expect("TASK094_DUPLICATE_COMMAND");
        ledger
            .execute(command, task094_store_authority())
            .expect("TASK094_PERSIST_DUPLICATE_HISTORY");
        claims.push(claim);
        identities.push(identity);
    }
    let claims: [TaskIngressClaim; 2] = claims
        .try_into()
        .unwrap_or_else(|_| panic!("TASK094_DUPLICATE_CLAIM_CARDINALITY"));
    let identities: [TaskLedgerStreamIdentity; 2] = identities
        .try_into()
        .unwrap_or_else(|_| panic!("TASK094_DUPLICATE_IDENTITY_CARDINALITY"));
    assert_ne!(claims[0].stream_id(), claims[1].stream_id());
    let mut canonical_streams = Vec::with_capacity(2);
    for (index, identity) in identities.iter().enumerate() {
        let loaded = ledger
            .load_stream(identity.clone())
            .unwrap_or_else(|_| panic!("TASK094_DUPLICATE_V6_REPLAY_{index}"));
        assert_eq!(loaded.stream().identity(), identity);
        assert_eq!(
            loaded.stream().head().stream_id(),
            claims[index].stream_id()
        );
        assert_eq!(loaded.stream().events().len(), 1);
        assert_eq!(loaded.stream().commands().len(), 1);
        assert_eq!(loaded.retained_checkpoint(), loaded.stream().checkpoint());
        canonical_streams.push(loaded.stream().clone());
    }
    let canonical_streams: [VerifiedStream; 2] = canonical_streams
        .try_into()
        .unwrap_or_else(|_| panic!("TASK094_DUPLICATE_STREAM_CARDINALITY"));
    drop(ledger);
    set_task094_runtime_admission(migrator, false);
    let ledger_fingerprint = historical_duplicate_ledger_fingerprint(migrator, &claims);
    println!("TASK094_DUPLICATE_HISTORY_V6_CANONICAL_REPLAY_PASS");
    HistoricalDuplicateFixture {
        claims,
        identities,
        canonical_streams,
        ledger_fingerprint,
    }
}

fn prove_historical_duplicate_replay_v7(
    config: &LiveConfig,
    target: &MigrationTarget,
    migrator: &mut Client,
    fixture: &HistoricalDuplicateFixture,
) {
    set_task094_runtime_admission(migrator, true);
    let mut ledger = PostgresTaskLedger::new(
        config.role_client(
            target.database_name(),
            DatabaseRole::Runtime,
            REQUIRED_APPLICATION_NAME,
        ),
        target,
    )
    .expect("TASK094_V7_DUPLICATE_REPLAY_LEDGER");
    for (index, identity) in fixture.identities.iter().enumerate() {
        let loaded = ledger
            .load_stream(identity.clone())
            .unwrap_or_else(|_| panic!("TASK094_DUPLICATE_V7_REPLAY_{index}"));
        assert_eq!(loaded.stream().identity(), identity);
        assert_eq!(
            loaded.stream().head().stream_id(),
            fixture.claims[index].stream_id()
        );
        assert_eq!(loaded.stream().events().len(), 1);
        assert_eq!(loaded.stream().commands().len(), 1);
        assert_eq!(loaded.retained_checkpoint(), loaded.stream().checkpoint());
        assert_eq!(loaded.stream(), &fixture.canonical_streams[index]);
    }
    drop(ledger);
    set_task094_runtime_admission(migrator, false);
    println!("TASK094_DUPLICATE_HISTORY_V7_CANONICAL_REPLAY_PASS");
}

fn historical_duplicate_ledger_fingerprint(
    client: &mut Client,
    claims: &[TaskIngressClaim; 2],
) -> Vec<String> {
    let first = task094_digest_bytes(claims[0].stream_id());
    let second = task094_digest_bytes(claims[1].stream_id());
    [
        "SELECT pg_catalog.count(*)::text || ':' || pg_catalog.md5(COALESCE(\
             pg_catalog.string_agg((pg_catalog.to_jsonb(t) \
                 - 'task_subject_kind' - 'task_subject_digest')::text,E'\\n' \
                 ORDER BY pg_catalog.encode(t.stream_id,'hex')),'')) \
           FROM ONLY control.task_ledger_streams t \
          WHERE t.stream_id IN ($1::bytea,$2::bytea)",
        "SELECT pg_catalog.count(*)::text || ':' || pg_catalog.md5(COALESCE(\
             pg_catalog.string_agg(pg_catalog.to_jsonb(t)::text,E'\\n' \
                 ORDER BY pg_catalog.encode(t.stream_id,'hex'),t.sequence),'')) \
           FROM ONLY control.task_ledger_events t \
          WHERE t.stream_id IN ($1::bytea,$2::bytea)",
        "SELECT pg_catalog.count(*)::text || ':' || pg_catalog.md5(COALESCE(\
             pg_catalog.string_agg(pg_catalog.to_jsonb(t)::text,E'\\n' \
                 ORDER BY pg_catalog.encode(t.stream_id,'hex'),t.command_id),'')) \
           FROM ONLY control.task_ledger_commands t \
          WHERE t.stream_id IN ($1::bytea,$2::bytea)",
        "SELECT pg_catalog.count(*)::text || ':' || pg_catalog.md5(COALESCE(\
             pg_catalog.string_agg(pg_catalog.to_jsonb(t)::text,E'\\n' \
                 ORDER BY pg_catalog.encode(t.admission_digest,'hex')),'')) \
           FROM ONLY control.task_ledger_outbox t \
          WHERE t.stream_id IN ($1::bytea,$2::bytea)",
        "SELECT pg_catalog.count(*)::text || ':' || pg_catalog.md5(COALESCE(\
             pg_catalog.string_agg(pg_catalog.to_jsonb(t)::text,E'\\n' \
                 ORDER BY t.transaction_id),'')) \
           FROM ONLY control.terminal_transactions t \
          WHERE t.transaction_id IN (\
              SELECT c.store_transaction_id FROM ONLY control.task_ledger_commands c \
               WHERE c.stream_id IN ($1::bytea,$2::bytea))",
    ]
    .into_iter()
    .map(|query| {
        client
            .query_one(query, &[&first, &second])
            .expect("TASK094_DUPLICATE_LEDGER_FINGERPRINT_QUERY")
            .get(0)
    })
    .collect()
}

fn prove_historical_canary_claim_backfill(
    config: &LiveConfig,
    target: &MigrationTarget,
    migrator: &mut Client,
    claim: &TaskIngressClaim,
) {
    set_task094_runtime_admission(migrator, true);
    let mut ledger = PostgresTaskLedger::new(
        config.role_client(
            target.database_name(),
            DatabaseRole::Runtime,
            REQUIRED_APPLICATION_NAME,
        ),
        target,
    )
    .expect("TASK094_V7_LEDGER");
    let loaded = ledger
        .load_ingress_claim_by_request(TASK_SUBMIT_INGRESS_ID, claim.client_request_id())
        .expect("TASK094_LOAD_HISTORICAL_CLAIM")
        .expect("TASK094_HISTORICAL_CLAIM_PRESENT");
    assert_eq!(&loaded, claim);
    println!("TASK094_HISTORICAL_CANARY_BACKFILL_PASS");
}

fn assert_historical_ambiguity_database_error(error: &postgres::Error, label: &str) {
    let database = error
        .as_db_error()
        .unwrap_or_else(|| panic!("TASK094_{label}_DATABASE_ERROR_REQUIRED"));
    assert_eq!(database.code().code(), "LTX01", "TASK094_{label}_SQLSTATE");
    assert_eq!(
        database.message(),
        "LATTICE_TASK_INGRESS_HISTORICAL_AMBIGUOUS",
        "TASK094_{label}_STATIC_DIAGNOSTIC"
    );
}

fn prove_historical_duplicate_migration(
    config: &LiveConfig,
    target: &MigrationTarget,
    migrator: &mut Client,
    fixture: &HistoricalDuplicateFixture,
) {
    assert_eq!(
        historical_duplicate_ledger_fingerprint(migrator, &fixture.claims),
        fixture.ledger_fingerprint,
        "TASK094_DUPLICATE_V6_IDENTITY_COLUMNS_MUST_REMAIN_EXACT"
    );
    let claim_count: i64 = migrator
        .query_one(
            "SELECT count(*)::bigint FROM ONLY control.task_ingress_claims \
              WHERE ingress_id=$1 AND client_request_id=$2",
            &[
                &TASK_SUBMIT_INGRESS_ID,
                &DUPLICATE_HISTORY_CLIENT_REQUEST_ID,
            ],
        )
        .expect("TASK094_DUPLICATE_ACTIVE_CLAIM_COUNT")
        .get(0);
    assert_eq!(
        claim_count, 0,
        "TASK094_DUPLICATE_HISTORY_MUST_NOT_SELECT_WINNER"
    );
    let rows = migrator
        .query(
            "SELECT a.stream_id, \
                    a.schema_version='lattice.task-ledger.task-ingress-historical-ambiguity/1.0' \
                    AND a.request_kind='CONTROLLED_CODEX_CANARY' \
                    AND a.ingress_request_digest=a.stream_id \
                    AND a.event_sequence=1 \
                    AND a.command_id='mcp-submit:' || a.client_request_id \
                    AND a.stream_id=e.stream_id AND a.event_sequence=e.sequence \
                    AND a.event_digest=e.event_digest AND a.command_id=e.command_id \
                    AND a.command_request_digest=e.request_digest \
                    AND a.stream_id=c.stream_id AND a.command_id=c.command_id \
                    AND a.command_request_digest=c.request_digest AS exact_linkage \
               FROM ONLY control.task_ingress_historical_ambiguities a \
               JOIN ONLY control.task_ledger_events e \
                 ON e.stream_id=a.stream_id AND e.sequence=a.event_sequence \
               JOIN ONLY control.task_ledger_commands c \
                 ON c.stream_id=a.stream_id AND c.command_id=a.command_id \
              WHERE a.ingress_id=$1 AND a.client_request_id=$2 \
              ORDER BY a.stream_id",
            &[
                &TASK_SUBMIT_INGRESS_ID,
                &DUPLICATE_HISTORY_CLIENT_REQUEST_ID,
            ],
        )
        .expect("TASK094_DUPLICATE_AMBIGUITY_ROWS");
    assert_eq!(rows.len(), 2, "TASK094_DUPLICATE_AMBIGUITY_CARDINALITY");
    assert!(
        rows.iter().all(|row| row.get::<_, bool>(1)),
        "TASK094_DUPLICATE_AMBIGUITY_LINKAGE_MUST_BE_EXACT"
    );
    let mut retained_streams = rows
        .iter()
        .map(|row| row.get::<_, Vec<u8>>(0))
        .collect::<Vec<_>>();
    let mut expected_streams = fixture
        .claims
        .iter()
        .map(|claim| task094_digest_bytes(claim.stream_id()))
        .collect::<Vec<_>>();
    retained_streams.sort();
    expected_streams.sort();
    assert_eq!(retained_streams, expected_streams);

    let acl_closed: bool = migrator
        .query_one(
            "SELECT NOT pg_catalog.has_table_privilege('lattice_runtime', \
                       'control.task_ingress_historical_ambiguities', \
                       'SELECT,INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER') \
                    AND NOT pg_catalog.has_table_privilege('lattice_runtime_login', \
                       'control.task_ingress_historical_ambiguities', \
                       'SELECT,INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER') \
                    AND NOT pg_catalog.has_table_privilege('lattice_guardian', \
                       'control.task_ingress_historical_ambiguities', \
                       'SELECT,INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER') \
                    AND NOT pg_catalog.has_table_privilege('lattice_readonly', \
                       'control.task_ingress_historical_ambiguities', \
                       'SELECT,INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER')",
            &[],
        )
        .expect("TASK094_DUPLICATE_AMBIGUITY_ACL")
        .get(0);
    assert!(
        acl_closed,
        "TASK094_DUPLICATE_AMBIGUITY_TABLE_MUST_BE_RUNTIME_BLIND"
    );

    set_task094_runtime_admission(migrator, true);
    let mut ledger = PostgresTaskLedger::new(
        config.role_client(
            target.database_name(),
            DatabaseRole::Runtime,
            REQUIRED_APPLICATION_NAME,
        ),
        target,
    )
    .expect("TASK094_V7_DUPLICATE_LEDGER");
    let failure = ledger
        .load_ingress_claim_by_request(TASK_SUBMIT_INGRESS_ID, DUPLICATE_HISTORY_CLIENT_REQUEST_ID)
        .expect_err("TASK094_DUPLICATE_HISTORY_READ_MUST_FAIL_CLOSED");
    assert_eq!(
        failure.kind(),
        PostgresTaskLedgerErrorKind::CommandSubstitution
    );
    assert_eq!(
        failure.kind().code(),
        "POSTGRES_TASK_LEDGER_COMMAND_SUBSTITUTED"
    );
    drop(ledger);

    let ambiguity = migrator
        .query_one(
            "SELECT ingress_request_digest,stream_id,event_sequence::text,event_digest, \
                    command_id::text,command_request_digest \
               FROM ONLY control.task_ingress_historical_ambiguities \
              WHERE ingress_id=$1 AND client_request_id=$2 \
              ORDER BY stream_id LIMIT 1",
            &[
                &TASK_SUBMIT_INGRESS_ID,
                &DUPLICATE_HISTORY_CLIENT_REQUEST_ID,
            ],
        )
        .expect("TASK094_DUPLICATE_AMBIGUITY_CALL_ARGUMENTS");
    let ingress_request_digest = ambiguity.get::<_, Vec<u8>>(0);
    let stream_id = ambiguity.get::<_, Vec<u8>>(1);
    let event_sequence = ambiguity.get::<_, String>(2);
    let event_digest = ambiguity.get::<_, Vec<u8>>(3);
    let command_id = ambiguity.get::<_, String>(4);
    let command_request_digest = ambiguity.get::<_, Vec<u8>>(5);
    let mut runtime = config.role_client(
        target.database_name(),
        DatabaseRole::Runtime,
        REQUIRED_APPLICATION_NAME,
    );
    {
        let mut transaction = runtime
            .build_transaction()
            .isolation_level(IsolationLevel::Serializable)
            .start()
            .expect("TASK094_DUPLICATE_PREPARE_TRANSACTION");
        transaction
            .batch_execute("SET LOCAL synchronous_commit=on; SET LOCAL search_path=pg_catalog")
            .expect("TASK094_DUPLICATE_PREPARE_SETTINGS");
        let failure = transaction
            .query(
                "SELECT * FROM control.task_ingress_prepare_v1($1,$2,$3,$4::bytea,$5::bytea)",
                &[
                    &TASK_SUBMIT_INGRESS_ID,
                    &DUPLICATE_HISTORY_CLIENT_REQUEST_ID,
                    &"CONTROLLED_CODEX_CANARY",
                    &ingress_request_digest,
                    &stream_id,
                ],
            )
            .expect_err("TASK094_DUPLICATE_PREPARE_MUST_FAIL_CLOSED");
        assert_historical_ambiguity_database_error(&failure, "DUPLICATE_PREPARE");
    }
    {
        let mut transaction = runtime
            .build_transaction()
            .isolation_level(IsolationLevel::Serializable)
            .start()
            .expect("TASK094_DUPLICATE_RECORD_TRANSACTION");
        transaction
            .batch_execute("SET LOCAL synchronous_commit=on; SET LOCAL search_path=pg_catalog")
            .expect("TASK094_DUPLICATE_RECORD_SETTINGS");
        let failure = transaction
            .query_one(
                "SELECT control.task_ingress_record_v1($1,$2,$3,$4,$5::bytea,$6::bytea,$7,$8::bytea,$9,$10::bytea)",
                &[
                    &"lattice.task-ledger.task-ingress-claim/1.0",
                    &TASK_SUBMIT_INGRESS_ID,
                    &DUPLICATE_HISTORY_CLIENT_REQUEST_ID,
                    &"CONTROLLED_CODEX_CANARY",
                    &ingress_request_digest,
                    &stream_id,
                    &event_sequence,
                    &event_digest,
                    &command_id,
                    &command_request_digest,
                ],
            )
            .expect_err("TASK094_DUPLICATE_RECORD_MUST_FAIL_CLOSED");
        assert_historical_ambiguity_database_error(&failure, "DUPLICATE_RECORD");
    }
    set_task094_runtime_admission(migrator, false);
    println!("TASK094_DUPLICATE_HISTORY_AMBIGUITY_PASS");
}

fn v7_duplicate_migration_fingerprint(
    client: &mut Client,
    fixture: &HistoricalDuplicateFixture,
) -> Vec<String> {
    let mut fingerprint = v5_bridge_fingerprint(client);
    fingerprint.extend(historical_duplicate_ledger_fingerprint(
        client,
        &fixture.claims,
    ));
    for query in [
        "SELECT pg_catalog.count(*)::text || ':' || pg_catalog.md5(COALESCE(\
             pg_catalog.string_agg(pg_catalog.to_jsonb(t)::text,E'\\n' \
               ORDER BY t.ingress_id,t.client_request_id,t.stream_id),'')) \
           FROM ONLY control.task_ingress_historical_ambiguities t",
        "SELECT pg_catalog.count(*)::text || ':' || pg_catalog.md5(COALESCE(\
             pg_catalog.string_agg(pg_catalog.to_jsonb(t)::text,E'\\n' \
               ORDER BY t.ingress_id,t.client_request_id),'')) \
           FROM ONLY control.task_ingress_claims t",
    ] {
        fingerprint.push(
            client
                .query_one(query, &[])
                .expect("TASK094_V7_DUPLICATE_FINGERPRINT_QUERY")
                .get(0),
        );
    }
    fingerprint
}

fn assert_fresh_v7_verifier_rejects(
    config: &LiveConfig,
    target: &MigrationTarget,
    expected_kind: PostgresStoreSetupErrorKind,
    label: &str,
) {
    let mut verifier = config.role_client(
        target.database_name(),
        DatabaseRole::Migrator,
        REQUIRED_APPLICATION_NAME,
    );
    let failure = match verify_postgres_schema(&mut verifier, target, DatabaseRole::Migrator) {
        Ok(_) => panic!("TASK094_{label}_DRIFT_MUST_FAIL"),
        Err(failure) => failure,
    };
    assert_eq!(failure.kind(), expected_kind, "TASK094_{label}_ERROR_KIND");
}

fn assert_fresh_store_verifier_passes(
    config: &LiveConfig,
    target: &MigrationTarget,
    expected_schema_version: u16,
    label: &str,
) {
    let mut verifier = config.role_client(
        target.database_name(),
        DatabaseRole::Migrator,
        REQUIRED_APPLICATION_NAME,
    );
    let evidence = verify_postgres_schema(&mut verifier, target, DatabaseRole::Migrator)
        .unwrap_or_else(|_| panic!("TASK094_{label}_REPAIR_VERIFY"));
    assert_eq!(evidence.schema_version(), expected_schema_version);
}

fn assert_fresh_v7_verifier_passes(config: &LiveConfig, target: &MigrationTarget, label: &str) {
    assert_fresh_store_verifier_passes(config, target, 7, label);
}

fn prove_store_catalog_drift_case(
    config: &LiveConfig,
    target: &MigrationTarget,
    migrator: &mut Client,
    label: &str,
    apply_sql: &str,
    repair_sql: &str,
    expected_kind: PostgresStoreSetupErrorKind,
    expected_schema_version: u16,
) {
    migrator
        .batch_execute(apply_sql)
        .unwrap_or_else(|_| panic!("TASK094_{label}_DRIFT_APPLY"));
    let failure = {
        let mut verifier = config.role_client(
            target.database_name(),
            DatabaseRole::Migrator,
            REQUIRED_APPLICATION_NAME,
        );
        verify_postgres_schema(&mut verifier, target, DatabaseRole::Migrator).err()
    };
    migrator
        .batch_execute(repair_sql)
        .unwrap_or_else(|_| panic!("TASK094_{label}_DRIFT_REPAIR"));
    let failure = failure.unwrap_or_else(|| panic!("TASK094_{label}_DRIFT_MUST_FAIL"));
    assert_eq!(failure.kind(), expected_kind, "TASK094_{label}_ERROR_KIND");
    assert_fresh_store_verifier_passes(config, target, expected_schema_version, label);
    println!("TASK094_V{expected_schema_version}_{label}_DRIFT_REJECTED_AND_REPAIRED");
}

fn prove_v7_catalog_drift_case(
    config: &LiveConfig,
    target: &MigrationTarget,
    migrator: &mut Client,
    label: &str,
    apply_sql: &str,
    repair_sql: &str,
    expected_kind: PostgresStoreSetupErrorKind,
) {
    prove_store_catalog_drift_case(
        config,
        target,
        migrator,
        label,
        apply_sql,
        repair_sql,
        expected_kind,
        7,
    );
}

fn prove_v7_admin_catalog_drift_case(
    config: &LiveConfig,
    target: &MigrationTarget,
    label: &str,
    apply_sql: &str,
    repair_sql: &str,
    expected_kind: PostgresStoreSetupErrorKind,
) {
    {
        let mut admin = config.connect(
            target.database_name(),
            "lattice-devos-task094-writer-catalog-drift",
        );
        admin
            .batch_execute(apply_sql)
            .unwrap_or_else(|_| panic!("TASK094_{label}_DRIFT_APPLY"));
    }
    let failure = {
        let mut verifier = config.role_client(
            target.database_name(),
            DatabaseRole::Migrator,
            REQUIRED_APPLICATION_NAME,
        );
        verify_postgres_schema(&mut verifier, target, DatabaseRole::Migrator).err()
    };
    {
        let mut admin = config.connect(
            target.database_name(),
            "lattice-devos-task094-writer-catalog-repair",
        );
        admin
            .batch_execute(repair_sql)
            .unwrap_or_else(|_| panic!("TASK094_{label}_DRIFT_REPAIR"));
    }
    let failure = failure.unwrap_or_else(|| panic!("TASK094_{label}_DRIFT_MUST_FAIL"));
    assert_eq!(failure.kind(), expected_kind, "TASK094_{label}_ERROR_KIND");
    assert_fresh_v7_verifier_passes(config, target, label);
    println!("TASK094_V7_{label}_DRIFT_REJECTED_AND_REPAIRED");
}

fn prove_v7_verifier_rejects_profile_and_lineage_drift(
    config: &LiveConfig,
    target: &MigrationTarget,
    migrator: &mut Client,
) {
    {
        let mut admin = config.connect(
            target.database_name(),
            "lattice-devos-task094-membership-drift",
        );
        admin
            .batch_execute(
                "GRANT lattice_migrator TO lattice_runtime_login \
                 WITH ADMIN FALSE, INHERIT FALSE, SET TRUE",
            )
            .expect("TASK094_LOGIN_ROLE_MEMBERSHIP_DRIFT_APPLY");
    }
    let membership_failure = {
        let mut verifier = config.role_client(
            target.database_name(),
            DatabaseRole::Migrator,
            REQUIRED_APPLICATION_NAME,
        );
        verify_postgres_schema(&mut verifier, target, DatabaseRole::Migrator).err()
    };
    {
        let mut admin = config.connect(
            target.database_name(),
            "lattice-devos-task094-membership-repair",
        );
        admin
            .batch_execute("REVOKE lattice_migrator FROM lattice_runtime_login")
            .expect("TASK094_LOGIN_ROLE_MEMBERSHIP_DRIFT_REPAIR");
    }
    let membership_failure =
        membership_failure.expect("TASK094_LOGIN_ROLE_MEMBERSHIP_DRIFT_MUST_FAIL");
    assert_eq!(
        membership_failure.kind(),
        PostgresStoreSetupErrorKind::PermissionDenied,
        "TASK094_LOGIN_ROLE_MEMBERSHIP_ERROR_KIND"
    );
    assert_eq!(
        membership_failure.code(),
        "STORE_DATABASE_PERMISSION_DENIED",
        "TASK094_LOGIN_ROLE_MEMBERSHIP_ERROR_CODE"
    );
    assert_fresh_v7_verifier_passes(config, target, "LOGIN_ROLE_MEMBERSHIP");
    println!("TASK094_V7_LOGIN_ROLE_MEMBERSHIP_DRIFT_REJECTED_AND_REPAIRED");

    {
        let mut admin = config.connect(
            target.database_name(),
            "lattice-devos-task094-membership-option-drift",
        );
        admin
            .batch_execute(
                "GRANT lattice_runtime TO lattice_runtime_login \
                 WITH ADMIN FALSE, INHERIT TRUE, SET TRUE",
            )
            .expect("TASK094_LOGIN_ROLE_MEMBERSHIP_OPTION_DRIFT_APPLY");
    }
    let membership_option_failure = {
        let mut verifier = config.role_client(
            target.database_name(),
            DatabaseRole::Migrator,
            REQUIRED_APPLICATION_NAME,
        );
        verify_postgres_schema(&mut verifier, target, DatabaseRole::Migrator).err()
    };
    {
        let mut admin = config.connect(
            target.database_name(),
            "lattice-devos-task094-membership-option-repair",
        );
        admin
            .batch_execute(
                "GRANT lattice_runtime TO lattice_runtime_login \
                 WITH ADMIN FALSE, INHERIT FALSE, SET TRUE",
            )
            .expect("TASK094_LOGIN_ROLE_MEMBERSHIP_OPTION_DRIFT_REPAIR");
    }
    let membership_option_failure =
        membership_option_failure.expect("TASK094_LOGIN_ROLE_MEMBERSHIP_OPTION_DRIFT_MUST_FAIL");
    assert_eq!(
        membership_option_failure.kind(),
        PostgresStoreSetupErrorKind::PermissionDenied,
        "TASK094_LOGIN_ROLE_MEMBERSHIP_OPTION_ERROR_KIND"
    );
    assert_eq!(
        membership_option_failure.code(),
        "STORE_DATABASE_PERMISSION_DENIED",
        "TASK094_LOGIN_ROLE_MEMBERSHIP_OPTION_ERROR_CODE"
    );
    assert_fresh_v7_verifier_passes(config, target, "LOGIN_ROLE_MEMBERSHIP_OPTION");
    println!("TASK094_V7_LOGIN_ROLE_MEMBERSHIP_OPTION_DRIFT_REJECTED_AND_REPAIRED");

    {
        let mut admin = config.connect(
            target.database_name(),
            "lattice-devos-task094-role-attribute-drift",
        );
        admin
            .batch_execute("ALTER ROLE lattice_guardian SUPERUSER")
            .expect("TASK094_CAPABILITY_ROLE_ATTRIBUTE_DRIFT_APPLY");
    }
    let role_attribute_failure = {
        let mut verifier = config.role_client(
            target.database_name(),
            DatabaseRole::Migrator,
            REQUIRED_APPLICATION_NAME,
        );
        verify_postgres_schema(&mut verifier, target, DatabaseRole::Migrator).err()
    };
    {
        let mut admin = config.connect(
            target.database_name(),
            "lattice-devos-task094-role-attribute-repair",
        );
        admin
            .batch_execute("ALTER ROLE lattice_guardian NOSUPERUSER")
            .expect("TASK094_CAPABILITY_ROLE_ATTRIBUTE_DRIFT_REPAIR");
    }
    let role_attribute_failure =
        role_attribute_failure.expect("TASK094_CAPABILITY_ROLE_ATTRIBUTE_DRIFT_MUST_FAIL");
    assert_eq!(
        role_attribute_failure.kind(),
        PostgresStoreSetupErrorKind::PermissionDenied,
        "TASK094_CAPABILITY_ROLE_ATTRIBUTE_ERROR_KIND"
    );
    assert_eq!(
        role_attribute_failure.code(),
        "STORE_DATABASE_PERMISSION_DENIED",
        "TASK094_CAPABILITY_ROLE_ATTRIBUTE_ERROR_CODE"
    );
    assert_fresh_v7_verifier_passes(config, target, "CAPABILITY_ROLE_ATTRIBUTE");
    println!("TASK094_V7_CAPABILITY_ROLE_ATTRIBUTE_DRIFT_REJECTED_AND_REPAIRED");

    {
        let mut admin = config.connect(
            target.database_name(),
            "lattice-devos-task094-parameter-acl-drift",
        );
        admin
            .batch_execute(
                "GRANT SET ON PARAMETER session_replication_role TO lattice_runtime_login",
            )
            .expect("TASK094_PARAMETER_ACL_DRIFT_APPLY");
    }
    let parameter_acl_failure = {
        let mut verifier = config.role_client(
            target.database_name(),
            DatabaseRole::Migrator,
            REQUIRED_APPLICATION_NAME,
        );
        verify_postgres_schema(&mut verifier, target, DatabaseRole::Migrator).err()
    };
    {
        let mut admin = config.connect(
            target.database_name(),
            "lattice-devos-task094-parameter-acl-repair",
        );
        admin
            .batch_execute(
                "REVOKE ALL ON PARAMETER session_replication_role FROM lattice_runtime_login",
            )
            .expect("TASK094_PARAMETER_ACL_DRIFT_REPAIR");
    }
    let parameter_acl_failure =
        parameter_acl_failure.expect("TASK094_PARAMETER_ACL_DRIFT_MUST_FAIL");
    assert_eq!(
        parameter_acl_failure.kind(),
        PostgresStoreSetupErrorKind::PermissionDenied,
        "TASK094_PARAMETER_ACL_ERROR_KIND"
    );
    assert_eq!(
        parameter_acl_failure.code(),
        "STORE_DATABASE_PERMISSION_DENIED",
        "TASK094_PARAMETER_ACL_ERROR_CODE"
    );
    assert_fresh_v7_verifier_passes(config, target, "PARAMETER_ACL");
    println!("TASK094_V7_PARAMETER_ACL_DRIFT_REJECTED_AND_REPAIRED");

    {
        let mut admin = config.connect(
            target.database_name(),
            "lattice-devos-task094-cross-db-public-acl-drift",
        );
        admin
            .batch_execute("GRANT CONNECT ON DATABASE postgres TO PUBLIC")
            .expect("TASK094_CROSS_DATABASE_PUBLIC_ACL_DRIFT_APPLY");
    }
    let cross_database_acl_failure = {
        let mut verifier = config.role_client(
            target.database_name(),
            DatabaseRole::Migrator,
            REQUIRED_APPLICATION_NAME,
        );
        verify_postgres_schema(&mut verifier, target, DatabaseRole::Migrator).err()
    };
    {
        let mut admin = config.connect(
            target.database_name(),
            "lattice-devos-task094-cross-db-public-acl-repair",
        );
        admin
            .batch_execute("REVOKE ALL ON DATABASE postgres FROM PUBLIC")
            .expect("TASK094_CROSS_DATABASE_PUBLIC_ACL_DRIFT_REPAIR");
    }
    let cross_database_acl_failure =
        cross_database_acl_failure.expect("TASK094_CROSS_DATABASE_PUBLIC_ACL_DRIFT_MUST_FAIL");
    assert_eq!(
        cross_database_acl_failure.kind(),
        PostgresStoreSetupErrorKind::PermissionDenied,
        "TASK094_CROSS_DATABASE_PUBLIC_ACL_ERROR_KIND"
    );
    assert_eq!(
        cross_database_acl_failure.code(),
        "STORE_DATABASE_PERMISSION_DENIED",
        "TASK094_CROSS_DATABASE_PUBLIC_ACL_ERROR_CODE"
    );
    assert_fresh_v7_verifier_passes(config, target, "CROSS_DATABASE_PUBLIC_ACL");
    println!("TASK094_V7_CROSS_DATABASE_PUBLIC_ACL_DRIFT_REJECTED_AND_REPAIRED");

    prove_v7_catalog_drift_case(
        config,
        target,
        migrator,
        "READMODEL_SCHEMA_HEADER",
        "COMMENT ON SCHEMA readmodel IS 'TASK094_DRIFT'",
        "COMMENT ON SCHEMA readmodel IS 'LATTICE_DEVOS_READMODEL_SCHEMA_V7'",
        PostgresStoreSetupErrorKind::CorruptCatalog,
    );
    prove_v7_catalog_drift_case(
        config,
        target,
        migrator,
        "LEGACY_RUNTIME_ADMISSION_ACL",
        "GRANT UPDATE ON TABLE control.runtime_admission TO lattice_guardian",
        "REVOKE UPDATE ON TABLE control.runtime_admission FROM lattice_guardian",
        PostgresStoreSetupErrorKind::PermissionDenied,
    );
    prove_v7_catalog_drift_case(
        config,
        target,
        migrator,
        "LEGACY_LEDGER_FUNCTION_PUBLIC_ACL",
        "GRANT EXECUTE ON FUNCTION control.task_ledger_read_head_v4(smallint,text,bytea,text,text) TO PUBLIC",
        "REVOKE EXECUTE ON FUNCTION control.task_ledger_read_head_v4(smallint,text,bytea,text,text) FROM PUBLIC",
        PostgresStoreSetupErrorKind::PermissionDenied,
    );
    prove_v7_catalog_drift_case(
        config,
        target,
        migrator,
        "OWNED_TYPE",
        "CREATE TYPE control.task094_catalog_drift_type AS ENUM ('DRIFT')",
        "DROP TYPE control.task094_catalog_drift_type",
        PostgresStoreSetupErrorKind::CorruptCatalog,
    );
    prove_v7_catalog_drift_case(
        config,
        target,
        migrator,
        "RUNTIME_ADMISSION_POLICY",
        "CREATE POLICY task094_catalog_drift_policy ON control.runtime_admission USING (true)",
        "DROP POLICY task094_catalog_drift_policy ON control.runtime_admission",
        PostgresStoreSetupErrorKind::CorruptCatalog,
    );
    prove_v7_catalog_drift_case(
        config,
        target,
        migrator,
        "NONPUBLIC_DEFAULT_TABLE_ACL",
        "ALTER DEFAULT PRIVILEGES FOR ROLE lattice_migrator IN SCHEMA control GRANT SELECT ON TABLES TO lattice_guardian",
        "ALTER DEFAULT PRIVILEGES FOR ROLE lattice_migrator IN SCHEMA control REVOKE SELECT ON TABLES FROM lattice_guardian",
        PostgresStoreSetupErrorKind::PermissionDenied,
    );
    prove_v7_admin_catalog_drift_case(
        config,
        target,
        "NON_MIGRATOR_DEFAULT_TABLE_ACL",
        "ALTER DEFAULT PRIVILEGES FOR ROLE lattice_runtime GRANT SELECT ON TABLES TO lattice_guardian",
        "ALTER DEFAULT PRIVILEGES FOR ROLE lattice_runtime REVOKE SELECT ON TABLES FROM lattice_guardian",
        PostgresStoreSetupErrorKind::PermissionDenied,
    );
    prove_v7_catalog_drift_case(
        config,
        target,
        migrator,
        "WRITER_GUARDIAN_TABLE_ACL",
        "GRANT SELECT ON TABLE writer_lease.writer_lease_heads TO lattice_guardian",
        "REVOKE SELECT ON TABLE writer_lease.writer_lease_heads FROM lattice_guardian",
        PostgresStoreSetupErrorKind::PermissionDenied,
    );
    prove_v7_catalog_drift_case(
        config,
        target,
        migrator,
        "WRITER_GUARDIAN_SCHEMA_ACL",
        "GRANT USAGE ON SCHEMA writer_lease TO lattice_guardian",
        "REVOKE USAGE ON SCHEMA writer_lease FROM lattice_guardian",
        PostgresStoreSetupErrorKind::PermissionDenied,
    );
    prove_v7_admin_catalog_drift_case(
        config,
        target,
        "WRITER_TABLE_OWNER",
        "ALTER TABLE writer_lease.writer_lease_heads OWNER TO lattice_guardian",
        "ALTER TABLE writer_lease.writer_lease_heads OWNER TO lattice_migrator",
        PostgresStoreSetupErrorKind::CorruptCatalog,
    );
    prove_v7_catalog_drift_case(
        config,
        target,
        migrator,
        "WRITER_TYPE",
        "CREATE TYPE writer_lease.task094_writer_drift_type AS ENUM ('DRIFT')",
        "DROP TYPE writer_lease.task094_writer_drift_type",
        PostgresStoreSetupErrorKind::CorruptCatalog,
    );
    prove_v7_catalog_drift_case(
        config,
        target,
        migrator,
        "WRITER_TRIGGER",
        "CREATE TRIGGER task094_writer_drift_trigger BEFORE UPDATE ON writer_lease.writer_lease_heads FOR EACH ROW EXECUTE FUNCTION pg_catalog.suppress_redundant_updates_trigger()",
        "DROP TRIGGER task094_writer_drift_trigger ON writer_lease.writer_lease_heads",
        PostgresStoreSetupErrorKind::CorruptCatalog,
    );
    prove_v7_catalog_drift_case(
        config,
        target,
        migrator,
        "AMBIGUITY_ACL",
        "GRANT SELECT ON TABLE control.task_ingress_historical_ambiguities TO lattice_readonly",
        "REVOKE SELECT ON TABLE control.task_ingress_historical_ambiguities FROM lattice_readonly",
        PostgresStoreSetupErrorKind::PermissionDenied,
    );
    prove_v7_catalog_drift_case(
        config,
        target,
        migrator,
        "AMBIGUITY_EVENT_FK",
        "ALTER TABLE control.task_ingress_historical_ambiguities DROP CONSTRAINT task_ingress_historical_ambiguities_event_fk",
        "ALTER TABLE control.task_ingress_historical_ambiguities ADD CONSTRAINT task_ingress_historical_ambiguities_event_fk FOREIGN KEY (stream_id,event_sequence) REFERENCES control.task_ledger_events (stream_id,sequence)",
        PostgresStoreSetupErrorKind::CorruptCatalog,
    );
    prove_v7_catalog_drift_case(
        config,
        target,
        migrator,
        "INGRESS_FUNCTION_SECURITY",
        "ALTER FUNCTION control.task_ingress_read_by_request_v1(text,text) SECURITY INVOKER",
        "ALTER FUNCTION control.task_ingress_read_by_request_v1(text,text) SECURITY DEFINER",
        PostgresStoreSetupErrorKind::CorruptCatalog,
    );
    prove_v7_catalog_drift_case(
        config,
        target,
        migrator,
        "INGRESS_FUNCTION_PUBLIC_ACL",
        "GRANT EXECUTE ON FUNCTION control.task_ingress_read_by_request_v1(text,text) TO PUBLIC",
        "REVOKE EXECUTE ON FUNCTION control.task_ingress_read_by_request_v1(text,text) FROM PUBLIC",
        PostgresStoreSetupErrorKind::PermissionDenied,
    );

    let ambiguity = migrator
        .query_one(
            "SELECT ingress_id::text,client_request_id::text,stream_id,event_digest \
               FROM ONLY control.task_ingress_historical_ambiguities \
              ORDER BY ingress_id,client_request_id,stream_id LIMIT 1",
            &[],
        )
        .expect("TASK094_LINEAGE_DRIFT_SOURCE");
    let ingress_id = ambiguity.get::<_, String>(0);
    let client_request_id = ambiguity.get::<_, String>(1);
    let stream_id = ambiguity.get::<_, Vec<u8>>(2);
    let event_digest = ambiguity.get::<_, Vec<u8>>(3);
    let replacement = vec![0xabu8; 32];
    assert_ne!(event_digest, replacement, "TASK094_LINEAGE_DRIFT_DISTINCT");
    assert_eq!(
        migrator
            .execute(
                "UPDATE ONLY control.task_ingress_historical_ambiguities \
                    SET event_digest=$1::bytea \
                  WHERE ingress_id=$2 AND client_request_id=$3 AND stream_id=$4::bytea",
                &[&replacement, &ingress_id, &client_request_id, &stream_id],
            )
            .expect("TASK094_LINEAGE_DRIFT_APPLY"),
        1
    );
    assert_fresh_v7_verifier_rejects(
        config,
        target,
        PostgresStoreSetupErrorKind::HistoryMismatch,
        "AMBIGUITY_LINEAGE",
    );
    assert_eq!(
        migrator
            .execute(
                "UPDATE ONLY control.task_ingress_historical_ambiguities \
                    SET event_digest=$1::bytea \
                  WHERE ingress_id=$2 AND client_request_id=$3 AND stream_id=$4::bytea",
                &[&event_digest, &ingress_id, &client_request_id, &stream_id],
            )
            .expect("TASK094_LINEAGE_DRIFT_REPAIR"),
        1
    );
    assert_fresh_v7_verifier_passes(config, target, "AMBIGUITY_LINEAGE");
    println!("TASK094_V7_AMBIGUITY_LINEAGE_DRIFT_REJECTED_AND_REPAIRED");
}

fn prove_v7_verifier_rejects_historical_binding_drift(
    config: &LiveConfig,
    target: &MigrationTarget,
    migrator: &mut Client,
    claim: &TaskIngressClaim,
) {
    let stream_id = task094_digest_bytes(claim.stream_id());
    let command_id = format!("mcp-submit:{}", claim.client_request_id());

    let missing_event_failure = migrator
        .execute(
            "DELETE FROM ONLY control.task_ledger_events \
              WHERE stream_id=$1::bytea AND sequence=1",
            &[&stream_id],
        )
        .expect_err("TASK094_V7_HISTORICAL_EVENT_DELETE_MUST_FAIL");
    assert_eq!(
        missing_event_failure
            .as_db_error()
            .expect("TASK094_V7_HISTORICAL_EVENT_DELETE_DATABASE_ERROR")
            .code(),
        &SqlState::FOREIGN_KEY_VIOLATION,
        "TASK094_V7_HISTORICAL_EVENT_DELETE_SQLSTATE"
    );
    assert_fresh_v7_verifier_passes(config, target, "HISTORICAL_EVENT_DELETE");
    println!("TASK094_V7_HISTORICAL_EVENT_DELETE_REJECTED");

    let orphan_command_id = "mcp-submit:task094-orphan-v7";
    migrator
        .batch_execute(
            "CREATE TEMPORARY TABLE task094_orphan_command_backup \
             ON COMMIT PRESERVE ROWS AS \
             SELECT * FROM ONLY control.task_ledger_commands WITH NO DATA",
        )
        .expect("TASK094_V7_HISTORICAL_ORPHAN_COMMAND_BACKUP_TABLE");
    assert_eq!(
        migrator
            .execute(
                "INSERT INTO pg_temp.task094_orphan_command_backup \
                 SELECT * FROM ONLY control.task_ledger_commands \
                  WHERE stream_id=$1::bytea AND command_id=$2",
                &[&stream_id, &command_id],
            )
            .expect("TASK094_V7_HISTORICAL_ORPHAN_COMMAND_BACKUP"),
        1
    );
    assert_eq!(
        migrator
            .execute(
                "UPDATE pg_temp.task094_orphan_command_backup SET command_id=$1",
                &[&orphan_command_id],
            )
            .expect("TASK094_V7_HISTORICAL_ORPHAN_COMMAND_REKEY"),
        1
    );
    assert_eq!(
        migrator
            .execute(
                "INSERT INTO control.task_ledger_commands \
                 SELECT * FROM pg_temp.task094_orphan_command_backup",
                &[],
            )
            .expect("TASK094_V7_HISTORICAL_ORPHAN_COMMAND_DRIFT_APPLY"),
        1
    );
    assert_fresh_v7_verifier_rejects(
        config,
        target,
        PostgresStoreSetupErrorKind::HistoryMismatch,
        "HISTORICAL_ORPHAN_COMMAND",
    );
    assert_eq!(
        migrator
            .execute(
                "DELETE FROM ONLY control.task_ledger_commands \
                  WHERE stream_id=$1::bytea AND command_id=$2",
                &[&stream_id, &orphan_command_id],
            )
            .expect("TASK094_V7_HISTORICAL_ORPHAN_COMMAND_DRIFT_REPAIR"),
        1
    );
    migrator
        .batch_execute("DROP TABLE pg_temp.task094_orphan_command_backup")
        .expect("TASK094_V7_HISTORICAL_ORPHAN_COMMAND_BACKUP_DROP");
    assert_fresh_v7_verifier_passes(config, target, "HISTORICAL_ORPHAN_COMMAND");
    println!("TASK094_V7_HISTORICAL_ORPHAN_COMMAND_DRIFT_REJECTED_AND_REPAIRED");

    assert_eq!(
        migrator
            .execute(
                "UPDATE ONLY control.task_ledger_events SET audit_outcome='DENIED' \
                  WHERE stream_id=$1::bytea AND sequence=1",
                &[&stream_id],
            )
            .expect("TASK094_V7_HISTORICAL_AUDIT_OUTCOME_DRIFT_APPLY"),
        1
    );
    assert_fresh_v7_verifier_rejects(
        config,
        target,
        PostgresStoreSetupErrorKind::HistoryMismatch,
        "HISTORICAL_AUDIT_OUTCOME",
    );
    assert_eq!(
        migrator
            .execute(
                "UPDATE ONLY control.task_ledger_events SET audit_outcome='RECORDED' \
                  WHERE stream_id=$1::bytea AND sequence=1",
                &[&stream_id],
            )
            .expect("TASK094_V7_HISTORICAL_AUDIT_OUTCOME_DRIFT_REPAIR"),
        1
    );
    assert_fresh_v7_verifier_passes(config, target, "HISTORICAL_AUDIT_OUTCOME");
    println!("TASK094_V7_HISTORICAL_AUDIT_OUTCOME_DRIFT_REJECTED_AND_REPAIRED");

    let original_actor: String = migrator
        .query_one(
            "SELECT actor_id::text FROM ONLY control.task_ledger_commands \
              WHERE stream_id=$1::bytea AND command_id=$2",
            &[&stream_id, &command_id],
        )
        .expect("TASK094_V7_HISTORICAL_COMMAND_ACTOR_SOURCE")
        .get(0);
    assert_eq!(
        migrator
            .execute(
                "UPDATE ONLY control.task_ledger_commands SET actor_id='task094-corrupt-actor' \
                  WHERE stream_id=$1::bytea AND command_id=$2",
                &[&stream_id, &command_id],
            )
            .expect("TASK094_V7_HISTORICAL_COMMAND_ACTOR_DRIFT_APPLY"),
        1
    );
    assert_fresh_v7_verifier_rejects(
        config,
        target,
        PostgresStoreSetupErrorKind::HistoryMismatch,
        "HISTORICAL_COMMAND_ACTOR_BINDING",
    );
    assert_eq!(
        migrator
            .execute(
                "UPDATE ONLY control.task_ledger_commands SET actor_id=$1 \
                  WHERE stream_id=$2::bytea AND command_id=$3",
                &[&original_actor, &stream_id, &command_id],
            )
            .expect("TASK094_V7_HISTORICAL_COMMAND_ACTOR_DRIFT_REPAIR"),
        1
    );
    assert_fresh_v7_verifier_passes(config, target, "HISTORICAL_COMMAND_ACTOR_BINDING");
    println!("TASK094_V7_HISTORICAL_COMMAND_ACTOR_BINDING_DRIFT_REJECTED_AND_REPAIRED");

    let original_request_digest: Vec<u8> = migrator
        .query_one(
            "SELECT request_digest FROM ONLY control.task_ledger_commands \
              WHERE stream_id=$1::bytea AND command_id=$2",
            &[&stream_id, &command_id],
        )
        .expect("TASK094_V7_HISTORICAL_COMMAND_REQUEST_DIGEST_SOURCE")
        .get(0);
    let corrupt_request_digest = vec![0x42_u8; 32];
    assert_ne!(original_request_digest, corrupt_request_digest);
    assert_eq!(
        migrator
            .execute(
                "UPDATE ONLY control.task_ledger_commands SET request_digest=$1::bytea \
                  WHERE stream_id=$2::bytea AND command_id=$3",
                &[&corrupt_request_digest, &stream_id, &command_id],
            )
            .expect("TASK094_V7_HISTORICAL_COMMAND_REQUEST_DIGEST_DRIFT_APPLY"),
        1
    );
    assert_fresh_v7_verifier_rejects(
        config,
        target,
        PostgresStoreSetupErrorKind::HistoryMismatch,
        "HISTORICAL_COMMAND_REQUEST_DIGEST_BINDING",
    );
    assert_eq!(
        migrator
            .execute(
                "UPDATE ONLY control.task_ledger_commands SET request_digest=$1::bytea \
                  WHERE stream_id=$2::bytea AND command_id=$3",
                &[&original_request_digest, &stream_id, &command_id],
            )
            .expect("TASK094_V7_HISTORICAL_COMMAND_REQUEST_DIGEST_DRIFT_REPAIR"),
        1
    );
    assert_fresh_v7_verifier_passes(config, target, "HISTORICAL_COMMAND_REQUEST_DIGEST_BINDING");
    println!("TASK094_V7_HISTORICAL_COMMAND_REQUEST_DIGEST_BINDING_DRIFT_REJECTED_AND_REPAIRED");

    let original_action: String = migrator
        .query_one(
            "SELECT action_id::text FROM ONLY control.task_ledger_events \
              WHERE stream_id=$1::bytea AND sequence=1",
            &[&stream_id],
        )
        .expect("TASK094_V7_HISTORICAL_EVENT_ACTION_SOURCE")
        .get(0);
    assert_eq!(
        migrator
            .execute(
                "UPDATE ONLY control.task_ledger_events SET action_id='GENERAL_TASK_INTAKE_V1' \
                  WHERE stream_id=$1::bytea AND sequence=1",
                &[&stream_id],
            )
            .expect("TASK094_V7_HISTORICAL_EVENT_ACTION_DRIFT_APPLY"),
        1
    );
    assert_fresh_v7_verifier_rejects(
        config,
        target,
        PostgresStoreSetupErrorKind::HistoryMismatch,
        "HISTORICAL_EVENT_ACTION_INVISIBILITY",
    );
    assert_eq!(
        migrator
            .execute(
                "UPDATE ONLY control.task_ledger_events SET action_id=$1 \
                  WHERE stream_id=$2::bytea AND sequence=1",
                &[&original_action, &stream_id],
            )
            .expect("TASK094_V7_HISTORICAL_EVENT_ACTION_DRIFT_REPAIR"),
        1
    );
    assert_fresh_v7_verifier_passes(config, target, "HISTORICAL_EVENT_ACTION_INVISIBILITY");
    println!("TASK094_V7_HISTORICAL_EVENT_ACTION_INVISIBILITY_DRIFT_REJECTED_AND_REPAIRED");

    assert_eq!(
        migrator
            .execute(
                "UPDATE ONLY control.task_ledger_events SET action_id='GENERAL_TASK_INTAKE_V1' \
                  WHERE stream_id=$1::bytea AND sequence=1",
                &[&stream_id],
            )
            .expect("TASK094_V7_HISTORICAL_DUAL_ACTION_EVENT_DRIFT_APPLY"),
        1
    );
    assert_eq!(
        migrator
            .execute(
                "UPDATE ONLY control.task_ledger_commands SET action_id='GENERAL_TASK_INTAKE_V1' \
                  WHERE stream_id=$1::bytea AND command_id=$2",
                &[&stream_id, &command_id],
            )
            .expect("TASK094_V7_HISTORICAL_DUAL_ACTION_COMMAND_DRIFT_APPLY"),
        1
    );
    assert_fresh_v7_verifier_rejects(
        config,
        target,
        PostgresStoreSetupErrorKind::HistoryMismatch,
        "HISTORICAL_DUAL_ACTION_INVISIBILITY",
    );
    assert_eq!(
        migrator
            .execute(
                "UPDATE ONLY control.task_ledger_events SET action_id=$1 \
                  WHERE stream_id=$2::bytea AND sequence=1",
                &[&original_action, &stream_id],
            )
            .expect("TASK094_V7_HISTORICAL_DUAL_ACTION_EVENT_DRIFT_REPAIR"),
        1
    );
    assert_eq!(
        migrator
            .execute(
                "UPDATE ONLY control.task_ledger_commands SET action_id=$1 \
                  WHERE stream_id=$2::bytea AND command_id=$3",
                &[&original_action, &stream_id, &command_id],
            )
            .expect("TASK094_V7_HISTORICAL_DUAL_ACTION_COMMAND_DRIFT_REPAIR"),
        1
    );
    assert_fresh_v7_verifier_passes(config, target, "HISTORICAL_DUAL_ACTION_INVISIBILITY");
    println!("TASK094_V7_HISTORICAL_DUAL_ACTION_INVISIBILITY_DRIFT_REJECTED_AND_REPAIRED");

    assert_eq!(
        migrator
            .execute(
                "UPDATE ONLY control.task_ledger_events SET action_id='GENERAL_TASK_INTAKE_V1' \
                  WHERE stream_id=$1::bytea AND sequence=1",
                &[&stream_id],
            )
            .expect("TASK094_V7_HISTORICAL_TRIPLE_RECAST_EVENT_DRIFT_APPLY"),
        1
    );
    assert_eq!(
        migrator
            .execute(
                "UPDATE ONLY control.task_ledger_commands SET action_id='GENERAL_TASK_INTAKE_V1' \
                  WHERE stream_id=$1::bytea AND command_id=$2",
                &[&stream_id, &command_id],
            )
            .expect("TASK094_V7_HISTORICAL_TRIPLE_RECAST_COMMAND_DRIFT_APPLY"),
        1
    );
    assert_eq!(
        migrator
            .execute(
                "UPDATE ONLY control.task_ingress_claims SET request_kind='GENERAL_TASK' \
                  WHERE stream_id=$1::bytea AND command_id=$2",
                &[&stream_id, &command_id],
            )
            .expect("TASK094_V7_HISTORICAL_TRIPLE_RECAST_CLAIM_DRIFT_APPLY"),
        1
    );
    let recast_ingress_digest = vec![0x5a_u8; 32];
    assert_ne!(stream_id, recast_ingress_digest);
    assert_eq!(
        migrator
            .execute(
                "UPDATE ONLY control.task_ingress_claims SET ingress_request_digest=$1::bytea \
                  WHERE stream_id=$2::bytea AND command_id=$3",
                &[&recast_ingress_digest, &stream_id, &command_id],
            )
            .expect("TASK094_V7_HISTORICAL_TRIPLE_RECAST_INGRESS_DIGEST_DRIFT_APPLY"),
        1
    );
    assert_fresh_v7_verifier_rejects(
        config,
        target,
        PostgresStoreSetupErrorKind::HistoryMismatch,
        "HISTORICAL_TRIPLE_RECAST_INVISIBILITY",
    );
    let authority_receipt_digest = vec![0x6b_u8; 32];
    assert_eq!(
        migrator
            .execute(
                "INSERT INTO control.task_submission_envelopes (\
                     schema_version,ingress_id,client_request_id,objective,\
                     project_display_name,project_authority_receipt_digest,project_id,\
                     project_snapshot_id,task_id,task_revision,task_subject_kind,\
                     intake_digest,stream_id,task_ref,admission_action,envelope_digest,\
                     event_sequence,event_digest,command_id,request_digest,\
                     ingress_request_digest) \
                 SELECT 'lattice.task-ledger.task-submission/1.0',c.ingress_id,\
                        c.client_request_id,'TASK094 forged historical envelope','TASK094',\
                        $1::bytea,s.project_id,s.project_snapshot_id,s.task_id,s.task_revision,\
                        'GENERAL_TASK_INTAKE',s.task_subject_digest,s.stream_id,\
                        pg_catalog.encode(pg_catalog.sha256(s.stream_id || \
                            pg_catalog.convert_to('TASK094_FORGED_ENVELOPE','UTF8')),'hex'),\
                        'GENERAL_TASK_INTAKE_V1',e.subject_digest,e.sequence,e.event_digest,\
                        e.command_id,e.request_digest,c.ingress_request_digest \
                   FROM ONLY control.task_ingress_claims AS c \
                   JOIN ONLY control.task_ledger_streams AS s ON s.stream_id=c.stream_id \
                   JOIN ONLY control.task_ledger_events AS e \
                     ON e.stream_id=c.stream_id AND e.sequence=c.event_sequence \
                   JOIN ONLY control.task_ledger_commands AS m \
                     ON m.stream_id=c.stream_id AND m.command_id=c.command_id \
                  WHERE c.stream_id=$2::bytea AND c.command_id=$3",
                &[&authority_receipt_digest, &stream_id, &command_id],
            )
            .expect("TASK094_V7_HISTORICAL_FORGED_ENVELOPE_DRIFT_APPLY"),
        1
    );
    assert_fresh_v7_verifier_rejects(
        config,
        target,
        PostgresStoreSetupErrorKind::HistoryMismatch,
        "HISTORICAL_FORGED_ENVELOPE_STREAM_SEMANTICS",
    );
    assert_eq!(
        migrator
            .execute(
                "DELETE FROM ONLY control.task_submission_envelopes \
                  WHERE stream_id=$1::bytea",
                &[&stream_id],
            )
            .expect("TASK094_V7_HISTORICAL_FORGED_ENVELOPE_DRIFT_REPAIR"),
        1
    );
    println!("TASK094_V7_HISTORICAL_FORGED_ENVELOPE_STREAM_SEMANTICS_DRIFT_REJECTED_AND_REPAIRED");
    assert_eq!(
        migrator
            .execute(
                "UPDATE ONLY control.task_ingress_claims \
                    SET request_kind='CONTROLLED_CODEX_CANARY', \
                        ingress_request_digest=stream_id \
                  WHERE stream_id=$1::bytea AND command_id=$2",
                &[&stream_id, &command_id],
            )
            .expect("TASK094_V7_HISTORICAL_TRIPLE_RECAST_CLAIM_DRIFT_REPAIR"),
        1
    );
    assert_eq!(
        migrator
            .execute(
                "UPDATE ONLY control.task_ledger_events SET action_id=$1 \
                  WHERE stream_id=$2::bytea AND sequence=1",
                &[&original_action, &stream_id],
            )
            .expect("TASK094_V7_HISTORICAL_TRIPLE_RECAST_EVENT_DRIFT_REPAIR"),
        1
    );
    assert_eq!(
        migrator
            .execute(
                "UPDATE ONLY control.task_ledger_commands SET action_id=$1 \
                  WHERE stream_id=$2::bytea AND command_id=$3",
                &[&original_action, &stream_id, &command_id],
            )
            .expect("TASK094_V7_HISTORICAL_TRIPLE_RECAST_COMMAND_DRIFT_REPAIR"),
        1
    );
    assert_fresh_v7_verifier_passes(config, target, "HISTORICAL_TRIPLE_RECAST_INVISIBILITY");
    println!("TASK094_V7_HISTORICAL_TRIPLE_RECAST_INVISIBILITY_DRIFT_REJECTED_AND_REPAIRED");
}

fn rewrite_historical_canary_command_id(
    config: &LiveConfig,
    target: &MigrationTarget,
    claim: &TaskIngressClaim,
    expected_command_id: &str,
    replacement_command_id: &str,
) {
    let mut admin = config.connect(
        target.database_name(),
        "lattice-devos-task094-history-corruption",
    );
    let mut transaction = admin
        .transaction()
        .expect("TASK094_HISTORY_REWRITE_TRANSACTION");
    transaction
        .batch_execute("SET LOCAL session_replication_role = replica")
        .expect("TASK094_HISTORY_REWRITE_DISABLE_FK_TRIGGERS");
    let stream_id = task094_digest_bytes(claim.stream_id());
    assert_eq!(
        transaction
            .execute(
                "UPDATE ONLY control.task_ledger_commands SET command_id=$1 \
                 WHERE stream_id=$2::bytea AND command_id=$3",
                &[&replacement_command_id, &stream_id, &expected_command_id],
            )
            .expect("TASK094_HISTORY_REWRITE_COMMAND"),
        1
    );
    assert_eq!(
        transaction
            .execute(
                "UPDATE ONLY control.task_ledger_events SET command_id=$1 \
                 WHERE stream_id=$2::bytea AND sequence=1 AND command_id=$3",
                &[&replacement_command_id, &stream_id, &expected_command_id],
            )
            .expect("TASK094_HISTORY_REWRITE_EVENT"),
        1
    );
    transaction
        .commit()
        .expect("TASK094_HISTORY_REWRITE_COMMIT");
}

fn assert_failed_v7_migration_preserves_v6(
    migrator: &mut Client,
    target: &MigrationTarget,
    duplicate_fixture: &HistoricalDuplicateFixture,
    label: &str,
) {
    let failure = match apply_migrations(migrator, target) {
        Ok(_) => panic!("TASK094_{label}_MUST_REJECT_V7"),
        Err(failure) => failure,
    };
    assert_eq!(
        failure.kind().code(),
        "STORE_MIGRATION_TRANSACTION_FAILED",
        "TASK094_{label}_FAILURE_KIND"
    );
    let retained = migrator
        .query_one(
            "SELECT \
                (SELECT current_schema_version FROM ONLY control.schema_compatibility \
                  WHERE singleton), \
                (SELECT count(*)::bigint FROM ONLY control.migration_history \
                  WHERE ordinal=8), \
                (SELECT global_schema_version FROM ONLY \
                    writer_lease.writer_lease_extension_identity WHERE singleton), \
                pg_catalog.to_regclass('control.task_ingress_claims')::text, \
                pg_catalog.to_regclass('control.task_ingress_historical_ambiguities')::text, \
                pg_catalog.to_regclass('control.task_submission_envelopes')::text",
            &[],
        )
        .unwrap_or_else(|_| panic!("TASK094_{label}_ROLLBACK_STATE"));
    assert_eq!(retained.get::<_, i16>(0), 6);
    assert_eq!(retained.get::<_, i64>(1), 0);
    assert_eq!(retained.get::<_, i16>(2), 6);
    assert_eq!(retained.get::<_, Option<String>>(3), None);
    assert_eq!(retained.get::<_, Option<String>>(4), None);
    assert_eq!(retained.get::<_, Option<String>>(5), None);
    assert_eq!(
        historical_duplicate_ledger_fingerprint(migrator, &duplicate_fixture.claims),
        duplicate_fixture.ledger_fingerprint,
        "TASK094_{label}_MUST_NOT_REWRITE_DUPLICATE_HISTORY"
    );
}

fn prove_historical_binding_migration_rejected(
    target: &MigrationTarget,
    migrator: &mut Client,
    claim: &TaskIngressClaim,
    duplicate_fixture: &HistoricalDuplicateFixture,
) {
    let stream_id = task094_digest_bytes(claim.stream_id());
    let command_id = format!("mcp-submit:{}", claim.client_request_id());

    assert_eq!(
        migrator
            .execute(
                "UPDATE ONLY control.task_ledger_events SET audit_outcome='DENIED' \
                  WHERE stream_id=$1::bytea AND sequence=1",
                &[&stream_id],
            )
            .expect("TASK094_HISTORICAL_AUDIT_OUTCOME_DRIFT_APPLY"),
        1
    );
    assert_failed_v7_migration_preserves_v6(
        migrator,
        target,
        duplicate_fixture,
        "HISTORICAL_AUDIT_OUTCOME",
    );
    assert_eq!(
        migrator
            .execute(
                "UPDATE ONLY control.task_ledger_events SET audit_outcome='RECORDED' \
                  WHERE stream_id=$1::bytea AND sequence=1",
                &[&stream_id],
            )
            .expect("TASK094_HISTORICAL_AUDIT_OUTCOME_DRIFT_REPAIR"),
        1
    );
    println!("TASK094_HISTORICAL_AUDIT_OUTCOME_MIGRATION_REJECTION_PASS");

    let original_actor: String = migrator
        .query_one(
            "SELECT actor_id::text FROM ONLY control.task_ledger_commands \
              WHERE stream_id=$1::bytea AND command_id=$2",
            &[&stream_id, &command_id],
        )
        .expect("TASK094_HISTORICAL_COMMAND_ACTOR_SOURCE")
        .get(0);
    assert_eq!(
        migrator
            .execute(
                "UPDATE ONLY control.task_ledger_commands SET actor_id='task094-corrupt-actor' \
                  WHERE stream_id=$1::bytea AND command_id=$2",
                &[&stream_id, &command_id],
            )
            .expect("TASK094_HISTORICAL_COMMAND_ACTOR_DRIFT_APPLY"),
        1
    );
    assert_failed_v7_migration_preserves_v6(
        migrator,
        target,
        duplicate_fixture,
        "HISTORICAL_COMMAND_ACTOR_BINDING",
    );
    assert_eq!(
        migrator
            .execute(
                "UPDATE ONLY control.task_ledger_commands SET actor_id=$1 \
                  WHERE stream_id=$2::bytea AND command_id=$3",
                &[&original_actor, &stream_id, &command_id],
            )
            .expect("TASK094_HISTORICAL_COMMAND_ACTOR_DRIFT_REPAIR"),
        1
    );
    println!("TASK094_HISTORICAL_COMMAND_ACTOR_BINDING_MIGRATION_REJECTION_PASS");

    let original_request_digest: Vec<u8> = migrator
        .query_one(
            "SELECT request_digest FROM ONLY control.task_ledger_commands \
              WHERE stream_id=$1::bytea AND command_id=$2",
            &[&stream_id, &command_id],
        )
        .expect("TASK094_HISTORICAL_COMMAND_REQUEST_DIGEST_SOURCE")
        .get(0);
    let corrupt_request_digest = vec![0x42_u8; 32];
    assert_ne!(original_request_digest, corrupt_request_digest);
    assert_eq!(
        migrator
            .execute(
                "UPDATE ONLY control.task_ledger_commands SET request_digest=$1::bytea \
                  WHERE stream_id=$2::bytea AND command_id=$3",
                &[&corrupt_request_digest, &stream_id, &command_id],
            )
            .expect("TASK094_HISTORICAL_COMMAND_REQUEST_DIGEST_DRIFT_APPLY"),
        1
    );
    assert_failed_v7_migration_preserves_v6(
        migrator,
        target,
        duplicate_fixture,
        "HISTORICAL_COMMAND_REQUEST_DIGEST_BINDING",
    );
    assert_eq!(
        migrator
            .execute(
                "UPDATE ONLY control.task_ledger_commands SET request_digest=$1::bytea \
                  WHERE stream_id=$2::bytea AND command_id=$3",
                &[&original_request_digest, &stream_id, &command_id],
            )
            .expect("TASK094_HISTORICAL_COMMAND_REQUEST_DIGEST_DRIFT_REPAIR"),
        1
    );
    println!("TASK094_HISTORICAL_COMMAND_REQUEST_DIGEST_BINDING_MIGRATION_REJECTION_PASS");

    migrator
        .batch_execute(
            "CREATE TEMPORARY TABLE task094_missing_event_backup \
             ON COMMIT PRESERVE ROWS AS \
             SELECT * FROM ONLY control.task_ledger_events WITH NO DATA",
        )
        .expect("TASK094_HISTORICAL_MISSING_EVENT_BACKUP_TABLE");
    assert_eq!(
        migrator
            .execute(
                "INSERT INTO pg_temp.task094_missing_event_backup \
                 SELECT * FROM ONLY control.task_ledger_events \
                  WHERE stream_id=$1::bytea AND sequence=1",
                &[&stream_id],
            )
            .expect("TASK094_HISTORICAL_MISSING_EVENT_BACKUP"),
        1
    );
    assert_eq!(
        migrator
            .execute(
                "DELETE FROM ONLY control.task_ledger_events \
                  WHERE stream_id=$1::bytea AND sequence=1",
                &[&stream_id],
            )
            .expect("TASK094_HISTORICAL_MISSING_EVENT_DRIFT_APPLY"),
        1
    );
    assert_failed_v7_migration_preserves_v6(
        migrator,
        target,
        duplicate_fixture,
        "HISTORICAL_MISSING_EVENT",
    );
    assert_eq!(
        migrator
            .execute(
                "INSERT INTO control.task_ledger_events \
                 SELECT * FROM pg_temp.task094_missing_event_backup",
                &[],
            )
            .expect("TASK094_HISTORICAL_MISSING_EVENT_DRIFT_REPAIR"),
        1
    );
    migrator
        .batch_execute("DROP TABLE pg_temp.task094_missing_event_backup")
        .expect("TASK094_HISTORICAL_MISSING_EVENT_BACKUP_DROP");
    println!("TASK094_HISTORICAL_MISSING_EVENT_MIGRATION_REJECTION_PASS");

    let original_action: String = migrator
        .query_one(
            "SELECT action_id::text FROM ONLY control.task_ledger_events \
              WHERE stream_id=$1::bytea AND sequence=1",
            &[&stream_id],
        )
        .expect("TASK094_HISTORICAL_EVENT_ACTION_SOURCE")
        .get(0);
    assert_eq!(
        migrator
            .execute(
                "UPDATE ONLY control.task_ledger_events SET action_id='GENERAL_TASK_INTAKE_V1' \
                  WHERE stream_id=$1::bytea AND sequence=1",
                &[&stream_id],
            )
            .expect("TASK094_HISTORICAL_EVENT_ACTION_DRIFT_APPLY"),
        1
    );
    assert_failed_v7_migration_preserves_v6(
        migrator,
        target,
        duplicate_fixture,
        "HISTORICAL_EVENT_ACTION_INVISIBILITY",
    );
    assert_eq!(
        migrator
            .execute(
                "UPDATE ONLY control.task_ledger_events SET action_id=$1 \
                  WHERE stream_id=$2::bytea AND sequence=1",
                &[&original_action, &stream_id],
            )
            .expect("TASK094_HISTORICAL_EVENT_ACTION_DRIFT_REPAIR"),
        1
    );
    println!("TASK094_HISTORICAL_EVENT_ACTION_INVISIBILITY_MIGRATION_REJECTION_PASS");

    assert_eq!(
        migrator
            .execute(
                "UPDATE ONLY control.task_ledger_events SET action_id='GENERAL_TASK_INTAKE_V1' \
                  WHERE stream_id=$1::bytea AND sequence=1",
                &[&stream_id],
            )
            .expect("TASK094_HISTORICAL_DUAL_ACTION_EVENT_DRIFT_APPLY"),
        1
    );
    assert_eq!(
        migrator
            .execute(
                "UPDATE ONLY control.task_ledger_commands SET action_id='GENERAL_TASK_INTAKE_V1' \
                  WHERE stream_id=$1::bytea AND command_id=$2",
                &[&stream_id, &command_id],
            )
            .expect("TASK094_HISTORICAL_DUAL_ACTION_COMMAND_DRIFT_APPLY"),
        1
    );
    assert_failed_v7_migration_preserves_v6(
        migrator,
        target,
        duplicate_fixture,
        "HISTORICAL_DUAL_ACTION_INVISIBILITY",
    );
    assert_eq!(
        migrator
            .execute(
                "UPDATE ONLY control.task_ledger_events SET action_id=$1 \
                  WHERE stream_id=$2::bytea AND sequence=1",
                &[&original_action, &stream_id],
            )
            .expect("TASK094_HISTORICAL_DUAL_ACTION_EVENT_DRIFT_REPAIR"),
        1
    );
    assert_eq!(
        migrator
            .execute(
                "UPDATE ONLY control.task_ledger_commands SET action_id=$1 \
                  WHERE stream_id=$2::bytea AND command_id=$3",
                &[&original_action, &stream_id, &command_id],
            )
            .expect("TASK094_HISTORICAL_DUAL_ACTION_COMMAND_DRIFT_REPAIR"),
        1
    );
    println!("TASK094_HISTORICAL_DUAL_ACTION_INVISIBILITY_MIGRATION_REJECTION_PASS");
}

fn prove_historical_secret_client_migration_rejected(
    config: &LiveConfig,
    target: &MigrationTarget,
    migrator: &mut Client,
    claim: &TaskIngressClaim,
    duplicate_fixture: &HistoricalDuplicateFixture,
) {
    let original_command_id = format!("mcp-submit:{}", claim.client_request_id());
    let rejected_command_id = "mcp-submit:secret:value";
    rewrite_historical_canary_command_id(
        config,
        target,
        claim,
        &original_command_id,
        rejected_command_id,
    );
    let failure = apply_migrations(migrator, target)
        .expect_err("TASK094_SECRET_SHAPED_HISTORICAL_CLIENT_ID_MUST_REJECT_V7");
    assert_eq!(
        failure.kind().code(),
        "STORE_MIGRATION_TRANSACTION_FAILED",
        "TASK094_SECRET_SHAPED_HISTORY_FAILURE_KIND"
    );
    let retained = migrator
        .query_one(
            "SELECT \
                (SELECT current_schema_version FROM ONLY control.schema_compatibility \
                  WHERE singleton), \
                (SELECT count(*)::bigint FROM ONLY control.migration_history \
                  WHERE ordinal=8), \
                (SELECT global_schema_version FROM ONLY \
                    writer_lease.writer_lease_extension_identity WHERE singleton), \
                pg_catalog.to_regclass('control.task_ingress_claims')::text, \
                pg_catalog.to_regclass('control.task_ingress_historical_ambiguities')::text, \
                pg_catalog.to_regclass('control.task_submission_envelopes')::text",
            &[],
        )
        .expect("TASK094_SECRET_SHAPED_HISTORY_ROLLBACK_STATE");
    assert_eq!(retained.get::<_, i16>(0), 6);
    assert_eq!(retained.get::<_, i64>(1), 0);
    assert_eq!(retained.get::<_, i16>(2), 6);
    assert_eq!(retained.get::<_, Option<String>>(3), None);
    assert_eq!(retained.get::<_, Option<String>>(4), None);
    assert_eq!(retained.get::<_, Option<String>>(5), None);
    assert_eq!(
        historical_duplicate_ledger_fingerprint(migrator, &duplicate_fixture.claims),
        duplicate_fixture.ledger_fingerprint,
        "TASK094_FAILED_V7_MUST_NOT_REWRITE_DUPLICATE_HISTORY"
    );
    rewrite_historical_canary_command_id(
        config,
        target,
        claim,
        rejected_command_id,
        &original_command_id,
    );
    let restored = migrator
        .query_one(
            "SELECT \
                (SELECT count(*)::bigint FROM ONLY control.task_ledger_commands \
                  WHERE stream_id=$1::bytea AND command_id=$2), \
                (SELECT count(*)::bigint FROM ONLY control.task_ledger_events \
                  WHERE stream_id=$1::bytea AND sequence=1 AND command_id=$2), \
                (SELECT current_schema_version FROM ONLY control.schema_compatibility \
                  WHERE singleton)",
            &[
                &task094_digest_bytes(claim.stream_id()),
                &original_command_id,
            ],
        )
        .expect("TASK094_RESTORED_HISTORY_AFTER_SECRET_REJECTION");
    assert_eq!(restored.get::<_, i64>(0), 1);
    assert_eq!(restored.get::<_, i64>(1), 1);
    assert_eq!(restored.get::<_, i16>(2), 6);
    println!("TASK094_HISTORICAL_SECRET_CLIENT_MIGRATION_REJECTION_PASS");
}

fn install_exact_f252_v4_rebind(client: &mut Client) {
    let current_sql = include_str!("../../../db/extensions/writer-lease/v4-rebind.sql");
    let anchor = "AND c.manifest_sha256 =";
    let remainder = current_sql
        .split_once(anchor)
        .map(|(_, remainder)| remainder)
        .expect("TASK094_CURRENT_V7_REBIND_MANIFEST_ANCHOR");
    let first_quote = remainder
        .find('\'')
        .expect("TASK094_CURRENT_V7_REBIND_MANIFEST_QUOTE");
    let current_manifest = remainder
        .get(first_quote + 1..first_quote + 65)
        .expect("TASK094_CURRENT_V7_REBIND_MANIFEST_VALUE");
    assert!(is_lower_hex(current_manifest, 64));
    assert_ne!(current_manifest, F252_V7_MANIFEST_SHA256);
    assert_eq!(current_sql.matches(current_manifest).count(), 4);
    let legacy_sql = current_sql.replace(current_manifest, F252_V7_MANIFEST_SHA256);
    assert_eq!(legacy_sql.matches(F252_V7_MANIFEST_SHA256).count(), 4);
    client
        .batch_execute(&legacy_sql)
        .expect("TASK094_INSTALL_EXACT_F252_V4_REBIND");
}

fn assert_v6_legacy_f252_rebind_profile(
    client: &mut Client,
    target: &V3ExtensionTarget,
    marker: &str,
) {
    assert_eq!(
        inspect_v3_bootstrap_profile(client, target)
            .unwrap_or_else(|_| panic!("TASK094_{marker}_LEGACY_PROFILE_FAILED")),
        V3BootstrapProfile::V6V4BridgeLegacyF252Rebind,
        "TASK094_{marker}_LEGACY_PROFILE"
    );
}

fn assert_v6_migration_state(client: &mut Client, marker: &str) {
    let row = client
        .query_one(
            "SELECT c.current_schema_version, \
                    (SELECT count(*)::bigint FROM ONLY control.migration_history WHERE ordinal=8), \
                    w.global_schema_version \
               FROM ONLY control.schema_compatibility AS c \
               CROSS JOIN ONLY writer_lease.writer_lease_extension_identity AS w \
              WHERE c.singleton AND w.singleton",
            &[],
        )
        .unwrap_or_else(|_| panic!("TASK094_{marker}_V6_STATE_QUERY"));
    assert_eq!(row.get::<_, i16>(0), 6, "TASK094_{marker}_SCHEMA_VERSION");
    assert_eq!(row.get::<_, i64>(1), 0, "TASK094_{marker}_V7_HISTORY");
    assert_eq!(row.get::<_, i16>(2), 6, "TASK094_{marker}_WRITER_VERSION");
}

#[test]
#[ignore = "requires the marker-owned disposable PostgreSQL live fixture"]
fn task094_writer_v3_transition_composition() {
    let config = LiveConfig::from_environment().expect("TASK094_LIVE_GATE_NOT_CONFIGURED");
    let mut admin = config.connect("postgres", "lattice-devos-task094-admin");
    create_fixed_roles(&mut admin, &config.password);
    let target = provision_database(&config, &mut admin);
    drop(admin);

    println!("TASK094_STAGE_FRESH_V5_ENTER");
    let mut migrator = config.role_client(
        target.database_name(),
        DatabaseRole::Migrator,
        REQUIRED_APPLICATION_NAME,
    );
    assert_eq!(
        apply_migrations(&mut migrator, &target).expect("TASK094_FRESH_V5_FAILED"),
        MigrationApplyOutcome::Applied {
            executable_count: 5
        },
        "TASK094_FRESH_MUST_STOP_AT_EXACT_V5"
    );
    drop(migrator);
    println!("TASK094_STAGE_FRESH_V5_PASS");

    println!("TASK094_STAGE_MEMORY_V3_ENTER");
    install_codebase_memory_v2(&config, &target);
    upgrade_codebase_memory_v3(&config, &target);
    println!("TASK094_STAGE_MEMORY_V3_PASS");

    let database_identity = ContentDigest::from_sha256(
        target
            .expected_database_identity_sha256()
            .as_str()
            .to_owned(),
    )
    .expect("TASK094_DATABASE_IDENTITY_DIGEST");
    let writer_v2_target = ExtensionTarget::new(
        target.database_name().to_owned(),
        database_identity.clone(),
        ContentDigest::from_sha256(CURRENT_V5_MANIFEST_SHA256.to_owned())
            .expect("TASK094_V5_MANIFEST_DIGEST"),
        ContentDigest::from_sha256(CODEBASE_MEMORY_V3_MANIFEST_SHA256.to_owned())
            .expect("TASK094_MEMORY_V3_MANIFEST_DIGEST"),
    )
    .expect("TASK094_WRITER_V2_TARGET");
    let writer_v3_target =
        V3ExtensionTarget::new(target.database_name().to_owned(), database_identity.clone())
            .expect("TASK094_WRITER_V3_TARGET");
    let writer_v4_target =
        V4ExtensionTarget::new(target.database_name().to_owned(), database_identity)
            .expect("TASK094_WRITER_V4_TARGET");
    let mut migrator = config.role_client(
        target.database_name(),
        DatabaseRole::Migrator,
        REQUIRED_APPLICATION_NAME,
    );
    println!("TASK094_STAGE_WRITER_V2_ENTER");
    assert_eq!(
        apply_extension(&mut migrator, &writer_v2_target).expect("TASK094_WRITER_V2_CURRENT"),
        ExtensionApplyOutcome::Installed
    );
    println!("TASK094_STAGE_WRITER_V2_PASS");
    println!("TASK094_STAGE_WRITER_V3_BRIDGE_ENTER");
    assert_eq!(
        apply_v3_extension(&mut migrator, &writer_v3_target).expect("TASK094_WRITER_V3_BRIDGE"),
        ExtensionApplyOutcome::Bridged
    );
    println!("TASK094_STAGE_WRITER_V3_BRIDGE_PASS");
    migrator
        .batch_execute("COMMENT ON TABLE writer_lease.writer_lease_heads IS 'TASK094_DRIFT'")
        .expect("TASK094_WRITER_V3_BRIDGE_RELATION_DRIFT_APPLY");
    let v3_bridge_relation_failure = inspect_v3_bootstrap_profile(&mut migrator, &writer_v3_target)
        .expect_err("TASK094_WRITER_V3_BRIDGE_RELATION_DRIFT_MUST_FAIL");
    assert_eq!(
        v3_bridge_relation_failure.kind(),
        lattice_postgres_writer_lease::ExtensionSetupErrorKind::PartialOrCollidingProfile,
        "TASK094_WRITER_V3_BRIDGE_RELATION_DRIFT_ERROR_KIND"
    );
    migrator
        .batch_execute(
            "COMMENT ON TABLE writer_lease.writer_lease_heads IS \
             'LATTICE_WRITER_LEASE_HEADS_V1'",
        )
        .expect("TASK094_WRITER_V3_BRIDGE_RELATION_DRIFT_REPAIR");
    assert_eq!(
        inspect_v3_bootstrap_profile(&mut migrator, &writer_v3_target)
            .expect("TASK094_WRITER_V3_BRIDGE_RELATION_REPAIR_PROFILE"),
        V3BootstrapProfile::V5Bridge
    );
    println!("TASK094_V5_WRITER_V3_BRIDGE_RELATION_DRIFT_REJECTED_AND_REPAIRED");

    println!("TASK094_STAGE_REBIND_FAILURE_ATOMICITY_ENTER");
    insert_active_head_failure_fixture(&mut migrator);
    assert_active_head_rebind_sqlstate(&mut migrator);
    assert_rebind_failure_preserves_exact_v5_bridge(&mut migrator, &target, "active_head");
    migrator
        .execute(
            "DELETE FROM ONLY writer_lease.writer_lease_heads WHERE project_id='task094-failure-active'",
            &[],
        )
        .expect("TASK094_REMOVE_ACTIVE_HEAD_FAILURE_FIXTURE");
    assert_drift_failure_preserves_state(
        &mut migrator,
        &target,
        "identity",
        "UPDATE ONLY writer_lease.writer_lease_extension_identity \
         SET global_manifest_sha256 = repeat('a', 64) WHERE singleton",
        "UPDATE ONLY writer_lease.writer_lease_extension_identity \
         SET global_manifest_sha256 = $1 WHERE singleton",
        &[&CURRENT_V5_MANIFEST_SHA256],
    );
    assert_ledger_drift_preserves_state(&mut migrator, &target);
    assert_acl_drift_preserves_state(&mut migrator, &target);
    assert_exact_v5_bridge(&mut migrator);
    println!("TASK094_STAGE_REBIND_FAILURE_ATOMICITY_PASS");

    println!("TASK094_STAGE_STORE_V6_ENTER");
    assert_eq!(
        apply_migrations(&mut migrator, &target).expect("TASK094_STORE_V6_FAILED"),
        MigrationApplyOutcome::Applied {
            executable_count: 1
        },
        "TASK094_EXACT_V5_TO_V6_OUTCOME"
    );
    let evidence = verify_postgres_schema(&mut migrator, &target, DatabaseRole::Migrator)
        .expect("TASK094_STORE_V6_SCHEMA_VERIFY");
    assert_eq!(evidence.schema_version(), 6);
    prove_store_catalog_drift_case(
        &config,
        &target,
        &mut migrator,
        "WRITER_V3_CURRENT_TABLE_ACL",
        "GRANT SELECT ON TABLE writer_lease.writer_lease_heads TO lattice_guardian",
        "REVOKE SELECT ON TABLE writer_lease.writer_lease_heads FROM lattice_guardian",
        PostgresStoreSetupErrorKind::PermissionDenied,
        6,
    );
    prove_store_catalog_drift_case(
        &config,
        &target,
        &mut migrator,
        "CONTROL_SCHEMA_HEADER",
        "COMMENT ON SCHEMA control IS 'TASK094_DRIFT'",
        "COMMENT ON SCHEMA control IS 'LATTICE_DEVOS_CONTROL_SCHEMA_V6'",
        PostgresStoreSetupErrorKind::CorruptCatalog,
        6,
    );
    let historical_claim = persist_historical_canary_before_v4(&config, &target, &mut migrator);
    let duplicate_fixture =
        persist_historical_duplicate_canaries_before_v4(&config, &target, &mut migrator);
    println!("TASK094_STAGE_WRITER_V4_BRIDGE_ENTER");
    assert_eq!(
        apply_v4_extension(&mut migrator, &writer_v4_target).expect("TASK094_WRITER_V4_BRIDGE"),
        ExtensionApplyOutcome::Bridged
    );
    assert_eq!(
        inspect_v3_bootstrap_profile(&mut migrator, &writer_v3_target)
            .expect("TASK094_WRITER_V4_BRIDGE_PROFILE"),
        V3BootstrapProfile::V6V4Bridge
    );
    println!("TASK094_STAGE_WRITER_V4_BRIDGE_PASS");
    prove_historical_binding_migration_rejected(
        &target,
        &mut migrator,
        &historical_claim,
        &duplicate_fixture,
    );
    prove_historical_secret_client_migration_rejected(
        &config,
        &target,
        &mut migrator,
        &historical_claim,
        &duplicate_fixture,
    );
    migrator
        .batch_execute(
            "CREATE TYPE writer_lease.task094_writer_bridge_drift_type AS ENUM ('DRIFT')",
        )
        .expect("TASK094_WRITER_V4_BRIDGE_TYPE_DRIFT_APPLY");
    let bridge_failure = apply_migrations(&mut migrator, &target)
        .expect_err("TASK094_WRITER_V4_BRIDGE_TYPE_DRIFT_MUST_FAIL");
    migrator
        .batch_execute("DROP TYPE writer_lease.task094_writer_bridge_drift_type")
        .expect("TASK094_WRITER_V4_BRIDGE_TYPE_DRIFT_REPAIR");
    assert_eq!(
        bridge_failure.kind(),
        PostgresStoreSetupErrorKind::CorruptCatalog,
        "TASK094_WRITER_V4_BRIDGE_TYPE_DRIFT_ERROR_KIND"
    );
    assert_eq!(
        inspect_v3_bootstrap_profile(&mut migrator, &writer_v3_target)
            .expect("TASK094_WRITER_V4_BRIDGE_TYPE_REPAIR_PROFILE"),
        V3BootstrapProfile::V6V4Bridge
    );
    println!("TASK094_V6_WRITER_V4_BRIDGE_TYPE_DRIFT_REJECTED_AND_REPAIRED");

    migrator
        .batch_execute("DROP PROCEDURE writer_lease.writer_lease_rebind_v4()")
        .expect("TASK094_WRITER_V4_REBIND_ABSENCE_APPLY");
    let absent_reapply = apply_v4_extension(&mut migrator, &writer_v4_target)
        .expect_err("TASK094_WRITER_V4_REBIND_ABSENCE_MUST_NOT_REPAIR");
    assert_eq!(
        absent_reapply.kind(),
        lattice_postgres_writer_lease::ExtensionSetupErrorKind::PartialOrCollidingProfile
    );
    let absent_retained: Option<String> = migrator
        .query_one(
            "SELECT pg_catalog.to_regprocedure(\
                'writer_lease.writer_lease_rebind_v4()')::text",
            &[],
        )
        .expect("TASK094_WRITER_V4_REBIND_ABSENCE_QUERY")
        .get(0);
    assert_eq!(absent_retained, None);
    assert_v6_migration_state(&mut migrator, "WRITER_V4_REBIND_ABSENCE");
    migrator
        .batch_execute(include_str!(
            "../../../db/extensions/writer-lease/v4-rebind.sql"
        ))
        .expect("TASK094_WRITER_V4_REBIND_ABSENCE_REPAIR");
    assert_eq!(
        inspect_v3_bootstrap_profile(&mut migrator, &writer_v3_target)
            .expect("TASK094_WRITER_V4_REBIND_ABSENCE_REPAIR_PROFILE"),
        V3BootstrapProfile::V6V4Bridge
    );
    println!("TASK094_WRITER_V4_REBIND_ABSENCE_REJECTED_WITHOUT_REPAIR");

    install_exact_f252_v4_rebind(&mut migrator);
    assert_v6_legacy_f252_rebind_profile(&mut migrator, &writer_v3_target, "F252_REBIND_INITIAL");
    migrator
        .batch_execute(
            "ALTER PROCEDURE writer_lease.writer_lease_rebind_v4() \
             SET statement_timeout = '29s'",
        )
        .expect("TASK094_F252_REBIND_METADATA_DRIFT_APPLY");
    let metadata_inspection = inspect_v3_bootstrap_profile(&mut migrator, &writer_v3_target)
        .expect_err("TASK094_F252_REBIND_METADATA_DRIFT_INSPECTION_MUST_FAIL");
    assert_eq!(
        metadata_inspection.kind(),
        lattice_postgres_writer_lease::ExtensionSetupErrorKind::PartialOrCollidingProfile
    );
    let metadata_writer_apply = apply_v4_extension(&mut migrator, &writer_v4_target)
        .expect_err("TASK094_F252_REBIND_METADATA_DRIFT_WRITER_APPLY_MUST_FAIL");
    assert_eq!(
        metadata_writer_apply.kind(),
        lattice_postgres_writer_lease::ExtensionSetupErrorKind::PartialOrCollidingProfile
    );
    let metadata_migration = apply_migrations(&mut migrator, &target)
        .expect_err("TASK094_F252_REBIND_METADATA_DRIFT_MIGRATION_MUST_FAIL");
    assert_eq!(
        metadata_migration.kind(),
        PostgresStoreSetupErrorKind::CorruptCatalog
    );
    let metadata_retained: String = migrator
        .query_one(
            "SELECT pg_catalog.array_to_string(p.proconfig,',') \
               FROM pg_catalog.pg_proc AS p \
               JOIN pg_catalog.pg_namespace AS n ON n.oid=p.pronamespace \
              WHERE n.nspname='writer_lease' AND p.proname='writer_lease_rebind_v4' \
                AND pg_catalog.pg_get_function_identity_arguments(p.oid)=''",
            &[],
        )
        .expect("TASK094_F252_REBIND_METADATA_RETAINED_QUERY")
        .get(0);
    assert!(metadata_retained.contains("statement_timeout=29s"));
    assert_v6_migration_state(&mut migrator, "F252_REBIND_METADATA_DRIFT");
    migrator
        .batch_execute(
            "ALTER PROCEDURE writer_lease.writer_lease_rebind_v4() \
             SET statement_timeout = '30s'",
        )
        .expect("TASK094_F252_REBIND_METADATA_DRIFT_REPAIR");
    assert_v6_legacy_f252_rebind_profile(
        &mut migrator,
        &writer_v3_target,
        "F252_REBIND_METADATA_REPAIR",
    );

    migrator
        .batch_execute(
            "GRANT EXECUTE ON PROCEDURE writer_lease.writer_lease_rebind_v4() \
             TO lattice_runtime",
        )
        .expect("TASK094_F252_REBIND_ACL_DRIFT_APPLY");
    let acl_inspection = inspect_v3_bootstrap_profile(&mut migrator, &writer_v3_target)
        .expect_err("TASK094_F252_REBIND_ACL_DRIFT_INSPECTION_MUST_FAIL");
    assert_eq!(
        acl_inspection.kind(),
        lattice_postgres_writer_lease::ExtensionSetupErrorKind::PartialOrCollidingProfile
    );
    let acl_writer_apply = apply_v4_extension(&mut migrator, &writer_v4_target)
        .expect_err("TASK094_F252_REBIND_ACL_DRIFT_WRITER_APPLY_MUST_FAIL");
    assert_eq!(
        acl_writer_apply.kind(),
        lattice_postgres_writer_lease::ExtensionSetupErrorKind::PartialOrCollidingProfile
    );
    let acl_migration = apply_migrations(&mut migrator, &target)
        .expect_err("TASK094_F252_REBIND_ACL_DRIFT_MIGRATION_MUST_FAIL");
    assert_eq!(
        acl_migration.kind(),
        PostgresStoreSetupErrorKind::CorruptCatalog
    );
    let acl_retained: bool = migrator
        .query_one(
            "SELECT pg_catalog.has_function_privilege(\
                'lattice_runtime','writer_lease.writer_lease_rebind_v4()','EXECUTE')",
            &[],
        )
        .expect("TASK094_F252_REBIND_ACL_RETAINED_QUERY")
        .get(0);
    assert!(acl_retained, "TASK094_F252_REBIND_ACL_DRIFT_WAS_NORMALIZED");
    assert_v6_migration_state(&mut migrator, "F252_REBIND_ACL_DRIFT");
    migrator
        .batch_execute(
            "REVOKE EXECUTE ON PROCEDURE writer_lease.writer_lease_rebind_v4() \
             FROM lattice_runtime",
        )
        .expect("TASK094_F252_REBIND_ACL_DRIFT_REPAIR");
    assert_v6_legacy_f252_rebind_profile(
        &mut migrator,
        &writer_v3_target,
        "F252_REBIND_ACL_REPAIR",
    );
    println!("TASK094_F252_REBIND_METADATA_AND_ACL_DRIFT_REJECTED_ATOMICALLY");

    assert_eq!(
        apply_v4_extension(&mut migrator, &writer_v4_target)
            .expect("TASK094_F252_REBIND_WRITER_RECONCILIATION_FAILED"),
        ExtensionApplyOutcome::Bridged,
        "TASK094_F252_REBIND_WRITER_RECONCILIATION_OUTCOME"
    );
    assert_eq!(
        inspect_v3_bootstrap_profile(&mut migrator, &writer_v3_target)
            .expect("TASK094_F252_REBIND_WRITER_RECONCILIATION_PROFILE"),
        V3BootstrapProfile::V6V4Bridge
    );
    assert_v6_migration_state(&mut migrator, "F252_REBIND_WRITER_RECONCILED");
    println!("TASK094_F252_REBIND_WRITER_OWNED_RECONCILIATION_PASS");

    assert_eq!(
        apply_migrations(&mut migrator, &target).expect("TASK094_STORE_V7_FAILED"),
        MigrationApplyOutcome::Applied {
            executable_count: 1
        },
        "TASK094_EXACT_V6_TO_V7_OUTCOME"
    );
    let evidence = verify_postgres_schema(&mut migrator, &target, DatabaseRole::Migrator)
        .expect("TASK094_STORE_V7_SCHEMA_VERIFY");
    assert_eq!(evidence.schema_version(), 7);
    prove_historical_duplicate_replay_v7(&config, &target, &mut migrator, &duplicate_fixture);
    prove_historical_duplicate_migration(&config, &target, &mut migrator, &duplicate_fixture);
    prove_v7_verifier_rejects_historical_binding_drift(
        &config,
        &target,
        &mut migrator,
        &historical_claim,
    );
    prove_v7_verifier_rejects_profile_and_lineage_drift(&config, &target, &mut migrator);
    let before_retry = v7_duplicate_migration_fingerprint(&mut migrator, &duplicate_fixture);
    drop(migrator);
    let mut migrator = config.role_client(
        target.database_name(),
        DatabaseRole::Migrator,
        REQUIRED_APPLICATION_NAME,
    );
    assert_eq!(
        apply_migrations(&mut migrator, &target).expect("TASK094_STORE_V7_RETRY_FAILED"),
        MigrationApplyOutcome::AlreadyCurrent
    );
    assert_eq!(
        v7_duplicate_migration_fingerprint(&mut migrator, &duplicate_fixture),
        before_retry,
        "TASK094_STORE_V7_RETRY_MUST_BE_BYTE_EXACT"
    );
    migrator
        .batch_execute("CALL writer_lease.writer_lease_rebind_v4()")
        .expect("TASK094_WRITER_V4_V7_REBIND_RETRY");
    assert_eq!(
        inspect_v3_bootstrap_profile(&mut migrator, &writer_v3_target)
            .expect("TASK094_WRITER_V4_V7_PROFILE"),
        V3BootstrapProfile::V7V4Current
    );
    prove_historical_canary_claim_backfill(&config, &target, &mut migrator, &historical_claim);
    set_task094_runtime_admission(&mut migrator, false);
    let final_evidence = verify_postgres_schema(&mut migrator, &target, DatabaseRole::Migrator)
        .expect("TASK094_FINAL_STOPPED_V7_SCHEMA_VERIFY");
    assert_eq!(final_evidence.schema_version(), 7);
    println!("TASK094_STAGE_STORE_V7_PASS");
    println!(
        "TASK094_TRANSITION_OK database_uuid={} manifest_sha256={}",
        evidence.database_uuid(),
        evidence.manifest_sha256().as_str()
    );
}

fn assert_active_head_rebind_sqlstate(client: &mut Client) {
    let failure = client
        .batch_execute("CALL writer_lease.writer_lease_rebind_v3()")
        .expect_err("TASK094_ACTIVE_HEAD_MUST_REJECT_WRITER_REBIND");
    assert_eq!(
        failure
            .as_db_error()
            .expect("TASK094_ACTIVE_HEAD_DATABASE_ERROR")
            .code(),
        &SqlState::OBJECT_NOT_IN_PREREQUISITE_STATE,
        "TASK094_ACTIVE_HEAD_SQLSTATE_MUST_BE_55000"
    );
}

fn assert_rebind_failure_preserves_exact_v5_bridge(
    client: &mut Client,
    target: &MigrationTarget,
    label: &str,
) {
    let before = v5_bridge_fingerprint(client);
    let failure = apply_migrations(client, target).expect_err("TASK094_REBIND_FAILURE_MUST_FAIL");
    assert_eq!(
        failure.kind().code(),
        "STORE_MIGRATION_TRANSACTION_FAILED",
        "TASK094_{label}_FAILURE_KIND"
    );
    assert_eq!(
        v5_bridge_fingerprint(client),
        before,
        "TASK094_{label}_MUST_ROLL_BACK_HISTORY_COMPATIBILITY_IDENTITY_LEDGER_AND_RUNTIME_ACL"
    );
}

fn assert_drift_failure_preserves_state(
    client: &mut Client,
    target: &MigrationTarget,
    label: &str,
    introduce_sql: &str,
    repair_sql: &str,
    repair_parameters: &[&(dyn postgres::types::ToSql + Sync)],
) {
    let introduced = client
        .execute(introduce_sql, &[])
        .unwrap_or_else(|_| panic!("TASK094_{label}_DRIFT_INTRODUCTION_FAILED"));
    assert_eq!(introduced, 1, "TASK094_{label}_DRIFT_MUST_CHANGE_ONE_ROW");
    let before = v5_bridge_fingerprint(client);
    let failure = apply_migrations(client, target).expect_err("TASK094_DRIFT_MUST_FAIL_CLOSED");
    assert_eq!(
        failure.kind().code(),
        "STORE_MIGRATION_TRANSACTION_FAILED",
        "TASK094_{label}_DRIFT_FAILURE_KIND"
    );
    assert_eq!(
        v5_bridge_fingerprint(client),
        before,
        "TASK094_{label}_DRIFT_MUST_NOT_PARTIALLY_APPLY"
    );
    client
        .execute(repair_sql, repair_parameters)
        .unwrap_or_else(|_| panic!("TASK094_{label}_DRIFT_REPAIR_FAILED"));
}

fn assert_acl_drift_preserves_state(client: &mut Client, target: &MigrationTarget) {
    client
        .batch_execute("GRANT USAGE ON SCHEMA writer_lease TO lattice_runtime")
        .expect("TASK094_ACL_DRIFT_INTRODUCTION_FAILED");
    let before = v5_bridge_fingerprint(client);
    let failure = apply_migrations(client, target).expect_err("TASK094_ACL_DRIFT_MUST_FAIL_CLOSED");
    assert_eq!(
        failure.kind().code(),
        "STORE_POSTGRES_CATALOG_CORRUPT",
        "TASK094_ACL_DRIFT_FAILURE_KIND"
    );
    assert_eq!(
        v5_bridge_fingerprint(client),
        before,
        "TASK094_ACL_DRIFT_MUST_NOT_PARTIALLY_APPLY"
    );
    client
        .batch_execute("REVOKE USAGE ON SCHEMA writer_lease FROM lattice_runtime")
        .expect("TASK094_ACL_DRIFT_REPAIR_FAILED");
}

fn assert_ledger_drift_preserves_state(client: &mut Client, target: &MigrationTarget) {
    client
        .batch_execute(
            "ALTER TABLE writer_lease.writer_lease_extension_ledger \
             DROP CONSTRAINT writer_lease_extension_ledger_profile_v3; \
             UPDATE ONLY writer_lease.writer_lease_extension_ledger \
             SET ledger_ordinal = 3 WHERE ledger_ordinal = 2",
        )
        .expect("TASK094_LEDGER_DRIFT_INTRODUCTION_FAILED");
    let before = v5_bridge_fingerprint(client);
    let failure =
        apply_migrations(client, target).expect_err("TASK094_LEDGER_DRIFT_MUST_FAIL_CLOSED");
    assert_eq!(
        failure.kind().code(),
        "STORE_POSTGRES_CATALOG_CORRUPT",
        "TASK094_LEDGER_DRIFT_FAILURE_KIND"
    );
    assert_eq!(
        v5_bridge_fingerprint(client),
        before,
        "TASK094_LEDGER_DRIFT_MUST_NOT_PARTIALLY_APPLY"
    );
    client
        .batch_execute(
            "UPDATE ONLY writer_lease.writer_lease_extension_ledger \
             SET ledger_ordinal = 2 WHERE ledger_ordinal = 3; \
             ALTER TABLE writer_lease.writer_lease_extension_ledger \
             ADD CONSTRAINT writer_lease_extension_ledger_profile_v3 CHECK (\
                extension_id = 'lattice-writer-lease' AND (\
                    (ledger_ordinal = 1 AND extension_schema_version = 1 \
                     AND global_schema_version = 3 AND required_memory_schema_version = 2 \
                     AND event_kind = 'INSTALLED') OR \
                    (ledger_ordinal = 1 AND extension_schema_version = 2 \
                     AND global_schema_version = 5 AND required_memory_schema_version = 3 \
                     AND event_kind = 'INSTALLED') OR \
                    (ledger_ordinal = 1 AND extension_schema_version = 3 \
                     AND global_schema_version = 6 AND required_memory_schema_version = 3 \
                     AND event_kind = 'INSTALLED') OR \
                    (ledger_ordinal = 2 AND extension_schema_version = 2 \
                     AND global_schema_version = 3 AND required_memory_schema_version = 2 \
                     AND event_kind = 'UPGRADED') OR \
                    (ledger_ordinal = 2 AND extension_schema_version = 3 \
                     AND global_schema_version = 5 AND required_memory_schema_version = 3 \
                     AND event_kind = 'UPGRADED') OR \
                    (ledger_ordinal = 3 AND extension_schema_version = 2 \
                     AND global_schema_version = 5 AND required_memory_schema_version = 3 \
                     AND event_kind = 'REBOUND') OR \
                    (ledger_ordinal = 3 AND extension_schema_version = 3 \
                     AND global_schema_version = 6 AND required_memory_schema_version = 3 \
                     AND event_kind = 'REBOUND') OR \
                    (ledger_ordinal = 4 AND extension_schema_version = 3 \
                     AND global_schema_version = 5 AND required_memory_schema_version = 3 \
                     AND event_kind = 'UPGRADED') OR \
                    (ledger_ordinal = 5 AND extension_schema_version = 3 \
                     AND global_schema_version = 6 AND required_memory_schema_version = 3 \
                     AND event_kind = 'REBOUND')))",
        )
        .expect("TASK094_LEDGER_DRIFT_REPAIR_FAILED");
}

fn insert_active_head_failure_fixture(client: &mut Client) {
    client
        .execute(
            "INSERT INTO writer_lease.writer_lease_heads (\
                 project_id,row_version,snapshot_schema_version,snapshot_bytes,\
                 snapshot_bytes_sha256,snapshot_digest,fencing_high_water,lease_revision,\
                 command_high_water,command_tail_digest,current_status,current_receipt_digest,\
                 current_project_snapshot_id,current_task_id,current_task_revision,\
                 current_task_spec_digest,current_attempt_id,current_lease_id,\
                 current_lease_holder_id,current_worktree_id,current_holder_process_id,\
                 current_holder_process_start_identity,current_daemon_instance_id,\
                 current_daemon_epoch,current_fencing_token,current_expires_at) VALUES (\
                 'task094-failure-active',0,1,decode('01','hex'),\
                 pg_catalog.sha256(decode('01','hex')),decode(repeat('11',32),'hex'),\
                 1,1,0,NULL,'ACTIVE',decode(repeat('12',32),'hex'),'snapshot-094',\
                 'task-094','1',decode(repeat('13',32),'hex'),'attempt-094','lease-094',\
                 'holder-094','worktree-094',1,decode(repeat('14',32),'hex'),'daemon-094',\
                 1,1,'2026-08-24T00:00:00Z')",
            &[],
        )
        .expect("TASK094_INSERT_ACTIVE_HEAD_FAILURE_FIXTURE");
}

fn v5_bridge_fingerprint(client: &mut Client) -> Vec<String> {
    [
        "SELECT pg_catalog.md5(COALESCE(pg_catalog.string_agg(\
             pg_catalog.to_jsonb(t)::text,E'\\n' ORDER BY t.ordinal),'')) \
           FROM ONLY control.migration_history t",
        "SELECT pg_catalog.md5(pg_catalog.to_jsonb(c)::text) \
           FROM ONLY control.schema_compatibility c WHERE c.singleton",
        "SELECT pg_catalog.md5(pg_catalog.to_jsonb(w)::text) \
           FROM ONLY writer_lease.writer_lease_extension_identity w WHERE w.singleton",
        "SELECT pg_catalog.md5(COALESCE(pg_catalog.string_agg(\
             pg_catalog.to_jsonb(l)::text,E'\\n' ORDER BY l.ledger_ordinal),'')) \
           FROM ONLY writer_lease.writer_lease_extension_ledger l",
        "SELECT pg_catalog.md5(COALESCE(pg_catalog.string_agg(\
             p.proname::text || ':' || \
             pg_catalog.has_function_privilege('lattice_runtime',p.oid,'EXECUTE')::text, \
             E'\\n' ORDER BY p.proname,pg_catalog.pg_get_function_identity_arguments(p.oid)),'')) \
           FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
          WHERE n.nspname='writer_lease'",
    ]
    .into_iter()
    .map(|query| {
        client
            .query_one(query, &[])
            .unwrap_or_else(|_| panic!("TASK094_V5_BRIDGE_FINGERPRINT_QUERY_FAILED"))
            .get(0)
    })
    .collect()
}

fn assert_exact_v5_bridge(client: &mut Client) {
    let row = client
        .query_one(
            "SELECT \
               (SELECT pg_catalog.count(*) FROM ONLY control.migration_history) = 6, \
               NOT EXISTS (SELECT 1 FROM ONLY control.migration_history WHERE ordinal=7), \
               (SELECT manifest_sha256=$1 AND current_schema_version=5 \
                         AND min_reader=5 AND max_reader=5 AND min_writer=5 AND max_writer=5 \
                  FROM ONLY control.schema_compatibility WHERE singleton), \
               (SELECT extension_schema_version=3 AND global_schema_version=5 \
                         AND global_manifest_sha256=$1 \
                  FROM ONLY writer_lease.writer_lease_extension_identity WHERE singleton), \
               (SELECT pg_catalog.string_agg(ledger_ordinal::text || ':' || event_kind::text || \
                   ':' || extension_schema_version::text || ':' || global_schema_version::text, \
                   ',' ORDER BY ledger_ordinal) \
                  FROM ONLY writer_lease.writer_lease_extension_ledger) = '1:INSTALLED:2:5,2:UPGRADED:3:5', \
               NOT pg_catalog.has_schema_privilege('lattice_runtime','writer_lease','USAGE'), \
               (SELECT pg_catalog.count(*) FROM pg_catalog.pg_proc p \
                 JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
                 WHERE n.nspname='writer_lease' \
                   AND pg_catalog.has_function_privilege('lattice_runtime',p.oid,'EXECUTE')) = 0",
            &[&CURRENT_V5_MANIFEST_SHA256],
        )
        .expect("TASK094_EXACT_V5_BRIDGE_QUERY");
    for index in 0..7 {
        assert!(
            row.get::<_, bool>(index),
            "TASK094_EXACT_V5_BRIDGE_ASSERTION_{index}"
        );
    }
}

fn create_fixed_roles(admin: &mut Client, password: &str) {
    let quoted_password: String = admin
        .query_one("SELECT quote_literal($1::text)", &[&password])
        .expect("TASK094_PASSWORD_QUOTE_FAILED")
        .get(0);
    admin
        .batch_execute(&format!(
            "CREATE ROLE lattice_migrator NOLOGIN NOSUPERUSER INHERIT NOCREATEDB NOCREATEROLE \
                 NOREPLICATION NOBYPASSRLS CONNECTION LIMIT -1; \
             CREATE ROLE lattice_runtime NOLOGIN NOSUPERUSER INHERIT NOCREATEDB NOCREATEROLE \
                 NOREPLICATION NOBYPASSRLS CONNECTION LIMIT -1; \
             CREATE ROLE lattice_guardian NOLOGIN NOSUPERUSER INHERIT NOCREATEDB NOCREATEROLE \
                 NOREPLICATION NOBYPASSRLS CONNECTION LIMIT -1; \
             CREATE ROLE lattice_readonly NOLOGIN NOSUPERUSER INHERIT NOCREATEDB NOCREATEROLE \
                 NOREPLICATION NOBYPASSRLS CONNECTION LIMIT -1; \
             CREATE ROLE lattice_migrator_login LOGIN NOSUPERUSER NOINHERIT NOCREATEDB NOCREATEROLE \
                 NOREPLICATION NOBYPASSRLS CONNECTION LIMIT -1 PASSWORD {quoted_password}; \
             CREATE ROLE lattice_runtime_login LOGIN NOSUPERUSER NOINHERIT NOCREATEDB NOCREATEROLE \
                 NOREPLICATION NOBYPASSRLS CONNECTION LIMIT -1 PASSWORD {quoted_password}; \
             CREATE ROLE lattice_guardian_login LOGIN NOSUPERUSER NOINHERIT NOCREATEDB NOCREATEROLE \
                 NOREPLICATION NOBYPASSRLS CONNECTION LIMIT -1 PASSWORD {quoted_password}; \
             CREATE ROLE lattice_readonly_login LOGIN NOSUPERUSER NOINHERIT NOCREATEDB NOCREATEROLE \
                 NOREPLICATION NOBYPASSRLS CONNECTION LIMIT -1 PASSWORD {quoted_password}; \
             GRANT lattice_migrator TO lattice_migrator_login WITH ADMIN FALSE, INHERIT FALSE, SET TRUE; \
             GRANT lattice_runtime TO lattice_runtime_login WITH ADMIN FALSE, INHERIT FALSE, SET TRUE; \
             GRANT lattice_guardian TO lattice_guardian_login WITH ADMIN FALSE, INHERIT FALSE, SET TRUE; \
             GRANT lattice_readonly TO lattice_readonly_login WITH ADMIN FALSE, INHERIT FALSE, SET TRUE; \
             REVOKE ALL ON DATABASE postgres FROM PUBLIC; \
             REVOKE ALL ON DATABASE template0 FROM PUBLIC; \
             REVOKE ALL ON DATABASE template1 FROM PUBLIC"
        ))
        .expect("TASK094_ROLE_PROVISION_FAILED");
}

fn provision_database(config: &LiveConfig, admin: &mut Client) -> MigrationTarget {
    let target = config.target();
    let quoted_name = quoted_database_name(target.database_name());
    admin
        .batch_execute(&format!(
            "CREATE DATABASE {quoted_name} OWNER lattice_migrator"
        ))
        .expect("TASK094_DATABASE_CREATE_FAILED");
    set_exact_database_access(admin, target.database_name());
    set_exact_pre_role_function_access(config, target.database_name());
    admin
        .batch_execute(&format!(
            "SET ROLE lattice_migrator; COMMENT ON DATABASE {quoted_name} IS '{}'; RESET ROLE",
            target.database_comment()
        ))
        .expect("TASK094_DATABASE_BOUNDARY_PROVISION_FAILED");
    target
}

fn set_exact_database_access(admin: &mut Client, target_database: &str) {
    let database_names = admin
        .query(
            "SELECT datname::text FROM pg_database ORDER BY datname",
            &[],
        )
        .expect("TASK094_DATABASE_INVENTORY_FAILED");
    for row in &database_names {
        let database: String = row.get(0);
        let quoted = quoted_database_name(&database);
        admin
            .batch_execute(&format!(
                "REVOKE ALL ON DATABASE {quoted} FROM PUBLIC; \
                 REVOKE ALL ON DATABASE {quoted} FROM lattice_migrator_login, lattice_runtime_login, \
                 lattice_guardian_login, lattice_readonly_login"
            ))
            .expect("TASK094_DATABASE_ACCESS_REVOKE_FAILED");
    }
    let quoted_target = quoted_database_name(target_database);
    admin
        .batch_execute(&format!(
            "SET ROLE lattice_migrator; \
             GRANT CONNECT ON DATABASE {quoted_target} TO lattice_migrator, lattice_runtime, \
                 lattice_guardian, lattice_readonly, lattice_migrator_login, lattice_runtime_login, \
                 lattice_guardian_login, lattice_readonly_login; RESET ROLE"
        ))
        .expect("TASK094_DATABASE_ACCESS_GRANT_FAILED");
}

fn set_exact_pre_role_function_access(config: &LiveConfig, database: &str) {
    let mut admin = config.connect(database, "task094-function-boundary-provision");
    admin
        .batch_execute(
            "REVOKE ALL PRIVILEGES ON FUNCTION \
                 pg_catalog.lo_creat(integer), pg_catalog.lo_create(oid), \
                 pg_catalog.lo_from_bytea(oid, bytea), pg_catalog.lo_import(text), \
                 pg_catalog.lo_import(text, oid), \
                 pg_catalog.pg_logical_emit_message(boolean, text, text, boolean), \
                 pg_catalog.pg_logical_emit_message(boolean, text, bytea, boolean), \
                 pg_catalog.pg_advisory_lock(bigint), pg_catalog.pg_advisory_lock(integer, integer), \
                 pg_catalog.pg_advisory_lock_shared(bigint), \
                 pg_catalog.pg_advisory_lock_shared(integer, integer), \
                 pg_catalog.pg_try_advisory_lock(bigint), \
                 pg_catalog.pg_try_advisory_lock(integer, integer), \
                 pg_catalog.pg_try_advisory_lock_shared(bigint), \
                 pg_catalog.pg_try_advisory_lock_shared(integer, integer), \
                 pg_catalog.pg_advisory_xact_lock(bigint), \
                 pg_catalog.pg_advisory_xact_lock(integer, integer), \
                 pg_catalog.pg_advisory_xact_lock_shared(bigint), \
                 pg_catalog.pg_advisory_xact_lock_shared(integer, integer), \
                 pg_catalog.pg_try_advisory_xact_lock(bigint), \
                 pg_catalog.pg_try_advisory_xact_lock(integer, integer), \
                 pg_catalog.pg_try_advisory_xact_lock_shared(bigint), \
                 pg_catalog.pg_try_advisory_xact_lock_shared(integer, integer), \
                 pg_catalog.pg_cancel_backend(integer), \
                 pg_catalog.pg_terminate_backend(integer, bigint), \
                 pg_catalog.pg_export_snapshot(), pg_catalog.pg_current_xact_id(), \
                 pg_catalog.txid_current() FROM PUBLIC, lattice_migrator, lattice_runtime, \
                 lattice_guardian, lattice_readonly, lattice_migrator_login, lattice_runtime_login, \
                 lattice_guardian_login, lattice_readonly_login; \
             GRANT EXECUTE ON FUNCTION pg_catalog.pg_try_advisory_lock(bigint), \
                 pg_catalog.pg_advisory_xact_lock(bigint), pg_catalog.pg_current_xact_id() \
                 TO lattice_migrator",
        )
        .expect("TASK094_FUNCTION_BOUNDARY_PROVISION_FAILED");
}

fn install_codebase_memory_v2(config: &LiveConfig, target: &MigrationTarget) {
    let mut client = config.role_client(
        target.database_name(),
        DatabaseRole::Migrator,
        REQUIRED_APPLICATION_NAME,
    );
    let mut transaction = client
        .build_transaction()
        .isolation_level(IsolationLevel::ReadCommitted)
        .start()
        .expect("TASK094_MEMORY_V2_FIXTURE_TRANSACTION_FAILED");
    transaction
        .batch_execute("SET LOCAL search_path = pg_catalog; SET LOCAL row_security = on")
        .expect("TASK094_MEMORY_V2_FIXTURE_HARDEN_FAILED");
    transaction
        .batch_execute(CODEBASE_MEMORY_V2_SQL)
        .expect("TASK094_MEMORY_V2_FIXTURE_SQL_FAILED");
    transaction
        .execute(
            "INSERT INTO memory.codebase_memory_extension_identity (singleton, extension_id, \
             extension_schema_version, extension_path, extension_sql_sha256, extension_manifest_sha256, \
             database_uuid, database_identity_sha256, global_schema_version, global_manifest_sha256) \
             VALUES (true, 'lattice-codebase-memory', 2, $1, $2, $3, $4::text::uuid, $5, 3, $6)",
            &[&CODEBASE_MEMORY_V2_PATH, &CODEBASE_MEMORY_V2_SQL_SHA256, &CODEBASE_MEMORY_V2_MANIFEST_SHA256,
                &target.expected_database_uuid(), &target.expected_database_identity_sha256().as_str(),
                &"09c431df18ad71a4f44239a5d2ddf6b1774b8ffec06c7f9223f0e41757f3d407"],
        )
        .expect("TASK094_MEMORY_V2_FIXTURE_IDENTITY_FAILED");
    transaction
        .execute(
            "INSERT INTO memory.codebase_memory_extension_ledger (ledger_ordinal, singleton, extension_id, \
             extension_schema_version, extension_sql_sha256, extension_manifest_sha256, database_uuid, \
             database_identity_sha256, global_schema_version, global_manifest_sha256, event_kind) \
             VALUES (1, true, 'lattice-codebase-memory', 2, $1, $2, $3::text::uuid, $4, 3, $5, 'INSTALLED')",
            &[&CODEBASE_MEMORY_V2_SQL_SHA256, &CODEBASE_MEMORY_V2_MANIFEST_SHA256,
                &target.expected_database_uuid(), &target.expected_database_identity_sha256().as_str(),
                &"09c431df18ad71a4f44239a5d2ddf6b1774b8ffec06c7f9223f0e41757f3d407"],
        )
        .expect("TASK094_MEMORY_V2_FIXTURE_LEDGER_FAILED");
    transaction
        .commit()
        .expect("TASK094_MEMORY_V2_FIXTURE_COMMIT_FAILED");
}

fn upgrade_codebase_memory_v3(config: &LiveConfig, target: &MigrationTarget) {
    let mut client = config.role_client(
        target.database_name(),
        DatabaseRole::Migrator,
        REQUIRED_APPLICATION_NAME,
    );
    let mut transaction = client
        .build_transaction()
        .isolation_level(IsolationLevel::ReadCommitted)
        .start()
        .expect("TASK094_MEMORY_V3_FIXTURE_TRANSACTION_FAILED");
    transaction
        .batch_execute("SET LOCAL search_path = pg_catalog; SET LOCAL row_security = on")
        .expect("TASK094_MEMORY_V3_FIXTURE_HARDEN_FAILED");
    transaction
        .batch_execute(CODEBASE_MEMORY_V3_SQL)
        .expect("TASK094_MEMORY_V3_FIXTURE_SQL_FAILED");
    assert_eq!(
        transaction.execute(
            "UPDATE ONLY memory.codebase_memory_extension_identity SET extension_schema_version = 3, \
             extension_path = $1, extension_sql_sha256 = $2, extension_manifest_sha256 = $3, \
             global_schema_version = 5, global_manifest_sha256 = $4 WHERE singleton \
             AND extension_schema_version = 2 AND extension_path = $5 AND global_schema_version = 3 \
             AND global_manifest_sha256 = $6",
            &[&CODEBASE_MEMORY_V3_PATH, &CODEBASE_MEMORY_V3_SQL_SHA256, &CODEBASE_MEMORY_V3_MANIFEST_SHA256,
                &CURRENT_V5_MANIFEST_SHA256, &CODEBASE_MEMORY_V2_PATH,
                &"09c431df18ad71a4f44239a5d2ddf6b1774b8ffec06c7f9223f0e41757f3d407"],
        ).expect("TASK094_MEMORY_V3_FIXTURE_IDENTITY_FAILED"),
        1,
        "TASK094_MEMORY_V3_FIXTURE_IDENTITY_MISSING"
    );
    transaction.execute(
        "INSERT INTO memory.codebase_memory_extension_ledger (ledger_ordinal, singleton, extension_id, \
         extension_schema_version, extension_sql_sha256, extension_manifest_sha256, database_uuid, \
         database_identity_sha256, global_schema_version, global_manifest_sha256, event_kind) \
         VALUES (2, true, 'lattice-codebase-memory', 3, $1, $2, $3::text::uuid, $4, 5, $5, 'UPGRADED')",
        &[&CODEBASE_MEMORY_V3_SQL_SHA256, &CODEBASE_MEMORY_V3_MANIFEST_SHA256,
            &target.expected_database_uuid(), &target.expected_database_identity_sha256().as_str(),
            &CURRENT_V5_MANIFEST_SHA256],
    ).expect("TASK094_MEMORY_V3_FIXTURE_LEDGER_FAILED");
    transaction
        .commit()
        .expect("TASK094_MEMORY_V3_FIXTURE_COMMIT_FAILED");
}

fn set_role_sql(role: DatabaseRole) -> &'static str {
    match role {
        DatabaseRole::Migrator => "SET ROLE lattice_migrator",
        DatabaseRole::Runtime => "SET ROLE lattice_runtime",
        DatabaseRole::Guardian => "SET ROLE lattice_guardian",
        DatabaseRole::ReadOnly => "SET ROLE lattice_readonly",
    }
}

fn quoted_database_name(value: &str) -> String {
    assert!(
        value.len() <= 63
            && value
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'),
        "TASK094_DATABASE_IDENTIFIER_INVALID"
    );
    format!("\"{value}\"")
}

fn required_environment(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("TASK094_REQUIRED_ENVIRONMENT_MISSING"))
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
