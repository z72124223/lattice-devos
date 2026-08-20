use std::error::Error;
use std::fmt;

use lattice_contracts::ContentDigest;
use postgres::error::SqlState;
use postgres::{Client, GenericClient, IsolationLevel, Transaction};

use crate::{
    APPROVAL_EXTENSION_ID, ExtensionManifestEvidence, sha256_hex,
    verify_embedded_extension_manifest,
};

const GLOBAL_MIGRATION_ADVISORY_LOCK: i64 = 0x4c41_5454_4943_4501;
const APPROVAL_EXTENSION_ADVISORY_LOCK: i64 = 0x4c41_5441_5050_5231;
const CURRENT_GLOBAL_SCHEMA_VERSION: i16 = 5;
const CURRENT_GLOBAL_MANIFEST_SHA256: &str =
    "f92a51fa19c4fe0ffebfc40f20924bd1209bb2441b1bc69f787bc3c4a925425d";
const CURRENT_MEMORY_SCHEMA_VERSION: i16 = 3;
const CURRENT_MEMORY_MANIFEST_SHA256: &str =
    "d4cc712d262ae1f7c96bd65526eab611c90e193363afd865af2126307b2903f0";
const MAX_SERIALIZATION_ATTEMPTS: usize = 3;
const CATALOG_SIGNATURE_DOMAIN: &[u8] = b"LATTICE_POSTGRES_CATALOG_SIGNATURE_V1\0";

const RELATION_PROFILE_SQL: &str = "SELECT pg_catalog.jsonb_build_array(\
    n.nspname,c.relname,c.relkind::text,o.rolname,c.relpersistence::text,c.relrowsecurity,\
    c.relforcerowsecurity,c.relhassubclass,c.relispartition,c.relreplident::text,\
    COALESCE(pg_catalog.array_to_string(c.reloptions,','),'<NULL>'),\
    COALESCE(pg_catalog.obj_description(c.oid,'pg_class'),'<NULL>'))::text \
    FROM pg_catalog.pg_class c \
    JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
    JOIN pg_catalog.pg_roles o ON o.oid=c.relowner \
    WHERE n.nspname='approval_verifier' AND c.relkind IN ('r','p') \
    ORDER BY n.nspname,c.relname";
const COLUMN_PROFILE_SQL: &str = "SELECT pg_catalog.jsonb_build_array(\
    n.nspname,c.relname,a.attnum,a.attname,pg_catalog.format_type(a.atttypid,a.atttypmod),\
    a.attnotnull,a.attisdropped,COALESCE(pg_catalog.pg_get_expr(ad.adbin,ad.adrelid,false),'<NULL>'),\
    a.attidentity::text,a.attgenerated::text,CASE WHEN coll.oid IS NULL THEN '<NULL>' \
    ELSE coll_ns.nspname||'.'||coll.collname END,a.attstorage::text,a.attcompression::text,\
    a.attstattarget)::text FROM pg_catalog.pg_class c \
    JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
    JOIN pg_catalog.pg_attribute a ON a.attrelid=c.oid \
    LEFT JOIN pg_catalog.pg_attrdef ad ON ad.adrelid=c.oid AND ad.adnum=a.attnum \
    LEFT JOIN pg_catalog.pg_collation coll ON coll.oid=a.attcollation \
    LEFT JOIN pg_catalog.pg_namespace coll_ns ON coll_ns.oid=coll.collnamespace \
    WHERE n.nspname='approval_verifier' AND c.relkind IN ('r','p') AND a.attnum>0 \
    ORDER BY n.nspname,c.relname,a.attnum";
const CONSTRAINT_PROFILE_SQL: &str = "SELECT pg_catalog.jsonb_build_array(\
    n.nspname,c.relname,con.conname,con.contype::text,con.convalidated,con.condeferrable,\
    con.condeferred,con.connoinherit,con.conislocal,con.coninhcount,con.conkey,ref_ns.nspname,\
    ref.relname,con.confkey,con.confupdtype::text,con.confdeltype::text,con.confmatchtype::text,\
    pg_catalog.pg_get_constraintdef(con.oid,false))::text \
    FROM pg_catalog.pg_constraint con \
    JOIN pg_catalog.pg_namespace n ON n.oid=con.connamespace \
    JOIN pg_catalog.pg_class c ON c.oid=con.conrelid \
    LEFT JOIN pg_catalog.pg_class ref ON ref.oid=con.confrelid \
    LEFT JOIN pg_catalog.pg_namespace ref_ns ON ref_ns.oid=ref.relnamespace \
    WHERE n.nspname='approval_verifier' ORDER BY n.nspname,c.relname,con.conname";
const INDEX_PROFILE_SQL: &str = "SELECT pg_catalog.jsonb_build_array(\
    n.nspname,t.relname,ix.relname,o.rolname,am.amname,ix.relpersistence::text,\
    COALESCE(pg_catalog.array_to_string(ix.reloptions,','),'<NULL>'),\
    COALESCE(ts.spcname,'<NULL>'),i.indisunique,i.indisprimary,i.indisvalid,i.indisready,\
    i.indislive,i.indisclustered,i.indisreplident,i.indnullsnotdistinct,\
    pg_catalog.pg_get_indexdef(i.indexrelid,0,true))::text \
    FROM pg_catalog.pg_index i \
    JOIN pg_catalog.pg_class t ON t.oid=i.indrelid \
    JOIN pg_catalog.pg_class ix ON ix.oid=i.indexrelid \
    JOIN pg_catalog.pg_namespace n ON n.oid=t.relnamespace \
    JOIN pg_catalog.pg_roles o ON o.oid=ix.relowner \
    JOIN pg_catalog.pg_am am ON am.oid=ix.relam \
    LEFT JOIN pg_catalog.pg_tablespace ts ON ts.oid=ix.reltablespace \
    WHERE n.nspname='approval_verifier' ORDER BY n.nspname,t.relname,ix.relname";
