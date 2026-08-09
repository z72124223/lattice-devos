use std::error::Error;
use std::fmt;

use lattice_contracts::ContentDigest;
use postgres::{Client, GenericClient, IsolationLevel, Transaction};

use crate::{
    ExtensionManifestEvidence, WRITER_LEASE_EXTENSION_ID, WRITER_LEASE_EXTENSION_PATH,
    WRITER_LEASE_EXTENSION_SCHEMA_VERSION, sha256_hex, verify_embedded_extension_manifest,
};

const CATALOG_SIGNATURE_DOMAIN: &[u8] = b"LATTICE_POSTGRES_CATALOG_SIGNATURE_V1\0";
const RELATION_PROFILE_SQL: &str = "SELECT pg_catalog.jsonb_build_array(\
    n.nspname,c.relname,c.relkind::text,o.rolname,c.relpersistence::text,c.relrowsecurity,\
    c.relforcerowsecurity,c.relhassubclass,c.relispartition,c.relreplident::text,\
    COALESCE(pg_catalog.array_to_string(c.reloptions,','),'<NULL>'),\
    COALESCE(pg_catalog.obj_description(c.oid,'pg_class'),'<NULL>'))::text \
    FROM pg_catalog.pg_class c \
    JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
    JOIN pg_catalog.pg_roles o ON o.oid=c.relowner \
    WHERE n.nspname='writer_lease' AND c.relkind IN ('r','p') \
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
    WHERE n.nspname='writer_lease' AND c.relkind IN ('r','p') AND a.attnum>0 \
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
    WHERE n.nspname='writer_lease' ORDER BY n.nspname,c.relname,con.conname";
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
    WHERE n.nspname='writer_lease' ORDER BY n.nspname,t.relname,ix.relname";
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
    WHERE n.nspname='writer_lease' \
    ORDER BY n.nspname,p.proname,pg_catalog.pg_get_function_identity_arguments(p.oid)";
const SCHEMA_ACL_PROFILE_SQL: &str = "SELECT pg_catalog.jsonb_build_array(\
    n.nspname,o.rolname,COALESCE(g.rolname,'PUBLIC'),r.rolname,a.privilege_type,a.is_grantable)::text \
    FROM pg_catalog.pg_namespace n JOIN pg_catalog.pg_roles o ON o.oid=n.nspowner \
    CROSS JOIN LATERAL pg_catalog.aclexplode(COALESCE(n.nspacl,pg_catalog.acldefault('n',n.nspowner))) a \
    LEFT JOIN pg_catalog.pg_roles g ON g.oid=a.grantee JOIN pg_catalog.pg_roles r ON r.oid=a.grantor \
    WHERE n.nspname='writer_lease' ORDER BY n.nspname,o.rolname,COALESCE(g.rolname,'PUBLIC'),\
    r.rolname,a.privilege_type,a.is_grantable";
const TABLE_ACL_PROFILE_SQL: &str = "SELECT pg_catalog.jsonb_build_array(\
    n.nspname,c.relname,o.rolname,COALESCE(g.rolname,'PUBLIC'),r.rolname,a.privilege_type,\
    a.is_grantable)::text FROM pg_catalog.pg_class c \
    JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
    JOIN pg_catalog.pg_roles o ON o.oid=c.relowner \
    CROSS JOIN LATERAL pg_catalog.aclexplode(COALESCE(c.relacl,pg_catalog.acldefault('r',c.relowner))) a \
    LEFT JOIN pg_catalog.pg_roles g ON g.oid=a.grantee JOIN pg_catalog.pg_roles r ON r.oid=a.grantor \
    WHERE n.nspname='writer_lease' AND c.relkind IN ('r','p') \
    ORDER BY n.nspname,c.relname,o.rolname,COALESCE(g.rolname,'PUBLIC'),r.rolname,\
    a.privilege_type,a.is_grantable";
const FUNCTION_ACL_PROFILE_SQL: &str = "SELECT pg_catalog.jsonb_build_array(\
    n.nspname,p.proname,pg_catalog.pg_get_function_identity_arguments(p.oid),o.rolname,\
    COALESCE(g.rolname,'PUBLIC'),r.rolname,a.privilege_type,a.is_grantable)::text \
    FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
    JOIN pg_catalog.pg_roles o ON o.oid=p.proowner \
    CROSS JOIN LATERAL pg_catalog.aclexplode(COALESCE(p.proacl,pg_catalog.acldefault('f',p.proowner))) a \
    LEFT JOIN pg_catalog.pg_roles g ON g.oid=a.grantee JOIN pg_catalog.pg_roles r ON r.oid=a.grantor \
    WHERE n.nspname='writer_lease' ORDER BY n.nspname,p.proname,\
    pg_catalog.pg_get_function_identity_arguments(p.oid),o.rolname,COALESCE(g.rolname,'PUBLIC'),\
    r.rolname,a.privilege_type,a.is_grantable";
