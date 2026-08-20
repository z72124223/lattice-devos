use std::error::Error;
use std::fmt;

use lattice_contracts::ContentDigest;
use postgres::{Client, GenericClient, IsolationLevel};
use sha2::{Digest, Sha256};

use crate::{
    ARTIFACT_EXTENSION_ID, ARTIFACT_EXTENSION_SQL, ExtensionManifestEvidence, digest_bytes,
    verify_embedded_extension_manifest,
};

const ARTIFACT_EXTENSION_ADVISORY_LOCK: i64 = 0x4c41_5441_5254_4631;
const CURRENT_GLOBAL_SCHEMA_VERSION: i16 = 5;
const CURRENT_MEMORY_SCHEMA_VERSION: i16 = 3;
const RELATION_PROFILE_SQL: &str = "SELECT pg_catalog.jsonb_build_array(\
    n.nspname,c.relname,c.relkind::text,o.rolname,c.relpersistence::text,c.relrowsecurity,\
    c.relforcerowsecurity,c.relhassubclass,c.relispartition,c.relreplident::text,\
    COALESCE(pg_catalog.array_to_string(c.reloptions,','),'<NULL>'),\
    COALESCE(pg_catalog.obj_description(c.oid,'pg_class'),'<NULL>'))::text \
    FROM pg_catalog.pg_class c \
    JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
    JOIN pg_catalog.pg_roles o ON o.oid=c.relowner \
    WHERE n.nspname='artifact_store' AND c.relkind IN ('r','p') \
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
    WHERE n.nspname='artifact_store' AND c.relkind IN ('r','p') AND a.attnum>0 \
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
    WHERE n.nspname='artifact_store' ORDER BY n.nspname,c.relname,con.conname";
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
    WHERE n.nspname='artifact_store' ORDER BY n.nspname,t.relname,ix.relname";
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
    WHERE n.nspname='artifact_store' \
    ORDER BY n.nspname,p.proname,pg_catalog.pg_get_function_identity_arguments(p.oid)";
const SCHEMA_ACL_PROFILE_SQL: &str = "SELECT pg_catalog.jsonb_build_array(\
    n.nspname,o.rolname,COALESCE(g.rolname,'PUBLIC'),r.rolname,a.privilege_type,a.is_grantable)::text \
    FROM pg_catalog.pg_namespace n JOIN pg_catalog.pg_roles o ON o.oid=n.nspowner \
    CROSS JOIN LATERAL pg_catalog.aclexplode(COALESCE(n.nspacl,pg_catalog.acldefault('n',n.nspowner))) a \
    LEFT JOIN pg_catalog.pg_roles g ON g.oid=a.grantee JOIN pg_catalog.pg_roles r ON r.oid=a.grantor \
    WHERE n.nspname='artifact_store' ORDER BY n.nspname,o.rolname,COALESCE(g.rolname,'PUBLIC'),\
    r.rolname,a.privilege_type,a.is_grantable";
const TABLE_ACL_PROFILE_SQL: &str = "SELECT pg_catalog.jsonb_build_array(\
    n.nspname,c.relname,o.rolname,COALESCE(g.rolname,'PUBLIC'),r.rolname,a.privilege_type,\
    a.is_grantable)::text FROM pg_catalog.pg_class c \
    JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
    JOIN pg_catalog.pg_roles o ON o.oid=c.relowner \
    CROSS JOIN LATERAL pg_catalog.aclexplode(COALESCE(c.relacl,pg_catalog.acldefault('r',c.relowner))) a \
    LEFT JOIN pg_catalog.pg_roles g ON g.oid=a.grantee JOIN pg_catalog.pg_roles r ON r.oid=a.grantor \
    WHERE n.nspname='artifact_store' AND c.relkind IN ('r','p') \
    ORDER BY n.nspname,c.relname,o.rolname,COALESCE(g.rolname,'PUBLIC'),r.rolname,\
    a.privilege_type,a.is_grantable";