const FUNCTION_PROFILE_SQL: &str = "SELECT pg_catalog.jsonb_build_array(\
    n.nspname,p.proname,pg_catalog.pg_get_function_identity_arguments(p.oid),\
    pg_catalog.pg_get_function_result(p.oid),o.rolname,l.lanname,p.prokind::text,p.prosecdef,\
    p.proleakproof,p.provolatile::text,p.proparallel::text,p.proisstrict,p.proretset,p.pronargs,\
    p.pronargdefaults,p.prorettype::regtype::text,p.proargtypes::text,\
    COALESCE(p.proallargtypes::text,'<NULL>'),COALESCE(p.proargmodes::text,'<NULL>'),\
    COALESCE(p.proargnames::text,'<NULL>'),COALESCE(pg_catalog.array_to_string(p.proconfig,','),'<NULL>'),\
    COALESCE(p.probin,'<NULL>'),p.prosrc,pg_catalog.pg_get_functiondef(p.oid),\
    COALESCE(pg_catalog.obj_description(p.oid,'pg_proc'),'<NULL>'))::text \
    FROM pg_catalog.pg_proc p \
    JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
    JOIN pg_catalog.pg_roles o ON o.oid=p.proowner \
    JOIN pg_catalog.pg_language l ON l.oid=p.prolang \
    WHERE n.nspname='approval_verifier' \
    ORDER BY n.nspname,p.proname,pg_catalog.pg_get_function_identity_arguments(p.oid)";
const SCHEMA_ACL_PROFILE_SQL: &str = "SELECT pg_catalog.jsonb_build_array(\
    n.nspname,o.rolname,COALESCE(g.rolname,'PUBLIC'),r.rolname,a.privilege_type,a.is_grantable)::text \
    FROM pg_catalog.pg_namespace n JOIN pg_catalog.pg_roles o ON o.oid=n.nspowner \
    CROSS JOIN LATERAL pg_catalog.aclexplode(COALESCE(n.nspacl,pg_catalog.acldefault('n',n.nspowner))) a \
    LEFT JOIN pg_catalog.pg_roles g ON g.oid=a.grantee JOIN pg_catalog.pg_roles r ON r.oid=a.grantor \
    WHERE n.nspname='approval_verifier' ORDER BY n.nspname,o.rolname,COALESCE(g.rolname,'PUBLIC'),\
    r.rolname,a.privilege_type,a.is_grantable";
const TABLE_ACL_PROFILE_SQL: &str = "SELECT pg_catalog.jsonb_build_array(\
    n.nspname,c.relname,o.rolname,COALESCE(g.rolname,'PUBLIC'),r.rolname,a.privilege_type,\
    a.is_grantable)::text FROM pg_catalog.pg_class c \
    JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
    JOIN pg_catalog.pg_roles o ON o.oid=c.relowner \
    CROSS JOIN LATERAL pg_catalog.aclexplode(COALESCE(c.relacl,pg_catalog.acldefault('r',c.relowner))) a \
    LEFT JOIN pg_catalog.pg_roles g ON g.oid=a.grantee JOIN pg_catalog.pg_roles r ON r.oid=a.grantor \
    WHERE n.nspname='approval_verifier' AND c.relkind IN ('r','p') \
    ORDER BY n.nspname,c.relname,o.rolname,COALESCE(g.rolname,'PUBLIC'),r.rolname,\
    a.privilege_type,a.is_grantable";
const FUNCTION_ACL_PROFILE_SQL: &str = "SELECT pg_catalog.jsonb_build_array(\
    n.nspname,p.proname,pg_catalog.pg_get_function_identity_arguments(p.oid),o.rolname,\
    COALESCE(g.rolname,'PUBLIC'),r.rolname,a.privilege_type,a.is_grantable)::text \
    FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
    JOIN pg_catalog.pg_roles o ON o.oid=p.proowner \
    CROSS JOIN LATERAL pg_catalog.aclexplode(COALESCE(p.proacl,pg_catalog.acldefault('f',p.proowner))) a \
    LEFT JOIN pg_catalog.pg_roles g ON g.oid=a.grantee JOIN pg_catalog.pg_roles r ON r.oid=a.grantor \
    WHERE n.nspname='approval_verifier' ORDER BY n.nspname,p.proname,\
    pg_catalog.pg_get_function_identity_arguments(p.oid),o.rolname,COALESCE(g.rolname,'PUBLIC'),\
    r.rolname,a.privilege_type,a.is_grantable";
const COLUMN_ACL_PROFILE_SQL: &str = "SELECT pg_catalog.jsonb_build_array(\
    n.nspname,c.relname,a.attnum,a.attname,COALESCE(g.rolname,'PUBLIC'),r.rolname,x.privilege_type,\
    x.is_grantable)::text FROM pg_catalog.pg_class c \
    JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
    JOIN pg_catalog.pg_attribute a ON a.attrelid=c.oid \
    CROSS JOIN LATERAL pg_catalog.aclexplode(a.attacl) x \
    LEFT JOIN pg_catalog.pg_roles g ON g.oid=x.grantee JOIN pg_catalog.pg_roles r ON r.oid=x.grantor \
    WHERE n.nspname='approval_verifier' AND c.relkind IN ('r','p') AND a.attnum>0 \
    ORDER BY n.nspname,c.relname,a.attnum,COALESCE(g.rolname,'PUBLIC'),r.rolname,\
    x.privilege_type,x.is_grantable";
const TYPE_PROFILE_SQL: &str = "SELECT pg_catalog.jsonb_build_array(\
    n.nspname,t.typname,t.typtype::text,t.typcategory::text,t.typispreferred,t.typisdefined,\
    t.typdelim::text,o.rolname,COALESCE(c.relname,'<NULL>'),COALESCE(e.typname,'<NULL>'),\
    COALESCE(pg_catalog.obj_description(t.oid,'pg_type'),'<NULL>'))::text \
    FROM pg_catalog.pg_type t JOIN pg_catalog.pg_namespace n ON n.oid=t.typnamespace \
    JOIN pg_catalog.pg_roles o ON o.oid=t.typowner \
    LEFT JOIN pg_catalog.pg_class c ON c.oid=t.typrelid \
    LEFT JOIN pg_catalog.pg_type e ON e.oid=t.typelem \
    WHERE n.nspname='approval_verifier' ORDER BY n.nspname,t.typname";