const COLUMN_ACL_PROFILE_SQL: &str = "SELECT pg_catalog.jsonb_build_array(\
    n.nspname,c.relname,a.attnum,a.attname,COALESCE(g.rolname,'PUBLIC'),r.rolname,x.privilege_type,\
    x.is_grantable)::text FROM pg_catalog.pg_class c \
    JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
    JOIN pg_catalog.pg_attribute a ON a.attrelid=c.oid \
    CROSS JOIN LATERAL pg_catalog.aclexplode(a.attacl) x \
    LEFT JOIN pg_catalog.pg_roles g ON g.oid=x.grantee JOIN pg_catalog.pg_roles r ON r.oid=x.grantor \
    WHERE n.nspname='writer_lease' AND c.relkind IN ('r','p') AND a.attnum>0 \
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
    WHERE n.nspname='writer_lease' ORDER BY n.nspname,t.typname";

const EXPECTED_CATALOG_PROFILES: [(&str, usize, &str); 10] = [
    (
        RELATION_PROFILE_SQL,
        5,
        "c20048700ff120bc6488c4608eb79df36d329ad817f2b0e45e47020d867b8251",
    ),
    (
        COLUMN_PROFILE_SQL,
        73,
        "560e93c2a765db0024c0e74d25a51b90cfc72b204601139de8fdb688d48c0610",
    ),
    (
        CONSTRAINT_PROFILE_SQL,
        28,
        "3deab2f6ee712692d5ec75682030462ebd4dd4712ff26e40bf323abdd683c5d3",
    ),
    (
        INDEX_PROFILE_SQL,
        9,
        "a30a0abfca0a824d75f2f29eb85a8424af35da485f1fef1bc5f852b9be7151a4",
    ),
    (
        FUNCTION_PROFILE_SQL,
        7,
        "638941fbd31edbec9d9f860974aac280845063693acc00bd5f6f8c3aa650adc9",
    ),
    (
        SCHEMA_ACL_PROFILE_SQL,
        3,
        "a2e1be8a403a96b679c18ddfa75e476fa1d6ceeccc1ccf62ff6424b2c259ef7b",
    ),
    (
        TABLE_ACL_PROFILE_SQL,
        40,
        "b99ef0c0ea5b550ae5e805d29b0020e31c1800a016b0de82cda566d7b25e9569",
    ),
    (
        FUNCTION_ACL_PROFILE_SQL,
        14,
        "4e1a2ba0c5abcfe928b66b839166f2bebeecca73a0514f02344c9bbb695b0c44",
    ),
    (
        COLUMN_ACL_PROFILE_SQL,
        0,
        "a7ccfc938fbf121a9b807070f69bd5b851be6aa89a8261043ef07336ea7b8dbd",
    ),
    (
        TYPE_PROFILE_SQL,
        10,
        "1d6642e77600a93da5b00dda0ee64c15474b4ca2741c51ca760597e7f90ac003",
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
    pub(crate) const fn new(kind: ExtensionSetupErrorKind) -> Self {
        Self { kind }
    }

    #[must_use]
    pub const fn kind(self) -> ExtensionSetupErrorKind {
        self.kind
    }

    #[must_use]
    pub const fn code(self) -> &'static str {
        match self.kind {
            ExtensionSetupErrorKind::InvalidTarget => "WRITER_LEASE_EXTENSION_INVALID_TARGET",
            ExtensionSetupErrorKind::ManifestMismatch => "WRITER_LEASE_EXTENSION_MANIFEST_MISMATCH",
            ExtensionSetupErrorKind::UnsupportedFoundation => {
                "WRITER_LEASE_EXTENSION_FOUNDATION_UNSUPPORTED"
            }
            ExtensionSetupErrorKind::PartialOrCollidingProfile => {
                "WRITER_LEASE_EXTENSION_PROFILE_COLLISION"
            }
            ExtensionSetupErrorKind::PermissionDenied => "WRITER_LEASE_EXTENSION_PERMISSION_DENIED",
            ExtensionSetupErrorKind::Database => "WRITER_LEASE_EXTENSION_DATABASE_FAILURE",
        }
    }
}

impl fmt::Display for ExtensionSetupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl Error for ExtensionSetupError {}