const FUNCTION_ACL_PROFILE_SQL: &str = "SELECT pg_catalog.jsonb_build_array(\
    n.nspname,p.proname,pg_catalog.pg_get_function_identity_arguments(p.oid),o.rolname,\
    COALESCE(g.rolname,'PUBLIC'),r.rolname,a.privilege_type,a.is_grantable)::text \
    FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
    JOIN pg_catalog.pg_roles o ON o.oid=p.proowner \
    CROSS JOIN LATERAL pg_catalog.aclexplode(COALESCE(p.proacl,pg_catalog.acldefault('f',p.proowner))) a \
    LEFT JOIN pg_catalog.pg_roles g ON g.oid=a.grantee JOIN pg_catalog.pg_roles r ON r.oid=a.grantor \
    WHERE n.nspname='artifact_store' ORDER BY n.nspname,p.proname,\
    pg_catalog.pg_get_function_identity_arguments(p.oid),o.rolname,COALESCE(g.rolname,'PUBLIC'),\
    r.rolname,a.privilege_type,a.is_grantable";
const COLUMN_ACL_PROFILE_SQL: &str = "SELECT pg_catalog.jsonb_build_array(\
    n.nspname,c.relname,a.attnum,a.attname,COALESCE(g.rolname,'PUBLIC'),r.rolname,x.privilege_type,\
    x.is_grantable)::text FROM pg_catalog.pg_class c \
    JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
    JOIN pg_catalog.pg_attribute a ON a.attrelid=c.oid \
    CROSS JOIN LATERAL pg_catalog.aclexplode(a.attacl) x \
    LEFT JOIN pg_catalog.pg_roles g ON g.oid=x.grantee JOIN pg_catalog.pg_roles r ON r.oid=x.grantor \
    WHERE n.nspname='artifact_store' AND c.relkind IN ('r','p') AND a.attnum>0 \
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
    WHERE n.nspname='artifact_store' ORDER BY n.nspname,t.typname";

const CATALOG_SIGNATURE_DOMAIN: &[u8] = b"LATTICE_POSTGRES_CATALOG_SIGNATURE_V1\0";
const EXPECTED_CATALOG_PROFILES: [(&str, usize, &str); 10] = [
    (
        RELATION_PROFILE_SQL,
        4,
        "f31b9f4b35f19639fb39f662231e72819c52d46b619dbc146cac901df0b5103b",
    ),
    (
        COLUMN_PROFILE_SQL,
        40,
        "3541c5de8cc454074fca11b46991d07af96e1ad5e8cefd866bc8dd1b1cdcc15f",
    ),
    (
        CONSTRAINT_PROFILE_SQL,
        39,
        "0d1ad8708e7923f5ce1a0ec171f4a14afeaad77f098989a350ba988b26dc7bae",
    ),
    (
        INDEX_PROFILE_SQL,
        5,
        "7839cf0ff950a85bf7156861a36f02be1c52f90f0f88907614fc1ec013484c65",
    ),
    (
        FUNCTION_PROFILE_SQL,
        3,
        "f046b69013a4c0ce9d15b88508e8292a4eb946697b475d06636225215512757c",
    ),
    (
        SCHEMA_ACL_PROFILE_SQL,
        3,
        "23afd513002fca8f73002c38d9d780a5bcbb45320dd969753c99196d9ca87014",
    ),
    (
        TABLE_ACL_PROFILE_SQL,
        32,
        "0f2ab3eedd7b720c8ee492ec4ccbc0f71c9ec68bb7534eda15bfc10707ceb8e7",
    ),
    (
        FUNCTION_ACL_PROFILE_SQL,
        6,
        "2cc4e628e7dec7ec6e2927508ce10e87e8a91a32287283e00ca0bf0d25360694",
    ),
    (
        COLUMN_ACL_PROFILE_SQL,
        0,
        "a7ccfc938fbf121a9b807070f69bd5b851be6aa89a8261043ef07336ea7b8dbd",
    ),
    (
        TYPE_PROFILE_SQL,
        8,
        "7a51797ea9f81cb178fd05893be18682f1c5eef206b345c6cfee1ec9f4c3cb8e",
    ),
];

