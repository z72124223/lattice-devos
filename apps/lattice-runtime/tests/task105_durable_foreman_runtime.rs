//! Marker-owned PostgreSQL acceptance for TASK-105.

use std::env;
use std::ffi::OsStr;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use lattice_contracts::{
    AttemptId, ContentDigest, DaemonEpoch, HolderProcessId, ProjectId, RuntimeAdmissionMode,
    RuntimeKind, StoreAuthorityHead, StoreAuthorityRevision, StoreDaemonInstanceId,
    WriterLeaseAuthorityHead,
};
use lattice_postgres_codebase_memory::{
    ExtensionTarget as MemoryExtensionTarget, apply_extension as apply_memory_extension,
    verify_embedded_extension_manifest as verify_memory_manifest,
    verify_embedded_v2_extension_manifest as verify_memory_v2_manifest,
};
use lattice_postgres_foreman::{
    ExtensionApplyOutcome as ForemanExtensionApplyOutcome,
    ExtensionTarget as ForemanExtensionTarget, apply_extension as apply_foreman_extension,
};
use lattice_postgres_store::{
    DatabaseRole, MigrationApplyOutcome, MigrationBootstrapProfile, MigrationTarget,
    POSTGRES_SCHEMA_VERSION, PostgresTaskLedger, PostgresTaskLedgerErrorKind, apply_migrations,
    inspect_migration_profile, migration_manifest, verify_embedded_manifest,
    verify_postgres_schema,
};
use lattice_postgres_writer_lease::{
    ExtensionApplyOutcome, ExtensionSetupErrorKind, ExtensionTarget as WriterExtensionTarget,
    PostgresWriterLease, V3BootstrapProfile, V3ExtensionTarget, V4ExtensionTarget,
    V5ExtensionTarget, apply_extension as apply_writer_extension, apply_v3_extension,
    apply_v4_extension, apply_v5_extension, inspect_v3_bootstrap_profile,
    verify_embedded_v1_extension_manifest as verify_writer_v1_manifest,
    verify_embedded_v2_extension_manifest as verify_writer_v2_manifest,
    verify_embedded_v3_rebind_manifest,
};
use lattice_task_ledger::{VerifiedStream, foreman_coordination_identity};
use lattice_writer_lease::{
    CommandOutcome as LeaseCommandOutcome, WriterLeaseAcquireRequest, WriterLeaseReleaseRequest,
    WriterLeaseRepository, WriterLeaseRepositoryCommand,
};
use postgres::config::SslMode;
use postgres::{Client, Config, NoTls};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const V5_MANIFEST_SHA256: &str = "f92a51fa19c4fe0ffebfc40f20924bd1209bb2441b1bc69f787bc3c4a925425d";
const HISTORICAL_GLOBAL_MANIFEST_SHA256: &str =
    "09c431df18ad71a4f44239a5d2ddf6b1774b8ffec06c7f9223f0e41757f3d407";
const LEGACY_V1_MANIFEST_SHA256: &str =
    "9b126a41e542b71d434b5786e35acb66575967d055a6733b9d6bf0b8c9f0eada";
const WRITER_V3_MANIFEST_SHA256: &str =
    "eab2812fa3d94cd3466d7c003386f805a973fd7def1f16aeb15b52f47dad78e4";
const WRITER_V5_MANIFEST_SHA256: &str =
    "354aa40bc2ed30b7500cffea3a9227d94b766d150798824e39225cf664cca5ad";
const CURRENT_V7_MANIFEST_SHA256: &str =
    "584a446464ab2f7ebd8b85543ba36a6d52b0a708502c39d2653b8814d84313f8";
const LEGACY_V8_MANIFEST_SHA256: &str =
    "01373ed5092e90bf6a9e383955cd70d0fd4e0ed821667f1905b69e313005ea82";
const CURRENT_V8_MANIFEST_SHA256: &str =
    "2b1fcbbc81261c28ab06ac3180f75c2ee458e57a4adc7e49bc399209f421de60";
const FUTURE_V9_MANIFEST_SHA256: &str =
    "16ce514cd8cbfe48b58e887ed20a2ef1db0752280fb481d5057f33b3615d6b86";
const POSTGRES_BOOTSTRAP_GLOBAL_ADVISORY_LOCK: i64 = 0x4c41_5454_4943_4501;

fn independent_manifest_sha256(include_future_v9: bool) -> String {
    fn field(hasher: &mut Sha256, value: &[u8]) {
        hasher.update(
            u64::try_from(value.len())
                .expect("field length")
                .to_be_bytes(),
        );
        hasher.update(value);
    }

    let mut hasher = Sha256::new();
    hasher.update(b"LATTICE_POSTGRES_MIGRATION_MANIFEST_V1\0");
    for entry in migration_manifest() {
        field(&mut hasher, &entry.ordinal().to_be_bytes());
        field(&mut hasher, entry.id().as_bytes());
        field(&mut hasher, entry.path().as_bytes());
        field(
            &mut hasher,
            &u64::try_from(entry.byte_length())
                .expect("migration length")
                .to_be_bytes(),
        );
        field(&mut hasher, entry.sha256().as_bytes());
        field(&mut hasher, entry.status().as_str().as_bytes());
        field(&mut hasher, entry.transaction_mode().as_str().as_bytes());
        for version in [
            entry.schema_version(),
            *entry.reader_compatibility().start(),
            *entry.reader_compatibility().end(),
            *entry.writer_compatibility().start(),
            *entry.writer_compatibility().end(),
        ] {
            field(&mut hasher, &version.to_be_bytes());
        }
    }
    if include_future_v9 {
        field(&mut hasher, &11_u16.to_be_bytes());
        field(&mut hasher, b"0011_unsupported_fixture");
        field(&mut hasher, b"db/migrations/0011_unsupported_fixture.sql");
        field(&mut hasher, &1_u64.to_be_bytes());
        field(&mut hasher, "d".repeat(64).as_bytes());
        field(&mut hasher, b"EXECUTABLE");
        field(&mut hasher, b"RUNNER_OWNED");
        for _ in 0..5 {
            field(&mut hasher, &9_u16.to_be_bytes());
        }
    }
    let digest = hasher.finalize();
    let mut encoded = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut encoded, "{byte:02x}").expect("hex encoding");
    }
    encoded
}

fn future_v9_manifest_sha256() -> String {
    independent_manifest_sha256(true)
}
const FRESH_CATALOG_FINGERPRINT_QUERIES: [&str; 3] = [
    "SELECT pg_catalog.md5(COALESCE(pg_catalog.string_agg(\
       n.nspname||':'||o.rolname||':'||COALESCE(\
       pg_catalog.obj_description(n.oid,'pg_namespace'),'<NULL>'),E'\\n'\
       ORDER BY n.nspname),'')) FROM pg_catalog.pg_namespace n \
       JOIN pg_catalog.pg_roles o ON o.oid=n.nspowner \
      WHERE n.nspname IN ('control','memory','readmodel','writer_lease')",
    "SELECT pg_catalog.md5(COALESCE(pg_catalog.string_agg(\
       n.nspname||':'||c.relname||':'||c.relkind::text,E'\\n'\
       ORDER BY n.nspname,c.relname),'')) FROM pg_catalog.pg_class c \
       JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
      WHERE n.nspname IN ('control','memory','readmodel','writer_lease')",
    "SELECT pg_catalog.md5(COALESCE(pg_catalog.string_agg(\
       n.nspname||':'||p.proname||':'||pg_catalog.oidvectortypes(p.proargtypes),E'\\n'\
       ORDER BY n.nspname,p.proname,p.oid),'')) FROM pg_catalog.pg_proc p \
       JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
      WHERE n.nspname IN ('control','memory','readmodel','writer_lease')",
];

#[derive(Clone)]
struct LiveConfig {
    host: String,
    port: u16,
    password: String,
    run_id: String,
}

impl LiveConfig {
    fn from_environment() -> Option<Self> {
        if env::var("LATTICE_TASK105_LIVE").ok().as_deref() != Some("1") {
            return None;
        }
        assert_eq!(required("LATTICE_TASK105_PHASE"), "durable_foreman_restart");
        let host = required("LATTICE_TASK019_HOST");
        let port = required("LATTICE_TASK019_PORT")
            .parse::<u16>()
            .expect("TASK105_PORT_INVALID");
        let password = required("LATTICE_TASK019_PASSWORD");
        let run_id = required("LATTICE_TASK019_RUN_ID");
        assert_eq!(host, "127.0.0.1");
        assert!(port != 0 && port != 5432 && port != 58_743 && port != 4317);
        assert_eq!(run_id.len(), 32);
        assert!(
            run_id
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        );
        Some(Self {
            host,
            port,
            password,
            run_id,
        })
    }

    fn database_name(&self) -> String {
        format!("lattice_task019_{}_base", &self.run_id[..8])
    }

    fn child_database(&self, discriminator: u32) -> Self {
        assert_ne!(discriminator, 0);
        let prefix = u32::from_str_radix(&self.run_id[..8], 16).expect("TASK105_CHILD_RUN_PREFIX")
            ^ discriminator;
        Self {
            host: self.host.clone(),
            port: self.port,
            password: self.password.clone(),
            run_id: format!("{prefix:08x}{}", &self.run_id[8..]),
        }
    }

    fn bootstrap_client(&self) -> Client {
        self.try_bootstrap_client()
            .expect("TASK105_BOOTSTRAP_CONNECT")
    }