/// Installs the one embedded profile or verifies an exact current no-op.
///
/// # Errors
///
/// Fails closed for a foundation mismatch, partial/colliding profile,
/// permission drift, or database ambiguity.
pub fn apply_extension(
    client: &mut Client,
    target: &ExtensionTarget,
) -> Result<ExtensionApplyOutcome, ExtensionSetupError> {
    let manifest = verify_embedded_extension_manifest()
        .map_err(|_| ExtensionSetupError::new(ExtensionSetupErrorKind::ManifestMismatch))?;
    let mut transaction = client
        .build_transaction()
        .isolation_level(IsolationLevel::Serializable)
        .start()
        .map_err(map_database)?;
    enter_migrator(&mut transaction)?;
    let foundation = verify_foundation(&mut transaction, target)?;
    let schema_exists: bool = transaction
        .query_one(
            "SELECT pg_catalog.to_regnamespace('writer_lease') IS NOT NULL",
            &[],
        )
        .map_err(map_database)?
        .try_get(0)
        .map_err(map_database)?;
    if schema_exists {
        verify_profile(
            &mut transaction,
            target,
            &manifest,
            &foundation.database_uuid,
        )?;
        transaction.commit().map_err(map_database)?;
        return Ok(ExtensionApplyOutcome::AlreadyCurrent);
    }

    let sql = std::str::from_utf8(manifest.bytes())
        .map_err(|_| ExtensionSetupError::new(ExtensionSetupErrorKind::ManifestMismatch))?;
    transaction.batch_execute(sql).map_err(map_database)?;
    let extension_schema_version = i16::try_from(WRITER_LEASE_EXTENSION_SCHEMA_VERSION)
        .map_err(|_| ExtensionSetupError::new(ExtensionSetupErrorKind::ManifestMismatch))?;
    transaction
        .execute(
            "INSERT INTO writer_lease.writer_lease_extension_identity (\
                 singleton, extension_id, extension_schema_version, extension_path, \
                 extension_sql_sha256, extension_manifest_sha256, database_uuid, \
                 database_identity_sha256, global_schema_version, global_manifest_sha256, \
                 required_memory_schema_version, required_memory_manifest_sha256\
             ) VALUES (true, $1, $2, $3, $4, $5, $6::text::uuid, $7, 3, $8, 2, $9)",
            &[
                &WRITER_LEASE_EXTENSION_ID,
                &extension_schema_version,
                &WRITER_LEASE_EXTENSION_PATH,
                &manifest.sql_sha256().as_str(),
                &manifest.manifest_sha256().as_str(),
                &foundation.database_uuid,
                &target.database_identity_digest().as_str(),
                &target.global_manifest_digest().as_str(),
                &target.memory_manifest_digest().as_str(),
            ],
        )
        .map_err(map_database)?;
    transaction
        .execute(
            "INSERT INTO writer_lease.writer_lease_extension_ledger (\
                 ledger_ordinal, singleton, extension_id, extension_schema_version, \
                 extension_sql_sha256, extension_manifest_sha256, database_uuid, \
                 database_identity_sha256, global_schema_version, global_manifest_sha256, \
                 required_memory_schema_version, required_memory_manifest_sha256, event_kind\
             ) SELECT 1, singleton, extension_id, extension_schema_version, \
                      extension_sql_sha256, extension_manifest_sha256, database_uuid, \
                      database_identity_sha256, global_schema_version, global_manifest_sha256, \
                      required_memory_schema_version, required_memory_manifest_sha256, 'INSTALLED' \
                 FROM ONLY writer_lease.writer_lease_extension_identity WHERE singleton",
            &[],
        )
        .map_err(map_database)?;
    verify_profile(
        &mut transaction,
        target,
        &manifest,
        &foundation.database_uuid,
    )?;
    transaction.commit().map_err(map_database)?;
    Ok(ExtensionApplyOutcome::Installed)
}

/// Verifies the exact catalog, ACL, and identity profile without mutation.
///
/// # Errors
///
/// Rejects any partial, extra, substituted, or privilege-drifted profile.
pub fn verify_extension(
    client: &mut Client,
    target: &ExtensionTarget,
) -> Result<(), ExtensionSetupError> {
    let manifest = verify_embedded_extension_manifest()
        .map_err(|_| ExtensionSetupError::new(ExtensionSetupErrorKind::ManifestMismatch))?;
    let mut transaction = client
        .build_transaction()
        .isolation_level(IsolationLevel::RepeatableRead)
        .read_only(true)
        .start()
        .map_err(map_database)?;
    enter_migrator(&mut transaction)?;
    let foundation = verify_foundation(&mut transaction, target)?;
    verify_profile(
        &mut transaction,
        target,
        &manifest,
        &foundation.database_uuid,
    )?;
    transaction.commit().map_err(map_database)
}

struct FoundationEvidence {
    database_uuid: String,
}

