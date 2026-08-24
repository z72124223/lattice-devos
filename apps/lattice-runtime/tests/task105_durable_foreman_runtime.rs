//! Marker-owned PostgreSQL acceptance for TASK-105.

use std::env;
use std::io::Write;
use std::process::{Command, Stdio};

use lattice_contracts::ContentDigest;
use lattice_postgres_codebase_memory::{
    ExtensionTarget as MemoryExtensionTarget, apply_extension as apply_memory_extension,
    verify_embedded_extension_manifest as verify_memory_manifest,
    verify_embedded_v2_extension_manifest as verify_memory_v2_manifest,
};
use lattice_postgres_store::{
    DatabaseRole, MigrationApplyOutcome, MigrationBootstrapProfile, MigrationTarget,
    apply_migrations, inspect_migration_profile, migration_manifest,
};
use lattice_postgres_writer_lease::{
    ExtensionApplyOutcome, ExtensionSetupErrorKind, ExtensionTarget as WriterExtensionTarget,
    V3BootstrapProfile, V3ExtensionTarget, apply_extension as apply_writer_extension,
    apply_v3_extension, inspect_v3_bootstrap_profile,
    verify_embedded_v1_extension_manifest as verify_writer_v1_manifest,
    verify_embedded_v2_extension_manifest as verify_writer_v2_manifest,
    verify_embedded_v3_rebind_manifest,
};
use postgres::config::SslMode;
use postgres::{Client, Config, NoTls};
use serde_json::{Value, json};

const V5_MANIFEST_SHA256: &str = "f92a51fa19c4fe0ffebfc40f20924bd1209bb2441b1bc69f787bc3c4a925425d";
const HISTORICAL_GLOBAL_MANIFEST_SHA256: &str =
    "09c431df18ad71a4f44239a5d2ddf6b1774b8ffec06c7f9223f0e41757f3d407";
const LEGACY_V1_MANIFEST_SHA256: &str =
    "9b126a41e542b71d434b5786e35acb66575967d055a6733b9d6bf0b8c9f0eada";