const EXPECTED_CATALOG_PROFILES: [(&str, usize, &str); 10] = [
    (
        RELATION_PROFILE_SQL,
        5,
        "7e78c47139e50c0ca5c9760b36bf5ff07ef41f9fd938aced546d0ef4be5222dd",
    ),
    (
        COLUMN_PROFILE_SQL,
        62,
        "ad2b6c3df0d3a027f4144bdedb79f94ba66bcbfa36bd8b416c75219fcfe6c42e",
    ),
    (
        CONSTRAINT_PROFILE_SQL,
        52,
        "43c564bf7b1d7f2dd75154f892a14d17b0fa9a6004dcf8460503aecc4fe5e790",
    ),
    (
        INDEX_PROFILE_SQL,
        10,
        "e750cde948cc3201fdfa9c31daae2084f5368c47f8f7e4841a753d389a75ce24",
    ),
    (
        FUNCTION_PROFILE_SQL,
        5,
        "65aa758f25c4fad7e11ec6d0683d682e327d35b846e10bcf2802c8035bdd0e56",
    ),
    (
        SCHEMA_ACL_PROFILE_SQL,
        3,
        "a9efd4bc586cabc44d3c83ff5e3e8bb3c65d0bdb8baed3337d0d25f581a57a73",
    ),
    (
        TABLE_ACL_PROFILE_SQL,
        40,
        "4b15ab573041a14f6bf4eaedfe378f4adfe0238cc940f6a473356306fbf375ed",
    ),
    (
        FUNCTION_ACL_PROFILE_SQL,
        10,
        "0db310acbfe2dd82a5c8e6e98a2f3f09cbb14a5b5df7c644d42d271e71e1e11a",
    ),
    (
        COLUMN_ACL_PROFILE_SQL,
        0,
        "a7ccfc938fbf121a9b807070f69bd5b851be6aa89a8261043ef07336ea7b8dbd",
    ),
    (
        TYPE_PROFILE_SQL,
        10,
        "6d62784de33e9f97543763507888da6fab76a95a9175b5d913f7a3b871537d31",
    ),
];

const EXPECTED_RELATIONS: [&str; 5] = [
    "approval_commands",
    "approval_effect_claims",
    "approval_extension_identity",
    "approval_extension_ledger",
    "approval_heads",
];
const EXPECTED_FUNCTIONS: [(&str, &str); 5] = [
    (
        "approval_verifier_commit_plan_v1",
        "bigint, bytea, text, text, text, bigint, bytea, bytea, bigint, bytea, bytea, bytea, text, text, bytea, bytea, bytea, bytea, bytea, bytea, bytea, text, text, text, text, bytea, bytea, bytea, bytea, bytea",
    ),
    ("approval_verifier_load_commands_v1", ""),
    (
        "approval_verifier_load_current_v1",
        "text, text, text, text, text, text",
    ),
    ("approval_verifier_load_effects_v1", ""),
    (
        "approval_verifier_load_for_update_v1",
        "text, bytea, bytea, bytea, bytea, text, text, text, text, text",
    ),
];

/// Exact database/global/Memory identity admitted by this extension.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExtensionTarget {
    database_name: String,
    database_identity_digest: ContentDigest,
    global_manifest_digest: ContentDigest,
    memory_manifest_digest: ContentDigest,
}

impl ExtensionTarget {
    /// Constructs a fixed target without accepting a URL, credential, or SQL.
    ///
    /// # Errors
    ///
    /// Rejects an unsafe or unbounded database name.
    pub fn new(
        database_name: String,
        database_identity_digest: ContentDigest,
        global_manifest_digest: ContentDigest,
        memory_manifest_digest: ContentDigest,
    ) -> Result<Self, ExtensionSetupError> {
        if database_name.is_empty()
            || database_name.len() > 63
            || !database_name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        {
            return Err(ExtensionSetupError::new(
                ExtensionSetupErrorKind::InvalidTarget,
            ));
        }
        Ok(Self {
            database_name,
            database_identity_digest,
            global_manifest_digest,
            memory_manifest_digest,
        })
    }

    #[must_use]
    pub fn database_name(&self) -> &str {
        &self.database_name
    }

    #[must_use]
    pub const fn database_identity_digest(&self) -> &ContentDigest {
        &self.database_identity_digest
    }

    #[must_use]
    pub const fn global_manifest_digest(&self) -> &ContentDigest {
        &self.global_manifest_digest
    }

    #[must_use]
    pub const fn memory_manifest_digest(&self) -> &ContentDigest {
        &self.memory_manifest_digest
    }
}

/// Administrative apply result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExtensionApplyOutcome {
    Installed,
    AlreadyCurrent,
}

/// Closed extension setup/verifier failure classes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExtensionSetupErrorKind {
    InvalidTarget,
    ManifestMismatch,
    UnsupportedFoundation,
    PartialOrCollidingProfile,
    PermissionDenied,
    Database,
}

/// Non-secret setup/verifier failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExtensionSetupError {
    kind: ExtensionSetupErrorKind,
}

impl ExtensionSetupError {
    const fn new(kind: ExtensionSetupErrorKind) -> Self {
        Self { kind }
    }

    #[must_use]
    pub const fn kind(self) -> ExtensionSetupErrorKind {
        self.kind
    }

    #[must_use]
    pub const fn code(self) -> &'static str {
        match self.kind {
            ExtensionSetupErrorKind::InvalidTarget => "APPROVAL_EXTENSION_INVALID_TARGET",
            ExtensionSetupErrorKind::ManifestMismatch => "APPROVAL_EXTENSION_MANIFEST_MISMATCH",
            ExtensionSetupErrorKind::UnsupportedFoundation => {
                "APPROVAL_EXTENSION_FOUNDATION_UNSUPPORTED"
            }
            ExtensionSetupErrorKind::PartialOrCollidingProfile => {
                "APPROVAL_EXTENSION_PROFILE_COLLISION"
            }
            ExtensionSetupErrorKind::PermissionDenied => "APPROVAL_EXTENSION_PERMISSION_DENIED",
            ExtensionSetupErrorKind::Database => "APPROVAL_EXTENSION_DATABASE_FAILURE",
        }
    }
}