fn enter_migrator(transaction: &mut Transaction<'_>) -> Result<(), ExtensionSetupError> {
    transaction
        .batch_execute(
            "SET LOCAL ROLE lattice_migrator; \
             SET LOCAL search_path = pg_catalog; \
             SET LOCAL row_security = on; \
             SET LOCAL lock_timeout = '5s'; \
             SET LOCAL statement_timeout = '30s'; \
             SET LOCAL idle_in_transaction_session_timeout = '30s';",
        )
        .map_err(|_| ExtensionSetupError::new(ExtensionSetupErrorKind::PermissionDenied))
}

fn verify_foundation<C: GenericClient>(
    client: &mut C,
    target: &ExtensionTarget,
) -> Result<FoundationEvidence, ExtensionSetupError> {
    let row = client
        .query_one(
            "SELECT pg_catalog.current_database()::text, d.database_uuid::text, \
                    pg_catalog.btrim(m.database_identity_sha256)::text, c.current_schema_version, \
                    pg_catalog.btrim(c.manifest_sha256)::text, m.extension_schema_version, \
                    pg_catalog.btrim(m.extension_manifest_sha256)::text \
               FROM ONLY control.database_identity AS d \
               CROSS JOIN ONLY control.schema_compatibility AS c \
               CROSS JOIN ONLY memory.codebase_memory_extension_identity AS m \
              WHERE m.singleton AND m.database_uuid = d.database_uuid",
            &[],
        )
        .map_err(|_| ExtensionSetupError::new(ExtensionSetupErrorKind::UnsupportedFoundation))?;
    let database_name: String = row.try_get(0).map_err(map_database)?;
    let database_uuid: String = row.try_get(1).map_err(map_database)?;
    let database_identity: String = row.try_get(2).map_err(map_database)?;
    let global_version: i16 = row.try_get(3).map_err(map_database)?;
    let global_manifest: String = row.try_get(4).map_err(map_database)?;
    let memory_version: i16 = row.try_get(5).map_err(map_database)?;
    let memory_manifest: String = row.try_get(6).map_err(map_database)?;
    if database_name != target.database_name()
        || database_identity != target.database_identity_digest().as_str()
        || global_version != 3
        || global_manifest != target.global_manifest_digest().as_str()
        || memory_version != 2
        || memory_manifest != target.memory_manifest_digest().as_str()
    {
        return Err(ExtensionSetupError::new(
            ExtensionSetupErrorKind::UnsupportedFoundation,
        ));
    }
    Ok(FoundationEvidence { database_uuid })
}

