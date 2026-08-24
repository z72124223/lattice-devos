use std::error::Error;
use std::fmt;
use std::time::{Duration, Instant};

use lattice_contracts::{ContentDigest, ProjectId};
use lattice_writer_lease::{
    CommandOutcome, UntrustedWriterLeaseSnapshot, VerifiedWriterLeaseAggregate,
    WriterLeaseCheckpoint, verify_snapshot_against_checkpoint,
};
use postgres::error::SqlState;
use postgres::{Client, GenericClient, IsolationLevel, Transaction};

use crate::{
    ExtensionManifestEvidence, WRITER_LEASE_EXTENSION_ID, WRITER_LEASE_EXTENSION_PATH,
    WRITER_LEASE_V1_EXTENSION_PATH, WRITER_LEASE_V3_EXTENSION_PATH, sha256_hex,
    verify_embedded_extension_manifest, verify_embedded_v1_extension_manifest,
    verify_embedded_v3_extension_manifest, verify_embedded_v3_rebind_manifest,
};

const GLOBAL_MIGRATION_ADVISORY_LOCK: i64 = 0x4c41_5454_4943_4501;
const MEMORY_EXTENSION_ADVISORY_LOCK: i64 = 0x4c41_5443_4d45_4d31;
const WRITER_LEASE_EXTENSION_ADVISORY_LOCK: i64 = 0x4c41_5457_4c45_4131;
const MAX_SERIALIZATION_ATTEMPTS: usize = 3;
const GLOBAL_APPLY_GATE_TIMEOUT: Duration = Duration::from_secs(30);
const GLOBAL_APPLY_GATE_POLL_INTERVAL: Duration = Duration::from_millis(20);
const HISTORICAL_GLOBAL_MANIFEST_SHA256: &str =
    "09c431df18ad71a4f44239a5d2ddf6b1774b8ffec06c7f9223f0e41757f3d407";
const CURRENT_GLOBAL_MANIFEST_SHA256: &str =
    "f92a51fa19c4fe0ffebfc40f20924bd1209bb2441b1bc69f787bc3c4a925425d";
const V6_GLOBAL_MANIFEST_SHA256: &str =
    "4a004488543ce39266ec046607a938958da51567fe747cb22f2e731f30b36ed7";
const HISTORICAL_MEMORY_MANIFEST_SHA256: &str =
    "0aedbd7d9ef7ca07fc2910d0da34c163cc83e3dd56f9b28292ae1f4f0c3c4d7e";
const CURRENT_MEMORY_MANIFEST_SHA256: &str =
    "d4cc712d262ae1f7c96bd65526eab611c90e193363afd865af2126307b2903f0";

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