impl fmt::Display for ExtensionSetupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl Error for ExtensionSetupError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SetupAttemptError {
    Setup(ExtensionSetupError),
    SerializationFailure,
}

impl SetupAttemptError {
    fn into_public(self) -> ExtensionSetupError {
        match self {
            Self::Setup(error) => error,
            Self::SerializationFailure => {
                ExtensionSetupError::new(ExtensionSetupErrorKind::Database)
            }
        }
    }
}

impl From<ExtensionSetupError> for SetupAttemptError {
    fn from(error: ExtensionSetupError) -> Self {
        Self::Setup(error)
    }
}

struct FoundationEvidence {
    database_uuid: String,
}

/// Installs or verifies the exact Approval extension profile.
///
/// # Errors
///
/// Fails closed for foundation, catalog, ACL, permission, or database drift.
pub fn apply_extension(
    client: &mut Client,
    target: &ExtensionTarget,
) -> Result<ExtensionApplyOutcome, ExtensionSetupError> {
    let manifest = verify_embedded_extension_manifest()
        .map_err(|_| ExtensionSetupError::new(ExtensionSetupErrorKind::ManifestMismatch))?;
    for attempt in 1..=MAX_SERIALIZATION_ATTEMPTS {
        match apply_extension_attempt(client, target, &manifest) {
            Err(SetupAttemptError::SerializationFailure)
                if attempt < MAX_SERIALIZATION_ATTEMPTS => {}
            result => return result.map_err(SetupAttemptError::into_public),
        }
    }
    Err(ExtensionSetupError::new(ExtensionSetupErrorKind::Database))
}

fn apply_extension_attempt(
    client: &mut Client,
    target: &ExtensionTarget,
    manifest: &ExtensionManifestEvidence,
) -> Result<ExtensionApplyOutcome, SetupAttemptError> {
    let mut transaction = client
        .build_transaction()
        .isolation_level(IsolationLevel::Serializable)
        .start()
        .map_err(map_database)?;
    enter_migrator(&mut transaction)?;
    acquire_locks(&mut transaction)?;
    let foundation = verify_foundation(&mut transaction, target)?;
    let schema_count: i64 = transaction
        .query_one(
            "SELECT pg_catalog.count(*) FROM pg_catalog.pg_namespace WHERE nspname='approval_verifier'",
            &[],
        )
        .and_then(|row| row.try_get(0))
        .map_err(map_database)?;
    let outcome = if schema_count == 0 {
        install_fresh(&mut transaction, target, &foundation, manifest)?;
        ExtensionApplyOutcome::Installed
    } else if schema_count == 1 {
        ExtensionApplyOutcome::AlreadyCurrent
    } else {
        return Err(profile_collision());
    };
    verify_current_profile(&mut transaction, target, &foundation, manifest)?;
    transaction.commit().map_err(map_database)?;
    Ok(outcome)
}

/// Verifies the exact installed profile without mutation.
///
/// # Errors
///
/// Rejects any partial, extra, substituted, or privilege-drifted profile.
pub fn verify_extension(
    client: &mut Client,
    target: &ExtensionTarget,
) -> Result<(), ExtensionSetupError> {
    verify_extension_attempt(client, target).map_err(SetupAttemptError::into_public)
}

fn verify_extension_attempt(
    client: &mut Client,
    target: &ExtensionTarget,
) -> Result<(), SetupAttemptError> {
    let manifest = verify_embedded_extension_manifest()
        .map_err(|_| setup_error(ExtensionSetupErrorKind::ManifestMismatch))?;
    let mut transaction = client
        .build_transaction()
        .isolation_level(IsolationLevel::RepeatableRead)
        .read_only(true)
        .start()
        .map_err(map_database)?;
    enter_migrator(&mut transaction)?;
    acquire_locks(&mut transaction)?;
    let foundation = verify_foundation(&mut transaction, target)?;
    verify_current_profile(&mut transaction, target, &foundation, &manifest)?;
    transaction.commit().map_err(map_database)
}

fn enter_migrator(transaction: &mut Transaction<'_>) -> Result<(), SetupAttemptError> {
    transaction
        .batch_execute(
            "SET LOCAL ROLE lattice_migrator; \
             SET LOCAL search_path = pg_catalog; \
             SET LOCAL row_security = on; \
             SET LOCAL lock_timeout = '5s'; \
             SET LOCAL statement_timeout = '30s'; \
             SET LOCAL idle_in_transaction_session_timeout = '30s';",
        )
        .map_err(|error| map_database_or(error, ExtensionSetupErrorKind::PermissionDenied))
}

fn acquire_locks<C: GenericClient>(client: &mut C) -> Result<(), SetupAttemptError> {
    for lock in [
        GLOBAL_MIGRATION_ADVISORY_LOCK,
        APPROVAL_EXTENSION_ADVISORY_LOCK,
    ] {
        client
            .execute("SELECT pg_catalog.pg_advisory_xact_lock($1)", &[&lock])
            .map_err(map_database)?;
    }
    Ok(())
}