const WRITER_V3_MANIFEST_SHA256: &str =
    "eab2812fa3d94cd3466d7c003386f805a973fd7def1f16aeb15b52f47dad78e4";
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
        let mut config = Config::new();
        config
            .host(&self.host)
            .port(self.port)
            .user("runtime_bootstrap")
            .password(&self.password)
            .dbname(&self.database_name())
            .application_name("lattice-task105-migration-observer")
            .ssl_mode(SslMode::Disable);
        config.connect(NoTls).expect("TASK105_BOOTSTRAP_CONNECT")
    }

    fn revoke_login_database_privileges(&self) {
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
            .expect("TASK105_CAPABILITY_HANDOFF_CONNECT")
            .batch_execute(&format!(
                "REVOKE ALL ON DATABASE {} FROM \
                 lattice_migrator_login,lattice_runtime_login,\
                 lattice_guardian_login,lattice_readonly_login",
                self.database_name()
            ))
            .expect("TASK105_CAPABILITY_HANDOFF_REVOKE");
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
        assert_eq!(manifest.len(), 7, "TASK105_FIXTURE_MANIFEST_SIZE");
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
                    (SELECT extension_schema_version=3 AND global_schema_version=6 \
                      FROM ONLY writer_lease.writer_lease_extension_identity WHERE singleton), \
                    pg_catalog.has_schema_privilege('lattice_runtime','writer_lease','USAGE'), \
                    pg_catalog.has_function_privilege('lattice_runtime', \
                      'writer_lease.writer_lease_bind_runtime_v3(text,bigint,bytea,text,text,text,text,text)', \
                      'EXECUTE')",
                &[],
            )
            .expect("TASK105_V6_WRITER_V3_PROFILE");
        for index in 0..5 {
            assert!(row.get::<_, bool>(index), "TASK105_V6_WRITER_V3_{index}");
        }
    }

    fn make_v6_writer_bridge_pending(&self) {
        self.bootstrap_client()
            .batch_execute(
                "DELETE FROM ONLY writer_lease.writer_lease_extension_ledger \
                    WHERE ledger_ordinal=3 AND event_kind='REBOUND'; \
                 REVOKE ALL ON ALL FUNCTIONS IN SCHEMA writer_lease FROM lattice_runtime; \
                 REVOKE USAGE ON SCHEMA writer_lease FROM lattice_runtime",
            )
            .expect("TASK105_MAKE_V6_WRITER_BRIDGE_PENDING");
    }

    fn assert_v6_writer_bridge_pending(&self) {
        let mut client = self.bootstrap_client();
        let row = client
            .query_one(
                "SELECT \
                    (SELECT extension_schema_version=3 AND global_schema_version=6 \
                       AND global_manifest_sha256=(SELECT manifest_sha256 \
                         FROM ONLY control.schema_compatibility WHERE singleton) \
                       FROM ONLY writer_lease.writer_lease_extension_identity WHERE singleton), \
                    (SELECT pg_catalog.string_agg(ledger_ordinal::text || ':' || event_kind::text, \
                         ',' ORDER BY ledger_ordinal)='1:INSTALLED,2:UPGRADED' \
                       FROM ONLY writer_lease.writer_lease_extension_ledger), \
                    NOT pg_catalog.has_schema_privilege('lattice_runtime','writer_lease','USAGE'), \
                    (SELECT pg_catalog.count(*) FILTER (WHERE \
                         pg_catalog.has_function_privilege('lattice_runtime',p.oid,'EXECUTE'))=0 \
                       FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n \
                         ON n.oid=p.pronamespace WHERE n.nspname='writer_lease')",
                &[],
            )
            .expect("TASK105_V6_WRITER_BRIDGE_PENDING_PROFILE");
        for index in 0..4 {
            assert!(
                row.get::<_, bool>(index),
                "TASK105_V6_WRITER_PENDING_{index}"
            );
        }
    }

    fn introduce_partial_writer_acl(&self) {
        self.bootstrap_client()
            .batch_execute(
                "REVOKE EXECUTE ON FUNCTION writer_lease.writer_lease_bind_runtime_v3(\
                    text,bigint,bytea,text,text,text,text,text) FROM lattice_runtime",
            )
            .expect("TASK105_INTRODUCE_PARTIAL_WRITER");
    }

    fn repair_partial_writer_acl(&self) {
        self.bootstrap_client()
            .batch_execute(
                "GRANT EXECUTE ON FUNCTION writer_lease.writer_lease_bind_runtime_v3(\
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
                 SET extension_manifest_sha256='{WRITER_V3_MANIFEST_SHA256}' WHERE singleton"
            ))
            .expect("TASK105_REPAIR_CORRUPT_WRITER");
    }

    fn introduce_unsupported_history(&self) {
        self.bootstrap_client()
            .batch_execute(
                "INSERT INTO control.migration_history (ordinal,migration_id,migration_path,\
                    byte_length,checksum_sha256,migration_status,transaction_mode,schema_version,\
                    min_reader,max_reader,min_writer,max_writer) VALUES (8,'0008_unsupported_fixture',\
                    'db/migrations/0008_unsupported_fixture.sql',1,repeat('d',64),'EXECUTABLE',\
                    'RUNNER_OWNED',7,7,7,7,7)",
            )
            .expect("TASK105_INTRODUCE_UNSUPPORTED_HISTORY");
    }

    fn repair_unsupported_history(&self) {
        self.bootstrap_client()
            .batch_execute(
                "DELETE FROM ONLY control.migration_history \
                 WHERE ordinal=8 AND migration_id='0008_unsupported_fixture'",
            )
            .expect("TASK105_REPAIR_UNSUPPORTED_HISTORY");
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

    fn v6_absent_writer_fingerprint(&self) -> Vec<String> {
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
                .expect("TASK105_V6_ABSENT_FINGERPRINT")
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
}

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("TASK105_ENV_MISSING:{name}"))
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