    fn try_bootstrap_client(&self) -> Result<Client, &'static str> {
        let mut config = Config::new();
        config
            .host(&self.host)
            .port(self.port)
            .user("runtime_bootstrap")
            .password(&self.password)
            .dbname(&self.database_name())
            .application_name("lattice-task105-migration-observer")
            .ssl_mode(SslMode::Disable);
        config
            .connect(NoTls)
            .map_err(|_| "TASK105_BOOTSTRAP_CONNECT")
    }

    fn runtime_client(&self) -> Client {
        let mut config = Config::new();
        config
            .host(&self.host)
            .port(self.port)
            .user("lattice_runtime_login")
            .password(&self.password)
            .dbname(&self.database_name())
            .application_name("lattice-devos-task019")
            .ssl_mode(SslMode::Disable);
        let mut client = config.connect(NoTls).expect("TASK105_RUNTIME_CONNECT");
        client
            .batch_execute("SET ROLE lattice_runtime")
            .expect("TASK105_RUNTIME_ROLE");
        client
    }

    fn runtime_login_enabled(&self) -> Result<bool, &'static str> {
        self.bootstrap_client()
            .query_one(
                "SELECT rolcanlogin FROM pg_catalog.pg_roles WHERE rolname='lattice_runtime_login'",
                &[],
            )
            .map(|row| row.get(0))
            .map_err(|_| "TASK105_RUNTIME_LOGIN_READ")
    }

    fn alter_runtime_login(&self, enabled: bool) -> Result<(), &'static str> {
        self.bootstrap_client()
            .batch_execute(if enabled {
                "ALTER ROLE lattice_runtime_login LOGIN"
            } else {
                "ALTER ROLE lattice_runtime_login NOLOGIN"
            })
            .map_err(|_| "TASK105_RUNTIME_LOGIN_ALTER")
    }

    fn revoke_login_database_privileges(&self) {
        self.try_revoke_login_database_privileges()
            .expect("TASK105_CAPABILITY_HANDOFF_REVOKE");
    }

    fn try_revoke_login_database_privileges(&self) -> Result<(), &'static str> {
        let mut config = Config::new();
        config
            .host(&self.host)
            .port(self.port)
            .user("runtime_bootstrap")
            .password(&self.password)
            .dbname("postgres")
            .application_name("lattice-task105-capability-handoff")
            .ssl_mode(SslMode::Disable);
        config
            .connect(NoTls)
            .map_err(|_| "TASK105_CAPABILITY_HANDOFF_CONNECT")?
            .batch_execute(&format!(
                "REVOKE ALL ON DATABASE {} FROM \
                 lattice_migrator_login,lattice_runtime_login,\
                 lattice_guardian_login,lattice_readonly_login",
                self.database_name()
            ))
            .map_err(|_| "TASK105_CAPABILITY_HANDOFF_REVOKE")
    }

    fn try_drop_database(&self) -> Result<(), &'static str> {
        let database = self.database_name();
        let mut config = Config::new();
        config
            .host(&self.host)
            .port(self.port)
            .user("runtime_bootstrap")
            .password(&self.password)
            .dbname("postgres")
            .application_name("lattice-task105-child-cleanup")
            .ssl_mode(SslMode::Disable);
        let mut client = config
            .connect(NoTls)
            .map_err(|_| "TASK105_CHILD_CLEANUP_CONNECT")?;
        let active: i64 = client
            .query_one(
                "SELECT pg_catalog.count(*) FROM pg_catalog.pg_stat_activity \
                  WHERE datname=$1 AND pid<>pg_catalog.pg_backend_pid()",
                &[&database],
            )
            .map_err(|_| "TASK105_CHILD_CLEANUP_ACTIVITY")?
            .get(0);
        if active != 0 {
            return Err("TASK105_CHILD_CLEANUP_ACTIVE");
        }
        client
            .batch_execute(&format!("DROP DATABASE {database}"))
            .map_err(|_| "TASK105_CHILD_CLEANUP_DROP")
    }

    fn assert_login_capability(&self, expected_connect: bool) {
        let roles = [
            ("lattice_migrator", "lattice_migrator_login"),
            ("lattice_runtime", "lattice_runtime_login"),
            ("lattice_guardian", "lattice_guardian_login"),
            ("lattice_readonly", "lattice_readonly_login"),
        ];
        let database = self.database_name();
        let mut observer = Config::new();
        observer
            .host(&self.host)
            .port(self.port)
            .user("runtime_bootstrap")
            .password(&self.password)
            .dbname("postgres")
            .application_name("lattice-task105-capability-proof")
            .ssl_mode(SslMode::Disable);
        let mut observer = observer
            .connect(NoTls)
            .expect("TASK105_CAPABILITY_PROOF_CONNECT");
        let public_connect: bool = observer
            .query_one(
                "SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_database d \
                   CROSS JOIN LATERAL pg_catalog.aclexplode(COALESCE( \
                     d.datacl,pg_catalog.acldefault('d',d.datdba))) a \
                  WHERE d.datname=$1 AND a.grantee=0 AND a.privilege_type='CONNECT')",
                &[&database],
            )
            .expect("TASK105_PUBLIC_CONNECT_QUERY")
            .get(0);
        assert!(!public_connect, "TASK105_PUBLIC_CONNECT_FORBIDDEN");
        for (capability, login) in roles {
            let row = observer
                .query_one(
                    "SELECT NOT capability.rolcanlogin, login.rolcanlogin, \
                            EXISTS (SELECT 1 FROM pg_catalog.pg_database d \
                              CROSS JOIN LATERAL pg_catalog.aclexplode(d.datacl) a \
                             WHERE d.datname=$1 AND a.grantee=login.oid \
                               AND a.privilege_type='CONNECT'), \
                            NOT pg_catalog.has_database_privilege(login.oid,$1,'CREATE'), \
                            NOT pg_catalog.has_database_privilege(login.oid,$1,'TEMP') \
                       FROM pg_catalog.pg_roles capability \
                       JOIN pg_catalog.pg_roles login ON login.rolname=$3 \
                      WHERE capability.rolname=$2",
                    &[&database, &capability, &login],
                )
                .expect("TASK105_LOGIN_CAPABILITY_QUERY");
            assert!(row.get::<_, bool>(0), "TASK105_CAPABILITY_MUST_BE_NOLOGIN");
            assert!(row.get::<_, bool>(1), "TASK105_LOGIN_ROLE_MUST_LOGIN");
            assert_eq!(
                row.get::<_, bool>(2),
                expected_connect,
                "TASK105_DIRECT_CONNECT_GRANT"
            );
            assert!(row.get::<_, bool>(3), "TASK105_LOGIN_CREATE_FORBIDDEN");
            assert!(row.get::<_, bool>(4), "TASK105_LOGIN_TEMP_FORBIDDEN");

            let mut login_config = Config::new();
            login_config
                .host(&self.host)
                .port(self.port)
                .user(login)
                .password(&self.password)
                .dbname(&database)
                .application_name("lattice-task105-login-proof")
                .ssl_mode(SslMode::Disable);
            assert_eq!(
                login_config.connect(NoTls).is_ok(),
                expected_connect,
                "TASK105_LOGIN_CONNECTIVITY:{login}"
            );
        }
    }

    fn migration_target(&self) -> MigrationTarget {
        MigrationTarget::new(self.database_name(), self.run_id.clone())
            .expect("TASK105_MIGRATION_TARGET")
    }

    fn migrator_client(&self) -> Client {
        let mut config = Config::new();
        config
            .host(&self.host)
            .port(self.port)
            .user(DatabaseRole::Migrator.login_role())
            .password(&self.password)
            .dbname(&self.database_name())
            .application_name("lattice-devos-task019")
            .ssl_mode(SslMode::Disable);
        let mut client = config.connect(NoTls).expect("TASK105_MIGRATOR_CONNECT");
        client
            .batch_execute("SET ROLE lattice_migrator")
            .expect("TASK105_MIGRATOR_ROLE");
        client
    }

    fn prepare_v5_store_only(&self) {
        let target = self.migration_target();
        let mut migrator = self.migrator_client();
        assert_eq!(
            apply_migrations(&mut migrator, &target).expect("TASK105_FIXTURE_STORE_V5"),
            MigrationApplyOutcome::Applied {
                executable_count: 5
            }
        );
    }

    fn introduce_partial_writer_on_fresh_store(&self) {
        self.bootstrap_client()
            .batch_execute(
                "CREATE SCHEMA writer_lease AUTHORIZATION lattice_migrator;\
                 REVOKE ALL ON SCHEMA writer_lease FROM PUBLIC;\
                 COMMENT ON SCHEMA writer_lease IS 'TASK105_PARTIAL_WRITER'",
            )
            .expect("TASK105_FRESH_PARTIAL_WRITER");
    }

    fn assert_store_migration_profile(&self, expected: MigrationBootstrapProfile) {
        let target = self.migration_target();
        let mut migrator = self.migrator_client();
        assert_eq!(
            inspect_migration_profile(&mut migrator, &target)
                .expect("TASK105_STORE_MIGRATION_PROFILE"),
            expected
        );
    }

    fn fresh_catalog_fingerprint(&self) -> Vec<String> {
        let mut client = self.bootstrap_client();
        FRESH_CATALOG_FINGERPRINT_QUERIES
            .into_iter()
            .map(|query| {
                client
                    .query_one(query, &[])
                    .expect("TASK105_FRESH_FINGERPRINT_QUERY")
                    .get(0)
            })
            .collect()
    }

    fn prepare_legacy_v1_store(&self) {
        let target = self.migration_target();
        let manifest = migration_manifest();
        assert_eq!(manifest.len(), 10, "TASK105_FIXTURE_MANIFEST_SIZE");
        let foundation = &manifest[1];
        let mut migrator = self.migrator_client();
        migrator
            .batch_execute(
                std::str::from_utf8(foundation.bytes()).expect("TASK105_FIXTURE_STORE_V1_UTF8"),
            )
            .expect("TASK105_FIXTURE_STORE_V1_SQL");
        for entry in &manifest[..2] {
            assert_eq!(
                migrator
                    .execute(
                        "INSERT INTO control.migration_history (\
                             ordinal,migration_id,migration_path,byte_length,checksum_sha256,\
                             migration_status,transaction_mode,schema_version,\
                             min_reader,max_reader,min_writer,max_writer\
                         ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)",
                        &[
                            &i16::try_from(entry.ordinal()).expect("TASK105_FIXTURE_ORDINAL"),
                            &entry.id(),
                            &entry.path(),
                            &i64::try_from(entry.byte_length()).expect("TASK105_FIXTURE_BYTES"),
                            &entry.sha256(),
                            &entry.status().as_str(),
                            &entry.transaction_mode().as_str(),
                            &i16::try_from(entry.schema_version()).expect("TASK105_FIXTURE_SCHEMA"),
                            &i16::try_from(*entry.reader_compatibility().start())
                                .expect("TASK105_FIXTURE_MIN_READER"),
                            &i16::try_from(*entry.reader_compatibility().end())
                                .expect("TASK105_FIXTURE_MAX_READER"),
                            &i16::try_from(*entry.writer_compatibility().start())
                                .expect("TASK105_FIXTURE_MIN_WRITER"),
                            &i16::try_from(*entry.writer_compatibility().end())
                                .expect("TASK105_FIXTURE_MAX_WRITER"),
                        ],
                    )
                    .expect("TASK105_FIXTURE_STORE_V1_HISTORY"),
                1
            );
        }
        assert_eq!(
            migrator
                .execute(
                    "INSERT INTO control.database_identity (singleton,database_uuid)\
                     VALUES (true,$1::text::uuid)",
                    &[&target.expected_database_uuid()],
                )
                .expect("TASK105_FIXTURE_STORE_V1_IDENTITY"),
            1
        );
        assert_eq!(
            migrator
                .execute(
                    "INSERT INTO control.schema_compatibility (\
                         singleton,manifest_sha256,current_schema_version,\
                         min_reader,max_reader,min_writer,max_writer\
                     ) VALUES (true,$1,1,1,1,1,1)",
                    &[&LEGACY_V1_MANIFEST_SHA256],
                )
                .expect("TASK105_FIXTURE_STORE_V1_COMPATIBILITY"),
            1
        );
        assert_eq!(
            inspect_migration_profile(&mut migrator, &target)
                .expect("TASK105_FIXTURE_STORE_V1_PROFILE"),
            MigrationBootstrapProfile::LegacyPrefix
        );
    }

    fn legacy_v1_fingerprint(&self) -> Vec<String> {
        let mut client = self.bootstrap_client();
        [
            "SELECT pg_catalog.md5(COALESCE(pg_catalog.string_agg(\
               pg_catalog.to_jsonb(x)::text,E'\\n' ORDER BY x.ordinal),''))\
             FROM ONLY control.migration_history x",
            "SELECT pg_catalog.md5(pg_catalog.to_jsonb(x)::text)\
             FROM ONLY control.schema_compatibility x WHERE singleton",
            "SELECT pg_catalog.md5(pg_catalog.to_jsonb(x)::text)\
             FROM ONLY control.database_identity x WHERE singleton",
            "SELECT pg_catalog.md5(pg_catalog.to_jsonb(x)::text)\
             FROM ONLY control.runtime_admission x WHERE singleton",
            "SELECT pg_catalog.md5(COALESCE(pg_catalog.string_agg(\
               n.nspname||':'||c.relname||':'||c.relkind::text,E'\\n'\
               ORDER BY n.nspname,c.relname),'')) FROM pg_catalog.pg_class c \
               JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
              WHERE n.nspname IN ('control','memory','readmodel','writer_lease')",
            "SELECT pg_catalog.md5(COALESCE(pg_catalog.string_agg(\
               n.nspname||':'||p.proname||':'||pg_catalog.oidvectortypes(p.proargtypes),E'\\n'\
               ORDER BY n.nspname,p.proname,p.oid),'')) FROM pg_catalog.pg_proc p \
               JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
              WHERE n.nspname IN ('control','memory','readmodel','writer_lease')",
        ]
        .into_iter()
        .map(|query| {
            client
                .query_one(query, &[])
                .expect("TASK105_LEGACY_FINGERPRINT_QUERY")
                .get(0)
        })
        .collect()
    }

    fn prepare_v5_writer_v2_current(&self) {
        self.prepare_v5_store_only();
        let target = self.migration_target();
        let mut migrator = self.migrator_client();
        let memory_target = MemoryExtensionTarget::new(self.database_name(), self.run_id.clone())
            .expect("TASK105_FIXTURE_MEMORY_TARGET");
        apply_memory_extension(&mut migrator, &memory_target).expect("TASK105_FIXTURE_MEMORY_V3");
        let memory = verify_memory_manifest().expect("TASK105_FIXTURE_MEMORY_MANIFEST");
        let database_identity =
            ContentDigest::from_sha256(target.expected_database_identity_sha256().as_str())
                .expect("TASK105_FIXTURE_DATABASE_IDENTITY");
        let writer_v2 = WriterExtensionTarget::new(
            self.database_name(),
            database_identity.clone(),
            ContentDigest::from_sha256(V5_MANIFEST_SHA256).expect("TASK105_FIXTURE_V5_MANIFEST"),
            memory.manifest_sha256().clone(),
        )
        .expect("TASK105_FIXTURE_WRITER_V2_TARGET");
        assert_eq!(
            apply_writer_extension(&mut migrator, &writer_v2).expect("TASK105_FIXTURE_WRITER_V2"),
            ExtensionApplyOutcome::Installed
        );
    }

    fn prepare_v5_memory_v2_writer_v2_bridge_pending(&self) {
        self.prepare_v5_store_only();
        let target = self.migration_target();
        let database_identity = target.expected_database_identity_sha256().as_str();
        let memory_v2 = verify_memory_v2_manifest().expect("TASK105_FIXTURE_MEMORY_V2_MANIFEST");
        let writer_v1 = verify_writer_v1_manifest().expect("TASK105_FIXTURE_WRITER_V1_MANIFEST");
        let writer_v2 = verify_writer_v2_manifest().expect("TASK105_FIXTURE_WRITER_V2_MANIFEST");
        let mut migrator = self.migrator_client();

        migrator
            .batch_execute(
                std::str::from_utf8(memory_v2.bytes()).expect("TASK105_FIXTURE_MEMORY_V2_UTF8"),
            )
            .expect("TASK105_FIXTURE_MEMORY_V2_SQL");
        assert_eq!(
            migrator
                .execute(
                    "INSERT INTO memory.codebase_memory_extension_identity (\
                         singleton,extension_id,extension_schema_version,extension_path,\
                         extension_sql_sha256,extension_manifest_sha256,database_uuid,\
                         database_identity_sha256,global_schema_version,global_manifest_sha256\
                     ) VALUES (true,$1,2,$2,$3,$4,$5::text::uuid,$6,3,$7)",
                    &[
                        &memory_v2.extension_id(),
                        &memory_v2.path(),
                        &memory_v2.sql_sha256().as_str(),
                        &memory_v2.manifest_sha256().as_str(),
                        &target.expected_database_uuid(),
                        &database_identity,
                        &HISTORICAL_GLOBAL_MANIFEST_SHA256,
                    ],
                )
                .expect("TASK105_FIXTURE_MEMORY_V2_IDENTITY"),
            1
        );
        assert_eq!(
            migrator
                .execute(
                    "INSERT INTO memory.codebase_memory_extension_ledger (\
                         ledger_ordinal,singleton,extension_id,extension_schema_version,\
                         extension_sql_sha256,extension_manifest_sha256,database_uuid,\
                         database_identity_sha256,global_schema_version,global_manifest_sha256,\
                         event_kind\
                     ) SELECT 1,singleton,extension_id,extension_schema_version,\
                         extension_sql_sha256,extension_manifest_sha256,database_uuid,\
                         database_identity_sha256,global_schema_version,global_manifest_sha256,\
                         'INSTALLED' FROM ONLY memory.codebase_memory_extension_identity \
                         WHERE singleton",
                    &[],
                )
                .expect("TASK105_FIXTURE_MEMORY_V2_LEDGER"),
            1
        );

        migrator
            .batch_execute(
                std::str::from_utf8(writer_v1.bytes()).expect("TASK105_FIXTURE_WRITER_V1_UTF8"),
            )
            .expect("TASK105_FIXTURE_WRITER_V1_SQL");
        assert_eq!(
            migrator
                .execute(
                    "INSERT INTO writer_lease.writer_lease_extension_identity (\
                         singleton,extension_id,extension_schema_version,extension_path,\
                         extension_sql_sha256,extension_manifest_sha256,database_uuid,\
                         database_identity_sha256,global_schema_version,global_manifest_sha256,\
                         required_memory_schema_version,required_memory_manifest_sha256\
                     ) VALUES (true,$1,1,$2,$3,$4,$5::text::uuid,$6,3,$7,2,$8)",
                    &[
                        &writer_v1.extension_id(),
                        &writer_v1.path(),
                        &writer_v1.sql_sha256().as_str(),
                        &writer_v1.manifest_sha256().as_str(),
                        &target.expected_database_uuid(),
                        &database_identity,
                        &HISTORICAL_GLOBAL_MANIFEST_SHA256,
                        &memory_v2.manifest_sha256().as_str(),
                    ],
                )
                .expect("TASK105_FIXTURE_WRITER_V1_IDENTITY"),
            1
        );
        assert_eq!(
            migrator
                .execute(
                    "INSERT INTO writer_lease.writer_lease_extension_ledger (\
                         ledger_ordinal,singleton,extension_id,extension_schema_version,\
                         extension_sql_sha256,extension_manifest_sha256,database_uuid,\
                         database_identity_sha256,global_schema_version,global_manifest_sha256,\
                         required_memory_schema_version,required_memory_manifest_sha256,event_kind\
                     ) SELECT 1,singleton,extension_id,extension_schema_version,\
                         extension_sql_sha256,extension_manifest_sha256,database_uuid,\
                         database_identity_sha256,global_schema_version,global_manifest_sha256,\
                         required_memory_schema_version,required_memory_manifest_sha256,'INSTALLED'\
                         FROM ONLY writer_lease.writer_lease_extension_identity WHERE singleton",
                    &[],
                )
                .expect("TASK105_FIXTURE_WRITER_V1_LEDGER"),
            1
        );
        migrator
            .batch_execute(
                std::str::from_utf8(writer_v2.bytes()).expect("TASK105_FIXTURE_WRITER_V2_UTF8"),
            )
            .expect("TASK105_FIXTURE_WRITER_V2_SQL");
        assert_eq!(
            migrator
                .execute(
                    "UPDATE ONLY writer_lease.writer_lease_extension_identity SET \
                         extension_schema_version=2,extension_path=$1,\
                         extension_sql_sha256=$2,extension_manifest_sha256=$3 \
                     WHERE singleton AND extension_id=$4 AND extension_schema_version=1 \
                       AND extension_path=$5 AND extension_sql_sha256=$6 \
                       AND extension_manifest_sha256=$7",
                    &[
                        &writer_v2.path(),
                        &writer_v2.sql_sha256().as_str(),
                        &writer_v2.manifest_sha256().as_str(),
                        &writer_v1.extension_id(),
                        &writer_v1.path(),
                        &writer_v1.sql_sha256().as_str(),
                        &writer_v1.manifest_sha256().as_str(),
                    ],
                )
                .expect("TASK105_FIXTURE_WRITER_V2_IDENTITY"),
            1
        );
        assert_eq!(
            migrator
                .execute(
                    "INSERT INTO writer_lease.writer_lease_extension_ledger (\
                         ledger_ordinal,singleton,extension_id,extension_schema_version,\
                         extension_sql_sha256,extension_manifest_sha256,database_uuid,\
                         database_identity_sha256,global_schema_version,global_manifest_sha256,\
                         required_memory_schema_version,required_memory_manifest_sha256,event_kind\
                     ) SELECT 2,singleton,extension_id,extension_schema_version,\
                         extension_sql_sha256,extension_manifest_sha256,database_uuid,\
                         database_identity_sha256,global_schema_version,global_manifest_sha256,\
                         required_memory_schema_version,required_memory_manifest_sha256,'UPGRADED'\
                         FROM ONLY writer_lease.writer_lease_extension_identity WHERE singleton",
                    &[],
                )
                .expect("TASK105_FIXTURE_WRITER_V2_LEDGER"),
            1
        );
    }

    fn prepare_v5_writer_v3_bridge(&self) {
        self.prepare_v5_writer_v2_current();
        let target = self.migration_target();
        let mut migrator = self.migrator_client();
        let database_identity =
            ContentDigest::from_sha256(target.expected_database_identity_sha256().as_str())
                .expect("TASK105_FIXTURE_DATABASE_IDENTITY");
        let writer_v3 = V3ExtensionTarget::new(self.database_name(), database_identity)
            .expect("TASK105_FIXTURE_WRITER_V3_TARGET");
        assert_eq!(
            apply_v3_extension(&mut migrator, &writer_v3).expect("TASK105_FIXTURE_WRITER_V3"),
            ExtensionApplyOutcome::Bridged
        );
    }

    fn advance_v5_to_v6(&self) {
        let mut migrator = self.migrator_client();
        assert_eq!(
            apply_migrations(&mut migrator, &self.migration_target())
                .expect("TASK105_FIXTURE_STORE_V6"),
            MigrationApplyOutcome::Applied {
                executable_count: 1
            }
        );
    }

    fn prepare_legacy_v8_foreman_base(&self) {
        let target = self.migration_target();
        let database_identity =
            ContentDigest::from_sha256(target.expected_database_identity_sha256().as_str())
                .expect("TASK105_LEGACY_V8_DATABASE_IDENTITY");
        let writer_v4 = V4ExtensionTarget::new(self.database_name(), database_identity.clone())
            .expect("TASK105_LEGACY_V8_WRITER_V4_TARGET");
        let writer_v5 = V5ExtensionTarget::new(self.database_name(), database_identity)
            .expect("TASK105_LEGACY_V8_WRITER_V5_TARGET");
        let foreman = ForemanExtensionTarget::new(self.database_name(), self.run_id.clone())
            .expect("TASK105_LEGACY_V8_FOREMAN_TARGET");
        let mut migrator = self.migrator_client();
        assert_eq!(
            apply_v4_extension(&mut migrator, &writer_v4).expect("TASK105_LEGACY_V8_WRITER_V4"),
            ExtensionApplyOutcome::Bridged
        );
        assert_eq!(
            apply_migrations(&mut migrator, &target).expect("TASK105_LEGACY_V8_STORE_V7"),
            MigrationApplyOutcome::Applied {
                executable_count: 1
            }
        );
        assert_eq!(
            apply_v5_extension(&mut migrator, &writer_v5).expect("TASK105_LEGACY_V8_WRITER_V5"),
            ExtensionApplyOutcome::Activated
        );
        assert!(matches!(
            apply_foreman_extension(&mut migrator, &foreman)
                .expect("TASK105_LEGACY_V8_FOREMAN_BASE"),
            ForemanExtensionApplyOutcome::Installed(_)
        ));

        let entry = &migration_manifest()[8];
        assert_eq!(entry.ordinal(), 9);
        assert_eq!(entry.id(), "0009_external_verified_result_adoption");
        let sql = std::str::from_utf8(entry.bytes()).expect("TASK105_LEGACY_V8_SQL_UTF8");
        let ordinal = i16::try_from(entry.ordinal()).expect("TASK105_LEGACY_V8_ORDINAL");
        let byte_length =
            i64::try_from(entry.byte_length()).expect("TASK105_LEGACY_V8_BYTE_LENGTH");
        let schema_version =
            i16::try_from(entry.schema_version()).expect("TASK105_LEGACY_V8_SCHEMA_VERSION");
        let min_reader = i16::try_from(*entry.reader_compatibility().start())
            .expect("TASK105_LEGACY_V8_MIN_READER");
        let max_reader = i16::try_from(*entry.reader_compatibility().end())
            .expect("TASK105_LEGACY_V8_MAX_READER");
        let min_writer = i16::try_from(*entry.writer_compatibility().start())
            .expect("TASK105_LEGACY_V8_MIN_WRITER");
        let max_writer = i16::try_from(*entry.writer_compatibility().end())
            .expect("TASK105_LEGACY_V8_MAX_WRITER");
        let mut transaction = migrator
            .transaction()
            .expect("TASK105_LEGACY_V8_TRANSACTION");
        transaction
            .batch_execute(sql)
            .expect("TASK105_LEGACY_V8_APPLY_SQL");
        assert_eq!(
            transaction
                .execute(
                    "INSERT INTO control.migration_history (\
                         ordinal,migration_id,migration_path,byte_length,checksum_sha256,\
                         migration_status,transaction_mode,schema_version,\
                         min_reader,max_reader,min_writer,max_writer\
                     ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)",
                    &[
                        &ordinal,
                        &entry.id(),
                        &entry.path(),
                        &byte_length,
                        &entry.sha256(),
                        &entry.status().as_str(),
                        &entry.transaction_mode().as_str(),
                        &schema_version,
                        &min_reader,
                        &max_reader,
                        &min_writer,
                        &max_writer,
                    ],
                )
                .expect("TASK105_LEGACY_V8_HISTORY"),
            1
        );
        assert_eq!(
            transaction
                .execute(
                    "UPDATE ONLY control.schema_compatibility SET \
                         manifest_sha256=$1,current_schema_version=8,\
                         min_reader=8,max_reader=8,min_writer=8,max_writer=8,\
                         updated_at=clock_timestamp() \
                       WHERE singleton AND manifest_sha256=$2 \
                         AND current_schema_version=7 \
                         AND min_reader=7 AND max_reader=7 \
                         AND min_writer=7 AND max_writer=7",
                    &[&LEGACY_V8_MANIFEST_SHA256, &CURRENT_V7_MANIFEST_SHA256],
                )
                .expect("TASK105_LEGACY_V8_COMPATIBILITY"),
            1
        );
        transaction.commit().expect("TASK105_LEGACY_V8_COMMIT");
        assert_eq!(
            inspect_migration_profile(&mut migrator, &target).expect("TASK105_LEGACY_V8_PROFILE"),
            MigrationBootstrapProfile::V8LegacyPrefix
        );
        drop(migrator);
        self.activate_configured_authority();
        self.assert_configured_authority_active();
    }

    fn activate_configured_authority(&self) {
        let authority = store_authority_from_environment();
        assert_eq!(
            self.bootstrap_client()
                .execute(
                    "UPDATE ONLY control.runtime_admission SET \
                         admission_mode='ACTIVE',daemon_instance_id=$1,daemon_epoch=$2,\
                         authority_revision=$3,observation_digest=decode($4,'hex'),\
                         authority_head_digest=decode($5,'hex'),updated_at=clock_timestamp() \
                       WHERE singleton AND admission_mode='STOPPED' \
                         AND daemon_instance_id IS NULL AND daemon_epoch IS NULL \
                         AND authority_revision=0 AND observation_digest IS NULL \
                         AND authority_head_digest IS NULL",
                    &[
                        &authority.daemon_instance_id().as_str(),
                        &i64::try_from(authority.daemon_epoch().get())
                            .expect("TASK105_CONFIGURED_EPOCH"),
                        &i64::try_from(authority.revision().get())
                            .expect("TASK105_CONFIGURED_REVISION"),
                        &authority.observation_digest().as_str(),
                        &authority.head_digest().as_str(),
                    ],
                )
                .expect("TASK105_ACTIVATE_CONFIGURED_AUTHORITY"),
            1
        );
    }

    fn assert_configured_authority_active(&self) {
        let authority = store_authority_from_environment();
        let row = self
            .bootstrap_client()
            .query_one(
                "SELECT admission_mode::text,daemon_instance_id::text,daemon_epoch,\
                        authority_revision,pg_catalog.encode(observation_digest,'hex'),\
                        pg_catalog.encode(authority_head_digest,'hex') \
                   FROM ONLY control.runtime_admission WHERE singleton",
                &[],
            )
            .expect("TASK105_CONFIGURED_AUTHORITY_QUERY");
        assert_eq!(row.get::<_, String>(0), "ACTIVE");
        assert_eq!(
            row.get::<_, Option<String>>(1).as_deref(),
            Some(authority.daemon_instance_id().as_str())
        );
        assert_eq!(
            row.get::<_, Option<i64>>(2),
            Some(i64::try_from(authority.daemon_epoch().get()).expect("TASK105_ACTIVE_EPOCH"))
        );
        assert_eq!(
            row.get::<_, Option<i64>>(3),
            Some(i64::try_from(authority.revision().get()).expect("TASK105_ACTIVE_REVISION"))
        );
        assert_eq!(
            row.get::<_, Option<String>>(4).as_deref(),
            Some(authority.observation_digest().as_str())
        );
        assert_eq!(
            row.get::<_, Option<String>>(5).as_deref(),
            Some(authority.head_digest().as_str())
        );
    }

    fn assert_v8_writer_v5_foreman_pending_stopped(&self) {
        let row = self
            .bootstrap_client()
            .query_one(
                "SELECT \
                    (SELECT pg_catalog.count(*)=10 FROM ONLY control.migration_history), \
                    (SELECT current_schema_version=8 AND manifest_sha256=$1 \
                       FROM ONLY control.schema_compatibility WHERE singleton), \
                    (SELECT extension_schema_version=5 AND global_schema_version=7 \
                       AND global_manifest_sha256=$2 \
                       FROM ONLY writer_lease.writer_lease_extension_identity WHERE singleton), \
                    (SELECT extension_schema_version=1 AND global_schema_version=7 \
                       AND global_manifest_sha256=$2 \
                       FROM ONLY foreman_execution.extension_identity WHERE singleton), \
                    (SELECT pg_catalog.count(*)=1 \
                       AND pg_catalog.count(*) FILTER (WHERE ledger_ordinal=1 \
                         AND global_schema_version=7 AND global_manifest_sha256=$2 \
                         AND event_kind='INSTALLED')=1 \
                       FROM ONLY foreman_execution.extension_ledger), \
                    (SELECT admission_mode='STOPPED' AND daemon_instance_id IS NULL \
                       AND daemon_epoch IS NULL AND authority_revision=0 \
                       AND observation_digest IS NULL AND authority_head_digest IS NULL \
                       FROM ONLY control.runtime_admission WHERE singleton)",
                &[&CURRENT_V8_MANIFEST_SHA256, &CURRENT_V7_MANIFEST_SHA256],
            )
            .expect("TASK105_V8_FOREMAN_PENDING_STOPPED_QUERY");
        for index in 0..6 {
            assert!(
                row.get::<_, bool>(index),
                "TASK105_V8_FOREMAN_PENDING_STOPPED_{index}"
            );
        }
    }

    fn prove_v5_bridge_retry_does_not_repair_rebind_boundary(&self) {
        let target = self.migration_target();
        let database_identity =
            ContentDigest::from_sha256(target.expected_database_identity_sha256().as_str())
                .expect("TASK105_FIXTURE_DATABASE_IDENTITY");
        let writer_v3 = V3ExtensionTarget::new(self.database_name(), database_identity)
            .expect("TASK105_FIXTURE_WRITER_V3_TARGET");
        let mut migrator = self.migrator_client();
        assert_eq!(
            inspect_v3_bootstrap_profile(&mut migrator, &writer_v3)
                .expect("TASK105_V5_BRIDGE_PREFLIGHT"),
            V3BootstrapProfile::V5Bridge
        );
        migrator
            .batch_execute("DROP PROCEDURE writer_lease.writer_lease_rebind_v3();")
            .expect("TASK105_REMOVE_REBIND_BOUNDARY");
        assert_eq!(
            apply_v3_extension(&mut migrator, &writer_v3)
                .expect_err("TASK105_BRIDGE_RETRY_MUST_NOT_REPAIR")
                .kind(),
            ExtensionSetupErrorKind::PartialOrCollidingProfile
        );
        let absent: bool = migrator
            .query_one(
                "SELECT pg_catalog.to_regprocedure( \
                   'writer_lease.writer_lease_rebind_v3()') IS NULL",
                &[],
            )
            .expect("TASK105_REBIND_BOUNDARY_ABSENT_QUERY")
            .get(0);
        assert!(absent, "TASK105_BRIDGE_RETRY_REPAIRED_BOUNDARY");
        let rebind = verify_embedded_v3_rebind_manifest().expect("TASK105_FIXTURE_REBIND_MANIFEST");
        migrator
            .batch_execute(
                std::str::from_utf8(rebind.bytes()).expect("TASK105_FIXTURE_REBIND_UTF8"),
            )
            .expect("TASK105_RESTORE_REBIND_BOUNDARY");
        assert_eq!(
            inspect_v3_bootstrap_profile(&mut migrator, &writer_v3)
                .expect("TASK105_V5_BRIDGE_RESTORED"),
            V3BootstrapProfile::V5Bridge
        );
    }

    fn assert_v5_writer_v2_current(&self) {
        let row = self
            .bootstrap_client()
            .query_one(
                "SELECT \
                    (SELECT pg_catalog.count(*)=6 FROM ONLY control.migration_history), \
                    (SELECT current_schema_version=5 AND manifest_sha256=$1 \
                       FROM ONLY control.schema_compatibility WHERE singleton), \
                    (SELECT extension_schema_version=2 AND global_schema_version=5 \
                       AND required_memory_schema_version=3 \
                       FROM ONLY writer_lease.writer_lease_extension_identity WHERE singleton), \
                    pg_catalog.has_schema_privilege('lattice_runtime','writer_lease','USAGE'), \
                    pg_catalog.has_function_privilege('lattice_runtime', \
                      'writer_lease.writer_lease_bind_runtime_v2(text,bigint,bytea,text,text,text,text,text)', \
                      'EXECUTE'), \
                    pg_catalog.to_regprocedure( \
                      'writer_lease.writer_lease_bind_runtime_v3(text,bigint,bytea,text,text,text,text,text)') \
                      IS NULL",
                &[&V5_MANIFEST_SHA256],
            )
            .expect("TASK105_V5_WRITER_V2_PROFILE");
        for index in 0..6 {
            assert!(row.get::<_, bool>(index), "TASK105_V5_WRITER_V2_{index}");
        }
    }

    fn assert_v5_memory_v2_writer_v2_bridge_pending(&self) {
        let target = self.migration_target();
        let database_identity =
            ContentDigest::from_sha256(target.expected_database_identity_sha256().as_str())
                .expect("TASK105_FIXTURE_DATABASE_IDENTITY");
        let writer_v3 = V3ExtensionTarget::new(self.database_name(), database_identity)
            .expect("TASK105_FIXTURE_WRITER_V3_TARGET");
        let mut migrator = self.migrator_client();
        assert_eq!(
            inspect_v3_bootstrap_profile(&mut migrator, &writer_v3)
                .expect("TASK105_V5_MEMORY_V2_WRITER_PENDING_PREFLIGHT"),
            V3BootstrapProfile::V5FallbackRequired
        );
        let row = migrator
            .query_one(
                "SELECT\
                    (SELECT extension_schema_version=2 AND global_schema_version=3 \
                       FROM ONLY memory.codebase_memory_extension_identity WHERE singleton),\
                    (SELECT pg_catalog.count(*)=1 \
                       FROM ONLY memory.codebase_memory_extension_ledger),\
                    (SELECT extension_schema_version=2 AND global_schema_version=3 \
                       AND required_memory_schema_version=2 \
                       FROM ONLY writer_lease.writer_lease_extension_identity WHERE singleton),\
                    (SELECT pg_catalog.string_agg(ledger_ordinal::text||':'||event_kind::text,','\
                       ORDER BY ledger_ordinal)='1:INSTALLED,2:UPGRADED'\
                       FROM ONLY writer_lease.writer_lease_extension_ledger),\
                    NOT pg_catalog.has_schema_privilege('lattice_runtime','writer_lease','USAGE')",
                &[],
            )
            .expect("TASK105_V5_MEMORY_V2_WRITER_PENDING_PROFILE");
        for index in 0..5 {
            assert!(
                row.get::<_, bool>(index),
                "TASK105_V5_MEMORY_V2_WRITER_PENDING_{index}"
            );
        }
    }

    fn assert_v5_writer_absent(&self) {
        let row = self
            .bootstrap_client()
            .query_one(
                "SELECT \
                    (SELECT pg_catalog.count(*)=6 FROM ONLY control.migration_history), \
                    (SELECT current_schema_version=5 AND manifest_sha256=$1 \
                       FROM ONLY control.schema_compatibility WHERE singleton), \
                    pg_catalog.to_regnamespace('writer_lease') IS NULL, \
                    pg_catalog.to_regnamespace('memory') IS NOT NULL, \
                    (SELECT pg_catalog.count(*)=0 FROM pg_catalog.pg_class c \
                      JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
                      WHERE n.nspname='memory'), \
                    (SELECT pg_catalog.count(*)=0 FROM pg_catalog.pg_proc p \
                      JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
                      WHERE n.nspname='memory'), \
                    (SELECT admission_mode='STOPPED' FROM ONLY control.runtime_admission \
                       WHERE singleton)",
                &[&V5_MANIFEST_SHA256],
            )
            .expect("TASK105_V5_WRITER_ABSENT_PROFILE");
        for index in 0..7 {
            assert!(
                row.get::<_, bool>(index),
                "TASK105_V5_WRITER_ABSENT_{index}"
            );
        }
    }

    fn v5_fallback_fingerprint(&self) -> Vec<String> {
        let mut client = self.bootstrap_client();
        [
            "SELECT pg_catalog.md5(COALESCE(pg_catalog.string_agg( \
               pg_catalog.to_jsonb(x)::text,E'\\n' ORDER BY x.ordinal),'')) \
             FROM ONLY control.migration_history x",
            "SELECT pg_catalog.md5(pg_catalog.to_jsonb(x)::text) \
             FROM ONLY control.schema_compatibility x WHERE singleton",
            "SELECT pg_catalog.md5(pg_catalog.to_jsonb(x)::text) \
             FROM ONLY control.database_identity x WHERE singleton",
            "SELECT pg_catalog.md5(pg_catalog.to_jsonb(x)::text) \
             FROM ONLY control.runtime_admission x WHERE singleton",
            "SELECT pg_catalog.md5(COALESCE(pg_catalog.string_agg( \
               n.nspname||':'||c.relname||':'||c.relkind::text,E'\\n' \
               ORDER BY c.relname),'')) FROM pg_catalog.pg_namespace n \
               LEFT JOIN pg_catalog.pg_class c ON c.relnamespace=n.oid \
              WHERE n.nspname='memory'",
            "SELECT pg_catalog.md5(COALESCE(pg_catalog.string_agg( \
               d.defaclobjtype::text||':'||d.defaclnamespace::text||':'|| \
               d.defaclacl::text,E'\\n' ORDER BY d.oid),'')) \
             FROM pg_catalog.pg_default_acl d",
            "SELECT pg_catalog.md5(COALESCE(pg_catalog.string_agg( \
               n.nspname||':'||COALESCE(c.relname,'')||':'||COALESCE(p.proname,''), \
               E'\\n' ORDER BY c.relname,p.proname),'')) \
             FROM pg_catalog.pg_namespace n \
             LEFT JOIN pg_catalog.pg_class c ON c.relnamespace=n.oid \
             LEFT JOIN pg_catalog.pg_proc p ON p.pronamespace=n.oid \
             WHERE n.nspname IN ('memory','writer_lease')",
        ]
        .into_iter()
        .map(|query| {
            client
                .query_one(query, &[])
                .expect("TASK105_V5_FALLBACK_FINGERPRINT_QUERY")
                .get(0)
        })
        .collect()
    }

    fn introduce_memory_default_acl_drift(&self) {
        self.bootstrap_client()
            .batch_execute(
                "SET ROLE lattice_migrator; \
                 ALTER DEFAULT PRIVILEGES IN SCHEMA memory \
                 GRANT SELECT ON TABLES TO lattice_runtime; RESET ROLE;",
            )
            .expect("TASK105_MEMORY_DEFAULT_ACL_DRIFT");
    }

    fn repair_memory_default_acl_drift(&self) {
        self.bootstrap_client()
            .batch_execute(
                "SET ROLE lattice_migrator; \
                 ALTER DEFAULT PRIVILEGES IN SCHEMA memory \
                 REVOKE SELECT ON TABLES FROM lattice_runtime; RESET ROLE;",
            )
            .expect("TASK105_MEMORY_DEFAULT_ACL_REPAIR");
    }

    fn introduce_global_default_acl_drift(&self) {
        self.bootstrap_client()
            .batch_execute(
                "SET ROLE lattice_migrator; \
                 ALTER DEFAULT PRIVILEGES \
                 GRANT SELECT ON TABLES TO lattice_runtime; RESET ROLE;",
            )
            .expect("TASK105_GLOBAL_DEFAULT_ACL_DRIFT");
    }

    fn repair_global_default_acl_drift(&self) {
        self.bootstrap_client()
            .batch_execute(
                "SET ROLE lattice_migrator; \
                 ALTER DEFAULT PRIVILEGES \
                 REVOKE SELECT ON TABLES FROM lattice_runtime; RESET ROLE;",
            )
            .expect("TASK105_GLOBAL_DEFAULT_ACL_REPAIR");
    }

    fn introduce_partial_memory_catalog(&self) {
        self.bootstrap_client()
            .batch_execute(
                "SET ROLE lattice_migrator; \
                 CREATE TABLE memory.task105_partial_memory (id integer); RESET ROLE;",
            )
            .expect("TASK105_MEMORY_PARTIAL_CREATE");
    }

    fn repair_partial_memory_catalog(&self) {
        self.bootstrap_client()
            .batch_execute(
                "SET ROLE lattice_migrator; \
                 DROP TABLE memory.task105_partial_memory; RESET ROLE;",
            )
            .expect("TASK105_MEMORY_PARTIAL_REPAIR");
    }

    fn durable_profile_fingerprint(&self) -> Vec<String> {
        let mut client = self.bootstrap_client();
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
            "SELECT pg_catalog.md5(pg_catalog.to_jsonb(a)::text) \
               FROM ONLY control.runtime_admission a WHERE a.singleton",
        ]
        .into_iter()
        .map(|query| {
            client
                .query_one(query, &[])
                .expect("TASK105_PROFILE_FINGERPRINT_QUERY")
                .get(0)
        })
        .collect()
    }

    fn foreman_profile_fingerprint(&self) -> Vec<String> {
        let mut client = self.bootstrap_client();
        [
            "SELECT pg_catalog.md5(pg_catalog.to_jsonb(f)::text) \
               FROM ONLY foreman_execution.extension_identity f WHERE f.singleton",
            "SELECT pg_catalog.md5(COALESCE(pg_catalog.string_agg(\
                 pg_catalog.to_jsonb(f)::text,E'\\n' ORDER BY f.ledger_ordinal),'')) \
               FROM ONLY foreman_execution.extension_ledger f",
        ]
        .into_iter()
        .map(|query| {
            client
                .query_one(query, &[])
                .expect("TASK105_FOREMAN_PROFILE_FINGERPRINT_QUERY")
                .get(0)
        })
        .collect()
    }

    fn assert_v5_writer_v3_bridge(&self) {
        let mut client = self.bootstrap_client();
        let row = client
            .query_one(
                "SELECT \
                    (SELECT pg_catalog.count(*) FROM ONLY control.migration_history)=6, \
                    (SELECT current_schema_version=5 AND manifest_sha256=$1 \
                       FROM ONLY control.schema_compatibility WHERE singleton), \
                    (SELECT extension_schema_version=3 AND global_schema_version=5 \
                       FROM ONLY writer_lease.writer_lease_extension_identity WHERE singleton), \
                    NOT pg_catalog.has_schema_privilege('lattice_runtime','writer_lease','USAGE')",
                &[&V5_MANIFEST_SHA256],
            )
            .expect("TASK105_V5_WRITER_V3_PROFILE");
        for index in 0..4 {
            assert!(row.get::<_, bool>(index), "TASK105_V5_WRITER_V3_{index}");
        }
    }

    fn assert_v6_writer_v3_current(&self) {
        let mut client = self.bootstrap_client();
        let row = client
            .query_one(
                "SELECT \
                    (SELECT pg_catalog.count(*) FROM ONLY control.migration_history)=7, \
                    (SELECT current_schema_version=6 FROM ONLY control.schema_compatibility \
                      WHERE singleton), \
                    (SELECT extension_schema_version=3 AND extension_manifest_sha256=$1 \
                      AND global_schema_version=6 \
                      FROM ONLY writer_lease.writer_lease_extension_identity WHERE singleton), \
                    (SELECT pg_catalog.count(*) IN (1,3,5) \
                      AND pg_catalog.count(*) FILTER (WHERE extension_schema_version=3 \
                        AND global_schema_version=6 AND event_kind='REBOUND')=1 \
                      FROM ONLY writer_lease.writer_lease_extension_ledger), \
                    pg_catalog.has_schema_privilege('lattice_runtime','writer_lease','USAGE'), \
                    pg_catalog.has_function_privilege('lattice_runtime', \
                      'writer_lease.writer_lease_bind_runtime_v3(text,bigint,bytea,text,text,text,text,text)', \
                      'EXECUTE')",
                &[&WRITER_V3_MANIFEST_SHA256],
            )
            .expect("TASK105_V6_WRITER_V3_PROFILE");
        for index in 0..6 {
            assert!(row.get::<_, bool>(index), "TASK105_V6_WRITER_V3_{index}");
        }
    }

    fn assert_v8_writer_v5_successor(&self) {
        let mut client = self.bootstrap_client();
        let row = client
            .query_one(
                "SELECT \
                    (SELECT pg_catalog.count(*) FROM ONLY control.migration_history)=10, \
                    (SELECT current_schema_version=8 AND manifest_sha256=$1 \
                      FROM ONLY control.schema_compatibility WHERE singleton), \
                    (SELECT extension_schema_version=5 \
                      AND extension_manifest_sha256=$3 \
                      AND global_schema_version=7 AND global_manifest_sha256=$2 \
                      AND required_memory_schema_version=3 \
                      FROM ONLY writer_lease.writer_lease_extension_identity WHERE singleton), \
                    (SELECT pg_catalog.count(*) IN (4,6,8) \
                      AND pg_catalog.count(*) FILTER (WHERE extension_schema_version=5 \
                        AND global_schema_version=7 AND event_kind='UPGRADED')=1 \
                      FROM ONLY writer_lease.writer_lease_extension_ledger), \
                    pg_catalog.has_schema_privilege('lattice_runtime','writer_lease','USAGE'), \
                    pg_catalog.has_function_privilege('lattice_runtime', \
                      'writer_lease.writer_lease_bind_runtime_v5(text,bigint,bytea,text,text,text,text,text)', \
                      'EXECUTE'), \
                    NOT pg_catalog.has_function_privilege('lattice_runtime', \
                      'writer_lease.writer_lease_bind_runtime_v4(text,bigint,bytea,text,text,text,text,text)', \
                      'EXECUTE'), \
                    (SELECT global_schema_version=8 AND global_manifest_sha256=$1 \
                       FROM ONLY foreman_execution.extension_identity WHERE singleton), \
                    (SELECT pg_catalog.count(*)=2 \
                       AND pg_catalog.count(*) FILTER (WHERE ledger_ordinal=2 \
                         AND global_schema_version=8 AND global_manifest_sha256=$1 \
                         AND event_kind='REBOUND')=1 \
                       FROM ONLY foreman_execution.extension_ledger)",
                &[
                    &CURRENT_V8_MANIFEST_SHA256,
                    &CURRENT_V7_MANIFEST_SHA256,
                    &WRITER_V5_MANIFEST_SHA256,
                ],
            )
            .expect("TASK105_V8_WRITER_V5_SUCCESSOR_PROFILE");
        for index in 0..9 {
            assert!(
                row.get::<_, bool>(index),
                "TASK105_V8_WRITER_V5_SUCCESSOR_{index}"
            );
        }
    }

    fn introduce_partial_writer_acl(&self) {
        self.bootstrap_client()
            .batch_execute(
                "REVOKE EXECUTE ON FUNCTION writer_lease.writer_lease_bind_runtime_v5(\
                    text,bigint,bytea,text,text,text,text,text) FROM lattice_runtime",
            )
            .expect("TASK105_INTRODUCE_PARTIAL_WRITER");
    }

    fn repair_partial_writer_acl(&self) {
        self.bootstrap_client()
            .batch_execute(
                "GRANT EXECUTE ON FUNCTION writer_lease.writer_lease_bind_runtime_v5(\
                    text,bigint,bytea,text,text,text,text,text) TO lattice_runtime",
            )
            .expect("TASK105_REPAIR_PARTIAL_WRITER");
    }

    fn introduce_corrupt_writer_identity(&self) {
        self.bootstrap_client()
            .batch_execute(
                "UPDATE ONLY writer_lease.writer_lease_extension_identity \
                 SET extension_manifest_sha256=repeat('d',64) WHERE singleton",
            )
            .expect("TASK105_INTRODUCE_CORRUPT_WRITER");
    }

    fn repair_corrupt_writer_identity(&self) {
        self.bootstrap_client()
            .batch_execute(&format!(
                "UPDATE ONLY writer_lease.writer_lease_extension_identity \
                 SET extension_manifest_sha256='{WRITER_V5_MANIFEST_SHA256}' WHERE singleton"
            ))
            .expect("TASK105_REPAIR_CORRUPT_WRITER");
    }

    fn introduce_unsupported_history(&self, coherent_future_profile: bool) {
        let mut client = self.bootstrap_client();
        let mut transaction = client.transaction().expect("TASK105_FUTURE_TRANSACTION");
        transaction
            .batch_execute(
                "INSERT INTO control.migration_history (ordinal,migration_id,migration_path,\
                    byte_length,checksum_sha256,migration_status,transaction_mode,schema_version,\
                    min_reader,max_reader,min_writer,max_writer) VALUES (11,'0011_unsupported_fixture',\
                    'db/migrations/0011_unsupported_fixture.sql',1,repeat('d',64),'EXECUTABLE',\
                    'RUNNER_OWNED',9,9,9,9,9)",
            )
            .expect("TASK105_INTRODUCE_UNSUPPORTED_HISTORY");
        if coherent_future_profile {
            transaction
                .execute(
                    "UPDATE ONLY control.schema_compatibility \
                     SET manifest_sha256=$1,current_schema_version=9,min_reader=9,max_reader=9,\
                         min_writer=9,max_writer=9 WHERE singleton=true",
                    &[&future_v9_manifest_sha256()],
                )
                .expect("TASK105_INTRODUCE_FUTURE_COMPATIBILITY");
        }
        transaction.commit().expect("TASK105_FUTURE_COMMIT");
    }

    fn repair_unsupported_history(&self) -> Result<(), &'static str> {
        let current_manifest =
            verify_embedded_manifest().map_err(|_| "TASK105_CURRENT_MANIFEST")?;
        let mut client = self.try_bootstrap_client()?;
        let mut transaction = client
            .transaction()
            .map_err(|_| "TASK105_REPAIR_TRANSACTION")?;
        let deleted = transaction
            .execute(
                "DELETE FROM ONLY control.migration_history \
                 WHERE ordinal=11 AND migration_id='0011_unsupported_fixture'",
                &[],
            )
            .map_err(|_| "TASK105_REPAIR_UNSUPPORTED_HISTORY")?;
        if deleted != 1 {
            return Err("TASK105_REPAIR_UNSUPPORTED_HISTORY_COUNT");
        }
        let updated = transaction
            .execute(
                "UPDATE ONLY control.schema_compatibility \
                 SET manifest_sha256=$1,current_schema_version=$2,min_reader=$2,max_reader=$2,\
                     min_writer=$2,max_writer=$2 WHERE singleton=true",
                &[
                    &current_manifest.manifest_sha256().as_str(),
                    &i16::try_from(POSTGRES_SCHEMA_VERSION).expect("fixed current schema version"),
                ],
            )
            .map_err(|_| "TASK105_REPAIR_CURRENT_COMPATIBILITY")?;
        if updated != 1 {
            return Err("TASK105_REPAIR_CURRENT_COMPATIBILITY_COUNT");
        }
        transaction.commit().map_err(|_| "TASK105_REPAIR_COMMIT")?;
        Ok(())
    }

    fn migration_fingerprint(&self) -> (i64, i16, String) {
        let mut client = self.bootstrap_client();
        let row = client
            .query_one(
                "SELECT (SELECT pg_catalog.count(*) FROM ONLY control.migration_history), \
                        current_schema_version, manifest_sha256::text \
                   FROM ONLY control.schema_compatibility WHERE singleton",
                &[],
            )
            .expect("TASK105_MIGRATION_FINGERPRINT");
        (row.get(0), row.get(1), row.get(2))
    }

    fn assert_coherent_future_atomic_snapshot(&self) {
        let row = self
            .bootstrap_client()
            .query_one(
                "SELECT \
                   COALESCE(array_agg(h.ordinal ORDER BY h.ordinal),ARRAY[]::smallint[]), \
                   COALESCE(array_agg(h.migration_id::text ORDER BY h.ordinal),ARRAY[]::text[]), \
                   COALESCE(array_agg(h.migration_path::text ORDER BY h.ordinal),ARRAY[]::text[]), \
                   COALESCE(array_agg(h.byte_length ORDER BY h.ordinal),ARRAY[]::bigint[]), \
                   COALESCE(array_agg(h.checksum_sha256::text ORDER BY h.ordinal),ARRAY[]::text[]), \
                   COALESCE(array_agg(h.migration_status::text ORDER BY h.ordinal),ARRAY[]::text[]), \
                   COALESCE(array_agg(h.transaction_mode::text ORDER BY h.ordinal),ARRAY[]::text[]), \
                   COALESCE(array_agg(h.schema_version ORDER BY h.ordinal),ARRAY[]::smallint[]), \
                   COALESCE(array_agg(h.min_reader ORDER BY h.ordinal),ARRAY[]::smallint[]), \
                   COALESCE(array_agg(h.max_reader ORDER BY h.ordinal),ARRAY[]::smallint[]), \
                   COALESCE(array_agg(h.min_writer ORDER BY h.ordinal),ARRAY[]::smallint[]), \
                   COALESCE(array_agg(h.max_writer ORDER BY h.ordinal),ARRAY[]::smallint[]), \
                   (SELECT c.manifest_sha256 FROM ONLY control.schema_compatibility c \
                     WHERE c.singleton=true), \
                   (SELECT c.current_schema_version FROM ONLY control.schema_compatibility c \
                     WHERE c.singleton=true), \
                   (SELECT c.min_reader FROM ONLY control.schema_compatibility c \
                     WHERE c.singleton=true), \
                   (SELECT c.max_reader FROM ONLY control.schema_compatibility c \
                     WHERE c.singleton=true), \
                   (SELECT c.min_writer FROM ONLY control.schema_compatibility c \
                     WHERE c.singleton=true), \
                   (SELECT c.max_writer FROM ONLY control.schema_compatibility c \
                     WHERE c.singleton=true) \
                 FROM ONLY control.migration_history h",
                &[],
            )
            .expect("TASK105_FUTURE_ATOMIC_QUERY");
        assert_eq!(
            row.columns()
                .iter()
                .map(|column| column.type_().name())
                .collect::<Vec<_>>(),
            [
                "_int2", "_text", "_text", "_int8", "_text", "_text", "_text", "_int2", "_int2",
                "_int2", "_int2", "_int2", "bpchar", "int2", "int2", "int2", "int2", "int2"
            ]
        );
        let ordinals = row.get::<_, Vec<i16>>(0);
        let ids = row.get::<_, Vec<String>>(1);
        let paths = row.get::<_, Vec<String>>(2);
        let lengths = row.get::<_, Vec<i64>>(3);
        let checksums = row.get::<_, Vec<String>>(4);
        let statuses = row.get::<_, Vec<String>>(5);
        let modes = row.get::<_, Vec<String>>(6);
        let schemas = row.get::<_, Vec<i16>>(7);
        let min_readers = row.get::<_, Vec<i16>>(8);
        let max_readers = row.get::<_, Vec<i16>>(9);
        let min_writers = row.get::<_, Vec<i16>>(10);
        let max_writers = row.get::<_, Vec<i16>>(11);
        for length in [
            ordinals.len(),
            ids.len(),
            paths.len(),
            lengths.len(),
            checksums.len(),
            statuses.len(),
            modes.len(),
            schemas.len(),
            min_readers.len(),
            max_readers.len(),
            min_writers.len(),
            max_writers.len(),
        ] {
            assert_eq!(length, 11, "TASK105_FUTURE_ATOMIC_VECTOR_LENGTH");
        }
        let manifest = row.get::<_, Option<String>>(12);
        let versions = (13..=17)
            .map(|index| row.get::<_, Option<i16>>(index))
            .collect::<Option<Vec<_>>>()
            .expect("TASK105_FUTURE_ATOMIC_COMPATIBILITY");
        assert_eq!(manifest.as_deref(), Some(FUTURE_V9_MANIFEST_SHA256));
        assert_eq!(versions, [9; 5]);

        fn field(hasher: &mut Sha256, value: &[u8]) {
            hasher.update(
                u64::try_from(value.len())
                    .expect("field length")
                    .to_be_bytes(),
            );
            hasher.update(value);
        }
        let mut hasher = Sha256::new();
        hasher.update(b"LATTICE_POSTGRES_MIGRATION_MANIFEST_V1\0");
        for index in 0..11 {
            field(
                &mut hasher,
                &u16::try_from(ordinals[index])
                    .expect("TASK105_FUTURE_ATOMIC_ORDINAL")
                    .to_be_bytes(),
            );
            for value in [&ids[index], &paths[index]] {
                field(&mut hasher, value.as_bytes());
            }
            field(
                &mut hasher,
                &u64::try_from(lengths[index])
                    .expect("TASK105_FUTURE_ATOMIC_BYTE_LENGTH")
                    .to_be_bytes(),
            );
            for value in [&checksums[index], &statuses[index], &modes[index]] {
                field(&mut hasher, value.as_bytes());
            }
            for value in [
                schemas[index],
                min_readers[index],
                max_readers[index],
                min_writers[index],
                max_writers[index],
            ] {
                field(
                    &mut hasher,
                    &u16::try_from(value)
                        .expect("TASK105_FUTURE_ATOMIC_VERSION")
                        .to_be_bytes(),
                );
            }
        }
        let mut decoded_digest = String::with_capacity(64);
        for byte in hasher.finalize() {
            use std::fmt::Write as _;
            write!(&mut decoded_digest, "{byte:02x}").expect("TASK105_FUTURE_ATOMIC_DIGEST_HEX");
        }
        assert_eq!(decoded_digest, FUTURE_V9_MANIFEST_SHA256);
    }

    fn v8_absent_writer_fingerprint(&self) -> Vec<String> {
        let mut client = self.bootstrap_client();
        [
            "SELECT pg_catalog.md5(COALESCE(pg_catalog.string_agg( \
               pg_catalog.to_jsonb(x)::text,E'\\n' ORDER BY x.ordinal),'')) \
             FROM ONLY control.migration_history x",
            "SELECT pg_catalog.md5(pg_catalog.to_jsonb(x)::text) \
             FROM ONLY control.schema_compatibility x WHERE singleton",
            "SELECT pg_catalog.md5(pg_catalog.to_jsonb(x)::text) \
             FROM ONLY control.database_identity x WHERE singleton",
            "SELECT pg_catalog.md5(COALESCE(pg_catalog.string_agg( \
               n.nspname||':'||COALESCE(c.relname,'')||':'|| \
               COALESCE(p.proname,''),E'\\n' ORDER BY c.relname,p.proname),'')) \
             FROM pg_catalog.pg_namespace n \
             LEFT JOIN pg_catalog.pg_class c ON c.relnamespace=n.oid \
             LEFT JOIN pg_catalog.pg_proc p ON p.pronamespace=n.oid \
             WHERE n.nspname='memory'",
            "SELECT pg_catalog.md5(pg_catalog.to_jsonb(x)::text) \
             FROM ONLY memory.codebase_memory_extension_identity x WHERE singleton",
            "SELECT pg_catalog.md5(COALESCE(pg_catalog.string_agg( \
               pg_catalog.to_jsonb(x)::text,E'\\n' ORDER BY x.ledger_ordinal),'')) \
             FROM ONLY memory.codebase_memory_extension_ledger x",
            "SELECT pg_catalog.md5(pg_catalog.count(*)::text) \
             FROM pg_catalog.pg_namespace WHERE nspname='writer_lease'",
            "SELECT pg_catalog.md5(pg_catalog.to_jsonb(x)::text) \
             FROM ONLY control.runtime_admission x WHERE singleton",
        ]
        .into_iter()
        .map(|query| {
            client
                .query_one(query, &[])
                .expect("TASK105_V8_ABSENT_FINGERPRINT")
                .get(0)
        })
        .collect()
    }

    fn assert_writer_namespace_absent(&self) {
        let absent: bool = self
            .bootstrap_client()
            .query_one(
                "SELECT pg_catalog.to_regnamespace('writer_lease') IS NULL",
                &[],
            )
            .expect("TASK105_WRITER_NAMESPACE_ABSENT_QUERY")
            .get(0);
        assert!(absent, "TASK105_WRITER_NAMESPACE_PRESENT");
    }

    fn remove_disposable_writer_profile(&self) {
        self.bootstrap_client()
            .batch_execute(
                "SET ROLE lattice_migrator; DROP SCHEMA writer_lease CASCADE; RESET ROLE;",
            )
            .expect("TASK105_REMOVE_DISPOSABLE_WRITER_PROFILE");
    }

    fn foreman_counts(&self) -> ([i64; 3], [String; 3], Option<String>) {
        let stream_hex = foreman_stream_hex();
        let row = self
            .bootstrap_client()
            .query_one(
                "SELECT \
                   (SELECT pg_catalog.count(*) FROM ONLY control.task_ledger_commands c \
                     WHERE c.stream_id=pg_catalog.decode($1,'hex')), \
                   (SELECT pg_catalog.count(*) FROM ONLY control.task_ledger_events e \
                     WHERE e.stream_id=pg_catalog.decode($1,'hex')), \
                   (SELECT pg_catalog.count(*) FROM ONLY control.task_ledger_foreman_snapshots f \
                     WHERE f.stream_id=pg_catalog.decode($1,'hex')), \
                   COALESCE((SELECT s.sequence::text FROM ONLY control.task_ledger_streams s \
                     WHERE s.stream_id=pg_catalog.decode($1,'hex')),'0'), \
                   COALESCE((SELECT s.event_count::text FROM ONLY control.task_ledger_streams s \
                     WHERE s.stream_id=pg_catalog.decode($1,'hex')),'0'), \
                   COALESCE((SELECT s.command_count::text FROM ONLY control.task_ledger_streams s \
                     WHERE s.stream_id=pg_catalog.decode($1,'hex')),'0'), \
                   (SELECT pg_catalog.encode(s.head_digest,'hex') \
                      FROM ONLY control.task_ledger_streams s \
                     WHERE s.stream_id=pg_catalog.decode($1,'hex'))",
                &[&stream_hex],
            )
            .expect("TASK105_FOREMAN_COUNTS");
        (
            [row.get(0), row.get(1), row.get(2)],
            [row.get(3), row.get(4), row.get(5)],
            row.get(6),
        )
    }

    fn assert_writer_command_absent(&self, checkpoint_id: &str) {
        let command_id = foreman_acquire_command_id(checkpoint_id);
        let count: i64 = self
            .bootstrap_client()
            .query_one(
                "SELECT pg_catalog.count(*) \
                   FROM ONLY writer_lease.writer_lease_commands c \
                  WHERE c.project_id=$1 AND c.command_id=$2",
                &[&"lattice-control", &command_id],
            )
            .expect("TASK105_RACE_WRITER_ABSENCE_QUERY")
            .get(0);
        assert_eq!(count, 0, "TASK105_RACE_WRITER_COMMAND_MUST_BE_ABSENT");
    }
}