fn verify_foundation<C: GenericClient>(
    client: &mut C,
    target: &ExtensionTarget,
) -> Result<FoundationEvidence, SetupAttemptError> {
    let row = client
        .query_one(
            "SELECT pg_catalog.current_database()::text,d.database_uuid::text, \
                    pg_catalog.btrim(m.database_identity_sha256)::text, \
                    c.current_schema_version,pg_catalog.btrim(c.manifest_sha256)::text, \
                    m.extension_schema_version,pg_catalog.btrim(m.extension_manifest_sha256)::text \
               FROM ONLY control.database_identity AS d \
               CROSS JOIN ONLY control.schema_compatibility AS c \
               CROSS JOIN ONLY memory.codebase_memory_extension_identity AS m \
              WHERE d.singleton AND c.singleton AND m.singleton AND m.database_uuid=d.database_uuid",
            &[],
        )
        .map_err(|error| map_database_or(error, ExtensionSetupErrorKind::UnsupportedFoundation))?;
    let database_name: String = row.try_get(0).map_err(map_database)?;
    let database_uuid: String = row.try_get(1).map_err(map_database)?;
    let database_identity: String = row.try_get(2).map_err(map_database)?;
    let global_version: i16 = row.try_get(3).map_err(map_database)?;
    let global_manifest: String = row.try_get(4).map_err(map_database)?;
    let memory_version: i16 = row.try_get(5).map_err(map_database)?;
    let memory_manifest: String = row.try_get(6).map_err(map_database)?;
    if database_name != target.database_name()
        || database_identity != target.database_identity_digest().as_str()
        || global_version != CURRENT_GLOBAL_SCHEMA_VERSION
        || global_manifest != CURRENT_GLOBAL_MANIFEST_SHA256
        || global_manifest != target.global_manifest_digest().as_str()
        || memory_version != CURRENT_MEMORY_SCHEMA_VERSION
        || memory_manifest != CURRENT_MEMORY_MANIFEST_SHA256
        || memory_manifest != target.memory_manifest_digest().as_str()
    {
        return Err(setup_error(ExtensionSetupErrorKind::UnsupportedFoundation));
    }
    Ok(FoundationEvidence { database_uuid })
}

fn install_fresh<C: GenericClient>(
    client: &mut C,
    target: &ExtensionTarget,
    foundation: &FoundationEvidence,
    manifest: &ExtensionManifestEvidence,
) -> Result<(), SetupAttemptError> {
    let sql = std::str::from_utf8(manifest.bytes())
        .map_err(|_| setup_error(ExtensionSetupErrorKind::ManifestMismatch))?;
    client.batch_execute(sql).map_err(map_database)?;
    let sql_digest = decode_digest(manifest.sql_sha256())?;
    let manifest_digest = decode_digest(manifest.manifest_sha256())?;
    let parameters: [&(dyn postgres::types::ToSql + Sync); 8] = [
        &sql_digest,
        &manifest_digest,
        &foundation.database_uuid,
        &target.database_identity_digest().as_str(),
        &CURRENT_GLOBAL_SCHEMA_VERSION,
        &target.global_manifest_digest().as_str(),
        &CURRENT_MEMORY_SCHEMA_VERSION,
        &target.memory_manifest_digest().as_str(),
    ];
    client
        .execute(
            "INSERT INTO approval_verifier.approval_extension_identity( \
                 singleton,extension_id,schema_version,sql_sha256,manifest_sha256,database_uuid, \
                 database_identity_sha256,global_schema_version,global_manifest_sha256, \
                 required_memory_schema_version,required_memory_manifest_sha256) \
             VALUES(true,'lattice-approval-verifier',1,$1,$2,$3::text::uuid,$4,$5,$6,$7,$8)",
            &parameters,
        )
        .map_err(map_database)?;
    client
        .execute(
            "INSERT INTO approval_verifier.approval_extension_ledger( \
                 ordinal,event_type,extension_id,schema_version,sql_sha256,manifest_sha256, \
                 database_uuid,database_identity_sha256,global_schema_version, \
                 global_manifest_sha256,required_memory_schema_version,required_memory_manifest_sha256) \
             VALUES(1,'INSTALLED','lattice-approval-verifier',1,$1,$2,$3::text::uuid,$4,$5,$6,$7,$8)",
            &parameters,
        )
        .map_err(map_database)?;
    Ok(())
}

fn verify_current_profile<C: GenericClient>(
    client: &mut C,
    target: &ExtensionTarget,
    foundation: &FoundationEvidence,
    manifest: &ExtensionManifestEvidence,
) -> Result<(), SetupAttemptError> {
    verify_catalog(client)?;
    verify_acl(client)?;
    for (query, expected_rows, expected_signature) in EXPECTED_CATALOG_PROFILES {
        verify_catalog_profile(client, query, expected_rows, expected_signature)?;
    }
    verify_namespace_and_effective_acl_closure(client)?;
    let sql_digest = decode_digest(manifest.sql_sha256())?;
    let manifest_digest = decode_digest(manifest.manifest_sha256())?;
    let row = client
        .query_one(
            "SELECT i.extension_id,i.schema_version,i.sql_sha256,i.manifest_sha256, \
                    i.database_uuid::text,pg_catalog.btrim(i.database_identity_sha256)::text, \
                    i.global_schema_version,pg_catalog.btrim(i.global_manifest_sha256)::text, \
                    i.required_memory_schema_version, \
                    pg_catalog.btrim(i.required_memory_manifest_sha256)::text, \
                    (SELECT pg_catalog.count(*) FROM ONLY approval_verifier.approval_extension_identity), \
                    (SELECT pg_catalog.count(*) FROM ONLY approval_verifier.approval_extension_ledger), \
                    (SELECT pg_catalog.count(*) FROM ONLY approval_verifier.approval_extension_ledger l \
                      WHERE l.ordinal=1 AND l.event_type='INSTALLED' \
                        AND l.extension_id=i.extension_id AND l.schema_version=i.schema_version \
                        AND l.sql_sha256=i.sql_sha256 AND l.manifest_sha256=i.manifest_sha256 \
                        AND l.database_uuid=i.database_uuid \
                        AND l.database_identity_sha256=i.database_identity_sha256 \
                        AND l.global_schema_version=i.global_schema_version \
                        AND l.global_manifest_sha256=i.global_manifest_sha256 \
                        AND l.required_memory_schema_version=i.required_memory_schema_version \
                        AND l.required_memory_manifest_sha256=i.required_memory_manifest_sha256) \
               FROM ONLY approval_verifier.approval_extension_identity i WHERE i.singleton",
            &[],
        )
        .map_err(|_| profile_collision())?;
    let exact = row.try_get::<_, String>(0).map_err(map_database)? == APPROVAL_EXTENSION_ID
        && row.try_get::<_, i16>(1).map_err(map_database)? == 1
        && row.try_get::<_, Vec<u8>>(2).map_err(map_database)? == sql_digest
        && row.try_get::<_, Vec<u8>>(3).map_err(map_database)? == manifest_digest
        && row.try_get::<_, String>(4).map_err(map_database)? == foundation.database_uuid
        && row.try_get::<_, String>(5).map_err(map_database)?
            == target.database_identity_digest().as_str()
        && row.try_get::<_, i16>(6).map_err(map_database)? == CURRENT_GLOBAL_SCHEMA_VERSION
        && row.try_get::<_, String>(7).map_err(map_database)?
            == target.global_manifest_digest().as_str()
        && row.try_get::<_, i16>(8).map_err(map_database)? == CURRENT_MEMORY_SCHEMA_VERSION
        && row.try_get::<_, String>(9).map_err(map_database)?
            == target.memory_manifest_digest().as_str()
        && row.try_get::<_, i64>(10).map_err(map_database)? == 1
        && row.try_get::<_, i64>(11).map_err(map_database)? == 1
        && row.try_get::<_, i64>(12).map_err(map_database)? == 1;
    if !exact {
        return Err(profile_collision());
    }
    Ok(())
}