const EXPECTED_RELATIONS: [&str; 4] = [
    "artifact_extension_identity",
    "artifact_extension_ledger",
    "artifact_store_head",
    "artifact_store_transition",
];
const EXPECTED_FUNCTIONS: [(&str, &str); 3] = [
    (
        "artifact_store_commit_snapshot_v1",
        "text, bytea, bytea, bytea, bytea, bytea, bytea, text, text, text, text, text",
    ),
    (
        "artifact_store_load_current_v1",
        "text, text, text, text, text, text",
    ),
    (
        "artifact_store_load_for_update_v1",
        "text, text, text, text, text, text",
    ),
];

/// Prevalidated physical database/profile identity. Connection strings and
/// credentials are deliberately absent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExtensionTarget {
    database_name: String,
    database_identity_digest: ContentDigest,
    global_manifest_digest: ContentDigest,
    memory_manifest_digest: ContentDigest,
}

impl ExtensionTarget {
    /// Constructs one exact accepted target profile.
    ///
    /// # Errors
    ///
    /// Rejects malformed database names.
    pub fn new(
        database_name: impl Into<String>,
        database_identity_digest: ContentDigest,
        global_manifest_digest: ContentDigest,
        memory_manifest_digest: ContentDigest,
    ) -> Result<Self, SetupError> {
        let database_name = database_name.into();
        if database_name.is_empty()
            || database_name.len() > 63
            || !database_name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        {
            return Err(SetupError::new(SetupErrorKind::InvalidTarget));
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

/// Closed setup failure kind.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SetupErrorKind {
    InvalidTarget,
    EmbeddedManifest,
    Database,
    FoundationMismatch,
    InstallSchema,
    IdentityRecord,
    ProfileCollision,
    SerializationExhausted,
    CommitOutcomeUnknown,
}

/// Redacted extension setup failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SetupError {
    kind: SetupErrorKind,
}

impl SetupError {
    #[must_use]
    pub const fn new(kind: SetupErrorKind) -> Self {
        Self { kind }
    }

    #[must_use]
    pub const fn kind(self) -> SetupErrorKind {
        self.kind
    }

    #[must_use]
    pub const fn code(self) -> &'static str {
        match self.kind {
            SetupErrorKind::InvalidTarget => "ARTIFACT_EXTENSION_INVALID_TARGET",
            SetupErrorKind::EmbeddedManifest => "ARTIFACT_EXTENSION_MANIFEST_INVALID",
            SetupErrorKind::Database => "ARTIFACT_EXTENSION_DATABASE_UNAVAILABLE",
            SetupErrorKind::FoundationMismatch => "ARTIFACT_EXTENSION_FOUNDATION_MISMATCH",
            SetupErrorKind::InstallSchema => "ARTIFACT_EXTENSION_INSTALL_SCHEMA_REJECTED",
            SetupErrorKind::IdentityRecord => "ARTIFACT_EXTENSION_IDENTITY_RECORD_REJECTED",
            SetupErrorKind::ProfileCollision => "ARTIFACT_EXTENSION_PROFILE_COLLISION",
            SetupErrorKind::SerializationExhausted => "ARTIFACT_EXTENSION_SERIALIZATION_EXHAUSTED",
            SetupErrorKind::CommitOutcomeUnknown => "ARTIFACT_EXTENSION_COMMIT_OUTCOME_UNKNOWN",
        }
    }
}

impl fmt::Display for SetupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl Error for SetupError {}