fn foreman_stream_hex() -> String {
    VerifiedStream::vacant(
        foreman_coordination_identity().expect("TASK105_FOREMAN_IDENTITY"),
        RuntimeKind::Live,
    )
    .expect("TASK105_FOREMAN_VACANT_STREAM")
    .head()
    .stream_id()
    .as_str()
    .to_owned()
}

fn foreman_acquire_command_id(checkpoint_id: &str) -> String {
    let digest = Sha256::digest(checkpoint_id.as_bytes());
    let mut suffix = String::with_capacity(24);
    for byte in &digest[..12] {
        use std::fmt::Write as _;
        write!(&mut suffix, "{byte:02x}").expect("TASK105_FOREMAN_COMMAND_HEX");
    }
    format!("foreman-acquire-{suffix}")
}

fn foreman_writer_repository(config: &LiveConfig) -> PostgresWriterLease {
    let database_identity = ContentDigest::from_sha256(
        config
            .migration_target()
            .expected_database_identity_sha256()
            .as_str(),
    )
    .expect("TASK105_RACE_DATABASE_IDENTITY");
    let target = V5ExtensionTarget::new(config.database_name(), database_identity)
        .expect("TASK105_RACE_WRITER_TARGET");
    PostgresWriterLease::new_v5_v7(
        config.runtime_client(),
        &target,
        &store_authority_from_environment(),
        600,
    )
    .expect("TASK105_RACE_WRITER_REPOSITORY")
}