fn verify_catalog_profile<C: GenericClient>(
    client: &mut C,
    query: &str,
    expected_rows: usize,
    expected_signature: &str,
) -> Result<(), SetupAttemptError> {
    let rows = client
        .query(query, &[])
        .map_err(map_database)?
        .into_iter()
        .map(|row| row.try_get::<_, String>(0).map_err(map_database))
        .collect::<Result<Vec<_>, _>>()?;
    if rows.len() != expected_rows || catalog_signature(&rows)? != expected_signature {
        return Err(profile_collision());
    }
    Ok(())
}

fn catalog_signature(rows: &[String]) -> Result<String, SetupAttemptError> {
    let mut framed = Vec::with_capacity(
        CATALOG_SIGNATURE_DOMAIN.len() + 8 + rows.iter().map(|row| row.len() + 8).sum::<usize>(),
    );
    framed.extend_from_slice(CATALOG_SIGNATURE_DOMAIN);
    framed.extend_from_slice(
        &u64::try_from(rows.len())
            .map_err(|_| profile_collision())?
            .to_be_bytes(),
    );
    for row in rows {
        framed.extend_from_slice(
            &u64::try_from(row.len())
                .map_err(|_| profile_collision())?
                .to_be_bytes(),
        );
        framed.extend_from_slice(row.as_bytes());
    }
    Ok(sha256_hex(&framed))
}

fn verify_namespace_and_effective_acl_closure<C: GenericClient>(
    client: &mut C,
) -> Result<(), SetupAttemptError> {
    let row = client
        .query_one(
            "SELECT \
             (SELECT pg_catalog.count(*) FROM pg_catalog.pg_trigger tr \
               JOIN pg_catalog.pg_class c ON c.oid=tr.tgrelid \
               JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
              WHERE n.nspname='approval_verifier' AND tr.tgisinternal \
                AND tr.tgenabled='O' AND tr.tgconstraint<>0), \
             (SELECT pg_catalog.count(*) FROM pg_catalog.pg_trigger tr \
               JOIN pg_catalog.pg_class c ON c.oid=tr.tgrelid \
               JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
              WHERE n.nspname='approval_verifier' AND NOT tr.tgisinternal), \
             (SELECT pg_catalog.count(*) FROM pg_catalog.pg_rewrite rw \
               JOIN pg_catalog.pg_class c ON c.oid=rw.ev_class \
               JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
              WHERE n.nspname='approval_verifier'), \
             (SELECT pg_catalog.count(*) FROM pg_catalog.pg_policy p \
               JOIN pg_catalog.pg_class c ON c.oid=p.polrelid \
               JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
              WHERE n.nspname='approval_verifier'), \
             (SELECT pg_catalog.count(*) FROM pg_catalog.pg_inherits i \
               JOIN pg_catalog.pg_class c ON c.oid=i.inhrelid \
               JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
              WHERE n.nspname='approval_verifier'), \
             (SELECT pg_catalog.count(*) FROM pg_catalog.pg_default_acl d \
               JOIN pg_catalog.pg_namespace n ON n.oid=d.defaclnamespace \
              WHERE n.nspname='approval_verifier'), \
             ((SELECT pg_catalog.count(*) FROM pg_catalog.pg_collation x \
                JOIN pg_catalog.pg_namespace n ON n.oid=x.collnamespace WHERE n.nspname='approval_verifier') + \
              (SELECT pg_catalog.count(*) FROM pg_catalog.pg_conversion x \
                JOIN pg_catalog.pg_namespace n ON n.oid=x.connamespace WHERE n.nspname='approval_verifier') + \
              (SELECT pg_catalog.count(*) FROM pg_catalog.pg_operator x \
                JOIN pg_catalog.pg_namespace n ON n.oid=x.oprnamespace WHERE n.nspname='approval_verifier') + \
              (SELECT pg_catalog.count(*) FROM pg_catalog.pg_opclass x \
                JOIN pg_catalog.pg_namespace n ON n.oid=x.opcnamespace WHERE n.nspname='approval_verifier') + \
              (SELECT pg_catalog.count(*) FROM pg_catalog.pg_opfamily x \
                JOIN pg_catalog.pg_namespace n ON n.oid=x.opfnamespace WHERE n.nspname='approval_verifier') + \
              (SELECT pg_catalog.count(*) FROM pg_catalog.pg_statistic_ext x \
                JOIN pg_catalog.pg_namespace n ON n.oid=x.stxnamespace WHERE n.nspname='approval_verifier') + \
              (SELECT pg_catalog.count(*) FROM pg_catalog.pg_ts_config x \
                JOIN pg_catalog.pg_namespace n ON n.oid=x.cfgnamespace WHERE n.nspname='approval_verifier') + \
              (SELECT pg_catalog.count(*) FROM pg_catalog.pg_ts_dict x \
                JOIN pg_catalog.pg_namespace n ON n.oid=x.dictnamespace WHERE n.nspname='approval_verifier') + \
              (SELECT pg_catalog.count(*) FROM pg_catalog.pg_ts_parser x \
                JOIN pg_catalog.pg_namespace n ON n.oid=x.prsnamespace WHERE n.nspname='approval_verifier') + \
              (SELECT pg_catalog.count(*) FROM pg_catalog.pg_ts_template x \
                JOIN pg_catalog.pg_namespace n ON n.oid=x.tmplnamespace WHERE n.nspname='approval_verifier')), \
             (SELECT pg_catalog.count(*) FROM pg_catalog.pg_class c \
               JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
               CROSS JOIN pg_catalog.pg_roles roles \
              WHERE n.nspname='approval_verifier' AND c.relkind IN ('r','p') \
                AND NOT roles.rolsuper AND roles.rolname !~ '^pg_' \
                AND roles.rolname <> 'lattice_migrator' \
                AND pg_catalog.has_table_privilege(roles.rolname,c.oid,\
                  'SELECT,INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER,MAINTAIN')), \
             (SELECT pg_catalog.count(*) FROM pg_catalog.pg_proc p \
               JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
               CROSS JOIN pg_catalog.pg_roles roles \
              WHERE n.nspname='approval_verifier' \
                AND NOT roles.rolsuper AND roles.rolname !~ '^pg_' \
                AND roles.rolname NOT IN ('lattice_migrator','lattice_runtime') \
                AND pg_catalog.has_function_privilege(roles.rolname,p.oid,'EXECUTE')), \
             (SELECT pg_catalog.count(*) FROM pg_catalog.pg_proc p \
               JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
              WHERE n.nspname='approval_verifier' \
                AND NOT pg_catalog.has_function_privilege('lattice_runtime',p.oid,'EXECUTE')), \
             pg_catalog.has_schema_privilege('lattice_runtime','approval_verifier','USAGE'), \
             (pg_catalog.has_schema_privilege('lattice_runtime','approval_verifier','CREATE') \
               OR EXISTS (SELECT 1 FROM pg_catalog.pg_roles roles \
                  WHERE NOT roles.rolsuper AND roles.rolname !~ '^pg_' \
                    AND roles.rolname NOT IN ('lattice_migrator','lattice_runtime') \
                    AND (pg_catalog.has_schema_privilege(roles.rolname,'approval_verifier','USAGE') \
                      OR pg_catalog.has_schema_privilege(roles.rolname,'approval_verifier','CREATE'))))",
            &[],
        )
        .map_err(map_database)?;
    let expected_counts = [4_i64, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    for (index, expected) in expected_counts.into_iter().enumerate() {
        if row.try_get::<_, i64>(index).map_err(map_database)? != expected {
            return Err(profile_collision());
        }
    }
    if !row.try_get::<_, bool>(10).map_err(map_database)?
        || row.try_get::<_, bool>(11).map_err(map_database)?
    {
        return Err(profile_collision());
    }
    Ok(())
}