const V1_EXPECTED_CATALOG_PROFILES: [(&str, usize, &str); 10] = [
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

const V2_BRIDGE_EXPECTED_CATALOG_PROFILES: [(&str, usize, &str); 10] = [
    (
        RELATION_PROFILE_SQL,
        5,
        "382b81889838d60c02ce5c31f77454e93f23372d90b3137a47663c5de74f9670",
    ),
    (
        COLUMN_PROFILE_SQL,
        73,
        "560e93c2a765db0024c0e74d25a51b90cfc72b204601139de8fdb688d48c0610",
    ),
    (
        CONSTRAINT_PROFILE_SQL,
        27,
        "3463b3ac82c1a7c53e5a80c41995f882ffe5f3f07fc5a82a97d50582d4d26915",
    ),
    (
        INDEX_PROFILE_SQL,
        8,
        "66b315513cbf50c3c7dbc143eb7061c6dbb823d7eac853c50f83434caf1a1022",
    ),
    (
        FUNCTION_PROFILE_SQL,
        9,
        "caa34168b5f9da4c8d2d02fce6e98882d73456c7c1f5c1af2b71f404efc647d1",
    ),
    (
        SCHEMA_ACL_PROFILE_SQL,
        2,
        "f8a84b870fcb8b091dbc7f9cf6835fb4311064eec5c83b31159a9a936a11e738",
    ),
    (
        TABLE_ACL_PROFILE_SQL,
        40,
        "b99ef0c0ea5b550ae5e805d29b0020e31c1800a016b0de82cda566d7b25e9569",
    ),
    (
        FUNCTION_ACL_PROFILE_SQL,
        9,
        "73951f1b33a4d6b3c4742fb49f91cf0601f04fd472b21c4db8bb36815fed0e89",
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

const V2_CURRENT_EXPECTED_CATALOG_PROFILES: [(&str, usize, &str); 10] = [
    (
        RELATION_PROFILE_SQL,
        5,
        "382b81889838d60c02ce5c31f77454e93f23372d90b3137a47663c5de74f9670",
    ),
    (
        COLUMN_PROFILE_SQL,
        73,
        "560e93c2a765db0024c0e74d25a51b90cfc72b204601139de8fdb688d48c0610",
    ),
    (
        CONSTRAINT_PROFILE_SQL,
        27,
        "3463b3ac82c1a7c53e5a80c41995f882ffe5f3f07fc5a82a97d50582d4d26915",
    ),
    (
        INDEX_PROFILE_SQL,
        8,
        "66b315513cbf50c3c7dbc143eb7061c6dbb823d7eac853c50f83434caf1a1022",
    ),
    (
        FUNCTION_PROFILE_SQL,
        9,
        "caa34168b5f9da4c8d2d02fce6e98882d73456c7c1f5c1af2b71f404efc647d1",
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
        16,
        "bd5b05d60340a1b9f9fbf1de2b4bed8586b7eede4fd8d7c4825841c221e89b7a",
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

/// Exact database identity for the fixed Writer v3 schema-v6 transition.
///
/// Global v5/v6 and Memory-v3 manifest identities are compile-time constants;
/// callers cannot substitute a profile or SQL path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct V3ExtensionTarget {
    database_name: String,
    database_identity_digest: ContentDigest,
}

impl V3ExtensionTarget {
    /// Constructs the fixed v3 transition target without SQL, credentials, or
    /// caller-selected manifests.
    ///
    /// # Errors
    ///
    /// Rejects an unsafe or unbounded database name.
    pub fn new(
        database_name: String,
        database_identity_digest: ContentDigest,
    ) -> Result<Self, ExtensionSetupError> {
        ExtensionTarget::new(
            database_name.clone(),
            database_identity_digest.clone(),
            fixed_digest(CURRENT_GLOBAL_MANIFEST_SHA256)?,
            fixed_digest(CURRENT_MEMORY_MANIFEST_SHA256)?,
        )?;
        Ok(Self {
            database_name,
            database_identity_digest,
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

    fn predecessor(&self) -> Result<ExtensionTarget, ExtensionSetupError> {
        ExtensionTarget::new(
            self.database_name.clone(),
            self.database_identity_digest.clone(),
            fixed_digest(CURRENT_GLOBAL_MANIFEST_SHA256)?,
            fixed_digest(CURRENT_MEMORY_MANIFEST_SHA256)?,
        )
    }

    fn successor(&self) -> Result<ExtensionTarget, ExtensionSetupError> {
        ExtensionTarget::new(
            self.database_name.clone(),
            self.database_identity_digest.clone(),
            fixed_digest(V6_GLOBAL_MANIFEST_SHA256)?,
            fixed_digest(CURRENT_MEMORY_MANIFEST_SHA256)?,
        )
    }
}

fn fixed_digest(value: &str) -> Result<ContentDigest, ExtensionSetupError> {
    ContentDigest::from_sha256(value.to_owned())
        .map_err(|_| ExtensionSetupError::new(ExtensionSetupErrorKind::ManifestMismatch))
}

/// Administrative apply result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExtensionApplyOutcome {
    Installed,
    Bridged,
    BridgePending,
    Activated,
    Rebound,
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

struct GlobalApplyGate<'client> {
    client: &'client mut Client,
    held: bool,
}

fn try_global_apply_gate_lock(client: &mut Client) -> Result<bool, ExtensionSetupError> {
    let mut transaction = client.transaction().map_err(map_public_database)?;
    enter_migrator(&mut transaction).map_err(SetupAttemptError::into_public)?;
    let acquired = transaction
        .query_one(
            "SELECT pg_catalog.pg_try_advisory_lock($1)",
            &[&GLOBAL_MIGRATION_ADVISORY_LOCK],
        )
        .and_then(|row| row.try_get(0))
        .map_err(map_public_database)?;
    if let Err(error) = transaction.commit() {
        if acquired {
            let _ = release_global_apply_gate_lock(client);
        }
        return Err(map_public_database(error));
    }
    Ok(acquired)
}

fn release_global_apply_gate_lock(client: &mut Client) -> Result<bool, ExtensionSetupError> {
    let mut transaction = client.transaction().map_err(map_public_database)?;
    enter_migrator(&mut transaction).map_err(SetupAttemptError::into_public)?;
    let unlocked = transaction
        .query_one(
            "SELECT pg_catalog.pg_advisory_unlock($1)",
            &[&GLOBAL_MIGRATION_ADVISORY_LOCK],
        )
        .and_then(|row| row.try_get(0))
        .map_err(map_public_database)?;
    transaction.commit().map_err(map_public_database)?;
    Ok(unlocked)
}

impl<'client> GlobalApplyGate<'client> {
    fn acquire(client: &'client mut Client) -> Result<Self, ExtensionSetupError> {
        let started_at = Instant::now();
        loop {
            let acquired = try_global_apply_gate_lock(client)?;
            if acquired {
                return Ok(Self { client, held: true });
            }

            let elapsed = started_at.elapsed();
            if elapsed >= GLOBAL_APPLY_GATE_TIMEOUT {
                return Err(ExtensionSetupError::new(ExtensionSetupErrorKind::Database));
            }
            std::thread::sleep(std::cmp::min(
                GLOBAL_APPLY_GATE_POLL_INTERVAL,
                GLOBAL_APPLY_GATE_TIMEOUT.saturating_sub(elapsed),
            ));
        }
    }

    fn client(&mut self) -> &mut Client {
        self.client
    }

    fn release(mut self) -> Result<(), ExtensionSetupError> {
        let unlocked = release_global_apply_gate_lock(self.client)?;
        self.held = false;
        if !unlocked {
            return Err(ExtensionSetupError::new(ExtensionSetupErrorKind::Database));
        }
        Ok(())
    }
}

impl Drop for GlobalApplyGate<'_> {
    fn drop(&mut self) {
        if self.held && release_global_apply_gate_lock(self.client).unwrap_or(false) {
            self.held = false;
        }
    }
}

#[allow(clippy::needless_pass_by_value)]
fn map_public_database(error: postgres::Error) -> ExtensionSetupError {
    map_database(error).into_public()
}

/// Installs, bridges, activates, or verifies the closed Writer Lease v2 profile.
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
    let v1_manifest = verify_embedded_v1_extension_manifest()
        .map_err(|_| ExtensionSetupError::new(ExtensionSetupErrorKind::ManifestMismatch))?;
    let mut gate = GlobalApplyGate::acquire(client)?;
    let result = apply_extension_under_gate(gate.client(), target, &v1_manifest, &manifest);
    let release = gate.release();
    release?;
    result
}

fn apply_extension_under_gate(
    client: &mut Client,
    target: &ExtensionTarget,
    v1_manifest: &ExtensionManifestEvidence,
    manifest: &ExtensionManifestEvidence,
) -> Result<ExtensionApplyOutcome, ExtensionSetupError> {
    for attempt in 1..=MAX_SERIALIZATION_ATTEMPTS {
        match apply_extension_attempt(client, target, v1_manifest, manifest) {
            Err(SetupAttemptError::SerializationFailure)
                if attempt < MAX_SERIALIZATION_ATTEMPTS => {}
            Err(SetupAttemptError::SerializationFailure) => {
                return Err(ExtensionSetupError::new(ExtensionSetupErrorKind::Database));
            }
            Err(SetupAttemptError::Setup(error)) => return Err(error),
            Ok(outcome) => return Ok(outcome),
        }
    }
    Err(ExtensionSetupError::new(ExtensionSetupErrorKind::Database))
}

#[allow(clippy::too_many_lines)]
fn apply_extension_attempt(
    client: &mut Client,
    target: &ExtensionTarget,
    v1_manifest: &ExtensionManifestEvidence,
    manifest: &ExtensionManifestEvidence,
) -> Result<ExtensionApplyOutcome, SetupAttemptError> {
    let mut transaction = client
        .build_transaction()
        .isolation_level(IsolationLevel::Serializable)
        .start()
        .map_err(map_database)?;
    enter_migrator(&mut transaction)?;
    acquire_common_locks(&mut transaction)?;
    let foundation = verify_foundation(&mut transaction, target)?;
    let state = classify_state(&mut transaction, &foundation)?;
    let outcome = match state {
        InstalledState::Fresh => {
            if foundation.profile != FoundationProfile::G5MemoryV3 {
                return Err(setup_attempt_error(
                    ExtensionSetupErrorKind::UnsupportedFoundation,
                ));
            }
            install_fresh_current(&mut transaction, target, &foundation, v1_manifest, manifest)?;
            ExtensionApplyOutcome::Installed
        }
        InstalledState::G3MemoryV2WriterV1Current => {
            verify_v1_profile(
                &mut transaction,
                target,
                v1_manifest,
                &foundation.database_uuid,
            )?;
            verify_bridge_safety(&mut transaction)?;
            apply_v1_to_v2_bridge(&mut transaction, target, &foundation, v1_manifest, manifest)?;
            ExtensionApplyOutcome::Bridged
        }
        InstalledState::G3MemoryV2WriterV2Bridge => {
            verify_v2_bridge_profile(
                &mut transaction,
                target,
                &foundation.database_uuid,
                v1_manifest,
                manifest,
            )?;
            verify_bridge_safety(&mut transaction)?;
            ExtensionApplyOutcome::Bridged
        }
        InstalledState::G5MemoryV2WriterV2BridgePending => {
            verify_v2_bridge_profile(
                &mut transaction,
                target,
                &foundation.database_uuid,
                v1_manifest,
                manifest,
            )?;
            verify_bridge_safety(&mut transaction)?;
            ExtensionApplyOutcome::BridgePending
        }
        InstalledState::G5MemoryV3WriterV2BridgePending => {
            verify_v2_bridge_profile(
                &mut transaction,
                target,
                &foundation.database_uuid,
                v1_manifest,
                manifest,
            )?;
            verify_bridge_safety(&mut transaction)?;
            activate_v2_current(&mut transaction, target, &foundation, manifest)?;
            verify_v2_current_profile(
                &mut transaction,
                target,
                &foundation.database_uuid,
                v1_manifest,
                manifest,
            )?;
            ExtensionApplyOutcome::Activated
        }
        InstalledState::G5MemoryV3WriterV2Current => {
            verify_v2_current_profile(
                &mut transaction,
                target,
                &foundation.database_uuid,
                v1_manifest,
                manifest,
            )?;
            verify_replay_safe_history(&mut transaction)?;
            ExtensionApplyOutcome::AlreadyCurrent
        }
    };
    transaction.commit().map_err(map_database)?;
    Ok(outcome)
}

/// Applies the append-only Writer v3 bridge on the exact schema-v5/Memory-v3
/// predecessor while keeping every Writer runtime privilege closed.
///
/// # Errors
///
/// Rejects any non-v2-current predecessor, live authority, partial profile,
/// changed extension identity, or database/catalog ambiguity.
pub fn apply_v3_extension(
    client: &mut Client,
    target: &V3ExtensionTarget,
) -> Result<ExtensionApplyOutcome, ExtensionSetupError> {
    let v1 = verify_embedded_v1_extension_manifest()
        .map_err(|_| ExtensionSetupError::new(ExtensionSetupErrorKind::ManifestMismatch))?;
    let v2 = verify_embedded_extension_manifest()
        .map_err(|_| ExtensionSetupError::new(ExtensionSetupErrorKind::ManifestMismatch))?;
    let v3 = verify_embedded_v3_extension_manifest()
        .map_err(|_| ExtensionSetupError::new(ExtensionSetupErrorKind::ManifestMismatch))?;
    let rebind = verify_embedded_v3_rebind_manifest()
        .map_err(|_| ExtensionSetupError::new(ExtensionSetupErrorKind::ManifestMismatch))?;
    let predecessor = target.predecessor()?;
    let mut gate = GlobalApplyGate::acquire(client)?;
    let result = apply_v3_extension_under_gate(gate.client(), &predecessor, &v1, &v2, &v3, &rebind);
    gate.release()?;
    result
}

fn apply_v3_extension_under_gate(
    client: &mut Client,
    target: &ExtensionTarget,
    v1: &ExtensionManifestEvidence,
    v2: &ExtensionManifestEvidence,
    v3: &ExtensionManifestEvidence,
    rebind: &ExtensionManifestEvidence,
) -> Result<ExtensionApplyOutcome, ExtensionSetupError> {
    for attempt in 1..=MAX_SERIALIZATION_ATTEMPTS {
        match apply_v3_extension_attempt(client, target, v1, v2, v3, rebind) {
            Err(SetupAttemptError::SerializationFailure)
                if attempt < MAX_SERIALIZATION_ATTEMPTS => {}
            Err(SetupAttemptError::SerializationFailure) => {
                return Err(ExtensionSetupError::new(ExtensionSetupErrorKind::Database));
            }
            Err(SetupAttemptError::Setup(error)) => return Err(error),
            Ok(outcome) => return Ok(outcome),
        }
    }
    Err(ExtensionSetupError::new(ExtensionSetupErrorKind::Database))
}

fn apply_v3_extension_attempt(
    client: &mut Client,
    target: &ExtensionTarget,
    v1: &ExtensionManifestEvidence,
    v2: &ExtensionManifestEvidence,
    v3: &ExtensionManifestEvidence,
    rebind: &ExtensionManifestEvidence,
) -> Result<ExtensionApplyOutcome, SetupAttemptError> {
    let mut transaction = client
        .build_transaction()
        .isolation_level(IsolationLevel::Serializable)
        .start()
        .map_err(map_database)?;
    enter_migrator(&mut transaction)?;
    acquire_common_locks(&mut transaction)?;
    let foundation = verify_foundation(&mut transaction, target)?;
    if foundation.profile != FoundationProfile::G5MemoryV3 {
        return Err(setup_attempt_error(
            ExtensionSetupErrorKind::UnsupportedFoundation,
        ));
    }
    let outcome = match classify_v3_state(&mut transaction)? {
        V3InstalledState::V2Current => {
            verify_v2_current_profile(&mut transaction, target, &foundation.database_uuid, v1, v2)?;
            verify_bridge_safety(&mut transaction)?;
            apply_v2_to_v3_bridge(&mut transaction, target, v3)?;
            ensure_v3_rebind_boundary(&mut transaction, rebind)?;
            verify_v3_bridge_profile(
                &mut transaction,
                target,
                &foundation.database_uuid,
                v1,
                v2,
                v3,
            )?;
            ExtensionApplyOutcome::Bridged
        }
        V3InstalledState::G5MemoryV3WriterV3Bridge => {
            verify_bridge_safety(&mut transaction)?;
            ensure_v3_rebind_boundary(&mut transaction, rebind)?;
            verify_v3_bridge_profile(
                &mut transaction,
                target,
                &foundation.database_uuid,
                v1,
                v2,
                v3,
            )?;
            ExtensionApplyOutcome::Bridged
        }
        V3InstalledState::Absent
        | V3InstalledState::G6MemoryV3WriterV3BridgePending
        | V3InstalledState::G6MemoryV3WriterV3Current => {
            return Err(setup_attempt_error(
                ExtensionSetupErrorKind::UnsupportedFoundation,
            ));
        }
    };
    transaction.commit().map_err(map_database)?;
    Ok(outcome)
}

/// Installs a fresh Writer v3 current profile on schema v6, or invokes the
/// same fixed Writer-owned rebind procedure used by the Store transition.
///
/// # Errors
///
/// Rejects partial, colliding, active/suspect, wrong-generation, changed-byte,
/// or catalog/ACL state. Exact current retry is read-only and idempotent.
pub fn rebind_v3_extension(
    client: &mut Client,
    target: &V3ExtensionTarget,
) -> Result<ExtensionApplyOutcome, ExtensionSetupError> {
    rebind_v3_extension_with_policy(client, target, true)
}

/// Rebinds or verifies an already-present Writer v3 profile on schema v6.
///
/// Unlike [`rebind_v3_extension`], this product-bootstrap boundary rejects an
/// absent Writer profile before mutation. It therefore cannot silently install
/// new Writer state into a Store v6 database whose extension history is missing.
///
/// # Errors
///
/// Rejects absent, partial, colliding, active/suspect, wrong-generation,
/// changed-byte, or catalog/ACL state. Exact current retry is read-only.
pub fn rebind_existing_v3_extension(
    client: &mut Client,
    target: &V3ExtensionTarget,
) -> Result<ExtensionApplyOutcome, ExtensionSetupError> {
    rebind_v3_extension_with_policy(client, target, false)
}

fn rebind_v3_extension_with_policy(
    client: &mut Client,
    target: &V3ExtensionTarget,
    allow_fresh_install: bool,
) -> Result<ExtensionApplyOutcome, ExtensionSetupError> {
    let v1 = verify_embedded_v1_extension_manifest()
        .map_err(|_| ExtensionSetupError::new(ExtensionSetupErrorKind::ManifestMismatch))?;
    let v2 = verify_embedded_extension_manifest()
        .map_err(|_| ExtensionSetupError::new(ExtensionSetupErrorKind::ManifestMismatch))?;
    let v3 = verify_embedded_v3_extension_manifest()
        .map_err(|_| ExtensionSetupError::new(ExtensionSetupErrorKind::ManifestMismatch))?;
    let rebind = verify_embedded_v3_rebind_manifest()
        .map_err(|_| ExtensionSetupError::new(ExtensionSetupErrorKind::ManifestMismatch))?;
    let successor = target.successor()?;
    let mut gate = GlobalApplyGate::acquire(client)?;
    let result = rebind_v3_extension_attempt(
        gate.client(),
        &successor,
        &v1,
        &v2,
        &v3,
        &rebind,
        allow_fresh_install,
    );
    gate.release()?;
    result
}

fn rebind_v3_extension_attempt(
    client: &mut Client,
    target: &ExtensionTarget,
    v1: &ExtensionManifestEvidence,
    v2: &ExtensionManifestEvidence,
    v3: &ExtensionManifestEvidence,
    rebind: &ExtensionManifestEvidence,
    allow_fresh_install: bool,
) -> Result<ExtensionApplyOutcome, ExtensionSetupError> {
    let mut transaction = client
        .build_transaction()
        .isolation_level(IsolationLevel::Serializable)
        .start()
        .map_err(map_public_database)?;
    enter_migrator(&mut transaction).map_err(SetupAttemptError::into_public)?;
    acquire_common_locks(&mut transaction).map_err(SetupAttemptError::into_public)?;
    let foundation =
        verify_v3_foundation(&mut transaction, target).map_err(SetupAttemptError::into_public)?;
    let outcome =
        match classify_v3_state(&mut transaction).map_err(SetupAttemptError::into_public)? {
            V3InstalledState::Absent => {
                if !allow_fresh_install {
                    return Err(ExtensionSetupError::new(
                        ExtensionSetupErrorKind::UnsupportedFoundation,
                    ));
                }
                install_fresh_v3_current(&mut transaction, target, &foundation, v1, v2, v3, rebind)
                    .map_err(SetupAttemptError::into_public)?;
                ExtensionApplyOutcome::Installed
            }
            V3InstalledState::G6MemoryV3WriterV3BridgePending => {
                verify_bridge_safety(&mut transaction).map_err(SetupAttemptError::into_public)?;
                verify_v3_rebind_boundary(&mut transaction, rebind)
                    .map_err(SetupAttemptError::into_public)?;
                transaction
                    .batch_execute("CALL writer_lease.writer_lease_rebind_v3()")
                    .map_err(map_public_database)?;
                verify_v3_current_profile(
                    &mut transaction,
                    target,
                    &foundation.database_uuid,
                    v1,
                    v2,
                    v3,
                )
                .map_err(SetupAttemptError::into_public)?;
                ExtensionApplyOutcome::Rebound
            }
            V3InstalledState::G6MemoryV3WriterV3Current => {
                verify_v3_rebind_boundary(&mut transaction, rebind)
                    .map_err(SetupAttemptError::into_public)?;
                verify_v3_current_profile(
                    &mut transaction,
                    target,
                    &foundation.database_uuid,
                    v1,
                    v2,
                    v3,
                )
                .map_err(SetupAttemptError::into_public)?;
                ExtensionApplyOutcome::AlreadyCurrent
            }
            V3InstalledState::V2Current | V3InstalledState::G5MemoryV3WriterV3Bridge => {
                return Err(ExtensionSetupError::new(
                    ExtensionSetupErrorKind::UnsupportedFoundation,
                ));
            }
        };
    transaction.commit().map_err(map_public_database)?;
    Ok(outcome)
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
    verify_extension_attempt(client, target).map_err(SetupAttemptError::into_public)
}

fn verify_extension_attempt(
    client: &mut Client,
    target: &ExtensionTarget,
) -> Result<(), SetupAttemptError> {
    let manifest = verify_embedded_extension_manifest()
        .map_err(|_| setup_attempt_error(ExtensionSetupErrorKind::ManifestMismatch))?;
    let mut transaction = client
        .build_transaction()
        .isolation_level(IsolationLevel::RepeatableRead)
        .read_only(true)
        .start()
        .map_err(map_database)?;
    enter_migrator(&mut transaction)?;
    acquire_common_locks(&mut transaction)?;
    let foundation = verify_foundation(&mut transaction, target)?;
    let v1_manifest = verify_embedded_v1_extension_manifest()
        .map_err(|_| setup_attempt_error(ExtensionSetupErrorKind::ManifestMismatch))?;
    if classify_state(&mut transaction, &foundation)? != InstalledState::G5MemoryV3WriterV2Current {
        return Err(setup_attempt_error(
            ExtensionSetupErrorKind::UnsupportedFoundation,
        ));
    }
    verify_v2_current_profile(
        &mut transaction,
        target,
        &foundation.database_uuid,
        &v1_manifest,
        &manifest,
    )?;
    verify_replay_safe_history(&mut transaction)?;
    transaction.commit().map_err(map_database)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FoundationProfile {
    G3MemoryV2,
    G5MemoryV2,
    G5MemoryV3,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InstalledState {
    Fresh,
    G3MemoryV2WriterV1Current,
    G3MemoryV2WriterV2Bridge,
    G5MemoryV2WriterV2BridgePending,
    G5MemoryV3WriterV2BridgePending,
    G5MemoryV3WriterV2Current,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum V3InstalledState {
    Absent,
    V2Current,
    G5MemoryV3WriterV3Bridge,
    G6MemoryV3WriterV3BridgePending,
    G6MemoryV3WriterV3Current,
}

struct FoundationEvidence {
    database_uuid: String,
    profile: FoundationProfile,
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

fn acquire_common_locks<C: GenericClient>(client: &mut C) -> Result<(), SetupAttemptError> {
    for lock in [
        GLOBAL_MIGRATION_ADVISORY_LOCK,
        MEMORY_EXTENSION_ADVISORY_LOCK,
        WRITER_LEASE_EXTENSION_ADVISORY_LOCK,
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
        || global_manifest != target.global_manifest_digest().as_str()
        || memory_manifest != target.memory_manifest_digest().as_str()
    {
        return Err(setup_attempt_error(
            ExtensionSetupErrorKind::UnsupportedFoundation,
        ));
    }
    let profile = match (
        global_version,
        global_manifest.as_str(),
        memory_version,
        memory_manifest.as_str(),
    ) {
        (3, HISTORICAL_GLOBAL_MANIFEST_SHA256, 2, HISTORICAL_MEMORY_MANIFEST_SHA256) => {
            FoundationProfile::G3MemoryV2
        }
        (5, CURRENT_GLOBAL_MANIFEST_SHA256, 2, HISTORICAL_MEMORY_MANIFEST_SHA256) => {
            FoundationProfile::G5MemoryV2
        }
        (5, CURRENT_GLOBAL_MANIFEST_SHA256, 3, CURRENT_MEMORY_MANIFEST_SHA256) => {
            FoundationProfile::G5MemoryV3
        }
        _ => {
            return Err(setup_attempt_error(
                ExtensionSetupErrorKind::UnsupportedFoundation,
            ));
        }
    };
    Ok(FoundationEvidence {
        database_uuid,
        profile,
    })
}

fn verify_v3_foundation<C: GenericClient>(
    client: &mut C,
    target: &ExtensionTarget,
) -> Result<FoundationEvidence, SetupAttemptError> {
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
        || global_version != 6
        || global_manifest != V6_GLOBAL_MANIFEST_SHA256
        || global_manifest != target.global_manifest_digest().as_str()
        || memory_version != 3
        || memory_manifest != CURRENT_MEMORY_MANIFEST_SHA256
        || memory_manifest != target.memory_manifest_digest().as_str()
    {
        return Err(setup_attempt_error(
            ExtensionSetupErrorKind::UnsupportedFoundation,
        ));
    }
    Ok(FoundationEvidence {
        database_uuid,
        profile: FoundationProfile::G5MemoryV3,
    })
}

#[allow(clippy::too_many_lines)]
fn classify_state<C: GenericClient>(
    client: &mut C,
    foundation: &FoundationEvidence,
) -> Result<InstalledState, SetupAttemptError> {
    let schema_exists: bool = client
        .query_one(
            "SELECT pg_catalog.to_regnamespace('writer_lease') IS NOT NULL",
            &[],
        )
        .map_err(map_database)?
        .try_get(0)
        .map_err(map_database)?;
    if !schema_exists {
        return Ok(InstalledState::Fresh);
    }
    let row = client
        .query_opt(
            "SELECT i.extension_schema_version, i.extension_path::text, \
                    i.global_schema_version, i.required_memory_schema_version, \
                    (SELECT pg_catalog.count(*) \
                       FROM ONLY writer_lease.writer_lease_extension_ledger), \
                    (SELECT pg_catalog.string_agg(\
                         l.ledger_ordinal::text || ':' || l.event_kind::text, ',' \
                         ORDER BY l.ledger_ordinal) \
                       FROM ONLY writer_lease.writer_lease_extension_ledger AS l) \
               FROM ONLY writer_lease.writer_lease_extension_identity AS i \
              WHERE i.singleton",
            &[],
        )
        .map_err(map_database)?
        .ok_or_else(|| {
            ExtensionSetupError::new(ExtensionSetupErrorKind::PartialOrCollidingProfile)
        })?;
    let version: i16 = row.try_get(0).map_err(map_database)?;
    let path: String = row.try_get(1).map_err(map_database)?;
    let global: i16 = row.try_get(2).map_err(map_database)?;
    let memory: i16 = row.try_get(3).map_err(map_database)?;
    let ledger_count: i64 = row.try_get(4).map_err(map_database)?;
    let ledger_shape: Option<String> = row.try_get(5).map_err(map_database)?;
    match (
        foundation.profile,
        version,
        path.as_str(),
        global,
        memory,
        ledger_count,
        ledger_shape.as_deref(),
    ) {
        (
            FoundationProfile::G3MemoryV2,
            1,
            WRITER_LEASE_V1_EXTENSION_PATH,
            3,
            2,
            1,
            Some("1:INSTALLED"),
        ) => Ok(InstalledState::G3MemoryV2WriterV1Current),
        (
            FoundationProfile::G3MemoryV2,
            2,
            WRITER_LEASE_EXTENSION_PATH,
            3,
            2,
            2,
            Some("1:INSTALLED,2:UPGRADED"),
        ) => Ok(InstalledState::G3MemoryV2WriterV2Bridge),
        (
            FoundationProfile::G5MemoryV2,
            2,
            WRITER_LEASE_EXTENSION_PATH,
            3,
            2,
            2,
            Some("1:INSTALLED,2:UPGRADED"),
        ) => Ok(InstalledState::G5MemoryV2WriterV2BridgePending),
        (
            FoundationProfile::G5MemoryV3,
            2,
            WRITER_LEASE_EXTENSION_PATH,
            3,
            2,
            2,
            Some("1:INSTALLED,2:UPGRADED"),
        ) => Ok(InstalledState::G5MemoryV3WriterV2BridgePending),
        (
            FoundationProfile::G5MemoryV3,
            2,
            WRITER_LEASE_EXTENSION_PATH,
            5,
            3,
            1,
            Some("1:INSTALLED"),
        )
        | (
            FoundationProfile::G5MemoryV3,
            2,
            WRITER_LEASE_EXTENSION_PATH,
            5,
            3,
            3,
            Some("1:INSTALLED,2:UPGRADED,3:REBOUND"),
        ) => Ok(InstalledState::G5MemoryV3WriterV2Current),
        _ => Err(setup_attempt_error(
            ExtensionSetupErrorKind::PartialOrCollidingProfile,
        )),
    }
}

fn classify_v3_state<C: GenericClient>(
    client: &mut C,
) -> Result<V3InstalledState, SetupAttemptError> {
    let schema_exists: bool = client
        .query_one(
            "SELECT pg_catalog.to_regnamespace('writer_lease') IS NOT NULL",
            &[],
        )
        .map_err(map_database)?
        .try_get(0)
        .map_err(map_database)?;
    if !schema_exists {
        return Ok(V3InstalledState::Absent);
    }
    let row = client
        .query_opt(
            "SELECT i.extension_schema_version, i.extension_path::text, \
                    i.global_schema_version, i.required_memory_schema_version, \
                    (SELECT pg_catalog.count(*) \
                       FROM ONLY writer_lease.writer_lease_extension_ledger), \
                    (SELECT pg_catalog.string_agg( \
                         l.ledger_ordinal::text || ':' || l.event_kind::text || ':' || \
                         l.extension_schema_version::text || ':' || l.global_schema_version::text, \
                         ',' ORDER BY l.ledger_ordinal) \
                       FROM ONLY writer_lease.writer_lease_extension_ledger AS l) \
               FROM ONLY writer_lease.writer_lease_extension_identity AS i \
              WHERE i.singleton",
            &[],
        )
        .map_err(map_database)?
        .ok_or_else(profile_collision)?;
    let version: i16 = row.try_get(0).map_err(map_database)?;
    let path: String = row.try_get(1).map_err(map_database)?;
    let global: i16 = row.try_get(2).map_err(map_database)?;
    let memory: i16 = row.try_get(3).map_err(map_database)?;
    let ledger_count: i64 = row.try_get(4).map_err(map_database)?;
    let ledger_shape: Option<String> = row.try_get(5).map_err(map_database)?;
    match (
        version,
        path.as_str(),
        global,
        memory,
        ledger_count,
        ledger_shape.as_deref(),
    ) {
        (2, WRITER_LEASE_EXTENSION_PATH, 5, 3, 1, Some("1:INSTALLED:2:5"))
        | (
            2,
            WRITER_LEASE_EXTENSION_PATH,
            5,
            3,
            3,
            Some("1:INSTALLED:1:3,2:UPGRADED:2:3,3:REBOUND:2:5"),
        ) => Ok(V3InstalledState::V2Current),
        (3, WRITER_LEASE_V3_EXTENSION_PATH, 5, 3, 2, Some("1:INSTALLED:2:5,2:UPGRADED:3:5"))
        | (
            3,
            WRITER_LEASE_V3_EXTENSION_PATH,
            5,
            3,
            4,
            Some("1:INSTALLED:1:3,2:UPGRADED:2:3,3:REBOUND:2:5,4:UPGRADED:3:5"),
        ) => Ok(V3InstalledState::G5MemoryV3WriterV3Bridge),
        (3, WRITER_LEASE_V3_EXTENSION_PATH, 6, 3, 2, Some("1:INSTALLED:2:5,2:UPGRADED:3:5"))
        | (
            3,
            WRITER_LEASE_V3_EXTENSION_PATH,
            6,
            3,
            4,
            Some("1:INSTALLED:1:3,2:UPGRADED:2:3,3:REBOUND:2:5,4:UPGRADED:3:5"),
        ) => Ok(V3InstalledState::G6MemoryV3WriterV3BridgePending),
        (3, WRITER_LEASE_V3_EXTENSION_PATH, 6, 3, 1, Some("1:INSTALLED:3:6"))
        | (
            3,
            WRITER_LEASE_V3_EXTENSION_PATH,
            6,
            3,
            3,
            Some("1:INSTALLED:2:5,2:UPGRADED:3:5,3:REBOUND:3:6"),
        )
        | (
            3,
            WRITER_LEASE_V3_EXTENSION_PATH,
            6,
            3,
            5,
            Some("1:INSTALLED:1:3,2:UPGRADED:2:3,3:REBOUND:2:5,4:UPGRADED:3:5,5:REBOUND:3:6"),
        ) => Ok(V3InstalledState::G6MemoryV3WriterV3Current),
        _ => Err(profile_collision()),
    }
}

fn install_fresh_current<C: GenericClient>(
    client: &mut C,
    target: &ExtensionTarget,
    foundation: &FoundationEvidence,
    v1_manifest: &ExtensionManifestEvidence,
    manifest: &ExtensionManifestEvidence,
) -> Result<(), SetupAttemptError> {
    for embedded in [v1_manifest, manifest] {
        let sql = std::str::from_utf8(embedded.bytes())
            .map_err(|_| setup_attempt_error(ExtensionSetupErrorKind::ManifestMismatch))?;
        client.batch_execute(sql).map_err(map_database)?;
    }
    let inserted = client
        .execute(
            "INSERT INTO writer_lease.writer_lease_extension_identity (\
                 singleton, extension_id, extension_schema_version, extension_path, \
                 extension_sql_sha256, extension_manifest_sha256, database_uuid, \
                 database_identity_sha256, global_schema_version, global_manifest_sha256, \
                 required_memory_schema_version, required_memory_manifest_sha256\
             ) VALUES (true, $1, 2, $2, $3, $4, $5::text::uuid, $6, 5, $7, 3, $8)",
            &[
                &WRITER_LEASE_EXTENSION_ID,
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
    if inserted != 1 {
        return Err(setup_attempt_error(
            ExtensionSetupErrorKind::PartialOrCollidingProfile,
        ));
    }
    insert_current_ledger(client, 1, "INSTALLED")?;
    activate_runtime_acl(client)?;
    verify_v2_current_profile(
        client,
        target,
        &foundation.database_uuid,
        v1_manifest,
        manifest,
    )
}

fn apply_v1_to_v2_bridge<C: GenericClient>(
    client: &mut C,
    target: &ExtensionTarget,
    foundation: &FoundationEvidence,
    v1_manifest: &ExtensionManifestEvidence,
    manifest: &ExtensionManifestEvidence,
) -> Result<(), SetupAttemptError> {
    let sql = std::str::from_utf8(manifest.bytes())
        .map_err(|_| setup_attempt_error(ExtensionSetupErrorKind::ManifestMismatch))?;
    client.batch_execute(sql).map_err(map_database)?;
    let updated = client
        .execute(
            "UPDATE ONLY writer_lease.writer_lease_extension_identity SET \
                 extension_schema_version = 2, extension_path = $1, \
                 extension_sql_sha256 = $2, extension_manifest_sha256 = $3 \
              WHERE singleton AND extension_id = $4 AND extension_schema_version = 1 \
                AND extension_path = $5 AND extension_sql_sha256 = $6 \
                AND extension_manifest_sha256 = $7 AND database_uuid = $8::text::uuid \
                AND database_identity_sha256 = $9 AND global_schema_version = 3 \
                AND global_manifest_sha256 = $10 \
                AND required_memory_schema_version = 2 \
                AND required_memory_manifest_sha256 = $11",
            &[
                &WRITER_LEASE_EXTENSION_PATH,
                &manifest.sql_sha256().as_str(),
                &manifest.manifest_sha256().as_str(),
                &WRITER_LEASE_EXTENSION_ID,
                &WRITER_LEASE_V1_EXTENSION_PATH,
                &v1_manifest.sql_sha256().as_str(),
                &v1_manifest.manifest_sha256().as_str(),
                &foundation.database_uuid,
                &target.database_identity_digest().as_str(),
                &HISTORICAL_GLOBAL_MANIFEST_SHA256,
                &HISTORICAL_MEMORY_MANIFEST_SHA256,
            ],
        )
        .map_err(map_database)?;
    if updated != 1 {
        return Err(setup_attempt_error(
            ExtensionSetupErrorKind::PartialOrCollidingProfile,
        ));
    }
    insert_current_ledger(client, 2, "UPGRADED")?;
    verify_v2_bridge_profile(
        client,
        target,
        &foundation.database_uuid,
        v1_manifest,
        manifest,
    )
}

fn apply_v2_to_v3_bridge<C: GenericClient>(
    client: &mut C,
    target: &ExtensionTarget,
    v3: &ExtensionManifestEvidence,
) -> Result<(), SetupAttemptError> {
    let prior_ledger_count: i64 = client
        .query_one(
            "SELECT pg_catalog.count(*) FROM ONLY writer_lease.writer_lease_extension_ledger",
            &[],
        )
        .map_err(map_database)?
        .try_get(0)
        .map_err(map_database)?;
    let next_ordinal = match prior_ledger_count {
        1 => 2_i16,
        3 => 4_i16,
        _ => return Err(profile_collision()),
    };
    let sql = std::str::from_utf8(v3.bytes())
        .map_err(|_| setup_attempt_error(ExtensionSetupErrorKind::ManifestMismatch))?;
    client.batch_execute(sql).map_err(map_database)?;
    let updated = client
        .execute(
            "UPDATE ONLY writer_lease.writer_lease_extension_identity SET \
                 extension_schema_version = 3, extension_path = $1, \
                 extension_sql_sha256 = $2, extension_manifest_sha256 = $3 \
              WHERE singleton AND extension_id = $4 AND extension_schema_version = 2 \
                AND extension_path = $5 AND database_identity_sha256 = $6 \
                AND global_schema_version = 5 AND global_manifest_sha256 = $7 \
                AND required_memory_schema_version = 3 \
                AND required_memory_manifest_sha256 = $8",
            &[
                &WRITER_LEASE_V3_EXTENSION_PATH,
                &v3.sql_sha256().as_str(),
                &v3.manifest_sha256().as_str(),
                &WRITER_LEASE_EXTENSION_ID,
                &WRITER_LEASE_EXTENSION_PATH,
                &target.database_identity_digest().as_str(),
                &CURRENT_GLOBAL_MANIFEST_SHA256,
                &CURRENT_MEMORY_MANIFEST_SHA256,
            ],
        )
        .map_err(map_database)?;
    if updated != 1 {
        return Err(profile_collision());
    }
    insert_current_ledger(client, next_ordinal, "UPGRADED")
}

fn ensure_v3_rebind_boundary<C: GenericClient>(
    client: &mut C,
    rebind: &ExtensionManifestEvidence,
) -> Result<(), SetupAttemptError> {
    let count: i64 = client
        .query_one(
            "SELECT pg_catalog.count(*) FROM pg_catalog.pg_proc p \
               JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
              WHERE n.nspname='writer_lease' AND p.proname='writer_lease_rebind_v3'",
            &[],
        )
        .map_err(map_database)?
        .try_get(0)
        .map_err(map_database)?;
    match count {
        0 => {
            let sql = std::str::from_utf8(rebind.bytes()).map_err(|_| profile_collision())?;
            client.batch_execute(sql).map_err(map_database)?;
        }
        1 => {}
        _ => return Err(profile_collision()),
    }
    verify_v3_rebind_boundary(client, rebind)
}

fn verify_v3_rebind_boundary<C: GenericClient>(
    client: &mut C,
    rebind: &ExtensionManifestEvidence,
) -> Result<(), SetupAttemptError> {
    let sql = std::str::from_utf8(rebind.bytes()).map_err(|_| profile_collision())?;
    let marker = "$lattice_writer_lease_rebind_v3$";
    let expected_body = sql
        .split_once(marker)
        .and_then(|(_, remainder)| remainder.rsplit_once(marker).map(|(body, _)| body.trim()))
        .ok_or_else(profile_collision)?;
    let rows = client
        .query(
            "SELECT p.prokind::text,l.lanname,r.rolname,p.prosecdef,p.provolatile::text, \
                    p.proparallel::text,pg_catalog.pg_get_function_identity_arguments(p.oid), \
                    COALESCE(pg_catalog.array_to_string(p.proconfig,','),'<NULL>'),p.prosrc, \
                    COALESCE(pg_catalog.obj_description(p.oid,'pg_proc'),'<NULL>') \
               FROM pg_catalog.pg_proc p \
               JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
               JOIN pg_catalog.pg_language l ON l.oid=p.prolang \
               JOIN pg_catalog.pg_roles r ON r.oid=p.proowner \
              WHERE n.nspname='writer_lease' AND p.proname='writer_lease_rebind_v3'",
            &[],
        )
        .map_err(map_database)?;
    if rows.len() != 1 {
        return Err(profile_collision());
    }
    let row = &rows[0];
    if row.try_get::<_, String>(0).map_err(map_database)? != "p"
        || row.try_get::<_, String>(1).map_err(map_database)? != "plpgsql"
        || row.try_get::<_, String>(2).map_err(map_database)? != "lattice_migrator"
        || row.try_get::<_, bool>(3).map_err(map_database)?
        || row.try_get::<_, String>(4).map_err(map_database)? != "v"
        || row.try_get::<_, String>(5).map_err(map_database)? != "u"
        || !row
            .try_get::<_, String>(6)
            .map_err(map_database)?
            .is_empty()
        || row.try_get::<_, String>(7).map_err(map_database)?
            != "search_path=pg_catalog,row_security=on,lock_timeout=5s,statement_timeout=30s"
        || row.try_get::<_, String>(8).map_err(map_database)?.trim() != expected_body
        || row.try_get::<_, String>(9).map_err(map_database)? != "LATTICE_WRITER_LEASE_REBIND_V3"
    {
        return Err(profile_collision());
    }
    Ok(())
}

fn install_fresh_v3_current<C: GenericClient>(
    client: &mut C,
    target: &ExtensionTarget,
    foundation: &FoundationEvidence,
    v1: &ExtensionManifestEvidence,
    v2: &ExtensionManifestEvidence,
    v3: &ExtensionManifestEvidence,
    rebind: &ExtensionManifestEvidence,
) -> Result<(), SetupAttemptError> {
    for embedded in [v1, v2, v3] {
        let sql = std::str::from_utf8(embedded.bytes())
            .map_err(|_| setup_attempt_error(ExtensionSetupErrorKind::ManifestMismatch))?;
        client.batch_execute(sql).map_err(map_database)?;
    }
    ensure_v3_rebind_boundary(client, rebind)?;
    let inserted = client
        .execute(
            "INSERT INTO writer_lease.writer_lease_extension_identity (\
                 singleton, extension_id, extension_schema_version, extension_path, \
                 extension_sql_sha256, extension_manifest_sha256, database_uuid, \
                 database_identity_sha256, global_schema_version, global_manifest_sha256, \
                 required_memory_schema_version, required_memory_manifest_sha256\
             ) VALUES (true, $1, 3, $2, $3, $4, $5::text::uuid, $6, 6, $7, 3, $8)",
            &[
                &WRITER_LEASE_EXTENSION_ID,
                &WRITER_LEASE_V3_EXTENSION_PATH,
                &v3.sql_sha256().as_str(),
                &v3.manifest_sha256().as_str(),
                &foundation.database_uuid,
                &target.database_identity_digest().as_str(),
                &V6_GLOBAL_MANIFEST_SHA256,
                &CURRENT_MEMORY_MANIFEST_SHA256,
            ],
        )
        .map_err(map_database)?;
    if inserted != 1 {
        return Err(profile_collision());
    }
    insert_current_ledger(client, 1, "INSTALLED")?;
    activate_v3_runtime_acl(client)?;
    verify_v3_current_profile(client, target, &foundation.database_uuid, v1, v2, v3)
}

fn activate_v2_current<C: GenericClient>(
    client: &mut C,
    target: &ExtensionTarget,
    foundation: &FoundationEvidence,
    manifest: &ExtensionManifestEvidence,
) -> Result<(), SetupAttemptError> {
    let updated = client
        .execute(
            "UPDATE ONLY writer_lease.writer_lease_extension_identity SET \
                 global_schema_version = 5, global_manifest_sha256 = $1, \
                 required_memory_schema_version = 3, required_memory_manifest_sha256 = $2 \
              WHERE singleton AND extension_id = $3 AND extension_schema_version = 2 \
                AND extension_path = $4 AND extension_sql_sha256 = $5 \
                AND extension_manifest_sha256 = $6 AND database_uuid = $7::text::uuid \
                AND database_identity_sha256 = $8 AND global_schema_version = 3 \
                AND global_manifest_sha256 = $9 \
                AND required_memory_schema_version = 2 \
                AND required_memory_manifest_sha256 = $10",
            &[
                &target.global_manifest_digest().as_str(),
                &target.memory_manifest_digest().as_str(),
                &WRITER_LEASE_EXTENSION_ID,
                &WRITER_LEASE_EXTENSION_PATH,
                &manifest.sql_sha256().as_str(),
                &manifest.manifest_sha256().as_str(),
                &foundation.database_uuid,
                &target.database_identity_digest().as_str(),
                &HISTORICAL_GLOBAL_MANIFEST_SHA256,
                &HISTORICAL_MEMORY_MANIFEST_SHA256,
            ],
        )
        .map_err(map_database)?;
    if updated != 1 {
        return Err(setup_attempt_error(
            ExtensionSetupErrorKind::PartialOrCollidingProfile,
        ));
    }
    insert_current_ledger(client, 3, "REBOUND")?;
    activate_runtime_acl(client)
}

fn insert_current_ledger<C: GenericClient>(
    client: &mut C,
    ordinal: i16,
    event: &str,
) -> Result<(), SetupAttemptError> {
    let inserted = client
        .execute(
            "INSERT INTO writer_lease.writer_lease_extension_ledger (\
                 ledger_ordinal, singleton, extension_id, extension_schema_version, \
                 extension_sql_sha256, extension_manifest_sha256, database_uuid, \
                 database_identity_sha256, global_schema_version, global_manifest_sha256, \
                 required_memory_schema_version, required_memory_manifest_sha256, event_kind\
             ) SELECT $1, singleton, extension_id, extension_schema_version, \
                      extension_sql_sha256, extension_manifest_sha256, database_uuid, \
                      database_identity_sha256, global_schema_version, global_manifest_sha256, \
                      required_memory_schema_version, required_memory_manifest_sha256, $2 \
                 FROM ONLY writer_lease.writer_lease_extension_identity WHERE singleton",
            &[&ordinal, &event],
        )
        .map_err(map_database)?;
    if inserted != 1 {
        return Err(setup_attempt_error(
            ExtensionSetupErrorKind::PartialOrCollidingProfile,
        ));
    }
    Ok(())
}

fn activate_runtime_acl<C: GenericClient>(client: &mut C) -> Result<(), SetupAttemptError> {
    client
        .batch_execute(
            "GRANT USAGE ON SCHEMA writer_lease TO lattice_runtime; \
             GRANT EXECUTE ON FUNCTION writer_lease.writer_lease_bind_runtime_v2(\
                 text,bigint,bytea,text,text,text,text,text) TO lattice_runtime; \
             GRANT EXECUTE ON FUNCTION writer_lease.writer_lease_load_for_update_v2(\
                 text,bytea,bytea,bytea,text) TO lattice_runtime; \
             GRANT EXECUTE ON FUNCTION writer_lease.writer_lease_commit_plan_v1(\
                 text,bigint,bytea,bigint,bytea,text,bytea,text,text,bigint,bytea,bytea,\
                 bytea,bytea,bigint,bigint,bigint,bytea,text,bytea,text,text,text,bytea,\
                 text,text,text,text,bigint,bytea,text,bigint,bigint,text,bigint,text,bytea,\
                 bytea,bytea,bytea,bytea,text,text,bytea,bytea,bytea,text,bytea\
             ) TO lattice_runtime; \
             GRANT EXECUTE ON FUNCTION writer_lease.writer_lease_load_commands_v1(text) \
                 TO lattice_runtime; \
             GRANT EXECUTE ON FUNCTION writer_lease.writer_lease_load_current_v1(text) \
                 TO lattice_runtime; \
             GRANT EXECUTE ON FUNCTION writer_lease.writer_lease_assert_current_v1(\
                 text,text,text,text,bytea,text,text,text,text,bigint,bytea,text,bigint,bigint,bytea\
             ) TO lattice_runtime; \
             GRANT EXECUTE ON FUNCTION writer_lease.writer_lease_load_transitions_v1(text) \
                 TO lattice_runtime;",
        )
        .map_err(map_database)
}

fn activate_v3_runtime_acl<C: GenericClient>(client: &mut C) -> Result<(), SetupAttemptError> {
    client
        .batch_execute(
            "GRANT USAGE ON SCHEMA writer_lease TO lattice_runtime; \
             GRANT EXECUTE ON FUNCTION writer_lease.writer_lease_bind_runtime_v3(\
                 text,bigint,bytea,text,text,text,text,text) TO lattice_runtime; \
             GRANT EXECUTE ON FUNCTION writer_lease.writer_lease_load_for_update_v3(\
                 text,bytea,bytea,bytea,text) TO lattice_runtime; \
             GRANT EXECUTE ON FUNCTION writer_lease.writer_lease_commit_plan_v1(\
                 text,bigint,bytea,bigint,bytea,text,bytea,text,text,bigint,bytea,bytea,\
                 bytea,bytea,bigint,bigint,bigint,bytea,text,bytea,text,text,text,bytea,\
                 text,text,text,text,bigint,bytea,text,bigint,bigint,text,bigint,text,bytea,\
                 bytea,bytea,bytea,bytea,text,text,bytea,bytea,bytea,text,bytea\
             ) TO lattice_runtime; \
             GRANT EXECUTE ON FUNCTION writer_lease.writer_lease_load_commands_v1(text) \
                 TO lattice_runtime; \
             GRANT EXECUTE ON FUNCTION writer_lease.writer_lease_load_current_v1(text) \
                 TO lattice_runtime; \
             GRANT EXECUTE ON FUNCTION writer_lease.writer_lease_assert_current_v1(\
                 text,text,text,text,bytea,text,text,text,text,bigint,bytea,text,bigint,bigint,bytea\
             ) TO lattice_runtime; \
             GRANT EXECUTE ON FUNCTION writer_lease.writer_lease_load_transitions_v1(text) \
                 TO lattice_runtime;",
        )
        .map_err(map_database)
}

fn verify_bridge_safety<C: GenericClient>(client: &mut C) -> Result<(), SetupAttemptError> {
    lock_semantic_tables(client)?;
    let admission: String = client
        .query_one(
            "SELECT a.admission_mode::text FROM ONLY control.runtime_admission AS a \
              WHERE a.singleton FOR SHARE OF a",
            &[],
        )
        .map_err(map_database)?
        .try_get(0)
        .map_err(map_database)?;
    let live_authorities: i64 = client
        .query_one(
            "SELECT pg_catalog.count(*) \
               FROM ONLY writer_lease.writer_lease_heads AS h \
              WHERE h.current_status IN ('ACTIVE','SUSPECT')",
            &[],
        )
        .map_err(map_database)?
        .try_get(0)
        .map_err(map_database)?;
    if admission != "STOPPED" || live_authorities != 0 {
        return Err(setup_attempt_error(
            ExtensionSetupErrorKind::UnsupportedFoundation,
        ));
    }
    verify_replay_safe_history(client)
}

fn lock_semantic_tables<C: GenericClient>(client: &mut C) -> Result<(), SetupAttemptError> {
    client
        .batch_execute(
            "LOCK TABLE writer_lease.writer_lease_heads, \
                        writer_lease.writer_lease_commands, \
                        writer_lease.writer_lease_transitions IN SHARE MODE;",
        )
        .map_err(map_database)
}

fn verify_replay_safe_history<C: GenericClient>(client: &mut C) -> Result<(), SetupAttemptError> {
    let heads = client
        .query(
            "SELECT h.project_id::text, h.row_version, h.snapshot_schema_version, \
                    h.snapshot_bytes, pg_catalog.encode(h.snapshot_bytes_sha256,'hex'), \
                    pg_catalog.encode(h.snapshot_digest,'hex'), h.fencing_high_water, \
                    h.lease_revision, h.command_high_water, \
                    CASE WHEN h.command_tail_digest IS NULL THEN NULL \
                         ELSE pg_catalog.encode(h.command_tail_digest,'hex') END, \
                    h.current_status::text, \
                    CASE WHEN h.current_receipt_digest IS NULL THEN NULL ELSE \
                         pg_catalog.encode(h.current_receipt_digest,'hex') END, \
                    h.current_project_snapshot_id::text, h.current_task_id::text, \
                    h.current_task_revision::text, \
                    CASE WHEN h.current_task_spec_digest IS NULL THEN NULL ELSE \
                         pg_catalog.encode(h.current_task_spec_digest,'hex') END, \
                    h.current_attempt_id::text, h.current_lease_id::text, \
                    h.current_lease_holder_id::text, h.current_worktree_id::text, \
                    h.current_holder_process_id, \
                    CASE WHEN h.current_holder_process_start_identity IS NULL THEN NULL ELSE \
                         pg_catalog.encode(h.current_holder_process_start_identity,'hex') END, \
                    h.current_daemon_instance_id::text, h.current_daemon_epoch, \
                    h.current_fencing_token, h.current_expires_at \
               FROM ONLY writer_lease.writer_lease_heads AS h ORDER BY h.project_id",
            &[],
        )
        .map_err(map_database)?;
    for row in heads {
        let project_text: String = row.try_get(0).map_err(map_database)?;
        let project_id = ProjectId::new(project_text).map_err(|_| profile_collision())?;
        let row_version: i64 = row.try_get(1).map_err(map_database)?;
        let snapshot_schema_version: i16 = row.try_get(2).map_err(map_database)?;
        let snapshot_bytes: Vec<u8> = row.try_get(3).map_err(map_database)?;
        let snapshot_sha256: String = row.try_get(4).map_err(map_database)?;
        let snapshot_digest = content_digest(row.try_get(5).map_err(map_database)?)?;
        let fencing_high_water: i64 = row.try_get(6).map_err(map_database)?;
        let lease_revision: i64 = row.try_get(7).map_err(map_database)?;
        let command_high_water: i64 = row.try_get(8).map_err(map_database)?;
        let command_tail = row
            .try_get::<_, Option<String>>(9)
            .map_err(map_database)?
            .map(content_digest)
            .transpose()?;
        if snapshot_sha256 != sha256_hex(&snapshot_bytes)
            || snapshot_schema_version != 1
            || row_version < 0
            || row_version != command_high_water
            || fencing_high_water < 0
            || lease_revision < 0
            || command_high_water < 0
        {
            return Err(profile_collision());
        }
        let snapshot = UntrustedWriterLeaseSnapshot::from_canonical_bytes(&snapshot_bytes)
            .map_err(|_| profile_collision())?;
        let checkpoint = WriterLeaseCheckpoint::new(
            project_id.clone(),
            u64::try_from(command_high_water).map_err(|_| profile_collision())?,
            command_tail,
            snapshot_digest,
        )
        .map_err(|_| profile_collision())?;
        let aggregate = verify_snapshot_against_checkpoint(&snapshot, &checkpoint)
            .map_err(|_| profile_collision())?;
        if aggregate.fencing_high_water()
            != u64::try_from(fencing_high_water).map_err(|_| profile_collision())?
            || aggregate.revision()
                != u64::try_from(lease_revision).map_err(|_| profile_collision())?
        {
            return Err(profile_collision());
        }
        verify_head_projection(&row, &aggregate)?;
        verify_physical_history(client, &project_id, &aggregate)?;
    }
    Ok(())
}

fn verify_head_projection(
    row: &postgres::Row,
    aggregate: &VerifiedWriterLeaseAggregate,
) -> Result<(), SetupAttemptError> {
    let status: Option<String> = row.try_get(10).map_err(map_database)?;
    let receipt_digest: Option<String> = row.try_get(11).map_err(map_database)?;
    let project_snapshot_id: Option<String> = row.try_get(12).map_err(map_database)?;
    let task_id: Option<String> = row.try_get(13).map_err(map_database)?;
    let task_revision: Option<String> = row.try_get(14).map_err(map_database)?;
    let task_spec_digest: Option<String> = row.try_get(15).map_err(map_database)?;
    let attempt_id: Option<String> = row.try_get(16).map_err(map_database)?;
    let lease_id: Option<String> = row.try_get(17).map_err(map_database)?;
    let lease_holder_id: Option<String> = row.try_get(18).map_err(map_database)?;
    let worktree_id: Option<String> = row.try_get(19).map_err(map_database)?;
    let holder_process_id: Option<i64> = row.try_get(20).map_err(map_database)?;
    let holder_process_start_identity: Option<String> = row.try_get(21).map_err(map_database)?;
    let daemon_instance_id: Option<String> = row.try_get(22).map_err(map_database)?;
    let daemon_epoch: Option<i64> = row.try_get(23).map_err(map_database)?;
    let fencing_token: Option<i64> = row.try_get(24).map_err(map_database)?;
    let expires_at: Option<String> = row.try_get(25).map_err(map_database)?;
    match aggregate.current_receipt() {
        None => {
            if status.is_some()
                || receipt_digest.is_some()
                || project_snapshot_id.is_some()
                || task_id.is_some()
                || task_revision.is_some()
                || task_spec_digest.is_some()
                || attempt_id.is_some()
                || lease_id.is_some()
                || lease_holder_id.is_some()
                || worktree_id.is_some()
                || holder_process_id.is_some()
                || holder_process_start_identity.is_some()
                || daemon_instance_id.is_some()
                || daemon_epoch.is_some()
                || fencing_token.is_some()
                || expires_at.is_some()
            {
                return Err(profile_collision());
            }
        }
        Some(receipt) => {
            let identity = receipt.identity();
            if status.as_deref() != Some(receipt.status().as_str())
                || receipt_digest.as_deref() != Some(receipt.receipt_digest().as_str())
                || project_snapshot_id.as_deref() != Some(identity.project_snapshot_id().as_str())
                || task_id.as_deref() != Some(identity.task_id().as_str())
                || task_revision.as_deref() != Some(identity.task_revision())
                || task_spec_digest.as_deref() != Some(identity.task_spec_digest().as_str())
                || attempt_id.as_deref() != Some(identity.attempt_id().as_str())
                || lease_id.as_deref() != Some(identity.lease_id())
                || lease_holder_id.as_deref() != Some(identity.lease_holder_id())
                || worktree_id.as_deref() != Some(identity.worktree_id())
                || holder_process_id
                    != Some(
                        i64::try_from(identity.holder_process_id().get())
                            .map_err(|_| profile_collision())?,
                    )
                || holder_process_start_identity.as_deref()
                    != Some(identity.holder_process_start_identity().as_str())
                || daemon_instance_id.as_deref() != Some(identity.daemon_instance_id())
                || daemon_epoch
                    != Some(
                        i64::try_from(identity.daemon_epoch().get())
                            .map_err(|_| profile_collision())?,
                    )
                || fencing_token
                    != Some(
                        i64::try_from(identity.fencing_token().get())
                            .map_err(|_| profile_collision())?,
                    )
                || expires_at.as_deref() != Some(receipt.expires_at())
            {
                return Err(profile_collision());
            }
        }
    }
    Ok(())
}

fn verify_physical_history<C: GenericClient>(
    client: &mut C,
    project_id: &ProjectId,
    aggregate: &VerifiedWriterLeaseAggregate,
) -> Result<(), SetupAttemptError> {
    let command_rows = client
        .query(
            "SELECT c.ordinal, c.command_id::text, c.repository_request_bytes, \
                    pg_catalog.encode(c.repository_request_sha256,'hex'), c.request_bytes, \
                    pg_catalog.encode(c.request_digest,'hex'), \
                    CASE WHEN c.previous_receipt_digest IS NULL THEN NULL \
                         ELSE pg_catalog.encode(c.previous_receipt_digest,'hex') END, \
                    c.outcome::text, c.denial_reason::text, \
                    CASE WHEN c.transition_digest IS NULL THEN NULL \
                         ELSE pg_catalog.encode(c.transition_digest,'hex') END, \
                    c.receipt_bytes, pg_catalog.encode(c.receipt_digest,'hex') \
               FROM ONLY writer_lease.writer_lease_commands AS c \
              WHERE c.project_id=$1 ORDER BY c.ordinal",
            &[&project_id.as_str()],
        )
        .map_err(map_database)?;
    if command_rows.len() != aggregate.command_receipts().len() {
        return Err(profile_collision());
    }
    for (row, receipt) in command_rows.iter().zip(aggregate.command_receipts()) {
        let ordinal: i64 = row.try_get(0).map_err(map_database)?;
        let command_id: String = row.try_get(1).map_err(map_database)?;
        let repository_bytes: Vec<u8> = row.try_get(2).map_err(map_database)?;
        let repository_sha256: String = row.try_get(3).map_err(map_database)?;
        let request_bytes: Vec<u8> = row.try_get(4).map_err(map_database)?;
        let request_digest: String = row.try_get(5).map_err(map_database)?;
        let previous_digest: Option<String> = row.try_get(6).map_err(map_database)?;
        let outcome: String = row.try_get(7).map_err(map_database)?;
        let denial: Option<String> = row.try_get(8).map_err(map_database)?;
        let transition_digest: Option<String> = row.try_get(9).map_err(map_database)?;
        let receipt_bytes: Vec<u8> = row.try_get(10).map_err(map_database)?;
        let receipt_digest: String = row.try_get(11).map_err(map_database)?;
        let expected_repository = receipt
            .request
            .repository_intent_canonical_bytes()
            .map_err(|_| profile_collision())?;
        let expected_request = receipt
            .request
            .canonical_bytes()
            .map_err(|_| profile_collision())?;
        let expected_receipt = receipt.canonical_bytes().map_err(|_| profile_collision())?;
        let (expected_outcome, expected_denial) = match receipt.outcome {
            CommandOutcome::Applied => ("APPLIED", None),
            CommandOutcome::Denied(value) => ("DENIED", Some(value.as_str())),
        };
        if ordinal != i64::try_from(receipt.ordinal).map_err(|_| profile_collision())?
            || command_id != receipt.request.command_id()
            || repository_sha256 != sha256_hex(&repository_bytes)
            || repository_bytes != expected_repository
            || request_bytes != expected_request
            || request_digest != receipt.request_digest.as_str()
            || previous_digest.as_deref()
                != receipt
                    .previous_receipt_digest
                    .as_ref()
                    .map(ContentDigest::as_str)
            || outcome != expected_outcome
            || denial.as_deref() != expected_denial
            || transition_digest.as_deref()
                != receipt
                    .transition_digest
                    .as_ref()
                    .map(ContentDigest::as_str)
            || receipt_bytes != expected_receipt
            || receipt_digest != receipt.receipt_digest.as_str()
        {
            return Err(profile_collision());
        }
    }

    let transition_rows = client
        .query(
            "SELECT t.ordinal, t.command_id::text, t.transition_kind::text, \
                    t.transition_bytes, pg_catalog.encode(t.transition_digest,'hex') \
               FROM ONLY writer_lease.writer_lease_transitions AS t \
              WHERE t.project_id=$1 ORDER BY t.ordinal",
            &[&project_id.as_str()],
        )
        .map_err(map_database)?;
    if transition_rows.len() != aggregate.transitions().len() {
        return Err(profile_collision());
    }
    for (row, transition) in transition_rows.iter().zip(aggregate.transitions()) {
        let ordinal: i64 = row.try_get(0).map_err(map_database)?;
        let command_id: String = row.try_get(1).map_err(map_database)?;
        let kind: String = row.try_get(2).map_err(map_database)?;
        let bytes: Vec<u8> = row.try_get(3).map_err(map_database)?;
        let digest: String = row.try_get(4).map_err(map_database)?;
        if ordinal != i64::try_from(transition.ordinal).map_err(|_| profile_collision())?
            || command_id != transition.command_id
            || kind != transition.kind.as_str()
            || bytes
                != transition
                    .canonical_bytes()
                    .map_err(|_| profile_collision())?
            || digest != transition.transition_digest.as_str()
        {
            return Err(profile_collision());
        }
    }
    Ok(())
}

fn content_digest(value: String) -> Result<ContentDigest, SetupAttemptError> {
    ContentDigest::from_sha256(value).map_err(|_| profile_collision())
}

fn setup_attempt_error(kind: ExtensionSetupErrorKind) -> SetupAttemptError {
    SetupAttemptError::Setup(ExtensionSetupError::new(kind))
}

fn profile_collision() -> SetupAttemptError {
    setup_attempt_error(ExtensionSetupErrorKind::PartialOrCollidingProfile)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RuntimeProfile {
    Quarantined,
    Current,
}

fn verify_v3_bridge_profile<C: GenericClient>(
    client: &mut C,
    target: &ExtensionTarget,
    database_uuid: &str,
    v1: &ExtensionManifestEvidence,
    v2: &ExtensionManifestEvidence,
    v3: &ExtensionManifestEvidence,
) -> Result<(), SetupAttemptError> {
    verify_v3_catalog(client, RuntimeProfile::Quarantined)?;
    let identity = load_identity_shape(client)?;
    let expected_identity = identity_shape(
        database_uuid,
        target.database_identity_digest().as_str(),
        CURRENT_GLOBAL_MANIFEST_SHA256,
        CURRENT_MEMORY_MANIFEST_SHA256,
        v3,
        5,
        3,
    );
    let ledger = load_ledger_shape(client)?;
    let fresh_v2 = vec![
        ledger_shape(
            1,
            database_uuid,
            target.database_identity_digest().as_str(),
            CURRENT_GLOBAL_MANIFEST_SHA256,
            CURRENT_MEMORY_MANIFEST_SHA256,
            v2,
            5,
            3,
            "INSTALLED",
        ),
        ledger_shape(
            2,
            database_uuid,
            target.database_identity_digest().as_str(),
            CURRENT_GLOBAL_MANIFEST_SHA256,
            CURRENT_MEMORY_MANIFEST_SHA256,
            v3,
            5,
            3,
            "UPGRADED",
        ),
    ];
    let upgraded = vec![
        ledger_shape(
            1,
            database_uuid,
            target.database_identity_digest().as_str(),
            HISTORICAL_GLOBAL_MANIFEST_SHA256,
            HISTORICAL_MEMORY_MANIFEST_SHA256,
            v1,
            3,
            2,
            "INSTALLED",
        ),
        ledger_shape(
            2,
            database_uuid,
            target.database_identity_digest().as_str(),
            HISTORICAL_GLOBAL_MANIFEST_SHA256,
            HISTORICAL_MEMORY_MANIFEST_SHA256,
            v2,
            3,
            2,
            "UPGRADED",
        ),
        ledger_shape(
            3,
            database_uuid,
            target.database_identity_digest().as_str(),
            CURRENT_GLOBAL_MANIFEST_SHA256,
            CURRENT_MEMORY_MANIFEST_SHA256,
            v2,
            5,
            3,
            "REBOUND",
        ),
        ledger_shape(
            4,
            database_uuid,
            target.database_identity_digest().as_str(),
            CURRENT_GLOBAL_MANIFEST_SHA256,
            CURRENT_MEMORY_MANIFEST_SHA256,
            v3,
            5,
            3,
            "UPGRADED",
        ),
    ];
    if identity != expected_identity || (ledger != fresh_v2 && ledger != upgraded) {
        return Err(profile_collision());
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn verify_v3_current_profile<C: GenericClient>(
    client: &mut C,
    target: &ExtensionTarget,
    database_uuid: &str,
    v1: &ExtensionManifestEvidence,
    v2: &ExtensionManifestEvidence,
    v3: &ExtensionManifestEvidence,
) -> Result<(), SetupAttemptError> {
    verify_v3_catalog(client, RuntimeProfile::Current)?;
    let identity = load_identity_shape(client)?;
    let expected_identity = identity_shape(
        database_uuid,
        target.database_identity_digest().as_str(),
        V6_GLOBAL_MANIFEST_SHA256,
        CURRENT_MEMORY_MANIFEST_SHA256,
        v3,
        6,
        3,
    );
    let ledger = load_ledger_shape(client)?;
    let fresh = vec![ledger_shape(
        1,
        database_uuid,
        target.database_identity_digest().as_str(),
        V6_GLOBAL_MANIFEST_SHA256,
        CURRENT_MEMORY_MANIFEST_SHA256,
        v3,
        6,
        3,
        "INSTALLED",
    )];
    let fresh_v2_upgrade = vec![
        ledger_shape(
            1,
            database_uuid,
            target.database_identity_digest().as_str(),
            CURRENT_GLOBAL_MANIFEST_SHA256,
            CURRENT_MEMORY_MANIFEST_SHA256,
            v2,
            5,
            3,
            "INSTALLED",
        ),
        ledger_shape(
            2,
            database_uuid,
            target.database_identity_digest().as_str(),
            CURRENT_GLOBAL_MANIFEST_SHA256,
            CURRENT_MEMORY_MANIFEST_SHA256,
            v3,
            5,
            3,
            "UPGRADED",
        ),
        ledger_shape(
            3,
            database_uuid,
            target.database_identity_digest().as_str(),
            V6_GLOBAL_MANIFEST_SHA256,
            CURRENT_MEMORY_MANIFEST_SHA256,
            v3,
            6,
            3,
            "REBOUND",
        ),
    ];
    let upgraded = vec![
        ledger_shape(
            1,
            database_uuid,
            target.database_identity_digest().as_str(),
            HISTORICAL_GLOBAL_MANIFEST_SHA256,
            HISTORICAL_MEMORY_MANIFEST_SHA256,
            v1,
            3,
            2,
            "INSTALLED",
        ),
        ledger_shape(
            2,
            database_uuid,
            target.database_identity_digest().as_str(),
            HISTORICAL_GLOBAL_MANIFEST_SHA256,
            HISTORICAL_MEMORY_MANIFEST_SHA256,
            v2,
            3,
            2,
            "UPGRADED",
        ),
        ledger_shape(
            3,
            database_uuid,
            target.database_identity_digest().as_str(),
            CURRENT_GLOBAL_MANIFEST_SHA256,
            CURRENT_MEMORY_MANIFEST_SHA256,
            v2,
            5,
            3,
            "REBOUND",
        ),
        ledger_shape(
            4,
            database_uuid,
            target.database_identity_digest().as_str(),
            CURRENT_GLOBAL_MANIFEST_SHA256,
            CURRENT_MEMORY_MANIFEST_SHA256,
            v3,
            5,
            3,
            "UPGRADED",
        ),
        ledger_shape(
            5,
            database_uuid,
            target.database_identity_digest().as_str(),
            V6_GLOBAL_MANIFEST_SHA256,
            CURRENT_MEMORY_MANIFEST_SHA256,
            v3,
            6,
            3,
            "REBOUND",
        ),
    ];
    if identity != expected_identity
        || (ledger != fresh && ledger != fresh_v2_upgrade && ledger != upgraded)
    {
        return Err(profile_collision());
    }
    Ok(())
}

fn verify_v2_bridge_profile<C: GenericClient>(
    client: &mut C,
    target: &ExtensionTarget,
    database_uuid: &str,
    v1_manifest: &ExtensionManifestEvidence,
    manifest: &ExtensionManifestEvidence,
) -> Result<(), SetupAttemptError> {
    verify_v2_catalog(client, RuntimeProfile::Quarantined)?;
    let identity = load_identity_shape(client)?;
    let expected_identity = identity_shape(
        database_uuid,
        target.database_identity_digest().as_str(),
        HISTORICAL_GLOBAL_MANIFEST_SHA256,
        HISTORICAL_MEMORY_MANIFEST_SHA256,
        manifest,
        3,
        2,
    );
    let ledger = load_ledger_shape(client)?;
    let expected_ledger = vec![
        ledger_shape(
            1,
            database_uuid,
            target.database_identity_digest().as_str(),
            HISTORICAL_GLOBAL_MANIFEST_SHA256,
            HISTORICAL_MEMORY_MANIFEST_SHA256,
            v1_manifest,
            3,
            2,
            "INSTALLED",
        ),
        ledger_shape(
            2,
            database_uuid,
            target.database_identity_digest().as_str(),
            HISTORICAL_GLOBAL_MANIFEST_SHA256,
            HISTORICAL_MEMORY_MANIFEST_SHA256,
            manifest,
            3,
            2,
            "UPGRADED",
        ),
    ];
    if identity != expected_identity || ledger != expected_ledger {
        return Err(setup_attempt_error(
            ExtensionSetupErrorKind::PartialOrCollidingProfile,
        ));
    }
    Ok(())
}

fn verify_v2_current_profile<C: GenericClient>(
    client: &mut C,
    target: &ExtensionTarget,
    database_uuid: &str,
    v1_manifest: &ExtensionManifestEvidence,
    manifest: &ExtensionManifestEvidence,
) -> Result<(), SetupAttemptError> {
    verify_v2_catalog(client, RuntimeProfile::Current)?;
    let identity = load_identity_shape(client)?;
    let expected_identity = identity_shape(
        database_uuid,
        target.database_identity_digest().as_str(),
        target.global_manifest_digest().as_str(),
        target.memory_manifest_digest().as_str(),
        manifest,
        5,
        3,
    );
    let ledger = load_ledger_shape(client)?;
    let current = ledger_shape(
        1,
        database_uuid,
        target.database_identity_digest().as_str(),
        target.global_manifest_digest().as_str(),
        target.memory_manifest_digest().as_str(),
        manifest,
        5,
        3,
        "INSTALLED",
    );
    let upgraded = vec![
        ledger_shape(
            1,
            database_uuid,
            target.database_identity_digest().as_str(),
            HISTORICAL_GLOBAL_MANIFEST_SHA256,
            HISTORICAL_MEMORY_MANIFEST_SHA256,
            v1_manifest,
            3,
            2,
            "INSTALLED",
        ),
        ledger_shape(
            2,
            database_uuid,
            target.database_identity_digest().as_str(),
            HISTORICAL_GLOBAL_MANIFEST_SHA256,
            HISTORICAL_MEMORY_MANIFEST_SHA256,
            manifest,
            3,
            2,
            "UPGRADED",
        ),
        ledger_shape(
            3,
            database_uuid,
            target.database_identity_digest().as_str(),
            target.global_manifest_digest().as_str(),
            target.memory_manifest_digest().as_str(),
            manifest,
            5,
            3,
            "REBOUND",
        ),
    ];
    if identity != expected_identity || (ledger != [current] && ledger != upgraded) {
        return Err(setup_attempt_error(
            ExtensionSetupErrorKind::PartialOrCollidingProfile,
        ));
    }
    Ok(())
}

fn load_identity_shape<C: GenericClient>(client: &mut C) -> Result<String, SetupAttemptError> {
    let rows = client
        .query(
            "SELECT pg_catalog.concat_ws('|', i.extension_id, i.extension_schema_version, \
                    i.extension_path, pg_catalog.btrim(i.extension_sql_sha256), \
                    pg_catalog.btrim(i.extension_manifest_sha256), i.database_uuid, \
                    pg_catalog.btrim(i.database_identity_sha256), i.global_schema_version, \
                    pg_catalog.btrim(i.global_manifest_sha256), \
                    i.required_memory_schema_version, \
                    pg_catalog.btrim(i.required_memory_manifest_sha256))::text \
               FROM ONLY writer_lease.writer_lease_extension_identity AS i \
              ORDER BY i.singleton",
            &[],
        )
        .map_err(map_database)?;
    if rows.len() != 1 {
        return Err(setup_attempt_error(
            ExtensionSetupErrorKind::PartialOrCollidingProfile,
        ));
    }
    rows[0].try_get(0).map_err(map_database)
}

fn load_ledger_shape<C: GenericClient>(client: &mut C) -> Result<Vec<String>, SetupAttemptError> {
    client
        .query(
            "SELECT pg_catalog.concat_ws('|', l.ledger_ordinal, l.singleton, l.extension_id, \
                    l.extension_schema_version, pg_catalog.btrim(l.extension_sql_sha256), \
                    pg_catalog.btrim(l.extension_manifest_sha256), l.database_uuid, \
                    pg_catalog.btrim(l.database_identity_sha256), l.global_schema_version, \
                    pg_catalog.btrim(l.global_manifest_sha256), \
                    l.required_memory_schema_version, \
                    pg_catalog.btrim(l.required_memory_manifest_sha256), l.event_kind)::text \
               FROM ONLY writer_lease.writer_lease_extension_ledger AS l \
              ORDER BY l.ledger_ordinal",
            &[],
        )
        .map_err(map_database)?
        .into_iter()
        .map(|row| row.try_get(0).map_err(map_database))
        .collect()
}

fn identity_shape(
    database_uuid: &str,
    database_identity: &str,
    global_manifest: &str,
    memory_manifest: &str,
    manifest: &ExtensionManifestEvidence,
    global_version: i16,
    memory_version: i16,
) -> String {
    format!(
        "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
        WRITER_LEASE_EXTENSION_ID,
        manifest.schema_version(),
        manifest.path(),
        manifest.sql_sha256().as_str(),
        manifest.manifest_sha256().as_str(),
        database_uuid,
        database_identity,
        global_version,
        global_manifest,
        memory_version,
        memory_manifest
    )
}

#[allow(clippy::too_many_arguments)]
fn ledger_shape(
    ordinal: i16,
    database_uuid: &str,
    database_identity: &str,
    global_manifest: &str,
    memory_manifest: &str,
    manifest: &ExtensionManifestEvidence,
    global_version: i16,
    memory_version: i16,
    event: &str,
) -> String {
    format!(
        "{}|t|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
        ordinal,
        WRITER_LEASE_EXTENSION_ID,
        manifest.schema_version(),
        manifest.sql_sha256().as_str(),
        manifest.manifest_sha256().as_str(),
        database_uuid,
        database_identity,
        global_version,
        global_manifest,
        memory_version,
        memory_manifest,
        event
    )
}

#[allow(clippy::too_many_lines)]
fn verify_v2_catalog<C: GenericClient>(
    client: &mut C,
    runtime: RuntimeProfile,
) -> Result<(), SetupAttemptError> {
    let catalog_profiles = match runtime {
        RuntimeProfile::Quarantined => &V2_BRIDGE_EXPECTED_CATALOG_PROFILES,
        RuntimeProfile::Current => &V2_CURRENT_EXPECTED_CATALOG_PROFILES,
    };
    for &(query, expected_rows, expected_signature) in catalog_profiles {
        verify_catalog_profile(client, query, expected_rows, expected_signature)?;
    }
    let expected_runtime_functions = match runtime {
        RuntimeProfile::Quarantined => 0_i64,
        RuntimeProfile::Current => 7_i64,
    };
    let expected_usage = runtime == RuntimeProfile::Current;
    let row = client
        .query_one(
            "SELECT \
               (SELECT pg_catalog.count(*) FROM pg_catalog.pg_namespace n \
                 JOIN pg_catalog.pg_roles r ON r.oid=n.nspowner \
                WHERE n.nspname='writer_lease' AND r.rolname='lattice_migrator'), \
               (SELECT pg_catalog.count(*) FROM pg_catalog.pg_class c \
                 JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
                WHERE n.nspname='writer_lease' AND c.relkind IN ('r','p','v','m','S','f')), \
               (SELECT pg_catalog.count(*) FROM pg_catalog.pg_class c \
                 JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
                 JOIN pg_catalog.pg_roles r ON r.oid=c.relowner \
                WHERE n.nspname='writer_lease' AND c.relkind IN ('r','p','v','m','S','f') \
                  AND r.rolname='lattice_migrator'), \
               (SELECT pg_catalog.count(*) FROM pg_catalog.pg_proc p \
                 JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
                WHERE n.nspname='writer_lease'), \
               (SELECT pg_catalog.count(*) FROM pg_catalog.pg_proc p \
                 JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
                 JOIN pg_catalog.pg_roles r ON r.oid=p.proowner \
                WHERE n.nspname='writer_lease' AND p.prosecdef \
                  AND r.rolname='lattice_migrator'), \
               (SELECT pg_catalog.count(*) FROM pg_catalog.pg_proc p \
                 JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
                WHERE n.nspname='writer_lease' \
                  AND pg_catalog.has_function_privilege('lattice_runtime',p.oid,'EXECUTE')), \
               (SELECT pg_catalog.count(*) FROM pg_catalog.pg_class c \
                 JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
                WHERE n.nspname='writer_lease' AND (\
                  pg_catalog.has_table_privilege('lattice_runtime',c.oid,'SELECT') OR \
                  pg_catalog.has_table_privilege('lattice_runtime',c.oid,'INSERT') OR \
                  pg_catalog.has_table_privilege('lattice_runtime',c.oid,'UPDATE') OR \
                  pg_catalog.has_table_privilege('lattice_runtime',c.oid,'DELETE'))), \
               (SELECT pg_catalog.count(*) FROM pg_catalog.pg_constraint c \
                 JOIN pg_catalog.pg_namespace n ON n.oid=c.connamespace \
                WHERE n.nspname='writer_lease'), \
               (SELECT pg_catalog.count(*) FROM pg_catalog.pg_class c \
                 JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
                WHERE n.nspname='writer_lease' AND c.relkind='i'), \
               pg_catalog.has_schema_privilege('lattice_runtime','writer_lease','USAGE'), \
               pg_catalog.has_schema_privilege('lattice_runtime','writer_lease','CREATE'), \
               pg_catalog.obj_description('writer_lease.writer_lease_extension_identity'::regclass,'pg_class'), \
               pg_catalog.obj_description('writer_lease.writer_lease_extension_ledger'::regclass,'pg_class'), \
               pg_catalog.obj_description('writer_lease'::regnamespace,'pg_namespace')",
            &[],
        )
        .map_err(map_database)?;
    let counts = [1_i64, 5, 5, 9, 9, expected_runtime_functions, 0, 27, 8];
    for (index, expected) in counts.into_iter().enumerate() {
        let observed: i64 = row.try_get(index).map_err(map_database)?;
        if observed != expected {
            return Err(setup_attempt_error(
                ExtensionSetupErrorKind::PartialOrCollidingProfile,
            ));
        }
    }
    let runtime_usage: bool = row.try_get(9).map_err(map_database)?;
    let runtime_create: bool = row.try_get(10).map_err(map_database)?;
    let identity_comment: String = row.try_get(11).map_err(map_database)?;
    let ledger_comment: String = row.try_get(12).map_err(map_database)?;
    let schema_comment: String = row.try_get(13).map_err(map_database)?;
    if runtime_usage != expected_usage
        || runtime_create
        || identity_comment != "LATTICE_WRITER_LEASE_EXTENSION_IDENTITY_V2"
        || ledger_comment != "LATTICE_WRITER_LEASE_EXTENSION_LEDGER_V2"
        || schema_comment != "LATTICE_WRITER_LEASE_SCHEMA_V2"
    {
        return Err(setup_attempt_error(
            ExtensionSetupErrorKind::PartialOrCollidingProfile,
        ));
    }

    let functions = client
        .query(
            "SELECT p.proname::text || '(' || pg_catalog.oidvectortypes(p.proargtypes) \
                    || ')|' || p.provolatile::text || '|' || p.proparallel::text || '|' \
                    || pg_catalog.has_function_privilege('lattice_runtime',p.oid,'EXECUTE')::text \
               FROM pg_catalog.pg_proc p \
               JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
              WHERE n.nspname='writer_lease' ORDER BY p.proname",
            &[],
        )
        .map_err(map_database)?
        .into_iter()
        .map(|row| row.try_get::<_, String>(0).map_err(map_database))
        .collect::<Result<Vec<_>, _>>()?;
    let allowed = |name: &str| {
        runtime == RuntimeProfile::Current
            && !matches!(
                name,
                "writer_lease_bind_runtime_v1" | "writer_lease_load_for_update_v1"
            )
    };
    let expected = [
        ("writer_lease_assert_current_v1", "text, text, text, text, bytea, text, text, text, text, bigint, bytea, text, bigint, bigint, bytea", "s", "s"),
        ("writer_lease_bind_runtime_v1", "text, bigint, bytea, text, text, text, text, text", "s", "s"),
        ("writer_lease_bind_runtime_v2", "text, bigint, bytea, text, text, text, text, text", "s", "s"),
        ("writer_lease_commit_plan_v1", "text, bigint, bytea, bigint, bytea, text, bytea, text, text, bigint, bytea, bytea, bytea, bytea, bigint, bigint, bigint, bytea, text, bytea, text, text, text, bytea, text, text, text, text, bigint, bytea, text, bigint, bigint, text, bigint, text, bytea, bytea, bytea, bytea, bytea, text, text, bytea, bytea, bytea, text, bytea", "v", "u"),
        ("writer_lease_load_commands_v1", "text", "s", "s"),
        ("writer_lease_load_current_v1", "text", "s", "s"),
        ("writer_lease_load_for_update_v1", "text, bytea, bytea, bytea, text", "v", "u"),
        ("writer_lease_load_for_update_v2", "text, bytea, bytea, bytea, text", "v", "u"),
        ("writer_lease_load_transitions_v1", "text", "s", "s"),
    ]
    .map(|(name, args, volatility, parallel)| {
        format!("{name}({args})|{volatility}|{parallel}|{}", allowed(name))
    });
    if functions != expected {
        return Err(setup_attempt_error(
            ExtensionSetupErrorKind::PartialOrCollidingProfile,
        ));
    }
    verify_v2_function_sources(client)?;
    verify_v2_structural_invariants(client)?;
    let expected_missing = match runtime {
        RuntimeProfile::Quarantined => 9,
        RuntimeProfile::Current => 2,
    };
    verify_namespace_and_effective_acl_closure(client, expected_missing, expected_usage)
}

#[allow(clippy::too_many_lines)]
fn verify_v3_catalog<C: GenericClient>(
    client: &mut C,
    runtime: RuntimeProfile,
) -> Result<(), SetupAttemptError> {
    let rebind = verify_embedded_v3_rebind_manifest().map_err(|_| profile_collision())?;
    verify_v3_rebind_boundary(client, &rebind)?;
    for (query, rows, signature) in [
        (
            COLUMN_PROFILE_SQL,
            73,
            "560e93c2a765db0024c0e74d25a51b90cfc72b204601139de8fdb688d48c0610",
        ),
        (
            TABLE_ACL_PROFILE_SQL,
            40,
            "b99ef0c0ea5b550ae5e805d29b0020e31c1800a016b0de82cda566d7b25e9569",
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
    ] {
        verify_catalog_profile(client, query, rows, signature)?;
    }
    let expected_runtime_functions = match runtime {
        RuntimeProfile::Quarantined => 0_i64,
        RuntimeProfile::Current => 7_i64,
    };
    let expected_usage = runtime == RuntimeProfile::Current;
    let row = client
        .query_one(
            "SELECT \
               (SELECT pg_catalog.count(*) FROM pg_catalog.pg_namespace n \
                 JOIN pg_catalog.pg_roles r ON r.oid=n.nspowner \
                WHERE n.nspname='writer_lease' AND r.rolname='lattice_migrator'), \
               (SELECT pg_catalog.count(*) FROM pg_catalog.pg_class c \
                 JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
                WHERE n.nspname='writer_lease' AND c.relkind IN ('r','p','v','m','S','f')), \
               (SELECT pg_catalog.count(*) FROM pg_catalog.pg_proc p \
                 JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
                WHERE n.nspname='writer_lease'), \
               (SELECT pg_catalog.count(*) FROM pg_catalog.pg_proc p \
                 JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
                 JOIN pg_catalog.pg_roles r ON r.oid=p.proowner \
                WHERE n.nspname='writer_lease' AND r.rolname='lattice_migrator'), \
               (SELECT pg_catalog.count(*) FILTER (WHERE p.prosecdef) \
                  FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
                 WHERE n.nspname='writer_lease'), \
               (SELECT pg_catalog.count(*) FROM pg_catalog.pg_proc p \
                 JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
                WHERE n.nspname='writer_lease' \
                  AND pg_catalog.has_function_privilege('lattice_runtime',p.oid,'EXECUTE')), \
               (SELECT pg_catalog.count(*) FROM pg_catalog.pg_class c \
                 JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
                WHERE n.nspname='writer_lease' AND (\
                  pg_catalog.has_table_privilege('lattice_runtime',c.oid,'SELECT') OR \
                  pg_catalog.has_table_privilege('lattice_runtime',c.oid,'INSERT') OR \
                  pg_catalog.has_table_privilege('lattice_runtime',c.oid,'UPDATE') OR \
                  pg_catalog.has_table_privilege('lattice_runtime',c.oid,'DELETE'))), \
               (SELECT pg_catalog.count(*) FROM pg_catalog.pg_constraint c \
                 JOIN pg_catalog.pg_namespace n ON n.oid=c.connamespace \
                WHERE n.nspname='writer_lease'), \
               (SELECT pg_catalog.count(*) FROM pg_catalog.pg_class c \
                 JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
                WHERE n.nspname='writer_lease' AND c.relkind='i'), \
               pg_catalog.has_schema_privilege('lattice_runtime','writer_lease','USAGE'), \
               pg_catalog.has_schema_privilege('lattice_runtime','writer_lease','CREATE'), \
               pg_catalog.obj_description('writer_lease.writer_lease_extension_identity'::regclass,'pg_class'), \
               pg_catalog.obj_description('writer_lease.writer_lease_extension_ledger'::regclass,'pg_class'), \
               pg_catalog.obj_description('writer_lease'::regnamespace,'pg_namespace')",
            &[],
        )
        .map_err(map_database)?;
    let counts = [1_i64, 5, 12, 12, 11, expected_runtime_functions, 0, 27, 8];
    for (index, expected) in counts.into_iter().enumerate() {
        if row.try_get::<_, i64>(index).map_err(map_database)? != expected {
            return Err(profile_collision());
        }
    }
    if row.try_get::<_, bool>(9).map_err(map_database)? != expected_usage
        || row.try_get::<_, bool>(10).map_err(map_database)?
        || row.try_get::<_, String>(11).map_err(map_database)?
            != "LATTICE_WRITER_LEASE_EXTENSION_IDENTITY_V3"
        || row.try_get::<_, String>(12).map_err(map_database)?
            != "LATTICE_WRITER_LEASE_EXTENSION_LEDGER_V3"
        || row.try_get::<_, String>(13).map_err(map_database)? != "LATTICE_WRITER_LEASE_SCHEMA_V3"
    {
        return Err(profile_collision());
    }

    let functions = client
        .query(
            "SELECT p.proname::text, p.prokind::text, p.provolatile::text, \
                    p.proparallel::text, p.prosecdef, \
                    pg_catalog.pg_get_function_identity_arguments(p.oid), \
                    pg_catalog.has_function_privilege('lattice_runtime',p.oid,'EXECUTE') \
               FROM pg_catalog.pg_proc p \
               JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
              WHERE n.nspname='writer_lease' \
              ORDER BY p.proname,pg_catalog.pg_get_function_identity_arguments(p.oid)",
            &[],
        )
        .map_err(map_database)?;
    let expected = [
        (
            "writer_lease_assert_current_v1",
            "f",
            "s",
            "s",
            true,
            runtime == RuntimeProfile::Current,
        ),
        ("writer_lease_bind_runtime_v1", "f", "s", "s", true, false),
        ("writer_lease_bind_runtime_v2", "f", "s", "s", true, false),
        (
            "writer_lease_bind_runtime_v3",
            "f",
            "s",
            "s",
            true,
            runtime == RuntimeProfile::Current,
        ),
        (
            "writer_lease_commit_plan_v1",
            "f",
            "v",
            "u",
            true,
            runtime == RuntimeProfile::Current,
        ),
        (
            "writer_lease_load_commands_v1",
            "f",
            "s",
            "s",
            true,
            runtime == RuntimeProfile::Current,
        ),
        (
            "writer_lease_load_current_v1",
            "f",
            "s",
            "s",
            true,
            runtime == RuntimeProfile::Current,
        ),
        (
            "writer_lease_load_for_update_v1",
            "f",
            "v",
            "u",
            true,
            false,
        ),
        (
            "writer_lease_load_for_update_v2",
            "f",
            "v",
            "u",
            true,
            false,
        ),
        (
            "writer_lease_load_for_update_v3",
            "f",
            "v",
            "u",
            true,
            runtime == RuntimeProfile::Current,
        ),
        (
            "writer_lease_load_transitions_v1",
            "f",
            "s",
            "s",
            true,
            runtime == RuntimeProfile::Current,
        ),
        ("writer_lease_rebind_v3", "p", "v", "u", false, false),
    ];
    if functions.len() != expected.len() {
        return Err(profile_collision());
    }
    for (row, (name, kind, volatility, parallel, security_definer, runtime_execute)) in
        functions.iter().zip(expected)
    {
        if row.try_get::<_, String>(0).map_err(map_database)? != name
            || row.try_get::<_, String>(1).map_err(map_database)? != kind
            || row.try_get::<_, String>(2).map_err(map_database)? != volatility
            || row.try_get::<_, String>(3).map_err(map_database)? != parallel
            || row.try_get::<_, bool>(4).map_err(map_database)? != security_definer
            || row.try_get::<_, bool>(6).map_err(map_database)? != runtime_execute
            || (name == "writer_lease_rebind_v3"
                && !row
                    .try_get::<_, String>(5)
                    .map_err(map_database)?
                    .is_empty())
        {
            return Err(profile_collision());
        }
    }
    verify_v3_function_sources(client)?;
    let expected_missing = match runtime {
        RuntimeProfile::Quarantined => 12,
        RuntimeProfile::Current => 5,
    };
    verify_namespace_and_effective_acl_closure(client, expected_missing, expected_usage)
}

fn verify_v3_function_sources<C: GenericClient>(client: &mut C) -> Result<(), SetupAttemptError> {
    let v1 = verify_embedded_v1_extension_manifest().map_err(|_| profile_collision())?;
    let v2 = verify_embedded_extension_manifest().map_err(|_| profile_collision())?;
    let v3 = verify_embedded_v3_extension_manifest().map_err(|_| profile_collision())?;
    let v1_sql = std::str::from_utf8(v1.bytes()).map_err(|_| profile_collision())?;
    let v2_sql = std::str::from_utf8(v2.bytes()).map_err(|_| profile_collision())?;
    let v3_sql = std::str::from_utf8(v3.bytes()).map_err(|_| profile_collision())?;
    let descriptors = [
        (
            "writer_lease_assert_current_v1",
            "lattice_writer_lease_assert_current_v1",
            v1_sql,
        ),
        (
            "writer_lease_bind_runtime_v1",
            "lattice_writer_lease_bind_runtime_v1",
            v1_sql,
        ),
        (
            "writer_lease_bind_runtime_v2",
            "lattice_writer_lease_bind_runtime_v2",
            v2_sql,
        ),
        (
            "writer_lease_bind_runtime_v3",
            "lattice_writer_lease_bind_runtime_v3",
            v3_sql,
        ),
        (
            "writer_lease_commit_plan_v1",
            "lattice_writer_lease_commit_plan_v1",
            v1_sql,
        ),
        (
            "writer_lease_load_commands_v1",
            "lattice_writer_lease_load_commands_v1",
            v1_sql,
        ),
        (
            "writer_lease_load_current_v1",
            "lattice_writer_lease_load_current_v1",
            v1_sql,
        ),
        (
            "writer_lease_load_for_update_v1",
            "lattice_writer_lease_load_for_update_v1",
            v1_sql,
        ),
        (
            "writer_lease_load_for_update_v2",
            "lattice_writer_lease_load_for_update_v2",
            v2_sql,
        ),
        (
            "writer_lease_load_for_update_v3",
            "lattice_writer_lease_load_for_update_v3",
            v3_sql,
        ),
        (
            "writer_lease_load_transitions_v1",
            "lattice_writer_lease_load_transitions_v1",
            v1_sql,
        ),
    ];
    let observed = client
        .query(
            "SELECT p.proname::text,p.prosrc::text FROM pg_catalog.pg_proc p \
               JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
              WHERE n.nspname='writer_lease' AND p.prokind='f' ORDER BY p.proname",
            &[],
        )
        .map_err(map_database)?;
    if observed.len() != descriptors.len() {
        return Err(profile_collision());
    }
    for (row, (name, delimiter, sql)) in observed.iter().zip(descriptors) {
        if row.try_get::<_, String>(0).map_err(map_database)? != name
            || row.try_get::<_, String>(1).map_err(map_database)?
                != embedded_function_source(sql, delimiter)?
        {
            return Err(profile_collision());
        }
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn verify_v2_structural_invariants<C: GenericClient>(
    client: &mut C,
) -> Result<(), SetupAttemptError> {
    // V2 does not alter columns, table ACLs, column ACLs, or table-backed
    // types. Reuse the frozen v1 signatures for those unchanged surfaces.
    for (query, rows, signature) in [
        (
            COLUMN_PROFILE_SQL,
            73,
            "560e93c2a765db0024c0e74d25a51b90cfc72b204601139de8fdb688d48c0610",
        ),
        (
            TABLE_ACL_PROFILE_SQL,
            40,
            "b99ef0c0ea5b550ae5e805d29b0020e31c1800a016b0de82cda566d7b25e9569",
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
    ] {
        verify_catalog_profile(client, query, rows, signature)?;
    }

    let relations = client
        .query(
            "SELECT c.relname::text, COALESCE(pg_catalog.array_to_string(c.reloptions,','),''), \
                    COALESCE(pg_catalog.obj_description(c.oid,'pg_class'),'') \
               FROM pg_catalog.pg_class AS c \
               JOIN pg_catalog.pg_namespace AS n ON n.oid=c.relnamespace \
              WHERE n.nspname='writer_lease' AND c.relkind='r' ORDER BY c.relname",
            &[],
        )
        .map_err(map_database)?
        .into_iter()
        .map(|row| {
            Ok(format!(
                "{}|{}|{}",
                row.try_get::<_, String>(0).map_err(map_database)?,
                row.try_get::<_, String>(1).map_err(map_database)?,
                row.try_get::<_, String>(2).map_err(map_database)?
            ))
        })
        .collect::<Result<Vec<_>, SetupAttemptError>>()?;
    let expected_relations = [
        "writer_lease_commands||LATTICE_WRITER_LEASE_COMMANDS_V1",
        "writer_lease_extension_identity||LATTICE_WRITER_LEASE_EXTENSION_IDENTITY_V2",
        "writer_lease_extension_ledger||LATTICE_WRITER_LEASE_EXTENSION_LEDGER_V2",
        "writer_lease_heads||LATTICE_WRITER_LEASE_HEADS_V1",
        "writer_lease_transitions||LATTICE_WRITER_LEASE_TRANSITIONS_V1",
    ];
    if relations.iter().map(String::as_str).ne(expected_relations) {
        return Err(profile_collision());
    }

    let indexes = client
        .query(
            "SELECT c.relname::text, COALESCE(pg_catalog.array_to_string(c.reloptions,','),'') \
               FROM pg_catalog.pg_class AS c \
               JOIN pg_catalog.pg_namespace AS n ON n.oid=c.relnamespace \
              WHERE n.nspname='writer_lease' AND c.relkind='i' ORDER BY c.relname",
            &[],
        )
        .map_err(map_database)?
        .into_iter()
        .map(|row| {
            Ok(format!(
                "{}|{}",
                row.try_get::<_, String>(0).map_err(map_database)?,
                row.try_get::<_, String>(1).map_err(map_database)?
            ))
        })
        .collect::<Result<Vec<_>, SetupAttemptError>>()?;
    let expected_indexes = [
        "writer_lease_commands_id_unique|",
        "writer_lease_commands_pkey|",
        "writer_lease_commands_receipt_unique|",
        "writer_lease_extension_identity_pkey|",
        "writer_lease_extension_ledger_pkey|",
        "writer_lease_heads_pkey|",
        "writer_lease_transitions_digest_unique|",
        "writer_lease_transitions_pkey|",
    ];
    if indexes.iter().map(String::as_str).ne(expected_indexes) {
        return Err(profile_collision());
    }

    let version_definition: String = client
        .query_one(
            "SELECT pg_catalog.pg_get_constraintdef(c.oid,false) \
               FROM pg_catalog.pg_constraint AS c \
               JOIN pg_catalog.pg_namespace AS n ON n.oid=c.connamespace \
              WHERE n.nspname='writer_lease' \
                AND c.conname='writer_lease_heads_versions'",
            &[],
        )
        .map_err(map_database)?
        .try_get(0)
        .map_err(map_database)?;
    for required in [
        "row_version >= 0",
        "snapshot_schema_version = 1",
        "fencing_high_water >= 0",
        "lease_revision >= 0",
        "command_high_water >= 0",
    ] {
        if !version_definition.contains(required) {
            return Err(profile_collision());
        }
    }
    Ok(())
}

fn verify_v2_function_sources<C: GenericClient>(client: &mut C) -> Result<(), SetupAttemptError> {
    let v1 = verify_embedded_v1_extension_manifest().map_err(|_| profile_collision())?;
    let v2 = verify_embedded_extension_manifest().map_err(|_| profile_collision())?;
    let v1_sql = std::str::from_utf8(v1.bytes()).map_err(|_| profile_collision())?;
    let v2_sql = std::str::from_utf8(v2.bytes()).map_err(|_| profile_collision())?;
    let descriptors = [
        (
            "writer_lease_assert_current_v1",
            "lattice_writer_lease_assert_current_v1",
            v1_sql,
        ),
        (
            "writer_lease_bind_runtime_v1",
            "lattice_writer_lease_bind_runtime_v1",
            v1_sql,
        ),
        (
            "writer_lease_bind_runtime_v2",
            "lattice_writer_lease_bind_runtime_v2",
            v2_sql,
        ),
        (
            "writer_lease_commit_plan_v1",
            "lattice_writer_lease_commit_plan_v1",
            v1_sql,
        ),
        (
            "writer_lease_load_commands_v1",
            "lattice_writer_lease_load_commands_v1",
            v1_sql,
        ),
        (
            "writer_lease_load_current_v1",
            "lattice_writer_lease_load_current_v1",
            v1_sql,
        ),
        (
            "writer_lease_load_for_update_v1",
            "lattice_writer_lease_load_for_update_v1",
            v1_sql,
        ),
        (
            "writer_lease_load_for_update_v2",
            "lattice_writer_lease_load_for_update_v2",
            v2_sql,
        ),
        (
            "writer_lease_load_transitions_v1",
            "lattice_writer_lease_load_transitions_v1",
            v1_sql,
        ),
    ];
    let observed = client
        .query(
            "SELECT p.proname::text, p.prosrc::text FROM pg_catalog.pg_proc p \
               JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
              WHERE n.nspname='writer_lease' ORDER BY p.proname",
            &[],
        )
        .map_err(map_database)?;
    if observed.len() != descriptors.len() {
        return Err(profile_collision());
    }
    for (row, (expected_name, delimiter, sql)) in observed.iter().zip(descriptors) {
        let name: String = row.try_get(0).map_err(map_database)?;
        let source: String = row.try_get(1).map_err(map_database)?;
        let expected_source = embedded_function_source(sql, delimiter)?;
        if name != expected_name || source != expected_source {
            return Err(profile_collision());
        }
    }
    Ok(())
}

fn embedded_function_source<'a>(
    sql: &'a str,
    delimiter: &str,
) -> Result<&'a str, SetupAttemptError> {
    let open = format!("AS ${delimiter}$");
    let close = format!("${delimiter}$;");
    let start = sql.find(&open).ok_or_else(profile_collision)? + open.len();
    let remainder = &sql[start..];
    let end = remainder.find(&close).ok_or_else(profile_collision)?;
    Ok(&remainder[..end])
}

#[allow(clippy::too_many_lines)]
fn verify_v1_profile<C: GenericClient>(
    client: &mut C,
    target: &ExtensionTarget,
    manifest: &ExtensionManifestEvidence,
    database_uuid: &str,
) -> Result<(), SetupAttemptError> {
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
        return Err(setup_attempt_error(
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
        return Err(setup_attempt_error(
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
        return Err(setup_attempt_error(
            ExtensionSetupErrorKind::PartialOrCollidingProfile,
        ));
    }

    for (query, expected_rows, expected_signature) in V1_EXPECTED_CATALOG_PROFILES {
        verify_catalog_profile(client, query, expected_rows, expected_signature)?;
    }
    verify_namespace_and_effective_acl_closure(client, 0, true)?;

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
        || extension_version != i16::try_from(manifest.schema_version()).unwrap_or(-1)
        || extension_path != manifest.path()
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
        return Err(setup_attempt_error(
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
) -> Result<(), SetupAttemptError> {
    let rows = client
        .query(query, &[])
        .map_err(map_database)?
        .into_iter()
        .map(|row| row.try_get::<_, String>(0).map_err(map_database))
        .collect::<Result<Vec<_>, _>>()?;
    if rows.len() != expected_rows {
        return Err(setup_attempt_error(
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
        return Err(setup_attempt_error(
            ExtensionSetupErrorKind::PartialOrCollidingProfile,
        ));
    }
    Ok(())
}

fn verify_namespace_and_effective_acl_closure<C: GenericClient>(
    client: &mut C,
    expected_runtime_missing: i64,
    expected_runtime_usage: bool,
) -> Result<(), SetupAttemptError> {
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
             pg_catalog.has_schema_privilege('lattice_runtime','writer_lease','USAGE'), \
             (pg_catalog.has_schema_privilege('lattice_runtime','writer_lease','CREATE') \
               OR EXISTS (SELECT 1 FROM pg_catalog.pg_roles roles \
                  WHERE NOT roles.rolsuper AND roles.rolname !~ '^pg_' \
                    AND roles.rolname NOT IN ('lattice_migrator','lattice_runtime') \
                    AND (pg_catalog.has_schema_privilege(roles.rolname,'writer_lease','USAGE') \
                      OR pg_catalog.has_schema_privilege(roles.rolname,'writer_lease','CREATE'))))",
            &[],
        )
        .map_err(map_database)?;
    let expected_counts = [12_i64, 0, 0, 0, 0, 0, 0, 0, 0, 0, expected_runtime_missing];
    for (index, expected) in expected_counts.into_iter().enumerate() {
        let observed: i64 = row.try_get(index).map_err(map_database)?;
        if observed != expected {
            return Err(setup_attempt_error(
                ExtensionSetupErrorKind::PartialOrCollidingProfile,
            ));
        }
    }
    let runtime_usage: bool = row.try_get(11).map_err(map_database)?;
    let schema_acl_drift: bool = row.try_get(12).map_err(map_database)?;
    if runtime_usage != expected_runtime_usage || schema_acl_drift {
        return Err(setup_attempt_error(
            ExtensionSetupErrorKind::PartialOrCollidingProfile,
        ));
    }
    Ok(())
}

#[allow(clippy::needless_pass_by_value)]
fn map_database(error: postgres::Error) -> SetupAttemptError {
    if error.code() == Some(&SqlState::T_R_SERIALIZATION_FAILURE) {
        SetupAttemptError::SerializationFailure
    } else {
        ExtensionSetupError::new(ExtensionSetupErrorKind::Database).into()
    }
}

fn map_database_or(error: postgres::Error, fallback: ExtensionSetupErrorKind) -> SetupAttemptError {
    let database = map_database(error);
    match database {
        SetupAttemptError::SerializationFailure => database,
        SetupAttemptError::Setup(_) => ExtensionSetupError::new(fallback).into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use postgres::NoTls;

    const MEASURED_PROFILES: [(&str, &str); 10] = [
        ("RELATION", RELATION_PROFILE_SQL),
        ("COLUMN", COLUMN_PROFILE_SQL),
        ("CONSTRAINT", CONSTRAINT_PROFILE_SQL),
        ("INDEX", INDEX_PROFILE_SQL),
        ("FUNCTION", FUNCTION_PROFILE_SQL),
        ("SCHEMA_ACL", SCHEMA_ACL_PROFILE_SQL),
        ("TABLE_ACL", TABLE_ACL_PROFILE_SQL),
        ("FUNCTION_ACL", FUNCTION_ACL_PROFILE_SQL),
        ("COLUMN_ACL", COLUMN_ACL_PROFILE_SQL),
        ("TYPE", TYPE_PROFILE_SQL),
    ];

    fn measured_signature(rows: &[String]) -> String {
        let mut framed = Vec::with_capacity(
            CATALOG_SIGNATURE_DOMAIN.len()
                + 8
                + rows.iter().map(|row| row.len() + 8).sum::<usize>(),
        );
        framed.extend_from_slice(CATALOG_SIGNATURE_DOMAIN);
        framed.extend_from_slice(
            &u64::try_from(rows.len())
                .expect("bounded PostgreSQL catalog rows")
                .to_be_bytes(),
        );
        for row in rows {
            framed.extend_from_slice(
                &u64::try_from(row.len())
                    .expect("bounded PostgreSQL catalog row")
                    .to_be_bytes(),
            );
            framed.extend_from_slice(row.as_bytes());
        }
        sha256_hex(&framed)
    }

    fn measurement_digest(name: &str) -> ContentDigest {
        ContentDigest::from_sha256(
            std::env::var(name).unwrap_or_else(|_| panic!("{name} is required")),
        )
        .unwrap_or_else(|_| panic!("{name} must be lowercase SHA-256"))
    }

    #[test]
    fn task076_catalog_measurement_when_requested() {
        let Ok(profile) = std::env::var("LATTICE_TASK076_CATALOG_MEASURE") else {
            return;
        };
        assert!(
            matches!(profile.as_str(), "bridge" | "current"),
            "LATTICE_TASK076_CATALOG_MEASURE must be bridge or current"
        );
        let url = std::env::var("LATTICE_WRITER_LEASE_MIGRATOR_URL")
            .expect("existing Writer migrator URL is required for catalog measurement");
        let mut client = Client::connect(&url, NoTls).expect("catalog measurement connection");
        let mut transaction = client
            .build_transaction()
            .isolation_level(IsolationLevel::RepeatableRead)
            .read_only(true)
            .start()
            .expect("read-only catalog measurement transaction");
        enter_migrator(&mut transaction).expect("catalog measurement migrator boundary");
        acquire_common_locks(&mut transaction).expect("ordered catalog measurement locks");
        let target = ExtensionTarget::new(
            std::env::var("LATTICE_WRITER_LEASE_DATABASE_NAME")
                .expect("existing Writer database name is required"),
            measurement_digest("LATTICE_WRITER_LEASE_DATABASE_IDENTITY_SHA256"),
            measurement_digest("LATTICE_WRITER_LEASE_GLOBAL_MANIFEST_SHA256"),
            measurement_digest("LATTICE_WRITER_LEASE_MEMORY_MANIFEST_SHA256"),
        )
        .expect("exact catalog measurement target");
        let foundation =
            verify_foundation(&mut transaction, &target).expect("exact measurement foundation");
        let state = classify_state(&mut transaction, &foundation).expect("exact measurement state");
        let v1 = verify_embedded_v1_extension_manifest().expect("frozen Writer v1 manifest");
        let v2 = verify_embedded_extension_manifest().expect("frozen Writer v2 manifest");
        match profile.as_str() {
            "bridge" => {
                assert_eq!(foundation.profile, FoundationProfile::G3MemoryV2);
                assert_eq!(state, InstalledState::G3MemoryV2WriterV2Bridge);
                verify_v2_bridge_profile(
                    &mut transaction,
                    &target,
                    &foundation.database_uuid,
                    &v1,
                    &v2,
                )
                .expect("exact quarantined bridge profile before measurement");
            }
            "current" => {
                assert_eq!(foundation.profile, FoundationProfile::G5MemoryV3);
                assert_eq!(state, InstalledState::G5MemoryV3WriterV2Current);
                verify_v2_current_profile(
                    &mut transaction,
                    &target,
                    &foundation.database_uuid,
                    &v1,
                    &v2,
                )
                .expect("exact current profile before measurement");
            }
            _ => unreachable!(),
        }
        verify_replay_safe_history(&mut transaction)
            .expect("replay-safe Writer history before catalog measurement");
        let token_profile = profile.to_ascii_uppercase();
        for (name, query) in MEASURED_PROFILES {
            let rows = transaction
                .query(query, &[])
                .expect("catalog measurement query")
                .into_iter()
                .map(|row| row.get::<_, String>(0))
                .collect::<Vec<_>>();
            println!(
                "TASK076_WRITER_CATALOG_{token_profile}_{name}_ROWS={}",
                rows.len()
            );
            println!(
                "TASK076_WRITER_CATALOG_{token_profile}_{name}_SHA256={}",
                measured_signature(&rows)
            );
        }
        transaction
            .commit()
            .expect("commit read-only catalog measurement");
        println!("TASK076_WRITER_CATALOG_{token_profile}_MEASURE_PASS");
    }
}