/// Installs a fresh v1 extension or verifies an exact-current v1 profile.
/// Partial or drifted state is never repaired.
///
/// # Errors
///
/// Returns a closed setup failure and never includes SQL, DSN, credentials, or
/// stored bytes.
pub fn install_or_verify(
    client: &mut Client,
    target: &ExtensionTarget,
) -> Result<ExtensionManifestEvidence, SetupError> {
    let manifest = verify_embedded_extension_manifest()?;
    let mut transaction = client
        .build_transaction()
        .isolation_level(IsolationLevel::Serializable)
        .start()
        .map_err(|_| SetupError::new(SetupErrorKind::Database))?;
    transaction
        .batch_execute("SET LOCAL ROLE lattice_migrator; SET LOCAL synchronous_commit = on")
        .map_err(|_| SetupError::new(SetupErrorKind::FoundationMismatch))?;
    transaction
        .query_one(
            "SELECT pg_catalog.pg_advisory_xact_lock($1)",
            &[&ARTIFACT_EXTENSION_ADVISORY_LOCK],
        )
        .map_err(|_| SetupError::new(SetupErrorKind::ProfileCollision))?;
    let foundation = verify_foundation(&mut transaction, target)?;
    let schema_exists: bool = transaction
        .query_one(
            "SELECT pg_catalog.to_regnamespace('artifact_store') IS NOT NULL",
            &[],
        )
        .and_then(|row| row.try_get(0))
        .map_err(|_| SetupError::new(SetupErrorKind::Database))?;
    if schema_exists {
        verify_current_profile(&mut transaction, target, &manifest, &foundation)?;
    } else {
        transaction
            .batch_execute(ARTIFACT_EXTENSION_SQL)
            .map_err(|_| SetupError::new(SetupErrorKind::InstallSchema))?;
        record_identity(&mut transaction, target, &manifest, &foundation)?;
        verify_current_profile(&mut transaction, target, &manifest, &foundation)?;
    }
    transaction
        .commit()
        .map_err(|_| SetupError::new(SetupErrorKind::CommitOutcomeUnknown))?;
    Ok(manifest)
}

fn catalog_signature(rows: &[String]) -> Result<String, SetupError> {
    let mut framed = Vec::with_capacity(
        CATALOG_SIGNATURE_DOMAIN.len() + 8 + rows.iter().map(|row| row.len() + 8).sum::<usize>(),
    );
    framed.extend_from_slice(CATALOG_SIGNATURE_DOMAIN);
    framed.extend_from_slice(
        &u64::try_from(rows.len())
            .map_err(|_| profile())?
            .to_be_bytes(),
    );
    for row in rows {
        framed.extend_from_slice(
            &u64::try_from(row.len())
                .map_err(|_| profile())?
                .to_be_bytes(),
        );
        framed.extend_from_slice(row.as_bytes());
    }
    let bytes = Sha256::digest(&framed);
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}").map_err(|_| profile())?;
    }
    Ok(output)
}

fn verify_catalog_profile<C: GenericClient>(
    client: &mut C,
    query: &str,
    expected_rows: usize,
    expected_signature: &str,
) -> Result<(), SetupError> {
    let rows = client
        .query(query, &[])
        .map_err(|_| profile())?
        .into_iter()
        .map(|row| row.try_get::<_, String>(0).map_err(|_| profile()))
        .collect::<Result<Vec<_>, _>>()?;
    if rows.len() != expected_rows || catalog_signature(&rows)? != expected_signature {
        return Err(profile());
    }
    Ok(())
}

struct FoundationEvidence {
    database_uuid: String,
}