fn verify_catalog<C: GenericClient>(client: &mut C) -> Result<(), SetupAttemptError> {
    let schema_owner: Option<String> = client
        .query_opt(
            "SELECT o.rolname FROM pg_catalog.pg_namespace n \
             JOIN pg_catalog.pg_roles o ON o.oid=n.nspowner \
             WHERE n.nspname='approval_verifier'",
            &[],
        )
        .map_err(map_database)?
        .map(|row| row.get(0));
    if schema_owner.as_deref() != Some("lattice_migrator") {
        return Err(profile_collision());
    }
    let relations: Vec<String> = client
        .query(
            "SELECT c.relname FROM pg_catalog.pg_class c \
             JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
             JOIN pg_catalog.pg_roles o ON o.oid=c.relowner \
             WHERE n.nspname='approval_verifier' AND c.relkind IN ('r','p') \
               AND o.rolname='lattice_migrator' ORDER BY c.relname",
            &[],
        )
        .map_err(map_database)?
        .into_iter()
        .map(|row| row.get(0))
        .collect();
    if relations != EXPECTED_RELATIONS {
        return Err(profile_collision());
    }
    let all_relation_count: i64 = client
        .query_one(
            "SELECT pg_catalog.count(*) FROM pg_catalog.pg_class c \
             JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
             WHERE n.nspname='approval_verifier' AND c.relkind NOT IN ('i','I')",
            &[],
        )
        .and_then(|row| row.try_get(0))
        .map_err(map_database)?;
    if all_relation_count != i64::try_from(EXPECTED_RELATIONS.len()).unwrap_or(-1) {
        return Err(profile_collision());
    }
    let functions: Vec<(String, String)> = client
        .query(
            "SELECT p.proname,pg_catalog.oidvectortypes(p.proargtypes) \
             FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
             JOIN pg_catalog.pg_roles o ON o.oid=p.proowner \
             WHERE n.nspname='approval_verifier' AND o.rolname='lattice_migrator' \
             ORDER BY p.proname,pg_catalog.oidvectortypes(p.proargtypes)",
            &[],
        )
        .map_err(map_database)?
        .into_iter()
        .map(|row| (row.get(0), row.get(1)))
        .collect();
    let expected: Vec<(String, String)> = EXPECTED_FUNCTIONS
        .iter()
        .map(|(name, arguments)| ((*name).to_owned(), (*arguments).to_owned()))
        .collect();
    if functions != expected {
        return Err(profile_collision());
    }
    let type_names: Vec<String> = client
        .query(
            "SELECT t.typname FROM pg_catalog.pg_type t \
             JOIN pg_catalog.pg_namespace n ON n.oid=t.typnamespace \
             WHERE n.nspname='approval_verifier' ORDER BY t.typname",
            &[],
        )
        .map_err(map_database)?
        .into_iter()
        .map(|row| row.get(0))
        .collect();
    let expected_types: Vec<String> = EXPECTED_RELATIONS
        .iter()
        .flat_map(|name| [(*name).to_owned(), format!("_{name}")])
        .collect::<Vec<_>>();
    let mut expected_types = expected_types;
    expected_types.sort();
    if type_names != expected_types {
        return Err(profile_collision());
    }
    Ok(())
}