struct RaceAuthorityCleanup {
    repository: PostgresWriterLease,
    project_id: ProjectId,
    expected: WriterLeaseAuthorityHead,
    armed: bool,
}

impl RaceAuthorityCleanup {
    fn new(
        repository: PostgresWriterLease,
        project_id: ProjectId,
        expected: WriterLeaseAuthorityHead,
    ) -> Self {
        Self {
            repository,
            project_id,
            expected,
            armed: true,
        }
    }

    fn disarm_after_release(&mut self) -> Result<(), &'static str> {
        if self
            .repository
            .current_authority(&self.project_id)
            .map_err(|_| "TASK105_RACE_AUTHORITY_CURRENT")?
            .is_some()
        {
            return Err("TASK105_RACE_AUTHORITY_STILL_CURRENT");
        }
        self.armed = false;
        Ok(())
    }
}

impl Drop for RaceAuthorityCleanup {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let Ok(Some(current)) = self.repository.current_authority(&self.project_id) else {
            return;
        };
        if current.independent_head() != &self.expected {
            return;
        }
        let _ = self
            .repository
            .execute(WriterLeaseRepositoryCommand::Release(
                WriterLeaseReleaseRequest {
                    command_id: "task105-race-cleanup-release".to_owned(),
                    project_id: self.project_id.clone(),
                    expected_head: current.independent_head().clone(),
                },
            ));
    }
}

struct ForemanStreamLock {
    client: Client,
    key: i64,
    held: bool,
}

impl ForemanStreamLock {
    fn acquire(config: &LiveConfig) -> Self {
        let mut client = config.bootstrap_client();
        let stream_hex = foreman_stream_hex();
        let key: i64 = client
            .query_one(
                "SELECT pg_catalog.hashtextextended( \
                   'lattice.task-ledger.stream.v1:' || $1,0)",
                &[&stream_hex],
            )
            .expect("TASK105_FOREMAN_STREAM_LOCK_KEY")
            .get(0);
        client
            .query("SELECT pg_catalog.pg_advisory_lock($1)", &[&key])
            .expect("TASK105_FOREMAN_STREAM_LOCK_ACQUIRE");
        Self {
            client,
            key,
            held: true,
        }
    }

    fn acquire_fixed(config: &LiveConfig, key: i64) -> Self {
        let mut client = config.bootstrap_client();
        client
            .query("SELECT pg_catalog.pg_advisory_lock($1)", &[&key])
            .expect("TASK105_FIXED_ADVISORY_LOCK_ACQUIRE");
        Self {
            client,
            key,
            held: true,
        }
    }