#[allow(clippy::too_many_lines)]
fn verify_profile<C: GenericClient>(
    client: &mut C,
    target: &ExtensionTarget,
    manifest: &ExtensionManifestEvidence,
    database_uuid: &str,
) -> Result<(), ExtensionSetupError> {
    let row = client
        .query_one(
            "SELECT \
               (SELECT pg_catalog.count(*) FROM pg_catalog.pg_namespace AS n \
                JOIN pg_catalog.pg_roles AS r ON r.oid = n.nspowner \
                WHERE n.nspname = 'writer_lease' AND r.rolname = 'lattice_migrator'), \
               (SELECT pg_catalog.count(*) FROM pg_catalog.pg_class AS c \
                JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace \
                WHERE n.nspname = 'writer_lease' AND c.relkind IN ('r','p','v','m','S','f')), \
               (SELECT pg_catalog.count(*) FROM pg_catalog.pg_class AS c \
                JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace \
                JOIN pg_catalog.pg_roles AS r ON r.oid = c.relowner \
                WHERE n.nspname = 'writer_lease' AND c.relkind IN ('r','p','v','m','S','f') \
                  AND r.rolname = 'lattice_migrator'), \
               (SELECT pg_catalog.count(*) FROM pg_catalog.pg_proc AS p \
                JOIN pg_catalog.pg_namespace AS n ON n.oid = p.pronamespace \
                WHERE n.nspname = 'writer_lease'), \
               (SELECT pg_catalog.count(*) FROM pg_catalog.pg_proc AS p \
                JOIN pg_catalog.pg_namespace AS n ON n.oid = p.pronamespace \
                JOIN pg_catalog.pg_roles AS r ON r.oid = p.proowner \
                WHERE n.nspname = 'writer_lease' AND p.prosecdef AND r.rolname = 'lattice_migrator'), \
               (SELECT pg_catalog.count(*) FROM pg_catalog.pg_proc AS p \
                JOIN pg_catalog.pg_namespace AS n ON n.oid = p.pronamespace \
                WHERE n.nspname = 'writer_lease' \
                  AND pg_catalog.has_function_privilege('lattice_runtime', p.oid, 'EXECUTE')), \
               (SELECT pg_catalog.count(*) FROM pg_catalog.pg_proc AS p \
                JOIN pg_catalog.pg_namespace AS n ON n.oid = p.pronamespace \
                CROSS JOIN LATERAL pg_catalog.aclexplode( \
                    COALESCE(p.proacl, pg_catalog.acldefault('f', p.proowner)) \
                ) AS a \
                WHERE n.nspname = 'writer_lease' \
                  AND (a.grantee = 0 \
                    OR (a.grantee <> p.proowner AND a.grantee <> ( \
                        SELECT oid FROM pg_catalog.pg_roles WHERE rolname = 'lattice_runtime' \
                    )) \
                    OR a.privilege_type <> 'EXECUTE' \
                    OR a.is_grantable)), \
               (SELECT pg_catalog.count(*) FROM pg_catalog.pg_class AS c \
                JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace \
                WHERE n.nspname = 'writer_lease' \
                  AND (pg_catalog.has_table_privilege('lattice_runtime', c.oid, 'SELECT') \
                    OR pg_catalog.has_table_privilege('lattice_runtime', c.oid, 'INSERT') \
                    OR pg_catalog.has_table_privilege('lattice_runtime', c.oid, 'UPDATE') \
                    OR pg_catalog.has_table_privilege('lattice_runtime', c.oid, 'DELETE'))), \
               (SELECT pg_catalog.count(*) FROM pg_catalog.pg_constraint AS c \
                JOIN pg_catalog.pg_namespace AS n ON n.oid = c.connamespace \
                WHERE n.nspname = 'writer_lease'), \
               (SELECT pg_catalog.count(*) FROM pg_catalog.pg_class AS c \
                JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace \
                WHERE n.nspname = 'writer_lease' AND c.relkind = 'i'), \
               pg_catalog.has_schema_privilege('lattice_runtime', 'writer_lease', 'USAGE'), \
               pg_catalog.has_schema_privilege('lattice_runtime', 'writer_lease', 'CREATE')",
            &[],
        )
        .map_err(map_database)?;
    let namespace_owner_count: i64 = row.try_get(0).map_err(map_database)?;
    let table_count: i64 = row.try_get(1).map_err(map_database)?;
    let table_owner_count: i64 = row.try_get(2).map_err(map_database)?;
    let function_count: i64 = row.try_get(3).map_err(map_database)?;
    let security_definer_count: i64 = row.try_get(4).map_err(map_database)?;
    let runtime_function_privileges: i64 = row.try_get(5).map_err(map_database)?;
    let unexpected_function_acl: i64 = row.try_get(6).map_err(map_database)?;
    let runtime_table_privileges: i64 = row.try_get(7).map_err(map_database)?;
    let constraint_count: i64 = row.try_get(8).map_err(map_database)?;
    let index_count: i64 = row.try_get(9).map_err(map_database)?;
    let runtime_usage: bool = row.try_get(10).map_err(map_database)?;
    let runtime_create: bool = row.try_get(11).map_err(map_database)?;
    if namespace_owner_count != 1
        || table_count != 5
        || table_owner_count != 5
        || function_count != 7
        || security_definer_count != 7
        || runtime_function_privileges != 7
        || unexpected_function_acl != 0
        || runtime_table_privileges != 0
        || constraint_count != 28
        || index_count != 9
        || !runtime_usage
        || runtime_create
    {
        return Err(ExtensionSetupError::new(
            ExtensionSetupErrorKind::PartialOrCollidingProfile,
        ));
    }

    let names = client
        .query(
            "SELECT c.relname::text FROM pg_catalog.pg_class AS c \
             JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace \
             WHERE n.nspname = 'writer_lease' AND c.relkind IN ('r','p','v','m','S','f') \
             ORDER BY c.relname",
            &[],
        )
        .map_err(map_database)?
        .into_iter()
        .map(|row| row.try_get::<_, String>(0).map_err(map_database))
        .collect::<Result<Vec<_>, _>>()?;
    let expected_tables = [
        "writer_lease_commands",
        "writer_lease_extension_identity",
        "writer_lease_extension_ledger",
        "writer_lease_heads",
        "writer_lease_transitions",
    ];
    if names.iter().map(String::as_str).ne(expected_tables) {
        return Err(ExtensionSetupError::new(
            ExtensionSetupErrorKind::PartialOrCollidingProfile,
        ));
    }

    let function_names = client
        .query(
            "SELECT p.proname::text || '(' || pg_catalog.oidvectortypes(p.proargtypes) \
                    || ')|' || p.provolatile::text || '|' || p.proparallel::text \
               FROM pg_catalog.pg_proc AS p \
             JOIN pg_catalog.pg_namespace AS n ON n.oid = p.pronamespace \
             WHERE n.nspname = 'writer_lease' ORDER BY p.proname",
            &[],
        )
        .map_err(map_database)?
        .into_iter()
        .map(|row| row.try_get::<_, String>(0).map_err(map_database))
        .collect::<Result<Vec<_>, _>>()?;
    let expected_functions = [
        "writer_lease_assert_current_v1(text, text, text, text, bytea, text, text, text, text, bigint, bytea, text, bigint, bigint, bytea)|s|s",
        "writer_lease_bind_runtime_v1(text, bigint, bytea, text, text, text, text, text)|s|s",
        "writer_lease_commit_plan_v1(text, bigint, bytea, bigint, bytea, text, bytea, text, text, bigint, bytea, bytea, bytea, bytea, bigint, bigint, bigint, bytea, text, bytea, text, text, text, bytea, text, text, text, text, bigint, bytea, text, bigint, bigint, text, bigint, text, bytea, bytea, bytea, bytea, bytea, text, text, bytea, bytea, bytea, text, bytea)|v|u",
        "writer_lease_load_commands_v1(text)|s|s",
        "writer_lease_load_current_v1(text)|s|s",
        "writer_lease_load_for_update_v1(text, bytea, bytea, bytea, text)|v|u",
        "writer_lease_load_transitions_v1(text)|s|s",
    ];
    if function_names
        .iter()
        .map(String::as_str)
        .ne(expected_functions)
    {
        return Err(ExtensionSetupError::new(
            ExtensionSetupErrorKind::PartialOrCollidingProfile,
        ));
    }

    for (query, expected_rows, expected_signature) in EXPECTED_CATALOG_PROFILES {
        verify_catalog_profile(client, query, expected_rows, expected_signature)?;
    }
    verify_namespace_and_effective_acl_closure(client)?;

    let identity = client
        .query_one(
            "SELECT i.extension_id::text, i.extension_schema_version, i.extension_path::text, \
                    pg_catalog.btrim(i.extension_sql_sha256)::text, \
                    pg_catalog.btrim(i.extension_manifest_sha256)::text, i.database_uuid::text, \
                    pg_catalog.btrim(i.database_identity_sha256)::text, i.global_schema_version, \
                    pg_catalog.btrim(i.global_manifest_sha256)::text, \
                    i.required_memory_schema_version, \
                    pg_catalog.btrim(i.required_memory_manifest_sha256)::text, \
                    l.ledger_ordinal, l.event_kind::text, \
                    (l.singleton IS NOT DISTINCT FROM i.singleton \
                     AND l.extension_id IS NOT DISTINCT FROM i.extension_id \
                     AND l.extension_schema_version IS NOT DISTINCT FROM i.extension_schema_version \
                     AND l.extension_sql_sha256 IS NOT DISTINCT FROM i.extension_sql_sha256 \
                     AND l.extension_manifest_sha256 IS NOT DISTINCT FROM i.extension_manifest_sha256 \
                     AND l.database_uuid IS NOT DISTINCT FROM i.database_uuid \
                     AND l.database_identity_sha256 IS NOT DISTINCT FROM i.database_identity_sha256 \
                     AND l.global_schema_version IS NOT DISTINCT FROM i.global_schema_version \
                     AND l.global_manifest_sha256 IS NOT DISTINCT FROM i.global_manifest_sha256 \
                     AND l.required_memory_schema_version IS NOT DISTINCT FROM i.required_memory_schema_version \
                     AND l.required_memory_manifest_sha256 IS NOT DISTINCT FROM i.required_memory_manifest_sha256) \
               FROM ONLY writer_lease.writer_lease_extension_identity AS i \
               JOIN ONLY writer_lease.writer_lease_extension_ledger AS l USING (singleton) \
              WHERE i.singleton",
            &[],
        )
        .map_err(map_database)?;
    let extension_id: String = identity.try_get(0).map_err(map_database)?;
    let extension_version: i16 = identity.try_get(1).map_err(map_database)?;
    let extension_path: String = identity.try_get(2).map_err(map_database)?;
    let sql_digest: String = identity.try_get(3).map_err(map_database)?;
    let extension_manifest: String = identity.try_get(4).map_err(map_database)?;
    let observed_database_uuid: String = identity.try_get(5).map_err(map_database)?;
    let database_identity: String = identity.try_get(6).map_err(map_database)?;
    let global_version: i16 = identity.try_get(7).map_err(map_database)?;
    let global_manifest: String = identity.try_get(8).map_err(map_database)?;
    let memory_version: i16 = identity.try_get(9).map_err(map_database)?;
    let memory_manifest: String = identity.try_get(10).map_err(map_database)?;
    let ledger_ordinal: i16 = identity.try_get(11).map_err(map_database)?;
    let ledger_event: String = identity.try_get(12).map_err(map_database)?;
    let ledger_matches: bool = identity.try_get(13).map_err(map_database)?;
    if extension_id != WRITER_LEASE_EXTENSION_ID
        || extension_version != i16::try_from(WRITER_LEASE_EXTENSION_SCHEMA_VERSION).unwrap_or(-1)
        || extension_path != WRITER_LEASE_EXTENSION_PATH
        || sql_digest != manifest.sql_sha256().as_str()
        || extension_manifest != manifest.manifest_sha256().as_str()
        || observed_database_uuid != database_uuid
        || database_identity != target.database_identity_digest().as_str()
        || global_version != 3
        || global_manifest != target.global_manifest_digest().as_str()
        || memory_version != 2
        || memory_manifest != target.memory_manifest_digest().as_str()
        || ledger_ordinal != 1
        || ledger_event != "INSTALLED"
        || !ledger_matches
    {
        return Err(ExtensionSetupError::new(
            ExtensionSetupErrorKind::PartialOrCollidingProfile,
        ));
    }
    Ok(())
}