fn verify_foundation<C: GenericClient>(
    client: &mut C,
    target: &ExtensionTarget,
) -> Result<FoundationEvidence, SetupError> {
    let row = client
        .query_one(
            "SELECT pg_catalog.current_database()::text,d.database_uuid::text,\
                    pg_catalog.btrim(m.database_identity_sha256)::text,\
                    c.current_schema_version,pg_catalog.btrim(c.manifest_sha256)::text,\
                    m.extension_schema_version,pg_catalog.btrim(m.extension_manifest_sha256)::text \
               FROM ONLY control.database_identity d \
               CROSS JOIN ONLY control.schema_compatibility c \
               CROSS JOIN ONLY memory.codebase_memory_extension_identity m \
              WHERE d.singleton AND c.singleton AND m.singleton AND m.database_uuid=d.database_uuid",
            &[],
        )
        .map_err(|_| SetupError::new(SetupErrorKind::FoundationMismatch))?;
    let database_name: String = row.try_get(0).map_err(|_| foundation())?;
    let database_uuid: String = row.try_get(1).map_err(|_| foundation())?;
    let identity: String = row.try_get(2).map_err(|_| foundation())?;
    let global_version: i16 = row.try_get(3).map_err(|_| foundation())?;
    let global_manifest: String = row.try_get(4).map_err(|_| foundation())?;
    let memory_version: i16 = row.try_get(5).map_err(|_| foundation())?;
    let memory_manifest: String = row.try_get(6).map_err(|_| foundation())?;
    if database_name != target.database_name
        || identity != target.database_identity_digest.as_str()
        || global_version != CURRENT_GLOBAL_SCHEMA_VERSION
        || global_manifest != target.global_manifest_digest.as_str()
        || memory_version != CURRENT_MEMORY_SCHEMA_VERSION
        || memory_manifest != target.memory_manifest_digest.as_str()
    {
        return Err(foundation());
    }
    Ok(FoundationEvidence { database_uuid })
}

fn record_identity<C: GenericClient>(
    client: &mut C,
    target: &ExtensionTarget,
    manifest: &ExtensionManifestEvidence,
    foundation: &FoundationEvidence,
) -> Result<(), SetupError> {
    let sql = digest_bytes(manifest.sql_sha256())?;
    let manifest_digest = digest_bytes(manifest.manifest_sha256())?;
    let parameters: [&(dyn postgres::types::ToSql + Sync); 8] = [
        &sql,
        &manifest_digest,
        &foundation.database_uuid,
        &target.database_identity_digest.as_str(),
        &CURRENT_GLOBAL_SCHEMA_VERSION,
        &target.global_manifest_digest.as_str(),
        &CURRENT_MEMORY_SCHEMA_VERSION,
        &target.memory_manifest_digest.as_str(),
    ];
    client
        .execute(
            "INSERT INTO artifact_store.artifact_extension_identity(\
                singleton,extension_id,schema_version,sql_sha256,manifest_sha256,database_uuid,\
                database_identity_sha256,global_schema_version,global_manifest_sha256,\
                required_memory_schema_version,required_memory_manifest_sha256) \
             VALUES(true,'lattice-postgres-artifact-store',1,$1,$2,$3::text::uuid,$4,$5,$6,$7,$8)",
            &parameters,
        )
        .map_err(|_| SetupError::new(SetupErrorKind::IdentityRecord))?;
    client
        .execute(
            "INSERT INTO artifact_store.artifact_extension_ledger(\
                ordinal,event_type,extension_id,schema_version,sql_sha256,manifest_sha256,\
                database_uuid,database_identity_sha256,global_schema_version,global_manifest_sha256,\
                required_memory_schema_version,required_memory_manifest_sha256) \
             VALUES(1,'INSTALLED','lattice-postgres-artifact-store',1,$1,$2,$3::text::uuid,$4,$5,$6,$7,$8)",
            &parameters,
        )
        .map_err(|_| SetupError::new(SetupErrorKind::ProfileCollision))?;
    Ok(())
}