    fn wait_for_one_ungranted_waiter_for(&mut self, timeout: Duration) -> Result<(), &'static str> {
        self.wait_for_one_ungranted_waiter_until(Instant::now() + timeout)
    }

    fn wait_for_one_ungranted_waiter_until(
        &mut self,
        deadline: Instant,
    ) -> Result<(), &'static str> {
        loop {
            let waiting: i64 = self
                .client
                .query_one(
                    "SELECT pg_catalog.count(*) \
                       FROM pg_catalog.pg_locks held \
                       JOIN pg_catalog.pg_locks waiting \
                         ON waiting.locktype=held.locktype \
                        AND waiting.database IS NOT DISTINCT FROM held.database \
                        AND waiting.classid IS NOT DISTINCT FROM held.classid \
                        AND waiting.objid IS NOT DISTINCT FROM held.objid \
                        AND waiting.objsubid IS NOT DISTINCT FROM held.objsubid \
                        AND waiting.mode=held.mode \
                      WHERE held.pid=pg_catalog.pg_backend_pid() \
                        AND held.locktype='advisory' AND held.granted \
                        AND waiting.pid<>held.pid AND NOT waiting.granted",
                    &[],
                )
                .map_err(|_| "TASK105_FOREMAN_STREAM_WAITER_QUERY")?
                .get(0);
            if waiting == 1 {
                return Ok(());
            }
            if waiting > 1 {
                return Err("TASK105_FOREMAN_STREAM_WAITER_DUPLICATE");
            }
            if Instant::now() >= deadline {
                return Err("TASK105_FOREMAN_STREAM_WAITER_TIMEOUT");
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn wait_for_bootstrap_session_count(
        &mut self,
        config: &LiveConfig,
        expected: i64,
        timeout: Duration,
    ) -> Result<(), &'static str> {
        let deadline = Instant::now() + timeout;
        loop {
            let sessions: i64 = self
                .client
                .query_one(
                    "SELECT pg_catalog.count(*) FROM pg_catalog.pg_stat_activity \
                      WHERE datname=$1 AND usename='lattice_migrator_login' \
                        AND application_name='lattice-devos-task019' \
                        AND pid<>pg_catalog.pg_backend_pid()",
                    &[&config.database_name()],
                )
                .map_err(|_| "TASK105_BOOTSTRAP_SESSION_QUERY")?
                .get(0);
            if sessions == expected {
                return Ok(());
            }
            if sessions > expected {
                return Err("TASK105_BOOTSTRAP_SESSION_DUPLICATE");
            }
            if Instant::now() >= deadline {
                return Err("TASK105_BOOTSTRAP_SESSION_TIMEOUT");
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn wait_for_outer_bootstrap_gate_proof(
        &mut self,
        config: &LiveConfig,
        timeout: Duration,
    ) -> Result<(), &'static str> {
        let deadline = Instant::now() + timeout;
        loop {
            let proof = self
                .client
                .query_one(
                    "SELECT \
                        (SELECT pg_catalog.count(*) FROM pg_catalog.pg_stat_activity \
                          WHERE datname=$1 AND usename='lattice_migrator_login' \
                            AND application_name='lattice-devos-task019' \
                            AND pid<>pg_catalog.pg_backend_pid() \
                            AND query LIKE '%pg_try_advisory_lock%'), \
                        (SELECT pg_catalog.count(*) FROM pg_catalog.pg_locks \
                          WHERE locktype='advisory' AND granted AND objsubid=1 \
                            AND ((classid::bigint << 32) | objid::bigint)=$2)",
                    &[
                        &config.database_name(),
                        &POSTGRES_BOOTSTRAP_GLOBAL_ADVISORY_LOCK,
                    ],
                )
                .map_err(|_| "TASK105_OUTER_BOOTSTRAP_GATE_QUERY")?;
            let contenders: i64 = proof.get(0);
            let holders: i64 = proof.get(1);
            if contenders == 1 && holders == 1 {
                return Ok(());
            }
            if contenders > 1 || holders > 1 {
                return Err("TASK105_OUTER_BOOTSTRAP_GATE_DUPLICATE");
            }
            if Instant::now() >= deadline {
                return Err("TASK105_OUTER_BOOTSTRAP_GATE_TIMEOUT");
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn release(&mut self) -> Result<(), &'static str> {
        if !self.held {
            return Ok(());
        }
        let unlocked: bool = self
            .client
            .query_one("SELECT pg_catalog.pg_advisory_unlock($1)", &[&self.key])
            .map_err(|_| "TASK105_FOREMAN_STREAM_LOCK_RELEASE")?
            .get(0);
        if !unlocked {
            return Err("TASK105_FOREMAN_STREAM_LOCK_NOT_HELD");
        }
        self.held = false;
        Ok(())
    }
}

impl Drop for ForemanStreamLock {
    fn drop(&mut self) {
        let _ = self.release();
    }
}

struct DisposableRaceDatabase<'a> {
    parent: &'a LiveConfig,
    child: LiveConfig,
    cleanup_pending: bool,
}

impl<'a> DisposableRaceDatabase<'a> {
    fn new(parent: &'a LiveConfig) -> Self {
        let child = parent.child_database(0x6000_0000);
        assert_ne!(child.run_id, parent.run_id);
        Self {
            parent,
            child,
            cleanup_pending: false,
        }
    }

    fn initialize(&mut self) {
        self.cleanup_pending = true;
        self.parent.revoke_login_database_privileges();
        self.parent.assert_login_capability(false);
        run_latticed_admin(&self.child, "--postgres-initialize", true);
        self.child.assert_login_capability(true);
        self.parent.assert_login_capability(false);
    }

    fn config(&self) -> LiveConfig {
        self.child.clone()
    }

    fn cleanup(&mut self) -> Result<(), &'static str> {
        if !self.cleanup_pending {
            return Ok(());
        }
        self.child.try_revoke_login_database_privileges()?;
        self.child.try_drop_database()?;
        run_latticed_admin(self.parent, "--postgres-initialize", true);
        self.cleanup_pending = false;
        Ok(())
    }
}

impl Drop for DisposableRaceDatabase<'_> {
    fn drop(&mut self) {
        if self.cleanup_pending {
            let _ = self.child.try_revoke_login_database_privileges();
            let _ = self.child.try_drop_database();
        }
    }
}

struct ForemanWorkerCorruption<'a> {
    config: &'a LiveConfig,
    command_id: Option<String>,
}

impl<'a> ForemanWorkerCorruption<'a> {
    fn introduce(config: &'a LiveConfig) -> Self {
        let command_id: String = config
            .bootstrap_client()
            .query_one(
                "SELECT command_id::text FROM ONLY control.task_ledger_foreman_snapshots \
                  ORDER BY generation DESC LIMIT 1",
                &[],
            )
            .expect("TASK105_FOREMAN_CORRUPT_TARGET")
            .get(0);
        let changed = config
            .bootstrap_client()
            .execute(
                "UPDATE ONLY control.task_ledger_foreman_snapshots \
                    SET worker_id='sole-foreman-v2' \
                  WHERE command_id=$1 AND worker_id='sole-foreman-v1'",
                &[&command_id],
            )
            .expect("TASK105_FOREMAN_CORRUPT_INTRODUCE");
        let guard = Self {
            config,
            command_id: Some(command_id),
        };
        assert_eq!(changed, 1);
        guard
    }

    fn restore(&mut self) -> Result<(), &'static str> {
        let Some(command_id) = self.command_id.as_ref() else {
            return Ok(());
        };
        let restored = self
            .config
            .bootstrap_client()
            .execute(
                "UPDATE ONLY control.task_ledger_foreman_snapshots \
                    SET worker_id='sole-foreman-v1' \
                  WHERE command_id=$1 AND worker_id='sole-foreman-v2'",
                &[command_id],
            )
            .map_err(|_| "TASK105_FOREMAN_CORRUPT_RESTORE")?;
        if restored != 1 {
            return Err("TASK105_FOREMAN_CORRUPT_RESTORE_COUNT");
        }
        self.command_id = None;
        Ok(())
    }
}

impl Drop for ForemanWorkerCorruption<'_> {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

struct RuntimeLoginDisabled<'a> {
    config: &'a LiveConfig,
    disabled: bool,
}

impl<'a> RuntimeLoginDisabled<'a> {
    fn introduce(config: &'a LiveConfig) -> Self {
        assert!(
            config
                .runtime_login_enabled()
                .expect("TASK105_RUNTIME_LOGIN_READ")
        );
        config
            .alter_runtime_login(false)
            .expect("TASK105_RUNTIME_LOGIN_DISABLE");
        let guard = Self {
            config,
            disabled: true,
        };
        assert_eq!(
            guard
                .config
                .runtime_login_enabled()
                .expect("TASK105_RUNTIME_LOGIN_VERIFY"),
            false
        );
        guard
    }

    fn restore(&mut self) -> Result<(), &'static str> {
        if self.disabled {
            self.config.alter_runtime_login(true)?;
            if !self.config.runtime_login_enabled()? {
                return Err("TASK105_RUNTIME_LOGIN_RESTORE_VERIFY");
            }
            self.disabled = false;
        }
        Ok(())
    }
}

impl Drop for RuntimeLoginDisabled<'_> {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

struct UnsupportedHistory<'a> {
    config: &'a LiveConfig,
    introduced: bool,
}

impl<'a> UnsupportedHistory<'a> {
    fn introduce(config: &'a LiveConfig, coherent_future_profile: bool) -> Self {
        config.introduce_unsupported_history(coherent_future_profile);
        Self {
            config,
            introduced: true,
        }
    }

    fn restore(&mut self) -> Result<(), &'static str> {
        if self.introduced {
            self.config.repair_unsupported_history()?;
            self.introduced = false;
        }
        Ok(())
    }
}

impl Drop for UnsupportedHistory<'_> {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

struct WriterContention {
    repository: PostgresWriterLease,
    project_id: ProjectId,
    authority: Option<WriterLeaseAuthorityHead>,
}

impl WriterContention {
    fn acquire(config: &LiveConfig) -> Self {
        let identity = foreman_coordination_identity().expect("TASK105_FOREMAN_IDENTITY");
        let database_identity = ContentDigest::from_sha256(
            config
                .migration_target()
                .expected_database_identity_sha256()
                .as_str(),
        )
        .expect("TASK105_WRITER_DATABASE_IDENTITY");
        let target = V5ExtensionTarget::new(config.database_name(), database_identity)
            .expect("TASK105_WRITER_V5_TARGET");
        let mut repository = PostgresWriterLease::new_v5_v7(
            config.runtime_client(),
            &target,
            &store_authority_from_environment(),
            600,
        )
        .expect("TASK105_WRITER_CONTENTION_REPOSITORY");
        let Some(task_spec_digest) = identity.task_spec_digest().cloned() else {
            panic!("TASK105_FOREMAN_TASK_SPEC_IDENTITY");
        };
        let acquired = repository
            .execute(WriterLeaseRepositoryCommand::Acquire(
                WriterLeaseAcquireRequest {
                    command_id: "task105-runtime-status-contention-acquire".to_owned(),
                    expected_head: None,
                    project_id: identity.project_id().clone(),
                    project_snapshot_id: identity.project_snapshot_id().clone(),
                    task_id: identity.task_id().clone(),
                    task_revision: identity.task_revision().to_owned(),
                    task_spec_digest,
                    attempt_id: AttemptId::new("task105-runtime-status-contention")
                        .expect("TASK105_WRITER_ATTEMPT"),
                    lease_id: "task105-runtime-status-contention-lease".to_owned(),
                    lease_holder_id: "task105-runtime-status-contention-holder".to_owned(),
                    worktree_id: "task105-durable-foreman-runtime".to_owned(),
                    holder_process_id: HolderProcessId::new(u64::from(std::process::id()))
                        .expect("TASK105_WRITER_PROCESS"),
                    holder_process_start_identity: ContentDigest::from_sha256("f".repeat(64))
                        .expect("TASK105_WRITER_PROCESS_START"),
                },
            ))
            .expect("TASK105_WRITER_CONTENTION_ACQUIRE");
        assert_eq!(acquired.outcome, LeaseCommandOutcome::Applied);
        let authority = acquired.after.expect("TASK105_WRITER_CONTENTION_HEAD");
        let mut guard = Self {
            repository,
            project_id: identity.project_id().clone(),
            authority: Some(authority),
        };
        assert!(
            guard
                .repository
                .current_authority(identity.project_id())
                .expect("TASK105_WRITER_CONTENTION_CURRENT")
                .is_some()
        );
        guard
    }

    fn release(&mut self) -> Result<(), &'static str> {
        let Some(authority) = self.authority.as_ref() else {
            return Ok(());
        };
        let released = self
            .repository
            .execute(WriterLeaseRepositoryCommand::Release(
                WriterLeaseReleaseRequest {
                    command_id: "task105-runtime-status-contention-release".to_owned(),
                    project_id: self.project_id.clone(),
                    expected_head: authority.clone(),
                },
            ))
            .map_err(|_| "TASK105_WRITER_CONTENTION_RELEASE")?;
        if released.outcome != LeaseCommandOutcome::Applied || released.after.is_some() {
            return Err("TASK105_WRITER_CONTENTION_RELEASE_RECEIPT");
        }
        if self
            .repository
            .current_authority(&self.project_id)
            .map_err(|_| "TASK105_WRITER_CONTENTION_RELEASED_CURRENT")?
            .is_some()
        {
            return Err("TASK105_WRITER_CONTENTION_RELEASED_ACTIVE");
        }
        self.authority = None;
        Ok(())
    }
}

impl Drop for WriterContention {
    fn drop(&mut self) {
        if self.authority.is_some() {
            let _ = self.release();
        }
    }
}

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("TASK105_ENV_MISSING:{name}"))
}

fn store_authority_from_environment() -> StoreAuthorityHead {
    StoreAuthorityHead::new(
        RuntimeKind::Live,
        StoreDaemonInstanceId::new(required("LATTICE_STORE_DAEMON_INSTANCE_ID"))
            .expect("TASK105_STORE_DAEMON"),
        DaemonEpoch::new(
            required("LATTICE_STORE_DAEMON_EPOCH")
                .parse()
                .expect("TASK105_STORE_EPOCH_VALUE"),
        )
        .expect("TASK105_STORE_EPOCH"),
        RuntimeAdmissionMode::Active,
        StoreAuthorityRevision::new(
            required("LATTICE_STORE_AUTHORITY_REVISION")
                .parse()
                .expect("TASK105_STORE_REVISION_VALUE"),
        )
        .expect("TASK105_STORE_REVISION"),
        ContentDigest::from_sha256(required("LATTICE_STORE_OBSERVATION_DIGEST"))
            .expect("TASK105_STORE_OBSERVATION"),
        ContentDigest::from_sha256(required("LATTICE_STORE_AUTHORITY_HEAD_DIGEST"))
            .expect("TASK105_STORE_HEAD"),
    )
    .expect("TASK105_STORE_AUTHORITY")
}

fn assert_no_merged_sql_continuation_tokens() {
    let source = include_str!("task105_durable_foreman_runtime.rs").as_bytes();
    for continuation in 1..source.len().saturating_sub(1) {
        if source[continuation] != b'\\'
            || !(source[continuation - 1].is_ascii_alphanumeric()
                || source[continuation - 1] == b'_')
        {
            continue;
        }
        let mut next = continuation + 1;
        if source[next] == b'\r' {
            next += 1;
        }
        if source.get(next) != Some(&b'\n') {
            continue;
        }
        next += 1;
        while source.get(next).is_some_and(u8::is_ascii_whitespace) {
            next += 1;
        }
        assert!(
            source
                .get(next)
                .is_none_or(|byte| !(byte.is_ascii_alphabetic() || *byte == b'_')),
            "TASK105_SQL_CONTINUATION_MERGED_TOKEN_AT_BYTE:{continuation}"
        );
    }
}

fn run_latticed_admin(config: &LiveConfig, argument: &str, expected_success: bool) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_latticed"))
        .arg(argument)
        .env("LATTICE_TASK019_RUN_ID", &config.run_id)
        .output()
        .expect("TASK105_LATTICED_ADMIN_START");
    assert_eq!(
        output.status.success(),
        expected_success,
        "TASK105_LATTICED_ADMIN_STATUS:{argument}:{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8(output.stderr).expect("TASK105_LATTICED_ADMIN_STDERR");
    if expected_success {
        let expected = if argument == "--postgres-initialize" {
            "LATTICE_POSTGRES_INITIALIZE_READY\n"
        } else {
            "LATTICE_POSTGRES_BOOTSTRAP_READY\n"
        };
        assert_eq!(stderr.replace("\r\n", "\n"), expected);
    } else {
        assert!(!stderr.contains("READY"));
        assert!(stderr.len() <= 128);
    }
    stderr
}

fn run_concurrent_bootstrap_retry(config: &LiveConfig) {
    let mut foreman_apply_lock = ForemanStreamLock::acquire_fixed(config, 7_212_400_260_826);
    thread::scope(|scope| {
        let runner_a = scope.spawn(|| run_latticed_admin(config, "--postgres-bootstrap", true));
        foreman_apply_lock
            .wait_for_one_ungranted_waiter_for(Duration::from_secs(35))
            .expect("TASK105_CONCURRENT_FOREMAN_WAITER");
        let runner_b = scope.spawn(|| run_latticed_admin(config, "--postgres-bootstrap", true));
        foreman_apply_lock
            .wait_for_bootstrap_session_count(config, 2, Duration::from_secs(3))
            .expect("TASK105_CONCURRENT_BOOTSTRAP_SESSIONS");
        foreman_apply_lock
            .wait_for_outer_bootstrap_gate_proof(config, Duration::from_secs(3))
            .expect("TASK105_CONCURRENT_OUTER_GATE_PROOF");
        foreman_apply_lock
            .release()
            .expect("TASK105_CONCURRENT_FOREMAN_RELEASE");
        for runner in [runner_a, runner_b] {
            let stderr = runner.join().expect("TASK105_BOOTSTRAP_RUNNER_JOIN");
            assert_eq!(
                stderr.replace("\r\n", "\n"),
                "LATTICE_POSTGRES_BOOTSTRAP_READY\n"
            );
        }
    });
}

fn run_latticed(requests: &[Value]) -> Vec<Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_latticed"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("TASK105_LATTICED_START");
    let input = requests
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    child
        .stdin
        .take()
        .expect("TASK105_LATTICED_STDIN")
        .write_all(input.as_bytes())
        .expect("TASK105_LATTICED_WRITE");
    let output = child.wait_with_output().expect("TASK105_LATTICED_WAIT");
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        panic!("TASK105_LATTICED_FAILED:{stderr}");
    }
    String::from_utf8(output.stdout)
        .expect("TASK105_LATTICED_UTF8")
        .lines()
        .map(|line| serde_json::from_str(line).expect("TASK105_LATTICED_JSON"))
        .collect()
}

fn poll_child_exit(child: &mut Child, timeout: Duration) -> Option<std::process::ExitStatus> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            return Some(status);
        }
        if Instant::now() >= deadline {
            return None;
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn terminate_exact_process_tree(pid: u32) -> Result<(), &'static str> {
    let Some(system_root) = env::var_os("SystemRoot") else {
        return Err("TASK105_SYSTEM_ROOT_MISSING");
    };
    let executable = std::path::PathBuf::from(system_root)
        .join("System32")
        .join("taskkill.exe");
    let mut terminator = match Command::new(executable)
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return Err("TASK105_TASKKILL_START"),
    };
    if poll_child_exit(&mut terminator, Duration::from_secs(5)).is_some() {
        return Ok(());
    }
    let _ = terminator.kill();
    if poll_child_exit(&mut terminator, Duration::from_secs(2)).is_some() {
        Err("TASK105_TASKKILL_TIMEOUT")
    } else {
        Err("TASK105_TASKKILL_CLEANUP_FAILED")
    }
}

struct InteractiveLatticed {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: Receiver<Result<String, String>>,
    stdout_reader: Option<JoinHandle<()>>,
    stderr_reader: Option<JoinHandle<String>>,
    finished: bool,
}