fn initialize_request() -> Value {
    json!({
        "jsonrpc":"2.0", "id":1, "method":"initialize",
        "params":{
            "protocolVersion":"2025-11-25", "capabilities":{},
            "clientInfo":{"name":"task105-live","version":"1"}
        }
    })
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

#[test]
fn task105_checkpoint_survives_a_fresh_latticed_process_without_migration() {
    assert_no_merged_sql_continuation_tokens();
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
    println!("TASK105_STAGE_INITIALIZE_ENTER");
    run_latticed_admin(&config, "--postgres-initialize", true);
    config.prepare_v5_writer_v3_bridge();
    config.assert_v5_writer_v3_bridge();
    config.prove_v5_bridge_retry_does_not_repair_rebind_boundary();
    println!("TASK105_STAGE_V5_BRIDGE_RETRY_VERIFY_ONLY_PASS");
    run_latticed_admin(&config, "--postgres-bootstrap", true);
    config.assert_v6_writer_v3_current();
    let migration_before = config.migration_fingerprint();
    assert_eq!(migration_before.1, 6);
    println!("TASK105_STAGE_V5_WRITER_V3_BOOTSTRAP_PASS");

    config.make_v6_writer_bridge_pending();
    config.assert_v6_writer_bridge_pending();
    let pending = config.durable_profile_fingerprint();
    run_latticed_admin(&config, "--postgres-bootstrap", true);
    config.assert_v6_writer_v3_current();
    assert_ne!(config.durable_profile_fingerprint(), pending);
    println!("TASK105_STAGE_V6_BRIDGE_PENDING_PASS");

    let current = config.durable_profile_fingerprint();
    run_latticed_admin(&config, "--postgres-bootstrap", true);
    assert_eq!(config.durable_profile_fingerprint(), current);
    assert_eq!(config.migration_fingerprint(), migration_before);
    println!("TASK105_STAGE_V6_CURRENT_NOOP_PASS");

    config.introduce_partial_writer_acl();
    let partial = config.durable_profile_fingerprint();
    assert_eq!(
        run_latticed_admin(&config, "--postgres-bootstrap", false).trim(),
        "LATTICE_WRITER_LEASE_REJECTED"
    );
    assert_eq!(config.durable_profile_fingerprint(), partial);
    config.repair_partial_writer_acl();
    config.assert_v6_writer_v3_current();
    println!("TASK105_STAGE_PARTIAL_FAIL_CLOSED_PASS");

    config.introduce_corrupt_writer_identity();
    let corrupt = config.durable_profile_fingerprint();
    assert_eq!(
        run_latticed_admin(&config, "--postgres-bootstrap", false).trim(),
        "LATTICE_WRITER_LEASE_REJECTED"
    );
    assert_eq!(config.durable_profile_fingerprint(), corrupt);
    config.repair_corrupt_writer_identity();
    config.assert_v6_writer_v3_current();
    println!("TASK105_STAGE_CORRUPT_FAIL_CLOSED_PASS");

    config.introduce_unsupported_history();
    let unsupported = config.durable_profile_fingerprint();
    assert_eq!(
        run_latticed_admin(&config, "--postgres-bootstrap", false).trim(),
        "LATTICED_RUNTIME_POSTGRES_VERIFICATION_REJECTED"
    );
    assert_eq!(config.durable_profile_fingerprint(), unsupported);
    config.repair_unsupported_history();
    config.assert_v6_writer_v3_current();
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
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"lattice_runtime_status"}}),
    ]);
    let status_b = &response(&process_b, 2)["result"]["structuredContent"]["foreman"];
    assert_eq!(status_b, status_a);
    assert_eq!(config.migration_fingerprint(), migration_before);
    println!("TASK105_STAGE_FRESH_PROCESS_REPLAY_PASS");

    config.remove_disposable_writer_profile();
    let absent_before = config.v6_absent_writer_fingerprint();
    assert_eq!(config.migration_fingerprint().1, 6);
    config.assert_writer_namespace_absent();
    assert_eq!(
        run_latticed_admin(&config, "--postgres-bootstrap", false)
            .replace("\r\n", "\n")
            .trim(),
        "LATTICE_WRITER_LEASE_REJECTED"
    );
    assert_eq!(config.v6_absent_writer_fingerprint(), absent_before);
    println!("TASK105_STAGE_V6_ABSENT_FAIL_CLOSED_PASS");

    let parent_run_id = required("LATTICE_TASK019_RUN_ID");
    let main_during_children = config.v6_absent_writer_fingerprint();

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
    assert_eq!(config.v6_absent_writer_fingerprint(), main_during_children);
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
    writer_v2.assert_v6_writer_v3_current();
    assert_eq!(config.v6_absent_writer_fingerprint(), main_during_children);
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
    bridge_pending.assert_v6_writer_v3_current();
    assert_eq!(config.v6_absent_writer_fingerprint(), main_during_children);
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
    assert_eq!(config.v6_absent_writer_fingerprint(), main_during_children);
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
    writer_absent.assert_v6_writer_v3_current();
    assert_eq!(config.v6_absent_writer_fingerprint(), main_during_children);

    writer_absent.revoke_login_database_privileges();
    writer_absent.assert_login_capability(false);
    run_latticed_admin(&config, "--postgres-initialize", true);
    config.assert_login_capability(true);
    writer_v2.assert_login_capability(false);
    bridge_pending.assert_login_capability(false);
    fresh_partial.assert_login_capability(false);
    legacy_v1.assert_login_capability(false);
    writer_absent.assert_login_capability(false);
    assert_eq!(config.v6_absent_writer_fingerprint(), main_during_children);
    assert_eq!(required("LATTICE_TASK019_RUN_ID"), parent_run_id);
    println!("TASK105_STAGE_V5_WRITER_ABSENT_EXECUTABLE_PASS");
}