fn verify_current_profile<C: GenericClient>(
    client: &mut C,
    target: &ExtensionTarget,
    manifest: &ExtensionManifestEvidence,
    foundation: &FoundationEvidence,
) -> Result<(), SetupError> {
    verify_catalog_shape(client)?;
    let sql = digest_bytes(manifest.sql_sha256())?;
    let manifest_digest = digest_bytes(manifest.manifest_sha256())?;
    let identity = client
        .query_one(
            "SELECT extension_id,schema_version,sql_sha256,manifest_sha256,database_uuid::text,\
                    pg_catalog.btrim(database_identity_sha256)::text,global_schema_version,\
                    pg_catalog.btrim(global_manifest_sha256)::text,required_memory_schema_version,\
                    pg_catalog.btrim(required_memory_manifest_sha256)::text,\
                    (SELECT pg_catalog.count(*) FROM ONLY artifact_store.artifact_extension_identity),\
                    (SELECT pg_catalog.count(*) FROM ONLY artifact_store.artifact_extension_ledger),\
                    (SELECT pg_catalog.count(*) FROM ONLY artifact_store.artifact_extension_ledger l \
                      WHERE l.ordinal=1 AND l.event_type='INSTALLED' \
                        AND l.extension_id=i.extension_id AND l.schema_version=i.schema_version \
                        AND l.sql_sha256=i.sql_sha256 AND l.manifest_sha256=i.manifest_sha256 \
                        AND l.database_uuid=i.database_uuid \
                        AND l.database_identity_sha256=i.database_identity_sha256 \
                        AND l.global_schema_version=i.global_schema_version \
                        AND l.global_manifest_sha256=i.global_manifest_sha256 \
                        AND l.required_memory_schema_version=i.required_memory_schema_version \
                        AND l.required_memory_manifest_sha256=i.required_memory_manifest_sha256) \
             FROM ONLY artifact_store.artifact_extension_identity i WHERE i.singleton",
            &[],
        )
        .map_err(|_| profile())?;
    let exact = identity.get::<_, String>(0) == ARTIFACT_EXTENSION_ID
        && identity.get::<_, i16>(1) == 1
        && identity.get::<_, Vec<u8>>(2) == sql
        && identity.get::<_, Vec<u8>>(3) == manifest_digest
        && identity.get::<_, String>(4) == foundation.database_uuid
        && identity.get::<_, String>(5) == target.database_identity_digest.as_str()
        && identity.get::<_, i16>(6) == CURRENT_GLOBAL_SCHEMA_VERSION
        && identity.get::<_, String>(7) == target.global_manifest_digest.as_str()
        && identity.get::<_, i16>(8) == CURRENT_MEMORY_SCHEMA_VERSION
        && identity.get::<_, String>(9) == target.memory_manifest_digest.as_str()
        && identity.get::<_, i64>(10) == 1
        && identity.get::<_, i64>(11) == 1
        && identity.get::<_, i64>(12) == 1;
    if !exact {
        return Err(profile());
    }
    Ok(())
}

fn verify_catalog_shape<C: GenericClient>(client: &mut C) -> Result<(), SetupError> {
    let schema_owner = client
        .query_opt(
            "SELECT o.rolname FROM pg_catalog.pg_namespace n \
             JOIN pg_catalog.pg_roles o ON o.oid=n.nspowner \
             WHERE n.nspname='artifact_store'",
            &[],
        )
        .map_err(|_| profile())?
        .map(|row| row.get::<_, String>(0));
    if schema_owner.as_deref() != Some("lattice_migrator") {
        return Err(profile());
    }
    let relations = client
        .query(
            "SELECT c.relname FROM pg_catalog.pg_class c \
             JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
             JOIN pg_catalog.pg_roles o ON o.oid=c.relowner \
             WHERE n.nspname='artifact_store' AND c.relkind='r' \
               AND o.rolname='lattice_migrator' ORDER BY c.relname",
            &[],
        )
        .map_err(|_| profile())?
        .into_iter()
        .map(|row| row.get::<_, String>(0))
        .collect::<Vec<_>>();
    if relations != EXPECTED_RELATIONS {
        return Err(profile());
    }
    let all_relation_count: i64 = client
        .query_one(
            "SELECT pg_catalog.count(*) FROM pg_catalog.pg_class c \
             JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
             WHERE n.nspname='artifact_store' AND c.relkind NOT IN ('i','I')",
            &[],
        )
        .and_then(|row| row.try_get(0))
        .map_err(|_| profile())?;
    if all_relation_count != i64::try_from(EXPECTED_RELATIONS.len()).unwrap_or(-1) {
        return Err(profile());
    }
    let functions = client
        .query(
            "SELECT p.proname,pg_catalog.oidvectortypes(p.proargtypes) \
             FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
             JOIN pg_catalog.pg_roles o ON o.oid=p.proowner \
             WHERE n.nspname='artifact_store' AND o.rolname='lattice_migrator' \
             ORDER BY p.proname,pg_catalog.oidvectortypes(p.proargtypes)",
            &[],
        )
        .map_err(|_| profile())?
        .into_iter()
        .map(|row| (row.get::<_, String>(0), row.get::<_, String>(1)))
        .collect::<Vec<_>>();
    let expected = EXPECTED_FUNCTIONS
        .iter()
        .map(|(name, args)| ((*name).to_owned(), (*args).to_owned()))
        .collect::<Vec<_>>();
    if functions != expected {
        return Err(profile());
    }
    for (query, expected_rows, expected_signature) in EXPECTED_CATALOG_PROFILES {
        verify_catalog_profile(client, query, expected_rows, expected_signature)?;
    }
    verify_namespace_and_effective_acl_closure(client)?;
    Ok(())
}