impl InteractiveLatticed {
    fn start_with_run_id(run_id: Option<&str>) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_latticed"));
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(run_id) = run_id {
            command.env("LATTICE_TASK019_RUN_ID", run_id);
        }
        let mut child = command.spawn().expect("TASK105_INTERACTIVE_LATTICED_START");
        let stdin = child.stdin.take().expect("TASK105_INTERACTIVE_STDIN");
        let child_stdout = child.stdout.take().expect("TASK105_INTERACTIVE_STDOUT");
        let child_stderr = child.stderr.take().expect("TASK105_INTERACTIVE_STDERR");
        let (sender, stdout) = mpsc::channel();
        let stdout_reader = thread::spawn(move || {
            for line in BufReader::new(child_stdout).lines() {
                let message = line.map_err(|error| error.to_string());
                if sender.send(message).is_err() {
                    break;
                }
            }
        });
        let stderr_reader = thread::spawn(move || {
            let mut stderr = String::new();
            BufReader::new(child_stderr)
                .read_to_string(&mut stderr)
                .expect("TASK105_INTERACTIVE_STDERR_READ");
            stderr
        });
        Self {
            child,
            stdin: Some(stdin),
            stdout,
            stdout_reader: Some(stdout_reader),
            stderr_reader: Some(stderr_reader),
            finished: false,
        }
    }

    fn start_initialized() -> Self {
        let mut session = Self::start_with_run_id(None);
        session.initialize();
        session
    }

    fn start_for(config: &LiveConfig) -> Self {
        Self::start_with_run_id(Some(&config.run_id))
    }

    fn start_for_dependency(config: &LiveConfig, fixture: &DependencyGitFixture) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_latticed"));
        command
            .env("LATTICE_TASK019_RUN_ID", &config.run_id)
            .env("LATTICE_GRAPHIFY_SOURCE_ROOT", &fixture.repository)
            .env("LATTICE_DEPENDENCY_WORKTREE_ROOT", &fixture.dependency_root)
            .env("LATTICE_DELIVERY_GIT_EXE", &fixture.git)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().expect("TASK106_INTERACTIVE_LATTICED_START");
        let stdin = child.stdin.take().expect("TASK106_INTERACTIVE_STDIN");
        let child_stdout = child.stdout.take().expect("TASK106_INTERACTIVE_STDOUT");
        let child_stderr = child.stderr.take().expect("TASK106_INTERACTIVE_STDERR");
        let (sender, stdout) = mpsc::channel();
        let stdout_reader = thread::spawn(move || {
            for line in BufReader::new(child_stdout).lines() {
                let message = line.map_err(|error| error.to_string());
                if sender.send(message).is_err() {
                    break;
                }
            }
        });
        let stderr_reader = thread::spawn(move || {
            let mut stderr = String::new();
            BufReader::new(child_stderr)
                .read_to_string(&mut stderr)
                .expect("TASK106_INTERACTIVE_STDERR_READ");
            stderr
        });
        Self {
            child,
            stdin: Some(stdin),
            stdout,
            stdout_reader: Some(stdout_reader),
            stderr_reader: Some(stderr_reader),
            finished: false,
        }
    }

    fn pid(&self) -> u32 {
        self.child.id()
    }

    fn initialize(&mut self) {
        let initialized = self.request(&initialize_request());
        assert_eq!(initialized["id"], 1);
        assert!(initialized.get("error").is_none());
        self.send(&json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
    }

    fn send(&mut self, request: &Value) {
        let stdin = self.stdin.as_mut().expect("TASK105_INTERACTIVE_STDIN_OPEN");
        writeln!(stdin, "{request}").expect("TASK105_INTERACTIVE_WRITE");
        stdin.flush().expect("TASK105_INTERACTIVE_FLUSH");
    }

    fn request(&mut self, request: &Value) -> Value {
        let expected_id = request
            .get("id")
            .cloned()
            .expect("TASK105_INTERACTIVE_REQUEST_ID");
        self.send(request);
        self.receive_with_timeout(&expected_id, Duration::from_secs(35))
    }

    fn receive_with_timeout(&mut self, expected_id: &Value, timeout: Duration) -> Value {
        let line = self
            .stdout
            .recv_timeout(timeout)
            .expect("TASK105_INTERACTIVE_RESPONSE_TIMEOUT")
            .expect("TASK105_INTERACTIVE_STDOUT_READ");
        let response: Value = serde_json::from_str(&line).expect("TASK105_INTERACTIVE_JSON");
        assert_eq!(response["jsonrpc"], "2.0");
        assert_eq!(&response["id"], expected_id);
        response
    }

    fn recv_expected(&mut self, id: i64, timeout: Duration) -> Value {
        self.receive_with_timeout(&json!(id), timeout)
    }

    fn request_status(&mut self, id: i64) -> Value {
        self.request(&runtime_status_request(id))
    }

    fn wait_bounded(
        &mut self,
        graceful: Duration,
    ) -> Result<std::process::ExitStatus, &'static str> {
        if let Some(status) = poll_child_exit(&mut self.child, graceful) {
            return Ok(status);
        }
        let owned_pid = self.child.id();
        let _ = self.child.kill();
        if let Some(status) = poll_child_exit(&mut self.child, Duration::from_secs(5)) {
            return Ok(status);
        }
        let _ = terminate_exact_process_tree(owned_pid);
        if let Some(status) = poll_child_exit(&mut self.child, Duration::from_secs(5)) {
            return Ok(status);
        }
        let _ = self.child.kill();
        poll_child_exit(&mut self.child, Duration::from_secs(2))
            .ok_or("TASK105_INTERACTIVE_CLEANUP_FAILED")
    }

    fn join_readers(&mut self) -> Result<String, &'static str> {
        self.stdout_reader
            .take()
            .ok_or("TASK105_INTERACTIVE_STDOUT_THREAD")?
            .join()
            .map_err(|_| "TASK105_INTERACTIVE_STDOUT_JOIN")?;
        let stderr = self
            .stderr_reader
            .take()
            .ok_or("TASK105_INTERACTIVE_STDERR_THREAD")?
            .join()
            .map_err(|_| "TASK105_INTERACTIVE_STDERR_JOIN")?;
        Ok(stderr)
    }

    fn finish(mut self) {
        self.stdin.take();
        let status = self
            .wait_bounded(Duration::from_secs(10))
            .expect("TASK105_INTERACTIVE_EXIT");
        let stderr = self
            .join_readers()
            .expect("TASK105_INTERACTIVE_READER_JOIN");
        self.finished = true;
        assert!(status.success(), "TASK105_INTERACTIVE_FAILED:{stderr}");
        assert_bounded_startup_diagnostics(&stderr);
    }
}

fn assert_bounded_startup_diagnostics(stderr: &str) {
    assert!(
        stderr.len() <= 16 * 1024,
        "TASK105_INTERACTIVE_STDERR_UNBOUNDED"
    );
    let lines = stderr.lines().collect::<Vec<_>>();
    assert!(!lines.is_empty(), "TASK105_INTERACTIVE_STDERR_EMPTY");
    assert!(lines.len() <= 64, "TASK105_INTERACTIVE_STDERR_LINES");
    let expected_keys = [
        "configuration_health",
        "dependency_health",
        "failure_classification",
        "last_completed_stage",
        "schema",
        "stage",
        "waiting_reason",
    ];
    for line in lines {
        assert!(line.len() <= 512, "TASK105_INTERACTIVE_STDERR_LINE");
        let value = serde_json::from_str::<Value>(line)
            .expect("TASK105_INTERACTIVE_STARTUP_DIAGNOSTIC_JSON");
        let object = value
            .as_object()
            .expect("TASK105_INTERACTIVE_STARTUP_DIAGNOSTIC_OBJECT");
        let mut keys = object.keys().map(String::as_str).collect::<Vec<_>>();
        keys.sort_unstable();
        assert_eq!(keys, expected_keys);
        assert_eq!(
            object["schema"],
            Value::String("lattice.latticed.startup-diagnostic.v1".to_owned())
        );
        for key in expected_keys.into_iter().filter(|key| *key != "schema") {
            let field = object[key]
                .as_str()
                .expect("TASK105_INTERACTIVE_STARTUP_DIAGNOSTIC_FIELD");
            assert!(!field.is_empty() && field.len() <= 64);
            assert!(
                field
                    .bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_'),
                "TASK105_INTERACTIVE_STARTUP_DIAGNOSTIC_VOCABULARY"
            );
        }
    }
}

impl Drop for InteractiveLatticed {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        self.stdin.take();
        match self.wait_bounded(Duration::from_millis(200)) {
            Ok(_) => {
                let _ = self.join_readers();
            }
            Err(code) => {
                eprintln!("TASK105_INTERACTIVE_CLEANUP_FAILED:{code}");
                self.stdout_reader.take();
                self.stderr_reader.take();
            }
        }
        self.finished = true;
    }
}

fn runtime_status_request(id: i64) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":"lattice_runtime_status"}})
}

fn assert_foreman_replay_error(response: &Value, code: &str) {
    assert_eq!(response["result"]["isError"], true);
    let expected = json!({"status":"ERROR","code":code});
    assert_eq!(response["result"]["structuredContent"], expected);
    assert_eq!(
        response["result"]["content"],
        json!([{"type":"text","text":expected.to_string()}])
    );
    assert!(response.to_string().len() <= 1024);
    assert!(
        response["result"]["structuredContent"]
            .get("foreman")
            .is_none()
    );
}

fn initialize_request() -> Value {
    json!({
        "jsonrpc":"2.0", "id":1, "method":"initialize",
        "params":{
            "protocolVersion":"2025-11-25", "capabilities":{},
            "clientInfo":{"name":"task105-live","version":"1"}
        }
    })
}

struct DependencyGitFixture {
    root: PathBuf,
    repository: PathBuf,
    dependency_root: PathBuf,
    child: PathBuf,
    base_sha: String,
    git: PathBuf,
}

impl DependencyGitFixture {
    fn new() -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("TASK106_FIXTURE_CLOCK")
            .as_nanos();
        let root = env::temp_dir().join(format!(
            "lattice-task106-dependency-{}-{unique}",
            std::process::id()
        ));
        let repository = root.join("repository");
        let dependency_root = root.join("dependency-worktrees");
        let child = dependency_root.join("task-107-worktree");
        fs::create_dir_all(&repository).expect("TASK106_REPOSITORY_ROOT");
        let git =
            fs::canonicalize(required("LATTICE_DELIVERY_GIT_EXE")).expect("TASK106_GIT_EXECUTABLE");
        let run = |cwd: &Path, arguments: &[&str]| {
            let output = Command::new(&git)
                .current_dir(cwd)
                .args(arguments)
                .output()
                .expect("TASK106_GIT_PROCESS");
            assert!(
                output.status.success(),
                "TASK106_GIT_REJECTED:{:?}:{}",
                arguments,
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8(output.stdout)
                .expect("TASK106_GIT_STDOUT")
                .trim()
                .to_owned()
        };
        run(&repository, &["init", "-b", "product-parent"]);
        run(&repository, &["config", "user.name", "LATTICE Test"]);
        run(
            &repository,
            &["config", "user.email", "lattice-test@invalid.example"],
        );
        fs::write(repository.join("base.txt"), b"base\n").expect("TASK106_BASE_FILE");
        run(&repository, &["add", "base.txt"]);
        run(&repository, &["commit", "-m", "base"]);
        let base_sha = run(&repository, &["rev-parse", "HEAD"]);
        let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("scripts")
            .join("lattice-dependency-worktree.mjs");
        assert!(script.is_file(), "TASK106_DEPENDENCY_CLI");
        let cli = Command::new(if cfg!(windows) { "node.exe" } else { "node" })
            .current_dir(&repository)
            .env("LATTICE_DEPENDENCY_WORKTREE_ROOT", &dependency_root)
            .args([
                script.as_os_str(),
                OsStr::new("create"),
                OsStr::new("TASK-106"),
                OsStr::new("TASK-107"),
                OsStr::new("TASK-107-WORKTREE"),
                OsStr::new(&base_sha),
            ])
            .output()
            .expect("TASK106_DEPENDENCY_CLI_PROCESS");
        assert!(
            cli.status.success() && cli.stderr.is_empty(),
            "TASK106_DEPENDENCY_CLI_REJECTED:{}",
            String::from_utf8_lossy(&cli.stderr)
        );
        let cli_binding =
            serde_json::from_slice::<Value>(&cli.stdout).expect("TASK106_DEPENDENCY_CLI_JSON");
        assert_eq!(cli_binding["parent_task_id"], "TASK-106");
        assert_eq!(cli_binding["dependency_task_id"], "TASK-107");
        assert_eq!(cli_binding["dependency_branch"], "lattice/task-107");
        assert_eq!(cli_binding["base_sha"], base_sha);
        run(&child, &["config", "user.name", "LATTICE Test"]);
        run(
            &child,
            &["config", "user.email", "lattice-test@invalid.example"],
        );
        let repository = fs::canonicalize(repository).expect("TASK106_REPOSITORY_CANONICAL");
        let dependency_root =
            fs::canonicalize(dependency_root).expect("TASK106_DEPENDENCY_ROOT_CANONICAL");
        let child = fs::canonicalize(child).expect("TASK106_CHILD_CANONICAL");
        Self {
            root,
            repository,
            dependency_root,
            child,
            base_sha,
            git,
        }
    }

    fn blocker(&self) -> Value {
        json!({
            "schema": "lattice.dependency-blocker/1.0",
            "parent_task_id": "TASK-106",
            "dependency_task_id": "TASK-107",
            "dependency_worktree_id": "TASK-107-WORKTREE",
            "dependency_branch": "lattice/task-107",
            "base_sha": self.base_sha,
            "next_action": "COMPLETE_DEPENDENCY",
        })
    }

    fn commit_dependency(&self) {
        fs::write(self.child.join("dependency.txt"), b"dependency\n")
            .expect("TASK106_DEPENDENCY_FILE");
        self.git(&self.child, &["add", "dependency.txt"]);
        self.git(&self.child, &["commit", "-m", "dependency"]);
    }

    fn merge_dependency(&self) {
        self.git(
            &self.repository,
            &[
                "merge",
                "--no-ff",
                "lattice/task-107",
                "-m",
                "merge dependency",
            ],
        );
    }

    fn git(&self, cwd: &Path, arguments: &[&str]) -> String {
        let output = Command::new(&self.git)
            .current_dir(cwd)
            .args(arguments)
            .output()
            .expect("TASK106_GIT_PROCESS");
        assert!(
            output.status.success(),
            "TASK106_GIT_REJECTED:{:?}:{}",
            arguments,
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout)
            .expect("TASK106_GIT_STDOUT")
            .trim()
            .to_owned()
    }

    fn cleanup(self) {
        self.git(
            &self.repository,
            &[
                "worktree",
                "remove",
                self.child.to_str().expect("TASK106_CHILD_PATH"),
            ],
        );
        fs::remove_dir_all(&self.root).expect("TASK106_FIXTURE_CLEANUP");
    }
}

fn checkpoint(
    id: i64,
    checkpoint_id: &str,
    generation: u64,
    state: &str,
    blocker: Value,
    evidence: char,
) -> Value {
    json!({
        "jsonrpc":"2.0", "id":id, "method":"tools/call",
        "params":{
            "name":"lattice_foreman_checkpoint",
            "arguments":{
                "checkpoint_id":checkpoint_id,
                "generation":generation,
                "occurred_at":if generation == 1 { "2026-08-25T00:00:01Z" } else { "2026-08-25T00:00:02Z" },
                "state":state,
                "blocker_ref":blocker,
                "heartbeat_ref":format!("heartbeat:sha256:{}", "a".repeat(64)),
                "evidence_ref":format!("evidence:sha256:{}", evidence.to_string().repeat(64))
            }
        }
    })
}

fn response<'a>(responses: &'a [Value], id: i64) -> &'a Value {
    responses
        .iter()
        .find(|value| value["id"] == id)
        .unwrap_or_else(|| panic!("TASK105_RESPONSE_MISSING:{id}"))
}

fn run_focused_dual_process_race(config: &LiveConfig) {
    let mut race_database = DisposableRaceDatabase::new(config);
    race_database.initialize();
    let race = race_database.config();
    run_latticed_admin(&race, "--postgres-bootstrap", true);
    race.assert_v8_writer_v5_successor();

    let mut process_a = InteractiveLatticed::start_for(&race);
    process_a.initialize();
    let mut process_b = InteractiveLatticed::start_for(&race);
    process_b.initialize();
    let mut stream_lock = ForemanStreamLock::acquire(&race);
    let generation_one = checkpoint(
        2,
        "task105-race-checkpoint-1",
        1,
        "ACTIVE",
        Value::Null,
        'b',
    );
    process_a.send(&generation_one);
    stream_lock
        .wait_for_one_ungranted_waiter_for(Duration::from_secs(35))
        .expect("TASK105_FOCUSED_FOREMAN_STREAM_WAITER");
    assert_eq!(
        race.foreman_counts(),
        ([0, 0, 0], ["0".into(), "0".into(), "0".into()], None)
    );

    let coordination_identity = foreman_coordination_identity().expect("TASK105_FOREMAN_IDENTITY");
    let mut writer_observer = foreman_writer_repository(&race);
    let current = writer_observer
        .current_authority(coordination_identity.project_id())
        .expect("TASK105_FOCUSED_WRITER_CURRENT")
        .expect("TASK105_FOCUSED_WRITER_AUTHORITY_MISSING");
    assert_eq!(
        current.independent_head().identity().holder_process_id().get(),
        u64::from(process_a.pid())
    );
    let mut authority_cleanup = RaceAuthorityCleanup::new(
        writer_observer,
        coordination_identity.project_id().clone(),
        current.independent_head().clone(),
    );

    let contender_checkpoint_id = "task105-focused-race-contender";
    process_b.send(&checkpoint(
        3,
        contender_checkpoint_id,
        1,
        "ACTIVE",
        Value::Null,
        'c',
    ));
    assert_foreman_replay_error(
        &process_b.recv_expected(3, Duration::from_secs(35)),
        "FOREMAN_REPLAY_UNAVAILABLE",
    );
    race.assert_writer_command_absent(contender_checkpoint_id);
    assert_eq!(
        race.foreman_counts(),
        ([0, 0, 0], ["0".into(), "0".into(), "0".into()], None)
    );

    stream_lock
        .release()
        .expect("TASK105_FOCUSED_FOREMAN_STREAM_UNLOCK");
    drop(stream_lock);
    let recorded = process_a.recv_expected(2, Duration::from_secs(35));
    assert_eq!(recorded["result"]["isError"], false);
    let first = recorded["result"]["structuredContent"].clone();
    assert_eq!(first["status"], "RECORDED");
    assert_eq!(first["exact_retry"], false);
    authority_cleanup
        .disarm_after_release()
        .expect("TASK105_FOCUSED_RACE_AUTHORITY_RELEASED");
    drop(authority_cleanup);
    process_a.finish();

    let replayed = process_b.request(&checkpoint(
        4,
        "task105-race-checkpoint-1",
        1,
        "ACTIVE",
        Value::Null,
        'b',
    ));
    assert_eq!(replayed["result"]["isError"], false);
    assert_eq!(replayed["result"]["structuredContent"]["status"], "REPLAYED");
    assert_eq!(
        replayed["result"]["structuredContent"]["ledger_digest"],
        first["ledger_digest"]
    );
    assert_eq!(
        replayed["result"]["structuredContent"]["checkpoint_digest"],
        first["checkpoint_digest"]
    );

    let generation_two = process_b.request(&checkpoint(
        5,
        "task105-race-checkpoint-2",
        2,
        "BLOCKED",
        json!("TASK-094"),
        'c',
    ));
    assert_eq!(generation_two["result"]["isError"], false);
    assert_eq!(
        generation_two["result"]["structuredContent"]["status"],
        "RECORDED"
    );
    assert_eq!(
        race.foreman_counts(),
        (
            [2, 2, 2],
            ["2".into(), "2".into(), "2".into()],
            generation_two["result"]["structuredContent"]["ledger_digest"]
                .as_str()
                .map(ToOwned::to_owned)
        )
    );
    process_b.finish();
    race_database
        .cleanup()
        .expect("TASK105_FOCUSED_RACE_DATABASE_CLEANUP");
    println!("TASK105_FOCUSED_DUAL_PROCESS_RACE_PASS");
}