fn verify_catalog_profile<C: GenericClient>(
    client: &mut C,
    query: &str,
    expected_rows: usize,
    expected_signature: &str,
) -> Result<(), ExtensionSetupError> {
    let rows = client
        .query(query, &[])
        .map_err(map_database)?
        .into_iter()
        .map(|row| row.try_get::<_, String>(0).map_err(map_database))
        .collect::<Result<Vec<_>, _>>()?;
    if rows.len() != expected_rows {
        return Err(ExtensionSetupError::new(
            ExtensionSetupErrorKind::PartialOrCollidingProfile,
        ));
    }
    let mut framed = Vec::with_capacity(
        CATALOG_SIGNATURE_DOMAIN.len() + 8 + rows.iter().map(|row| row.len() + 8).sum::<usize>(),
    );
    framed.extend_from_slice(CATALOG_SIGNATURE_DOMAIN);
    framed.extend_from_slice(
        &u64::try_from(rows.len())
            .map_err(|_| {
                ExtensionSetupError::new(ExtensionSetupErrorKind::PartialOrCollidingProfile)
            })?
            .to_be_bytes(),
    );
    for row in rows {
        framed.extend_from_slice(
            &u64::try_from(row.len())
                .map_err(|_| {
                    ExtensionSetupError::new(ExtensionSetupErrorKind::PartialOrCollidingProfile)
                })?
                .to_be_bytes(),
        );
        framed.extend_from_slice(row.as_bytes());
    }
    if sha256_hex(&framed) != expected_signature {
        return Err(ExtensionSetupError::new(
            ExtensionSetupErrorKind::PartialOrCollidingProfile,
        ));
    }
    Ok(())
}