fn verify_namespace_and_effective_acl_closure<C: GenericClient>(
    client: &mut C,
) -> Result<(), SetupError> {
    let row = client
        .query_one(
            "SELECT \
             (SELECT pg_catalog.count(*) FROM pg_catalog.pg_trigger tr \
               JOIN pg_catalog.pg_class c ON c.oid=tr.tgrelid \
               JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
              WHERE n.nspname='artifact_store' AND tr.tgisinternal \
                AND tr.tgenabled='O' AND tr.tgconstraint<>0), \
             (SELECT pg_catalog.count(*) FROM pg_catalog.pg_trigger tr \
               JOIN pg_catalog.pg_class c ON c.oid=tr.tgrelid \
               JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
              WHERE n.nspname='artifact_store' AND NOT tr.tgisinternal), \
             (SELECT pg_catalog.count(*) FROM pg_catalog.pg_rewrite rw \
               JOIN pg_catalog.pg_class c ON c.oid=rw.ev_class \
               JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
              WHERE n.nspname='artifact_store'), \
             (SELECT pg_catalog.count(*) FROM pg_catalog.pg_policy p \
               JOIN pg_catalog.pg_class c ON c.oid=p.polrelid \
               JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
              WHERE n.nspname='artifact_store'), \
             (SELECT pg_catalog.count(*) FROM pg_catalog.pg_inherits i \
               JOIN pg_catalog.pg_class c ON c.oid=i.inhrelid \
               JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
              WHERE n.nspname='artifact_store'), \
             (SELECT pg_catalog.count(*) FROM pg_catalog.pg_default_acl d \
               JOIN pg_catalog.pg_namespace n ON n.oid=d.defaclnamespace \
              WHERE n.nspname='artifact_store'), \
             ((SELECT pg_catalog.count(*) FROM pg_catalog.pg_collation x \
                JOIN pg_catalog.pg_namespace n ON n.oid=x.collnamespace WHERE n.nspname='artifact_store') + \
              (SELECT pg_catalog.count(*) FROM pg_catalog.pg_conversion x \
                JOIN pg_catalog.pg_namespace n ON n.oid=x.connamespace WHERE n.nspname='artifact_store') + \
              (SELECT pg_catalog.count(*) FROM pg_catalog.pg_operator x \
                JOIN pg_catalog.pg_namespace n ON n.oid=x.oprnamespace WHERE n.nspname='artifact_store') + \
              (SELECT pg_catalog.count(*) FROM pg_catalog.pg_opclass x \
                JOIN pg_catalog.pg_namespace n ON n.oid=x.opcnamespace WHERE n.nspname='artifact_store') + \
              (SELECT pg_catalog.count(*) FROM pg_catalog.pg_opfamily x \
                JOIN pg_catalog.pg_namespace n ON n.oid=x.opfnamespace WHERE n.nspname='artifact_store') + \
              (SELECT pg_catalog.count(*) FROM pg_catalog.pg_statistic_ext x \
                JOIN pg_catalog.pg_namespace n ON n.oid=x.stxnamespace WHERE n.nspname='artifact_store') + \
              (SELECT pg_catalog.count(*) FROM pg_catalog.pg_ts_config x \
                JOIN pg_catalog.pg_namespace n ON n.oid=x.cfgnamespace WHERE n.nspname='artifact_store') + \
              (SELECT pg_catalog.count(*) FROM pg_catalog.pg_ts_dict x \
                JOIN pg_catalog.pg_namespace n ON n.oid=x.dictnamespace WHERE n.nspname='artifact_store') + \
              (SELECT pg_catalog.count(*) FROM pg_catalog.pg_ts_parser x \
                JOIN pg_catalog.pg_namespace n ON n.oid=x.prsnamespace WHERE n.nspname='artifact_store') + \
              (SELECT pg_catalog.count(*) FROM pg_catalog.pg_ts_template x \
                JOIN pg_catalog.pg_namespace n ON n.oid=x.tmplnamespace WHERE n.nspname='artifact_store')), \
             (SELECT pg_catalog.count(*) FROM pg_catalog.pg_class c \
               JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
               CROSS JOIN pg_catalog.pg_roles roles \
              WHERE n.nspname='artifact_store' AND c.relkind IN ('r','p') \
                AND NOT roles.rolsuper AND roles.rolname !~ '^pg_' \
                AND roles.rolname <> 'lattice_migrator' \
                AND pg_catalog.has_table_privilege(roles.rolname,c.oid,\
                  'SELECT,INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER,MAINTAIN')), \
             (SELECT pg_catalog.count(*) FROM pg_catalog.pg_proc p \
               JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
               CROSS JOIN pg_catalog.pg_roles roles \
              WHERE n.nspname='artifact_store' \
                AND NOT roles.rolsuper AND roles.rolname !~ '^pg_' \
                AND roles.rolname NOT IN ('lattice_migrator','lattice_runtime') \
                AND pg_catalog.has_function_privilege(roles.rolname,p.oid,'EXECUTE')), \
             (SELECT pg_catalog.count(*) FROM pg_catalog.pg_proc p \
               JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
              WHERE n.nspname='artifact_store' \
                AND NOT pg_catalog.has_function_privilege('lattice_runtime',p.oid,'EXECUTE')), \
             pg_catalog.has_schema_privilege('lattice_runtime','artifact_store','USAGE'), \
             (pg_catalog.has_schema_privilege('lattice_runtime','artifact_store','CREATE') \
               OR EXISTS (SELECT 1 FROM pg_catalog.pg_roles roles \
                  WHERE NOT roles.rolsuper AND roles.rolname !~ '^pg_' \
                    AND roles.rolname NOT IN ('lattice_migrator','lattice_runtime') \
                    AND (pg_catalog.has_schema_privilege(roles.rolname,'artifact_store','USAGE') \
                      OR pg_catalog.has_schema_privilege(roles.rolname,'artifact_store','CREATE'))))",
            &[],
        )
        .map_err(|_| profile())?;
    for (index, expected) in [4_i64, 0, 0, 0, 0, 0, 0, 0, 0, 0].into_iter().enumerate() {
        if row.try_get::<_, i64>(index).map_err(|_| profile())? != expected {
            return Err(profile());
        }
    }
    if !row.try_get::<_, bool>(10).map_err(|_| profile())?
        || row.try_get::<_, bool>(11).map_err(|_| profile())?
    {
        return Err(profile());
    }
    Ok(())
}

const fn foundation() -> SetupError {
    SetupError::new(SetupErrorKind::FoundationMismatch)
}

const fn profile() -> SetupError {
    SetupError::new(SetupErrorKind::ProfileCollision)
}