#[test]
fn task105_checkpoint_survives_a_fresh_latticed_process_without_migration() {
    assert_no_merged_sql_continuation_tokens();
    assert_eq!(
        independent_manifest_sha256(false),
        CURRENT_V8_MANIFEST_SHA256
    );
    assert_eq!(future_v9_manifest_sha256(), FUTURE_V9_MANIFEST_SHA256);
    let source = include_str!("task105_durable_foreman_runtime.rs");
    let runtime_client = source
        .split_once("fn runtime_client(&self) -> Client")
        .expect("TASK105_RUNTIME_CLIENT_SOURCE")
        .1
        .split_once("fn runtime_login_enabled")
        .expect("TASK105_RUNTIME_CLIENT_BOUNDARY")
        .0;
    assert!(runtime_client.contains(".application_name(\"lattice-devos-task019\")"));
    assert!(!runtime_client.contains("lattice-task105-writer-contention"));
    for marker in [
        "TASK105_STAGE_FOREMAN_REPLAY_CORRUPT_PASS",
        "TASK105_STAGE_FOREMAN_REPLAY_UNAVAILABLE_PASS",
        "TASK105_STAGE_FOREMAN_WRITER_CONTENTION_PASS",
        "TASK105_STAGE_FOREMAN_REPLAY_EXTRA_HISTORY_CORRUPT_PASS",
        "TASK105_STAGE_FOREMAN_REPLAY_UNSUPPORTED_PASS",
        "TASK105_STAGE_FOREMAN_DUAL_PROCESS_RACE_PASS",
        "TASK105_STAGE_LEGACY_V8_FOREMAN_STOPPED_PASS",
        "TASK105_STAGE_BOOTSTRAP_DUAL_RUNNER_RETRY_PASS",
    ] {
        assert_eq!(
            source.match_indices(marker).count(),
            2,
            "TASK105_FAULT_STAGE_MISSING:{marker}"
        );
    }
    for race_contract in [
        "child_database(0x6000_0000)",
        "start_for(&race)",
        "process_a.send(&generation_one)",
        "process_a.recv_expected(2, Duration::from_secs(35))",
        "assert_eq!(same_generation_one[\"params\"], generation_one[\"params\"])",
        "process_b.send(&same_generation_one)",
        "lattice.task-ledger.stream.v1:",
        "wait_for_one_ungranted_waiter_for",
        "NOT waiting.granted",
        "process_b.recv_expected(3, Duration::from_secs(35))",
        "process_b.recv_expected(4, Duration::from_secs(35))",
        "Duration::from_millis(20)",
        "task105-race-contender",
        "holder_process_id().get()",
        "race.assert_writer_command_absent(contender_checkpoint_id)",
        "pg_catalog.encode(s.head_digest,'hex')",
        "TASK105_RACE_GENERATION_ONE_LEDGER_DIGEST",
        "TASK105_RACE_GENERATION_TWO_LEDGER_DIGEST",
        "race.foreman_counts()",
        "race.try_bootstrap_client().is_err()",
        "LEGACY_V8_MANIFEST_SHA256",
        "prepare_legacy_v8_foreman_base",
        "7_212_400_260_826",
        "wait_for_outer_bootstrap_gate_proof",
        "pg_try_advisory_lock",
        "0x4c41_5454_4943_4501",
        "LATTICED_RUNTIME_POSTGRES_FOREMAN_REJECTED",
        "assert_v8_writer_v5_foreman_pending_stopped",
        "run_concurrent_bootstrap_retry",
        "wait_for_bootstrap_session_count",
        "pg_stat_activity",
    ] {
        assert!(
            source.contains(race_contract),
            "TASK105_RACE_CONTRACT_MISSING:{race_contract}"
        );
    }
    assert_eq!(
        source
            .match_indices("wait_for_outer_bootstrap_gate_proof")
            .count(),
        4,
        "TASK105_OUTER_BOOTSTRAP_GATE_PROOF_MISSING"
    );
    for query in FRESH_CATALOG_FINGERPRINT_QUERIES {
        assert!(query.contains(" JOIN "));
        assert!(query.contains(" WHERE "));
        for merged_token in [
            "nJOIN",
            "cJOIN",
            "pJOIN",
            "nspownerWHERE",
            "relnamespaceWHERE",
            "pronamespaceWHERE",
        ] {
            assert!(!query.contains(merged_token));
        }
    }
    let Some(config) = LiveConfig::from_environment() else {
        return;
    };
    if env::var("LATTICE_TASK105_FOCUSED_DUAL_PROCESS_RACE")
        .ok()
        .as_deref()
        == Some("1")
    {
        run_latticed_admin(&config, "--postgres-initialize", true);
        run_focused_dual_process_race(&config);
        return;
    }
    println!("TASK105_STAGE_INITIALIZE_ENTER");
    run_latticed_admin(&config, "--postgres-initialize", true);
    config.prepare_v5_writer_v3_bridge();
    config.assert_v5_writer_v3_bridge();
    config.prove_v5_bridge_retry_does_not_repair_rebind_boundary();
    println!("TASK105_STAGE_V5_BRIDGE_RETRY_VERIFY_ONLY_PASS");
    config.advance_v5_to_v6();
    config.assert_v6_writer_v3_current();
    let old_v3_v6_current = config.durable_profile_fingerprint();
    config.prepare_legacy_v8_foreman_base();
    assert_eq!(
        config.migration_fingerprint(),
        (9, 8, LEGACY_V8_MANIFEST_SHA256.to_owned())
    );
    let mut foreman_apply_lock = ForemanStreamLock::acquire_fixed(&config, 7_212_400_260_826);
    let foreman_failure = thread::scope(|scope| {
        let runner = scope.spawn(|| run_latticed_admin(&config, "--postgres-bootstrap", false));
        foreman_apply_lock
            .wait_for_one_ungranted_waiter_for(Duration::from_secs(35))
            .expect("TASK105_FOREMAN_APPLY_WAITER");
        config.assert_v8_writer_v5_foreman_pending_stopped();
        runner.join().expect("TASK105_FOREMAN_FAILURE_JOIN")
    });
    assert_eq!(
        foreman_failure.replace("\r\n", "\n").trim(),
        "LATTICED_RUNTIME_POSTGRES_FOREMAN_REJECTED"
    );
    config.assert_v8_writer_v5_foreman_pending_stopped();
    foreman_apply_lock
        .release()
        .expect("TASK105_FOREMAN_APPLY_LOCK_RELEASE");
    println!("TASK105_STAGE_LEGACY_V8_FOREMAN_STOPPED_PASS");

    run_concurrent_bootstrap_retry(&config);
    config.assert_v8_writer_v5_successor();
    config.assert_configured_authority_active();
    assert_ne!(config.durable_profile_fingerprint(), old_v3_v6_current);
    println!("TASK105_STAGE_BOOTSTRAP_DUAL_RUNNER_RETRY_PASS");
    println!("TASK105_STAGE_V6_V3_CURRENT_TO_V8_V5_SUCCESSOR_PASS");

    let migration_before = config.migration_fingerprint();
    assert_eq!(migration_before.1, 8);
    let current = (
        config.durable_profile_fingerprint(),
        config.foreman_profile_fingerprint(),
    );
    run_latticed_admin(&config, "--postgres-bootstrap", true);
    assert_eq!(
        (
            config.durable_profile_fingerprint(),
            config.foreman_profile_fingerprint(),
        ),
        current
    );
    assert_eq!(config.migration_fingerprint(), migration_before);
    println!("TASK105_STAGE_V8_CURRENT_NOOP_PASS");

    config.introduce_partial_writer_acl();
    let partial = config.durable_profile_fingerprint();
    assert_eq!(
        run_latticed_admin(&config, "--postgres-bootstrap", false).trim(),
        "LATTICE_WRITER_LEASE_REJECTED"
    );
    assert_eq!(config.durable_profile_fingerprint(), partial);
    config.repair_partial_writer_acl();
    config.assert_v8_writer_v5_successor();
    println!("TASK105_STAGE_PARTIAL_FAIL_CLOSED_PASS");

    config.introduce_corrupt_writer_identity();
    let corrupt = config.durable_profile_fingerprint();
    assert_eq!(
        run_latticed_admin(&config, "--postgres-bootstrap", false).trim(),
        "LATTICE_WRITER_LEASE_REJECTED"
    );
    assert_eq!(config.durable_profile_fingerprint(), corrupt);
    config.repair_corrupt_writer_identity();
    config.assert_v8_writer_v5_successor();
    println!("TASK105_STAGE_CORRUPT_FAIL_CLOSED_PASS");

    let mut unsupported_history = UnsupportedHistory::introduce(&config, true);
    let unsupported = config.durable_profile_fingerprint();
    assert_eq!(
        run_latticed_admin(&config, "--postgres-bootstrap", false).trim(),
        "LATTICED_RUNTIME_POSTGRES_VERIFICATION_REJECTED"
    );
    assert_eq!(config.durable_profile_fingerprint(), unsupported);
    unsupported_history
        .restore()
        .expect("TASK105_BOOTSTRAP_UNSUPPORTED_RESTORE");
    config.assert_v8_writer_v5_successor();
    println!("TASK105_STAGE_UNSUPPORTED_FAIL_CLOSED_PASS");
    println!("TASK105_STAGE_INITIALIZE_PASS");

    let process_a = run_latticed(&[
        initialize_request(),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        checkpoint(2, "task105-checkpoint-1", 1, "ACTIVE", Value::Null, 'b'),
        checkpoint(3, "task105-checkpoint-1", 1, "ACTIVE", Value::Null, 'b'),
        checkpoint(4, "task105-checkpoint-1", 1, "ACTIVE", Value::Null, 'c'),
        checkpoint(5, "task105-checkpoint-gap", 3, "ACTIVE", Value::Null, 'd'),
        checkpoint(
            6,
            "task105-checkpoint-2",
            2,
            "BLOCKED",
            json!("TASK-094"),
            'e',
        ),
        json!({"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"lattice_runtime_status"}}),
    ]);
    let first = &response(&process_a, 2)["result"]["structuredContent"];
    assert_eq!(response(&process_a, 2)["result"]["isError"], false);
    assert_eq!(first["status"], "RECORDED");
    assert_eq!(
        response(&process_a, 3)["result"]["structuredContent"]["status"],
        "REPLAYED"
    );
    assert_eq!(
        response(&process_a, 3)["result"]["structuredContent"]["ledger_digest"],
        first["ledger_digest"]
    );
    assert_eq!(
        response(&process_a, 4)["result"]["structuredContent"]["code"],
        "FOREMAN_CHECKPOINT_ID_REUSE"
    );
    assert_eq!(
        response(&process_a, 5)["result"]["structuredContent"]["code"],
        "FOREMAN_GENERATION_INVALID"
    );
    assert_eq!(response(&process_a, 6)["result"]["isError"], false);
    let status_a = &response(&process_a, 7)["result"]["structuredContent"]["foreman"];
    let second = &response(&process_a, 6)["result"]["structuredContent"];
    assert_eq!(status_a["ledger_digest"], second["ledger_digest"]);
    assert_eq!(status_a["checkpoint_digest"], second["checkpoint_digest"]);
    assert_eq!(status_a["latest_generation"], 2);
    assert_eq!(status_a["active_count"], 0);
    assert_eq!(status_a["blocked_count"], 1);
    assert_eq!(status_a["completed_count"], 0);
    assert_eq!(status_a["next_action"], "RESOLVE_BLOCKERS");
    assert_eq!(status_a["degraded_code"], Value::Null);
    println!("TASK105_STAGE_PROCESS_A_PASS");

    let process_b = run_latticed(&[
        initialize_request(),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        runtime_status_request(2),
    ]);
    let baseline_status = response(&process_b, 2)["result"]["structuredContent"].clone();
    let status_b = &baseline_status["foreman"];
    assert_eq!(status_b, status_a);
    assert_eq!(config.migration_fingerprint(), migration_before);
    println!("TASK105_STAGE_FRESH_PROCESS_REPLAY_PASS");

    let mut interactive = InteractiveLatticed::start_initialized();
    assert_eq!(
        interactive.request_status(2)["result"]["structuredContent"],
        baseline_status
    );
    {
        let mut corruption = ForemanWorkerCorruption::introduce(&config);
        assert_foreman_replay_error(&interactive.request_status(3), "FOREMAN_REPLAY_CORRUPT");
        corruption
            .restore()
            .expect("TASK105_FOREMAN_CORRUPT_RESTORE");
    }
    assert_eq!(
        interactive.request_status(4)["result"]["structuredContent"],
        baseline_status
    );
    println!("TASK105_STAGE_FOREMAN_REPLAY_CORRUPT_PASS");

    {
        let mut unavailable = RuntimeLoginDisabled::introduce(&config);
        assert_foreman_replay_error(&interactive.request_status(5), "FOREMAN_REPLAY_UNAVAILABLE");
        unavailable
            .restore()
            .expect("TASK105_RUNTIME_LOGIN_RESTORE");
    }
    assert_eq!(
        interactive.request_status(6)["result"]["structuredContent"],
        baseline_status
    );
    println!("TASK105_STAGE_FOREMAN_REPLAY_UNAVAILABLE_PASS");

    let mut contention = WriterContention::acquire(&config);
    let mut contention_status = baseline_status.clone();
    contention_status["foreman"]["degraded_code"] = json!("FOREMAN_WRITER_CONTENTION");
    assert_eq!(
        interactive.request_status(7)["result"]["structuredContent"],
        contention_status
    );
    {
        let mut corruption = ForemanWorkerCorruption::introduce(&config);
        assert_foreman_replay_error(&interactive.request_status(8), "FOREMAN_REPLAY_CORRUPT");
        corruption
            .restore()
            .expect("TASK105_CONTENTION_CORRUPT_RESTORE");
    }
    assert_eq!(
        interactive.request_status(9)["result"]["structuredContent"],
        contention_status
    );
    contention
        .release()
        .expect("TASK105_WRITER_CONTENTION_RELEASE");
    assert_eq!(
        interactive.request_status(10)["result"]["structuredContent"],
        baseline_status
    );
    println!("TASK105_STAGE_FOREMAN_WRITER_CONTENTION_PASS");

    drop(
        PostgresTaskLedger::new(config.runtime_client(), &config.migration_target())
            .expect("TASK105_HEALTHY_LEDGER_BASELINE"),
    );

    let migration_before_unsupported = config.migration_fingerprint();
    let profile_before_unsupported = config.durable_profile_fingerprint();
    {
        let mut inconsistent = UnsupportedHistory::introduce(&config, false);
        assert_eq!(
            PostgresTaskLedger::new(config.runtime_client(), &config.migration_target())
                .err()
                .expect("TASK105_INCONSISTENT_HISTORY_REJECTED")
                .kind(),
            PostgresTaskLedgerErrorKind::RetainedRowCorrupt
        );
        assert_foreman_replay_error(&interactive.request_status(11), "FOREMAN_REPLAY_CORRUPT");
        inconsistent
            .restore()
            .expect("TASK105_INCONSISTENT_HISTORY_RESTORE");
    }
    assert_eq!(
        interactive.request_status(12)["result"]["structuredContent"],
        baseline_status
    );
    println!("TASK105_STAGE_FOREMAN_REPLAY_EXTRA_HISTORY_CORRUPT_PASS");

    {
        let mut unsupported = UnsupportedHistory::introduce(&config, true);
        let injected_migration = config.migration_fingerprint();
        let injected_profile = config.durable_profile_fingerprint();
        config.assert_coherent_future_atomic_snapshot();
        let profile_kind =
            inspect_migration_profile(&mut config.migrator_client(), &config.migration_target())
                .expect_err("TASK105_FUTURE_PROFILE_REJECTED")
                .kind();
        let verify_kind = verify_postgres_schema(
            &mut config.runtime_client(),
            &config.migration_target(),
            DatabaseRole::Runtime,
        )
        .expect_err("TASK105_FUTURE_VERIFY_REJECTED")
        .kind();
        let ledger_kind =
            PostgresTaskLedger::new(config.runtime_client(), &config.migration_target())
                .err()
                .expect("TASK105_FUTURE_LEDGER_REJECTED")
                .kind();
        assert_eq!(
            ledger_kind,
            PostgresTaskLedgerErrorKind::UnsupportedRetainedSchema
        );
        assert_eq!(
            profile_kind,
            lattice_postgres_store::PostgresStoreSetupErrorKind::UnsupportedFutureSchema
        );
        assert_eq!(
            verify_kind,
            lattice_postgres_store::PostgresStoreSetupErrorKind::UnsupportedFutureSchema
        );
        assert_foreman_replay_error(
            &interactive.request_status(13),
            "FOREMAN_REPLAY_UNSUPPORTED",
        );
        assert_eq!(config.migration_fingerprint(), injected_migration);
        assert_eq!(config.durable_profile_fingerprint(), injected_profile);
        unsupported
            .restore()
            .expect("TASK105_UNSUPPORTED_HISTORY_RESTORE");
    }
    assert_eq!(config.migration_fingerprint(), migration_before_unsupported);
    assert_eq!(
        config.durable_profile_fingerprint(),
        profile_before_unsupported
    );
    assert_eq!(
        interactive.request_status(14)["result"]["structuredContent"],
        baseline_status
    );
    interactive.finish();
    println!("TASK105_STAGE_FOREMAN_REPLAY_UNSUPPORTED_PASS");

    config.remove_disposable_writer_profile();
    let absent_before = config.v8_absent_writer_fingerprint();
    assert_eq!(config.migration_fingerprint().1, 8);
    config.assert_writer_namespace_absent();
    assert_eq!(
        run_latticed_admin(&config, "--postgres-bootstrap", false)
            .replace("\r\n", "\n")
            .trim(),
        "LATTICE_WRITER_LEASE_REJECTED"
    );
    assert_eq!(config.v8_absent_writer_fingerprint(), absent_before);
    println!("TASK105_STAGE_V8_ABSENT_FAIL_CLOSED_PASS");

    let parent_run_id = required("LATTICE_TASK019_RUN_ID");
    let main_during_children = config.v8_absent_writer_fingerprint();

    let fresh_partial = config.child_database(0x4000_0000);
    assert_ne!(fresh_partial.run_id, config.run_id);
    config.revoke_login_database_privileges();
    config.assert_login_capability(false);
    run_latticed_admin(&fresh_partial, "--postgres-initialize", true);
    fresh_partial.assert_login_capability(true);
    config.assert_login_capability(false);
    fresh_partial.introduce_partial_writer_on_fresh_store();
    fresh_partial.assert_store_migration_profile(MigrationBootstrapProfile::Fresh);
    let fresh_partial_before = fresh_partial.fresh_catalog_fingerprint();
    assert_eq!(
        run_latticed_admin(&fresh_partial, "--postgres-bootstrap", false).trim(),
        "LATTICE_WRITER_LEASE_REJECTED"
    );
    assert_eq!(
        fresh_partial.fresh_catalog_fingerprint(),
        fresh_partial_before
    );
    fresh_partial.assert_store_migration_profile(MigrationBootstrapProfile::Fresh);
    assert_eq!(config.v8_absent_writer_fingerprint(), main_during_children);
    println!("TASK105_STAGE_FRESH_PARTIAL_WRITER_FAIL_CLOSED_PASS");

    let writer_v2 = config.child_database(0x1000_0000);
    assert_ne!(writer_v2.run_id, config.run_id);
    assert_ne!(writer_v2.run_id, fresh_partial.run_id);
    fresh_partial.revoke_login_database_privileges();
    fresh_partial.assert_login_capability(false);
    run_latticed_admin(&writer_v2, "--postgres-initialize", true);
    writer_v2.assert_login_capability(true);
    config.assert_login_capability(false);
    writer_v2.prepare_v5_writer_v2_current();
    writer_v2.assert_v5_writer_v2_current();
    run_latticed_admin(&writer_v2, "--postgres-bootstrap", true);
    writer_v2.assert_v8_writer_v5_successor();
    assert_eq!(config.v8_absent_writer_fingerprint(), main_during_children);
    println!("TASK105_STAGE_V5_WRITER_V2_EXECUTABLE_PASS");

    let bridge_pending = config.child_database(0x3000_0000);
    assert_ne!(bridge_pending.run_id, config.run_id);
    assert_ne!(bridge_pending.run_id, writer_v2.run_id);
    assert_ne!(bridge_pending.run_id, fresh_partial.run_id);
    writer_v2.revoke_login_database_privileges();
    writer_v2.assert_login_capability(false);
    run_latticed_admin(&bridge_pending, "--postgres-initialize", true);
    bridge_pending.assert_login_capability(true);
    writer_v2.assert_login_capability(false);
    config.assert_login_capability(false);
    bridge_pending.prepare_v5_memory_v2_writer_v2_bridge_pending();
    bridge_pending.assert_v5_memory_v2_writer_v2_bridge_pending();
    run_latticed_admin(&bridge_pending, "--postgres-bootstrap", true);
    bridge_pending.assert_v8_writer_v5_successor();
    assert_eq!(config.v8_absent_writer_fingerprint(), main_during_children);
    println!("TASK105_STAGE_V5_MEMORY_V2_WRITER_PENDING_EXECUTABLE_PASS");

    let legacy_v1 = config.child_database(0x5000_0000);
    assert_ne!(legacy_v1.run_id, config.run_id);
    assert_ne!(legacy_v1.run_id, fresh_partial.run_id);
    assert_ne!(legacy_v1.run_id, writer_v2.run_id);
    assert_ne!(legacy_v1.run_id, bridge_pending.run_id);
    bridge_pending.revoke_login_database_privileges();
    bridge_pending.assert_login_capability(false);
    run_latticed_admin(&legacy_v1, "--postgres-initialize", true);
    legacy_v1.assert_login_capability(true);
    config.assert_login_capability(false);
    legacy_v1.prepare_legacy_v1_store();
    let legacy_before = legacy_v1.legacy_v1_fingerprint();
    assert_eq!(
        run_latticed_admin(&legacy_v1, "--postgres-bootstrap", false).trim(),
        "LATTICED_RUNTIME_POSTGRES_VERIFICATION_REJECTED"
    );
    assert_eq!(legacy_v1.legacy_v1_fingerprint(), legacy_before);
    assert_eq!(config.v8_absent_writer_fingerprint(), main_during_children);
    println!("TASK105_STAGE_LEGACY_PRODUCT_BOOTSTRAP_REJECTED_PASS");

    let writer_absent = config.child_database(0x2000_0000);
    assert_ne!(writer_absent.run_id, config.run_id);
    assert_ne!(writer_absent.run_id, writer_v2.run_id);
    assert_ne!(writer_absent.run_id, bridge_pending.run_id);
    assert_ne!(writer_absent.run_id, fresh_partial.run_id);
    assert_ne!(writer_absent.run_id, legacy_v1.run_id);
    legacy_v1.revoke_login_database_privileges();
    legacy_v1.assert_login_capability(false);
    run_latticed_admin(&writer_absent, "--postgres-initialize", true);
    writer_absent.assert_login_capability(true);
    writer_v2.assert_login_capability(false);
    bridge_pending.assert_login_capability(false);
    fresh_partial.assert_login_capability(false);
    legacy_v1.assert_login_capability(false);
    config.assert_login_capability(false);
    writer_absent.prepare_v5_store_only();
    writer_absent.assert_v5_writer_absent();

    writer_absent.introduce_global_default_acl_drift();
    let global_default_acl_drift = writer_absent.v5_fallback_fingerprint();
    assert_eq!(
        run_latticed_admin(&writer_absent, "--postgres-bootstrap", false).trim(),
        "LATTICE_GRAPH_MEMORY_CONFIGURATION_REJECTED"
    );
    assert_eq!(
        writer_absent.v5_fallback_fingerprint(),
        global_default_acl_drift
    );
    writer_absent.repair_global_default_acl_drift();
    writer_absent.assert_v5_writer_absent();
    println!("TASK105_STAGE_V5_GLOBAL_DEFAULT_ACL_FAIL_CLOSED_PASS");

    writer_absent.introduce_memory_default_acl_drift();
    let default_acl_drift = writer_absent.v5_fallback_fingerprint();
    assert_eq!(
        run_latticed_admin(&writer_absent, "--postgres-bootstrap", false).trim(),
        "LATTICE_GRAPH_MEMORY_CONFIGURATION_REJECTED"
    );
    assert_eq!(writer_absent.v5_fallback_fingerprint(), default_acl_drift);
    writer_absent.repair_memory_default_acl_drift();
    writer_absent.assert_v5_writer_absent();
    println!("TASK105_STAGE_V5_MEMORY_DEFAULT_ACL_FAIL_CLOSED_PASS");

    writer_absent.introduce_partial_memory_catalog();
    let partial_memory = writer_absent.v5_fallback_fingerprint();
    assert_eq!(
        run_latticed_admin(&writer_absent, "--postgres-bootstrap", false).trim(),
        "LATTICE_GRAPH_MEMORY_CONFIGURATION_REJECTED"
    );
    assert_eq!(writer_absent.v5_fallback_fingerprint(), partial_memory);
    writer_absent.repair_partial_memory_catalog();
    writer_absent.assert_v5_writer_absent();
    println!("TASK105_STAGE_V5_MEMORY_PARTIAL_FAIL_CLOSED_PASS");

    run_latticed_admin(&writer_absent, "--postgres-bootstrap", true);
    writer_absent.assert_v8_writer_v5_successor();
    assert_eq!(config.v8_absent_writer_fingerprint(), main_during_children);

    writer_absent.revoke_login_database_privileges();
    writer_absent.assert_login_capability(false);
    run_latticed_admin(&config, "--postgres-initialize", true);
    config.assert_login_capability(true);
    writer_v2.assert_login_capability(false);
    bridge_pending.assert_login_capability(false);
    fresh_partial.assert_login_capability(false);
    legacy_v1.assert_login_capability(false);
    writer_absent.assert_login_capability(false);
    assert_eq!(config.v8_absent_writer_fingerprint(), main_during_children);
    assert_eq!(required("LATTICE_TASK019_RUN_ID"), parent_run_id);
    println!("TASK105_STAGE_V5_WRITER_ABSENT_EXECUTABLE_PASS");

    let main_before_race = config.v8_absent_writer_fingerprint();
    let mut race_database = DisposableRaceDatabase::new(&config);
    race_database.initialize();
    let race = race_database.config();
    run_latticed_admin(&race, "--postgres-bootstrap", true);
    race.assert_v8_writer_v5_successor();
    let race_migration = race.migration_fingerprint();
    let race_durable = race.durable_profile_fingerprint();

    let mut process_a = InteractiveLatticed::start_for(&race);
    process_a.initialize();
    let mut process_b = InteractiveLatticed::start_for(&race);
    process_b.initialize();
    let mut stream_lock = ForemanStreamLock::acquire(&race);
    let generation_one = checkpoint(
        2,
        "task105-race-checkpoint-1",
        1,
        "ACTIVE",
        Value::Null,
        'b',
    );
    process_a.send(&generation_one);
    stream_lock
        .wait_for_one_ungranted_waiter_for(Duration::from_secs(35))
        .expect("TASK105_FOREMAN_STREAM_WAITER");
    assert_eq!(
        race.foreman_counts(),
        ([0, 0, 0], ["0".into(), "0".into(), "0".into()], None)
    );
    let coordination_identity = foreman_coordination_identity().expect("TASK105_FOREMAN_IDENTITY");
    let mut writer_observer = foreman_writer_repository(&race);
    let current = writer_observer
        .current_authority(coordination_identity.project_id())
        .expect("TASK105_RACE_WRITER_CURRENT")
        .expect("TASK105_RACE_WRITER_AUTHORITY_MISSING");
    let current_head = current.independent_head();
    let acquire_command_id = foreman_acquire_command_id("task105-race-checkpoint-1");
    let suffix = acquire_command_id
        .strip_prefix("foreman-acquire-")
        .expect("TASK105_RACE_WRITER_SUFFIX");
    assert_eq!(
        current_head.identity().project_id(),
        coordination_identity.project_id()
    );
    assert_eq!(
        current_head.identity().task_id(),
        coordination_identity.task_id()
    );
    assert_eq!(
        current_head.identity().attempt_id().as_str(),
        format!("foreman-attempt-{suffix}")
    );
    assert_eq!(
        current_head.identity().lease_id(),
        format!("foreman-lease-{suffix}")
    );
    assert_eq!(
        current_head.identity().lease_holder_id(),
        "latticed-foreman-v1"
    );
    assert_eq!(
        current_head.identity().worktree_id(),
        format!("foreman-worktree-{suffix}")
    );
    assert_eq!(
        current_head.identity().holder_process_id().get(),
        u64::from(process_a.pid())
    );
    let mut authority_cleanup = RaceAuthorityCleanup::new(
        writer_observer,
        coordination_identity.project_id().clone(),
        current_head.clone(),
    );
    let same_generation_one = checkpoint(
        3,
        "task105-race-checkpoint-1",
        1,
        "ACTIVE",
        Value::Null,
        'b',
    );
    assert_eq!(same_generation_one["params"], generation_one["params"]);
    process_b.send(&same_generation_one);
    assert_foreman_replay_error(
        &process_b.recv_expected(3, Duration::from_secs(35)),
        "FOREMAN_REPLAY_UNAVAILABLE",
    );
    let contender_checkpoint_id = "task105-race-contender";
    let contender = checkpoint(4, contender_checkpoint_id, 1, "ACTIVE", Value::Null, 'c');
    process_b.send(&contender);
    assert_foreman_replay_error(
        &process_b.recv_expected(4, Duration::from_secs(35)),
        "FOREMAN_REPLAY_UNAVAILABLE",
    );
    race.assert_writer_command_absent(contender_checkpoint_id);
    assert_eq!(
        race.foreman_counts(),
        ([0, 0, 0], ["0".into(), "0".into(), "0".into()], None)
    );
    stream_lock
        .release()
        .expect("TASK105_FOREMAN_STREAM_UNLOCK");
    drop(stream_lock);
    let recorded = process_a.recv_expected(2, Duration::from_secs(35));
    assert_eq!(recorded["result"]["isError"], false);
    let first = recorded["result"]["structuredContent"].clone();
    assert_eq!(first["status"], "RECORDED");
    assert_eq!(first["exact_retry"], false);
    authority_cleanup
        .disarm_after_release()
        .expect("TASK105_RACE_AUTHORITY_RELEASED");
    drop(authority_cleanup);

    let replayed = process_b.request(&checkpoint(
        5,
        "task105-race-checkpoint-1",
        1,
        "ACTIVE",
        Value::Null,
        'b',
    ));
    assert_eq!(replayed["result"]["isError"], false);
    let replayed = &replayed["result"]["structuredContent"];
    assert_eq!(replayed["status"], "REPLAYED");
    assert_eq!(replayed["exact_retry"], true);
    assert_eq!(replayed["ledger_digest"], first["ledger_digest"]);
    assert_eq!(replayed["checkpoint_digest"], first["checkpoint_digest"]);
    let generation_one_head = Some(
        first["ledger_digest"]
            .as_str()
            .expect("TASK105_RACE_GENERATION_ONE_LEDGER_DIGEST")
            .to_owned(),
    );
    assert_eq!(
        race.foreman_counts(),
        (
            [1, 1, 1],
            ["1".into(), "1".into(), "1".into()],
            generation_one_head.clone()
        )
    );
    process_a.finish();
    process_b.finish();

    let mut process_c = InteractiveLatticed::start_for(&race);
    process_c.initialize();
    let fresh_status = process_c.request_status(2);
    let fresh_foreman = &fresh_status["result"]["structuredContent"]["foreman"];
    assert_eq!(fresh_status["result"]["isError"], false);
    assert_eq!(fresh_foreman["latest_generation"], 1);
    assert_eq!(fresh_foreman["ledger_digest"], first["ledger_digest"]);
    assert_eq!(
        fresh_foreman["checkpoint_digest"],
        first["checkpoint_digest"]
    );
    assert_eq!(fresh_foreman["active_count"], 1);
    assert_eq!(fresh_foreman["blocked_count"], 0);
    assert_eq!(fresh_foreman["next_action"], "CONTINUE");

    let fresh_retry = process_c.request(&checkpoint(
        3,
        "task105-race-checkpoint-1",
        1,
        "ACTIVE",
        Value::Null,
        'b',
    ));
    assert_eq!(fresh_retry["result"]["isError"], false);
    assert_eq!(
        fresh_retry["result"]["structuredContent"]["status"],
        "REPLAYED"
    );
    assert_eq!(
        fresh_retry["result"]["structuredContent"]["ledger_digest"],
        first["ledger_digest"]
    );
    assert_eq!(
        fresh_retry["result"]["structuredContent"]["checkpoint_digest"],
        first["checkpoint_digest"]
    );
    let generation_two = process_c.request(&checkpoint(
        4,
        "task105-race-checkpoint-2",
        2,
        "BLOCKED",
        json!("TASK-094"),
        'c',
    ));
    assert_eq!(generation_two["result"]["isError"], false);
    assert_eq!(
        generation_two["result"]["structuredContent"]["status"],
        "RECORDED"
    );
    let generation_two_head = Some(
        generation_two["result"]["structuredContent"]["ledger_digest"]
            .as_str()
            .expect("TASK105_RACE_GENERATION_TWO_LEDGER_DIGEST")
            .to_owned(),
    );
    assert_eq!(
        race.foreman_counts(),
        (
            [2, 2, 2],
            ["2".into(), "2".into(), "2".into()],
            generation_two_head.clone()
        )
    );

    assert_foreman_replay_error(
        &process_c.request(&checkpoint(
            5,
            "task105-race-checkpoint-1",
            1,
            "ACTIVE",
            Value::Null,
            'd',
        )),
        "FOREMAN_CHECKPOINT_ID_REUSE",
    );
    assert_eq!(
        race.foreman_counts(),
        (
            [2, 2, 2],
            ["2".into(), "2".into(), "2".into()],
            generation_two_head
        )
    );
    process_c.finish();
    let dependency_fixture = DependencyGitFixture::new();
    let mut process_d = InteractiveLatticed::start_for_dependency(&race, &dependency_fixture);
    process_d.initialize();
    let generation_three = process_d.request(&checkpoint(
        2,
        "task106-parent-active",
        3,
        "ACTIVE",
        Value::Null,
        'd',
    ));
    assert_eq!(generation_three["result"]["isError"], false);
    let generation_four = process_d.request(&checkpoint(
        3,
        "task106-dependency-blocked",
        4,
        "BLOCKED",
        dependency_fixture.blocker(),
        'e',
    ));
    assert_eq!(generation_four["result"]["isError"], false);
    assert_eq!(
        generation_four["result"]["structuredContent"]["status"],
        "RECORDED"
    );
    process_d.finish();

    let mut process_e = InteractiveLatticed::start_for_dependency(&race, &dependency_fixture);
    process_e.initialize();
    let blocked_status = process_e.request_status(2);
    let blocked_dependency =
        &blocked_status["result"]["structuredContent"]["foreman"]["dependency"];
    assert_eq!(blocked_status["result"]["isError"], false);
    assert_eq!(
        blocked_status["result"]["structuredContent"]["foreman"]["schema"],
        "lattice.foreman-runtime-projection/1.1"
    );
    assert_eq!(blocked_dependency["parent_task_id"], "TASK-106");
    assert_eq!(blocked_dependency["dependency_task_id"], "TASK-107");
    assert_eq!(blocked_dependency["depends_on"], "TASK-107");
    assert_eq!(blocked_dependency["state"], "BLOCKED");
    assert_eq!(blocked_dependency["base_sha"], dependency_fixture.base_sha);
    assert_eq!(blocked_dependency["next_action"], "COMPLETE_DEPENDENCY");
    assert_eq!(blocked_dependency["verification_status"], "VERIFIED");

    assert_foreman_replay_error(
        &process_e.request(&checkpoint(
            21,
            "task106-parent-completed-without-resume",
            5,
            "COMPLETED",
            Value::Null,
            'f',
        )),
        "FOREMAN_DEPENDENCY_RECONCILIATION_REQUIRED",
    );

    let retry_drift = dependency_fixture.child.join("exact-retry-dirty.txt");
    fs::write(&retry_drift, b"must not be probed\n").expect("TASK106_RETRY_DIRTY");
    let exact_retry = process_e.request(&checkpoint(
        20,
        "task106-dependency-blocked",
        4,
        "BLOCKED",
        dependency_fixture.blocker(),
        'e',
    ));
    assert_eq!(exact_retry["result"]["isError"], false);
    assert_eq!(
        exact_retry["result"]["structuredContent"]["status"],
        "REPLAYED"
    );
    assert_eq!(
        exact_retry["result"]["structuredContent"]["exact_retry"],
        true
    );
    fs::remove_file(&retry_drift).expect("TASK106_RETRY_DIRTY_REMOVE");

    dependency_fixture.commit_dependency();
    let in_progress_status = process_e.request_status(3);
    assert_eq!(
        in_progress_status["result"]["structuredContent"]["foreman"]["dependency"]["verification_status"],
        "VERIFIED"
    );
    let resume_request = checkpoint(4, "task106-parent-resumed", 5, "ACTIVE", Value::Null, 'f');
    assert_foreman_replay_error(
        &process_e.request(&resume_request),
        "FOREMAN_DEPENDENCY_NOT_INTEGRATED",
    );
    dependency_fixture.merge_dependency();
    let resumed = process_e.request(&resume_request);
    assert_eq!(resumed["result"]["isError"], false);
    assert_eq!(resumed["result"]["structuredContent"]["status"], "RECORDED");
    process_e.finish();

    let mut process_f = InteractiveLatticed::start_for_dependency(&race, &dependency_fixture);
    process_f.initialize();
    let resumed_status = process_f.request_status(2);
    let resumed_foreman = &resumed_status["result"]["structuredContent"]["foreman"];
    assert_eq!(resumed_status["result"]["isError"], false);
    assert_eq!(resumed_foreman["latest_generation"], 5);
    assert_eq!(resumed_foreman["active_count"], 1);
    assert_eq!(resumed_foreman["blocked_count"], 0);
    assert_eq!(resumed_foreman["next_action"], "CONTINUE");
    assert_eq!(resumed_foreman["dependency"]["state"], "RESUMED");
    assert_eq!(resumed_foreman["dependency"]["depends_on"], "TASK-107");
    assert_eq!(
        resumed_foreman["dependency"]["next_action"],
        "CONTINUE_PARENT"
    );
    assert_eq!(
        resumed_foreman["dependency"]["verification_status"],
        "VERIFIED"
    );
    assert_eq!(
        resumed_foreman["dependency"]["dependency_branch"],
        "lattice/task-107"
    );
    assert_eq!(
        resumed_foreman["dependency"]["dependency_worktree_id"],
        "TASK-107-WORKTREE"
    );
    process_f.finish();
    dependency_fixture.cleanup();
    println!("TASK106_STAGE_DEPENDENCY_FRESH_PROCESS_REPLAY_PASS");
    assert_eq!(race.migration_fingerprint(), race_migration);
    assert_eq!(race.durable_profile_fingerprint(), race_durable);

    race_database
        .cleanup()
        .expect("TASK105_RACE_DATABASE_CLEANUP");
    assert!(race.try_bootstrap_client().is_err());
    config.assert_login_capability(true);
    assert_eq!(config.v8_absent_writer_fingerprint(), main_before_race);
    assert_eq!(required("LATTICE_TASK019_RUN_ID"), parent_run_id);
    println!("TASK105_STAGE_FOREMAN_DUAL_PROCESS_RACE_PASS");
}