fn verify_namespace_and_effective_acl_closure<C: GenericClient>(
    client: &mut C,
) -> Result<(), ExtensionSetupError> {
    let row = client
        .query_one(
            "SELECT \
             (SELECT pg_catalog.count(*) FROM pg_catalog.pg_trigger tr \
               JOIN pg_catalog.pg_class c ON c.oid=tr.tgrelid \
               JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
              WHERE n.nspname='writer_lease' AND tr.tgisinternal \
                AND tr.tgenabled='O' AND tr.tgconstraint<>0), \
             (SELECT pg_catalog.count(*) FROM pg_catalog.pg_trigger tr \
               JOIN pg_catalog.pg_class c ON c.oid=tr.tgrelid \
               JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
              WHERE n.nspname='writer_lease' AND NOT tr.tgisinternal), \
             (SELECT pg_catalog.count(*) FROM pg_catalog.pg_rewrite rw \
               JOIN pg_catalog.pg_class c ON c.oid=rw.ev_class \
               JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
              WHERE n.nspname='writer_lease'), \
             (SELECT pg_catalog.count(*) FROM pg_catalog.pg_policy p \
               JOIN pg_catalog.pg_class c ON c.oid=p.polrelid \
               JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
              WHERE n.nspname='writer_lease'), \
             (SELECT pg_catalog.count(*) FROM pg_catalog.pg_inherits i \
               JOIN pg_catalog.pg_class c ON c.oid=i.inhrelid \
               JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
              WHERE n.nspname='writer_lease'), \
             (SELECT pg_catalog.count(*) FROM pg_catalog.pg_default_acl d \
               JOIN pg_catalog.pg_namespace n ON n.oid=d.defaclnamespace \
              WHERE n.nspname='writer_lease'), \
             (SELECT pg_catalog.count(*) FROM pg_catalog.pg_class c \
               JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
              WHERE n.nspname='writer_lease' AND c.relkind NOT IN ('r','i')), \
             ((SELECT pg_catalog.count(*) FROM pg_catalog.pg_collation x \
                JOIN pg_catalog.pg_namespace n ON n.oid=x.collnamespace WHERE n.nspname='writer_lease') + \
              (SELECT pg_catalog.count(*) FROM pg_catalog.pg_conversion x \
                JOIN pg_catalog.pg_namespace n ON n.oid=x.connamespace WHERE n.nspname='writer_lease') + \
              (SELECT pg_catalog.count(*) FROM pg_catalog.pg_operator x \
                JOIN pg_catalog.pg_namespace n ON n.oid=x.oprnamespace WHERE n.nspname='writer_lease') + \
              (SELECT pg_catalog.count(*) FROM pg_catalog.pg_opclass x \
                JOIN pg_catalog.pg_namespace n ON n.oid=x.opcnamespace WHERE n.nspname='writer_lease') + \
              (SELECT pg_catalog.count(*) FROM pg_catalog.pg_opfamily x \
                JOIN pg_catalog.pg_namespace n ON n.oid=x.opfnamespace WHERE n.nspname='writer_lease') + \
              (SELECT pg_catalog.count(*) FROM pg_catalog.pg_statistic_ext x \
                JOIN pg_catalog.pg_namespace n ON n.oid=x.stxnamespace WHERE n.nspname='writer_lease') + \
              (SELECT pg_catalog.count(*) FROM pg_catalog.pg_ts_config x \
                JOIN pg_catalog.pg_namespace n ON n.oid=x.cfgnamespace WHERE n.nspname='writer_lease') + \
              (SELECT pg_catalog.count(*) FROM pg_catalog.pg_ts_dict x \
                JOIN pg_catalog.pg_namespace n ON n.oid=x.dictnamespace WHERE n.nspname='writer_lease') + \
              (SELECT pg_catalog.count(*) FROM pg_catalog.pg_ts_parser x \
                JOIN pg_catalog.pg_namespace n ON n.oid=x.prsnamespace WHERE n.nspname='writer_lease') + \
              (SELECT pg_catalog.count(*) FROM pg_catalog.pg_ts_template x \
                JOIN pg_catalog.pg_namespace n ON n.oid=x.tmplnamespace WHERE n.nspname='writer_lease')), \
             (SELECT pg_catalog.count(*) FROM pg_catalog.pg_class c \
               JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
               CROSS JOIN pg_catalog.pg_roles roles \
              WHERE n.nspname='writer_lease' AND c.relkind IN ('r','p') \
                AND NOT roles.rolsuper AND roles.rolname !~ '^pg_' \
                AND roles.rolname <> 'lattice_migrator' \
                AND pg_catalog.has_table_privilege(roles.rolname,c.oid,\
                  'SELECT,INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER,MAINTAIN')), \
             (SELECT pg_catalog.count(*) FROM pg_catalog.pg_proc p \
               JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
               CROSS JOIN pg_catalog.pg_roles roles \
              WHERE n.nspname='writer_lease' \
                AND NOT roles.rolsuper AND roles.rolname !~ '^pg_' \
                AND roles.rolname NOT IN ('lattice_migrator','lattice_runtime') \
                AND pg_catalog.has_function_privilege(roles.rolname,p.oid,'EXECUTE')), \
             (SELECT pg_catalog.count(*) FROM pg_catalog.pg_proc p \
               JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
              WHERE n.nspname='writer_lease' \
                AND NOT pg_catalog.has_function_privilege('lattice_runtime',p.oid,'EXECUTE')), \
             (NOT pg_catalog.has_schema_privilege('lattice_runtime','writer_lease','USAGE') \
               OR pg_catalog.has_schema_privilege('lattice_runtime','writer_lease','CREATE') \
               OR EXISTS (SELECT 1 FROM pg_catalog.pg_roles roles \
                  WHERE NOT roles.rolsuper AND roles.rolname !~ '^pg_' \
                    AND roles.rolname NOT IN ('lattice_migrator','lattice_runtime') \
                    AND (pg_catalog.has_schema_privilege(roles.rolname,'writer_lease','USAGE') \
                      OR pg_catalog.has_schema_privilege(roles.rolname,'writer_lease','CREATE'))))",
            &[],
        )
        .map_err(map_database)?;
    let expected_counts = [12_i64, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    for (index, expected) in expected_counts.into_iter().enumerate() {
        let observed: i64 = row.try_get(index).map_err(map_database)?;
        if observed != expected {
            return Err(ExtensionSetupError::new(
                ExtensionSetupErrorKind::PartialOrCollidingProfile,
            ));
        }
    }
    let schema_acl_drift: bool = row.try_get(11).map_err(map_database)?;
    if schema_acl_drift {
        return Err(ExtensionSetupError::new(
            ExtensionSetupErrorKind::PartialOrCollidingProfile,
        ));
    }
    Ok(())
}

fn map_database<E>(_error: E) -> ExtensionSetupError {
    ExtensionSetupError::new(ExtensionSetupErrorKind::Database)
}