fn verify_acl<C: GenericClient>(client: &mut C) -> Result<(), SetupAttemptError> {
    let schema_acl = client
        .query_one(
            "SELECT \
                 pg_catalog.has_schema_privilege('lattice_runtime','approval_verifier','USAGE'), \
                 pg_catalog.has_schema_privilege('lattice_runtime','approval_verifier','CREATE'), \
                 pg_catalog.has_schema_privilege('lattice_readonly','approval_verifier','USAGE'), \
                 EXISTS(SELECT 1 FROM pg_catalog.pg_namespace n \
                         CROSS JOIN LATERAL pg_catalog.aclexplode( \
                             COALESCE(n.nspacl,pg_catalog.acldefault('n',n.nspowner))) a \
                        WHERE n.nspname='approval_verifier' AND a.grantee=0 \
                          AND a.privilege_type='USAGE')",
            &[],
        )
        .map_err(map_database)?;
    if !schema_acl.get::<_, bool>(0)
        || schema_acl.get::<_, bool>(1)
        || schema_acl.get::<_, bool>(2)
        || schema_acl.get::<_, bool>(3)
    {
        return Err(profile_collision());
    }
    let forbidden_table_acl_count: i64 = client
        .query_one(
            "SELECT pg_catalog.count(*) FROM pg_catalog.pg_class c \
             JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
             CROSS JOIN (VALUES ('lattice_runtime'),('lattice_readonly')) AS r(role_name) \
             WHERE n.nspname='approval_verifier' AND c.relkind IN ('r','p') \
               AND pg_catalog.has_table_privilege(r.role_name, \
                   pg_catalog.format('%I.%I',n.nspname,c.relname), \
                   'SELECT,INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER')",
            &[],
        )
        .and_then(|row| row.try_get(0))
        .map_err(map_database)?;
    if forbidden_table_acl_count != 0 {
        return Err(profile_collision());
    }
    let public_table_acl_count: i64 = client
        .query_one(
            "SELECT pg_catalog.count(*) FROM pg_catalog.pg_class c \
             JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
             CROSS JOIN LATERAL pg_catalog.aclexplode( \
                 COALESCE(c.relacl,pg_catalog.acldefault('r',c.relowner))) a \
             WHERE n.nspname='approval_verifier' AND c.relkind IN ('r','p') \
               AND a.grantee=0",
            &[],
        )
        .and_then(|row| row.try_get(0))
        .map_err(map_database)?;
    if public_table_acl_count != 0 {
        return Err(profile_collision());
    }
    let function_acl = client
        .query_one(
            "SELECT \
                 pg_catalog.count(*) FILTER (WHERE pg_catalog.has_function_privilege( \
                     'lattice_runtime',p.oid,'EXECUTE')), \
                 pg_catalog.count(*) FILTER (WHERE pg_catalog.has_function_privilege( \
                     'lattice_readonly',p.oid,'EXECUTE')), \
                 (SELECT pg_catalog.count(*) FROM pg_catalog.pg_proc p2 \
                   JOIN pg_catalog.pg_namespace n2 ON n2.oid=p2.pronamespace \
                   CROSS JOIN LATERAL pg_catalog.aclexplode( \
                       COALESCE(p2.proacl,pg_catalog.acldefault('f',p2.proowner))) a \
                  WHERE n2.nspname='approval_verifier' AND a.grantee=0) \
             FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
             WHERE n.nspname='approval_verifier'",
            &[],
        )
        .map_err(map_database)?;
    if function_acl.get::<_, i64>(0) != i64::try_from(EXPECTED_FUNCTIONS.len()).unwrap_or(-1)
        || function_acl.get::<_, i64>(1) != 0
        || function_acl.get::<_, i64>(2) != 0
    {
        return Err(profile_collision());
    }
    let column_acl_count: i64 = client
        .query_one(
            "SELECT pg_catalog.count(*) FROM pg_catalog.pg_attribute a \
             JOIN pg_catalog.pg_class c ON c.oid=a.attrelid \
             JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
             WHERE n.nspname='approval_verifier' AND a.attnum>0 AND a.attacl IS NOT NULL",
            &[],
        )
        .and_then(|row| row.try_get(0))
        .map_err(map_database)?;
    if column_acl_count != 0 {
        return Err(profile_collision());
    }
    Ok(())
}

fn decode_digest(digest: &ContentDigest) -> Result<Vec<u8>, SetupAttemptError> {
    let text = digest.as_str().as_bytes();
    if text.len() != 64 {
        return Err(setup_error(ExtensionSetupErrorKind::ManifestMismatch));
    }
    text.chunks_exact(2)
        .map(|pair| {
            let pair = std::str::from_utf8(pair)
                .map_err(|_| setup_error(ExtensionSetupErrorKind::ManifestMismatch))?;
            u8::from_str_radix(pair, 16)
                .map_err(|_| setup_error(ExtensionSetupErrorKind::ManifestMismatch))
        })
        .collect()
}

fn profile_collision() -> SetupAttemptError {
    setup_error(ExtensionSetupErrorKind::PartialOrCollidingProfile)
}

fn setup_error(kind: ExtensionSetupErrorKind) -> SetupAttemptError {
    ExtensionSetupError::new(kind).into()
}

#[allow(clippy::needless_pass_by_value)]
fn map_database(error: postgres::Error) -> SetupAttemptError {
    if error.code() == Some(&SqlState::T_R_SERIALIZATION_FAILURE) {
        SetupAttemptError::SerializationFailure
    } else if error.code() == Some(&SqlState::INSUFFICIENT_PRIVILEGE) {
        setup_error(ExtensionSetupErrorKind::PermissionDenied)
    } else {
        setup_error(ExtensionSetupErrorKind::Database)
    }
}

#[allow(clippy::needless_pass_by_value)]
fn map_database_or(error: postgres::Error, fallback: ExtensionSetupErrorKind) -> SetupAttemptError {
    if error.code() == Some(&SqlState::T_R_SERIALIZATION_FAILURE) {
        SetupAttemptError::SerializationFailure
    } else if error.code() == Some(&SqlState::INSUFFICIENT_PRIVILEGE) {
        setup_error(ExtensionSetupErrorKind::PermissionDenied)
    } else {
        setup_error(fallback)
    }
}
